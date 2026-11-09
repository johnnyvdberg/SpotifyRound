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

pub mod scan;
pub mod stream;
pub mod system;
pub mod tour;

use anyhow::Result;
use slint::wgpu_30::wgpu;

use crate::gpu::Graphics;
use crate::visuals::Visual;
use stream::{Key, Streamer, sampler_entry, texture_entry, uniform_entry};
use system::{Kind, Planet, Stop, System, equatorial_to_system};
use tour::{Phase, Tour, View, normalize};

/// Size of the rendered frame.
pub const SIZE: u32 = 1080;
const MAX_BODIES: usize = 16;
/// Cloud layers stacked over the deck in low orbit: each a pass of about
/// 330 instructions where clouds are, so one (a second added little).
const CLOUD_LAYERS: u32 = 1;
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
    light: [f32; 4],
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
    /// The scanner's readings, while orbiting.
    pub scans: Vec<scan::Scan>,
    /// The minimap of the system, while in one.
    pub map: Option<Minimap>,
    /// The manoeuvre under way and why, while not orbiting.
    pub manoeuvre: Option<tour::Manoeuvre>,
}

/// A small tilted map of the system: its orbits, its planets, the route
/// being flown and where the ship is, laid out in a `MAP` square.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Minimap {
    /// Path commands for every orbit.
    pub orbits: String,
    /// Path commands for the route still to fly.
    pub route: String,
    pub dots: Vec<MapDot>,
    pub ship: [f32; 2],
}

/// A planet, or the star, on the minimap.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MapDot {
    pub at: [f32; 2],
    pub size: f32,
    /// sRGB.
    pub colour: [u8; 3],
    pub label: String,
    /// The stop being visited or made for.
    pub current: bool,
}

/// The minimap's side, in its own units.
pub const MAP: f32 = 240.0;

pub(crate) struct Solar {
    /// The frame's small pipelines, by name: `space`, `warp1`..`warp5`,
    /// `star`, `flat`, `limb`, `ring`, and per map slot `ground0`, `sea0`,
    /// `clouds0`, `glow0`, `moon0` (and `…1`).
    /// Each with its label, which the Pi estimate matches to the shader-db.
    pipelines: std::collections::HashMap<String, (String, wgpu::RenderPipeline)>,
    /// A drive to use for every jump instead of each system's own, for the
    /// screenshots.
    drive_override: Option<u32>,
    layout: wgpu::BindGroupLayout,
    uniforms: wgpu::Buffer,
    sampler: wgpu::Sampler,
    bind_group: Option<wgpu::BindGroup>,
    /// Each ringed planet's ring density by radius, a row per planet, and the
    /// system it was painted for.
    rings: wgpu::Texture,
    rings_for: u64,
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
                texture_entry(13, d2),
            ],
        });
        let mut source = String::from(include_str!("noise.wgsl"));
        source.push_str(crate::noise3d::WGSL);
        source.push_str(include_str!("frame.wgsl"));
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Solar frame"),
            source: wgpu::ShaderSource::Wgsl(source.into()),
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Solar frame"),
            bind_group_layouts: &[Some(&layout), Some(&streamer.noise().layout)],
            immediate_size: 0,
        });
        let pipeline = |label: &str,
                        vs: &str,
                        fs: &str,
                        blend: Option<wgpu::BlendState>,
                        constants: &[(&str, f64)]| {
            let options = wgpu::PipelineCompilationOptions {
                constants,
                ..Default::default()
            };
            let made = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(&pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some(vs),
                    buffers: &[],
                    compilation_options: options.clone(),
                },
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some(fs),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: wgpu::TextureFormat::Rgba8UnormSrgb,
                        blend,
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                    compilation_options: options,
                }),
                primitive: wgpu::PrimitiveState::default(),
                depth_stencil: None,
                multisample: wgpu::MultisampleState::default(),
                multiview_mask: None,
                cache: None,
            });
            (label.to_string(), made)
        };
        let over = Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING);
        let add = Some(wgpu::BlendState {
            color: wgpu::BlendComponent {
                src_factor: wgpu::BlendFactor::One,
                dst_factor: wgpu::BlendFactor::One,
                operation: wgpu::BlendOperation::Add,
            },
            alpha: wgpu::BlendComponent {
                src_factor: wgpu::BlendFactor::Zero,
                dst_factor: wgpu::BlendFactor::One,
                operation: wgpu::BlendOperation::Add,
            },
        });
        let multiply = Some(wgpu::BlendState {
            color: wgpu::BlendComponent {
                src_factor: wgpu::BlendFactor::Dst,
                dst_factor: wgpu::BlendFactor::Zero,
                operation: wgpu::BlendOperation::Add,
            },
            alpha: wgpu::BlendComponent {
                src_factor: wgpu::BlendFactor::Zero,
                dst_factor: wgpu::BlendFactor::One,
                operation: wgpu::BlendOperation::Add,
            },
        });
        // Twice the product: the shader outputs half the factor, so the 8-bit
        // target's clamp to 1 still lets it brighten (up to 2x) as well as
        // darken.
        let double_multiply = Some(wgpu::BlendState {
            color: wgpu::BlendComponent {
                src_factor: wgpu::BlendFactor::Dst,
                dst_factor: wgpu::BlendFactor::Src,
                operation: wgpu::BlendOperation::Add,
            },
            alpha: wgpu::BlendComponent {
                src_factor: wgpu::BlendFactor::Zero,
                dst_factor: wgpu::BlendFactor::One,
                operation: wgpu::BlendOperation::Add,
            },
        });
        let mut pipelines = std::collections::HashMap::new();
        pipelines.insert(
            "space".to_string(),
            pipeline("Solar space", "vs_background", "fs_space", None, &[]),
        );
        for (k, name) in tour::DRIVES.iter().enumerate() {
            let k = k as u32 + 1;
            pipelines.insert(
                format!("warp{k}"),
                pipeline(
                    &format!("Solar warp {name}"),
                    "vs_background",
                    "fs_warp",
                    None,
                    &[("WARP", k as f64)],
                ),
            );
        }
        for (name, entry, blend) in [
            ("star", "fs_star", over),
            ("flat", "fs_flat", over),
            ("limb", "fs_limb", add),
            ("ring", "fs_ring", over),
            ("eclipse", "fs_eclipse", multiply),
            ("ringshadow", "fs_ring_shadow", multiply),
        ] {
            pipelines.insert(
                name.to_string(),
                pipeline(&format!("Solar {name}"), "vs_body", entry, blend, &[]),
            );
        }
        for slot in 0..2 {
            for (name, entry, blend) in [
                ("ground", "fs_ground", over),
                ("detail", "fs_detail", double_multiply),
                ("cloudshadow", "fs_cloud_shadow", multiply),
                ("sea", "fs_sea", add),
                ("clouds", "fs_clouds", over),
                ("glow", "fs_glow", add),
                ("moon", "fs_moon", over),
            ] {
                pipelines.insert(
                    format!("{name}{slot}"),
                    pipeline(
                        &format!("Solar {name} {slot}"),
                        "vs_body",
                        entry,
                        blend,
                        &[("SLOT", slot as f64)],
                    ),
                );
            }
        }
        // The stacked cloud layers drawn in low orbit, per slot.
        for slot in 0..2 {
            for layer in 1..=CLOUD_LAYERS {
                pipelines.insert(
                    format!("cloudlayer{slot}x{layer}"),
                    pipeline(
                        &format!("Solar cloud layer {slot} {layer}"),
                        "vs_body",
                        "fs_cloud_layer",
                        over,
                        &[("SLOT", slot as f64), ("LAYER", layer as f64)],
                    ),
                );
            }
        }
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
                    view_formats: &[wgpu::TextureFormat::Rgba8UnormSrgb],
                });
                // Drawn through an sRGB view: the passes blend in linear light
                // and the frame Slint reads is encoded.
                let view = texture.create_view(&wgpu::TextureViewDescriptor {
                    format: Some(wgpu::TextureFormat::Rgba8UnormSrgb),
                    ..Default::default()
                });
                Frame { texture, view }
            })
            .collect();
        let tour = Tour::new(system_for(seed, 0), system_for(seed, 1));
        let rings = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Solar ring profiles"),
            size: wgpu::Extent3d {
                width: RING_TEXELS,
                height: RING_ROWS,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::R8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        Self {
            pipelines,
            drive_override: None,
            layout,
            uniforms,
            sampler,
            bind_group: None,
            rings,
            rings_for: u64::MAX,
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
        let (current, second) = self.tour.wanted(t);
        let system = &self.tour.system;
        let mut wanted: Vec<(Key, Planet)> = [current, second]
            .into_iter()
            .flatten()
            .map(|s| ((system.seed, s.key()), system.world(s)))
            .collect();
        let jumping = matches!(segment.phase, Phase::Jump);
        let departing = matches!(segment.phase, Phase::Depart(_));
        if jumping || (departing && wanted.len() < 2) {
            if wanted.len() >= 2 {
                wanted.pop();
            }
            let next = &self.tour.next;
            let first = next.itinerary()[0];
            wanted.push(((next.seed, first.key()), next.world(first)));
        }
        let mut skies = vec![(system.seed, system.sky)];
        if jumping || departing {
            skies.push((self.tour.next.seed, self.tour.next.sky));
        }
        let wanted: Vec<(Key, &Planet)> = wanted.iter().map(|(k, p)| (*k, p)).collect();
        self.streamer.want(&wanted, &skies);
    }

    fn bind(&mut self, gpu: &Graphics) {
        if self.rings_for != self.tour.system.seed {
            paint_rings(gpu, &self.rings, &self.tour.system);
            self.rings_for = self.tour.system.seed;
        }
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
        let rings = full(&self.rings);
        entries.push(wgpu::BindGroupEntry {
            binding: 13,
            resource: wgpu::BindingResource::TextureView(&rings),
        });
        self.bind_group = Some(gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Solar frame"),
            layout: &self.layout,
            entries: &entries,
        }));
        self.bound = wanted;
    }

    /// The shader's parameters at `t` into the visit, and the draws: which
    /// pipeline over which body, in order.
    fn params(&self, t: f64) -> (Params, Vec<(u32, String)>) {
        let view = self.tour.view(t);
        let system = &self.tour.system;
        let camera = view.position;
        let rel = |p: [f64; 3]| [p[0] - camera[0], p[1] - camera[1], p[2] - camera[2]];
        let f32x4 = |v: [f64; 3], w: f64| [v[0] as f32, v[1] as f32, v[2] as f32, w as f32];

        let star_rel = rel([0.0; 3]);
        let hidden = view.warp > 0.3;
        let mut bodies: Vec<(f64, Body, Vec<String>)> = Vec::new();
        if !hidden {
            let mut star = Body {
                place: f32x4(star_rel, system.star.radius),
                // A star's look: -1, then its surface's cell scale, their
                // contrast and its limb darkening.
                look: [
                    -1.0,
                    system.star.surface[0],
                    system.star.surface[1],
                    system.star.surface[2],
                ],
                ..Default::default()
            };
            if let Some(rect) = rect(&view, star_rel, system.star.radius * 1.02) {
                star.rect = rect;
                bodies.push((length(star_rel), star, vec!["star".into()]));
            }
            for (index, planet) in system.planets.iter().enumerate() {
                let centre = planet.position(t);
                let place = rel(centre);
                let slot = self.streamer.ready((system.seed, Stop::planet(index).key()));
                let mut body = Body {
                    place: f32x4(place, planet.radius),
                    rot: rotation(
                        planet,
                        spin_angle(planet, t, centre),
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
                    // The ring profile's row is the planet's index.
                    body.ring = [
                        rings.inner as f32,
                        rings.outer as f32,
                        rings.opacity,
                        index as f32,
                    ];
                    body.ring_colour = [rings.colour[0], rings.colour[1], rings.colour[2], 1.0];
                    bound = planet.radius * rings.outer;
                }
                if let Some(rect) = rect(&view, place, bound) {
                    body.rect = rect;
                    let eclipsed = eclipsed(system, t, centre, planet.radius, Some(index));
                    let close = length(place) < planet.radius * 1.6;
                    let passes =
                        surface_passes(planet.kind, slot, eclipsed, planet.rings.is_some(), close);
                    bodies.push((length(place), body, passes));
                }
                // Moons: a moon the size of a planet in full once its own
                // maps are in, the others once their planet's are.
                for (m, moon) in planet.moons.iter().enumerate().take(stream::MOONS as usize) {
                    let stop = Stop {
                        planet: index,
                        moon: Some(m),
                    };
                    let world = if moon.world {
                        self.streamer.ready((system.seed, stop.key()))
                    } else {
                        None
                    };
                    if world.is_none() && slot.is_none() {
                        continue;
                    }
                    let at = system.centre(stop, t);
                    let place = rel(at);
                    let angle = moon.phase + t * std::f64::consts::TAU / moon.period;
                    let air = moon.kind.air();
                    let bound = moon.radius * if air[0] > 0.0 { 1.08 } else { 1.0 };
                    let Some(rect) = rect(&view, place, bound) else {
                        continue;
                    };
                    let eclipsed = eclipsed(system, t, at, moon.radius, None);
                    let (look, passes) = match (world, slot) {
                        (Some(w), _) => (
                            [moon.kind as u32 as f32, w as f32, -1.0, night_mode(moon.kind)],
                            surface_passes(
                                moon.kind,
                                Some(w),
                                eclipsed,
                                false,
                                length(place) < moon.radius * 1.6,
                            ),
                        ),
                        (None, Some(s)) => {
                            let mut passes = vec![format!("moon{s}")];
                            if eclipsed {
                                passes.push("eclipse".into());
                            }
                            if air[0] > 0.0 {
                                passes.push("limb".into());
                            }
                            (
                                [moon.kind as u32 as f32, s as f32, m as f32, night_mode(moon.kind)],
                                passes,
                            )
                        }
                        (None, None) => continue,
                    };
                    bodies.push((
                        length(place),
                        Body {
                            place: f32x4(place, moon.radius),
                            // Locked to its planet, one face toward it; the
                            // clouds of a world drift on their own.
                            rot: rotation(
                                planet,
                                angle + std::f64::consts::PI,
                                t / moon.period * 0.2,
                            ),
                            look,
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
                        passes,
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
        let mut draws = Vec::new();
        for (index, (slot, (_, body, passes))) in list.iter_mut().zip(bodies.iter()).enumerate() {
            *slot = *body;
            for pass in passes {
                draws.push((index as u32, pass.clone()));
            }
        }

        // How much of the star's disc the bodies leave in view.
        let star_distance = length(star_rel);
        let star_dir = normalize(star_rel);
        let star_angle = (system.star.radius / star_distance).min(1.0).asin();
        let mut visible = 1.0;
        for (distance, body, _) in &bodies {
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
            light: [
                system.star.light[0],
                system.star.light[1],
                system.star.light[2],
                0.0,
            ],
            sky,
            warp: [
                view.warp as f32,
                view.flash as f32,
                t as f32,
                (t - (self.tour.total - tour::JUMP)) as f32,
            ],
            bodies: list,
        };
        (params, draws)
    }

    fn draw(&mut self, gpu: &Graphics, t: f64, frame: usize) {
        let mut encoder = gpu
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("Solar encoder"),
            });
        self.streamer.advance(gpu, &mut encoder);
        self.bind(gpu);
        let (params, draws) = self.params(t);
        let background = if params.warp[0] > 0.02 {
            format!("warp{}", self.drive().0)
        } else {
            "space".to_string()
        };
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
            pass.set_bind_group(1, &self.streamer.noise().group, &[]);
            let (label, pipeline) = &self.pipelines[&background];
            pass.set_pipeline(pipeline);
            let full = (SIZE * SIZE) as u64;
            crate::gpu::counted_draw(&mut pass, label, full, 0..3, 0..1);
            for (body, name) in &draws {
                let (label, pipeline) = &self.pipelines[name];
                pass.set_pipeline(pipeline);
                let r = params.bodies[*body as usize].rect;
                let covered = ((r[2] - r[0]).max(0.0) * (r[3] - r[1]).max(0.0) / 4.0 * full as f32) as u64;
                crate::gpu::counted_draw(&mut pass, label, covered, 0..6, *body..*body + 1);
            }
        }
        gpu.submit(encoder.finish());
    }

    /// The drive this visit's jump uses: its number and name.
    fn drive(&self) -> (u32, &'static str) {
        match self.drive_override {
            Some(k) => (k, tour::DRIVES[(k as usize - 1) % tour::DRIVES.len()]),
            None => self.tour.drive(),
        }
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
            scans: self.tour.scans(t),
            map: self.minimap(t),
            manoeuvre: self.tour.manoeuvre(t),
        };
    }

    /// The minimap at `t`: none during the jump, between systems.
    fn minimap(&self, t: f64) -> Option<Minimap> {
        let current = self.tour.current(t)?;
        let system = &self.tour.system;
        // Distances from the star on a log scale, so close-in planets and
        // the outermost giant all fit; the plane seen tilted, from above.
        let inner = system.planets.first().map_or(1.0, |p| p.orbit) * 0.5;
        let outer = system.planets.last().map_or(1.0, |p| p.orbit);
        let reach = (MAP as f64) * 0.45;
        let squash = 0.5;
        let radius = |r: f64| reach * (1.0 + r / inner).ln() / (1.0 + outer / inner).ln();
        let centre = MAP as f64 / 2.0;
        let project = |p: [f64; 3]| -> [f32; 2] {
            let flat = (p[0] * p[0] + p[2] * p[2]).sqrt();
            let r = radius(flat);
            let k = if flat > 1e-9 { r / flat } else { 0.0 };
            [
                (centre + p[0] * k) as f32,
                (centre + p[2] * k * squash - p[1] * k * 0.8) as f32,
            ]
        };
        let mut orbits = String::new();
        for planet in &system.planets {
            let (rx, ry) = (radius(planet.orbit), radius(planet.orbit) * squash);
            orbits.push_str(&format!(
                "M {:.1} {:.1} A {rx:.1} {ry:.1} 0 1 0 {:.1} {:.1} A {rx:.1} {ry:.1} 0 1 0 {:.1} {:.1} ",
                centre - rx,
                centre,
                centre + rx,
                centre,
                centre - rx,
                centre
            ));
        }
        let mut route = String::new();
        for (i, p) in self.tour.route(t).into_iter().enumerate() {
            let [x, y] = project(p);
            route.push_str(&format!("{} {x:.1} {y:.1} ", if i == 0 { "M" } else { "L" }));
        }
        let to_srgb = |c: [f32; 3], lift: f32| {
            c.map(|v| ((v.max(0.0).powf(1.0 / 2.2) * (1.0 - lift) + lift) * 255.0) as u8)
        };
        let mut dots = vec![MapDot {
            at: [centre as f32; 2],
            size: match system.star.kind {
                system::StarKind::RedGiant => 14.0,
                system::StarKind::WhiteDwarf => 5.0,
                system::StarKind::Main => 9.0,
            },
            colour: to_srgb(system.star.light.map(|c| c.min(1.0)), 0.3),
            label: String::new(),
            current: false,
        }];
        let visited: Vec<usize> = self.tour.itinerary.iter().map(|s| s.planet).collect();
        for (i, planet) in system.planets.iter().enumerate() {
            let colour = match planet.kind {
                Kind::Terran | Kind::Ocean | Kind::Jungle | Kind::Eyeball => [0.12, 0.3, 0.6],
                _ => planet.palette[0],
            };
            dots.push(MapDot {
                at: project(planet.position(t)),
                size: if planet.kind.is_giant() { 8.0 } else { 5.0 },
                colour: to_srgb(colour, 0.25),
                // The planet's letter, for the stops.
                label: if visited.contains(&i) {
                    planet.name.rsplit(' ').next().unwrap_or("").to_string()
                } else {
                    String::new()
                },
                current: current.planet == i,
            });
        }
        Some(Minimap {
            orbits,
            route,
            dots,
            ship: project(self.tour.view(t).position),
        })
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

/// Texels along a ring profile, and rows (one per planet).
const RING_TEXELS: u32 = 1024;
const RING_ROWS: u32 = 8;

/// Smooth 1D value noise in [0, 1], for the ring profiles.
fn noise1(x: f64, seed: u64) -> f64 {
    let hash = |i: i64| {
        let mut z = (i as u64).wrapping_mul(0x9e37_79b9_7f4a_7c15) ^ seed;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        ((z ^ (z >> 31)) >> 11) as f64 / (1u64 << 53) as f64
    };
    let i = x.floor();
    let f = x - i;
    let u = f * f * f * (f * (f * 6.0 - 15.0) + 10.0);
    hash(i as i64) * (1.0 - u) + hash(i as i64 + 1) * u
}

/// Paint the ring density profiles of `system`'s ringed planets, one row
/// each: ringlets, gaps and a main division.
fn paint_rings(gpu: &Graphics, texture: &wgpu::Texture, system: &System) {
    let mut data = vec![0u8; (RING_TEXELS * RING_ROWS) as usize];
    for (row, planet) in system.planets.iter().enumerate().take(RING_ROWS as usize) {
        let Some(rings) = &planet.rings else {
            continue;
        };
        let seed = rings.seed.to_bits() as u64;
        for t in 0..RING_TEXELS as usize {
            let x = (t as f64 + 0.5) / RING_TEXELS as f64;
            let n1 = noise1(x * 40.0, seed);
            let n2 = noise1(x * 160.0, seed ^ 1);
            let n3 = noise1(x * 9.0, seed ^ 2);
            let division = 1.0 - (-((x - 0.62) / 0.025).powi(2)).exp() * 0.9;
            let edges = smoothstep(0.0, 0.04, x) * smoothstep(1.0, 0.9, x);
            let density = ((0.25 + 0.75 * n3)
                * (0.45 + 0.55 * n1)
                * (0.6 + 0.4 * n2)
                * division
                * edges
                * 1.4)
                .clamp(0.0, 1.0);
            data[row * RING_TEXELS as usize + t] = (density * 255.0).round() as u8;
        }
    }
    gpu.queue.write_texture(
        texture.as_image_copy(),
        &data,
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(RING_TEXELS),
            rows_per_image: Some(RING_ROWS),
        },
        wgpu::Extent3d {
            width: RING_TEXELS,
            height: RING_ROWS,
            depth_or_array_layers: 1,
        },
    );
}

/// Whether another body of `system` could pass in front of the star as seen
/// from a body of `radius` at `at`: only then is its eclipse pass drawn.
fn eclipsed(system: &System, t: f64, at: [f64; 3], radius: f64, planet: Option<usize>) -> bool {
    let to_star = normalize([-at[0], -at[1], -at[2]]);
    let star_distance = length(at);
    let star_angle = (system.star.radius / star_distance).min(1.0).asin();
    let check = |centre: [f64; 3], r: f64| {
        let rel = [centre[0] - at[0], centre[1] - at[1], centre[2] - at[2]];
        let d = length(rel);
        if d < 1e-6 || d > star_distance {
            return false;
        }
        let separation = dot(normalize(rel), to_star).clamp(-1.0, 1.0).acos();
        separation < (r / d).min(1.0).asin() + star_angle + (radius / d).min(1.0).asin()
    };
    for (index, other) in system.planets.iter().enumerate() {
        let centre = other.position(t);
        if Some(index) != planet && check(centre, other.radius) {
            return true;
        }
        for m in 0..other.moons.len().min(stream::MOONS as usize) {
            let offset = other.moon_offset(m, t);
            let c = [
                centre[0] + offset[0],
                centre[1] + offset[1],
                centre[2] + offset[2],
            ];
            if length([c[0] - at[0], c[1] - at[1], c[2] - at[2]]) > 1e-6
                && check(c, other.moons[m].radius)
            {
                return true;
            }
        }
    }
    false
}

/// The passes that draw a planet-sized body of `kind` from map slot `slot`
/// (or flat, if its maps are not in), in order.
fn surface_passes(
    kind: Kind,
    slot: Option<usize>,
    eclipsed: bool,
    rings: bool,
    close: bool,
) -> Vec<String> {
    let mut passes: Vec<String> = match slot {
        Some(s) => {
            let mut passes = vec![format!("ground{s}")];
            // Close-up detail, only near the ground of a world with one: the
            // pass costs a frame's worth of pixels, so it is skipped higher up.
            if close && kind.has_ground() {
                passes.push(format!("detail{s}"));
            }
            if kind.has_clouds() {
                passes.push(format!("cloudshadow{s}"));
            }
            if kind.has_sea() {
                passes.push(format!("sea{s}"));
            }
            if kind.has_clouds() {
                passes.push(format!("clouds{s}"));
                // In low orbit, clouds that stand up from the deck.
                if close {
                    for layer in 1..=CLOUD_LAYERS {
                        passes.push(format!("cloudlayer{s}x{layer}"));
                    }
                }
            }
            passes
        }
        None => vec!["flat".into()],
    };
    if eclipsed {
        passes.push("eclipse".into());
    }
    if rings {
        passes.push("ringshadow".into());
    }
    if let Some(s) = slot {
        passes.push(format!("glow{s}"));
    }
    if kind.air()[0] > 0.0 {
        passes.push("limb".into());
    }
    if rings {
        passes.push("ring".into());
    }
    passes
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
        Kind::Terran | Kind::Jungle | Kind::Eyeball => [0.06, 0.12, 0.22, 0.0],
        Kind::Ocean => [0.04, 0.10, 0.24, 0.0],
        Kind::GasGiant | Kind::IceGiant | Kind::HotJupiter => {
            let c = planet.palette[0];
            [c[0], c[1], c[2], 0.25]
        }
        _ => {
            let c = planet.palette[0];
            [c[0], c[1], c[2], 0.0]
        }
    }
}

/// How far `planet` has turned on its axis at `t`. A world locked to its star
/// keeps its +z (longitude 0, where the generator puts its warm side) facing
/// the star instead.
fn spin_angle(planet: &Planet, t: f64, centre: [f64; 3]) -> f64 {
    if !planet.kind.locked() {
        return t * std::f64::consts::TAU / planet.day;
    }
    let to_star = normalize([-centre[0], -centre[1], -centre[2]]);
    let ex = equatorial_to_system(planet, [1.0, 0.0, 0.0]);
    let ez = equatorial_to_system(planet, [0.0, 0.0, 1.0]);
    dot(to_star, ex).atan2(dot(to_star, ez))
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
                    let from = from.map_or("dropout".to_string(), |p| p.to_string());
                    out.push((
                        format!("transfer-{from}-to-{to}-start"),
                        segment.start + 0.5,
                    ));
                    match tour.flyby_at(to) {
                        Some(closest) => {
                            out.push((format!("transfer-{from}-to-{to}-flyby-near"), closest - 3.0));
                            out.push((format!("transfer-{from}-to-{to}-flyby"), closest));
                        }
                        None => out.push((format!("transfer-{from}-to-{to}-cruise"), at(0.5))),
                    }
                    out.push((format!("transfer-{from}-to-{to}-arrival"), end - 1.5));
                }
                Phase::High(p) => out.push((format!("{p}-high-orbit"), at(0.5))),
                Phase::Descend(p) => out.push((format!("{p}-descend"), at(0.5))),
                Phase::Low(p) => out.push((format!("{p}-low-orbit"), at(0.5))),
                Phase::Climb(p) => out.push((format!("{p}-climb"), at(0.6))),
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
    /// `SOLAR_SHOTS_SEEDS` (comma separated, default 0,1,2,5: between them
    /// every kind of planet and moon is visited), written to
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
            .unwrap_or_else(|_| "0,1,2,5".into())
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
                "seed {seed}: {} ({}, {:.0} K), itinerary {:?}\n",
                system.star.name, system.star.title(), system.star.temperature, solar.tour.itinerary
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

    /// Every hyperjump drive at a few moments of a jump, to
    /// `SOLAR_SHOTS_DIR` (default `target/solar-shots`): `warp-<drive>-<n>.png`
    /// and a sheet of them all, a row per drive.
    #[test]
    #[ignore = "writes screenshots; needs a working GPU adapter"]
    fn warps() {
        let dir = std::env::var("SOLAR_SHOTS_DIR").unwrap_or_else(|_| "target/solar-shots".into());
        std::fs::create_dir_all(&dir).expect("a screenshot directory");
        let gpu = crate::gpu::headless().expect("a GPU adapter");
        let mut solar = Solar::with_seed(&gpu, 11);
        let start = solar.tour.total - tour::JUMP;
        let moments = [0.03, 0.12, 0.5, 0.85];
        let mut sheet =
            image::RgbaImage::new(270 * moments.len() as u32, 270 * tour::DRIVES.len() as u32);
        for (k, name) in tour::DRIVES.iter().enumerate() {
            solar.drive_override = Some(k as u32 + 1);
            for (m, at) in moments.iter().enumerate() {
                let t = start + tour::JUMP * at;
                solar.draw(&gpu, t, 0);
                let pixels = read_texture(&gpu, &solar.frames[0].texture);
                let file = name.to_lowercase().replace(' ', "-");
                let path = std::path::Path::new(&dir).join(format!("warp-{file}-{m}.png"));
                image::save_buffer(&path, &pixels, SIZE, SIZE, image::ExtendedColorType::Rgba8)
                    .expect("a screenshot");
                if let Some(image) = image::RgbaImage::from_raw(SIZE, SIZE, pixels) {
                    let thumb = image::imageops::thumbnail(&image, 270, 270);
                    image::imageops::overlay(&mut sheet, &thumb, 270 * m as i64, 270 * k as i64);
                }
            }
        }
        sheet
            .save(std::path::Path::new(&dir).join("warps-sheet.png"))
            .expect("a sheet");
    }

    /// One finished frame and its HUD at the first moment whose name holds
    /// `moment`, for the UI previews.
    pub(crate) fn render_once(seed: u64, moment: &str) -> (Vec<u8>, Hud) {
        let gpu = crate::gpu::headless().expect("a GPU adapter");
        let mut solar = Solar::with_seed(&gpu, seed);
        let t = moments(&solar.tour)
            .into_iter()
            .find(|(label, _)| label.ends_with(moment))
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

    /// Fragment counts for the Pi estimate: every draw of one frame at each
    /// key moment of a visit, for each seed in `PI_ESTIMATE_SEEDS` (default
    /// 5,23), and every draw of each frame while the first planet streams in.
    /// Written to `target/pi-estimate/solar.tsv`; `deploy/pi-estimate.sh`
    /// turns them into milliseconds on the Pi.
    #[test]
    #[ignore = "counts fragments for the Pi estimate; needs a GPU adapter with pipeline statistics"]
    fn pi_estimate() {
        let gpu = crate::gpu::headless().expect("a GPU adapter");
        assert!(
            crate::gpu::start_counting(&gpu.device),
            "this adapter cannot count fragments ({})",
            gpu.adapter
        );
        let seeds: Vec<u64> = std::env::var("PI_ESTIMATE_SEEDS")
            .unwrap_or_else(|_| "5,23".into())
            .split(',')
            .filter_map(|s| s.trim().parse().ok())
            .collect();
        let mut out = String::from("scene\tmoment\tframe\tlabel\tcovered\tkept\n");
        let mut push = |moment: &str, frame: usize, counts: Vec<(String, u64, u64)>| {
            for (label, covered, kept) in counts {
                out.push_str(&format!("solar\t{moment}\t{frame}\t{label}\t{covered}\t{kept}\n"));
            }
        };
        for seed in seeds {
            let mut solar = Solar::with_seed(&gpu, seed);
            // Streaming the first planet in: every frame's draws.
            let mut frame = 0;
            loop {
                solar.stream(1.0);
                solar.draw(&gpu, 1.0, frame % SLOTS);
                push(&format!("seed{seed} streaming"), frame, crate::gpu::take_counts(&gpu));
                frame += 1;
                if !solar.streamer.busy() || frame > 4000 {
                    break;
                }
            }
            for (label, t) in moments(&solar.tour) {
                settle(&mut solar, &gpu, t);
                crate::gpu::take_counts(&gpu);
                solar.draw(&gpu, t, 0);
                push(&format!("seed{seed} {label}"), 0, crate::gpu::take_counts(&gpu));
            }
        }
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("target/pi-estimate");
        std::fs::create_dir_all(&dir).expect("an output directory");
        std::fs::write(dir.join("solar.tsv"), out).expect("the counts");
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
