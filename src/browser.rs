//! Open a URL in the platform browser. Failures are not fatal: the URL is also
//! shown on the display.

use std::process::{Command, Stdio};

pub fn open(url: &str) -> bool {
    let mut command = command_for(url);
    command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map(|_| true)
        .unwrap_or(false)
}

#[cfg(target_os = "linux")]
fn command_for(url: &str) -> Command {
    let mut command = Command::new("xdg-open");
    command.arg(url);
    command
}

#[cfg(target_os = "windows")]
fn command_for(url: &str) -> Command {
    // `cmd /C start <url>` re-parses its command line and splits the URL at the
    // first `&`, silently dropping OAuth query parameters. Handing the URL to
    // the protocol handler directly keeps it intact.
    let mut command = Command::new("rundll32.exe");
    command.args(["url.dll,FileProtocolHandler", url]);
    command
}

#[cfg(target_os = "macos")]
fn command_for(url: &str) -> Command {
    let mut command = Command::new("open");
    command.arg(url);
    command
}

#[cfg(not(any(target_os = "linux", target_os = "windows", target_os = "macos")))]
fn command_for(_url: &str) -> Command {
    Command::new("true")
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_url_is_handed_over_intact() {
        let url = "https://accounts.spotify.com/authorize?client_id=x&response_type=code&state=y";
        let command = super::command_for(url);
        let args: Vec<String> = command
            .get_args()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect();
        assert!(
            args.iter().any(|arg| arg == url),
            "the URL must stay a single argument: {args:?}"
        );

        #[cfg(target_os = "windows")]
        assert_ne!(
            command.get_program().to_string_lossy().to_ascii_lowercase(),
            "cmd",
            "cmd.exe re-parses its command line and splits OAuth URLs at '&'"
        );
    }
}
