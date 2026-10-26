//! Music backend abstraction: Spotify's Web API, or the in-memory demo library.

mod api;
mod auth;
mod mock;

pub use api::{Api, ApiError, Device, PlaybackState, Playlist, QueueResponse, RepeatMode, Track};
pub use auth::Auth;
pub use mock::Mock;

use anyhow::Result;

/// Where playback state and the library come from.
#[derive(Clone)]
pub enum Source {
    Live(Api),
    Mock(Mock),
}

impl Source {
    pub async fn playback(&self) -> Result<Option<PlaybackState>> {
        match self {
            Self::Live(api) => api.playback().await.map_err(Into::into),
            Self::Mock(mock) => mock.playback(),
        }
    }

    pub async fn next(&self) -> Result<()> {
        match self {
            Self::Live(api) => api.next().await.map_err(Into::into),
            Self::Mock(mock) => mock.next(),
        }
    }

    pub async fn previous(&self) -> Result<()> {
        match self {
            Self::Live(api) => api.previous().await.map_err(Into::into),
            Self::Mock(mock) => mock.previous(),
        }
    }

    pub async fn set_playing(&self, playing: bool) -> Result<()> {
        match self {
            Self::Live(api) => api.set_playing(playing).await.map_err(Into::into),
            Self::Mock(mock) => mock.set_playing(playing),
        }
    }

    pub async fn seek(&self, position_ms: u64) -> Result<()> {
        match self {
            Self::Live(api) => api.seek(position_ms).await.map_err(Into::into),
            Self::Mock(mock) => mock.seek(position_ms),
        }
    }

    pub async fn set_shuffle(&self, enabled: bool) -> Result<()> {
        match self {
            Self::Live(api) => api.set_shuffle(enabled).await.map_err(Into::into),
            Self::Mock(mock) => mock.set_shuffle(enabled),
        }
    }

    pub async fn set_repeat(&self, mode: RepeatMode) -> Result<()> {
        match self {
            Self::Live(api) => api.set_repeat(mode).await.map_err(Into::into),
            Self::Mock(mock) => mock.set_repeat(mode),
        }
    }

    pub async fn play_context(&self, uri: &str) -> Result<()> {
        match self {
            Self::Live(api) => api.play_context(uri).await.map_err(Into::into),
            Self::Mock(mock) => mock.play_context(uri),
        }
    }

    pub async fn play_in_context(&self, context_uri: &str, uri: &str) -> Result<()> {
        match self {
            Self::Live(api) => api
                .play_in_context(context_uri, uri)
                .await
                .map_err(Into::into),
            Self::Mock(mock) => mock.play_in_context(context_uri, uri),
        }
    }

    pub async fn playlists(&self) -> Result<Vec<Playlist>> {
        match self {
            Self::Live(api) => api.playlists().await.map_err(Into::into),
            Self::Mock(mock) => mock.playlists(),
        }
    }

    pub async fn queue(&self) -> Result<QueueResponse> {
        match self {
            Self::Live(api) => api.queue().await.map_err(Into::into),
            Self::Mock(mock) => mock.queue(),
        }
    }

    pub async fn devices(&self) -> Result<Vec<Device>> {
        match self {
            Self::Live(api) => api.devices().await.map_err(Into::into),
            Self::Mock(mock) => mock.devices(),
        }
    }

    pub async fn transfer(&self, device_id: &str) -> Result<()> {
        match self {
            Self::Live(api) => api.transfer(device_id).await.map_err(Into::into),
            Self::Mock(mock) => mock.transfer(device_id),
        }
    }
}
