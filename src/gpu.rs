//! One wgpu device for the whole display.
//!
//! Slint renders the UI through wgpu (Skia on Vulkan), and the idle globe draws
//! into a texture on the same device, so a frame never leaves the GPU: no readback
//! and no re-upload. Slint has to be told about the device before any window
//! exists, which is why this sits in its own module rather than inside `globe`.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::Result;
use slint::wgpu_30::wgpu;
use wgpu_profiler::{GpuProfiler, GpuProfilerSettings};

/// The device and queue everything renders with.
#[derive(Clone)]
pub struct Graphics {
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
    /// No window and no event loop: submit on the calling thread.
    headless: bool,
    /// The Vulkan/adapter name, for the V3D checks and logs.
    #[allow(dead_code)]
    pub adapter: String,
    /// Timer-query profiling, when the adapter and `SPOTIFY_ROUND_PROFILE` ask
    /// for it. The V3D has no timestamp queries, so this is `None` on the Pi.
    profiler: Option<Arc<Mutex<GpuProfiler>>>,
    /// Set by [`Graphics::profiled`] so [`Graphics::submit`] knows to close the
    /// profiler frame after the command buffer has actually been submitted.
    profile_pending: Arc<AtomicBool>,
}

/// A point in the queue, made by [`Graphics::mark`]: set to the time the GPU
/// took once everything submitted before it is done.
pub struct Mark(Arc<std::sync::Mutex<Option<Duration>>>);

impl Graphics {
    /// Submit work recorded on a thread other than the UI thread.
    ///
    /// The submission itself is made on the UI thread. Before wgpu reconfigures
    /// the window's surface it waits for the queue to drain, and fails if
    /// another thread submits meanwhile; Slint reconfigures on the UI thread
    /// whenever the swapchain goes out of date, on Wayland even on the first
    /// frame, so a scene submitting from its own thread could bring the whole
    /// display down. The UI thread runs these in order, after anything this
    /// thread handed it before.
    ///
    /// Buffer and texture writes made before this call are in the submission;
    /// writes made after it may be too, until [`Graphics::wait`] says it is done.
    pub fn submit(&self, commands: wgpu::CommandBuffer) {
        let queue = self.queue.clone();
        let profiler = self.profiler.clone();
        let pending = self.profile_pending.clone();
        self.on_ui_thread(move || {
            queue.submit(std::iter::once(commands));
            if let Some(profiler) = profiler
                && pending.swap(false, Ordering::AcqRel)
            {
                let mut guard = profiler
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
                if let Err(err) = guard.end_frame() {
                    tracing::warn!(%err, "cannot close the GPU profile frame");
                } else if let Some(frame) =
                    guard.process_finished_frame(queue.get_timestamp_period())
                {
                    log_profile(&frame);
                }
            }
        });
    }

    /// Call `done` with the time from submission once the GPU has finished
    /// everything submitted so far, including what [`Graphics::submit`] has
    /// handed the UI thread. Called from a later poll of the device.
    pub fn when_done(&self, done: impl FnOnce(Duration) + Send + 'static) {
        let queue = self.queue.clone();
        self.on_ui_thread(move || {
            let submitted = Instant::now();
            queue.on_submitted_work_done(move || done(submitted.elapsed()));
        });
    }

    /// A marker after everything submitted so far, to wait for later with
    /// [`Graphics::wait_for`].
    pub fn mark(&self) -> Mark {
        let mark = Mark(Arc::new(std::sync::Mutex::new(None)));
        let report = mark.0.clone();
        self.when_done(move |took| {
            *report
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(took)
        });
        mark
    }

    /// Wait until the GPU has finished everything submitted so far.
    pub fn wait(&self, timeout: Duration) -> Option<Duration> {
        self.wait_for(&self.mark(), timeout)
    }

    /// Wait until the GPU has finished everything submitted before `mark`,
    /// polling the device rather than blocking on it so the UI thread's own
    /// submissions are never held up. Returns how long that took from its
    /// submission, or `None` after `timeout`.
    pub fn wait_for(&self, mark: &Mark, timeout: Duration) -> Option<Duration> {
        let until = Instant::now() + timeout;
        loop {
            let took = *mark
                .0
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            if took.is_some() || Instant::now() >= until {
                return took;
            }
            let _ = self.device.poll(wgpu::PollType::Poll);
            std::thread::sleep(Duration::from_micros(250));
        }
    }

    /// Whether timer-query profiling is active.
    pub fn profiling(&self) -> bool {
        self.profiler.is_some()
    }

    /// Run `f` inside a wgpu-profiler scope, if timer queries are available.
    ///
    /// The scope is closed and its queries resolved before this returns; the
    /// frame is ended after the command buffer is submitted. Without timer
    /// queries (the V3D, for example) this is just `f(encoder)`.
    pub fn profiled<R>(
        &self,
        label: &str,
        encoder: &mut wgpu::CommandEncoder,
        f: impl FnOnce(&mut wgpu::CommandEncoder) -> R,
    ) -> R {
        let Some(profiler) = &self.profiler else {
            return f(encoder);
        };
        let mut guard = profiler
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let result = {
            let mut scope = guard.scope(label, encoder);
            f(&mut scope)
        };
        guard.resolve_queries(encoder);
        self.profile_pending.store(true, Ordering::Release);
        result
    }

    fn on_ui_thread(&self, work: impl FnOnce() + Send + 'static) {
        if self.headless {
            work();
        } else if let Err(err) = slint::invoke_from_event_loop(work) {
            tracing::warn!(%err, "the UI thread is gone; GPU work dropped");
        }
    }
}

/// Create the device and make Slint use it.
///
/// Vulkan first, because that is what the Pi 5 runs; the other backends are only a
/// convenience so the display still works on a machine without a Vulkan driver.
/// `WGPU_BACKEND` picks one instead (`gl`, `vulkan`, …), to compare the Pi's
/// Vulkan driver against its OpenGL ES one.
pub fn install() -> Result<Graphics> {
    let mut last = "no GPU adapter available".to_owned();
    let order = match wgpu::Backends::from_env() {
        Some(chosen) => vec![chosen],
        None => vec![wgpu::Backends::VULKAN, wgpu::Backends::all()],
    };
    for backends in order {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends,
            ..wgpu::InstanceDescriptor::new_without_display_handle_from_env()
        });
        let Ok(adapter) =
            pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                ..Default::default()
            }))
        else {
            last = format!("no adapter for {backends:?}");
            continue;
        };
        let info = adapter.get_info();
        let graphics = match open(&adapter, "display", false) {
            Ok(graphics) => graphics,
            Err(err) => {
                last = format!("cannot open the {} adapter: {err}", info.name);
                continue;
            }
        };

        let configuration = slint::wgpu_30::WGPUConfiguration::Manual {
            instance,
            adapter,
            device: graphics.device.clone(),
            queue: graphics.queue.clone(),
        };
        let selector = slint::BackendSelector::new().require_wgpu_30(configuration);
        // Without a compositor the DRM/KMS backend draws straight to the panel.
        #[cfg(feature = "kms")]
        let selector = selector.backend_name("linuxkms".into());
        selector.select()?;

        tracing::info!(
            adapter = %info.name,
            backend = ?info.backend,
            profiling = graphics.profiling(),
            "graphics"
        );
        return Ok(graphics);
    }
    anyhow::bail!(last)
}

/// A device for headless rendering, without touching Slint's backend. Only the
/// design previews use this.
#[cfg(test)]
pub fn headless() -> Result<Graphics> {
    let backends =
        wgpu::Backends::from_env().unwrap_or(wgpu::Backends::VULKAN | wgpu::Backends::all());
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
        backends,
        ..wgpu::InstanceDescriptor::new_without_display_handle_from_env()
    });
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::HighPerformance,
        ..Default::default()
    }))?;
    open(&adapter, "preview", true)
}

/// Open a device, optionally with wgpu-profiler's timer-query features.
fn open(adapter: &wgpu::Adapter, label: &str, headless: bool) -> Result<Graphics> {
    let profile = std::env::var_os("SPOTIFY_ROUND_PROFILE").is_some();
    let mut features = wgpu::Features::empty();
    if profile {
        let wanted =
            wgpu::Features::TIMESTAMP_QUERY | wgpu::Features::TIMESTAMP_QUERY_INSIDE_ENCODERS;
        if adapter.features().contains(wanted) {
            features = adapter.features() & GpuProfiler::ALL_WGPU_TIMER_FEATURES;
        } else {
            tracing::info!(
                adapter = %adapter.get_info().name,
                "wgpu-profiler needs timestamp queries; disabled"
            );
        }
    }
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        label: Some(label),
        required_features: features,
        required_limits: adapter.limits(),
        ..Default::default()
    }))?;
    let profiler = if features.contains(wgpu::Features::TIMESTAMP_QUERY_INSIDE_ENCODERS) {
        let settings = GpuProfilerSettings {
            enable_debug_groups: false,
            ..Default::default()
        };
        match GpuProfiler::new(&device, settings) {
            Ok(profiler) => {
                tracing::info!(adapter = %adapter.get_info().name, "wgpu-profiler enabled");
                Some(Arc::new(Mutex::new(profiler)))
            }
            Err(err) => {
                tracing::warn!(%err, "cannot create the GPU profiler");
                None
            }
        }
    } else {
        None
    };
    Ok(Graphics {
        device,
        queue,
        headless,
        adapter: adapter.get_info().name,
        profiler,
        profile_pending: Arc::new(AtomicBool::new(false)),
    })
}

/// Log the slowest scopes of one finished profiler frame.
fn log_profile(frame: &[wgpu_profiler::GpuTimerQueryResult]) {
    for query in frame {
        let Some(time) = &query.time else {
            continue;
        };
        tracing::info!(
            scope = %query.label,
            ms = (time.end - time.start) * 1e3,
            "gpu profile"
        );
        log_profile(&query.nested_queries);
    }
}


/// Read a square RGBA texture back to CPU pixels, for tests and reference
/// frames.
#[cfg(test)]
pub(crate) fn read_texture(gpu: &Graphics, texture: &wgpu::Texture, size: u32) -> Vec<u8> {
    let padded_row = (size * 4).div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT)
        * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
    let buffer = gpu.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("Readback"),
        size: u64::from(padded_row) * u64::from(size),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = gpu.device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("Readback"),
    });
    encoder.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo {
            texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        wgpu::TexelCopyBufferInfo {
            buffer: &buffer,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(padded_row),
                rows_per_image: Some(size),
            },
        },
        wgpu::Extent3d {
            width: size,
            height: size,
            depth_or_array_layers: 1,
        },
    );
    gpu.queue.submit(std::iter::once(encoder.finish()));
    let slice = buffer.slice(..);
    slice.map_async(wgpu::MapMode::Read, |r| r.expect("a mapped frame"));
    gpu.device.poll(wgpu::PollType::wait_indefinitely()).ok();
    let mut pixels = Vec::with_capacity((size * size * 4) as usize);
    {
        let data = slice.get_mapped_range().expect("a mapped frame");
        for row in 0..size as usize {
            let start = row * padded_row as usize;
            pixels.extend_from_slice(&data[start..start + (size * 4) as usize]);
        }
    }
    buffer.unmap();
    pixels
}
