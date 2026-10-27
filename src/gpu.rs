//! One wgpu device for the whole display.
//!
//! Slint renders the UI through wgpu (femtovg on Vulkan), and the idle globe draws
//! into a texture on the same device, so a frame never leaves the GPU: no readback
//! and no re-upload. Slint has to be told about the device before any window
//! exists, which is why this sits in its own module rather than inside `globe`.

use anyhow::Result;
use slint::wgpu_30::wgpu;

/// The device and queue everything renders with.
#[derive(Clone)]
pub struct Graphics {
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
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
        return Ok(Graphics { device, queue });
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
    Ok(Graphics { device, queue })
}
