use std::path::PathBuf;

use clap::{ArgAction, Parser, Subcommand};

/// A round Spotify now-playing display for the Raspberry Pi.
#[derive(Debug, Parser)]
#[command(name = "spotify-round", version, about, long_about = None)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<Command>,

    /// Use the built-in demo library instead of Spotify
    #[arg(long, global = true)]
    pub mock: bool,

    /// Run in a window instead of full screen
    #[arg(long, global = true)]
    pub windowed: bool,

    /// Directory holding config.toml and tokens.json
    #[arg(long, global = true, value_name = "DIR")]
    pub config_dir: Option<PathBuf>,

    /// Spotify application client id
    #[arg(
        long,
        global = true,
        value_name = "ID",
        env = "SPOTIFY_ROUND_CLIENT_ID"
    )]
    pub client_id: Option<String>,

    /// Log filter, for example `info` or `spotify_round=debug`
    #[arg(long, global = true, value_name = "FILTER", env = "SPOTIFY_ROUND_LOG")]
    pub log: Option<String>,

    /// Increase log verbosity (repeatable)
    #[arg(short, long, global = true, action = ArgAction::Count)]
    pub verbose: u8,
}

#[derive(Debug, Clone, Subcommand)]
pub enum Command {
    /// Show the now-playing display (default)
    Run,

    /// Authorize a Spotify account and store the refresh token
    Auth {
        /// Print the authorization URL instead of opening a browser
        #[arg(long)]
        no_browser: bool,
    },
}

impl Cli {
    pub fn command(&self) -> Command {
        self.command.clone().unwrap_or(Command::Run)
    }
}
