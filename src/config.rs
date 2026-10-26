use std::fs;
use std::net::{Ipv4Addr, SocketAddr, UdpSocket};
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::cli::Cli;

const DEFAULT_PORT: u16 = 8888;
const DEFAULT_POLL_MS: u64 = 1000;
const TEMPLATE: &str = r#"# Spotify Round configuration
#
# Create an application at https://developer.spotify.com/dashboard, then copy
# its client id here. Add this exact redirect URI to the application:
#     http://127.0.0.1:8888/callback
client_id = ""

# Port used for the local OAuth callback listener.
redirect_port = 8888

# Host a phone should use to reach the login page; defaults to the address of
# the default route (for example 192.168.1.24).
# login_host = "spotify-round.local"

# How often the now-playing state is polled, in milliseconds.
poll_ms = 1000
"#;

/// Everything the application needs to know about its environment.
#[derive(Debug, Clone)]
pub struct Settings {
    pub config_dir: PathBuf,
    pub client_id: Option<String>,
    pub redirect_port: u16,
    /// Host a phone should use to reach the login page; `None` detects it.
    pub login_host: Option<String>,
    pub poll_interval: Duration,
    pub mock: bool,
    pub fullscreen: bool,
}

impl Settings {
    pub fn load(cli: &Cli) -> Result<Self> {
        let config_dir = cli
            .config_dir
            .clone()
            .or_else(|| std::env::var_os("SPOTIFY_ROUND_CONFIG_DIR").map(PathBuf::from))
            .or_else(default_config_dir)
            .context("could not determine the configuration directory")?;

        fs::create_dir_all(&config_dir)
            .with_context(|| format!("creating {}", config_dir.display()))?;

        let config_file = config_dir.join("config.toml");
        if !config_file.exists() {
            fs::write(&config_file, TEMPLATE)
                .with_context(|| format!("writing {}", config_file.display()))?;
            tracing::info!(path = %config_file.display(), "created a default configuration");
        }

        let file: FileConfig = match fs::read_to_string(&config_file) {
            Ok(text) => toml::from_str(&text)
                .with_context(|| format!("parsing {}", config_file.display()))?,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => FileConfig::default(),
            Err(err) => {
                return Err(err).with_context(|| format!("reading {}", config_file.display()));
            }
        };

        let client_id = cli
            .client_id
            .clone()
            .or(file.client_id)
            .map(|id| id.trim().to_owned())
            .filter(|id| !id.is_empty());

        Ok(Self {
            config_dir,
            client_id,
            redirect_port: file.redirect_port.unwrap_or(DEFAULT_PORT),
            login_host: file.login_host,
            poll_interval: Duration::from_millis(
                file.poll_ms.unwrap_or(DEFAULT_POLL_MS).clamp(250, 10_000),
            ),
            mock: cli.mock,
            fullscreen: !cli.windowed,
        })
    }

    pub fn config_file(&self) -> PathBuf {
        self.config_dir.join("config.toml")
    }

    pub fn token_file(&self) -> PathBuf {
        self.config_dir.join("tokens.json")
    }

    pub fn redirect_uri(&self) -> String {
        format!("http://127.0.0.1:{}/callback", self.redirect_port)
    }

    /// Base URL (`http://<host>:<port>`) of the login page a phone can reach.
    ///
    /// The Spotify callback stays on the loopback address — Spotify does not
    /// accept anything else — so this is only the page that hands the
    /// authorization code back to the display.
    pub fn login_base(&self) -> String {
        let host = self
            .login_host
            .as_deref()
            .map(str::trim)
            .map(|host| {
                host.trim_start_matches("http://")
                    .trim_start_matches("https://")
                    .trim_end_matches('/')
            })
            .filter(|host| !host.is_empty())
            .map(str::to_owned)
            .or_else(|| lan_ip().map(|ip| ip.to_string()))
            .unwrap_or_else(|| "127.0.0.1".to_owned());

        format!("http://{host}:{}", self.redirect_port)
    }
}

/// The IPv4 address this machine uses on the local network.
///
/// A UDP `connect` sends no packets; it only asks the routing table which
/// source address would be used, which is the one another device on the same
/// network can reach.
fn lan_ip() -> Option<Ipv4Addr> {
    let socket = UdpSocket::bind("0.0.0.0:0").ok()?;
    // TEST-NET-1: never routed, and nothing is sent anyway.
    socket.connect("192.0.2.1:9").ok()?;
    match socket.local_addr().ok()? {
        SocketAddr::V4(addr) => Some(*addr.ip()),
        SocketAddr::V6(_) => None,
    }
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct FileConfig {
    #[serde(default)]
    client_id: Option<String>,
    #[serde(default)]
    redirect_port: Option<u16>,
    #[serde(default)]
    login_host: Option<String>,
    #[serde(default)]
    poll_ms: Option<u64>,
}

fn default_config_dir() -> Option<PathBuf> {
    directories::ProjectDirs::from("", "", "spotify-round")
        .map(|dirs| dirs.config_dir().to_path_buf())
}

/// Restrict a token file to the current user on Unix.
pub fn write_private(path: &Path, contents: &[u8]) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).with_context(|| format!("creating {}", parent.display()))?;
    }
    fs::write(path, contents).with_context(|| format!("writing {}", path.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o600))
            .with_context(|| format!("securing {}", path.display()))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_partial_configuration() {
        let parsed: FileConfig = toml::from_str("client_id = \"abc\"\n").unwrap();
        assert_eq!(parsed.client_id.as_deref(), Some("abc"));
        assert_eq!(parsed.redirect_port, None);
        assert_eq!(parsed.poll_ms, None);
    }

    #[test]
    fn redirect_uri_uses_the_loopback_literal() {
        let settings = Settings {
            config_dir: PathBuf::from("."),
            client_id: Some("id".into()),
            redirect_port: 9999,
            login_host: None,
            poll_interval: Duration::from_secs(1),
            mock: false,
            fullscreen: true,
        };
        assert_eq!(settings.redirect_uri(), "http://127.0.0.1:9999/callback");
    }

    #[test]
    fn login_base_prefers_the_configured_host() {
        let mut settings = Settings {
            config_dir: PathBuf::from("."),
            client_id: Some("id".into()),
            redirect_port: 9999,
            login_host: Some("http://spotify-round.local/".into()),
            poll_interval: Duration::from_secs(1),
            mock: false,
            fullscreen: true,
        };
        assert_eq!(settings.login_base(), "http://spotify-round.local:9999");

        settings.login_host = None;
        assert!(settings.login_base().ends_with(":9999"));
    }
}
