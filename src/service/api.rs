//! Typed wrapper around the subset of the Spotify Web API this display needs.

use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use reqwest::{Method, Response, StatusCode};
use serde::Deserialize;
use serde_json::json;
use url::Url;

use super::auth::Auth;

const BASE: &str = "https://api.spotify.com/v1/";

/// Errors worth reacting to differently in the UI.
#[derive(Debug, thiserror::Error)]
pub enum ApiError {
    #[error("no active Spotify device")]
    NoActiveDevice,
    #[error("Spotify Premium is required to control playback")]
    PremiumRequired,
    #[error("Spotify is rate limiting requests")]
    RateLimited(Duration),
    #[error("Spotify refused the skip: nowhere to go")]
    SkipRestricted,
    #[error("Spotify returned {status}: {message}")]
    Spotify { status: u16, message: String },
    #[error(transparent)]
    Http(#[from] reqwest::Error),
    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

/// Repeat mode as understood by both the API and the UI (0 = off, 1 = all, 2 = one).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RepeatMode {
    Off,
    Context,
    Track,
}

impl RepeatMode {
    pub fn from_state(state: &str) -> Self {
        match state {
            "track" => Self::Track,
            "context" => Self::Context,
            _ => Self::Off,
        }
    }

    pub fn next(self) -> Self {
        match self {
            Self::Off => Self::Context,
            Self::Context => Self::Track,
            Self::Track => Self::Off,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Context => "context",
            Self::Track => "track",
        }
    }

    pub fn index(self) -> i32 {
        match self {
            Self::Off => 0,
            Self::Context => 1,
            Self::Track => 2,
        }
    }
}

#[derive(Clone)]
pub struct Api {
    http: reqwest::Client,
    auth: Arc<Auth>,
    base: Url,
}

impl Api {
    pub fn new(auth: Arc<Auth>) -> Result<Self> {
        Ok(Self {
            http: auth.http().clone(),
            auth,
            base: Url::parse(BASE).expect("the API base URL is valid"),
        })
    }

    #[cfg(test)]
    fn with_base(auth: Arc<Auth>, base: Url) -> Self {
        Self {
            http: auth.http().clone(),
            auth,
            base,
        }
    }

    pub async fn playback(&self) -> Result<Option<PlaybackState>, ApiError> {
        let response = self.send(Method::GET, "me/player", None).await?;
        if response.status() == StatusCode::NO_CONTENT {
            return Ok(None);
        }
        Ok(Some(response.json().await?))
    }

    pub async fn next(&self) -> Result<(), ApiError> {
        self.send(Method::POST, "me/player/next", None).await?;
        Ok(())
    }

    pub async fn previous(&self) -> Result<(), ApiError> {
        self.send(Method::POST, "me/player/previous", None).await?;
        Ok(())
    }

    pub async fn set_playing(&self, playing: bool) -> Result<(), ApiError> {
        let path = if playing {
            "me/player/play"
        } else {
            "me/player/pause"
        };
        self.send(Method::PUT, path, None).await?;
        Ok(())
    }

    pub async fn seek(&self, position_ms: u64) -> Result<(), ApiError> {
        self.send(
            Method::PUT,
            &format!("me/player/seek?position_ms={position_ms}"),
            None,
        )
        .await?;
        Ok(())
    }

    pub async fn set_shuffle(&self, enabled: bool) -> Result<(), ApiError> {
        self.send(
            Method::PUT,
            &format!("me/player/shuffle?state={enabled}"),
            None,
        )
        .await?;
        Ok(())
    }

    pub async fn set_repeat(&self, mode: RepeatMode) -> Result<(), ApiError> {
        self.send(
            Method::PUT,
            &format!("me/player/repeat?state={}", mode.as_str()),
            None,
        )
        .await?;
        Ok(())
    }

    pub async fn play_context(&self, uri: &str) -> Result<(), ApiError> {
        self.send(
            Method::PUT,
            "me/player/play",
            Some(json!({ "context_uri": uri })),
        )
        .await?;
        Ok(())
    }

    /// Jump to an item inside the context that is already playing.
    ///
    /// Spotify has no "jump to this queue entry", and sending an explicit `uris`
    /// list leaves some clients idle, so this restarts the current context from
    /// the tapped item instead.
    pub async fn play_in_context(&self, context_uri: &str, uri: &str) -> Result<(), ApiError> {
        self.send(
            Method::PUT,
            "me/player/play",
            Some(json!({ "context_uri": context_uri, "offset": { "uri": uri } })),
        )
        .await?;
        Ok(())
    }

    pub async fn playlists(&self) -> Result<Vec<Playlist>, ApiError> {
        let page: PlaylistPage = self
            .send(Method::GET, "me/playlists?limit=50", None)
            .await?
            .json()
            .await?;
        Ok(page.items.into_iter().flatten().collect())
    }

    pub async fn queue(&self) -> Result<QueueResponse, ApiError> {
        Ok(self
            .send(Method::GET, "me/player/queue", None)
            .await?
            .json()
            .await?)
    }

    /// Every Spotify Connect device that is currently open, active or not.
    pub async fn devices(&self) -> Result<Vec<Device>, ApiError> {
        let response: DevicesResponse = self
            .send(Method::GET, "me/player/devices", None)
            .await?
            .json()
            .await?;
        Ok(response.devices)
    }

    /// Move playback to another device, leaving it playing or paused as it was.
    pub async fn transfer(&self, device_id: &str) -> Result<(), ApiError> {
        self.send(
            Method::PUT,
            "me/player",
            Some(json!({ "device_ids": [device_id] })),
        )
        .await?;
        Ok(())
    }

    async fn send(
        &self,
        method: Method,
        path: &str,
        body: Option<serde_json::Value>,
    ) -> Result<Response, ApiError> {
        let token = self.auth.access_token().await.map_err(ApiError::Other)?;
        let url = self
            .base
            .join(path)
            .map_err(|err| ApiError::Other(err.into()))?;

        let bodyless_write = body.is_none() && matches!(method, Method::POST | Method::PUT);
        let mut request = self.http.request(method, url).bearer_auth(token);
        if let Some(body) = body {
            request = request.json(&body);
        } else if bodyless_write {
            // Spotify's edge answers 411 Length Required when a write carries no
            // Content-Length, which is what a completely absent body produces.
            request = request
                .header(reqwest::header::CONTENT_LENGTH, 0)
                .body(Vec::<u8>::new());
        }

        let response = request.send().await?;
        if response.status().is_success() {
            Ok(response)
        } else {
            Err(error_from(response).await)
        }
    }
}

async fn error_from(response: Response) -> ApiError {
    let status = response.status();
    let retry_after = response
        .headers()
        .get(reqwest::header::RETRY_AFTER)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<u64>().ok())
        .map(Duration::from_secs)
        .unwrap_or(Duration::from_secs(5));

    let body: Option<ErrorBody> = response.json().await.ok();
    let reason = body.as_ref().and_then(|body| body.error.reason.clone());
    let message = body
        .map(|body| body.error.message)
        .unwrap_or_else(|| status.to_string());

    match (status, reason.as_deref()) {
        (StatusCode::NOT_FOUND, Some("NO_ACTIVE_DEVICE")) => ApiError::NoActiveDevice,
        (StatusCode::FORBIDDEN, Some("PREMIUM_REQUIRED")) => ApiError::PremiumRequired,
        // Skipping past either end of the context answers 403 with this text.
        (StatusCode::FORBIDDEN, _) if message.contains("Restriction violated") => {
            ApiError::SkipRestricted
        }
        (StatusCode::TOO_MANY_REQUESTS, _) => ApiError::RateLimited(retry_after),
        _ => ApiError::Spotify {
            status: status.as_u16(),
            message,
        },
    }
}

#[derive(Debug, Deserialize)]
struct ErrorBody {
    error: ErrorDetail,
}

#[derive(Debug, Deserialize)]
struct ErrorDetail {
    #[serde(default)]
    message: String,
    #[serde(default)]
    reason: Option<String>,
}

// ---------------------------------------------------------------------------
// Data model
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize)]
pub struct Image {
    pub url: String,
    #[serde(default)]
    pub width: Option<u32>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Artist {
    pub name: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Album {
    pub name: String,
    #[serde(default)]
    pub images: Vec<Image>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Show {
    pub name: String,
    #[serde(default)]
    pub images: Vec<Image>,
}

/// A track or a podcast episode; both are "playable items" in the player state.
#[derive(Debug, Clone, Deserialize)]
pub struct Track {
    pub name: String,
    pub uri: String,
    #[serde(default)]
    pub duration_ms: u64,
    #[serde(default)]
    pub artists: Vec<Artist>,
    #[serde(default)]
    pub album: Option<Album>,
    #[serde(default)]
    pub show: Option<Show>,
    #[serde(default)]
    pub images: Option<Vec<Image>>,
}

impl Track {
    pub fn artist_names(&self) -> String {
        let names: Vec<String> = self.artists.iter().map(|a| a.name.clone()).collect();
        let fallback = self
            .show
            .as_ref()
            .map(|show| show.name.as_str())
            .unwrap_or("Unknown artist");
        crate::format::artists(&names, fallback)
    }

    pub fn album_name(&self) -> Option<&str> {
        self.album.as_ref().map(|album| album.name.as_str())
    }

    /// "Artist · Album", falling back gracefully for podcasts and local files.
    pub fn subtitle(&self) -> String {
        match self.album_name() {
            Some(album) if !album.is_empty() => format!("{} · {}", self.artist_names(), album),
            _ => self.artist_names(),
        }
    }

    pub fn cover_url(&self, min_width: u32) -> Option<&str> {
        let album_images = self.album.as_ref().map(|album| album.images.as_slice());
        let show_images = self.show.as_ref().map(|show| show.images.as_slice());
        album_images
            .filter(|images| !images.is_empty())
            .or(self.images.as_deref())
            .or(show_images.filter(|images| !images.is_empty()))
            .and_then(|images| pick_image(images, min_width))
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct Device {
    /// Spotify's id for the device. Some devices — Google Cast endpoints among
    /// them — have none, and are then only listed, never transferred to.
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub name: String,
    /// "Computer", "Smartphone", "Speaker", "TV", "CastVideo", "CastAudio", …
    #[serde(rename = "type", default)]
    pub kind: Option<String>,
    #[serde(default)]
    pub is_active: bool,
    /// Spotify refuses transport commands on a restricted device.
    #[serde(default)]
    pub is_restricted: bool,
}

impl Device {
    /// The id `PUT /me/player` needs to move playback here, if it has one.
    pub fn connect_id(&self) -> Option<&str> {
        self.id.as_deref().filter(|id| !id.is_empty())
    }

    /// Whether the device can be picked at all: a device without an id cannot be
    /// transferred to, and a restricted one refuses commands.
    pub fn selectable(&self) -> bool {
        self.connect_id().is_some() && !self.is_restricted
    }
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct DevicesResponse {
    #[serde(default)]
    pub devices: Vec<Device>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PlaybackContext {
    #[serde(default)]
    pub uri: Option<String>,
    #[serde(default, rename = "type")]
    pub kind: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PlaybackState {
    #[serde(default)]
    pub is_playing: bool,
    #[serde(default)]
    pub progress_ms: Option<u64>,
    #[serde(default)]
    pub item: Option<Track>,
    #[serde(default)]
    pub shuffle_state: bool,
    #[serde(default)]
    pub repeat_state: String,
    #[serde(default)]
    pub device: Option<Device>,
    #[serde(default)]
    pub context: Option<PlaybackContext>,
    #[serde(default)]
    pub actions: Option<Actions>,
}

impl PlaybackState {
    pub fn progress_ms(&self) -> u64 {
        self.progress_ms.unwrap_or(0)
    }

    /// Whether Spotify allows skipping backwards, i.e. the user is not already
    /// at the start of the playlist or album.
    pub fn can_skip_previous(&self) -> bool {
        !self
            .disallowed()
            .is_some_and(|disallows| disallows.skipping_prev)
    }

    pub fn can_skip_next(&self) -> bool {
        !self
            .disallowed()
            .is_some_and(|disallows| disallows.skipping_next)
    }

    fn disallowed(&self) -> Option<&Disallows> {
        self.actions.as_ref().map(|actions| &actions.disallows)
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct Actions {
    #[serde(default)]
    pub disallows: Disallows,
}

/// Commands Spotify will refuse right now; absent keys mean "allowed".
#[derive(Debug, Clone, Default, Deserialize)]
pub struct Disallows {
    #[serde(default)]
    pub skipping_prev: bool,
    #[serde(default)]
    pub skipping_next: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PlaylistPage {
    #[serde(default)]
    pub items: Vec<Option<Playlist>>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Playlist {
    pub name: String,
    pub uri: String,
    #[serde(default)]
    pub images: Vec<Image>,
    #[serde(default)]
    pub tracks: PlaylistTracks,
    #[serde(default)]
    pub owner: Option<Owner>,
}

impl Playlist {
    pub fn cover_url(&self, min_width: u32) -> Option<&str> {
        pick_image(&self.images, min_width)
    }

    pub fn subtitle(&self) -> String {
        let count = crate::format::track_count(self.tracks.total);
        match self
            .owner
            .as_ref()
            .and_then(|owner| owner.display_name.as_deref())
        {
            Some(owner) if !owner.is_empty() => format!("{count} · {owner}"),
            _ => count,
        }
    }
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct PlaylistTracks {
    #[serde(default)]
    pub total: u32,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Owner {
    #[serde(default)]
    pub display_name: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct QueueResponse {
    #[serde(default)]
    pub currently_playing: Option<Track>,
    #[serde(default)]
    pub queue: Vec<Track>,
}

/// Pick the smallest image that is still at least `min_width` wide.
pub fn pick_image(images: &[Image], min_width: u32) -> Option<&str> {
    if images.is_empty() {
        return None;
    }
    images
        .iter()
        .filter(|image| image.width.unwrap_or(0) >= min_width)
        .min_by_key(|image| image.width.unwrap_or(u32::MAX))
        .or_else(|| images.iter().max_by_key(|image| image.width.unwrap_or(0)))
        .map(|image| image.url.as_str())
}

#[cfg(test)]
mod tests {
    use super::*;

    const PLAYER: &str = r#"{
        "is_playing": true,
        "progress_ms": 61000,
        "shuffle_state": false,
        "repeat_state": "context",
        "device": {"name": "Kitchen", "is_active": true, "volume_percent": 40},
        "context": {"uri": "spotify:playlist:abc", "type": "playlist"},
        "item": {
            "name": "Night Drive",
            "uri": "spotify:track:1",
            "duration_ms": 210000,
            "artists": [{"name": "Nova"}, {"name": "Kite"}],
            "album": {"name": "After Hours", "images": [
                {"url": "small.jpg", "width": 64},
                {"url": "large.jpg", "width": 640}
            ]}
        }
    }"#;

    #[test]
    fn decodes_the_player_state() {
        let state: PlaybackState = serde_json::from_str(PLAYER).unwrap();
        assert!(state.is_playing);
        assert_eq!(state.progress_ms(), 61_000);
        assert_eq!(state.repeat_state, "context");
        assert_eq!(
            state.context.unwrap().uri.as_deref(),
            Some("spotify:playlist:abc")
        );

        let track = state.item.unwrap();
        assert_eq!(track.artist_names(), "Nova, Kite");
        assert_eq!(track.subtitle(), "Nova, Kite · After Hours");
        assert_eq!(track.cover_url(300), Some("large.jpg"));
        assert_eq!(track.cover_url(32), Some("small.jpg"));
    }

    #[test]
    fn decodes_episodes_without_artists() {
        let episode: Track = serde_json::from_str(
            r#"{"name":"Episode 1","uri":"spotify:episode:1","duration_ms":1000,
                "show":{"name":"The Show","images":[{"url":"show.jpg","width":300}]}}"#,
        )
        .unwrap();
        assert_eq!(episode.artist_names(), "The Show");
        assert_eq!(episode.cover_url(100), Some("show.jpg"));
    }

    #[test]
    fn decodes_a_playlist_page_with_null_entries() {
        let page: PlaylistPage = serde_json::from_str(
            r#"{"items":[null,{"name":"Focus","uri":"spotify:playlist:focus",
                "images":[{"url":"focus.jpg","width":300}],
                "tracks":{"total":42},"owner":{"display_name":"Sam"}}]}"#,
        )
        .unwrap();
        let playlists: Vec<Playlist> = page.items.into_iter().flatten().collect();
        assert_eq!(playlists.len(), 1);
        assert_eq!(playlists[0].subtitle(), "42 tracks · Sam");
        assert_eq!(playlists[0].cover_url(64), Some("focus.jpg"));
    }

    /// Cast devices are reported without an id (and sometimes without a type),
    /// so a strict `String` field would reject the whole answer.
    #[test]
    fn decodes_devices_without_an_id() {
        let response: DevicesResponse = serde_json::from_str(
            r#"{"devices":[
                {"id":null,"name":"Living room TV","type":"CastVideo","is_active":false,
                 "is_restricted":true},
                {"id":null,"name":"Kitchen speaker","is_active":false},
                {"id":"abc","name":"Phone","type":"Smartphone","is_active":true}
            ]}"#,
        )
        .unwrap();

        assert_eq!(response.devices.len(), 3);
        assert!(response.devices[0].id.is_none());
        assert_eq!(response.devices[0].kind.as_deref(), Some("CastVideo"));
        assert!(response.devices[0].is_restricted);
        assert!(response.devices[1].kind.is_none());
        assert_eq!(response.devices[2].id.as_deref(), Some("abc"));
    }

    #[test]
    fn decodes_skip_restrictions() {
        let state: PlaybackState = serde_json::from_str(
            r#"{"is_playing":true,"actions":{"disallows":{"skipping_prev":true}}}"#,
        )
        .unwrap();
        assert!(!state.can_skip_previous());
        assert!(state.can_skip_next(), "absent keys mean allowed");

        let state: PlaybackState = serde_json::from_str(PLAYER).unwrap();
        assert!(state.can_skip_previous(), "no actions block at all");
        assert!(state.can_skip_next());
    }

    #[test]
    fn repeat_cycles() {
        assert_eq!(RepeatMode::Off.next(), RepeatMode::Context);
        assert_eq!(RepeatMode::Context.next(), RepeatMode::Track);
        assert_eq!(RepeatMode::Track.next(), RepeatMode::Off);
        assert_eq!(RepeatMode::from_state("track").index(), 2);
    }

    #[test]
    fn decodes_castable_devices() {
        let response: DevicesResponse = serde_json::from_str(
            r#"{"devices":[
                {"id":"kitchen","name":"Kitchen","type":"Speaker","is_active":false,
                 "volume_percent":40,"supports_volume":true},
                {"id":"phone","name":"Phone","type":"Smartphone","is_active":true}
            ]}"#,
        )
        .unwrap();

        assert_eq!(response.devices.len(), 2);
        assert_eq!(response.devices[0].connect_id(), Some("kitchen"));
        assert_eq!(response.devices[0].kind.as_deref(), Some("Speaker"));
        assert!(!response.devices[0].is_active);
        assert!(response.devices[1].is_active);
    }

    #[test]
    fn a_device_without_optional_details_still_decodes() {
        let device: Device = serde_json::from_str(r#"{"id":"tv","name":"Lounge TV"}"#).unwrap();
        assert!(device.kind.is_none());
        assert!(!device.is_active);
        assert!(!device.is_restricted);
        assert!(
            device.selectable(),
            "no restriction is reported, so it is usable"
        );
    }

    /// Api pointed at a local server, with a token that never needs refreshing.
    fn test_api(port: u16) -> (Api, std::path::PathBuf) {
        use std::sync::Arc;

        let dir = std::env::temp_dir().join(format!("spotify-round-test-{port}"));
        std::fs::create_dir_all(&dir).unwrap();
        let auth = Arc::new(
            crate::service::auth::Auth::new(
                "client".into(),
                "http://127.0.0.1:8888/callback".into(),
                dir.join("tokens.json"),
            )
            .unwrap(),
        );
        auth.finish_login(crate::service::auth::Grant {
            access_token: "test-token".into(),
            expires_in: 3600,
            refresh_token: None,
            scopes: Vec::new(),
        })
        .unwrap();

        let api = Api::with_base(
            auth,
            Url::parse(&format!("http://127.0.0.1:{port}/v1/")).unwrap(),
        );
        (api, dir)
    }

    /// Spotify answers `411 Length Required` to a write without a body length,
    /// so every transport command must be framed with `Content-Length: 0`.
    #[tokio::test]
    async fn bodyless_writes_carry_a_content_length() {
        use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
        use tokio::net::TcpListener;

        let _ = rustls::crypto::ring::default_provider().install_default();
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();

        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut buffer = vec![0u8; 4096];
            let read = socket.read(&mut buffer).await.unwrap();
            let request = String::from_utf8_lossy(&buffer[..read]).into_owned();
            socket
                .write_all(b"HTTP/1.1 204 No Content\r\nContent-Length: 0\r\n\r\n")
                .await
                .unwrap();
            request
        });

        let (api, dir) = test_api(port);
        api.next().await.unwrap();

        let request = server.await.unwrap();
        let lower = request.to_ascii_lowercase();
        assert!(
            lower.starts_with("post /v1/me/player/next"),
            "unexpected request: {request}"
        );
        assert!(
            lower.contains("content-length: 0"),
            "missing content length: {request}"
        );
        assert!(
            lower.contains("authorization: bearer test-token"),
            "missing bearer token: {request}"
        );

        let _ = std::fs::remove_dir_all(dir);
    }

    /// The picker's list comes from `/me/player/devices`.
    #[tokio::test]
    async fn lists_the_open_devices() {
        use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
        use tokio::net::TcpListener;

        let _ = rustls::crypto::ring::default_provider().install_default();
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();

        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut buffer = vec![0u8; 4096];
            let read = socket.read(&mut buffer).await.unwrap();
            let request = String::from_utf8_lossy(&buffer[..read]).into_owned();
            let body = r#"{"devices":[
                {"id":"kitchen","name":"Kitchen","type":"Speaker","is_active":true},
                {"id":"phone","name":"Phone","type":"Smartphone","is_active":false}
            ]}"#;
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}",
                body.len()
            );
            socket.write_all(response.as_bytes()).await.unwrap();
            request
        });

        let (api, dir) = test_api(port);
        let devices = api.devices().await.unwrap();

        let request = server.await.unwrap();
        assert!(
            request
                .to_ascii_lowercase()
                .starts_with("get /v1/me/player/devices "),
            "unexpected request: {request}"
        );
        assert_eq!(devices.len(), 2);
        assert_eq!(devices[0].name, "Kitchen");
        assert!(devices[0].is_active);
        assert_eq!(devices[1].kind.as_deref(), Some("Smartphone"));

        let _ = std::fs::remove_dir_all(dir);
    }

    /// Transferring names the chosen device in the body of `PUT /me/player`.
    #[tokio::test]
    async fn transferring_names_the_chosen_device() {
        use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
        use tokio::net::TcpListener;

        let _ = rustls::crypto::ring::default_provider().install_default();
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();

        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut buffer = vec![0u8; 4096];
            let mut request = Vec::new();
            // The body can arrive in a second packet, so wait for the closing
            // brace of the JSON document rather than a single read.
            loop {
                let read = socket.read(&mut buffer).await.unwrap();
                if read == 0 {
                    break;
                }
                request.extend_from_slice(&buffer[..read]);
                if request.ends_with(b"}") {
                    break;
                }
            }
            socket
                .write_all(b"HTTP/1.1 204 No Content\r\nContent-Length: 0\r\n\r\n")
                .await
                .unwrap();
            String::from_utf8_lossy(&request).into_owned()
        });

        let (api, dir) = test_api(port);
        api.transfer("kitchen").await.unwrap();

        let request = server.await.unwrap();
        let lower = request.to_ascii_lowercase();
        assert!(
            lower.starts_with("put /v1/me/player "),
            "unexpected request: {request}"
        );
        assert!(
            request.contains(r#"{"device_ids":["kitchen"]}"#),
            "the device id is missing from the body: {request}"
        );

        let _ = std::fs::remove_dir_all(dir);
    }

    /// Skipping past either end of a playlist answers 403 "Restriction
    /// violated"; the display turns that into a hint, not a failure.
    #[tokio::test]
    async fn a_restricted_skip_is_classified() {
        use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
        use tokio::net::TcpListener;

        let _ = rustls::crypto::ring::default_provider().install_default();
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();

        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut buffer = vec![0u8; 4096];
            let _ = socket.read(&mut buffer).await.unwrap();
            let body = r#"{"error":{"status":403,"message":"Player command failed: Restriction violated","reason":"UNKNOWN"}}"#;
            let response = format!(
                "HTTP/1.1 403 Forbidden\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}",
                body.len()
            );
            socket.write_all(response.as_bytes()).await.unwrap();
        });

        let (api, dir) = test_api(port);
        let err = api
            .previous()
            .await
            .expect_err("a 403 must not look like success");
        assert!(matches!(err, ApiError::SkipRestricted), "{err:?}");

        server.await.unwrap();
        let _ = std::fs::remove_dir_all(dir);
    }
}
