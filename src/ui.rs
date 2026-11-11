//! Slint view layer.
//!
//! Every property write lives here so the controller can stay free of layout
//! details, and so no `slint::Image` is ever constructed off the UI thread.

slint::include_modules!();

use std::rc::Rc;

use anyhow::{Context, Result};
use slint::wgpu_30::wgpu;
use slint::{ComponentHandle as _, Model as _, ModelRc, VecModel};

use crate::artwork::Art;
use crate::motion::Frame;

/// Create the window.
///
/// The graphics backend is already chosen by then: [`crate::gpu::install`] hands
/// Slint a wgpu device on Vulkan before this runs, so Skia draws the UI and the
/// idle globe through the same GPU.
pub fn create_window(fullscreen: bool, show_fps: bool) -> Result<AppWindow> {
    let window = AppWindow::new().context("creating the Slint window")?;
    window.window().set_fullscreen(fullscreen);
    // Full screen is the kiosk, and a kiosk is driven by touch: no pointer.
    window.global::<Pointer>().set_hidden(fullscreen);
    window.global::<Stats>().set_shown(show_fps);
    let weak = window.as_weak();
    window.global::<Stats>().on_save_place(move || {
        if let Some(window) = weak.upgrade() {
            save_place(&window);
        }
    });
    watch_rendering_if_asked(&window);
    Ok(window)
}

/// Append where the dive is (`Stats.place`) to `places.txt` beside the
/// config file, with the time, so a spot met on a real run can be pointed
/// out later; the readout says whether it was kept.
fn save_place(window: &AppWindow) {
    let stats = window.global::<Stats>();
    let place = stats.get_place();
    if place.is_empty() {
        stats.set_saved("nothing to save".into());
        return;
    }
    let config = window.global::<App>().get_config_path();
    let file = std::path::Path::new(config.as_str())
        .parent()
        .map_or_else(|| "places.txt".into(), |dir| dir.join("places.txt"));
    let unix = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    let line = format!("unix {unix} · {place}\n");
    let written = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&file)
        .and_then(|mut f| std::io::Write::write_all(&mut f, line.as_bytes()));
    match written {
        Ok(()) => {
            tracing::info!(file = %file.display(), %place, "place saved");
            stats.set_saved("saved".into());
        }
        Err(err) => {
            tracing::warn!(%err, file = %file.display(), "saving the place failed");
            stats.set_saved("not saved".into());
        }
    }
}

thread_local! {
    static WATCHING: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Start counting frames once the readout is first shown, or from the start
/// when measuring at full speed.
///
/// Not before: while a rendering notifier is installed, Slint submits the
/// window clear in a render pass of its own, ahead of the frame proper, and on
/// the Pi's tile-based GPU every pass moves the whole framebuffer through
/// memory. A notifier cannot be removed again, so once the readout has been
/// shown the count goes on.
pub fn watch_rendering_if_asked(window: &AppWindow) {
    if WATCHING.get() {
        return;
    }
    let full_speed = std::env::var_os("SPOTIFY_ROUND_FULL_SPEED").is_some();
    if !full_speed && !window.global::<Stats>().get_shown() {
        return;
    }
    WATCHING.set(true);
    watch_rendering(window);
}

/// Say in the log which graphics API draws the frames and count the frames
/// presented for the frame-rate readout, with what each cost: the UI thread's
/// time from the start of a frame to its end, and the GPU's.
///
/// The player only draws when something changed, mostly the progress ring
/// moving half a pixel, so its frame rate follows the track's length rather
/// than the hardware. The milliseconds say how much of the 16.7 ms budget a
/// frame really takes.
///
/// With `SPOTIFY_ROUND_FULL_SPEED` set, every frame asks for the next at once,
/// whether anything changed or not, and the rate is logged each second: how
/// fast each screen can be drawn at all.
fn watch_rendering(window: &AppWindow) {
    let weak = window.as_weak();
    let full_speed = std::env::var_os("SPOTIFY_ROUND_FULL_SPEED").is_some();
    let mut frames = 0;
    let mut since = std::time::Instant::now();
    let mut started = None;
    let mut busy = std::time::Duration::ZERO;
    let mut gpu = crate::gpu::GpuLoad::new();
    let result = window
        .window()
        .set_rendering_notifier(move |state, api| match state {
            slint::RenderingState::RenderingSetup => {
                tracing::info!(renderer = ?api, "graphics context ready");
            }
            slint::RenderingState::BeforeRendering => {
                started = Some(std::time::Instant::now());
            }
            slint::RenderingState::AfterRendering => {
                frames += 1;
                if let Some(started) = started.take() {
                    busy += started.elapsed();
                }
                if full_speed && let Some(window) = weak.upgrade() {
                    window.window().request_redraw();
                }
                if since.elapsed() >= std::time::Duration::from_secs(1) {
                    let rate = frames;
                    let ms = busy.as_secs_f32() * 1000.0 / frames.max(1) as f32;
                    // The V3D's busy time covers every pass of the second: the
                    // ring, the UI and the compositor's.
                    let gpu_ms = gpu.per_frame(frames as u32).unwrap_or(-1.0);
                    (frames, since, busy) = (0, std::time::Instant::now(), Default::default());
                    // Written after the frame, so it is shown with the next one.
                    // Hidden, it is left alone, so it wakes nothing.
                    if let Some(window) = weak.upgrade() {
                        if full_speed {
                            let app = window.global::<App>();
                            let idle = window.global::<Idle>();
                            let stats = window.global::<Stats>();
                            tracing::info!(
                                fps = rate,
                                screen = app.get_screen(),
                                idle = idle.get_active(),
                                "full speed"
                            );
                            // The kiosk's stdout is its console, not the
                            // journal; a one-line status file is easier to read
                            // while measuring on the Pi.
                            let line = format!(
                                "{} {} {} {} {:.1} {:.1}\n",
                                rate,
                                app.get_screen(),
                                idle.get_active(),
                                stats.get_scene(),
                                stats.get_scene_ms(),
                                stats.get_scene_gpu_ms(),
                            );
                            let _ = std::fs::write("/tmp/spotify-round-fps", line);
                        }
                        let stats = window.global::<Stats>();
                        if stats.get_shown() {
                            stats.set_display(rate);
                            stats.set_display_ms(ms);
                            stats.set_display_gpu_ms(gpu_ms);
                        }
                    }
                }
            }
            _ => {}
        });
    if let Err(err) = result {
        tracing::warn!(%err, "the renderer cannot report its graphics API");
    }
}

/// View helpers on the generated window handle.
///
/// These live in a trait so closures handed to `upgrade_in_event_loop` can call
/// them directly on the `AppWindow` they receive.
pub trait WindowExt: Sized {
    fn set_config_path(&self, path: &str);
    fn set_auth_state(&self, state: i32, detail: &str);
    fn set_auth_url(&self, url: &str, qr: Option<&Art>);
    fn reveal_app(&self);
    fn set_screen(&self, screen: i32);
    fn set_status(&self, message: &str, visible: bool);
    fn set_track_text(&self, title: &str, subtitle: &str, has_track: bool);
    /// The idle card's lines under the title: the artists, the album, and the
    /// playlist the track plays from.
    fn set_card_text(&self, artist: &str, album: &str, context: &str);
    /// Whether the track is in Liked Songs; `None` while that is not known, or
    /// for an item that cannot be saved.
    fn set_liked(&self, liked: Option<bool>);
    fn set_cover(&self, art: Option<&Art>);
    fn set_playback(&self, playing: bool, controls: bool, shuffle: bool, repeat: i32);
    fn set_skip_state(&self, can_previous: bool, can_next: bool);
    /// Drive the queue's current-track pulse. Only written while the queue is
    /// on screen.
    fn set_pulse(&self, value: f32);
    /// Wrap the ring's render targets as images, once, so the renderer can cache
    /// them; the frames are then shown by index.
    fn set_ring_frames(&self, textures: Vec<wgpu::Texture>);
    /// Show one of the frames installed by `set_ring_frames`.
    fn show_ring_frame(&self, index: usize);
    /// Hand the ring one frame of pixels. Only the design previews need this:
    /// the running display shares textures instead.
    #[cfg(test)]
    fn set_ring_frame_image(&self, frame: slint::Image);
    /// Push the idle visualizer state: whether it is showing and its 0..1
    /// cross-fade.
    fn set_idle(&self, active: bool, fade: f32);
    /// Select a visualisation and the size its frame should be drawn at.
    fn set_idle_scene(&self, scene: i32, size: f32);
    /// Hand the visualizer one frame. Only the design previews need this: the
    /// running display shares textures instead of pixels.
    #[cfg(test)]
    fn set_idle_frame(&self, frame: slint::Image);
    /// Wrap every scene's render targets as images, once, so the renderer can
    /// cache them; the frames are then shown by index.
    fn set_idle_frames(&self, textures: Vec<Option<slint::wgpu_30::wgpu::Texture>>);
    /// Show one of the frames installed by `set_idle_frames`.
    fn show_idle_frame(&self, index: usize);
    /// The visualizer's frames in the last second, the mean time its thread
    /// spent on each and the mean time each took on the GPU, in ms.
    fn set_scene_stats(&self, frames: i32, ms: f32, gpu_ms: f32, place: &str);
    /// The seas of the dive's circuit in order, and the one it is in or bound
    /// for; none for a scene without a journey.
    fn set_seas(&self, names: &[&str], here: usize);
    /// The text a scene lays over its frame; empty strings hide it.
    fn set_idle_hud(&self, hud: &crate::solar::Hud);
    /// The labels a scene lays on the worlds in its frame; none hides them.
    fn set_idle_labels(&self, labels: &[crate::solar::labels::BodyLabel]);
    fn tick(&self, frame: &Frame);
    /// Name, icon kind and presence of the Spotify Connect device playback is
    /// running on.
    fn set_device(&self, name: &str, kind: &str, has_device: bool);
    /// Fill the device picker.
    fn set_devices(&self, rows: Vec<DeviceItem>);
    fn set_devices_open(&self, open: bool);
    fn set_devices_loading(&self, loading: bool);
    fn set_devices_empty(&self, message: &str);
    fn set_playlists(&self, rows: Vec<PlaylistItem>, subtitle: &str);
    fn set_playlists_loading(&self, loading: bool);
    fn set_playlists_error(&self, message: &str);
    fn set_current_playlist(&self, uri: &str);
    /// Give one playlist row its cover, in place. Skipped when the list was
    /// replaced since and `index` is no longer that playlist.
    fn set_playlist_cover(&self, index: usize, uri: &str, art: &Art);
    fn set_queue(&self, rows: Vec<QueueItem>, subtitle: &str, reset: bool);
    /// Give one queue row its cover, in place, like `set_playlist_cover`.
    fn set_queue_cover(&self, index: usize, uri: &str, art: &Art);
    fn set_queue_loading(&self, loading: bool);
    fn set_queue_error(&self, message: &str);
}

impl WindowExt for AppWindow {
    // ---- authentication ---------------------------------------------

    fn set_config_path(&self, path: &str) {
        self.global::<App>().set_config_path(path.into());
    }

    fn set_auth_state(&self, state: i32, detail: &str) {
        let app = self.global::<App>();
        app.set_auth(state);
        app.set_auth_detail(detail.into());
        if state != 1 {
            app.set_qr_ready(false);
        }
    }

    fn set_auth_url(&self, url: &str, qr: Option<&Art>) {
        let app = self.global::<App>();
        app.set_auth_url(url.into());
        match qr {
            Some(art) => {
                app.set_qr(art.image());
                app.set_qr_ready(true);
            }
            None => app.set_qr_ready(false),
        }
    }

    /// Fade the main UI in once the session is ready.
    fn reveal_app(&self) {
        self.global::<App>().set_ready(true);
    }

    // ---- navigation and status --------------------------------------

    fn set_screen(&self, screen: i32) {
        self.global::<App>().set_screen(screen);
    }

    fn set_status(&self, message: &str, visible: bool) {
        let app = self.global::<App>();
        app.set_status(message.into());
        app.set_status_visible(visible);
    }

    // ---- now playing -------------------------------------------------

    fn set_track_text(&self, title: &str, subtitle: &str, has_track: bool) {
        let player = self.global::<Player>();
        player.set_title(title.into());
        player.set_subtitle(subtitle.into());
        player.set_has_track(has_track);
    }

    fn set_card_text(&self, artist: &str, album: &str, context: &str) {
        let player = self.global::<Player>();
        player.set_artist(artist.into());
        player.set_album(album.into());
        player.set_context(context.into());
    }

    fn set_liked(&self, liked: Option<bool>) {
        let player = self.global::<Player>();
        player.set_liked(liked.unwrap_or(false));
        player.set_can_like(liked.is_some());
    }

    fn set_cover(&self, art: Option<&Art>) {
        let player = self.global::<Player>();
        match art {
            Some(art) => {
                player.set_cover(art.image());
                player.set_has_cover(true);
            }
            None => {
                player.set_cover(slint::Image::default());
                player.set_has_cover(false);
            }
        }
    }

    fn set_playback(&self, playing: bool, controls: bool, shuffle: bool, repeat: i32) {
        let player = self.global::<Player>();
        player.set_playing(playing);
        player.set_controls(controls);
        player.set_shuffle(shuffle);
        player.set_repeat(repeat);
    }

    fn set_skip_state(&self, can_previous: bool, can_next: bool) {
        let player = self.global::<Player>();
        player.set_can_previous(can_previous);
        player.set_can_next(can_next);
    }

    fn set_idle(&self, active: bool, fade: f32) {
        let idle = self.global::<Idle>();
        idle.set_active(active);
        idle.set_fade(fade);
    }

    fn set_idle_scene(&self, scene: i32, size: f32) {
        let idle = self.global::<Idle>();
        idle.set_scene(scene);
        idle.set_frame_size(size);
        // A new visit starts with a different scene: drop the previous frame so
        // the old one is never stretched to the new scene's size while the
        // renderer draws the first new frame.
        idle.set_frame(slint::Image::default());
    }

    #[cfg(test)]
    fn set_idle_frame(&self, frame: slint::Image) {
        self.global::<Idle>().set_frame(frame);
    }

    fn set_idle_frames(&self, textures: Vec<Option<wgpu::Texture>>) {
        let frames = textures
            .into_iter()
            .map(|texture| match texture {
                Some(texture) => match slint::Image::try_from(texture) {
                    Ok(image) => image,
                    Err(err) => {
                        tracing::warn!(%err, "cannot wrap a visualizer frame");
                        slint::Image::default()
                    }
                },
                // A scene whose renderer did not come up keeps its slot, so the
                // flat indices of the other scenes never move.
                None => slint::Image::default(),
            })
            .collect::<Vec<_>>();
        let idle = self.global::<Idle>();
        idle.set_frames(slint::ModelRc::new(VecModel::from(frames)));
    }

    fn set_scene_stats(&self, frames: i32, ms: f32, gpu_ms: f32, place: &str) {
        let stats = self.global::<Stats>();
        if stats.get_shown() {
            stats.set_scene(frames);
            stats.set_scene_ms(ms);
            stats.set_scene_gpu_ms(gpu_ms);
        }
        // Kept while hidden too, so a long press saves where the dive is now.
        stats.set_place(place.into());
    }

    fn set_seas(&self, names: &[&str], here: usize) {
        let idle = self.global::<Idle>();
        let names: Vec<slint::SharedString> = names.iter().map(|&n| n.into()).collect();
        idle.set_seas(slint::ModelRc::new(VecModel::from(names)));
        idle.set_sea(here as i32);
    }

    fn set_idle_hud(&self, hud: &crate::solar::Hud) {
        let idle = self.global::<Idle>();
        idle.set_hud_title(hud.title.as_str().into());
        let lines: Vec<HudLine> = hud
            .lines
            .iter()
            .map(|(label, value)| HudLine {
                label: label.as_str().into(),
                value: value.as_str().into(),
            })
            .collect();
        idle.set_hud_lines(ModelRc::from(Rc::new(VecModel::from(lines))));
        idle.set_hud_survey(hud.survey);
        let scans: Vec<ScanRow> = hud
            .scans
            .iter()
            .map(|scan| ScanRow {
                name: scan.name.as_str().into(),
                progress: scan.progress,
                value: scan.value.as_str().into(),
            })
            .collect();
        idle.set_hud_scans(ModelRc::from(Rc::new(VecModel::from(scans))));
        let manoeuvre = hud.manoeuvre.clone().unwrap_or_default();
        idle.set_man_name(manoeuvre.name.as_str().into());
        idle.set_man_text(manoeuvre.text.as_str().into());
        idle.set_man_progress(if manoeuvre.name.is_empty() {
            -1.0
        } else {
            manoeuvre.progress
        });
        idle.set_man_dv(manoeuvre.dv);
        idle.set_man_done(manoeuvre.done());
        idle.set_man_throttle(manoeuvre.throttle());
        let stages: Vec<HudLine> = manoeuvre
            .stages
            .iter()
            .map(|(label, value)| HudLine {
                label: label.as_str().into(),
                value: value.as_str().into(),
            })
            .collect();
        idle.set_man_stages(ModelRc::from(Rc::new(VecModel::from(stages))));
        let (stage, through) = manoeuvre.stage();
        idle.set_man_stage(stage as i32);
        idle.set_man_stage_progress(through);
        let map = hud.map.clone().unwrap_or_default();
        idle.set_map_orbits(map.orbits.as_str().into());
        idle.set_map_route(map.route.as_str().into());
        idle.set_map_ship_x(map.ship[0]);
        idle.set_map_ship_y(map.ship[1]);
        let dots: Vec<MapDot> = map
            .dots
            .iter()
            .map(|d| MapDot {
                x: d.at[0],
                y: d.at[1],
                size: d.size,
                color: slint::Color::from_rgb_u8(d.colour[0], d.colour[1], d.colour[2]),
                label: d.label.as_str().into(),
                current: d.current,
            })
            .collect();
        idle.set_map_dots(ModelRc::from(Rc::new(VecModel::from(dots))));
    }

    fn set_idle_labels(&self, labels: &[crate::solar::labels::BodyLabel]) {
        let rows: Vec<WorldLabel> = labels
            .iter()
            .map(|l| WorldLabel {
                x: l.at[0],
                y: l.at[1],
                left: l.left,
                name: l.name.as_str().into(),
                distance: l.distance.as_str().into(),
            })
            .collect();
        // They move every frame: rows are updated in place while their
        // number holds, so the repeater keeps its items.
        let idle = self.global::<Idle>();
        let model = idle.get_labels();
        if model.row_count() == rows.len() {
            for (i, row) in rows.into_iter().enumerate() {
                if model.row_data(i).as_ref() != Some(&row) {
                    model.set_row_data(i, row);
                }
            }
        } else {
            idle.set_labels(ModelRc::from(Rc::new(VecModel::from(rows))));
        }
    }

    fn show_idle_frame(&self, index: usize) {
        let idle = self.global::<Idle>();
        if let Some(frame) = idle.get_frames().row_data(index) {
            idle.set_frame(frame);
        }
    }

    fn set_pulse(&self, value: f32) {
        self.global::<Player>().set_pulse(value);
    }

    fn set_ring_frames(&self, textures: Vec<wgpu::Texture>) {
        let frames = textures
            .into_iter()
            .filter_map(|texture| match slint::Image::try_from(texture) {
                Ok(image) => Some(image),
                Err(err) => {
                    tracing::warn!(%err, "cannot wrap a ring frame");
                    None
                }
            })
            .collect::<Vec<_>>();
        let player = self.global::<Player>();
        player.set_ring_frames(ModelRc::new(VecModel::from(frames)));
    }

    fn show_ring_frame(&self, index: usize) {
        let player = self.global::<Player>();
        if let Some(frame) = player.get_ring_frames().row_data(index) {
            player.set_ring_frame(frame);
        }
    }

    #[cfg(test)]
    fn set_ring_frame_image(&self, frame: slint::Image) {
        self.global::<Player>().set_ring_frame(frame);
    }

    /// Push one animation frame produced by [`crate::motion::Motion`].
    fn tick(&self, frame: &Frame) {
        // The F key shows the readout from the Slint side; start counting then.
        watch_rendering_if_asked(self);
        let theme = self.global::<Theme>();
        theme.set_accent(frame.accent.color());
        theme.set_glow(frame.accent.with_alpha(0x55));
        theme.set_glow_soft(frame.accent.with_alpha(0x1c));
        theme.set_spin(frame.spin);

        self.global::<Player>().set_reveal(frame.reveal);
    }

    // ---- devices -----------------------------------------------------

    fn set_device(&self, name: &str, kind: &str, has_device: bool) {
        let player = self.global::<Player>();
        player.set_device(name.into());
        player.set_device_kind(kind.into());
        player.set_has_device(has_device);
    }

    fn set_devices(&self, rows: Vec<DeviceItem>) {
        let devices = self.global::<Devices>();
        devices.set_devices(ModelRc::from(Rc::new(VecModel::from(rows))));
        devices.set_loading(false);
        devices.set_empty("".into());
    }

    fn set_devices_open(&self, open: bool) {
        self.global::<Devices>().set_open(open);
    }

    fn set_devices_loading(&self, loading: bool) {
        let devices = self.global::<Devices>();
        devices.set_loading(loading);
        if loading {
            devices.set_empty("".into());
        }
    }

    fn set_devices_empty(&self, message: &str) {
        let devices = self.global::<Devices>();
        devices.set_loading(false);
        devices.set_empty(message.into());
    }

    // ---- library -----------------------------------------------------

    fn set_playlists(&self, rows: Vec<PlaylistItem>, subtitle: &str) {
        let library = self.global::<Library>();
        library.set_playlists(ModelRc::from(Rc::new(VecModel::from(rows))));
        library.set_playlists_subtitle(subtitle.into());
        library.set_playlists_loading(false);
        library.set_playlists_empty("".into());
    }

    fn set_playlists_loading(&self, loading: bool) {
        let library = self.global::<Library>();
        library.set_playlists_loading(loading);
        if loading {
            library.set_playlists_empty("".into());
        }
    }

    fn set_playlists_error(&self, message: &str) {
        let library = self.global::<Library>();
        library.set_playlists(ModelRc::from(Rc::new(VecModel::from(Vec::new()))));
        library.set_playlists_subtitle("".into());
        library.set_playlists_loading(false);
        library.set_playlists_empty(message.into());
    }

    fn set_current_playlist(&self, uri: &str) {
        self.global::<Library>().set_current_playlist(uri.into());
    }

    // Covers arrive one at a time. Replacing the whole model for each would
    // rebuild every row and upload every cover already shown again, once per
    // arrival; changing the one row redraws only that row.
    fn set_playlist_cover(&self, index: usize, uri: &str, art: &Art) {
        let rows = self.global::<Library>().get_playlists();
        if let Some(mut row) = rows.row_data(index)
            && row.uri == uri
        {
            row.cover = art.image();
            rows.set_row_data(index, row);
        }
    }

    fn set_queue(&self, rows: Vec<QueueItem>, subtitle: &str, reset: bool) {
        let library = self.global::<Library>();
        library.set_queue(ModelRc::from(Rc::new(VecModel::from(rows))));
        library.set_queue_subtitle(subtitle.into());
        library.set_queue_loading(false);
        library.set_queue_empty("".into());
        if reset {
            // Scrolls the list back to the track that is playing. Cover artwork
            // arrives later and must not move the list under the user's finger.
            library.set_queue_revision(library.get_queue_revision() + 1);
        }
    }

    fn set_queue_cover(&self, index: usize, uri: &str, art: &Art) {
        let rows = self.global::<Library>().get_queue();
        if let Some(mut row) = rows.row_data(index)
            && row.uri == uri
        {
            row.cover = art.image();
            rows.set_row_data(index, row);
        }
    }

    fn set_queue_loading(&self, loading: bool) {
        let library = self.global::<Library>();
        library.set_queue_loading(loading);
        if loading {
            library.set_queue_empty("".into());
        }
    }

    fn set_queue_error(&self, message: &str) {
        let library = self.global::<Library>();
        library.set_queue(ModelRc::from(Rc::new(VecModel::from(Vec::new()))));
        library.set_queue_subtitle("".into());
        library.set_queue_loading(false);
        library.set_queue_empty(message.into());
    }
}

/// Offscreen renderer for design review: `cargo test -- --ignored preview`.
#[cfg(test)]
mod preview {
    use std::rc::Rc;

    use super::*;
    use crate::format;
    use crate::motion::Motion;

    const SIZE: u32 = 1080;

    struct PreviewPlatform {
        window: Rc<slint::platform::software_renderer::MinimalSoftwareWindow>,
    }

    impl slint::platform::Platform for PreviewPlatform {
        fn create_window_adapter(
            &self,
        ) -> std::result::Result<Rc<dyn slint::platform::WindowAdapter>, slint::PlatformError>
        {
            Ok(self.window.clone())
        }
    }

    /// One frame of the GPU-drawn progress ring, rendered headlessly.
    fn ring(progress: f32, accent: crate::artwork::Rgb) -> slint::Image {
        backdrop(progress, accent, 1.0, 1.0)
    }

    /// The player's backdrop, with the ring showing as much as `ring` and the
    /// glow as strong as `glow`.
    fn backdrop(progress: f32, accent: crate::artwork::Rgb, ring: f32, glow: f32) -> slint::Image {
        let pixels = crate::ring::render_once(&crate::ring::Input {
            progress,
            accent,
            ring,
            glow,
        })
        .unwrap();
        slint::Image::from_rgba8(slint::SharedPixelBuffer::clone_from_slice(
            &pixels,
            crate::ring::SIZE,
            crate::ring::SIZE,
        ))
    }

    fn cover(hue: u16) -> std::sync::Arc<Art> {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let artwork = crate::artwork::Artwork::new().unwrap();
        let corner = crate::app::ART_CORNER;
        runtime
            .block_on(artwork.load(&format!("mock://cover/{hue}/2"), 480, corner))
            .unwrap()
    }

    /// Paint out everything outside the round panel, so the previews show exactly
    /// what the display can draw.
    fn cover_round_window(image: &mut image::RgbaImage) {
        let centre = SIZE as f32 / 2.0;
        for (x, y, pixel) in image.enumerate_pixels_mut() {
            let dx = x as f32 + 0.5 - centre;
            let dy = y as f32 + 0.5 - centre;
            if (dx * dx + dy * dy).sqrt() > centre {
                *pixel = image::Rgba([0, 0, 0, 255]);
            }
        }
    }

    fn save(
        window: &AppWindow,
        soft: &slint::platform::software_renderer::MinimalSoftwareWindow,
        name: &str,
    ) {
        let mut buffer = vec![slint::Rgb8Pixel::new(0, 0, 0); (SIZE * SIZE) as usize];

        // Let declarative transitions (slides, fades) settle before capturing.
        std::thread::sleep(std::time::Duration::from_millis(900));
        slint::platform::update_timers_and_animations();

        window.window().request_redraw();
        soft.draw_if_needed(|renderer| {
            renderer.render(&mut buffer, SIZE as usize);
        });

        let bytes: Vec<u8> = buffer
            .iter()
            .flat_map(|pixel| [pixel.r, pixel.g, pixel.b, 255])
            .collect();
        let mut image = image::RgbaImage::from_raw(SIZE, SIZE, bytes).unwrap();
        cover_round_window(&mut image);
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("target/preview");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(format!("{name}.png"));
        image.save(&path).unwrap();
        println!("wrote {}", path.display());
    }

    #[test]
    #[ignore = "renders design previews into target/preview"]
    fn render_design_previews() {
        let _ = rustls::crypto::ring::default_provider().install_default();
        let soft = slint::platform::software_renderer::MinimalSoftwareWindow::new(
            slint::platform::software_renderer::RepaintBufferType::NewBuffer,
        );
        slint::platform::set_platform(Box::new(PreviewPlatform {
            window: soft.clone(),
        }))
        .unwrap();

        let window = AppWindow::new().unwrap();
        window.show().unwrap();
        soft.set_size(slint::PhysicalSize::new(SIZE, SIZE));

        // ---- now playing ----
        window.reveal_app();
        window.set_auth_state(2, "");
        window.set_screen(1);

        let art = cover(258);
        let mut motion = Motion::new();
        motion.set_accent(art.accent());
        for _ in 0..120 {
            motion.update(0.016);
        }
        let mut frame = motion.update(0.016);
        frame.reveal = 1.0;
        window.tick(&frame);

        window.set_track_text("Neon Coast", "Mira Vale · Afterglow", true);
        window.set_cover(Some(&art));
        window.set_playback(true, true, false, 1);
        window.set_device("Demo speaker", "speaker", true);
        window.set_ring_frame_image(ring(0.34, art.accent()));
        window.set_current_playlist("spotify:playlist:demo-0");
        save(&window, &soft, "01-now-playing");

        // ---- frame-rate readout ----
        let stats = window.global::<Stats>();
        stats.set_shown(true);
        stats.set_display(33);
        stats.set_display_ms(1.4);
        stats.set_display_gpu_ms(6.2);
        save(&window, &soft, "fps");
        stats.set_display(60);
        stats.set_scene(60);
        stats.set_scene_ms(2.84);
        stats.set_scene_gpu_ms(9.6);
        stats.set_place("seed 4242 · coral · 1834 s · x 5012.3 y -3.1 z -2.4 · 87°".into());
        window.global::<Idle>().set_active(true);
        save(&window, &soft, "fps-idle");
        window.global::<Idle>().set_active(false);
        stats.set_shown(false);

        // ---- output devices ----
        let devices = [
            (
                "demo-speaker",
                "Demo speaker",
                "speaker",
                "Speaker",
                true,
                true,
            ),
            (
                "demo-laptop",
                "Kitchen laptop",
                "computer",
                "Computer",
                false,
                true,
            ),
            ("demo-phone", "Phone", "phone", "Phone", false, true),
            ("demo-tv", "Living room TV", "tv", "TV", false, true),
            (
                "",
                "Chromecast",
                "tv",
                "Chromecast · pick it in the Spotify app",
                false,
                false,
            ),
        ]
        .into_iter()
        .map(|(id, name, kind, detail, active, selectable)| DeviceItem {
            name: name.into(),
            kind: kind.into(),
            detail: detail.into(),
            active,
            selectable,
            id: id.into(),
        })
        .collect();
        window.set_devices(devices);
        window.set_devices_open(true);
        save(&window, &soft, "11-devices");
        window.set_devices_open(false);

        // ---- playlists ----
        let rows: Vec<PlaylistItem> = (0..6)
            .map(|index| {
                let art = cover(40 + index * 37);
                PlaylistItem {
                    title: format!("Playlist {index}").into(),
                    subtitle: format!("{} · Owner", format::track_count(6)).into(),
                    uri: format!("spotify:playlist:demo-{index}").into(),
                    cover: art.image(),
                }
            })
            .collect();
        window.set_screen(2);
        window.set_playlists(rows, "6 playlists");
        save(&window, &soft, "02-playlists");

        // ---- queue ----
        let rows: Vec<QueueItem> = (0..7)
            .map(|index| {
                let art = cover(200 + index * 13);
                QueueItem {
                    title: format!("Track number {index}").into(),
                    subtitle: "Mira Vale".into(),
                    duration: "3:24".into(),
                    cover: art.image(),
                    current: index == 0,
                    uri: format!("spotify:track:demo-{index}").into(),
                }
            })
            .collect();
        window.set_screen(0);
        window.set_queue(rows, "11 tracks up next", false);
        save(&window, &soft, "03-queue");

        // ---- login ----
        let url = "https://accounts.spotify.com/authorize?client_id=example&response_type=code&redirect_uri=http%3A%2F%2F127.0.0.1%3A8888%2Fcallback&code_challenge=abc&state=xyz";
        window.set_auth_state(0, "");
        window.set_auth_url(url, Some(&crate::artwork::Artwork::qr(url).unwrap()));
        window.set_ring_frame_image(backdrop(0.0, art.accent(), 0.0, 0.6));
        save(&window, &soft, "04-login");

        // ---- nothing playing ----
        window.set_auth_state(2, "");
        window.set_screen(1);
        window.set_track_text("Nothing playing", "Open Spotify and start playback", false);
        window.set_cover(None);
        window.set_playback(false, false, false, 0);
        window.set_ring_frame_image(backdrop(0.0, art.accent(), 0.0, 1.0));
        save(&window, &soft, "05-nothing-playing");

        // ---- missing configuration ----
        window.set_config_path("/home/pi/.config/spotify-round/config.toml");
        window.set_auth_state(3, "");
        window.set_ring_frame_image(backdrop(0.0, art.accent(), 0.0, 0.6));
        save(&window, &soft, "06-setup");

        // ---- a playlist that is still starting ----
        window.set_auth_state(2, "");
        window.set_ring_frame_image(ring(0.0, art.accent()));
        let art = cover(198);
        window.set_cover(Some(&art));
        window.set_track_text("Chill Vibes", "Starting playlist", true);
        window.set_playback(true, true, false, 0);
        save(&window, &soft, "07-starting-playlist");

        // ---- first track of a playlist: no way back ----
        window.set_track_text("Neon Coast", "Mira Vale · Afterglow", true);
        window.set_skip_state(false, true);
        window.set_ring_frame_image(ring(0.12, art.accent()));
        save(&window, &soft, "08-no-previous");

        // ---- idle visualizer, with the card of what plays ----
        window.set_card_text("Mira Vale", "Afterglow", "Late Night Drive");
        window.set_liked(Some(false));
        let pixels = crate::globe::render_once(0.0).unwrap();
        window.set_idle_scene(0, crate::globe::SIZE as f32);
        window.set_idle(true, 1.0);
        window.set_idle_frame(slint::Image::from_rgba8(
            slint::SharedPixelBuffer::clone_from_slice(
                &pixels,
                crate::globe::SIZE,
                crate::globe::SIZE,
            ),
        ));
        save(&window, &soft, "09-idle");
        // The track saved to Liked Songs, and playing on.
        window.set_liked(Some(true));
        window.set_skip_state(true, true);
        save(&window, &soft, "09-idle-liked");

        // Half-way through its cross-fade.
        window.set_idle(true, 0.45);
        save(&window, &soft, "10-idle-fade");

        // ---- the Nautilus ----
        let pixels = crate::dive3d::render_once(70.0, 7_321).unwrap();
        window.set_idle_scene(1, crate::dive3d::SIZE as f32);
        window.set_idle(true, 1.0);
        window.set_idle_frame(slint::Image::from_rgba8(
            slint::SharedPixelBuffer::clone_from_slice(
                &pixels,
                crate::dive3d::SIZE,
                crate::dive3d::SIZE,
            ),
        ));
        save(&window, &soft, "12-nautilus");

        // The chart a long press opens, turned a sea ahead of the boat.
        let names: Vec<&str> = (0..crate::voyage::BIOMES)
            .map(crate::dive3d::sea_title)
            .collect();
        window.set_seas(&names, 3);
        window.global::<Idle>().set_charting(true);
        save(&window, &soft, "13-sea-chart");
        window.global::<Idle>().set_charting(false);

        // The screensaver chooser a swipe down on the player opens.
        window.set_idle(false, 0.0);
        window.global::<App>().set_choosing(true);
        save(&window, &soft, "14-scene-picker");
        window.global::<App>().set_choosing(false);

        // ---- the solar tour, with its HUD ----
        let (pixels, hud, labels) = crate::solar::tests::render_once(11, "-high-orbit");
        window.set_idle_scene(3, crate::solar::SIZE as f32);
        window.set_idle(true, 1.0);
        window.set_idle_hud(&hud);
        window.set_idle_labels(&labels);
        window.set_idle_frame(slint::Image::from_rgba8(
            slint::SharedPixelBuffer::clone_from_slice(
                &pixels,
                crate::solar::SIZE,
                crate::solar::SIZE,
            ),
        ));
        save(&window, &soft, "15-solar");
        let (pixels, hud, labels) = crate::solar::tests::render_once(11, "jump-cruise");
        window.set_idle_hud(&hud);
        window.set_idle_labels(&labels);
        window.set_idle_frame(slint::Image::from_rgba8(
            slint::SharedPixelBuffer::clone_from_slice(
                &pixels,
                crate::solar::SIZE,
                crate::solar::SIZE,
            ),
        ));
        save(&window, &soft, "16-solar-jump");
        // A star flyby, with the minimap's route, a moon the size of a
        // planet in orbit, a gravity assist and an engine burn.
        for (seed, moment, name) in [
            (23, "-flyby", "17-solar-flyby"),
            (14, "m1-high-orbit", "18-solar-moon"),
            (13, "-flyby", "19-solar-assist"),
            (11, "capture-burn", "20-solar-burn"),
        ] {
            let (pixels, hud, labels) = crate::solar::tests::render_once(seed, moment);
            window.set_idle_hud(&hud);
            window.set_idle_labels(&labels);
            window.set_idle_frame(slint::Image::from_rgba8(
                slint::SharedPixelBuffer::clone_from_slice(
                    &pixels,
                    crate::solar::SIZE,
                    crate::solar::SIZE,
                ),
            ));
            save(&window, &soft, name);
        }
        window.set_idle_hud(&crate::solar::Hud::default());
        window.set_idle_labels(&[]);
    }
}
