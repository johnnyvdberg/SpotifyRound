//! Spotify authorization-code flow with PKCE.
//!
//! Spotify only allows `http://` redirect URIs for loopback IP literals, so the
//! callback lands on a small HTTP listener on `127.0.0.1`. A browser on the same
//! machine reaches that directly; a phone scanning the QR code cannot, because
//! `127.0.0.1` on the phone is the phone. The same listener therefore also
//! serves a login page on the local network: the phone approves access in
//! Spotify, the redirect to `127.0.0.1` fails there as expected, and the address
//! from the browser is pasted back into that page, which hands the code over.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use base64::Engine as _;
use rand::RngCore as _;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use url::Url;

use crate::config;

pub const SCOPES: &[&str] = &[
    "user-read-playback-state",
    "user-modify-playback-state",
    "user-read-currently-playing",
    "playlist-read-private",
    "playlist-read-collaborative",
];

const AUTHORIZE_URL: &str = "https://accounts.spotify.com/authorize";
const TOKEN_URL: &str = "https://accounts.spotify.com/api/token";
const CALLBACK_TIMEOUT: Duration = Duration::from_secs(300);

/// Raised when a command needs a token but the user has not authorized yet.
pub const NOT_AUTHORIZED: &str = "not authorized with Spotify yet";

/// Manages the access token and keeps the refresh token on disk.
pub struct Auth {
    http: reqwest::Client,
    client_id: String,
    redirect_uri: String,
    token_path: PathBuf,
    state: Mutex<TokenState>,
    refresh_lock: tokio::sync::Mutex<()>,
}

#[derive(Default)]
struct TokenState {
    access: Option<String>,
    expires_at: Option<Instant>,
    refresh: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct StoredToken {
    client_id: String,
    refresh_token: String,
    #[serde(default)]
    scopes: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct TokenResponse {
    access_token: String,
    #[serde(default)]
    expires_in: u64,
    #[serde(default)]
    refresh_token: Option<String>,
    #[serde(default)]
    scope: String,
}

/// A freshly granted set of tokens.
#[derive(Debug)]
pub struct Grant {
    pub(crate) access_token: String,
    pub(crate) expires_in: u64,
    pub(crate) refresh_token: Option<String>,
    pub(crate) scopes: Vec<String>,
}

impl Auth {
    pub fn new(client_id: String, redirect_uri: String, token_path: PathBuf) -> Result<Self> {
        let http = reqwest::Client::builder()
            .user_agent(concat!("spotify-round/", env!("CARGO_PKG_VERSION")))
            .timeout(Duration::from_secs(20))
            .build()
            .context("building the HTTP client")?;

        let refresh = load_refresh_token(&token_path, &client_id);
        Ok(Self {
            http,
            client_id,
            redirect_uri,
            token_path,
            state: Mutex::new(TokenState {
                refresh,
                ..Default::default()
            }),
            refresh_lock: tokio::sync::Mutex::new(()),
        })
    }

    pub fn is_authorized(&self) -> bool {
        self.state.lock().expect("token state").refresh.is_some()
    }

    /// Shared HTTP client; reusing it keeps the connection pool warm.
    pub fn http(&self) -> &reqwest::Client {
        &self.http
    }

    /// A valid access token, refreshed if it is about to expire.
    pub async fn access_token(&self) -> Result<String> {
        if let Some(token) = self.cached_access() {
            return Ok(token);
        }

        let _guard = self.refresh_lock.lock().await;
        if let Some(token) = self.cached_access() {
            return Ok(token);
        }

        self.refresh_now().await?;
        self.cached_access()
            .context("no access token available after refreshing")
    }

    fn cached_access(&self) -> Option<String> {
        let state = self.state.lock().expect("token state");
        match (&state.access, state.expires_at) {
            (Some(token), Some(expires)) if expires > Instant::now() + Duration::from_secs(60) => {
                Some(token.clone())
            }
            _ => None,
        }
    }

    async fn refresh_now(&self) -> Result<()> {
        let refresh = self
            .state
            .lock()
            .expect("token state")
            .refresh
            .clone()
            .context(NOT_AUTHORIZED)?;

        let response = self
            .http
            .post(TOKEN_URL)
            .form(&[
                ("grant_type", "refresh_token"),
                ("refresh_token", refresh.as_str()),
                ("client_id", self.client_id.as_str()),
            ])
            .send()
            .await
            .context("refreshing the Spotify access token")?;

        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            bail!("Spotify refused to refresh the token ({status}): {body}");
        }

        let token: TokenResponse = response
            .json()
            .await
            .context("decoding the token response")?;
        self.store(token, Vec::new())
    }

    /// Persist the result of an interactive login.
    pub fn finish_login(&self, grant: Grant) -> Result<()> {
        self.store(
            TokenResponse {
                access_token: grant.access_token,
                expires_in: grant.expires_in,
                refresh_token: grant.refresh_token,
                scope: String::new(),
            },
            grant.scopes,
        )
    }

    fn store(&self, token: TokenResponse, scopes: Vec<String>) -> Result<()> {
        let scopes = if scopes.is_empty() {
            token
                .scope
                .split_whitespace()
                .map(str::to_owned)
                .collect::<Vec<_>>()
        } else {
            scopes
        };

        let refresh_token = {
            let mut state = self.state.lock().expect("token state");
            state.access = Some(token.access_token);
            state.expires_at =
                Some(Instant::now() + Duration::from_secs(token.expires_in.saturating_sub(30)));
            if let Some(refresh) = token.refresh_token {
                state.refresh = Some(refresh);
            }
            state.refresh.clone()
        };

        if let Some(refresh_token) = refresh_token {
            let stored = StoredToken {
                client_id: self.client_id.clone(),
                refresh_token,
                scopes,
            };
            let json = serde_json::to_vec_pretty(&stored).context("encoding the token file")?;
            config::write_private(&self.token_path, &json)?;
        }
        Ok(())
    }

    /// Start an interactive login: binds the callback listener and builds the URLs.
    ///
    /// `login_base` is how another device on the network reaches this machine,
    /// for example `http://192.168.1.24:8888`. The page served there lets a
    /// phone approve access and hand the authorization code back, which is what
    /// makes the QR code work when the browser is not on this machine.
    pub async fn begin_login(&self, login_base: &str) -> Result<LoginFlow> {
        let port = Url::parse(&self.redirect_uri)
            .ok()
            .and_then(|url| url.port())
            .unwrap_or(8888);

        // Every interface, not just loopback: a phone cannot reach 127.0.0.1 on
        // this device. The listener only exists for the length of one login.
        let listener = TcpListener::bind(("0.0.0.0", port))
            .await
            .with_context(|| format!("binding the OAuth callback listener on port {port}"))?;

        let pkce = Pkce::generate();
        let state = random_urlsafe(24);
        let url = authorization_url(&self.client_id, &self.redirect_uri, &pkce.challenge, &state)?;
        let login_url = login_page_url(login_base)?;
        tracing::info!(%login_url, "serving the login page");

        Ok(LoginFlow {
            listener,
            login_url,
            url,
            verifier: pkce.verifier,
            state,
            client_id: self.client_id.clone(),
            redirect_uri: self.redirect_uri.clone(),
            http: self.http.clone(),
        })
    }
}

/// An in-flight interactive login.
pub struct LoginFlow {
    listener: TcpListener,
    login_url: Url,
    url: Url,
    verifier: String,
    state: String,
    client_id: String,
    redirect_uri: String,
    http: reqwest::Client,
}

impl LoginFlow {
    /// The Spotify authorization URL, for a browser on this machine.
    pub fn url(&self) -> &Url {
        &self.url
    }

    /// The page a phone on the same network opens; it hands the code back.
    pub fn login_url(&self) -> &Url {
        &self.login_url
    }

    /// Wait for the callback, then exchange the code.
    ///
    /// Two things can arrive: the redirect itself, when the browser runs on this
    /// machine, and a form post from the login page, when it runs elsewhere.
    pub async fn wait(self) -> Result<Grant> {
        let deadline = tokio::time::Instant::now() + CALLBACK_TIMEOUT;

        loop {
            let (mut socket, _) =
                match tokio::time::timeout_at(deadline, self.listener.accept()).await {
                    Ok(Ok(pair)) => pair,
                    Ok(Err(err)) => return Err(err).context("accepting the OAuth callback"),
                    Err(_) => bail!("timed out waiting for the Spotify authorization"),
                };

            let request = match read_request(&mut socket).await {
                Ok(request) => request,
                Err(err) => {
                    tracing::debug!(%err, "ignoring an unreadable callback request");
                    continue;
                }
            };

            match (request.method.as_str(), request.path()) {
                ("GET", "/") | ("GET", "/login") => {
                    let page = login_page(&self.url, &self.redirect_uri);
                    respond(&mut socket, "200 OK", &page).await;
                }
                ("GET", "/callback") => match parse_callback(&request.target) {
                    Callback::Code { code, state } => {
                        if state != self.state {
                            respond(&mut socket, "400 Bad Request", ERROR_PAGE).await;
                            bail!("the OAuth state did not match; please try again");
                        }
                        respond(&mut socket, "200 OK", SUCCESS_PAGE).await;
                        return self.exchange(&code).await;
                    }
                    Callback::Error(error) => {
                        respond(&mut socket, "200 OK", ERROR_PAGE).await;
                        bail!("Spotify denied the authorization: {error}");
                    }
                    Callback::Unexpected => {
                        // Browsers ask for /favicon.ico and similar; keep listening.
                        respond(&mut socket, "404 Not Found", NOT_FOUND_PAGE).await;
                    }
                },
                ("POST", "/callback") => match submitted_code(&request.body) {
                    Ok((code, state)) if state == self.state => {
                        // Answer after the exchange, so the phone does not claim
                        // success when Spotify refused the code.
                        match self.exchange(&code).await {
                            Ok(grant) => {
                                respond(&mut socket, "200 OK", SUCCESS_PAGE).await;
                                return Ok(grant);
                            }
                            Err(err) => {
                                respond(&mut socket, "200 OK", ERROR_PAGE).await;
                                return Err(err);
                            }
                        }
                    }
                    Ok(_) => {
                        let page = paste_page(
                            "That address belongs to an older login attempt. Scan the \
                             code on the display again.",
                        );
                        respond(&mut socket, "400 Bad Request", &page).await;
                    }
                    Err(message) => {
                        let page = paste_page(&message);
                        respond(&mut socket, "400 Bad Request", &page).await;
                    }
                },
                _ => respond(&mut socket, "404 Not Found", NOT_FOUND_PAGE).await,
            }
        }
    }

    async fn exchange(&self, code: &str) -> Result<Grant> {
        let response = self
            .http
            .post(TOKEN_URL)
            .form(&[
                ("grant_type", "authorization_code"),
                ("code", code),
                ("redirect_uri", self.redirect_uri.as_str()),
                ("client_id", self.client_id.as_str()),
                ("code_verifier", self.verifier.as_str()),
            ])
            .send()
            .await
            .context("exchanging the authorization code")?;

        let status = response.status();
        let body = response
            .text()
            .await
            .context("reading the token response")?;
        if !status.is_success() {
            bail!("Spotify rejected the authorization code ({status}): {body}");
        }

        let token: TokenResponse =
            serde_json::from_str(&body).context("decoding the token response")?;

        Ok(Grant {
            access_token: token.access_token,
            expires_in: token.expires_in,
            refresh_token: token.refresh_token,
            scopes: token.scope.split_whitespace().map(str::to_owned).collect(),
        })
    }
}

/// PKCE verifier/challenge pair (RFC 7636, S256).
#[derive(Debug)]
pub struct Pkce {
    pub verifier: String,
    pub challenge: String,
}

impl Pkce {
    pub fn generate() -> Self {
        let verifier = random_urlsafe(32);
        let challenge = Self::challenge(&verifier);
        Self {
            verifier,
            challenge,
        }
    }

    pub fn challenge(verifier: &str) -> String {
        let digest = Sha256::digest(verifier.as_bytes());
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(&digest[..])
    }
}

pub fn authorization_url(
    client_id: &str,
    redirect_uri: &str,
    challenge: &str,
    state: &str,
) -> Result<Url> {
    let mut url = Url::parse(AUTHORIZE_URL).context("building the authorization URL")?;
    url.query_pairs_mut()
        .append_pair("client_id", client_id)
        .append_pair("response_type", "code")
        .append_pair("redirect_uri", redirect_uri)
        .append_pair("scope", &SCOPES.join(" "))
        .append_pair("code_challenge_method", "S256")
        .append_pair("code_challenge", challenge)
        .append_pair("state", state);
    Ok(url)
}

fn random_urlsafe(bytes: usize) -> String {
    let mut buffer = vec![0u8; bytes];
    rand::rng().fill_bytes(&mut buffer);
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(&buffer)
}

fn load_refresh_token(path: &std::path::Path, client_id: &str) -> Option<String> {
    let text = std::fs::read_to_string(path).ok()?;
    match serde_json::from_str::<StoredToken>(&text) {
        Ok(stored) if stored.client_id == client_id => Some(stored.refresh_token),
        Ok(_) => {
            tracing::info!(
                "the stored token belongs to another client id; authorization is needed"
            );
            None
        }
        Err(err) => {
            tracing::warn!(%err, "ignoring an unreadable token file");
            None
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
enum Callback {
    Code { code: String, state: String },
    Error(String),
    Unexpected,
}

fn parse_callback(target: &str) -> Callback {
    let Ok(url) = Url::parse(&format!("http://127.0.0.1{target}")) else {
        return Callback::Unexpected;
    };
    if url.path() != "/callback" {
        return Callback::Unexpected;
    }
    parse_query(url.query().unwrap_or_default())
}

fn parse_query(query: &str) -> Callback {
    let Ok(url) = Url::parse(&format!("http://127.0.0.1/?{query}")) else {
        return Callback::Unexpected;
    };

    let params: HashMap<String, String> = url.query_pairs().into_owned().collect();
    if let Some(error) = params.get("error") {
        return Callback::Error(error.clone());
    }
    match (params.get("code"), params.get("state")) {
        (Some(code), Some(state)) => Callback::Code {
            code: code.clone(),
            state: state.clone(),
        },
        _ => Callback::Unexpected,
    }
}

/// A callback URL copied out of the phone's address bar.
///
/// The redirect target never loads on the phone, but the browser keeps the
/// address, so both the whole URL and just its query string are understood.
fn parse_pasted(input: &str) -> Callback {
    let input = input.trim();
    let query = match input.split_once('?') {
        Some((_, query)) => query,
        None => input,
    };
    parse_query(query)
}

/// Read the pasted address out of the login page's form body.
fn submitted_code(body: &str) -> std::result::Result<(String, String), String> {
    let pasted = form_field(body, "url").unwrap_or_default();
    if pasted.trim().is_empty() {
        return Err("Paste the address that Spotify sent your browser to.".to_owned());
    }

    match parse_pasted(&pasted) {
        Callback::Code { code, state } => Ok((code, state)),
        _ => Err(
            "That does not contain an authorization code. Copy the whole address from the \
             browser, including everything after the question mark."
                .to_owned(),
        ),
    }
}

/// One `application/x-www-form-urlencoded` field.
fn form_field(body: &str, name: &str) -> Option<String> {
    Url::parse(&format!("http://127.0.0.1/?{body}"))
        .ok()?
        .query_pairs()
        .find(|(key, _)| key == name)
        .map(|(_, value)| value.into_owned())
}

fn login_page_url(base: &str) -> Result<Url> {
    Url::parse(&format!("{}/", base.trim_end_matches('/')))
        .context("building the address of the login page")
}

/// The parts of an HTTP request the callback listener needs.
struct Request {
    method: String,
    target: String,
    body: String,
}

impl Request {
    fn path(&self) -> &str {
        self.target.split('?').next().unwrap_or("/")
    }
}

async fn read_request(socket: &mut TcpStream) -> Result<Request> {
    const MAX_HEAD: usize = 16 * 1024;
    const MAX_BODY: usize = 16 * 1024;

    let mut buffer = Vec::with_capacity(1024);
    let mut chunk = [0u8; 2048];
    let head_end = loop {
        let read = socket.read(&mut chunk).await?;
        if read == 0 {
            bail!("the connection closed before the request was complete");
        }
        buffer.extend_from_slice(&chunk[..read]);
        if let Some(offset) = buffer.windows(4).position(|window| window == b"\r\n\r\n") {
            break offset + 4;
        }
        if buffer.len() > MAX_HEAD {
            bail!("the request headers are too large");
        }
    };

    let head = String::from_utf8_lossy(&buffer[..head_end]).into_owned();
    let mut lines = head.lines();
    let mut request_line = lines.next().unwrap_or_default().split_whitespace();
    let method = request_line.next().unwrap_or_default().to_owned();
    let target = request_line.next().unwrap_or("/").to_owned();
    let content_length = lines
        .filter_map(|line| line.split_once(':'))
        .find(|(name, _)| name.eq_ignore_ascii_case("content-length"))
        .and_then(|(_, value)| value.trim().parse::<usize>().ok())
        .unwrap_or(0)
        .min(MAX_BODY);

    let mut body = buffer[head_end..].to_vec();
    while body.len() < content_length {
        let read = socket.read(&mut chunk).await?;
        if read == 0 {
            break;
        }
        body.extend_from_slice(&chunk[..read]);
    }
    body.truncate(content_length);

    Ok(Request {
        method,
        target,
        body: String::from_utf8_lossy(&body).into_owned(),
    })
}

async fn respond(socket: &mut TcpStream, status: &str, body: &str) {
    let response = format!(
        "HTTP/1.1 {status}\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    let _ = socket.write_all(response.as_bytes()).await;
    let _ = socket.flush().await;
    let _ = socket.shutdown().await;
}

const SUCCESS_PAGE: &str = r#"<!doctype html><html lang="en"><head><meta charset="utf-8">
<meta name="viewport" content="width=device-width,initial-scale=1"><title>Connected</title>
<style>
body{margin:0;height:100vh;display:grid;place-items:center;background:#08080b;color:#f5f5f8;
font-family:system-ui,-apple-system,"Segoe UI",sans-serif;text-align:center}
.card{padding:40px}
.dot{width:64px;height:64px;border-radius:32px;background:#1ed760;margin:0 auto 24px}
h1{font-size:26px;font-weight:600;margin:0 0 10px}
p{color:#9c9ca8;margin:0;font-size:16px}
</style></head><body><div class="card"><div class="dot"></div>
<h1>Spotify Round is connected</h1><p>You can close this window and enjoy the music.</p>
</div></body></html>"#;

const ERROR_PAGE: &str = r#"<!doctype html><html lang="en"><head><meta charset="utf-8">
<meta name="viewport" content="width=device-width,initial-scale=1"><title>Not connected</title>
<style>
body{margin:0;height:100vh;display:grid;place-items:center;background:#08080b;color:#f5f5f8;
font-family:system-ui,-apple-system,"Segoe UI",sans-serif;text-align:center}
.card{padding:40px}
.dot{width:64px;height:64px;border-radius:32px;background:#ff5c5c;margin:0 auto 24px}
h1{font-size:26px;font-weight:600;margin:0 0 10px}
p{color:#9c9ca8;margin:0;font-size:16px}
</style></head><body><div class="card"><div class="dot"></div>
<h1>Authorization was not completed</h1><p>Return to the display and try again.</p>
</div></body></html>"#;

const NOT_FOUND_PAGE: &str = "<!doctype html><html><body>Not found</body></html>";

/// Shared shell for the two pages a phone sees: dark, one column, thumb sized.
const PHONE_HEAD: &str = r#"<!doctype html><html lang="en"><head><meta charset="utf-8">
<meta name="viewport" content="width=device-width,initial-scale=1">
<style>
body{margin:0;min-height:100vh;display:grid;place-items:center;background:#08080b;color:#f5f5f8;
font-family:system-ui,-apple-system,"Segoe UI",sans-serif}
.card{padding:32px 24px;max-width:440px;width:100%;box-sizing:border-box}
h1{font-size:26px;font-weight:600;margin:0 0 10px}
p{color:#9c9ca8;font-size:16px;line-height:1.45;margin:0 0 18px}
.button{display:block;padding:16px;border-radius:999px;background:#1ed760;color:#08080b;
font-weight:600;font-size:18px;text-align:center;text-decoration:none}
ol{color:#9c9ca8;font-size:15px;line-height:1.5;padding-left:20px;margin:22px 0 16px}
code{color:#f5f5f8;font-size:15px}
input{width:100%;box-sizing:border-box;padding:14px;border-radius:14px;border:1px solid #2a2a33;
background:#14141a;color:#f5f5f8;font-size:15px;font-family:inherit}
button{margin-top:12px;width:100%;padding:15px;border-radius:999px;border:0;background:#1ed760;
color:#08080b;font-weight:600;font-size:16px;font-family:inherit}
</style></head><body><div class="card">"#;

const PHONE_FOOT: &str = "</div></body></html>";

/// The login page a phone opens: approve in Spotify, then paste the address
/// that Spotify redirected to.
const LOGIN_PAGE: &str = r##"<h1>Connect Spotify Round</h1>
<p>Approve access in Spotify, then hand the answer back to the display.</p>
<a class="button" href="{authorize}">Open Spotify</a>
<ol>
<li>Spotify sends this browser to <code>127.0.0.1</code>, which only exists on the display, so the
page will not load. That is expected.</li>
<li>Copy the whole address from the address bar.</li>
<li>Come back here and paste it below.</li>
</ol>
<form method="post" action="/callback">
<input name="url" type="text" inputmode="url" autocomplete="off" autocapitalize="off"
spellcheck="false" placeholder="{callback}?code=...">
<button type="submit">Connect the display</button>
</form>
<script>
const field = document.querySelector('input[name=url]');
field.addEventListener('paste', () => setTimeout(() => field.form.submit(), 50));
</script>
"##;

const PASTE_PAGE: &str = r#"<h1>That did not work</h1>
<p>{message}</p>
<a class="button" href="/">Try again</a>
"#;

fn login_page(authorize: &Url, callback: &str) -> String {
    let body = LOGIN_PAGE
        .replace("{authorize}", &escape_html(authorize.as_str()))
        .replace("{callback}", &escape_html(callback));
    format!("{PHONE_HEAD}{body}{PHONE_FOOT}")
}

fn paste_page(message: &str) -> String {
    let body = PASTE_PAGE.replace("{message}", &escape_html(message));
    format!("{PHONE_HEAD}{body}{PHONE_FOOT}")
}

fn escape_html(text: &str) -> String {
    let mut escaped = String::with_capacity(text.len());
    for ch in text.chars() {
        match ch {
            '&' => escaped.push_str("&amp;"),
            '<' => escaped.push_str("&lt;"),
            '>' => escaped.push_str("&gt;"),
            '"' => escaped.push_str("&quot;"),
            '\'' => escaped.push_str("&#39;"),
            other => escaped.push(other),
        }
    }
    escaped
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pkce_matches_the_rfc7636_example() {
        // RFC 7636, appendix B.
        assert_eq!(
            Pkce::challenge("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk"),
            "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"
        );
    }

    #[test]
    fn verifier_is_url_safe_and_long_enough() {
        let pkce = Pkce::generate();
        assert_eq!(pkce.verifier.len(), 43);
        assert!(
            pkce.verifier
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        );
    }

    #[test]
    fn authorization_url_carries_every_param() {
        let url = authorization_url(
            "client",
            "http://127.0.0.1:8888/callback",
            "challenge",
            "state",
        )
        .unwrap();
        let params: HashMap<_, _> = url.query_pairs().into_owned().collect();
        assert_eq!(params["client_id"], "client");
        assert_eq!(params["response_type"], "code");
        assert_eq!(params["code_challenge_method"], "S256");
        assert_eq!(params["code_challenge"], "challenge");
        assert_eq!(params["state"], "state");
        assert!(params["scope"].contains("user-modify-playback-state"));
    }

    #[test]
    fn parses_a_successful_callback() {
        assert_eq!(
            parse_callback("/callback?code=abc&state=xyz"),
            Callback::Code {
                code: "abc".into(),
                state: "xyz".into()
            }
        );
    }

    #[test]
    fn parses_a_denied_callback() {
        assert_eq!(
            parse_callback("/callback?error=access_denied&state=xyz"),
            Callback::Error("access_denied".into())
        );
    }

    #[test]
    fn ignores_unrelated_requests() {
        assert_eq!(parse_callback("/favicon.ico"), Callback::Unexpected);
        assert_eq!(parse_callback("/callback"), Callback::Unexpected);
    }

    #[test]
    fn parses_a_pasted_callback_address() {
        let expected = Callback::Code {
            code: "abc".into(),
            state: "xyz".into(),
        };
        assert_eq!(
            parse_pasted("http://127.0.0.1:8888/callback?code=abc&state=xyz"),
            expected
        );
        assert_eq!(
            parse_pasted("  127.0.0.1:8888/callback?code=abc&state=xyz\n"),
            expected
        );
        assert_eq!(parse_pasted("?code=abc&state=xyz"), expected);
        assert_eq!(parse_pasted("code=abc&state=xyz"), expected);
    }

    #[test]
    fn rejects_a_paste_without_a_code_and_state() {
        assert_eq!(parse_pasted("abc"), Callback::Unexpected);
        assert_eq!(parse_pasted(""), Callback::Unexpected);
        assert_eq!(parse_pasted("?code=abc"), Callback::Unexpected);
        assert_eq!(
            parse_pasted("http://127.0.0.1:8888/callback?error=access_denied"),
            Callback::Error("access_denied".into())
        );
    }

    #[test]
    fn reads_the_pasted_address_out_of_the_form_body() {
        let body = "url=http%3A%2F%2F127.0.0.1%3A8888%2Fcallback%3Fcode%3Dabc%26state%3Dxyz";
        assert_eq!(
            submitted_code(body),
            Ok(("abc".to_owned(), "xyz".to_owned()))
        );
        assert!(submitted_code("url=").is_err());
        assert!(submitted_code("other=1").is_err());
    }

    #[test]
    fn the_login_page_carries_an_escaped_authorization_url() {
        let url = authorization_url(
            "client",
            "http://127.0.0.1:8888/callback",
            "challenge",
            "state",
        )
        .unwrap();
        let page = login_page(&url, "http://127.0.0.1:8888/callback");
        assert!(
            page.contains(r#"href="https://accounts.spotify.com/authorize?client_id=client&amp;"#)
        );
        assert!(page.contains(r#"action="/callback""#));
        assert!(page.contains(r#"placeholder="http://127.0.0.1:8888/callback?code=...""#));
        assert!(!page.contains("{authorize}"));
    }

    #[tokio::test]
    async fn reads_a_form_body_that_arrives_after_the_headers() {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let address = listener.local_addr().unwrap();

        let client = tokio::spawn(async move {
            let mut stream = TcpStream::connect(address).await.unwrap();
            stream
                .write_all(b"POST /callback HTTP/1.1\r\nHost: x\r\nContent-Length: 7\r\n\r\n")
                .await
                .unwrap();
            stream.flush().await.unwrap();
            tokio::time::sleep(Duration::from_millis(20)).await;
            stream.write_all(b"url=abc").await.unwrap();
        });

        let (mut socket, _) = listener.accept().await.unwrap();
        let request = read_request(&mut socket).await.unwrap();
        assert_eq!(request.method, "POST");
        assert_eq!(request.path(), "/callback");
        assert_eq!(request.body, "url=abc");

        client.await.unwrap();
    }

    #[tokio::test]
    async fn serves_the_login_page_and_rejects_a_stale_paste() {
        let port = std::net::TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        let base = format!("http://127.0.0.1:{port}");
        let _ = rustls::crypto::ring::default_provider().install_default();
        let auth = Auth::new(
            "client".into(),
            format!("{base}/callback"),
            std::env::temp_dir().join(format!("spotify-round-auth-{port}.json")),
        )
        .unwrap();
        let flow = auth.begin_login(&base).await.unwrap();
        assert_eq!(flow.login_url().as_str(), format!("{base}/"));
        let address = format!("127.0.0.1:{port}").parse().unwrap();
        let server = tokio::spawn(async move { flow.wait().await });

        let page = exchange(address, "GET / HTTP/1.1\r\nHost: display\r\n\r\n").await;
        assert!(page.starts_with("HTTP/1.1 200 OK"), "{page}");
        assert!(page.contains("Connect Spotify Round"));
        assert!(page.contains("accounts.spotify.com/authorize"));

        let body = "url=http%3A%2F%2F127.0.0.1%3A8888%2Fcallback%3Fcode%3Dabc%26state%3Dstale";
        let request = format!(
            "POST /callback HTTP/1.1\r\nHost: display\r\n\
             Content-Type: application/x-www-form-urlencoded\r\nContent-Length: {}\r\n\r\n{body}",
            body.len()
        );
        let page = exchange(address, &request).await;
        assert!(page.starts_with("HTTP/1.1 400 Bad Request"), "{page}");
        assert!(page.contains("older login attempt"));

        server.abort();
    }

    /// Send one request and read the whole response.
    async fn exchange(address: std::net::SocketAddr, request: &str) -> String {
        let mut stream = TcpStream::connect(address).await.unwrap();
        stream.write_all(request.as_bytes()).await.unwrap();
        let mut response = String::new();
        stream.read_to_string(&mut response).await.unwrap();
        response
    }
}
