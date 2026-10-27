//! One wgpu device for the whole display.
//!
//! Slint renders the UI through wgpu (femtovg on Vulkan), and the idle globe draws
//! into a texture on the same device, so a frame never leaves the GPU: no readback
//! and no re-upload. Slint has to be told about the device before any window
//! exists, which is why this sits in its own module rather than inside `globe`.

use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::Result;
use slint::wgpu_30::wgpu;

/// The device and queue everything renders with.
#[derive(Clone)]
pub struct Graphics {
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
    /// No window and no event loop: submit on the calling thread.
    headless: bool,
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
        self.on_ui_thread(move || {
            queue.submit(std::iter::once(commands));
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
            *report.lock().unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(took)
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
            let took = *mark.0.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
            if took.is_some() || Instant::now() >= until {
                return took;
            }
            let _ = self.device.poll(wgpu::PollType::Poll);
            std::thread::sleep(Duration::from_micros(250));
        }
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
        let Ok((device, queue)) =
            pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
                label: Some("display"),
                required_features: wgpu::Features::empty(),
                required_limits: adapter.limits(),
                ..Default::default()
            }))
        else {
            last = format!("cannot open the {} adapter", info.name);
            continue;
        };

        let configuration = slint::wgpu_30::WGPUConfiguration::Manual {
            instance,
            adapter,
            device: device.clone(),
            queue: queue.clone(),
        };
        let selector = slint::BackendSelector::new().require_wgpu_30(configuration);
        // Without a compositor the DRM/KMS backend draws straight to the panel.
        #[cfg(feature = "kms")]
        let selector = selector.backend_name("linuxkms".into());
        selector.select()?;

        tracing::info!(adapter = %info.name, backend = ?info.backend, "graphics");
        return Ok(Graphics {
            device,
            queue,
            headless: false,
        });
    }
    anyhow::bail!(last)
}

/// A device for headless rendering, without touching Slint's backend. Only the
/// design previews use this.
#[cfg(test)]
pub fn headless() -> Result<Graphics> {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
        backends: wgpu::Backends::all(),
        ..wgpu::InstanceDescriptor::new_without_display_handle_from_env()
    });
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::HighPerformance,
        ..Default::default()
    }))?;
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        label: Some("preview"),
        required_features: wgpu::Features::empty(),
        required_limits: adapter.limits(),
        ..Default::default()
    }))?;
    Ok(Graphics {
        device,
        queue,
        headless: true,
    })
}
