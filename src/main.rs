mod app;
mod artwork;
mod browser;
mod cli;
mod config;
mod dive3d;
mod format;
mod globe;
mod gpu;
#[cfg(test)]
mod gpu_cost;
mod motion;
mod porthole;
mod ring;
mod service;
mod ui;
mod visuals;
mod voyage;

use anyhow::{Context, Result};
use clap::Parser;
use tracing_subscriber::EnvFilter;

use crate::cli::{Cli, Command};
use crate::config::Settings;

fn main() -> Result<()> {
    let cli = Cli::parse();
    init_tracing(&cli);
    install_crypto_provider();

    let settings = Settings::load(&cli)?;
    match cli.command() {
        Command::Run => app::run(settings),
        Command::Auth { no_browser } => authorize(settings, no_browser),
    }
}

fn init_tracing(cli: &Cli) {
    let default = match cli.verbose {
        0 => "warn",
        1 => "info",
        _ => "debug",
    };
    let mut filter = cli.log.clone().unwrap_or_else(|| default.to_owned());
    // Measuring the frame rate is asked for explicitly: its report is shown
    // whatever else the log leaves out.
    if std::env::var_os("SPOTIFY_ROUND_FULL_SPEED").is_some() {
        filter.push_str(",spotify_round::ui=info,spotify_round::visuals=debug");
    }
    let filter = EnvFilter::try_new(filter).unwrap_or_else(|_| EnvFilter::new("warn"));
    let _ = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(false)
        .try_init();
}

/// reqwest's rustls backend needs a crypto provider; pick ring so the binary
/// stays easy to cross-compile.
fn install_crypto_provider() {
    if rustls::crypto::ring::default_provider()
        .install_default()
        .is_err()
    {
        tracing::debug!("a rustls crypto provider was already installed");
    }
}

/// `spotify-round auth`: run the browser flow on this machine and persist the
/// refresh token so it can be copied to the display.
fn authorize(settings: Settings, no_browser: bool) -> Result<()> {
    let client_id = settings.client_id.clone().with_context(|| {
        format!(
            "no Spotify client id configured; add `client_id` to {} or pass --client-id",
            settings.config_file().display()
        )
    })?;

    let redirect_uri = settings.redirect_uri();
    let login_base = settings.login_base();
    let token_file = settings.token_file();
    let auth = service::Auth::new(client_id, redirect_uri.clone(), token_file.clone())?;

    let runtime = tokio::runtime::Runtime::new().context("starting the async runtime")?;
    runtime.block_on(async move {
        let flow = auth.begin_login(&login_base).await?;
        let url = flow.url().to_string();
        let page = flow.login_url().to_string();

        println!("Open this URL to authorize Spotify Round:\n\n  {url}\n");
        println!(
            "On a phone or another computer, open the login page instead; it hands the \
             authorization back to this machine:\n\n  {page}\n"
        );
        if !no_browser && browser::open(&url) {
            println!("A browser window should have opened.");
        }
        println!("Waiting for the callback on {redirect_uri} ...");

        let grant = flow.wait().await?;
        auth.finish_login(grant)?;

        println!("\nConnected. Tokens saved to {}", token_file.display());
        println!("Copy that file to the display to reuse this login.");
        Ok(())
    })
}
