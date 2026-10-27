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

use crate::dive::Bell;
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
    Bell,
    /// The same voyage as the bell, rasterised in 3D.
    Nautilus,
}

impl Scene {
    pub const ALL: [Scene; 3] = [Scene::Globe, Scene::Bell, Scene::Nautilus];

    pub fn index(self) -> usize {
        self as usize
    }

    /// The size the UI should draw the frame at. The globe leaves room for its
    /// atmosphere; the porthole fills the panel.
    pub fn size(self) -> f32 {
        match self {
            Scene::Globe => crate::globe::SIZE as f32,
            Scene::Bell => crate::dive::SIZE as f32,
            Scene::Nautilus => crate::dive3d::SIZE as f32,
        }
    }

    fn build(self, gpu: &Graphics) -> Result<Box<dyn Visual>> {
        match self {
            Scene::Globe => Ok(Box::new(Globe::new(gpu)?)),
            Scene::Bell => Ok(Box::new(Bell::new(gpu)?)),
            Scene::Nautilus => Ok(Box::new(Nautilus::new(gpu)?)),
        }
    }

    /// Pick at random from the scenes whose renderer came up. `available` is the
    /// bit mask published by [`spawn`]; an empty mask falls back to the first
    /// scene so a very early request still has something to show.
    pub fn random(available: u64) -> Self {
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
            let mut renderers: Vec<Option<Box<dyn Visual>>> = Vec::with_capacity(Scene::ALL.len());
            let mut mask = 0u64;
            for candidate in Scene::ALL {
                match candidate.build(&gpu) {
                    Ok(renderer) => {
                        mask |= 1 << candidate.index();
                        renderers.push(Some(renderer));
                    }
                    Err(err) => {
                        tracing::warn!(scene = ?candidate, %err, "this visualisation is unavailable");
                        renderers.push(None);
                    }
                }
            }
            available.store(mask, Ordering::SeqCst);
            if mask == 0 {
                tracing::warn!("no idle visualisation could be created");
                return;
            }

            // Every scene owns a fixed block of slots in one flat image model, so
            // adding a scene needs no extra Slint property.
            let textures = renderers
                .iter()
                .flat_map(|renderer| match renderer {
                    Some(renderer) => (0..SLOTS)
                        .map(|frame| Some(renderer.texture(frame)))
                        .collect::<Vec<_>>(),
                    None => vec![None; SLOTS],
                })
                .collect::<Vec<_>>();
            let slots = textures.len();
            if ui
                .upgrade_in_event_loop(move |ui| ui.set_idle_frames(textures))
                .is_err()
            {
                return;
            }

            let budget = Duration::from_secs(1) / FPS as u32;
            let mut clocks = [0.0f64; Scene::ALL.len()];
            let mut sequences = [0usize; Scene::ALL.len()];
            let mut seed = world_seed();
            let mut last = Instant::now();
            let mut deadline = last;
            let mut active = usize::MAX;
            let mut travelled = travel.load(Ordering::Relaxed);
            let mut drawn = 0u32;
            // Time spent recording and submitting frames since the last report.
            let mut busy = Duration::ZERO;
            let mut reported = Instant::now();

            loop {
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

                if requested != active {
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
                let slot = active * SLOTS + current;
                if ui
                    .upgrade_in_event_loop(move |ui| ui.show_idle_frame(slot))
                    .is_err()
                {
                    return;
                }
                sequences[active] = (current + 1) % SLOTS;
                drawn += 1;
                busy += begun.elapsed();
                if reported.elapsed() >= Duration::from_secs(1) {
                    let ms = busy.as_secs_f32() * 1e3 / drawn as f32;
                    tracing::debug!(fps = drawn, ms, slots, "visualizer");
                    let frames = drawn as i32;
                    if ui
                        .upgrade_in_event_loop(move |ui| ui.set_scene_stats(frames, ms))
                        .is_err()
                    {
                        return;
                    }
                    drawn = 0;
                    busy = Duration::ZERO;
                    reported = Instant::now();
                }

                deadline += budget;
                pace(deadline);
            }
        });

    if let Err(err) = spawned {
        tracing::warn!(%err, "cannot start the visualizer thread");
    }
}

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
            assert_eq!(Scene::random(0b10), Scene::Bell);
            assert_eq!(Scene::random(0b01), Scene::Globe);
        }
    }
}
