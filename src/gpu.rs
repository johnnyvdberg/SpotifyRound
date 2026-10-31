//! One wgpu device for the whole display.
//!
//! Slint renders the UI through wgpu (Skia on Vulkan), and the idle globe draws
//! into a texture on the same device, so a frame never leaves the GPU: no readback
//! and no re-upload. Slint has to be told about the device before any window
//! exists, which is why this sits in its own module rather than inside `globe`.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::Result;
use slint::wgpu_30::wgpu;
use wgpu_profiler::{GpuProfiler, GpuProfilerSettings};

/// A timer-query mark slot, and the pool it is returned to.
type MarkSlot = Arc<Mutex<Option<Duration>>>;
type MarkPool = Arc<Mutex<Vec<MarkSlot>>>;

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
    /// The `Mark` slots, reused instead of allocating an `Arc` per frame.
    marks: MarkPool,
    /// The last actual GPU time from the timer-query profiler, if it is
    /// running. On the V3D the profiler is absent; `GpuLoad` reads its busy
    /// counters instead.
    last_gpu_ms: Arc<Mutex<Option<f32>>>,
}

/// A point in the queue, made by [`Graphics::mark`]: set to the time the GPU
/// took once everything submitted before it is done.
pub struct Mark {
    slot: MarkSlot,
    pool: MarkPool,
}

impl Drop for Mark {
    fn drop(&mut self) {
        self.pool
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .push(self.slot.clone());
    }
}

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
        let last_gpu_ms = self.last_gpu_ms.clone();
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
                    if let Some(ms) = profile_ms(&frame) {
                        *last_gpu_ms
                            .lock()
                            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(ms);
                    }
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
        let slot = self
            .marks
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .pop()
            .unwrap_or_else(|| Arc::new(std::sync::Mutex::new(None)));
        *slot.lock().unwrap_or_else(|poisoned| poisoned.into_inner()) = None;
        let report = slot.clone();
        self.when_done(move |took| {
            *report
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(took)
        });
        Mark {
            slot,
            pool: self.marks.clone(),
        }
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
                .slot
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            if took.is_some() || Instant::now() >= until {
                return took;
            }
            let _ = self.device.poll(wgpu::PollType::Poll);
            std::thread::sleep(Duration::from_micros(250));
        }
    }

    /// The last Nautilus pass's GPU time in milliseconds, when timer queries
    /// are available. The V3D has none; [`GpuLoad`] reads its engine counters.
    pub fn gpu_ms(&self) -> Option<f32> {
        self.profiler.as_ref()?;
        *self
            .last_gpu_ms
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
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
        marks: Arc::new(Mutex::new(Vec::new())),
        last_gpu_ms: Arc::new(Mutex::new(None)),
    })
}

/// The Nautilus scope's GPU time from one finished profiler frame, in
/// milliseconds. The visualizer opens exactly that scope, so a recursive walk
/// finds it even if the profiler nests it.
fn profile_ms(frame: &[wgpu_profiler::GpuTimerQueryResult]) -> Option<f32> {
    for query in frame {
        if query.label == "Nautilus"
            && let Some(time) = &query.time
        {
            return Some(((time.end - time.start) * 1e3) as f32);
        }
        if let Some(ms) = profile_ms(&query.nested_queries) {
            return Some(ms);
        }
    }
    None
}

/// The Raspberry Pi's V3D busy counters, read from
/// `/sys/class/drm/card*/device/gpu_stats`. Each engine line is
/// `name timestamp jobs runtime`: `runtime` is cumulative busy nanoseconds.
/// The render engine's delta over the display's reporting second, divided by
/// the frames drawn, is the GPU time per frame on the device. Other GPUs have
/// no such file; there [`Graphics::gpu_ms`] takes over or the display says
/// `—`.
pub struct GpuLoad {
    path: Option<PathBuf>,
    last: Option<u64>,
}

impl GpuLoad {
    pub fn new() -> Self {
        let path = gpu_stats_path();
        let last = path.as_deref().and_then(read_render_runtime);
        Self { path, last }
    }

    /// GPU milliseconds per frame over `frames` frames since the last call.
    /// The first call only takes the baseline.
    pub fn per_frame(&mut self, frames: u32) -> Option<f32> {
        let now = read_render_runtime(self.path.as_deref()?)?;
        let last = self.last.replace(now)?;
        let delta = now.checked_sub(last)?;
        // A counter reset or a long stall is not a frame cost.
        if frames == 0 || delta > 5_000_000_000 {
            return None;
        }
        Some(delta as f32 / 1e6 / frames as f32)
    }
}

fn gpu_stats_path() -> Option<PathBuf> {
    let entries = std::fs::read_dir("/sys/class/drm").ok()?;
    entries.flatten().find_map(|entry| {
        let name = entry.file_name();
        if !name.to_string_lossy().starts_with("card") {
            return None;
        }
        let path = entry.path().join("device/gpu_stats");
        path.is_file().then_some(path)
    })
}

fn read_render_runtime(path: &std::path::Path) -> Option<u64> {
    let text = std::fs::read_to_string(path).ok()?;
    render_runtime(&text)
}

fn render_runtime(text: &str) -> Option<u64> {
    engine_runtime(text, "render")
}

/// The busy nanoseconds so far of the V3D's binning engine, which runs the
/// vertex shaders' binning pass and sorts triangles into tiles, and of its
/// render engine, which runs the vertex shaders again and every fragment:
/// how much of a frame is geometry and how much is pixels. `None` where there
/// is no `gpu_stats`.
#[cfg_attr(not(test), allow(dead_code))]
pub fn engine_busy() -> Option<(u64, u64)> {
    let text = std::fs::read_to_string(gpu_stats_path()?).ok()?;
    Some((
        engine_runtime(&text, "bin")?,
        engine_runtime(&text, "render")?,
    ))
}

fn engine_runtime(text: &str, engine: &str) -> Option<u64> {
    for line in text.lines() {
        let mut fields = line.split_whitespace();
        if fields.next() != Some(engine) {
            continue;
        }
        fields.next()?; // timestamp
        fields.next()?; // jobs
        return fields.next()?.parse().ok();
    }
    None
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
    let mut encoder = gpu
        .device
        .create_command_encoder(&wgpu::CommandEncoderDescriptor {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_v3d_counter_parser_reads_the_render_engine() {
        let text = "queue\ttimestamp\tjobs\truntime\n\
                    bin\t96819875704297\t25518675\t7578612819218\n\
                    render\t96819875704297\t25518675\t50863620570058\n\
                    tfu\t96819875704297\t31475491\t8148262068943\n";
        assert_eq!(render_runtime(text), Some(50_863_620_570_058));
        assert_eq!(engine_runtime(text, "bin"), Some(7_578_612_819_218));
        assert_eq!(render_runtime("no counters here\n"), None);
    }

    #[test]
    fn the_profiler_finds_the_nautilus_scope() {
        let time = 0.001..0.010;
        let frame = vec![wgpu_profiler::GpuTimerQueryResult {
            label: "Nautilus".into(),
            pid: 1,
            tid: std::thread::current().id(),
            time: Some(time),
            nested_queries: Vec::new(),
        }];
        assert!((profile_ms(&frame).unwrap() - 9.0).abs() < 1e-3);
    }
}
