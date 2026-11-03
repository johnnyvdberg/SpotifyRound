//! The museum's specimens and paintings, which only the museum shows, are
//! kept off the GPU the rest of the voyage. As the museum comes near they
//! are brought in a piece a frame, on the render thread, and once it is
//! behind they are let go again. Nothing is built or compressed here: both
//! come ready to copy, the models baked with every level of detail
//! (`mesh::baked`) and the paintings compressed by `build.rs`, so a frame
//! pays for one model's or one painting's copy and no more.
//!
//! The library leaves the museum's models empty (`Model::is_streamed`). They
//! go into a pair of buffers of their own, made the size of all of them when
//! the first comes in, and their levels are pointed into those; the draws
//! bind them for those models. The paintings are an ETC2 texture array
//! (decompressed for a GPU that cannot sample ETC2), bound at 14 in place of
//! a one-texel stand-in once every layer is written.

use super::mesh::{self, Lod, Model};
use super::paintings::{DATA, LEVELS, PAINTINGS, SIZE};
use crate::etc::{self, Layout};
use crate::gpu::Graphics;

/// The bytes of one painting's ETC2 RGB levels, SIZE square down to 4.
fn layer_bytes() -> usize {
    (0..LEVELS).map(|l| level_bytes(SIZE >> l)).sum()
}

fn level_bytes(size: u32) -> usize {
    (size as usize).div_ceil(4).pow(2) * Layout::Rgb.block_bytes()
}

pub struct Stream {
    /// The baked models still to come in, last first.
    pending: Vec<(Model, Vec<mesh::baked::Level>)>,
    /// How much of the buffers the models in so far fill: vertices and
    /// indices.
    filled: (u32, u32),
    /// The models' buffers, from the first model in.
    pub buffers: Option<(wgpu::Buffer, wgpu::Buffer)>,
    /// Per model, whether it is in and drawn from [`Stream::buffers`].
    pub resident: Vec<bool>,
    /// The library's own (empty) level of each model brought in, to give
    /// back when it goes.
    kept: Vec<(Model, Lod)>,
    /// The paintings' texture while it is filled, and how many layers are.
    paintings: Option<wgpu::Texture>,
    layers: usize,
    /// What binding 14 shows: the paintings, once all in, or the stand-in.
    pub view: wgpu::TextureView,
    standin: wgpu::TextureView,
    /// Whether the view is the paintings'.
    showing: bool,
}

impl Stream {
    pub fn new(gpu: &Graphics) -> Self {
        use wgpu::util::DeviceExt;
        let standin = gpu
            .device
            .create_texture_with_data(
                &gpu.queue,
                &wgpu::TextureDescriptor {
                    label: Some("Dive3d paintings (none)"),
                    size: wgpu::Extent3d {
                        width: 1,
                        height: 1,
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: wgpu::TextureFormat::Rgba8Unorm,
                    usage: wgpu::TextureUsages::TEXTURE_BINDING,
                    view_formats: &[],
                },
                wgpu::util::TextureDataOrder::LayerMajor,
                &[96, 88, 72, 255],
            )
            .create_view(&wgpu::TextureViewDescriptor {
                dimension: Some(wgpu::TextureViewDimension::D2Array),
                ..Default::default()
            });
        Self {
            pending: Vec::new(),
            filled: (0, 0),
            buffers: None,
            resident: vec![false; Model::ALL.len()],
            kept: Vec::new(),
            paintings: None,
            layers: 0,
            view: standin.clone(),
            standin,
            showing: false,
        }
    }

    /// Whether anything is in or on its way.
    pub fn holding(&self) -> bool {
        self.buffers.is_some() || self.paintings.is_some()
    }

    /// Whether every model and painting is in.
    #[cfg(test)]
    pub fn complete(&self) -> bool {
        self.showing
    }

    /// One frame's step: bring in the next piece if `wanted`, or let it all
    /// go if not. Returns whether binding 14's view changed, so the bind
    /// group must be made again.
    pub fn step(&mut self, gpu: &Graphics, lods: &mut [Vec<Lod>], wanted: bool) -> bool {
        if !wanted {
            if !self.holding() {
                return false;
            }
            for (model, empty) in self.kept.drain(..) {
                self.resident[model.index()] = false;
                lods[model.index()] = vec![empty];
            }
            self.pending.clear();
            self.filled = (0, 0);
            self.buffers = None;
            self.paintings = None;
            self.layers = 0;
            let changed = self.showing;
            self.view = self.standin.clone();
            self.showing = false;
            return changed;
        }
        if self.showing {
            return false;
        }
        // The models first, one a frame, into buffers made for all of them.
        if self.buffers.is_none() {
            self.pending = mesh::baked::models();
            self.pending.reverse();
            let (vertices, indices) = self
                .pending
                .iter()
                .flat_map(|(_, levels)| levels)
                .fold((0, 0), |(v, i), l| {
                    (v + l.vertices.len() as u64, i + l.indices.len() as u64)
                });
            let buffer = |label, size, usage| {
                gpu.device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some(label),
                    size,
                    usage: usage | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                })
            };
            self.buffers = Some((
                buffer(
                    "Dive3d museum vertices",
                    vertices,
                    wgpu::BufferUsages::VERTEX,
                ),
                buffer("Dive3d museum indices", indices, wgpu::BufferUsages::INDEX),
            ));
        }
        if let Some((model, levels)) = self.pending.pop() {
            let (vertex_buffer, index_buffer) = self.buffers.as_ref().expect("made above");
            let list = &mut lods[model.index()];
            let empty = list[0];
            self.kept.push((model, empty));
            list.clear();
            for level in levels {
                let (base, first) = self.filled;
                let stride = std::mem::size_of::<mesh::Packed>() as u64;
                gpu.queue
                    .write_buffer(vertex_buffer, u64::from(base) * stride, level.vertices);
                gpu.queue
                    .write_buffer(index_buffer, u64::from(first) * 4, level.indices);
                list.push(Lod {
                    first,
                    count: level.index_count(),
                    base: base as i32,
                    vertices: level.vertex_count(),
                    error: level.error,
                    stray: 0.0,
                    edge: 0.0,
                    ..empty
                });
                self.filled = (base + level.vertex_count(), first + level.index_count());
            }
            self.resident[model.index()] = true;
            return false;
        }
        // Then the paintings, a layer a frame.
        let compressed = gpu
            .device
            .features()
            .contains(wgpu::Features::TEXTURE_COMPRESSION_ETC2);
        // Made in a frame of its own: the allocation is the dearest step.
        let Some(texture) = &self.paintings else {
            self.paintings = Some(gpu.device.create_texture(&wgpu::TextureDescriptor {
                label: Some("Dive3d paintings"),
                size: wgpu::Extent3d {
                    width: SIZE,
                    height: SIZE,
                    depth_or_array_layers: PAINTINGS.len() as u32,
                },
                mip_level_count: LEVELS,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: if compressed {
                    wgpu::TextureFormat::Etc2Rgb8Unorm
                } else {
                    wgpu::TextureFormat::Rgba8Unorm
                },
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            }));
            return false;
        };
        if self.layers < PAINTINGS.len() {
            debug_assert_eq!(DATA.len(), PAINTINGS.len() * layer_bytes());
            let mut at = self.layers * layer_bytes();
            for level in 0..LEVELS {
                let size = SIZE >> level;
                let bytes = &DATA[at..at + level_bytes(size)];
                at += level_bytes(size);
                let decompressed;
                let (data, row) = if compressed {
                    (bytes, size / 4 * Layout::Rgb.block_bytes() as u32)
                } else {
                    decompressed =
                        etc::decompress(bytes, size as usize, size as usize, Layout::Rgb);
                    (&decompressed[..], size * 4)
                };
                gpu.queue.write_texture(
                    wgpu::TexelCopyTextureInfo {
                        texture,
                        mip_level: level,
                        origin: wgpu::Origin3d {
                            x: 0,
                            y: 0,
                            z: self.layers as u32,
                        },
                        aspect: wgpu::TextureAspect::All,
                    },
                    data,
                    wgpu::TexelCopyBufferLayout {
                        offset: 0,
                        bytes_per_row: Some(row),
                        rows_per_image: None,
                    },
                    wgpu::Extent3d {
                        width: size,
                        height: size,
                        depth_or_array_layers: 1,
                    },
                );
            }
            self.layers += 1;
            return false;
        }
        self.view = texture.create_view(&wgpu::TextureViewDescriptor {
            dimension: Some(wgpu::TextureViewDimension::D2Array),
            ..Default::default()
        });
        self.showing = true;
        true
    }
}
