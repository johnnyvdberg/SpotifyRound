//! In-memory demo backend used by `--mock`.
//!
//! It implements the same surface as the live API so the entire UI, including
//! artwork (procedurally generated covers) and controls, can be exercised
//! without Spotify credentials or network access.

use std::sync::Mutex;
use std::time::Instant;

use anyhow::Result;

use super::api::{
    Actions, Album, Artist, Device, Disallows, Image, PlaybackContext, PlaybackState, Playlist,
    PlaylistTracks, QueueResponse, RepeatMode, Track,
};

pub struct Mock {
    state: std::sync::Arc<Mutex<State>>,
}

impl Clone for Mock {
    fn clone(&self) -> Self {
        Self {
            state: self.state.clone(),
        }
    }
}

struct State {
    playlists: Vec<DemoPlaylist>,
    playlist: usize,
    track: usize,
    device: usize,
    playing: bool,
    base_ms: u64,
    started: Instant,
    shuffle: bool,
    repeat: RepeatMode,
}

/// A Spotify Connect device the demo playback can be moved to.
struct DemoDevice {
    /// `None` mirrors a device Spotify lists without an id, such as a Chromecast
    /// that is not wired into Spotify Connect: visible, not pickable.
    id: Option<&'static str>,
    name: &'static str,
    kind: &'static str,
}

struct DemoPlaylist {
    name: &'static str,
    owner: &'static str,
    hue: u16,
    tracks: Vec<DemoTrack>,
}

struct DemoTrack {
    title: &'static str,
    artist: &'static str,
    album: &'static str,
    seconds: u64,
    hue: u16,
}

impl Mock {
    pub fn new() -> Self {
        let playlists = LIBRARY
            .iter()
            .map(|(name, owner, hue, tracks)| DemoPlaylist {
                name,
                owner,
                hue: *hue,
                tracks: tracks
                    .iter()
                    .enumerate()
                    .map(|(index, (title, artist, album, seconds))| DemoTrack {
                        title,
                        artist,
                        album,
                        seconds: *seconds,
                        hue: (hue + 17 * index as u16) % 360,
                    })
                    .collect(),
            })
            .collect();

        Self {
            state: std::sync::Arc::new(Mutex::new(State {
                playlists,
                playlist: 0,
                track: 0,
                device: 0,
                playing: true,
                base_ms: 0,
                started: Instant::now(),
                shuffle: false,
                repeat: RepeatMode::Off,
            })),
        }
    }

    pub fn playback(&self) -> Result<Option<PlaybackState>> {
        let mut state = self.lock();
        state.advance_if_finished();
        Ok(Some(state.snapshot()))
    }

    pub fn next(&self) -> Result<()> {
        let mut state = self.lock();
        state.skip(1);
        Ok(())
    }

    pub fn previous(&self) -> Result<()> {
        let mut state = self.lock();
        // Mirrors Spotify: a restart of the current track when it is still early.
        if state.position_ms() > 3_000 {
            state.jump_to_current();
        } else {
            state.skip(-1);
        }
        Ok(())
    }

    pub fn set_playing(&self, playing: bool) -> Result<()> {
        let mut state = self.lock();
        state.set_playing(playing);
        Ok(())
    }

    pub fn seek(&self, position_ms: u64) -> Result<()> {
        let mut state = self.lock();
        state.base_ms = position_ms.min(state.duration_ms().saturating_sub(1));
        state.started = Instant::now();
        Ok(())
    }

    pub fn set_shuffle(&self, enabled: bool) -> Result<()> {
        self.lock().shuffle = enabled;
        Ok(())
    }

    pub fn set_repeat(&self, mode: RepeatMode) -> Result<()> {
        self.lock().repeat = mode;
        Ok(())
    }

    pub fn play_context(&self, uri: &str) -> Result<()> {
        let mut state = self.lock();
        if let Some(index) =
            (0..state.playlists.len()).find(|index| state.playlist_uri(*index) == uri)
        {
            state.playlist = index;
            state.track = 0;
            state.base_ms = 0;
            state.started = Instant::now();
            state.playing = true;
        }
        Ok(())
    }

    pub fn play_in_context(&self, _context: &str, uri: &str) -> Result<()> {
        let mut state = self.lock();
        if let Some((playlist, track)) = state.find_uri(uri) {
            state.playlist = playlist;
            state.track = track;
            state.base_ms = 0;
            state.started = Instant::now();
            state.playing = true;
        }
        Ok(())
    }

    pub fn playlists(&self) -> Result<Vec<Playlist>> {
        let state = self.lock();
        Ok((0..state.playlists.len())
            .map(|index| state.api_playlist(index))
            .collect())
    }

    pub fn queue(&self) -> Result<QueueResponse> {
        let mut state = self.lock();
        state.advance_if_finished();
        Ok(state.queue_snapshot())
    }

    pub fn devices(&self) -> Result<Vec<Device>> {
        Ok(self.lock().device_snapshot())
    }

    pub fn transfer(&self, device_id: &str) -> Result<()> {
        self.lock().set_device(device_id);
        Ok(())
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, State> {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

impl Default for Mock {
    fn default() -> Self {
        Self::new()
    }
}

impl State {
    fn playlist(&self) -> &DemoPlaylist {
        &self.playlists[self.playlist]
    }

    fn current(&self) -> &DemoTrack {
        &self.playlist().tracks[self.track]
    }

    fn duration_ms(&self) -> u64 {
        self.current().seconds * 1000
    }

    fn position_ms(&self) -> u64 {
        let elapsed = if self.playing {
            self.started.elapsed().as_millis() as u64
        } else {
            0
        };
        (self.base_ms + elapsed).min(self.duration_ms().saturating_sub(1))
    }

    fn set_playing(&mut self, playing: bool) {
        if playing == self.playing {
            return;
        }
        self.base_ms = self.position_ms();
        self.started = Instant::now();
        self.playing = playing;
    }

    fn jump_to_current(&mut self) {
        self.base_ms = 0;
        self.started = Instant::now();
    }

    fn skip(&mut self, delta: isize) {
        let count = self.playlist().tracks.len() as isize;
        let next = (self.track as isize + delta).rem_euclid(count);
        self.track = next as usize;
        self.jump_to_current();
    }

    fn advance_if_finished(&mut self) {
        if self.playing
            && self.started.elapsed().as_millis() as u64 + self.base_ms >= self.duration_ms()
        {
            self.skip(1);
        }
    }

    fn device_snapshot(&self) -> Vec<Device> {
        DEVICES
            .iter()
            .enumerate()
            .map(|(index, device)| Device {
                id: device.id.map(str::to_owned),
                name: device.name.to_owned(),
                kind: Some(device.kind.to_owned()),
                is_active: index == self.device,
                is_restricted: device.id.is_none(),
            })
            .collect()
    }

    fn set_device(&mut self, device_id: &str) {
        if let Some(index) = DEVICES
            .iter()
            .position(|device| device.id == Some(device_id))
        {
            self.device = index;
        }
    }

    fn snapshot(&self) -> PlaybackState {
        let device = &DEVICES[self.device];
        PlaybackState {
            is_playing: self.playing,
            progress_ms: Some(self.position_ms()),
            item: Some(self.api_track(self.playlist, self.track)),
            shuffle_state: self.shuffle,
            repeat_state: self.repeat.as_str().to_owned(),
            device: Some(Device {
                id: device.id.map(str::to_owned),
                name: device.name.to_owned(),
                kind: Some(device.kind.to_owned()),
                is_active: true,
                is_restricted: device.id.is_none(),
            }),
            context: Some(PlaybackContext {
                uri: Some(self.playlist_uri(self.playlist)),
                kind: Some("playlist".to_owned()),
            }),
            // Mirror Spotify: skipping past either end of the playlist is refused.
            actions: Some(Actions {
                disallows: Disallows {
                    skipping_prev: self.track == 0,
                    skipping_next: self.track + 1 >= self.playlist().tracks.len(),
                },
            }),
        }
    }

    fn queue_snapshot(&self) -> QueueResponse {
        let playlist = self.playlist();
        let queue = (1..12)
            .map(|offset| {
                let index = (self.track + offset) % playlist.tracks.len();
                self.api_track(self.playlist, index)
            })
            .collect();
        QueueResponse {
            currently_playing: Some(self.api_track(self.playlist, self.track)),
            queue,
        }
    }

    /// Find the playlist and track index behind a `spotify:track:` uri.
    fn find_uri(&self, uri: &str) -> Option<(usize, usize)> {
        (0..self.playlists.len()).find_map(|playlist| {
            let demo = &self.playlists[playlist];
            (0..demo.tracks.len())
                .find(|index| format!("spotify:track:{}-{index}", slug(demo.name)) == uri)
                .map(|index| (playlist, index))
        })
    }

    fn api_track(&self, playlist: usize, index: usize) -> Track {
        let demo = &self.playlists[playlist];
        let track = &demo.tracks[index];
        Track {
            name: track.title.to_owned(),
            uri: format!("spotify:track:{}-{index}", slug(demo.name)),
            duration_ms: track.seconds * 1000,
            artists: vec![Artist {
                name: track.artist.to_owned(),
            }],
            album: Some(Album {
                name: track.album.to_owned(),
                images: vec![cover_image(track.hue, index)],
            }),
            show: None,
            images: None,
        }
    }

    fn playlist_uri(&self, index: usize) -> String {
        format!("spotify:playlist:demo-{index}")
    }

    fn api_playlist(&self, index: usize) -> Playlist {
        let demo = &self.playlists[index];
        Playlist {
            name: demo.name.to_owned(),
            uri: self.playlist_uri(index),
            images: vec![cover_image(demo.hue, 99)],
            tracks: PlaylistTracks {
                total: demo.tracks.len() as u32,
            },
            owner: Some(super::api::Owner {
                display_name: Some(demo.owner.to_owned()),
            }),
        }
    }
}

fn cover_image(hue: u16, seed: usize) -> Image {
    Image {
        url: format!("mock://cover/{hue}/{seed}"),
        width: Some(300),
    }
}

fn slug(name: &str) -> String {
    name.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect()
}

/// The devices the picker offers in demo mode; the first one starts active.
///
/// The Chromecast carries no id, like the Cast endpoints Spotify reports for
/// devices it can list but not control, so the picker's "unavailable" state is
/// part of the demo too.
const DEVICES: &[DemoDevice] = &[
    DemoDevice {
        id: Some("demo-speaker"),
        name: "Demo speaker",
        kind: "Speaker",
    },
    DemoDevice {
        id: Some("demo-laptop"),
        name: "Kitchen laptop",
        kind: "Computer",
    },
    DemoDevice {
        id: Some("demo-phone"),
        name: "Phone",
        kind: "Smartphone",
    },
    DemoDevice {
        id: Some("demo-tv"),
        name: "Living room TV",
        kind: "TV",
    },
    DemoDevice {
        id: None,
        name: "Chromecast",
        kind: "CastVideo",
    },
];

type DemoPlaylistSpec = (
    &'static str,
    &'static str,
    u16,
    &'static [(&'static str, &'static str, &'static str, u64)],
);

const LIBRARY: &[DemoPlaylistSpec] = &[
    (
        "Late Night Drive",
        "Mira Vale",
        258,
        &[
            ("Neon Coast", "Mira Vale", "Afterglow", 214),
            ("Midnight Static", "Mira Vale", "Afterglow", 198),
            ("Chrome Skyline", "Kite Season", "Overpass", 241),
            ("Signal Bloom", "Kite Season", "Overpass", 187),
            ("Last Exit", "Mira Vale", "Afterglow", 226),
            ("Tail Lights", "Night Archive", "Slow Lane", 259),
        ],
    ),
    (
        "Focus Flow",
        "Ambient Room",
        196,
        &[
            ("Paper Light", "Ambient Room", "Soft Focus", 312),
            ("Slow Tides", "Ambient Room", "Soft Focus", 288),
            ("Glass Hours", "Nils Arden", "Long Form", 341),
            ("Quiet Engine", "Nils Arden", "Long Form", 274),
            ("North Window", "Ambient Room", "Soft Focus", 302),
            ("Soft Current", "Nils Arden", "Long Form", 256),
        ],
    ),
    (
        "Sunday Kitchen",
        "Hazel & Co",
        32,
        &[
            ("Coffee Steam", "Hazel & Co", "Second Helping", 203),
            ("Window Seat", "Hazel & Co", "Second Helping", 221),
            ("Butter & Brass", "The Blue Room", "Table for Two", 245),
            ("Peach Radio", "The Blue Room", "Table for Two", 189),
            ("Afternoon Blue", "Hazel & Co", "Second Helping", 232),
            ("Second Helping", "The Blue Room", "Table for Two", 268),
        ],
    ),
    (
        "Forest Walk",
        "Field Notes",
        122,
        &[
            ("Pine Needles", "Field Notes", "Understory", 236),
            ("Riverbend", "Field Notes", "Understory", 254),
            ("Moss & Stone", "Alden Frost", "Trailhead", 198),
            ("Golden Hour Trail", "Alden Frost", "Trailhead", 227),
            ("Fern", "Field Notes", "Understory", 181),
            ("Blackbird Morning", "Alden Frost", "Trailhead", 264),
        ],
    ),
    (
        "Deep Work",
        "Index",
        214,
        &[
            ("Cold Start", "Index", "Zero Draft", 292),
            ("Long Division", "Index", "Zero Draft", 305),
            ("Tunnel Vision", "Static Method", "Blueprint", 278),
            ("Signal Path", "Static Method", "Blueprint", 261),
            ("Zero Draft", "Index", "Zero Draft", 318),
            ("Quiet Compiler", "Static Method", "Blueprint", 284),
        ],
    ),
    (
        "Golden Hour",
        "June Avery",
        14,
        &[
            ("Sunroom", "June Avery", "Porch Light", 208),
            ("Tangerine", "June Avery", "Porch Light", 194),
            ("Slow Dance", "The Long Way", "Honeyline", 233),
            ("Porch Light", "The Long Way", "Honeyline", 219),
            ("Honeyline", "June Avery", "Porch Light", 246),
            ("Goodnight, June", "The Long Way", "Honeyline", 271),
        ],
    ),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn starts_on_the_first_track_of_the_first_playlist() {
        let mock = Mock::new();
        let state = mock.playback().unwrap().unwrap();
        assert_eq!(
            state.item.as_ref().unwrap().name,
            LIBRARY[0].3[0].0,
            "demo playback should start at the top of the library"
        );
        assert!(state.is_playing);
    }

    #[test]
    fn starts_playing_and_advances() {
        let mock = Mock::new();
        let first = mock.playback().unwrap().unwrap().item.unwrap().uri;

        mock.next().unwrap();
        let second = mock.playback().unwrap().unwrap().item.unwrap().uri;
        assert_ne!(first, second);

        mock.previous().unwrap();
        let back = mock.playback().unwrap().unwrap().item.unwrap().uri;
        assert_eq!(first, back);
    }

    #[test]
    fn switches_playlists_by_uri() {
        let mock = Mock::new();
        let target = mock.playlists().unwrap()[2].uri.clone();
        mock.play_context(&target).unwrap();
        let state = mock.playback().unwrap().unwrap();
        assert_eq!(state.context.unwrap().uri.as_deref(), Some(target.as_str()));
    }

    #[test]
    fn pause_freezes_progress() {
        let mock = Mock::new();
        mock.seek(30_000).unwrap();
        mock.set_playing(false).unwrap();
        let a = mock.playback().unwrap().unwrap().progress_ms();
        std::thread::sleep(std::time::Duration::from_millis(20));
        let b = mock.playback().unwrap().unwrap().progress_ms();
        assert_eq!(a, b);
    }

    #[test]
    fn queue_has_upcoming_tracks() {
        let mock = Mock::new();
        let queue = mock.queue().unwrap();
        assert!(queue.currently_playing.is_some());
        assert_eq!(queue.queue.len(), 11);
    }

    #[test]
    fn lists_devices_with_one_active() {
        let mock = Mock::new();
        let devices = mock.devices().unwrap();
        assert!(devices.len() > 1);
        assert_eq!(
            devices.iter().filter(|device| device.is_active).count(),
            1,
            "exactly one device is playing"
        );
        assert_eq!(devices[0].name, "Demo speaker");
    }

    #[test]
    fn a_cast_device_is_listed_without_an_id() {
        let mock = Mock::new();
        let devices = mock.devices().unwrap();
        let cast = devices
            .iter()
            .find(|device| device.kind.as_deref() == Some("CastVideo"))
            .expect("the demo library offers a Chromecast");
        assert!(cast.connect_id().is_none());
        assert!(cast.is_restricted);
        assert!(!cast.selectable(), "it cannot be transferred to");
    }

    #[test]
    fn transfer_moves_the_active_device() {
        let mock = Mock::new();
        mock.transfer("demo-tv").unwrap();

        let state = mock.playback().unwrap().unwrap();
        assert_eq!(state.device.as_ref().unwrap().name, "Living room TV");
        assert_eq!(state.device.as_ref().unwrap().kind.as_deref(), Some("TV"));

        let devices = mock.devices().unwrap();
        assert!(!devices[0].is_active);
        assert!(devices[3].is_active);
    }
}
