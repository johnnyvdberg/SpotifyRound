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
/// Slint a wgpu device on Vulkan before this runs, so femtovg draws the UI and the
/// idle globe through the same GPU.
pub fn create_window(fullscreen: bool, show_fps: bool) -> Result<AppWindow> {
    let window = AppWindow::new().context("creating the Slint window")?;
    window.window().set_fullscreen(fullscreen);
    // Full screen is the kiosk, and a kiosk is driven by touch: no pointer.
    window.global::<Pointer>().set_hidden(fullscreen);
    window.global::<Stats>().set_shown(show_fps);
    watch_rendering_if_asked(&window);
    Ok(window)
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
/// presented for the frame-rate readout.
///
/// With `SPOTIFY_ROUND_FULL_SPEED` set, every frame asks for the next at once,
/// whether anything changed or not, and the rate is logged each second: how
/// fast each screen can be drawn at all.
fn watch_rendering(window: &AppWindow) {
    let weak = window.as_weak();
    let full_speed = std::env::var_os("SPOTIFY_ROUND_FULL_SPEED").is_some();
    let mut frames = 0;
    let mut since = std::time::Instant::now();
    let result = window
        .window()
        .set_rendering_notifier(move |state, api| match state {
            slint::RenderingState::RenderingSetup => {
                tracing::info!(renderer = ?api, "graphics context ready");
            }
            slint::RenderingState::AfterRendering => {
                frames += 1;
                if full_speed && let Some(window) = weak.upgrade() {
                    window.window().request_redraw();
                }
                if since.elapsed() >= std::time::Duration::from_secs(1) {
                    let rate = frames;
                    (frames, since) = (0, std::time::Instant::now());
                    // Written after the frame, so it is shown with the next one.
                    // Hidden, it is left alone, so it wakes nothing.
                    if let Some(window) = weak.upgrade() {
                        if full_speed {
                            let app = window.global::<App>();
                            tracing::info!(
                                fps = rate,
                                screen = app.get_screen(),
                                idle = window.global::<Idle>().get_active(),
                                "full speed"
                            );
                        }
                        let stats = window.global::<Stats>();
                        if stats.get_shown() {
                            stats.set_display(rate);
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
    fn set_scene_stats(&self, frames: i32, ms: f32, gpu_ms: f32);
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
    fn set_queue(&self, rows: Vec<QueueItem>, subtitle: &str, reset: bool);
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

    fn set_scene_stats(&self, frames: i32, ms: f32, gpu_ms: f32) {
        let stats = self.global::<Stats>();
        if stats.get_shown() {
            stats.set_scene(frames);
            stats.set_scene_ms(ms);
            stats.set_scene_gpu_ms(gpu_ms);
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
    fn ring(progress: f32, scrubbing: bool, accent: crate::artwork::Rgb) -> slint::Image {
        backdrop(progress, scrubbing, accent, 1.0, 1.0)
    }

    /// The player's backdrop, with the ring showing as much as `ring` and the
    /// glow as strong as `glow`.
    fn backdrop(
        progress: f32,
        scrubbing: bool,
        accent: crate::artwork::Rgb,
        ring: f32,
        glow: f32,
    ) -> slint::Image {
        let pixels = crate::ring::render_once(&crate::ring::Input {
            progress,
            scrubbing,
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

    /// Paint out everything the round bezel hides, so the previews show exactly
    /// what the display can draw.
    fn cover_round_bezel(image: &mut image::RgbaImage) {
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
        cover_round_bezel(&mut image);
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
        window.set_ring_frame_image(ring(0.34, false, art.accent()));
        window.set_current_playlist("spotify:playlist:demo-0");
        save(&window, &soft, "01-now-playing");

        // ---- frame-rate readout ----
        let stats = window.global::<Stats>();
        stats.set_shown(true);
        stats.set_display(60);
        save(&window, &soft, "fps");
        stats.set_scene(60);
        stats.set_scene_ms(2.84);
        stats.set_scene_gpu_ms(9.6);
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
        window.set_ring_frame_image(backdrop(0.0, false, art.accent(), 0.0, 0.6));
        save(&window, &soft, "04-login");

        // ---- nothing playing ----
        window.set_auth_state(2, "");
        window.set_screen(1);
        window.set_track_text("Nothing playing", "Open Spotify and start playback", false);
        window.set_cover(None);
        window.set_playback(false, false, false, 0);
        window.set_ring_frame_image(backdrop(0.0, false, art.accent(), 0.0, 1.0));
        save(&window, &soft, "05-nothing-playing");

        // ---- missing configuration ----
        window.set_config_path("/home/pi/.config/spotify-round/config.toml");
        window.set_auth_state(3, "");
        window.set_ring_frame_image(backdrop(0.0, false, art.accent(), 0.0, 0.6));
        save(&window, &soft, "06-setup");

        // ---- a playlist that is still starting ----
        window.set_auth_state(2, "");
        window.set_ring_frame_image(ring(0.0, false, art.accent()));
        let art = cover(198);
        window.set_cover(Some(&art));
        window.set_track_text("Chill Vibes", "Starting playlist", true);
        window.set_playback(true, true, false, 0);
        save(&window, &soft, "07-starting-playlist");

        // ---- first track of a playlist: no way back ----
        window.set_track_text("Neon Coast", "Mira Vale · Afterglow", true);
        window.set_skip_state(false, true);
        window.set_ring_frame_image(ring(0.12, false, art.accent()));
        save(&window, &soft, "08-no-previous");

        // ---- idle visualizer ----
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
    }
}
