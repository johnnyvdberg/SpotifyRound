//! A grand tour of invented star systems, on the idle screen.
//!
//! Each visit is a generated system ([`system`]): a star, five to eight
//! planets of different kinds, and moons of the kinds our own system has. The
//! camera follows a timeline ([`tour`]): it drops out of a hyperjump, crosses
//! to three of the planets in turn, orbits each high up and then low over its
//! horizon, and after the last turns toward the next star and jumps, which
//! takes a couple of minutes, into the next system. The HUD names the
//! destination and shows the ETA, distances and real orbital speeds.
//!
//! **Built for the Pi 5's VideoCore VII.** The planet maps are painted on the
//! GPU while the scene runs, a few rows a frame ([`stream`]), into two slots:
//! the planet being visited and the next one, so new worlds stream in during
//! the crossing and the one left behind is overwritten. The frame is two
//! passes: a full-screen background of stars and the galaxy, then one small
//! quad per body, far to near, so a pixel only pays for the bodies over it.

pub mod stream;
pub mod system;
pub mod tour;

use anyhow::Result;
use slint::wgpu_30::wgpu;

use crate::gpu::Graphics;
use crate::visuals::Visual;
use stream::{Key, Streamer, sampler_entry, texture_entry, uniform_entry};
use system::{Kind, Planet, System, equatorial_to_system};
use tour::{Phase, Tour, View, normalize};

/// Size of the rendered frame.
pub const SIZE: u32 = 1080;
const MAX_BODIES: usize = 16;
const SLOTS: usize = crate::visuals::SLOTS;

#[repr(C)]
#[derive(Clone, Copy, Default, bytemuck::Pod, bytemuck::Zeroable)]
struct Body {
    place: [f32; 4],
    rot: [[f32; 4]; 3],
    look: [f32; 4],
    air: [f32; 4],
    colour: [f32; 4],
    ring: [f32; 4],
    ring_colour: [f32; 4],
    rect: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Params {
    frame: [f32; 4],
    right: [f32; 4],
    up: [f32; 4],
    forward: [f32; 4],
    star: [f32; 4],
    star_colour: [f32; 4],
    sky: [[f32; 4]; 3],
    warp: [f32; 4],
    bodies: [Body; MAX_BODIES],
}

struct Frame {
    texture: wgpu::Texture,
    view: wgpu::TextureView,
}

/// What the HUD shows: a title, and lines of labels and values.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Hud {
    pub title: String,
    pub labels: String,
    pub values: String,
}

pub(crate) struct Solar {
    background: wgpu::RenderPipeline,
    bodies: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    uniforms: wgpu::Buffer,
    sampler: wgpu::Sampler,
    bind_group: Option<wgpu::BindGroup>,
    bound: (u64, usize),
    frames: Vec<Frame>,
    streamer: Streamer,
    base_seed: u64,
    visit: u64,
    visit_start: f64,
    tour: Tour,
    hud: Hud,
    hud_at: f64,
}

fn system_for(base: u64, visit: u64) -> System {
    System::generate(base ^ visit.wrapping_mul(0x9e37_79b9_7f4a_7c15))
}

impl Solar {
    pub(crate) fn new(gpu: &Graphics) -> Result<Self> {
        let seed = std::env::var("SOLAR_SEED")
            .ok()
            .and_then(|v| v.trim().parse().ok())
            .unwrap_or_else(rand::random::<u64>);
        Ok(Self::with_seed(gpu, seed))
    }

    pub(crate) fn with_seed(gpu: &Graphics, seed: u64) -> Self {
        let device = &gpu.device;
        let streamer = Streamer::new(gpu);
        let d2 = wgpu::TextureViewDimension::D2;
        let array = wgpu::TextureViewDimension::D2Array;
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Solar frame"),
            entries: &[
                uniform_entry(0),
                texture_entry(1, d2),
                sampler_entry(2),
                texture_entry(3, d2),
                texture_entry(4, d2),
                texture_entry(5, d2),
                texture_entry(6, array),
                texture_entry(7, array),
                texture_entry(8, d2),
                texture_entry(9, d2),
                texture_entry(10, d2),
                texture_entry(11, array),
                texture_entry(12, array),
            ],
        });
        let mut source = String::from(include_str!("noise.wgsl"));
        source.push_str(include_str!("frame.wgsl"));
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Solar frame"),
            source: wgpu::ShaderSource::Wgsl(source.into()),
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Solar frame"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let pipeline = |label: &str, vs: &str, fs: &str, blend: Option<wgpu::BlendState>| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(&pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some(vs),
                    buffers: &[],
                    compilation_options: Default::default(),
                },
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some(fs),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: wgpu::TextureFormat::Rgba8Unorm,
                        blend,
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                    compilation_options: Default::default(),
                }),
                primitive: wgpu::PrimitiveState::default(),
                depth_stencil: None,
                multisample: wgpu::MultisampleState::default(),
                multiview_mask: None,
                cache: None,
            })
        };
        let background = pipeline("Solar background", "vs_background", "fs_background", None);
        let bodies = pipeline(
            "Solar bodies",
            "vs_body",
            "fs_body",
            Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
        );
        let uniforms = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Solar uniforms"),
            size: std::mem::size_of::<Params>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("Solar maps"),
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Linear,
            anisotropy_clamp: 16,
            ..Default::default()
        });
        let frames = (0..SLOTS)
            .map(|_| {
                let texture = device.create_texture(&wgpu::TextureDescriptor {
                    label: Some("Solar frame"),
                    size: wgpu::Extent3d {
                        width: SIZE,
                        height: SIZE,
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: wgpu::TextureFormat::Rgba8Unorm,
                    usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                        | wgpu::TextureUsages::TEXTURE_BINDING
                        | wgpu::TextureUsages::COPY_SRC,
                    view_formats: &[],
                });
                let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
                Frame { texture, view }
            })
            .collect();
        let tour = Tour::new(system_for(seed, 0), system_for(seed, 1));
        Self {
            background,
            bodies,
            layout,
            uniforms,
            sampler,
            bind_group: None,
            bound: (u64::MAX, usize::MAX),
            frames,
            streamer,
            base_seed: seed,
            visit: 0,
            visit_start: 0.0,
            tour,
            hud: Hud::default(),
            hud_at: f64::NEG_INFINITY,
        }
    }

    /// Move the tour on to the visit `seconds` falls in.
    fn catch_up(&mut self, seconds: f64) -> f64 {
        if seconds < self.visit_start {
            self.visit_start = seconds;
        }
        while seconds - self.visit_start >= self.tour.total {
            self.visit_start += self.tour.total;
            self.visit += 1;
            let next = system_for(self.base_seed, self.visit + 1);
            let current = std::mem::replace(&mut self.tour.next, next.clone());
            self.tour = Tour::new(current, next);
            tracing::info!(star = %self.tour.system.star.name, "arrived in a new system");
        }
        seconds - self.visit_start
    }

    /// Tell the streamer what to have ready at `t` into the visit.
    fn stream(&mut self, t: f64) {
        let (segment, _) = self.tour.segment(t);
        let (current, next) = self.tour.wanted(t);
        let seed = self.tour.system.seed;
        let next_first = self.tour.next.itinerary()[0];
        let mut wanted: Vec<(Key, &Planet)> = Vec::new();
        for planet in [current, next].into_iter().flatten() {
            wanted.push(((seed, planet), &self.tour.system.planets[planet]));
        }
        let leaving = matches!(segment.phase, Phase::Depart(_) | Phase::Jump);
        if leaving || wanted.len() < 2 {
            if wanted.len() >= 2 {
                wanted.pop();
            }
            if leaving {
                wanted.push((
                    (self.tour.next.seed, next_first),
                    &self.tour.next.planets[next_first],
                ));
            }
        }
        let mut skies = vec![(seed, self.tour.system.sky)];
        if leaving {
            skies.push((self.tour.next.seed, self.tour.next.sky));
        }
        self.streamer.want(&wanted, &skies);
    }

    fn bind(&mut self, gpu: &Graphics) {
        let sky = self.streamer.sky_ready(self.tour.system.seed).unwrap_or(0);
        let wanted = (self.streamer.version, sky);
        if self.bind_group.is_some() && self.bound == wanted {
            return;
        }
        let full = |t: &wgpu::Texture| t.create_view(&wgpu::TextureViewDescriptor::default());
        let array = |t: &wgpu::Texture| {
            t.create_view(&wgpu::TextureViewDescriptor {
                dimension: Some(wgpu::TextureViewDimension::D2Array),
                ..Default::default()
            })
        };
        let a = self.streamer.slot_maps(0);
        let b = self.streamer.slot_maps(1);
        let views = [
            full(self.streamer.sky(sky)),
            full(&a.day),
            full(&a.relief),
            full(&a.extra),
            array(&a.moon_day),
            array(&a.moon_relief),
            full(&b.day),
            full(&b.relief),
            full(&b.extra),
            array(&b.moon_day),
            array(&b.moon_relief),
        ];
        let mut entries = vec![
            wgpu::BindGroupEntry {
                binding: 0,
                resource: self.uniforms.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::TextureView(&views[0]),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: wgpu::BindingResource::Sampler(&self.sampler),
            },
        ];
        for (i, view) in views.iter().enumerate().skip(1) {
            entries.push(wgpu::BindGroupEntry {
                binding: 2 + i as u32,
                resource: wgpu::BindingResource::TextureView(view),
            });
        }
        self.bind_group = Some(gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Solar frame"),
            layout: &self.layout,
            entries: &entries,
        }));
        self.bound = wanted;
    }

    /// The shader's parameters at `t` into the visit, and how many bodies.
    fn params(&self, t: f64) -> (Params, u32) {
        let view = self.tour.view(t);
        let system = &self.tour.system;
        let camera = view.position;
        let rel = |p: [f64; 3]| [p[0] - camera[0], p[1] - camera[1], p[2] - camera[2]];
        let f32x4 = |v: [f64; 3], w: f64| [v[0] as f32, v[1] as f32, v[2] as f32, w as f32];

        let star_rel = rel([0.0; 3]);
        let hidden = view.warp > 0.3;
        let mut bodies: Vec<(f64, Body)> = Vec::new();
        if !hidden {
            let mut star = Body {
                place: f32x4(star_rel, system.star.radius),
                look: [-1.0, -1.0, -1.0, 0.0],
                ..Default::default()
            };
            if let Some(rect) = rect(&view, star_rel, system.star.radius * 1.02) {
                star.rect = rect;
                bodies.push((length(star_rel), star));
            }
            for (index, planet) in system.planets.iter().enumerate() {
                let centre = planet.position(t);
                let place = rel(centre);
                let slot = self.streamer.ready((system.seed, index));
                let mut body = Body {
                    place: f32x4(place, planet.radius),
                    rot: rotation(
                        planet,
                        t * std::f64::consts::TAU / planet.day,
                        t / planet.day * 0.035,
                    ),
                    look: [
                        planet.kind as u32 as f32,
                        slot.map_or(-1.0, |s| s as f32),
                        -1.0,
                        night_mode(planet.kind),
                    ],
                    air: {
                        let a = planet.kind.air();
                        [a[0], a[1], a[2], 0.0]
                    },
                    colour: flat_colour(planet),
                    ..Default::default()
                };
                let mut bound = planet.radius * if body.air[0] > 0.0 { 1.08 } else { 1.0 };
                if let Some(rings) = &planet.rings {
                    body.ring = [
                        rings.inner as f32,
                        rings.outer as f32,
                        rings.opacity,
                        rings.seed,
                    ];
                    body.ring_colour = [rings.colour[0], rings.colour[1], rings.colour[2], 1.0];
                    bound = planet.radius * rings.outer;
                }
                if let Some(rect) = rect(&view, place, bound) {
                    body.rect = rect;
                    bodies.push((length(place), body));
                }
                // Moons, once their planet's maps are in.
                let Some(slot) = slot else {
                    continue;
                };
                for (m, moon) in planet.moons.iter().enumerate().take(stream::MOONS as usize) {
                    let offset = planet.moon_offset(m, t);
                    let place = rel([
                        centre[0] + offset[0],
                        centre[1] + offset[1],
                        centre[2] + offset[2],
                    ]);
                    let angle = moon.phase + t * std::f64::consts::TAU / moon.period;
                    let air = moon.kind.air();
                    let bound = moon.radius * if air[0] > 0.0 { 1.08 } else { 1.0 };
                    let Some(rect) = rect(&view, place, bound) else {
                        continue;
                    };
                    bodies.push((
                        length(place),
                        Body {
                            place: f32x4(place, moon.radius),
                            rot: rotation(planet, angle + std::f64::consts::PI, 0.0),
                            look: [
                                moon.kind as u32 as f32,
                                slot as f32,
                                m as f32,
                                night_mode(moon.kind),
                            ],
                            air: [air[0], air[1], air[2], 0.0],
                            colour: [
                                moon.palette[0][0],
                                moon.palette[0][1],
                                moon.palette[0][2],
                                0.0,
                            ],
                            rect,
                            ..Default::default()
                        },
                    ));
                }
            }
        }
        // Far to near; the farthest dropped if there are too many.
        bodies.sort_by(|a, b| b.0.total_cmp(&a.0));
        if bodies.len() > MAX_BODIES {
            bodies.drain(0..bodies.len() - MAX_BODIES);
        }
        let count = bodies.len() as u32;
        let mut list = [Body::default(); MAX_BODIES];
        for (slot, (_, body)) in list.iter_mut().zip(bodies.iter()) {
            *slot = *body;
        }

        // How much of the star's disc the bodies leave in view.
        let star_distance = length(star_rel);
        let star_dir = normalize(star_rel);
        let star_angle = (system.star.radius / star_distance).min(1.0).asin();
        let mut visible = 1.0;
        for (distance, body) in &bodies {
            if body.look[0] < 0.0 || *distance > star_distance {
                continue;
            }
            let dir = normalize([
                body.place[0] as f64,
                body.place[1] as f64,
                body.place[2] as f64,
            ]);
            let separation = dot(dir, star_dir).clamp(-1.0, 1.0).acos();
            let radius = (body.place[3] as f64 / distance).min(1.0).asin();
            visible *= smoothstep(radius - star_angle, radius + star_angle, separation);
        }
        if hidden {
            visible = 0.0;
        }

        let sky = sky_rotation(system.sky);
        let right = cross(view.forward, view.up);
        let params = Params {
            frame: [
                SIZE as f32 / 2.0,
                view.half_fov as f32,
                (2.0 * view.half_fov / SIZE as f64) as f32,
                count as f32,
            ],
            right: f32x4(right, 0.0),
            up: f32x4(view.up, 0.0),
            forward: f32x4(view.forward, 0.0),
            star: f32x4(star_rel, system.star.radius),
            star_colour: [
                system.star.colour[0],
                system.star.colour[1],
                system.star.colour[2],
                visible as f32,
            ],
            sky,
            warp: [view.warp as f32, view.flash as f32, t as f32, 0.0],
            bodies: list,
        };
        (params, count)
    }

    fn draw(&mut self, gpu: &Graphics, t: f64, frame: usize) {
        let mut encoder = gpu
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("Solar encoder"),
            });
        self.streamer.advance(gpu, &mut encoder);
        self.bind(gpu);
        let (params, count) = self.params(t);
        gpu.queue
            .write_buffer(&self.uniforms, 0, bytemuck::bytes_of(&params));
        let Some(group) = &self.bind_group else {
            return;
        };
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Solar pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &self.frames[frame].view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_bind_group(0, group, &[]);
            pass.set_pipeline(&self.background);
            pass.draw(0..3, 0..1);
            if count > 0 {
                pass.set_pipeline(&self.bodies);
                pass.draw(0..6, 0..count);
            }
        }
        gpu.submit(encoder.finish());
    }

    fn update_hud(&mut self, t: f64) {
        if (t - self.hud_at).abs() < 0.25 {
            return;
        }
        self.hud_at = t;
        let (title, lines) = self.tour.hud(t);
        let labels = lines
            .iter()
            .map(|(l, _)| l.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        let values = lines
            .iter()
            .map(|(_, v)| v.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        self.hud = Hud {
            title,
            labels,
            values,
        };
    }

    fn render(&mut self, gpu: &Graphics, seconds: f64, frame: usize) -> Result<()> {
        let t = self.catch_up(seconds);
        self.stream(t);
        self.draw(gpu, t, frame);
        self.update_hud(t);
        Ok(())
    }
}

impl Visual for Solar {
    fn render(&mut self, gpu: &Graphics, seconds: f64, _seed: u64, frame: usize) -> Result<()> {
        Solar::render(self, gpu, seconds, frame)
    }

    fn texture(&self, frame: usize) -> wgpu::Texture {
        self.frames[frame].texture.clone()
    }

    fn hud(&self) -> Option<Hud> {
        Some(self.hud.clone())
    }
}

fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
fn cross(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}
fn length(a: [f64; 3]) -> f64 {
    dot(a, a).sqrt()
}
fn smoothstep(e0: f64, e1: f64, x: f64) -> f64 {
    let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

fn night_mode(kind: Kind) -> f32 {
    if kind.has_cities() {
        1.0
    } else if kind.glows() {
        2.0
    } else {
        0.0
    }
}

fn flat_colour(planet: &Planet) -> [f32; 4] {
    match planet.kind {
        Kind::Terran => [0.06, 0.12, 0.22, 0.0],
        Kind::Ocean => [0.04, 0.10, 0.24, 0.0],
        Kind::GasGiant | Kind::IceGiant => {
            let c = planet.palette[0];
            [c[0], c[1], c[2], 0.25]
        }
        _ => {
            let c = planet.palette[0];
            [c[0], c[1], c[2], 0.0]
        }
    }
}

/// Rows taking system directions into a body's turning frame: the planet's
/// equatorial frame, turned `angle` about its axis. The clouds' extra turn
/// rides in the first row's w.
fn rotation(planet: &Planet, angle: f64, clouds: f64) -> [[f32; 4]; 3] {
    let ex = equatorial_to_system(planet, [1.0, 0.0, 0.0]);
    let ey = equatorial_to_system(planet, [0.0, 1.0, 0.0]);
    let ez = equatorial_to_system(planet, [0.0, 0.0, 1.0]);
    let (s, c) = angle.sin_cos();
    let row0 = [0, 1, 2].map(|i| c * ex[i] - s * ez[i]);
    let row2 = [0, 1, 2].map(|i| s * ex[i] + c * ez[i]);
    let f = |r: [f64; 3], w: f64| [r[0] as f32, r[1] as f32, r[2] as f32, w as f32];
    [f(row0, clouds.rem_euclid(1.0)), f(ey, 0.0), f(row2, 0.0)]
}

/// Rows turning world directions into the sky map's frame. The generator
/// already lays the galaxy's band around the system's own pole, so this is
/// the identity; it is here to turn the sky later without touching the map.
fn sky_rotation(_sky: [f32; 4]) -> [[f32; 4]; 3] {
    [
        [1.0, 0.0, 0.0, 0.0],
        [0.0, 1.0, 0.0, 0.0],
        [0.0, 0.0, 1.0, 0.0],
    ]
}

/// The quad covering a sphere of `radius` at `centre` (camera-relative), in
/// clip space, or none if it is out of view.
fn rect(view: &View, centre: [f64; 3], radius: f64) -> Option<[f32; 4]> {
    let full = Some([-1.0, -1.0, 1.0, 1.0]);
    let distance = length(centre);
    let ahead = dot(centre, view.forward);
    if distance <= radius * 1.02 || ahead <= radius {
        // Around or beside the camera: the whole frame, if any of it can be
        // in view.
        return if ahead > -radius { full } else { None };
    }
    let right = cross(view.forward, view.up);
    // The silhouette: a circle around the centre, perpendicular to the line
    // of sight, on the cone of tangents.
    let axis = normalize(centre);
    let e1 = normalize(cross(
        axis,
        if axis[1].abs() < 0.9 {
            [0.0, 1.0, 0.0]
        } else {
            [1.0, 0.0, 0.0]
        },
    ));
    let e2 = cross(axis, e1);
    let spread = distance * (radius / distance).asin().tan();
    let (mut lo, mut hi) = ([f64::MAX; 2], [f64::MIN; 2]);
    for k in 0..16 {
        let a = k as f64 / 16.0 * std::f64::consts::TAU;
        let p = [0, 1, 2].map(|i| {
            centre[i]
                + spread * (a.cos() * e1[i] + a.sin() * e2[i]) / (std::f64::consts::PI / 16.0).cos()
        });
        let z = dot(p, view.forward);
        if z <= 1e-6 {
            return full;
        }
        let x = dot(p, right) / z / view.half_fov;
        let y = dot(p, view.up) / z / view.half_fov;
        lo = [lo[0].min(x), lo[1].min(y)];
        hi = [hi[0].max(x), hi[1].max(y)];
    }
    let margin = 3.0 / (SIZE as f64 / 2.0);
    let r = [
        (lo[0] - margin).max(-1.0),
        (lo[1] - margin).max(-1.0),
        (hi[0] + margin).min(1.0),
        (hi[1] + margin).min(1.0),
    ];
    if r[0] >= r[2] || r[1] >= r[3] {
        return None;
    }
    Some(r.map(|v| v as f32))
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    fn read_texture(gpu: &Graphics, texture: &wgpu::Texture) -> Vec<u8> {
        let row = SIZE * 4;
        let padded =
            row.div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT) * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
        let buffer = gpu.device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: padded as u64 * SIZE as u64,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = gpu.device.create_command_encoder(&Default::default());
        encoder.copy_texture_to_buffer(
            texture.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(padded),
                    rows_per_image: Some(SIZE),
                },
            },
            wgpu::Extent3d {
                width: SIZE,
                height: SIZE,
                depth_or_array_layers: 1,
            },
        );
        gpu.queue.submit(Some(encoder.finish()));
        let slice = buffer.slice(..);
        slice.map_async(wgpu::MapMode::Read, |_| {});
        gpu.device.poll(wgpu::PollType::wait_indefinitely()).ok();
        let data = slice.get_mapped_range().expect("mapped");
        let mut out = Vec::with_capacity((row * SIZE) as usize);
        for y in 0..SIZE as usize {
            out.extend_from_slice(&data[y * padded as usize..][..row as usize]);
        }
        out
    }

    /// Run the streamer until everything wanted at `t` is in.
    fn settle(solar: &mut Solar, gpu: &Graphics, t: f64) {
        for _ in 0..4000 {
            solar.stream(t);
            solar.draw(gpu, t, 0);
            gpu.device.poll(wgpu::PollType::wait_indefinitely()).ok();
            if !solar.streamer.busy() {
                break;
            }
        }
    }

    /// The moments of one visit worth a look: the start, middle and end of
    /// every crossing, each orbit high and low and the dive and climb between,
    /// the departure, and the jump as it spools up, at full speed and as it
    /// drops out.
    fn moments(tour: &Tour) -> Vec<(String, f64)> {
        let mut out = Vec::new();
        for segment in &tour.segments {
            let at = |u: f64| segment.start + segment.duration * u;
            let end = segment.start + segment.duration;
            match segment.phase {
                Phase::Transfer { from, to } => {
                    let from = from.map_or("dropout".to_string(), |p| format!("p{p}"));
                    out.push((
                        format!("transfer-{from}-to-p{to}-start"),
                        segment.start + 0.5,
                    ));
                    out.push((format!("transfer-{from}-to-p{to}-cruise"), at(0.5)));
                    out.push((format!("transfer-{from}-to-p{to}-arrival"), end - 1.5));
                }
                Phase::High(p) => out.push((format!("p{p}-high-orbit"), at(0.5))),
                Phase::Descend(p) => out.push((format!("p{p}-descend"), at(0.5))),
                Phase::Low(p) => out.push((format!("p{p}-low-orbit"), at(0.5))),
                Phase::Climb(p) => out.push((format!("p{p}-climb"), at(0.6))),
                Phase::Depart(_) => {
                    out.push(("depart-start".into(), segment.start + 1.0));
                    out.push(("depart-spool".into(), end - 1.0));
                }
                Phase::Jump => {
                    out.push(("jump-entry".into(), at(0.04)));
                    out.push(("jump-cruise".into(), at(0.5)));
                    out.push(("jump-dropout".into(), end - 2.0));
                }
            }
        }
        out
    }

    /// Screenshots at the critical moments of a visit, for each seed in
    /// `SOLAR_SHOTS_SEEDS` (comma separated, default 5,11,23), written to
    /// `SOLAR_SHOTS_DIR` (default `target/solar-shots`): one PNG per moment,
    /// a contact sheet per seed, and the HUD at each moment in `hud.txt`.
    /// `SOLAR_SHOTS=low` keeps only the moments whose names contain `low`.
    /// The streamer is run until the moment's maps are in, so each shot shows
    /// the finished assets.
    #[test]
    #[ignore = "writes screenshots; needs a working GPU adapter"]
    fn screenshots() {
        let dir = std::env::var("SOLAR_SHOTS_DIR").unwrap_or_else(|_| "target/solar-shots".into());
        let seeds: Vec<u64> = std::env::var("SOLAR_SHOTS_SEEDS")
            .unwrap_or_else(|_| "5,11,23".into())
            .split(',')
            .filter_map(|s| s.trim().parse().ok())
            .collect();
        let only = std::env::var("SOLAR_SHOTS").ok();
        std::fs::create_dir_all(&dir).expect("a screenshot directory");
        let gpu = crate::gpu::headless().expect("a GPU adapter");
        let mut notes = String::new();
        for seed in seeds {
            let mut solar = Solar::with_seed(&gpu, seed);
            let system = &solar.tour.system;
            notes.push_str(&format!(
                "seed {seed}: {} ({}-type, {:.0} K), itinerary {:?}\n",
                system.star.name, system.star.class, system.star.temperature, solar.tour.itinerary
            ));
            for (i, planet) in system.planets.iter().enumerate() {
                notes.push_str(&format!(
                    "  p{i} {} {:?} r={:.2} moons=[{}]{}\n",
                    planet.name,
                    planet.kind,
                    planet.radius,
                    planet
                        .moons
                        .iter()
                        .map(|m| format!("{} {:?}", m.name, m.kind))
                        .collect::<Vec<_>>()
                        .join(", "),
                    if planet.rings.is_some() { " rings" } else { "" }
                ));
            }
            let mut thumbs = Vec::new();
            for (k, (label, t)) in moments(&solar.tour).into_iter().enumerate() {
                if only.as_deref().is_some_and(|o| !label.contains(o)) {
                    continue;
                }
                settle(&mut solar, &gpu, t);
                solar.draw(&gpu, t, 0);
                let pixels = read_texture(&gpu, &solar.frames[0].texture);
                let name = format!("seed{seed}-{k:02}-{label}");
                let path = std::path::Path::new(&dir).join(format!("{name}.png"));
                image::save_buffer(&path, &pixels, SIZE, SIZE, image::ExtendedColorType::Rgba8)
                    .expect("a screenshot");
                solar.hud_at = f64::NEG_INFINITY;
                solar.update_hud(t);
                notes.push_str(&format!("  {name} (t={t:.1}s) {}\n", solar.hud.title));
                for (l, v) in solar.hud.labels.lines().zip(solar.hud.values.lines()) {
                    notes.push_str(&format!("      {l:>13}  {v}\n"));
                }
                if let Some(image) = image::RgbaImage::from_raw(SIZE, SIZE, pixels) {
                    thumbs.push(image::imageops::thumbnail(&image, 270, 270));
                }
            }
            // A contact sheet, six across.
            let across = 6u32;
            let rows = (thumbs.len() as u32).div_ceil(across);
            let mut sheet = image::RgbaImage::new(270 * across, 270 * rows.max(1));
            for (i, thumb) in thumbs.iter().enumerate() {
                let (x, y) = ((i as u32 % across) * 270, (i as u32 / across) * 270);
                image::imageops::overlay(&mut sheet, thumb, x as i64, y as i64);
            }
            sheet
                .save(std::path::Path::new(&dir).join(format!("seed{seed}-sheet.png")))
                .expect("a contact sheet");
        }
        std::fs::write(std::path::Path::new(&dir).join("hud.txt"), notes).expect("the HUD notes");
    }

    /// One finished frame and its HUD at the first moment whose name holds
    /// `moment`, for the UI previews.
    pub(crate) fn render_once(seed: u64, moment: &str) -> (Vec<u8>, Hud) {
        let gpu = crate::gpu::headless().expect("a GPU adapter");
        let mut solar = Solar::with_seed(&gpu, seed);
        let t = moments(&solar.tour)
            .into_iter()
            .find(|(label, _)| label.contains(moment))
            .map_or(60.0, |(_, t)| t);
        settle(&mut solar, &gpu, t);
        solar.draw(&gpu, t, 0);
        solar.hud_at = f64::NEG_INFINITY;
        solar.update_hud(t);
        (
            read_texture(&gpu, &solar.frames[0].texture),
            solar.hud.clone(),
        )
    }

    /// Frame times in the heaviest views, and how long streaming a planet
    /// takes in frames.
    #[test]
    #[ignore = "a benchmark; needs a working GPU adapter"]
    fn frame_time() {
        let gpu = crate::gpu::headless().expect("a GPU adapter");
        let mut solar = Solar::with_seed(&gpu, 5);
        let started = std::time::Instant::now();
        let mut frames = 0;
        let t0 = 1.0;
        loop {
            solar.stream(t0);
            solar.draw(&gpu, t0, frames % SLOTS);
            frames += 1;
            if !solar.streamer.busy() || frames > 5000 {
                break;
            }
        }
        gpu.device.poll(wgpu::PollType::wait_indefinitely()).ok();
        println!(
            "solar: first planet and sky streamed in {frames} frames, {:.0} ms",
            started.elapsed().as_secs_f64() * 1e3
        );
        for (label, t) in moments(&solar.tour) {
            settle(&mut solar, &gpu, t);
            let n = 30;
            let started = std::time::Instant::now();
            for i in 0..n {
                solar.draw(&gpu, t + i as f64 * 0.05, i % SLOTS);
            }
            gpu.device.poll(wgpu::PollType::wait_indefinitely()).ok();
            println!(
                "solar {label}: {:.2} ms a frame",
                started.elapsed().as_secs_f64() * 1e3 / n as f64
            );
        }
    }
}
