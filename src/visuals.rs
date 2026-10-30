//! The idle visualizer's scene registry.
//!
//! A scene owns its GPU resources and draws one frame at a time. The thread in
//! [`spawn`] picks whichever scene the controller selected, renders it at the
//! panel's refresh rate and hands the frame to Slint as a texture.
//!
//! Adding another visualisation is deliberately small: implement [`Visual`],
//! add a variant to [`Scene`] and return it from [`Scene::build`]. The UI keeps a
//! single image and a scene index, so it needs no knowledge of the scene itself.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU64, AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use anyhow::Result;
use rand::RngCore as _;
use slint::wgpu_30::wgpu;

use crate::dive3d::Nautilus;
use crate::globe::Globe;
use crate::gpu::Graphics;
use crate::ui::{AppWindow, WindowExt};

/// Render targets per scene. Each scene rotates through this many textures so the
/// frame Slint is drawing is never the one being written.
pub(crate) const SLOTS: usize = 3;
const FPS: u64 = crate::app::TARGET_FPS;

/// One visualisation that can take over the idle screen.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scene {
    Globe,
    /// The Nautilus' voyage, seen through its salon window.
    Nautilus,
}

impl Scene {
    pub const ALL: [Scene; 2] = [Scene::Globe, Scene::Nautilus];

    pub fn index(self) -> usize {
        self as usize
    }

    /// The size the UI should draw the frame at. The globe leaves room for its
    /// atmosphere; the Nautilus' sea fills the panel.
    pub fn size(self) -> f32 {
        match self {
            Scene::Globe => crate::globe::SIZE as f32,
            Scene::Nautilus => crate::dive3d::SIZE as f32,
        }
    }

    fn build(self, gpu: &Graphics) -> Result<Box<dyn Visual>> {
        match self {
            Scene::Globe => Ok(Box::new(Globe::new(gpu)?)),
            Scene::Nautilus => Ok(Box::new(Nautilus::new(gpu)?)),
        }
    }

    /// The scene named by `SPOTIFY_ROUND_SCENE` (`globe` or `nautilus`), which
    /// then shows from the start and stays: a fixed load for measuring one
    /// scene, together with `SPOTIFY_ROUND_FULL_SPEED`.
    pub fn forced() -> Option<Self> {
        static FORCED: std::sync::OnceLock<Option<Scene>> = std::sync::OnceLock::new();
        *FORCED.get_or_init(|| {
            match std::env::var("SPOTIFY_ROUND_SCENE")
                .ok()?
                .to_ascii_lowercase()
                .as_str()
            {
                "globe" => Some(Scene::Globe),
                "nautilus" => Some(Scene::Nautilus),
                other => {
                    tracing::warn!(scene = other, "SPOTIFY_ROUND_SCENE names no scene");
                    None
                }
            }
        })
    }

    /// Pick at random from the scenes whose renderer came up. `available` is the
    /// bit mask published by [`spawn`]; an empty mask falls back to the first
    /// scene so a very early request still has something to show.
    pub fn random(available: u64) -> Self {
        if let Some(scene) = Self::forced() {
            return scene;
        }
        let mask = if Self::ALL.len() >= 64 {
            available
        } else {
            available & ((1u64 << Self::ALL.len()) - 1)
        };
        if mask == 0 {
            return Self::ALL[0];
        }
        let count = mask.count_ones();
        let mut roll = rand::rng().next_u32() % count;
        for scene in Self::ALL {
            if mask & (1 << scene.index()) != 0 {
                if roll == 0 {
                    return scene;
                }
                roll -= 1;
            }
        }
        Self::ALL[0]
    }
}

/// A scene's rendering interface. The display thread only ever calls these.
pub(crate) trait Visual: Send {
    fn render(&mut self, gpu: &Graphics, seconds: f64, seed: u64, frame: usize) -> Result<()>;
    fn texture(&self, frame: usize) -> wgpu::Texture;
    /// A swipe on the visualizer: move `legs` stretches on (or back) through the
    /// scene's journey. Scenes without one ignore it.
    fn travel(&mut self, _legs: i32) {}

    /// Change the resolution the visual is drawn at; true if it changed.
    /// Scenes drawn at a fixed size ignore it.
    fn set_render_scale(&mut self, _gpu: &Graphics, _scale: f32) -> bool {
        false
    }
}

/// Render the selected scene while the visualizer is visible.
///
/// `scene` names the scene the controller wants, `available` reports which
/// scenes finished building, and `running` turns the whole render thread off
/// while the overlay is hidden. `travel` counts swipes on the visualizer, +1 on
/// and -1 back; only the change since the last frame matters.
pub fn spawn(
    ui: slint::Weak<AppWindow>,
    gpu: Graphics,
    running: Arc<AtomicBool>,
    scene: Arc<AtomicUsize>,
    available: Arc<AtomicU64>,
    travel: Arc<AtomicI64>,
) {
    let spawned = std::thread::Builder::new()
        .name("visualizer".into())
        .spawn(move || {
            // Scenes are built on a thread of their own and handed over as each
            // is ready, so the first can show while the rest still compile their
            // shaders: on the Pi that takes a while.
            let (finished, built) = std::sync::mpsc::channel();
            let builder = gpu.clone();
            let started = std::thread::Builder::new()
                .name("visualizer build".into())
                .spawn(move || {
                    for candidate in Scene::ALL {
                        let begun = Instant::now();
                        let renderer = candidate.build(&builder);
                        let seconds = begun.elapsed().as_secs_f32();
                        match &renderer {
                            Ok(_) => tracing::info!(scene = ?candidate, seconds, "visualisation built"),
                            Err(err) => tracing::warn!(scene = ?candidate, %err, seconds, "this visualisation is unavailable"),
                        }
                        if finished.send((candidate, renderer.ok())).is_err() {
                            return;
                        }
                    }
                });
            if let Err(err) = started {
                tracing::warn!(%err, "cannot start building the visualisations");
                return;
            }
            let mut renderers: Vec<Option<Box<dyn Visual>>> =
                (0..Scene::ALL.len()).map(|_| None).collect();
            let mut mask = 0u64;
            let mut building = true;
            let mut slots = 0;
            // Take in whatever finished building since last time, and show the UI
            // its render targets. Returns false once the window has gone.
            let take_built = |renderers: &mut Vec<Option<Box<dyn Visual>>>,
                                  mask: &mut u64,
                                  building: &mut bool,
                                  slots: &mut usize| {
                let mut fresh = false;
                loop {
                    match built.try_recv() {
                        Ok((candidate, renderer)) => {
                            if let Some(renderer) = renderer {
                                *mask |= 1 << candidate.index();
                                renderers[candidate.index()] = Some(renderer);
                                fresh = true;
                            }
                        }
                        Err(std::sync::mpsc::TryRecvError::Empty) => break,
                        Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                            if *building && *mask == 0 {
                                tracing::warn!("no idle visualisation could be created");
                            }
                            *building = false;
                            break;
                        }
                    }
                }
                if !fresh {
                    return true;
                }
                // Every scene owns a fixed block of slots in one flat image model,
                // so adding a scene needs no extra Slint property.
                let textures = renderers
                    .iter()
                    .flat_map(|renderer| match renderer {
                        Some(renderer) => (0..SLOTS)
                            .map(|frame| Some(renderer.texture(frame)))
                            .collect::<Vec<_>>(),
                        None => vec![None; SLOTS],
                    })
                    .collect::<Vec<_>>();
                *slots = textures.len();
                let shown = ui
                    .upgrade_in_event_loop(move |ui| ui.set_idle_frames(textures))
                    .is_ok();
                available.store(*mask, Ordering::SeqCst);
                shown
            };

            let budget = Duration::from_secs(1) / FPS as u32;
            let mut clocks = [0.0f64; Scene::ALL.len()];
            let mut sequences = [0usize; Scene::ALL.len()];
            let mut seed = world_seed();
            let mut last = Instant::now();
            let mut deadline = last;
            let mut active = usize::MAX;
            let mut travelled = travel.load(Ordering::Relaxed);
            let mut drawn = 0u32;
            // Time spent recording and submitting frames since the last
            // report. The GPU time comes from the V3D's busy counters on the
            // Pi, or from timer queries where the adapter has them.
            let mut busy = Duration::ZERO;
            let mut gpu_load = crate::gpu::GpuLoad::new();
            let mut reported = Instant::now();
            let mut in_flight = std::collections::VecDeque::new();
            // Adaptive resolution, off unless SPOTIFY_ROUND_ADAPTIVE is set:
            // the one-second report already has the GPU time, so step the
            // sea's render scale between 0.6 and 1.0 from it.
            let adaptive = std::env::var_os("SPOTIFY_ROUND_ADAPTIVE").is_some();
            let mut scale = 1.0f32;

            loop {
                if !take_built(&mut renderers, &mut mask, &mut building, &mut slots) {
                    return;
                }
                if mask == 0 {
                    if !building {
                        return;
                    }
                    // Nothing to draw yet.
                    std::thread::sleep(Duration::from_millis(100));
                    last = Instant::now();
                    deadline = last;
                    continue;
                }
                if !running.load(Ordering::Relaxed) {
                    std::thread::sleep(Duration::from_millis(60));
                    last = Instant::now();
                    deadline = last;
                    active = usize::MAX;
                    continue;
                }

                let now = Instant::now();
                let dt = now.duration_since(last).as_secs_f64();
                last = now;

                let requested = scene.load(Ordering::Relaxed).min(Scene::ALL.len() - 1);
                let requested = if mask & (1 << requested) != 0 {
                    requested
                } else {
                    mask.trailing_zeros() as usize
                };
                let switched = requested != active;

                if switched {
                    // A new visit gets a fresh dive: different seabed, wreck,
                    // jellyfish and plankton.
                    active = requested;
                    seed = world_seed();
                    clocks[active] = 0.0;
                    // Swipes made before this visit belong to another one.
                    travelled = travel.load(Ordering::Relaxed);
                }
                clocks[active] += dt;

                let Some(renderer) = renderers[active].as_mut() else {
                    continue;
                };
                let swiped = travel.load(Ordering::Relaxed);
                if swiped != travelled {
                    renderer.travel((swiped - travelled).clamp(-7, 7) as i32);
                    travelled = swiped;
                }
                let current = sequences[active];
                let begun = Instant::now();
                if let Err(err) = renderer.render(&gpu, clocks[active], seed, current) {
                    tracing::warn!(%err, "drawing a visualisation failed");
                    std::thread::sleep(Duration::from_secs(1));
                    continue;
                }
                busy += begun.elapsed();
                // Shown right after the UI thread submits it: both run there,
                // in order. Only the sea image swaps.
                let slot = active * SLOTS + current;
                if ui
                    .upgrade_in_event_loop(move |ui| {
                        ui.show_idle_frame(slot);
                    })
                    .is_err()
                {
                    return;
                }
                // Two frames in flight, no more: submitting faster than the GPU
                // finishes only queues work up ahead of the UI's frames, until
                // every frame on screen is late. One would leave the GPU idle
                // while the UI thread, waiting on the display, gets round to
                // submitting the next. Once a frame is done, the UI has taken it.
                in_flight.push_back(gpu.mark());
                while in_flight.len() > 1 {
                    if let Some(oldest) = in_flight.pop_front() {
                        // The result is only the wait; the display's GPU time
                        // comes from the counters or profiler below.
                        let _ = gpu.wait_for(&oldest, GPU_TIMEOUT);
                    }
                }
                sequences[active] = (current + 1) % SLOTS;
                drawn += 1;
                if reported.elapsed() >= Duration::from_secs(1) {
                    let ms = busy.as_secs_f32() * 1e3 / drawn as f32;
                    // `-1` means this adapter exposes neither the V3D counters
                    // nor timer queries; the UI shows `—`, not a made-up 16.7.
                    let gpu_ms = gpu_load
                        .per_frame(drawn)
                        .or_else(|| gpu.gpu_ms())
                        .unwrap_or(-1.0);
                    if adaptive {
                        let target = 1000.0 / FPS as f32;
                        let next = if gpu_ms > target * 0.97 {
                            (scale - 0.05).max(0.6)
                        } else if gpu_ms < target * 0.80 {
                            (scale + 0.05).min(1.0)
                        } else {
                            scale
                        };
                        if (next - scale).abs() > 1e-3 {
                            scale = next;
                            let changed = renderers
                                .iter_mut()
                                .flatten()
                                .map(|r| r.set_render_scale(&gpu, scale))
                                .any(|changed| changed);
                            // The UI holds images built from the old texture
                            // handles; hand it the rebuilt ones.
                            if changed {
                                let textures: Vec<Option<wgpu::Texture>> = renderers
                                    .iter()
                                    .flat_map(|r| match r {
                                        Some(r) => (0..SLOTS)
                                            .map(|f| Some(r.texture(f)))
                                            .collect::<Vec<_>>(),
                                        None => vec![None; SLOTS],
                                    })
                                    .collect();
                                if ui
                                    .upgrade_in_event_loop(move |ui| ui.set_idle_frames(textures))
                                    .is_err()
                                {
                                    return;
                                }
                            }
                        }
                    }
                    tracing::debug!(fps = drawn, ms, gpu_ms, slots, "visualizer");
                    let frames = drawn as i32;
                    if ui
                        .upgrade_in_event_loop(move |ui| ui.set_scene_stats(frames, ms, gpu_ms))
                        .is_err()
                    {
                        return;
                    }
                    drawn = 0;
                    busy = Duration::ZERO;
                    reported = Instant::now();
                }

                deadline += budget;
                // A frame that ran long is not made up for by rushing the next
                // ones out back to back.
                let now = Instant::now();
                if deadline < now {
                    deadline = now;
                }
                pace(deadline);
            }
        });

    if let Err(err) = spawned {
        tracing::warn!(%err, "cannot start the visualizer thread");
    }
}

/// How long to wait for the GPU, and the UI thread that submits for this one,
/// to finish a frame before drawing the next anyway, so a window that stops
/// drawing cannot stall the scene for good.
const GPU_TIMEOUT: Duration = Duration::from_millis(500);

fn world_seed() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos() as u64)
        .unwrap_or(0);
    nanos ^ rand::rng().next_u64()
}

/// Wait for `deadline`, spinning over the last fraction of a millisecond.
///
/// The Windows timer is coarse enough that a plain `sleep` to a 60 Hz deadline
/// lands a frame late about half the time; a short spin keeps motion even and
/// costs almost nothing.
fn pace(deadline: Instant) {
    const SPIN: Duration = Duration::from_micros(400);
    while let Some(rest) = deadline.checked_duration_since(Instant::now()) {
        if rest > SPIN {
            std::thread::sleep(rest - SPIN);
        } else {
            std::hint::spin_loop();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_empty_mask_still_offers_a_scene() {
        assert_eq!(Scene::random(0), Scene::ALL[0]);
    }

    #[test]
    fn only_available_scenes_are_chosen() {
        for _ in 0..64 {
            assert_eq!(Scene::random(0b10), Scene::Nautilus);
            assert_eq!(Scene::random(0b01), Scene::Globe);
        }
    }
}
