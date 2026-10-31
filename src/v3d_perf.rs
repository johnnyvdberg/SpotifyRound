//! The Raspberry Pi's V3D hardware performance counters, through the kernel's
//! v3d perfmon ioctls on the render node: what the GPU's units actually did
//! over a stretch of frames (shader cycles by stage, texture and tile-buffer
//! stalls, early-depth rejections, memory traffic), where the frame time only
//! says how long they took.
//!
//! A perfmon made global (`DRM_IOCTL_V3D_PERFMON_SET_GLOBAL`) counts every job
//! the GPU runs, whoever submitted it, so the counters sit round an unmodified
//! wgpu benchmark. Every job means the kiosk's too: freeze it first, as the
//! benchmarks already do. `spotify-round-tests v3d_perf --ignored --nocapture`
//! lists the counters this V3D has; `DIVE3D_COUNTERS` puts some into
//! `dive3d::tests::hotspots`.
//!
//! Elsewhere (no v3d render node) nothing opens and the callers carry on
//! without counters.

/// One counter the V3D offers, as the kernel describes it.
#[derive(Clone, Debug)]
pub struct Counter {
    pub id: u8,
    pub name: String,
    pub category: String,
    pub description: String,
}

#[cfg(target_os = "linux")]
mod sys {
    use std::fs::File;
    use std::os::fd::AsRawFd;

    // `include/uapi/drm/v3d_drm.h`.
    const COMMAND_BASE: u64 = 0x40;
    const PARAM_MAX_PERF_COUNTERS: u32 = 13;
    pub const MAX_PER_PERFMON: usize = 32;
    const CLEAR_GLOBAL: u32 = 1;

    #[repr(C)]
    struct GetParam {
        param: u32,
        pad: u32,
        value: u64,
    }

    #[repr(C)]
    struct Create {
        id: u32,
        ncounters: u32,
        counters: [u8; MAX_PER_PERFMON],
    }

    #[repr(C)]
    struct Destroy {
        id: u32,
    }

    #[repr(C)]
    struct GetValues {
        id: u32,
        pad: u32,
        values_ptr: u64,
    }

    #[repr(C)]
    struct GetCounter {
        counter: u8,
        name: [u8; 64],
        category: [u8; 32],
        description: [u8; 256],
        reserved: [u8; 7],
    }

    #[repr(C)]
    struct SetGlobal {
        flags: u32,
        id: u32,
    }

    /// `_IOC(dir, 'd', COMMAND_BASE + nr, size)` as Linux encodes it.
    const fn ioc(write_only: bool, nr: u64, size: usize) -> u64 {
        let dir: u64 = if write_only { 1 } else { 3 };
        (dir << 30) | ((size as u64) << 16) | ((b'd' as u64) << 8) | (COMMAND_BASE + nr)
    }

    fn call<T>(fd: &File, write_only: bool, nr: u64, arg: &mut T) -> Option<()> {
        let request = ioc(write_only, nr, std::mem::size_of::<T>());
        // SAFETY: `arg` is a `#[repr(C)]` struct laid out as the kernel's for
        // this request, and lives across the call.
        let result = unsafe {
            libc::ioctl(
                fd.as_raw_fd(),
                request as libc::c_ulong,
                arg as *mut T as *mut libc::c_void,
            )
        };
        (result == 0).then_some(())
    }

    fn text(bytes: &[u8]) -> String {
        let end = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
        String::from_utf8_lossy(&bytes[..end]).into_owned()
    }

    /// The V3D's render node: the one whose driver answers the v3d
    /// parameter query.
    pub fn open() -> Option<File> {
        (128..136).find_map(|n| {
            let fd = std::fs::OpenOptions::new()
                .read(true)
                .write(true)
                .open(format!("/dev/dri/renderD{n}"))
                .ok()?;
            count(&fd).map(|_| fd)
        })
    }

    pub fn param(fd: &File, param: u32) -> Option<u64> {
        let mut p = GetParam {
            param,
            pad: 0,
            value: 0,
        };
        call(fd, false, 0x04, &mut p)?;
        Some(p.value)
    }

    pub fn count(fd: &File) -> Option<u8> {
        u8::try_from(param(fd, PARAM_MAX_PERF_COUNTERS)?).ok()
    }

    /// Slices and QPUs a slice, from the core's second identity register
    /// (`DRM_V3D_PARAM_V3D_CORE0_IDENT1`: NSLC in bits 4-7, QUPS in 8-11).
    pub fn shape(fd: &File) -> Option<(u64, u64)> {
        let ident = param(fd, 5)?;
        Some(((ident >> 4) & 0xf, (ident >> 8) & 0xf))
    }

    pub fn describe(fd: &File, id: u8) -> Option<super::Counter> {
        let mut c = GetCounter {
            counter: id,
            name: [0; 64],
            category: [0; 32],
            description: [0; 256],
            reserved: [0; 7],
        };
        call(fd, false, 0x0c, &mut c)?;
        Some(super::Counter {
            id,
            name: text(&c.name),
            category: text(&c.category),
            description: text(&c.description),
        })
    }

    pub fn create(fd: &File, ids: &[u8]) -> Option<u32> {
        let mut c = Create {
            id: 0,
            ncounters: ids.len() as u32,
            counters: [0; MAX_PER_PERFMON],
        };
        c.counters[..ids.len()].copy_from_slice(ids);
        call(fd, false, 0x08, &mut c)?;
        Some(c.id)
    }

    pub fn set_global(fd: &File, id: u32, on: bool) -> Option<()> {
        let mut s = SetGlobal {
            flags: if on { 0 } else { CLEAR_GLOBAL },
            id,
        };
        call(fd, true, 0x0d, &mut s)
    }

    pub fn values(fd: &File, id: u32, n: usize) -> Option<Vec<u64>> {
        let mut out = vec![0u64; n];
        let mut v = GetValues {
            id,
            pad: 0,
            values_ptr: out.as_mut_ptr() as u64,
        };
        call(fd, false, 0x0a, &mut v)?;
        Some(out)
    }

    pub fn destroy(fd: &File, id: u32) {
        let mut d = Destroy { id };
        call(fd, false, 0x09, &mut d);
    }
}

/// Every counter this V3D offers, or `None` without one.
#[cfg(target_os = "linux")]
pub fn counters() -> Option<Vec<Counter>> {
    let fd = sys::open()?;
    let n = sys::count(&fd)?;
    (0..n).map(|id| sys::describe(&fd, id)).collect()
}

#[cfg(not(target_os = "linux"))]
pub fn counters() -> Option<Vec<Counter>> {
    None
}

/// How many slices the V3D has, and how many QPUs (shader cores) each:
/// what a count of QPU cycles is out of.
#[cfg(target_os = "linux")]
pub fn shape() -> Option<(u64, u64)> {
    sys::shape(&sys::open()?)
}

#[cfg(not(target_os = "linux"))]
pub fn shape() -> Option<(u64, u64)> {
    None
}

/// A global perfmon over some counters, counting every job the GPU runs
/// until it is dropped.
pub struct Perfmon {
    #[cfg(target_os = "linux")]
    fd: std::fs::File,
    id: u32,
    /// How many counters it counts.
    count: usize,
}

impl Perfmon {
    /// Count the counters named in `names` (as [`counters`] lists them),
    /// at most 32. `None` without a V3D, or if a name is unknown or the
    /// kernel refuses (another global perfmon is set).
    #[cfg(target_os = "linux")]
    pub fn start(names: &[&str]) -> Option<Self> {
        let fd = sys::open()?;
        let all = counters()?;
        let ids = names
            .iter()
            .map(|name| all.iter().find(|c| c.name == *name).map(|c| c.id))
            .collect::<Option<Vec<u8>>>()?;
        if ids.is_empty() || ids.len() > sys::MAX_PER_PERFMON {
            return None;
        }
        let id = sys::create(&fd, &ids)?;
        if sys::set_global(&fd, id, true).is_none() {
            sys::destroy(&fd, id);
            return None;
        }
        Some(Self {
            fd,
            id,
            count: ids.len(),
        })
    }

    #[cfg(not(target_os = "linux"))]
    pub fn start(_names: &[&str]) -> Option<Self> {
        None
    }

    /// What each counter has counted since the perfmon started, in the
    /// order they were named in. Read only once the GPU is idle.
    #[cfg(target_os = "linux")]
    pub fn read(&self) -> Option<Vec<u64>> {
        sys::values(&self.fd, self.id, self.count)
    }

    #[cfg(not(target_os = "linux"))]
    pub fn read(&self) -> Option<Vec<u64>> {
        let _ = (self.id, self.count);
        None
    }
}

#[cfg(target_os = "linux")]
impl Drop for Perfmon {
    fn drop(&mut self) {
        sys::set_global(&self.fd, self.id, false);
        sys::destroy(&self.fd, self.id);
    }
}

#[cfg(test)]
mod tests {
    /// List the counters, with their categories and descriptions.
    #[test]
    #[ignore = "lists the Pi's V3D counters; needs its render node"]
    fn v3d_perf_counters() {
        let Some(all) = super::counters() else {
            println!("no V3D render node with performance counters here");
            return;
        };
        if let Some((slices, qpus)) = super::shape() {
            println!("{slices} slices of {qpus} QPUs");
        }
        for c in all {
            println!(
                "{:>3} {:<40} {:<10} {}",
                c.id, c.name, c.category, c.description
            );
        }
    }
}
