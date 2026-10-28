//! Application controller: owns the backend, the polling loop and the mapping
//! from domain data to the Slint view state.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use slint::{ComponentHandle as _, Timer, TimerMode};
use tokio::runtime::Handle;
use tokio::sync::Notify;

use crate::artwork::{Art, Artwork};
use crate::browser;
use crate::config::Settings;
use crate::format;
use crate::gpu;
use crate::motion::Motion;
use crate::ring::{self, Ring};
use crate::service::{
    Api, ApiError, Auth, Device, Mock, PlaybackState, Playlist, QueueResponse, RepeatMode, Source,
    Track,
};
use crate::ui::{self, App, AppWindow, DeviceItem, Idle, PlaylistItem, QueueItem, WindowExt};
use crate::visuals::{self, Scene};

/// Screens are laid out left to right: queue, now playing, playlists.
const SCREEN_QUEUE: i32 = 0;
const SCREEN_NOW: i32 = 1;
const SCREEN_PLAYLISTS: i32 = 2;

const AUTH_NEEDED: i32 = 0;
const AUTH_WAITING: i32 = 1;
const AUTH_READY: i32 = 2;
const AUTH_NO_CLIENT_ID: i32 = 3;
const AUTH_FAILED: i32 = 4;

const ART_SIZE: u32 = 480;
const PLAYLIST_ART_SIZE: u32 = 160;
const QUEUE_ART_SIZE: u32 = 112;

// Corner radii baked into the decoded artwork; they match the display sizes so
// the softening looks the same on the big cover and on the small rows.
pub(crate) const ART_CORNER: u32 = 34;
const THUMB_CORNER: u32 = 21;
const QUEUE_CORNER: u32 = 15;

/// Shown in the picker when Spotify lists nothing at all. A Chromecast or Google
/// Home only registers with Spotify while its Spotify app is running, so it is
/// missing from the list until playback is started on it.
const NO_DEVICES: &str = "No Spotify Connect devices. Chromecast and Google Home only appear while Spotify is playing \
     on them — start it from the Spotify app, then refresh.";

const LIBRARY_TTL: Duration = Duration::from_secs(300);
const QUEUE_TTL: Duration = Duration::from_secs(8);
/// Devices come and go as phones and speakers come online, so the list is
/// re-fetched whenever the picker is opened after this long.
const DEVICES_TTL: Duration = Duration::from_secs(10);
/// The panel refreshes at 60 Hz. The tick timer, the globe renderer and the
/// progress ring all follow it, so nothing is ever asked for faster than the
/// display can show it.
pub const TARGET_FPS: u64 = 60;
/// One frame at the target rate, in nanoseconds. Milliseconds would make the
/// timer 62.5 Hz, which beats against the panel and drops a frame every few
/// seconds.
const TICK: Duration = Duration::from_nanos(1_000_000_000 / TARGET_FPS);
/// A poll that disagrees with the interpolated position by more than this is a
/// real seek or a long stall instead of network latency.
const POSITION_DRIFT_MS: u64 = 900;
/// A loaded but paused track: the idle visualizer waits this long in case
/// playback resumes.
const PAUSED_IDLE: Duration = Duration::from_secs(30);
/// Nothing to play at all: the visualizer takes over almost straight away, but
/// not so fast that the "start playback" hint is never seen.
const STOPPED_IDLE: Duration = Duration::from_secs(3);
/// How long the visualizer takes to fade in or out, in seconds.
const IDLE_FADE: f32 = 0.5;
/// How long the progress ring takes to fade in or out, in seconds.
const RING_FADE: f32 = 0.4;
/// How long the ambient glow takes to change strength, in seconds, and how
/// strong it is behind the login screens.
const GLOW_FADE: f32 = 0.9;
const GLOW_DIM: f32 = 0.6;

/// How many earlier items `previous` can walk back through.
const HISTORY_DEPTH: usize = 16;
/// Further into a track than this, Spotify restarts it instead of skipping back.
const PREVIOUS_RESTART_MS: u64 = 3_000;

pub fn run(settings: Settings) -> Result<()> {
    let graphics = gpu::install()?;
    let window = ui::create_window(settings.fullscreen, settings.show_fps)?;
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .context("starting the async runtime")?;

    let shared = Arc::new(Shared::new(
        window.as_weak(),
        settings,
        runtime.handle().clone(),
        graphics.clone(),
    )?);
    shared.bind(&window);
    visuals::spawn(
        window.as_weak(),
        graphics,
        shared.visualizer_active.clone(),
        shared.idle_scene.clone(),
        shared.idle_scenes.clone(),
        shared.idle_travel.clone(),
    );

    runtime.block_on(shared.bootstrap());

    // Timers belong to the thread that runs the Slint event loop.
    let ticker = Timer::default();
    {
        let shared = shared.clone();
        ticker.start(TimerMode::Repeated, TICK, move || shared.tick());
    }

    window.run()?;
    drop(ticker);
    Ok(())
}

// ---------------------------------------------------------------------------
// Shared state
// ---------------------------------------------------------------------------

struct Shared {
    ui: slint::Weak<AppWindow>,
    settings: Settings,
    rt: Handle,
    source: Option<Source>,
    auth: Option<Arc<Auth>>,
    artwork: Artwork,
    state: Mutex<Model>,
    motion: Mutex<Motion>,
    last_tick: Mutex<Instant>,
    poke: Notify,
    /// The rim, drawn on the shared GPU device and composited by Slint.
    ring: Ring,
    /// Round-robin over the ring's render targets.
    ring_frame: AtomicUsize,
    /// Whether the visualizer renderer has anything to do.
    visualizer_active: Arc<AtomicBool>,
    /// The scene the controller wants on screen.
    idle_scene: Arc<AtomicUsize>,
    /// Bit mask of the scenes whose renderer came up.
    idle_scenes: Arc<AtomicU64>,
    /// Swipes on the visualizer, +1 on and -1 back, summed.
    idle_travel: Arc<AtomicI64>,

    initial_auth: i32,
    poll_started: AtomicBool,
    auth_running: AtomicBool,
    playlists_busy: AtomicBool,
    queue_busy: AtomicBool,
    devices_busy: AtomicBool,
    covers: AtomicU64,
    playlists_generation: Arc<AtomicU64>,
    queue_generation: Arc<AtomicU64>,
    status_generation: Arc<AtomicU64>,
}

struct Model {
    playback: Option<PlaybackState>,
    clock: Clock,
    pending: Pending,
    /// Items that were on screen before the current one, most recent last.
    history: Vec<Track>,
    /// Stand-in shown while Spotify has not said which track it picked.
    placeholder: Option<Placeholder>,
    /// Artwork that is already on screen, so the next cover can swap in instead
    /// of fading up from nothing.
    art_visible: bool,
    /// Whether Spotify allows skipping in each direction from here.
    can_previous: bool,
    can_next: bool,
    /// A skip Spotify refused, kept so the button stays dim on this item even if
    /// a poll does not report the restriction itself.
    refused_skip: Option<(Skip, String)>,
    playlists: Vec<Playlist>,
    playlists_loaded: Option<Instant>,
    queue: Option<QueueResponse>,
    queue_loaded: Option<Instant>,
    /// Spotify Connect devices the picker offers, the active one included.
    devices: Vec<Device>,
    devices_loaded: Option<Instant>,
    devices_open: bool,
    screen: i32,
    /// What the rim is currently showing, so it is only redrawn when that
    /// changes. A paused track repeats the same input and costs nothing.
    ring_input: Option<ring::Input>,
    /// Live position while the user drags along the rim, in 0..1.
    scrub: Option<f32>,
    /// Bookkeeping for the idle visualizer.
    idle: IdleState,
}

/// Tracks when the display should fall back to the idle visualizer.
#[derive(Default)]
struct IdleState {
    /// When a loaded track stopped playing.
    paused_since: Option<Instant>,
    /// When there was nothing to play at all.
    stopped_since: Option<Instant>,
    /// Opened by the downward swipe.
    manual: bool,
    /// Closed by a tap; sticks until playback resumes.
    dismissed: bool,
    /// 0..1 cross-fade.
    fade: f32,
    /// Whether the visualizer was showing on the previous tick. Its rising edge
    /// picks a new scene at random.
    showing: bool,
    /// Whether playback was running at the last state change, so a poll that
    /// repeats the same state does not undo a manual summon.
    playing: bool,
}

impl Model {
    fn new() -> Self {
        Self {
            playback: None,
            clock: Clock::new(),
            pending: Pending::default(),
            history: Vec::new(),
            placeholder: None,
            art_visible: false,
            can_previous: false,
            can_next: false,
            refused_skip: None,
            playlists: Vec::new(),
            playlists_loaded: None,
            queue: None,
            queue_loaded: None,
            devices: Vec::new(),
            devices_loaded: None,
            devices_open: false,
            screen: SCREEN_NOW,
            ring_input: None,
            scrub: None,
            idle: IdleState::default(),
        }
    }

    /// Restarts the idle countdown when playback changes state.
    ///
    /// Resuming always wins: it clears both the countdown and the "dismissed"
    /// latch, so the visualizer is back in play the next time things stop.
    fn note_playback(&mut self, playing: bool, has_track: bool, now: Instant) {
        let resumed = playing && !self.idle.playing;
        self.idle.playing = playing;
        if playing {
            self.idle.paused_since = None;
            self.idle.stopped_since = None;
            // Only a real resume closes a visualizer the user summoned by hand.
            // The poll after a swipe repeats the playing state; clearing the
            // flags then would make the visuals vanish a second after opening.
            if resumed {
                self.idle.manual = false;
                self.idle.dismissed = false;
            }
            return;
        }
        if has_track {
            self.idle.stopped_since = None;
            self.idle.paused_since.get_or_insert(now);
        } else {
            self.idle.paused_since = None;
            self.idle.stopped_since.get_or_insert(now);
        }
    }

    /// Whether the idle visualizer belongs on screen right now.
    fn wants_visualizer(&self, now: Instant) -> bool {
        if self.idle.manual || Scene::forced().is_some() {
            return true;
        }
        if self.idle.dismissed {
            return false;
        }
        if let Some(since) = self.idle.stopped_since {
            return now.saturating_duration_since(since) >= STOPPED_IDLE;
        }
        if let Some(since) = self.idle.paused_since {
            return now.saturating_duration_since(since) >= PAUSED_IDLE;
        }
        false
    }

    /// Takes a polled state, letting a local command win while Spotify has not
    /// reported it back yet, and re-bases the interpolation clock on it.
    ///
    /// A poll that was already in flight when the user pressed play/pause or
    /// skipped a track used to undo the change, which made the view flicker.
    fn store_polled(&mut self, mut state: PlaybackState) -> PlaybackState {
        let (playing, shuffle, repeat, trusted) = self.pending.resolve(&state, Instant::now());
        state.is_playing = playing;
        state.shuffle_state = shuffle;
        state.repeat_state = repeat.as_str().to_owned();

        let mut trust_position = trusted;
        if self
            .pending
            .keeps_local_track(state.item.as_ref().map(|item| item.uri.as_str()))
        {
            state.item = self
                .playback
                .as_ref()
                .and_then(|playback| playback.item.clone());
            trust_position = false;
        } else {
            self.can_previous = state.can_skip_previous();
            self.can_next = state.can_skip_next();
            let polled = state.item.as_ref().map(|item| item.uri.as_str());
            if self
                .refused_skip
                .as_ref()
                .is_some_and(|(_, uri)| polled != Some(uri.as_str()))
            {
                self.refused_skip = None;
            }
        }
        if self.pending.track.is_none() {
            self.placeholder = None;
        }

        self.clock.apply(&state, trust_position);
        state
    }

    /// What the track area should show right now.
    fn now_playing(&self) -> NowPlayingView {
        if let Some(placeholder) = self.placeholder.as_ref() {
            return NowPlayingView {
                title: placeholder.title.clone(),
                subtitle: placeholder.subtitle.clone(),
                has_track: true,
                cover_url: placeholder.cover_url.clone(),
                playlist_uri: placeholder.playlist_uri.clone(),
            };
        }

        let item = self
            .playback
            .as_ref()
            .and_then(|playback| playback.item.as_ref());
        NowPlayingView {
            title: item
                .map(|item| item.name.clone())
                .unwrap_or_else(|| "Nothing playing".to_owned()),
            subtitle: item
                .map(Track::subtitle)
                .unwrap_or_else(|| "Start playback on a Spotify device".to_owned()),
            has_track: item.is_some(),
            cover_url: item
                .and_then(|item| item.cover_url(ART_SIZE))
                .map(str::to_owned),
            playlist_uri: self.playlist_uri(),
        }
    }

    fn playlist_uri(&self) -> String {
        match self
            .playback
            .as_ref()
            .and_then(|playback| playback.context.as_ref())
        {
            Some(context) if context.kind.as_deref() == Some("playlist") => {
                context.uri.clone().unwrap_or_default()
            }
            _ => String::new(),
        }
    }

    fn current_uri(&self) -> Option<String> {
        self.playback
            .as_ref()
            .and_then(|playback| playback.item.as_ref())
            .map(|item| item.uri.clone())
    }

    /// The playlist, album or show playback is running inside, if any. Queue
    /// entries can only be jumped to within a context like that.
    fn context_uri(&self) -> Option<String> {
        self.playback.as_ref()?.context.as_ref()?.uri.clone()
    }

    /// The devices the picker offers, including the ones Spotify lists without
    /// an id (Google Cast endpoints): those are shown so the picker does not look
    /// empty, but they are marked unselectable because Spotify gives no way to
    /// transfer to them.
    ///
    /// The highlight follows the player rather than the list, so a poll that
    /// reports playback somewhere else wins over a list fetched before it.
    fn device_rows(&self) -> Vec<DeviceItem> {
        let playing = self
            .playback
            .as_ref()
            .and_then(|state| state.device.as_ref())
            .and_then(Device::connect_id);
        self.devices
            .iter()
            .map(|device| {
                let mut row = device_item(device);
                if let Some(playing) = playing {
                    row.active = device.connect_id() == Some(playing);
                }
                row
            })
            .collect()
    }

    /// Names the device the user picked before the next poll confirms it.
    fn focus_device(&mut self, device: &Device) {
        if let Some(state) = self.playback.as_mut() {
            let mut device = device.clone();
            device.is_active = true;
            state.device = Some(device);
        }
    }

    /// The item Spotify should report after `next`, when the warm queue knows
    /// about it.
    fn upcoming(&self) -> Option<Track> {
        let queue = self.queue.as_ref()?;
        let playing = queue.currently_playing.as_ref()?;
        if Some(playing.uri.as_str()) != self.current_uri().as_deref() {
            return None;
        }
        queue.queue.first().cloned()
    }

    fn previous_track(&mut self) -> Option<Track> {
        self.history.pop()
    }

    /// Whether the transport may skip in `direction` right now.
    fn can_skip(&self, direction: Skip) -> bool {
        let allowed = match direction {
            Skip::Previous => self.can_previous,
            Skip::Next | Skip::Jump => self.can_next,
        };
        let refused = self.refused_skip.as_ref().is_some_and(|(refused, item)| {
            *refused == direction && Some(item.as_str()) == self.current_uri().as_deref()
        });
        allowed && !refused
    }

    /// Remembers that Spotify refused a skip, so the button greys out for this
    /// item even when the poll does not report the restriction.
    fn refuse_skip(&mut self, direction: Skip) {
        if let Some(item) = self.current_uri() {
            self.refused_skip = Some((direction, item));
        }
    }

    /// Puts `track` on screen before Spotify reports it, so skipping a track
    /// changes the artwork on the press instead of on the next poll.
    ///
    /// Returns `false` when the item is already on screen.
    fn install_track(&mut self, track: Track, direction: Skip) -> bool {
        let origin = self.current_uri();
        if origin.as_deref() == Some(track.uri.as_str()) {
            return false;
        }
        let Some(current) = self.playback.as_ref().map(|playback| playback.item.clone()) else {
            return false;
        };
        if direction != Skip::Previous
            && let Some(current) = current.as_ref()
        {
            self.remember(current.clone());
        }

        if let Some(state) = self.playback.as_mut() {
            state.item = Some(track.clone());
            state.progress_ms = Some(0);
        }
        self.placeholder = None;
        self.update_queue(origin.clone(), current, &track, direction);

        if let Some(state) = self.playback.as_ref() {
            self.clock.apply(state, true);
        }
        self.pending.set_track(origin, Some(track.uri));
        true
    }

    /// Keeps the warm queue in step with the skip, so pressing next repeatedly
    /// (or going back and then forward) lands on the right item.
    fn update_queue(
        &mut self,
        origin: Option<String>,
        current: Option<Track>,
        track: &Track,
        direction: Skip,
    ) {
        let Some(queue) = self.queue.as_mut() else {
            return;
        };
        if queue
            .currently_playing
            .as_ref()
            .map(|item| item.uri.as_str())
            != origin.as_deref()
        {
            return;
        }

        match direction {
            Skip::Next => {
                if queue.queue.first().map(|item| item.uri.as_str()) == Some(track.uri.as_str()) {
                    queue.queue.remove(0);
                }
            }
            Skip::Previous => {
                // What we are leaving becomes what `next` returns to.
                if let Some(previous) = current {
                    queue.queue.insert(0, previous);
                }
            }
            Skip::Jump => {
                // Everything up to and including the tapped item is consumed.
                if let Some(index) = queue.queue.iter().position(|item| item.uri == track.uri) {
                    queue.queue.drain(..=index);
                }
            }
        }
        queue.currently_playing = Some(track.clone());
    }

    /// Shows the artwork of a playlist the user just started: Spotify does not
    /// say which track it picks until the next poll.
    fn show_playlist(&mut self, playlist: &Playlist) {
        self.placeholder = Some(Placeholder {
            title: playlist.name.clone(),
            subtitle: "Starting playlist".to_owned(),
            cover_url: playlist.cover_url(ART_SIZE).map(str::to_owned),
            playlist_uri: playlist.uri.clone(),
        });
        // A freshly started playlist begins on its first item.
        self.can_previous = false;
        self.can_next = true;
        self.clock.reset();
        self.pending.set_track(self.current_uri(), None);
    }

    fn remember(&mut self, track: Track) {
        if self.history.last().map(|last| last.uri.as_str()) == Some(track.uri.as_str()) {
            return;
        }
        if self.history.len() == HISTORY_DEPTH {
            self.history.remove(0);
        }
        self.history.push(track);
    }

    fn drop_pending(&mut self) {
        self.pending.clear();
        self.placeholder = None;
    }
}

/// How long a local change is trusted while Spotify catches up.
const OPTIMISTIC_GRACE: Duration = Duration::from_secs(3);

/// Locally applied changes that a poll must not undo.
///
/// Commands update the UI immediately, but a poll that was already in flight
/// (or one sent right after the command) can still report the previous state.
/// Until Spotify confirms the new value — or the grace period expires — these
/// values win over the polled ones.
#[derive(Default)]
struct Pending {
    until: Option<Instant>,
    playing: Option<bool>,
    shuffle: Option<bool>,
    repeat: Option<RepeatMode>,
    position: bool,
    track: Option<TrackExpectation>,
}

/// The item a skip or a playlist start should produce.
struct TrackExpectation {
    /// URI that was on screen when the command was sent.
    origin: Option<String>,
    /// URI Spotify should report once it caught up; `None` when the command's
    /// outcome is unknown, as when a playlist starts on an arbitrary track.
    expected: Option<String>,
}

impl Pending {
    fn set_playing(&mut self, playing: bool) {
        self.playing = Some(playing);
        self.begin();
    }

    fn set_shuffle(&mut self, shuffle: bool) {
        self.shuffle = Some(shuffle);
        self.begin();
    }

    fn set_repeat(&mut self, repeat: RepeatMode) {
        self.repeat = Some(repeat);
        self.begin();
    }

    fn set_position(&mut self) {
        self.position = true;
        self.begin();
    }

    fn set_track(&mut self, origin: Option<String>, expected: Option<String>) {
        self.track = Some(TrackExpectation { origin, expected });
        self.begin();
    }

    fn clear(&mut self) {
        *self = Self::default();
    }

    fn begin(&mut self) {
        self.until = Some(Instant::now() + OPTIMISTIC_GRACE);
    }

    /// Reconcile a poll with the optimistic values.
    ///
    /// Returns the playback flags to store and whether the polled position can
    /// be trusted (it cannot while a local seek is settling).
    fn resolve(&mut self, polled: &PlaybackState, now: Instant) -> (bool, bool, RepeatMode, bool) {
        if self.until.is_none_or(|until| now >= until) {
            self.clear();
            return (
                polled.is_playing,
                polled.shuffle_state,
                RepeatMode::from_state(&polled.repeat_state),
                true,
            );
        }

        let playing = settle(&mut self.playing, polled.is_playing);
        let shuffle = settle(&mut self.shuffle, polled.shuffle_state);
        let polled_repeat = RepeatMode::from_state(&polled.repeat_state);
        let repeat = match self.repeat {
            Some(expected) if expected == polled_repeat => {
                self.repeat = None;
                polled_repeat
            }
            Some(expected) => expected,
            None => polled_repeat,
        };

        (playing, shuffle, repeat, !self.position)
    }

    /// Reconciles the polled item with the one a command installed.
    ///
    /// Returns `true` while the poll has not reported what the command asked
    /// for, which means it was sent before Spotify applied it. Accepting those
    /// would make rapid skips jump back to tracks the user already left.
    fn keeps_local_track(&mut self, polled: Option<&str>) -> bool {
        let Some(expectation) = self.track.as_ref() else {
            return false;
        };
        let stale = match expectation.expected.as_deref() {
            Some(expected) => polled != Some(expected),
            None => polled == expectation.origin.as_deref(),
        };
        if !stale {
            self.track = None;
        }
        stale
    }
}

/// Keep the local value until the poll agrees with it.
fn settle<T: Copy + PartialEq>(pending: &mut Option<T>, polled: T) -> T {
    match *pending {
        Some(expected) if expected == polled => {
            *pending = None;
            polled
        }
        Some(expected) => expected,
        None => polled,
    }
}

/// Which way a skip goes.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Skip {
    Previous,
    Next,
    /// Straight to a queue entry the user tapped.
    Jump,
}

/// The track area as the view needs it.
struct NowPlayingView {
    title: String,
    subtitle: String,
    has_track: bool,
    cover_url: Option<String>,
    playlist_uri: String,
}

/// Artwork and copy shown while Spotify has not said which track it picked,
/// so the previous track does not linger on screen.
struct Placeholder {
    title: String,
    subtitle: String,
    cover_url: Option<String>,
    playlist_uri: String,
}

/// Interpolates playback progress between polls so the ring and the scrubber
/// move smoothly instead of in one-second jumps.
struct Clock {
    duration_ms: u64,
    base_ms: u64,
    started: Instant,
    playing: bool,
}

impl Clock {
    fn new() -> Self {
        Self {
            duration_ms: 0,
            base_ms: 0,
            started: Instant::now(),
            playing: false,
        }
    }

    fn apply(&mut self, state: &PlaybackState, trust_position: bool) {
        self.duration_ms = state
            .item
            .as_ref()
            .map(|item| item.duration_ms)
            .unwrap_or(0);
        if trust_position {
            let reported = state.progress_ms().min(self.duration_ms.saturating_sub(1));
            // The answer to a poll describes the past: Spotify finishes the
            // request a moment after it read the position. Re-anchoring on it
            // every second drags the ring backwards, so small deviations from
            // the interpolated position are left alone and only real jumps —
            // a seek, a restart, a long stall — win.
            let drift = reported as i64 - self.position_ms() as i64;
            if drift.unsigned_abs() > POSITION_DRIFT_MS {
                self.base_ms = reported;
            } else {
                self.base_ms = self.position_ms();
            }
            self.started = Instant::now();
        }
        self.playing = state.is_playing && self.duration_ms > 0;
    }

    fn position_ms(&self) -> u64 {
        let elapsed = if self.playing {
            self.started.elapsed().as_millis() as u64
        } else {
            0
        };
        (self.base_ms + elapsed).min(self.duration_ms.saturating_sub(1))
    }

    fn progress(&self) -> f32 {
        if self.duration_ms == 0 {
            0.0
        } else {
            (self.position_ms() as f64 / self.duration_ms as f64) as f32
        }
    }

    fn set_ratio(&mut self, ratio: f32) {
        let ratio = ratio.clamp(0.0, 1.0);
        self.base_ms = (self.duration_ms as f64 * ratio as f64).round() as u64;
        self.started = Instant::now();
    }

    fn set_playing(&mut self, playing: bool) {
        self.base_ms = self.position_ms();
        self.started = Instant::now();
        self.playing = playing && self.duration_ms > 0;
    }

    fn reset(&mut self) {
        self.duration_ms = 0;
        self.base_ms = 0;
        self.playing = false;
    }
}

impl Shared {
    fn new(
        ui: slint::Weak<AppWindow>,
        settings: Settings,
        rt: Handle,
        graphics: gpu::Graphics,
    ) -> Result<Self> {
        let (source, auth, initial_auth) = if settings.mock {
            (Some(Source::Mock(Mock::new())), None, AUTH_READY)
        } else {
            match settings.client_id.clone() {
                Some(client_id) => {
                    let auth = Arc::new(Auth::new(
                        client_id,
                        settings.redirect_uri(),
                        settings.token_file(),
                    )?);
                    let state = if auth.is_authorized() {
                        AUTH_READY
                    } else {
                        AUTH_NEEDED
                    };
                    let api = Api::new(auth.clone())?;
                    (Some(Source::Live(api)), Some(auth), state)
                }
                None => (None, None, AUTH_NO_CLIENT_ID),
            }
        };

        Ok(Self {
            ui,
            settings,
            rt,
            source,
            auth,
            artwork: Artwork::new()?,
            state: Mutex::new(Model::new()),
            motion: Mutex::new(Motion::new()),
            last_tick: Mutex::new(Instant::now()),
            poke: Notify::new(),
            ring: Ring::new(&graphics),
            ring_frame: AtomicUsize::new(0),
            visualizer_active: Arc::new(AtomicBool::new(false)),
            idle_scene: Arc::new(AtomicUsize::new(0)),
            idle_scenes: Arc::new(AtomicU64::new(0)),
            idle_travel: Arc::new(AtomicI64::new(0)),
            initial_auth,
            poll_started: AtomicBool::new(false),
            auth_running: AtomicBool::new(false),
            playlists_busy: AtomicBool::new(false),
            queue_busy: AtomicBool::new(false),
            devices_busy: AtomicBool::new(false),
            covers: AtomicU64::new(0),
            playlists_generation: Arc::new(AtomicU64::new(0)),
            queue_generation: Arc::new(AtomicU64::new(0)),
            status_generation: Arc::new(AtomicU64::new(0)),
        })
    }

    fn bind(self: &Arc<Self>, window: &AppWindow) {
        let app = window.global::<App>();

        {
            let this = self.clone();
            app.on_navigate(move |screen| this.navigate(screen));
        }
        {
            let this = self.clone();
            app.on_next(move || {
                this.spawn_cmd(|this| async move { this.cmd_next().await });
            });
        }
        {
            let this = self.clone();
            app.on_previous(move || {
                this.spawn_cmd(|this| async move { this.cmd_previous().await });
            });
        }
        {
            let this = self.clone();
            app.on_toggle_play(move || {
                this.spawn_cmd(|this| async move { this.cmd_toggle_play().await });
            });
        }
        {
            let this = self.clone();
            app.on_seek(move |ratio| {
                this.spawn_cmd(move |this| async move { this.cmd_seek(ratio).await });
            });
        }
        {
            // Pointer moves arrive on the UI thread, so the preview is painted
            // straight away instead of waiting for the animation tick; that keeps
            // the rim glued to the finger.
            let this = self.clone();
            app.on_scrub(move |ratio| {
                this.lock().scrub = (ratio >= 0.0).then_some(ratio);
                if ratio < 0.0 {
                    return;
                }
                let Some(ui) = this.ui.upgrade() else {
                    return;
                };
                this.paint_ring(&ui, ratio, true, 0.0);
            });
        }
        {
            let this = self.clone();
            app.on_toggle_shuffle(move || {
                this.spawn_cmd(|this| async move { this.cmd_toggle_shuffle().await });
            });
        }
        {
            let this = self.clone();
            app.on_cycle_repeat(move || {
                this.spawn_cmd(|this| async move { this.cmd_cycle_repeat().await });
            });
        }
        {
            let this = self.clone();
            app.on_play_playlist(move |uri| {
                let uri = uri.to_string();
                this.spawn_cmd(move |this| async move { this.cmd_play_playlist(uri).await });
            });
        }
        {
            let this = self.clone();
            app.on_play_queue_item(move |uri| {
                let uri = uri.to_string();
                this.spawn_cmd(move |this| async move { this.cmd_play_queue_item(uri).await });
            });
        }
        {
            let this = self.clone();
            app.on_refresh_playlists(move || {
                this.spawn_cmd(|this| async move { this.load_playlists(true).await });
            });
        }
        {
            let this = self.clone();
            app.on_refresh_queue(move || {
                this.spawn_cmd(|this| async move { this.load_queue(true).await });
            });
        }
        {
            let this = self.clone();
            app.on_begin_auth(move || this.start_auth());
        }
        {
            let this = self.clone();
            app.on_show_visualizer(move || this.show_visualizer());
        }
        {
            let this = self.clone();
            app.on_open_devices(move || this.open_devices());
        }
        {
            let this = self.clone();
            app.on_close_devices(move || this.close_devices());
        }
        {
            let this = self.clone();
            app.on_select_device(move |id| {
                let id = id.to_string();
                this.spawn_cmd(move |this| async move { this.cmd_transfer(id).await });
            });
        }
        {
            let this = self.clone();
            app.on_refresh_devices(move || {
                this.spawn_cmd(|this| async move { this.load_devices().await });
            });
        }
        {
            let this = self.clone();
            window
                .global::<Idle>()
                .on_dismiss(move || this.dismiss_visualizer());
        }
        {
            let travel = self.idle_travel.clone();
            window.global::<Idle>().on_travel(move |legs| {
                travel.fetch_add(i64::from(legs), Ordering::Relaxed);
            });
        }

        // Wrapped once, so the renderer caches one image per render target
        // instead of seeing a new image every frame.
        window.set_ring_frames(self.ring.textures());
    }

    // ---- plumbing -----------------------------------------------------

    fn spawn<F>(&self, future: F)
    where
        F: std::future::Future<Output = ()> + Send + 'static,
    {
        self.rt.spawn(future);
    }

    /// Spawn a command that needs an owned handle to the shared state.
    fn spawn_cmd<F, Fut>(self: &Arc<Self>, run: F)
    where
        F: FnOnce(Arc<Self>) -> Fut + Send + 'static,
        Fut: std::future::Future<Output = ()> + Send + 'static,
    {
        let this = self.clone();
        let runtime = self.rt.clone();
        runtime.spawn(async move { run(this).await });
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Model> {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn lock_motion(&self) -> std::sync::MutexGuard<'_, Motion> {
        self.motion
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    async fn bootstrap(self: &Arc<Self>) {
        // Runs on the main thread before the event loop starts, so direct
        // property access is fine here.
        if let Some(ui) = self.ui.upgrade() {
            ui.set_config_path(&self.settings.config_file().display().to_string());
            ui.set_auth_state(self.initial_auth, "");
        }

        match self.initial_auth {
            AUTH_READY => self.start_session(),
            AUTH_NO_CLIENT_ID => tracing::warn!(
                path = %self.settings.config_file().display(),
                "no Spotify client id configured"
            ),
            _ => {}
        }
    }

    fn start_session(self: &Arc<Self>) {
        {
            let mut model = self.lock();
            model.screen = SCREEN_NOW;
        }

        let weak = self.ui.clone();
        let _ = self.ui.upgrade_in_event_loop(move |ui| {
            ui.set_auth_state(AUTH_READY, "");
            ui.set_screen(SCREEN_NOW);
            // Let the first frame land before fading the UI in.
            slint::Timer::single_shot(Duration::from_millis(160), move || {
                if let Some(ui) = weak.upgrade() {
                    ui.reveal_app();
                }
            });
        });

        self.start_polling();
        self.poke.notify_one();
    }

    fn start_polling(self: &Arc<Self>) {
        if self.poll_started.swap(true, Ordering::SeqCst) {
            return;
        }
        self.spawn_cmd(|this| async move {
            loop {
                let delay = this.poll_once().await;
                tokio::select! {
                    _ = tokio::time::sleep(delay) => {}
                    _ = this.poke.notified() => {}
                }
            }
        });
    }

    // ---- navigation ---------------------------------------------------

    fn navigate(self: &Arc<Self>, screen: i32) {
        let screen = screen.clamp(SCREEN_QUEUE, SCREEN_PLAYLISTS);
        self.lock().screen = screen;

        // Callbacks arrive on the UI thread, but `cmd_play_playlist` also
        // navigates from a worker thread, so always go through the event loop.
        let _ = self
            .ui
            .upgrade_in_event_loop(move |ui| ui.set_screen(screen));

        match screen {
            SCREEN_PLAYLISTS => {
                self.spawn_cmd(|this| async move { this.load_playlists(false).await });
            }
            SCREEN_QUEUE => {
                self.spawn_cmd(|this| async move { this.load_queue(false).await });
            }
            _ => {}
        }
    }

    // ---- playback commands --------------------------------------------

    async fn cmd_next(self: Arc<Self>) {
        let Some(source) = self.source.clone() else {
            return;
        };

        // The warm queue knows the upcoming item, so show it right away.
        let shown = {
            let mut model = self.lock();
            let upcoming = model.upcoming();
            match upcoming {
                Some(track) => model.install_track(track, Skip::Next),
                None => false,
            }
        };
        if shown {
            self.render_now_playing(true);
            self.prefetch_next_cover();
        } else {
            // The queue could not help: refresh it so the next press can show
            // the artwork up front.
            self.spawn_cmd(|this| async move { this.warm_queue().await });
        }

        if let Err(err) = source.next().await {
            self.lock().drop_pending();
            if skip_restricted(&err) {
                tracing::debug!(%err, "Spotify refused to skip forward");
                self.lock().refuse_skip(Skip::Next);
                self.render_now_playing(false);
            } else {
                self.report(&err);
            }
        }
        self.poke.notify_one();
    }

    async fn cmd_previous(self: Arc<Self>) {
        let Some(source) = self.source.clone() else {
            return;
        };

        let shown = {
            let mut model = self.lock();
            if model.clock.position_ms() > PREVIOUS_RESTART_MS {
                // Spotify restarts the track instead of skipping back.
                model.clock.set_ratio(0.0);
                model.pending.set_position();
                false
            } else {
                let previous = model.previous_track();
                match previous {
                    Some(track) => model.install_track(track, Skip::Previous),
                    None => false,
                }
            }
        };
        if shown {
            self.render_now_playing(true);
        } else {
            // The history could not help: refresh the queue so the next press
            // can show the artwork up front.
            self.spawn_cmd(|this| async move { this.warm_queue().await });
        }

        if let Err(err) = source.previous().await {
            self.lock().drop_pending();
            if skip_restricted(&err) {
                tracing::debug!(%err, "Spotify refused to skip back");
                self.lock().refuse_skip(Skip::Previous);
                self.render_now_playing(false);
            } else {
                self.report(&err);
            }
        }
        self.poke.notify_one();
    }

    async fn cmd_toggle_play(self: Arc<Self>) {
        let Some(source) = self.source.clone() else {
            return;
        };
        let current = self.lock().playback.as_ref().map(|state| state.is_playing);
        let Some(current) = current else {
            self.show_status("Nothing is playing".to_owned());
            return;
        };

        let target = !current;
        {
            let mut model = self.lock();
            model.clock.set_playing(target);
            model.pending.set_playing(target);
            if let Some(state) = model.playback.as_mut() {
                state.is_playing = target;
            }
        }
        self.push_playback();

        if let Err(err) = source.set_playing(target).await {
            self.lock().drop_pending();
            self.report(&err);
        }
        self.poke.notify_one();
    }

    async fn cmd_seek(self: Arc<Self>, ratio: f32) {
        let Some(source) = self.source.clone() else {
            return;
        };
        let position = {
            let mut model = self.lock();
            // The preview and the clock now agree, so the rim stays put.
            model.scrub = None;
            model.clock.set_ratio(ratio);
            model.pending.set_position();
            model.clock.position_ms()
        };
        if let Err(err) = source.seek(position).await {
            self.lock().drop_pending();
            self.report(&err);
        }
        self.poke.notify_one();
    }

    async fn cmd_toggle_shuffle(self: Arc<Self>) {
        let Some(source) = self.source.clone() else {
            return;
        };
        let target = {
            let mut model = self.lock();
            let Some(state) = model.playback.as_mut() else {
                return;
            };
            let target = !state.shuffle_state;
            state.shuffle_state = target;
            model.pending.set_shuffle(target);
            target
        };
        self.push_playback();
        if let Err(err) = source.set_shuffle(target).await {
            self.lock().drop_pending();
            self.report(&err);
        }
        self.poke.notify_one();
    }

    async fn cmd_cycle_repeat(self: Arc<Self>) {
        let Some(source) = self.source.clone() else {
            return;
        };
        let next = {
            let mut model = self.lock();
            let Some(state) = model.playback.as_mut() else {
                return;
            };
            let next = RepeatMode::from_state(&state.repeat_state).next();
            state.repeat_state = next.as_str().to_owned();
            model.pending.set_repeat(next);
            next
        };
        self.push_playback();
        if let Err(err) = source.set_repeat(next).await {
            self.lock().drop_pending();
            self.report(&err);
        }
        self.poke.notify_one();
    }

    async fn cmd_play_playlist(self: Arc<Self>, uri: String) {
        let Some(source) = self.source.clone() else {
            return;
        };

        let shown = {
            let mut model = self.lock();
            let playlist = model
                .playlists
                .iter()
                .find(|playlist| playlist.uri == uri)
                .cloned();
            match playlist {
                Some(playlist) => {
                    model.show_playlist(&playlist);
                    true
                }
                None => false,
            }
        };
        if shown {
            self.render_now_playing(true);
        }

        match source.play_context(&uri).await {
            Ok(()) => {
                self.navigate(SCREEN_NOW);
                self.poke.notify_one();
            }
            Err(err) => {
                self.lock().drop_pending();
                self.report(&err);
            }
        }
    }

    /// Jump to a tapped queue entry by restarting the current playlist there.
    async fn cmd_play_queue_item(self: Arc<Self>, uri: String) {
        let Some(source) = self.source.clone() else {
            return;
        };

        let target = {
            let model = self.lock();
            if model.current_uri().as_deref() == Some(uri.as_str()) {
                return;
            }
            let track = model
                .queue
                .as_ref()
                .and_then(|queue| queue.queue.iter().find(|item| item.uri == uri))
                .cloned();
            track.zip(model.context_uri())
        };
        let Some((track, context)) = target else {
            self.show_status("Can't jump from here".to_owned());
            return;
        };

        if self.lock().install_track(track.clone(), Skip::Jump) {
            self.render_now_playing(true);
        }

        match source.play_in_context(&context, &track.uri).await {
            Ok(()) => {
                self.navigate(SCREEN_NOW);
                self.poke.notify_one();
            }
            Err(err) => {
                self.lock().drop_pending();
                // Autoplay entries can sit in the queue without belonging to the
                // playlist, which Spotify rejects.
                if matches!(
                    err.downcast_ref::<ApiError>(),
                    Some(ApiError::Spotify {
                        status: 400 | 404,
                        ..
                    })
                ) {
                    self.show_status("Can't jump to that track".to_owned());
                } else {
                    self.report(&err);
                }
            }
        }
    }

    fn push_playback(&self) {
        let (playing, controls, shuffle, repeat) = {
            let model = self.lock();
            let state = model.playback.as_ref();
            (
                state.map(|state| state.is_playing).unwrap_or(false),
                state.map(controls_available).unwrap_or(false),
                state.map(|state| state.shuffle_state).unwrap_or(false),
                state
                    .map(|state| RepeatMode::from_state(&state.repeat_state).index())
                    .unwrap_or(0),
            )
        };
        let _ = self
            .ui
            .upgrade_in_event_loop(move |ui| ui.set_playback(playing, controls, shuffle, repeat));
    }

    // ---- output devices -----------------------------------------------

    /// Opens the device picker, re-listing the devices if the cached list has
    /// gone stale.
    fn open_devices(self: &Arc<Self>) {
        let stale = {
            let mut model = self.lock();
            model.devices_open = true;
            model
                .devices_loaded
                .is_none_or(|loaded| loaded.elapsed() >= DEVICES_TTL)
        };

        let _ = self.ui.upgrade_in_event_loop(move |ui| {
            ui.set_devices_open(true);
            if stale {
                // Avoid one frame of the "no devices" hint before the fetch.
                ui.set_devices_loading(true);
            }
        });
        if stale {
            self.spawn_cmd(|this| async move { this.load_devices().await });
        }
    }

    fn close_devices(self: &Arc<Self>) {
        self.lock().devices_open = false;
        let _ = self
            .ui
            .upgrade_in_event_loop(|ui| ui.set_devices_open(false));
    }

    async fn load_devices(self: &Arc<Self>) {
        let Some(source) = self.source.clone() else {
            return;
        };
        if self.devices_busy.swap(true, Ordering::SeqCst) {
            return;
        }
        let _ = self
            .ui
            .upgrade_in_event_loop(|ui| ui.set_devices_loading(true));

        let result = source.devices().await;
        self.devices_busy.store(false, Ordering::SeqCst);

        match result {
            Ok(devices) => {
                let rows = {
                    let mut model = self.lock();
                    model.devices = devices;
                    model.devices_loaded = Some(Instant::now());
                    model.device_rows()
                };
                let empty = rows.is_empty();
                let _ = self.ui.upgrade_in_event_loop(move |ui| {
                    ui.set_devices(rows);
                    if empty {
                        ui.set_devices_empty(NO_DEVICES);
                    }
                });
            }
            Err(err) => {
                tracing::debug!(%err, "listing the playback devices failed");
                let message = friendly(&err);
                let _ = self
                    .ui
                    .upgrade_in_event_loop(move |ui| ui.set_devices_empty(&message));
                self.report(&err);
            }
        }
    }

    /// Move playback to the picked device. The picker closes and the player
    /// names the new device straight away, before Spotify confirms.
    async fn cmd_transfer(self: Arc<Self>, device_id: String) {
        let Some(source) = self.source.clone() else {
            return;
        };

        let target = {
            let model = self.lock();
            model
                .devices
                .iter()
                .find(|device| device.connect_id() == Some(device_id.as_str()))
                .cloned()
        };
        let Some(target) = target else {
            self.show_status("That device is no longer available".to_owned());
            self.spawn_cmd(|this| async move { this.load_devices().await });
            return;
        };
        if !target.selectable() {
            self.show_status(
                if target.is_restricted {
                    "That device does not allow remote control"
                } else {
                    // Google Cast endpoints come back without an id, which is
                    // what `PUT /me/player` needs to move playback there.
                    "Start playback there from the Spotify app"
                }
                .to_owned(),
            );
            return;
        }
        let on_target = {
            let model = self.lock();
            match model
                .playback
                .as_ref()
                .and_then(|state| state.device.as_ref())
            {
                Some(device) => device.connect_id() == target.connect_id(),
                // Nothing is playing: the freshest list is all we have.
                None => target.is_active,
            }
        };
        if on_target {
            self.close_devices();
            return;
        }

        {
            let mut model = self.lock();
            model.devices_open = false;
            for device in &mut model.devices {
                device.is_active = device.connect_id() == target.connect_id();
            }
            model.focus_device(&target);
        }
        let _ = self
            .ui
            .upgrade_in_event_loop(|ui| ui.set_devices_open(false));
        self.render_now_playing(false);
        self.show_status(format!("Playing on {}", target.name));

        if let Err(err) = source.transfer(&device_id).await {
            // The list is what went stale: refresh it so a retry can succeed.
            if matches!(
                err.downcast_ref::<ApiError>(),
                Some(ApiError::Spotify { status: 404, .. })
            ) {
                self.show_status("That device is not available".to_owned());
                self.spawn_cmd(|this| async move { this.load_devices().await });
            } else {
                self.report(&err);
            }
        }
        self.poke.notify_one();
    }

    // ---- polling ------------------------------------------------------

    async fn poll_once(self: &Arc<Self>) -> Duration {
        let Some(source) = self.source.clone() else {
            return Duration::from_secs(5);
        };

        match source.playback().await {
            Ok(Some(state)) => {
                self.apply_state(state);
                self.settings.poll_interval
            }
            Ok(None) => {
                self.apply_idle();
                Duration::from_secs(2)
            }
            Err(err) => {
                if let Some(ApiError::RateLimited(delay)) = err.downcast_ref::<ApiError>() {
                    tracing::warn!(?delay, "rate limited by Spotify");
                    return *delay;
                }
                if needs_reauth(&err) {
                    tracing::warn!(error = %err, "the stored token is no longer valid");
                    self.show_status("Reconnect your Spotify account".to_owned());
                    let _ = self
                        .ui
                        .upgrade_in_event_loop(|ui| ui.set_auth_state(AUTH_NEEDED, ""));
                    return Duration::from_secs(60);
                }
                self.report(&err);
                Duration::from_secs(5)
            }
        }
    }

    fn apply_state(self: &Arc<Self>, polled: PlaybackState) {
        let (state, track_changed, queue_visible) = {
            let mut model = self.lock();
            let state = model.store_polled(polled);

            let previous = model
                .playback
                .as_ref()
                .and_then(|playback| playback.item.clone());
            let track_changed = previous.as_ref().map(|item| item.uri.clone())
                != state.item.as_ref().map(|item| item.uri.clone());
            if let (true, Some(previous)) = (track_changed, previous) {
                model.remember(previous);
            }

            model.playback = Some(state.clone());
            (state, track_changed, model.screen == SCREEN_QUEUE)
        };

        if track_changed && let Some(device) = state.device.as_ref() {
            tracing::info!(
                device = %device.name,
                track = %state.item.as_ref().map(|item| item.name.as_str()).unwrap_or("-"),
                "now playing"
            );
        }

        self.render_now_playing(track_changed);

        if track_changed {
            if queue_visible {
                self.spawn_cmd(|this| async move { this.load_queue(true).await });
            } else {
                self.spawn_cmd(|this| async move { this.warm_queue().await });
            }
        }
    }

    /// Pushes the model's current item and playback flags to the view.
    ///
    /// `track_changed` clears the artwork, ring and progress first because the
    /// item on screen is a different one.
    fn render_now_playing(self: &Arc<Self>, track_changed: bool) {
        self.note_idle();
        let (view, playing, controls, shuffle, repeat, can_previous, can_next, device) = {
            let model = self.lock();
            let state = model.playback.as_ref();
            (
                model.now_playing(),
                state.map(|state| state.is_playing).unwrap_or(false),
                state.map(controls_available).unwrap_or(false),
                state.map(|state| state.shuffle_state).unwrap_or(false),
                state
                    .map(|state| RepeatMode::from_state(&state.repeat_state).index())
                    .unwrap_or(0),
                model.can_skip(Skip::Previous),
                model.can_skip(Skip::Next),
                state
                    .and_then(|state| state.device.as_ref())
                    .map(|device| (device.name.clone(), device_kind(device.kind.as_deref()).0)),
            )
        };

        let reset = track_changed || !view.has_track;
        let NowPlayingView {
            title,
            subtitle,
            has_track,
            cover_url,
            playlist_uri,
        } = view;
        let (device_name, device_icon) = device.unwrap_or_default();
        let has_device = !device_name.is_empty();
        let _ = self.ui.upgrade_in_event_loop(move |ui| {
            ui.set_track_text(&title, &subtitle, has_track);
            ui.set_playback(playing, controls, shuffle, repeat);
            ui.set_skip_state(can_previous, can_next);
            ui.set_device(&device_name, device_icon, has_device);
            ui.set_current_playlist(&playlist_uri);
            // The artwork is left alone when the track changes: the old cover
            // stays on screen until the new one is ready, which keeps skips from
            // flashing.
        });

        if !reset {
            return;
        }

        self.covers.fetch_add(1, Ordering::SeqCst);
        match cover_url {
            Some(url) => self.load_cover(url),
            None => {
                self.lock().art_visible = false;
                let _ = self.ui.upgrade_in_event_loop(|ui| ui.set_cover(None));
            }
        }
    }

    fn apply_idle(self: &Arc<Self>) {
        {
            let mut model = self.lock();
            model.playback = None;
            model.clock.reset();
            model.drop_pending();
            model.can_previous = false;
            model.can_next = false;
            model.art_visible = false;
            model.note_playback(false, false, Instant::now());
        }

        let _ = self.ui.upgrade_in_event_loop(|ui| {
            ui.set_track_text("Nothing playing", "Open Spotify and start playback", false);
            ui.set_playback(false, false, false, 0);
            ui.set_device("", "speaker", false);
            ui.set_cover(None);
            ui.set_current_playlist("");
        });
    }

    /// Decodes the cover of the next queued item in the background, so a skip
    /// can show its artwork without waiting for a download.
    fn prefetch_next_cover(self: &Arc<Self>) {
        let url = {
            let model = self.lock();
            model
                .queue
                .as_ref()
                .and_then(|queue| queue.queue.first())
                .and_then(|track| track.cover_url(ART_SIZE))
                .map(str::to_owned)
        };
        let Some(url) = url else {
            return;
        };
        self.spawn_cmd(move |this| async move {
            if let Err(err) = this.artwork.load(&url, ART_SIZE, ART_CORNER).await {
                tracing::debug!(%err, "prefetching the next cover failed");
            }
        });
    }

    fn load_cover(self: &Arc<Self>, url: String) {
        let generation = self.covers.fetch_add(1, Ordering::SeqCst) + 1;
        self.spawn_cmd(move |this| async move {
            let art = match this.artwork.load(&url, ART_SIZE, ART_CORNER).await {
                Ok(art) => art,
                Err(err) => {
                    tracing::warn!(%err, "loading artwork failed");
                    return;
                }
            };
            if this.covers.load(Ordering::SeqCst) != generation {
                return;
            }
            let had_art = {
                let mut model = this.lock();
                std::mem::replace(&mut model.art_visible, true)
            };
            {
                let mut motion = this.lock_motion();
                motion.set_accent(art.accent());
                if had_art {
                    motion.swap();
                } else {
                    motion.reveal();
                }
            }
            let _ = this
                .ui
                .upgrade_in_event_loop(move |ui| ui.set_cover(Some(&art)));
        });
    }

    // ---- playlists ----------------------------------------------------

    async fn load_playlists(self: Arc<Self>, force: bool) {
        let Some(source) = self.source.clone() else {
            return;
        };

        if !force {
            let cached = {
                let model = self.lock();
                match model.playlists_loaded {
                    Some(loaded)
                        if loaded.elapsed() < LIBRARY_TTL && !model.playlists.is_empty() =>
                    {
                        Some(model.playlists.clone())
                    }
                    _ => None,
                }
            };
            if let Some(playlists) = cached {
                self.publish_playlists(playlists);
                return;
            }
        }

        if self.playlists_busy.swap(true, Ordering::SeqCst) {
            return;
        }
        let _ = self
            .ui
            .upgrade_in_event_loop(|ui| ui.set_playlists_loading(true));

        let result = source.playlists().await;
        self.playlists_busy.store(false, Ordering::SeqCst);

        match result {
            Ok(playlists) => {
                {
                    let mut model = self.lock();
                    model.playlists = playlists.clone();
                    model.playlists_loaded = Some(Instant::now());
                }
                self.publish_playlists(playlists);
            }
            Err(err) => {
                tracing::warn!(%err, "loading playlists failed");
                let message = friendly(&err);
                let _ = self
                    .ui
                    .upgrade_in_event_loop(move |ui| ui.set_playlists_error(&message));
            }
        }
    }

    fn publish_playlists(self: &Arc<Self>, playlists: Vec<Playlist>) {
        let rows: Vec<PlaylistRow> = playlists
            .iter()
            .map(|playlist| PlaylistRow {
                title: playlist.name.clone(),
                subtitle: playlist.subtitle(),
                uri: playlist.uri.clone(),
                cover: playlist.cover_url(PLAYLIST_ART_SIZE).map(str::to_owned),
            })
            .collect();

        let subtitle = match rows.len() {
            0 => String::new(),
            1 => "1 playlist".to_owned(),
            count => format!("{count} playlists"),
        };

        if rows.is_empty() {
            let _ = self
                .ui
                .upgrade_in_event_loop(|ui| ui.set_playlists_error("No playlists found"));
            return;
        }

        // Covers decoded before go in with the rows; only the missing ones are
        // loaded, and each then changes its own row instead of the whole list.
        let (covers, jobs) = self.split_covers(
            &rows,
            |row| row.cover.as_deref(),
            PLAYLIST_ART_SIZE,
            THUMB_CORNER,
            |row| &row.uri,
        );
        let generation = self.playlists_generation.fetch_add(1, Ordering::SeqCst) + 1;
        let _ = self.ui.upgrade_in_event_loop(move |ui| {
            let items = rows
                .iter()
                .enumerate()
                .map(|(index, row)| playlist_item(row, covers.get(&index).map(|art| art.as_ref())))
                .collect::<Vec<_>>();
            ui.set_playlists(items, &subtitle);
        });
        if jobs.is_empty() {
            return;
        }

        self.spawn_cmd(move |this| async move {
            let mut set = tokio::task::JoinSet::new();
            for job in jobs {
                let this = this.clone();
                set.spawn(async move {
                    let art = this
                        .artwork
                        .load(&job.url, PLAYLIST_ART_SIZE, THUMB_CORNER)
                        .await;
                    (job, art)
                });
            }

            while let Some(joined) = set.join_next().await {
                let Ok((job, Ok(art))) = joined else {
                    continue;
                };
                if this.playlists_generation.load(Ordering::SeqCst) != generation {
                    return;
                }
                let _ = this.ui.upgrade_in_event_loop(move |ui| {
                    ui.set_playlist_cover(job.index, &job.uri, &art);
                });
            }
        });
    }

    /// Splits a list's covers into those already decoded, by row, and the
    /// loads still to run.
    fn split_covers<T>(
        &self,
        rows: &[T],
        cover: impl Fn(&T) -> Option<&str>,
        size: u32,
        corner: u32,
        uri: impl Fn(&T) -> &str,
    ) -> (HashMap<usize, Arc<Art>>, Vec<CoverJob>) {
        let mut cached = HashMap::new();
        let mut jobs = Vec::new();
        for (index, row) in rows.iter().enumerate() {
            let Some(url) = cover(row) else { continue };
            match self.artwork.cached(url, size, corner) {
                Some(art) => {
                    cached.insert(index, art);
                }
                None => jobs.push(CoverJob {
                    index,
                    uri: uri(row).to_owned(),
                    url: url.to_owned(),
                }),
            }
        }
        (cached, jobs)
    }

    // ---- queue --------------------------------------------------------

    async fn load_queue(self: Arc<Self>, force: bool) {
        let Some(source) = self.source.clone() else {
            return;
        };

        if !force {
            let cached = {
                let model = self.lock();
                match (model.queue_loaded, &model.queue) {
                    (Some(loaded), Some(queue)) if loaded.elapsed() < QUEUE_TTL => {
                        Some(queue.clone())
                    }
                    _ => None,
                }
            };
            if let Some(queue) = cached {
                self.publish_queue(queue);
                return;
            }
        }

        if self.queue_busy.swap(true, Ordering::SeqCst) {
            return;
        }
        let _ = self
            .ui
            .upgrade_in_event_loop(|ui| ui.set_queue_loading(true));

        let result = source.queue().await;
        self.queue_busy.store(false, Ordering::SeqCst);

        match result {
            Ok(queue) => {
                {
                    let mut model = self.lock();
                    model.queue = Some(queue.clone());
                    model.queue_loaded = Some(Instant::now());
                }
                self.publish_queue(queue);
            }
            Err(err) => {
                tracing::warn!(%err, "loading the queue failed");
                let message = friendly(&err);
                let _ = self
                    .ui
                    .upgrade_in_event_loop(move |ui| ui.set_queue_error(&message));
            }
        }
    }

    /// Keeps the upcoming queue cached so `next` has something to show even
    /// when the queue screen was never opened. Only updates the model.
    async fn warm_queue(self: Arc<Self>) {
        let Some(source) = self.source.clone() else {
            return;
        };
        if self.queue_busy.swap(true, Ordering::SeqCst) {
            return;
        }
        let result = source.queue().await;
        self.queue_busy.store(false, Ordering::SeqCst);

        match result {
            Ok(queue) => {
                let mut model = self.lock();
                model.queue = Some(queue);
                model.queue_loaded = Some(Instant::now());
                drop(model);
                self.prefetch_next_cover();
            }
            Err(err) => tracing::debug!(%err, "warming the queue failed"),
        }
    }

    fn publish_queue(self: &Arc<Self>, response: QueueResponse) {
        let mut rows: Vec<QueueRow> = Vec::new();
        if let Some(current) = &response.currently_playing {
            rows.push(queue_row(current, true));
        }
        rows.extend(response.queue.iter().map(|track| queue_row(track, false)));

        let subtitle = match response.queue.len() {
            0 => "Nothing queued".to_owned(),
            count => format::track_count(count as u32),
        };

        if rows.is_empty() {
            let _ = self
                .ui
                .upgrade_in_event_loop(|ui| ui.set_queue_error("Nothing queued"));
            return;
        }

        // As for playlists: decoded covers go in with the rows, so a queue that
        // is shown again appears whole, and the rest fill in row by row.
        let (covers, jobs) = self.split_covers(
            &rows,
            |row| row.cover.as_deref(),
            QUEUE_ART_SIZE,
            QUEUE_CORNER,
            |row| &row.uri,
        );
        let generation = self.queue_generation.fetch_add(1, Ordering::SeqCst) + 1;
        let _ = self.ui.upgrade_in_event_loop(move |ui| {
            let items = rows
                .iter()
                .enumerate()
                .map(|(index, row)| queue_item(row, covers.get(&index).map(|art| art.as_ref())))
                .collect::<Vec<_>>();
            ui.set_queue(items, &subtitle, true);
        });
        if jobs.is_empty() {
            return;
        }

        self.spawn_cmd(move |this| async move {
            let mut set = tokio::task::JoinSet::new();
            for job in jobs {
                let this = this.clone();
                set.spawn(async move {
                    let art = this
                        .artwork
                        .load(&job.url, QUEUE_ART_SIZE, QUEUE_CORNER)
                        .await;
                    (job, art)
                });
            }

            while let Some(joined) = set.join_next().await {
                let Ok((job, Ok(art))) = joined else {
                    continue;
                };
                if this.queue_generation.load(Ordering::SeqCst) != generation {
                    return;
                }
                let _ = this.ui.upgrade_in_event_loop(move |ui| {
                    ui.set_queue_cover(job.index, &job.uri, &art);
                });
            }
        });
    }

    // ---- authorization ------------------------------------------------

    fn start_auth(self: &Arc<Self>) {
        if self.auth.is_none() || self.auth_running.swap(true, Ordering::SeqCst) {
            return;
        }
        self.spawn_cmd(|this| async move {
            if let Err(err) = this.auth_flow().await {
                tracing::warn!(%err, "authorization failed");
                let message = friendly(&err);
                let _ = this
                    .ui
                    .upgrade_in_event_loop(move |ui| ui.set_auth_state(AUTH_FAILED, &message));
            }
            this.auth_running.store(false, Ordering::SeqCst);
        });
    }

    async fn auth_flow(self: &Arc<Self>) -> Result<()> {
        let Some(auth) = self.auth.clone() else {
            return Ok(());
        };

        let _ = self
            .ui
            .upgrade_in_event_loop(|ui| ui.set_auth_state(AUTH_WAITING, ""));

        let flow = auth.begin_login(&self.settings.login_base()).await?;
        let login_url = flow.login_url().to_string();
        let qr = Artwork::qr(&login_url).ok();
        {
            let login_url = login_url.clone();
            let _ = self
                .ui
                .upgrade_in_event_loop(move |ui| ui.set_auth_url(&login_url, qr.as_ref()));
        }

        if !browser::open(flow.url().as_str()) {
            tracing::info!(%login_url, "no browser opened; open the URL shown on the display");
        }

        let grant = flow.wait().await?;
        auth.finish_login(grant)?;
        tracing::info!("Spotify account connected");

        self.start_session();
        Ok(())
    }

    /// Keeps the model's current playback flags and the idle visualizer in step.
    fn note_idle(&self) {
        let now = Instant::now();
        let mut model = self.lock();
        let playing = model
            .playback
            .as_ref()
            .map(|state| state.is_playing)
            .unwrap_or(false);
        let has_track = model
            .playback
            .as_ref()
            .and_then(|state| state.item.as_ref())
            .is_some()
            || model.placeholder.is_some();
        model.note_playback(playing, has_track, now);
    }

    /// Swipe down on the player: show the visualizer even while music plays.
    fn show_visualizer(self: &Arc<Self>) {
        let mut model = self.lock();
        model.idle.manual = true;
        model.idle.dismissed = false;
    }

    /// A tap on the visuals: back to the player, and stay there until playback
    /// changes.
    fn dismiss_visualizer(self: &Arc<Self>) {
        {
            let mut model = self.lock();
            model.idle.manual = false;
            model.idle.dismissed = true;
        }
        self.navigate(SCREEN_NOW);
    }

    // ---- animation tick ------------------------------------------------

    fn tick(self: &Arc<Self>) {
        let now = Instant::now();
        let dt = {
            let mut last = self
                .last_tick
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            let dt = now.duration_since(*last).as_secs_f32().min(0.25);
            *last = now;
            dt
        };

        // The visualizer takes over once playback has been idle long enough. The
        // fade is stepped here so it always advances, and the renderer is only
        // asked to work while the overlay is on screen.
        let available = self.idle_scenes.load(Ordering::Relaxed);
        let (visualizer, fade, fade_changed, screen, selected) = {
            let mut model = self.lock();
            let visualizer = model.wants_visualizer(now);
            let selected = if visualizer && !model.idle.showing && model.idle.fade <= 0.0 {
                Some(Scene::random(available))
            } else {
                None
            };
            model.idle.showing = visualizer;
            let target = if visualizer { 1.0 } else { 0.0 };
            let step = dt / IDLE_FADE;
            let previous = model.idle.fade;
            model.idle.fade = if previous < target {
                (previous + step).min(target)
            } else {
                (previous - step).max(target)
            };
            let changed = model.idle.fade != previous;
            (visualizer, model.idle.fade, changed, model.screen, selected)
        };
        if let Some(scene) = selected {
            self.idle_scene.store(scene.index(), Ordering::Relaxed);
            tracing::info!(?scene, "idle visualiser scene");
        }

        // The pulse only matters on the queue screen, and only while the
        // visualizer is not covering it. Freezing it there keeps the queue rows
        // from repainting when they are nowhere in sight, and leaves the phase
        // where it was for when the queue comes back.
        let queue_visible = screen == SCREEN_QUEUE && fade < 1.0;
        let frame = {
            let mut motion = self.lock_motion();
            motion.set_pulse_running(queue_visible);
            motion.update(dt)
        };

        let Some(ui) = self.ui.upgrade() else { return };
        if let Some(scene) = selected {
            ui.set_idle_scene(scene.index() as i32, scene.size());
        }
        ui.tick(&frame);
        ui.set_idle(visualizer, fade);
        self.visualizer_active
            .store(visualizer || fade > 0.0, Ordering::Relaxed);
        if fade_changed && fade <= 0.0 {
            // One frame more, so the player is painted back over the planet.
            ui.window().request_redraw();
        }

        if queue_visible {
            ui.set_pulse(frame.pulse);
        }

        if fade < 1.0 {
            let (progress, scrubbing) = {
                let model = self.lock();
                // While the finger is on the rim the preview wins, so the arc
                // and the head dot follow the drag instead of the clock.
                (
                    model.scrub.unwrap_or_else(|| model.clock.progress()),
                    model.scrub.is_some(),
                )
            };
            self.paint_ring(&ui, progress, scrubbing, dt);
        }
    }

    /// Draws the rim on the GPU, but only when what it shows has changed: a
    /// paused track repeats the same input and costs nothing. Runs on the UI
    /// thread, from the tick or from a scrub drag.
    ///
    /// The same pass paints the player's backdrop under the ring, so it runs
    /// whether a track is loaded or not. `dt` advances the fades: the ring in
    /// with the first track and out when playback stops, the glow up once
    /// connected.
    fn paint_ring(&self, ui: &AppWindow, progress: f32, scrubbing: bool, dt: f32) {
        let accent = self.lock_motion().accent();
        let connected = ui.global::<crate::ui::App>().get_auth() == AUTH_READY;

        let mut model = self.lock();
        let has_track = model.placeholder.is_some()
            || model
                .playback
                .as_ref()
                .and_then(|playback| playback.item.as_ref())
                .is_some();
        let previous = model.ring_input;
        let glow_target = if connected { 1.0 } else { GLOW_DIM };
        let input = ring::Input {
            // A ring fading out keeps the arc it had.
            progress: if has_track {
                progress
            } else {
                previous.map_or(0.0, |previous| previous.progress)
            },
            scrubbing,
            accent,
            ring: approach(
                previous.map_or(0.0, |previous| previous.ring),
                if has_track { 1.0 } else { 0.0 },
                dt / RING_FADE,
            ),
            glow: approach(
                previous.map_or(glow_target, |previous| previous.glow),
                glow_target,
                dt / GLOW_FADE,
            ),
        }
        .on_pixels();
        if model.ring_input.as_ref() == Some(&input) {
            return;
        }
        model.ring_input = Some(input);
        drop(model);

        let frame = self.ring_frame.fetch_add(1, Ordering::Relaxed) % ring::SLOTS;
        self.ring.render(&input, frame);
        ui.show_ring_frame(frame);
    }

    // ---- user feedback -------------------------------------------------

    fn report(self: &Arc<Self>, err: &anyhow::Error) {
        tracing::warn!(error = %err, "request failed");
        self.show_status(friendly(err));
    }

    fn show_status(self: &Arc<Self>, message: String) {
        let generation = self.status_generation.fetch_add(1, Ordering::SeqCst) + 1;
        {
            let message = message.clone();
            let _ = self
                .ui
                .upgrade_in_event_loop(move |ui| ui.set_status(&message, true));
        }

        let weak = self.ui.clone();
        let counter = self.status_generation.clone();
        self.spawn(async move {
            tokio::time::sleep(Duration::from_secs(4)).await;
            if counter.load(Ordering::SeqCst) == generation {
                let _ = weak.upgrade_in_event_loop(|ui| ui.set_status("", false));
            }
        });
    }
}

// ---------------------------------------------------------------------------
// Row view models
// ---------------------------------------------------------------------------

/// A list cover still to be loaded, and the row it belongs to.
struct CoverJob {
    index: usize,
    uri: String,
    url: String,
}

#[derive(Clone)]
struct PlaylistRow {
    title: String,
    subtitle: String,
    uri: String,
    cover: Option<String>,
}

#[derive(Clone)]
struct QueueRow {
    title: String,
    subtitle: String,
    duration: String,
    cover: Option<String>,
    uri: String,
    current: bool,
}

fn playlist_item(row: &PlaylistRow, cover: Option<&Art>) -> PlaylistItem {
    PlaylistItem {
        title: row.title.clone().into(),
        subtitle: row.subtitle.clone().into(),
        uri: row.uri.clone().into(),
        cover: cover.map(|art| art.image()).unwrap_or_default(),
    }
}

fn queue_row(track: &Track, current: bool) -> QueueRow {
    QueueRow {
        title: track.name.clone(),
        subtitle: track.artist_names(),
        duration: format::clock(track.duration_ms),
        cover: track.cover_url(QUEUE_ART_SIZE).map(str::to_owned),
        uri: track.uri.clone(),
        current,
    }
}

fn queue_item(row: &QueueRow, cover: Option<&Art>) -> QueueItem {
    QueueItem {
        title: row.title.clone().into(),
        subtitle: row.subtitle.clone().into(),
        duration: row.duration.clone().into(),
        cover: cover.map(|art| art.image()).unwrap_or_default(),
        current: row.current,
        uri: row.uri.clone().into(),
    }
}

/// Icon kind and human label for a Spotify Connect device type.
///
/// `CastVideo` and `CastAudio` are Google Cast endpoints: a Chromecast or a
/// Google Home. Spotify lists them, but does not hand out an id for them, so they
/// can be shown and not driven.
fn device_kind(kind: Option<&str>) -> (&'static str, &'static str) {
    match kind.unwrap_or_default().to_ascii_lowercase().as_str() {
        "computer" => ("computer", "Computer"),
        "smartphone" | "tablet" => ("phone", "Phone"),
        "speaker" => ("speaker", "Speaker"),
        "tv" => ("tv", "TV"),
        "castvideo" => ("tv", "Chromecast"),
        "castaudio" => ("speaker", "Chromecast audio"),
        "avr" | "stb" | "audiodongle" => ("speaker", "Hi-Fi"),
        "automobile" => ("car", "Car"),
        "game_console" => ("tv", "Console"),
        _ => ("speaker", "Device"),
    }
}

fn device_item(device: &Device) -> DeviceItem {
    let (icon, label) = device_kind(device.kind.as_deref());
    let id = device.connect_id().unwrap_or_default();
    let detail = if device.selectable() {
        label.to_owned()
    } else if device.is_restricted {
        format!("{label} · restricted")
    } else {
        format!("{label} · pick it in the Spotify app")
    };
    DeviceItem {
        name: device.name.clone().into(),
        kind: icon.into(),
        detail: detail.into(),
        active: device.is_active,
        selectable: device.selectable(),
        id: id.into(),
    }
}

/// Whether transport commands can reach the device playback is on.
///
/// Spotify documents `is_restricted` as "no Web API commands will be accepted by
/// this device" — a Chromecast that is casting counts as restricted — so the
/// controls are greyer out instead of failing on every press.
/// `from` moved toward `to` by at most `step`.
fn approach(from: f32, to: f32, step: f32) -> f32 {
    if from < to {
        (from + step).min(to)
    } else {
        (from - step).max(to)
    }
}

fn controls_available(state: &PlaybackState) -> bool {
    state.item.is_some()
        && state
            .device
            .as_ref()
            .map(|device| device.is_active && !device.is_restricted)
            .unwrap_or(true)
}

/// Spotify refuses a skip when it has nowhere to go, i.e. the user is already
/// at the start or the end of the playlist. That is not a failure worth an
/// alarming toast.
fn skip_restricted(err: &anyhow::Error) -> bool {
    matches!(
        err.downcast_ref::<ApiError>(),
        Some(ApiError::SkipRestricted)
    )
}

/// A revoked refresh token or an expired session: the user must authorize again.
fn needs_reauth(err: &anyhow::Error) -> bool {
    if matches!(
        err.downcast_ref::<ApiError>(),
        Some(ApiError::Spotify { status: 401, .. })
    ) {
        return true;
    }
    err.chain()
        .any(|cause| cause.to_string().contains("invalid_grant"))
}

fn friendly(err: &anyhow::Error) -> String {
    let message = match err.downcast_ref::<ApiError>() {
        Some(ApiError::NoActiveDevice) => "Open Spotify on a device, then try again".to_owned(),
        Some(ApiError::PremiumRequired) => "Playback control needs Spotify Premium".to_owned(),
        Some(ApiError::SkipRestricted) => "Nothing to skip to".to_owned(),
        Some(ApiError::RateLimited(_)) => "Spotify is busy, retrying shortly".to_owned(),
        Some(ApiError::Http(_)) => "Can't reach Spotify, check the network".to_owned(),
        _ => err.to_string(),
    };
    truncate(&message, 96)
}

fn truncate(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        text.to_owned()
    } else {
        let mut short: String = text.chars().take(max.saturating_sub(1)).collect();
        short.push('\u{2026}');
        short
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn playback_state(is_playing: bool) -> PlaybackState {
        PlaybackState {
            is_playing,
            progress_ms: Some(0),
            item: None,
            shuffle_state: false,
            repeat_state: "off".to_owned(),
            device: None,
            context: None,
            actions: None,
        }
    }

    fn track(uri: &str) -> Track {
        Track {
            name: uri.to_owned(),
            uri: uri.to_owned(),
            duration_ms: 180_000,
            artists: Vec::new(),
            album: None,
            show: None,
            images: None,
        }
    }

    fn device(id: Option<&str>, name: &str, kind: &str, is_active: bool) -> Device {
        Device {
            id: id.map(str::to_owned),
            name: name.to_owned(),
            kind: Some(kind.to_owned()),
            is_active,
            is_restricted: false,
        }
    }

    fn track_state(uri: &str) -> PlaybackState {
        PlaybackState {
            progress_ms: Some(30_000),
            item: Some(track(uri)),
            ..playback_state(true)
        }
    }

    fn queue_of(current: &str, upcoming: &[&str]) -> QueueResponse {
        QueueResponse {
            currently_playing: Some(track(current)),
            queue: upcoming.iter().map(|uri| track(uri)).collect(),
        }
    }

    fn playlist(name: &str, uri: &str, cover: &str) -> Playlist {
        serde_json::from_str(&format!(
            r#"{{"name":"{name}","uri":"{uri}","images":[{{"url":"{cover}","width":640}}]}}"#
        ))
        .expect("valid playlist fixture")
    }

    #[test]
    fn stale_polls_do_not_undo_a_local_pause() {
        let mut pending = Pending::default();
        pending.set_playing(false);

        // A poll that was already in flight still says "playing".
        let (playing, _, _, trust) = pending.resolve(&playback_state(true), Instant::now());
        assert!(!playing, "the local change must win until Spotify agrees");
        assert!(trust);

        // Once the poll agrees, the override is dropped.
        let (playing, _, _, _) = pending.resolve(&playback_state(false), Instant::now());
        assert!(!playing);
        assert!(pending.playing.is_none());
    }

    #[test]
    fn optimistic_values_expire() {
        let mut pending = Pending::default();
        pending.set_playing(false);

        let later = Instant::now() + OPTIMISTIC_GRACE + Duration::from_millis(1);
        let (playing, ..) = pending.resolve(&playback_state(true), later);
        assert!(playing, "after the grace period the polled value wins");
    }

    #[test]
    fn a_local_seek_hides_the_polled_position() {
        let mut pending = Pending::default();
        pending.set_position();
        let (_, _, _, trust) = pending.resolve(&playback_state(true), Instant::now());
        assert!(!trust);
    }

    #[test]
    fn shuffle_and_repeat_are_reconciled_too() {
        let mut pending = Pending::default();
        pending.set_shuffle(true);
        pending.set_repeat(RepeatMode::Track);

        let mut polled = playback_state(true);
        polled.repeat_state = "off".to_owned();

        let (_, shuffle, repeat, _) = pending.resolve(&polled, Instant::now());
        assert!(shuffle);
        assert_eq!(repeat, RepeatMode::Track);
    }

    #[test]
    fn a_late_poll_keeps_the_locally_paused_view() {
        let mut model = Model::new();
        model.playback = Some(playback_state(true));

        // What `cmd_toggle_play` does before talking to Spotify.
        model.clock.set_playing(false);
        model.pending.set_playing(false);
        if let Some(state) = model.playback.as_mut() {
            state.is_playing = false;
        }

        // The poll that was already in flight still reports the old state, and
        // this is the value the view is refreshed from.
        let stored = model.store_polled(playback_state(true));

        assert!(!stored.is_playing, "the icon must stay on pause");
        assert!(!model.clock.playing, "progress must not resume");

        // A poll that agrees releases the override.
        let stored = model.store_polled(playback_state(false));
        assert!(!stored.is_playing);
        assert!(model.pending.playing.is_none());
    }

    #[test]
    fn next_installs_the_upcoming_track() {
        let mut model = Model::new();
        model.playback = Some(track_state("a"));
        model.queue = Some(queue_of("a", &["b", "c"]));

        let upcoming = model
            .upcoming()
            .expect("the warm queue knows the next item");
        model.install_track(upcoming, Skip::Next);

        let view = model.now_playing();
        assert_eq!(view.title, "b", "the artwork must follow the press");
        assert_eq!(model.clock.position_ms(), 0);
        assert!(
            model.upcoming().is_some_and(|item| item.uri == "c"),
            "the queue must advance so next keeps working"
        );
    }

    #[test]
    fn a_stale_poll_keeps_the_optimistically_installed_track() {
        let mut model = Model::new();
        model.playback = Some(track_state("a"));

        model.install_track(track("b"), Skip::Next);

        // A poll sent before the skip finished still describes the old item.
        let stored = model.store_polled(track_state("a"));
        assert_eq!(
            stored.item.as_ref().map(|item| item.uri.as_str()),
            Some("b")
        );
        assert_eq!(
            model.clock.position_ms(),
            0,
            "the old position must not return"
        );

        // Once Spotify reports the new item the local copy is dropped.
        let stored = model.store_polled(track_state("b"));
        assert_eq!(
            stored.item.as_ref().map(|item| item.uri.as_str()),
            Some("b")
        );
        assert!(model.pending.track.is_none());
    }

    #[test]
    fn upcoming_ignores_a_stale_queue() {
        let mut model = Model::new();
        model.playback = Some(track_state("a"));
        model.queue = Some(queue_of("z", &["b"]));

        assert!(
            model.upcoming().is_none(),
            "a queue that disagrees with the player must not be used"
        );
    }

    #[test]
    fn reinstalling_the_current_track_is_a_no_op() {
        let mut model = Model::new();
        model.playback = Some(track_state("a"));

        assert!(!model.install_track(track("a"), Skip::Next));
        assert!(model.pending.track.is_none(), "no poll to wait for");
    }

    #[test]
    fn stale_polls_do_not_undo_the_latest_skip() {
        let mut model = Model::new();
        model.playback = Some(track_state("a"));
        model.queue = Some(queue_of("a", &["b", "c", "d"]));

        let b = model.upcoming().expect("queue knows the next item");
        model.install_track(b, Skip::Next);
        let c = model.upcoming().expect("queue advanced");
        model.install_track(c, Skip::Next);

        // Polls sent during the earlier presses must not win.
        for stale in ["a", "b"] {
            let stored = model.store_polled(track_state(stale));
            assert_eq!(
                stored.item.map(|item| item.uri),
                Some("c".to_owned()),
                "poll for {stale} must be ignored"
            );
        }

        // The poll that reports the track we asked for settles it.
        let stored = model.store_polled(track_state("c"));
        assert_eq!(stored.item.map(|item| item.uri), Some("c".to_owned()));
        assert!(model.pending.track.is_none());
    }

    #[test]
    fn starting_a_playlist_disables_previous_until_spotify_answers() {
        let mut model = Model::new();
        model.playback = Some(track_state("a"));
        model.can_previous = true;
        model.can_next = true;

        let chill = playlist("Chill", "spotify:playlist:1", "https://art/chill.jpg");
        model.show_playlist(&chill);
        assert!(
            !model.can_previous,
            "a new playlist starts at its first item"
        );
        assert!(model.can_next);

        // A poll that still describes the old track must not re-enable it.
        let stale: PlaybackState = serde_json::from_str(
            r#"{"is_playing":true,"item":{"name":"a","uri":"a"},
                "actions":{"disallows":{"skipping_prev":false}}}"#,
        )
        .unwrap();
        model.store_polled(stale);
        assert!(!model.can_previous);

        // Once the first track arrives, its own restriction wins.
        let arrived: PlaybackState = serde_json::from_str(
            r#"{"is_playing":true,"item":{"name":"x","uri":"x"},
                "actions":{"disallows":{"skipping_prev":true}}}"#,
        )
        .unwrap();
        model.store_polled(arrived);
        assert!(!model.can_previous);
        assert!(model.can_next);
    }

    #[test]
    fn a_refused_skip_dims_the_button_for_that_item_only() {
        let mut model = Model::new();
        model.playback = Some(track_state("a"));
        model.can_previous = true;
        model.can_next = true;

        model.refuse_skip(Skip::Previous);
        assert!(!model.can_skip(Skip::Previous));
        assert!(
            model.can_skip(Skip::Next),
            "the other direction is unaffected"
        );

        // Staying on the same item keeps it dim, even when the poll says nothing
        // about the restriction.
        model.store_polled(track_state("a"));
        assert!(!model.can_skip(Skip::Previous));

        // A new item clears it.
        model.playback = Some(track_state("b"));
        model.store_polled(track_state("b"));
        assert!(model.can_skip(Skip::Previous));
    }

    #[test]
    fn the_context_uri_is_read_from_the_player_state() {
        let mut model = Model::new();
        let mut state = track_state("a");
        assert!(model.context_uri().is_none(), "no context is playing");

        state.context = Some(
            serde_json::from_str(r#"{"uri":"spotify:playlist:x","type":"playlist"}"#).unwrap(),
        );
        model.playback = Some(state);
        assert_eq!(model.context_uri().as_deref(), Some("spotify:playlist:x"));
    }

    #[test]
    fn jumping_to_a_queued_item_consumes_the_earlier_ones() {
        let mut model = Model::new();
        model.playback = Some(track_state("a"));
        model.queue = Some(queue_of("a", &["b", "c", "d"]));

        let target = model.queue.as_ref().unwrap().queue[1].clone();
        assert!(model.install_track(target, Skip::Jump));

        assert_eq!(model.now_playing().title, "c");
        assert_eq!(
            model.upcoming().map(|item| item.uri),
            Some("d".to_owned()),
            "the queue carries on after the tapped item"
        );
        assert_eq!(
            model.previous_track().map(|item| item.uri),
            Some("a".to_owned()),
            "the item we left is still reachable with previous"
        );
    }

    #[test]
    fn going_back_and_forward_returns_to_the_same_track() {
        let mut model = Model::new();
        model.playback = Some(track_state("b"));
        model.queue = Some(queue_of("b", &["c", "d"]));
        model.remember(track("a"));

        // previous: b -> a
        let a = model.previous_track().expect("history knows the last item");
        assert!(model.install_track(a, Skip::Previous));
        assert_eq!(model.now_playing().title, "a");

        // next must go back to b, not skip on to c
        let next = model
            .upcoming()
            .expect("the queue knows where we came from");
        assert_eq!(next.uri, "b");
        assert!(model.install_track(next, Skip::Next));
        assert_eq!(model.now_playing().title, "b");
        assert_eq!(
            model.upcoming().map(|item| item.uri),
            Some("c".to_owned()),
            "the queue carries on from b"
        );
    }

    #[test]
    fn previous_walks_back_through_history() {
        let mut model = Model::new();
        model.playback = Some(track_state("a"));
        model.install_track(track("b"), Skip::Next);

        assert_eq!(
            model.previous_track().map(|item| item.uri),
            Some("a".to_owned())
        );
        assert!(model.previous_track().is_none());
    }

    #[test]
    fn starting_a_playlist_shows_its_cover_until_a_track_arrives() {
        let mut model = Model::new();
        model.playback = Some(track_state("a"));
        let chill = playlist("Chill", "spotify:playlist:1", "https://art/chill.jpg");

        model.show_playlist(&chill);
        let view = model.now_playing();
        assert_eq!(view.title, "Chill");
        assert_eq!(view.playlist_uri, "spotify:playlist:1");
        assert_eq!(view.cover_url.as_deref(), Some("https://art/chill.jpg"));
        assert!(view.has_track);

        // Spotify has not switched yet.
        model.store_polled(track_state("a"));
        assert_eq!(model.now_playing().title, "Chill");

        // The first track of the playlist arrives.
        let stored = model.store_polled(track_state("x"));
        model.playback = Some(stored);
        assert!(model.placeholder.is_none());
        assert_eq!(model.now_playing().title, "x");
    }

    #[test]
    fn clock_interpolates_and_clamps() {
        let mut clock = Clock::new();
        clock.duration_ms = 60_000;
        clock.base_ms = 30_000;
        clock.playing = true;
        clock.started = Instant::now() - Duration::from_secs(10);
        assert_eq!(clock.position_ms(), 40_000);
        assert!((clock.progress() - 2.0 / 3.0).abs() < 0.001);

        clock.base_ms = 59_000;
        clock.started = Instant::now() - Duration::from_secs(5);
        assert_eq!(clock.position_ms(), 59_999);
    }

    #[test]
    fn paused_clock_does_not_move() {
        let mut clock = Clock::new();
        clock.duration_ms = 60_000;
        clock.base_ms = 1_000;
        clock.playing = false;
        assert_eq!(clock.position_ms(), 1_000);
        clock.set_playing(true);
        clock.set_playing(false);
        assert!(clock.position_ms() >= 1_000);
    }

    #[test]
    fn seek_moves_the_clock() {
        let mut clock = Clock::new();
        clock.duration_ms = 200_000;
        clock.set_ratio(0.5);
        assert_eq!(clock.position_ms(), 100_000);
    }

    #[test]
    fn a_late_poll_does_not_drag_the_clock_backwards() {
        let mut clock = Clock::new();
        clock.duration_ms = 180_000;
        clock.base_ms = 0;
        clock.playing = true;
        clock.started = Instant::now() - Duration::from_secs(20);
        assert_eq!(clock.position_ms(), 20_000);

        // Spotify answers with the position it read a moment before replying.
        let mut state = track_state("x");
        state.progress_ms = Some(19_600);
        clock.apply(&state, true);
        assert_eq!(clock.position_ms(), 20_000);

        // A real seek still re-anchors the ring.
        state.progress_ms = Some(120_000);
        clock.apply(&state, true);
        assert_eq!(clock.position_ms(), 120_000);
    }

    #[test]
    fn the_visualizer_waits_for_the_pause_to_settle() {
        let mut model = Model::new();
        let now = Instant::now();
        model.note_playback(false, true, now);
        assert!(!model.wants_visualizer(now));
        assert!(!model.wants_visualizer(now + PAUSED_IDLE - Duration::from_secs(1)));
        assert!(model.wants_visualizer(now + PAUSED_IDLE));

        model.note_playback(true, true, now);
        assert!(!model.wants_visualizer(now + PAUSED_IDLE * 2));
    }

    #[test]
    fn nothing_playing_brings_the_visualizer_up_sooner() {
        let mut model = Model::new();
        let now = Instant::now();
        model.note_playback(false, false, now);
        assert!(!model.wants_visualizer(now));
        assert!(model.wants_visualizer(now + STOPPED_IDLE));
    }

    #[test]
    fn a_dismissed_visualizer_stays_away_until_playback_returns() {
        let mut model = Model::new();
        let now = Instant::now();
        model.note_playback(false, false, now);
        let later = now + STOPPED_IDLE * 5;
        assert!(model.wants_visualizer(later));

        model.idle.dismissed = true;
        assert!(!model.wants_visualizer(later));

        model.note_playback(true, true, later);
        model.note_playback(false, false, later);
        assert!(!model.wants_visualizer(later));
        assert!(model.wants_visualizer(later + STOPPED_IDLE));
    }

    #[test]
    fn polls_do_not_close_a_manually_opened_visualizer() {
        let mut model = Model::new();
        let now = Instant::now();
        model.note_playback(true, true, now);
        model.idle.manual = true;

        // The next polls still report the same playing track.
        model.note_playback(true, true, now + Duration::from_secs(1));
        assert!(model.wants_visualizer(now + Duration::from_secs(2)));

        // A real pause and resume does close it again.
        model.note_playback(false, true, now + Duration::from_secs(3));
        model.note_playback(true, true, now + Duration::from_secs(4));
        assert!(!model.wants_visualizer(now + Duration::from_secs(5)));
    }

    #[test]
    fn long_messages_are_shortened() {
        let message = "x".repeat(200);
        let short = truncate(&message, 20);
        assert_eq!(short.chars().count(), 20);
        assert!(short.ends_with('\u{2026}'));
    }

    #[test]
    fn missing_device_disables_controls() {
        let state: PlaybackState = serde_json::from_str(
            r#"{"is_playing":true,"item":{"name":"x","uri":"spotify:track:x","duration_ms":1000},
                "device":{"name":"Phone","is_active":false}}"#,
        )
        .unwrap();
        assert!(!controls_available(&state));
    }

    /// A Chromecast casting Spotify comes back with `id: null` and
    /// `is_restricted: true`; the player must still decode it and stop offering
    /// commands that Spotify would refuse.
    #[test]
    fn a_restricted_cast_device_decodes_and_disables_controls() {
        let state: PlaybackState = serde_json::from_str(
            r#"{"is_playing":true,
                "item":{"name":"x","uri":"spotify:track:x","duration_ms":1000},
                "device":{"id":null,"name":"Chromecast","type":"CastVideo",
                          "is_active":true,"is_restricted":true,
                          "volume_percent":null}}"#,
        )
        .unwrap();

        let device = state.device.as_ref().expect("the device is reported");
        assert_eq!(device.name, "Chromecast");
        assert!(device.connect_id().is_none());
        assert!(!device.selectable());
        assert!(
            !controls_available(&state),
            "a restricted device refuses Web API commands"
        );
    }

    #[test]
    fn devices_map_to_icon_kinds() {
        assert_eq!(device_kind(Some("Computer")).0, "computer");
        assert_eq!(device_kind(Some("Smartphone")).0, "phone");
        assert_eq!(device_kind(Some("Speaker")), ("speaker", "Speaker"));
        assert_eq!(device_kind(Some("TV")), ("tv", "TV"));
        assert_eq!(
            device_kind(Some("castvideo")),
            ("tv", "Chromecast"),
            "the type's case is not fixed by the API"
        );
        assert_eq!(
            device_kind(Some("CastAudio")),
            ("speaker", "Chromecast audio")
        );
        assert_eq!(device_kind(Some("AVR")).0, "speaker");
        assert_eq!(device_kind(Some("STB")).1, "Hi-Fi");
        assert_eq!(
            device_kind(Some("Something new")),
            ("speaker", "Device"),
            "an unknown kind still gets a usable icon"
        );
        assert_eq!(device_kind(None), ("speaker", "Device"), "no type at all");
    }

    #[test]
    fn the_picker_lists_devices_it_cannot_pick() {
        let mut model = Model::new();
        model.devices = vec![
            device(Some("kitchen"), "Kitchen", "Speaker", true),
            device(None, "Chromecast", "CastVideo", false),
            device(Some("phone"), "Phone", "Smartphone", false),
        ];

        let rows = model.device_rows();
        assert_eq!(
            rows.len(),
            3,
            "a device without an id must still be listed, or the picker looks broken"
        );
        assert!(rows[0].active && rows[0].selectable);
        assert!(
            !rows[1].selectable,
            "the Chromecast cannot be transferred to"
        );
        assert_eq!(
            rows[1].detail.as_str(),
            "Chromecast · pick it in the Spotify app"
        );
        assert_eq!(rows[1].kind.as_str(), "tv");
        assert!(rows[2].selectable);
    }

    #[test]
    fn a_restricted_device_is_listed_but_not_pickable() {
        let mut model = Model::new();
        let mut restricted = device(Some("tv"), "Lounge", "TV", false);
        restricted.is_restricted = true;
        model.devices = vec![restricted];

        let rows = model.device_rows();
        assert_eq!(rows.len(), 1);
        assert!(!rows[0].selectable);
        assert_eq!(rows[0].detail.as_str(), "TV · restricted");
    }

    #[test]
    fn the_picker_follows_the_playing_device() {
        let mut model = Model::new();
        let mut state = playback_state(true);
        state.device = Some(device(Some("tv"), "Lounge", "TV", true));
        model.playback = Some(state);
        model.devices = vec![
            device(Some("kitchen"), "Kitchen", "Speaker", true),
            device(Some("tv"), "Lounge", "TV", false),
        ];

        let rows = model.device_rows();
        assert!(!rows[0].active, "the stale list must not win");
        assert!(rows[1].active);
    }

    #[test]
    fn transferring_names_the_new_device_before_the_poll() {
        let mut model = Model::new();
        model.playback = Some(playback_state(true));
        model.devices = vec![
            device(Some("kitchen"), "Kitchen", "Speaker", true),
            device(Some("tv"), "Lounge", "TV", false),
        ];

        let target = model.devices[1].clone();
        model.focus_device(&target);

        let view = model.playback.as_ref().unwrap().device.as_ref().unwrap();
        assert_eq!(view.name, "Lounge");
        assert!(
            view.is_active,
            "the new device is where playback is about to run"
        );
    }
}
