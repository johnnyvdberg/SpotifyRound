//! What is in the water around the Nautilus, frame by frame.
//!
//! Everything is placed procedurally and deterministically: seabed features
//! come from cells of the world, so the same rock is always in the same place;
//! swimmers travel in processions at their own speed through the water, so the
//! Nautilus overtakes some and is passed by others. Each frame, the cells in
//! view are visited and turned into model instances — a fish is one instance,
//! a diver a dozen capsules — plus sprites for snow, bubbles and glows and the
//! point lights that matter most.
//!
//! The noise, biome mix and terrain height here are the same functions as in
//! `scene.wgsl`, so a rock placed on the seabed sits on the seabed the GPU draws.

use std::f32::consts::{PI, TAU};

use super::helm::Pose;
use super::mesh::Model;
use super::textures::Surface;
use crate::voyage::Biome;

pub const SHALLOWS: usize = 0;
pub const FOREST: usize = 1;
pub const WRECKS: usize = 2;
pub const ATLANTIS: usize = 3;
pub const CAVE: usize = 4;
pub const ABYSS: usize = 5;
pub const ARCTIC: usize = 6;

/// Height of the cave's vault above its floor, and the ice's thickness.
pub const CAVE_ROOF: f32 = 1.6;
pub const ICE: f32 = 0.15;

/// Half the view's width at unit distance.
pub const VIEW: f32 = 0.41;

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, bytemuck::Pod, bytemuck::Zeroable)]
pub struct Instance {
    pub rows: [[f32; 4]; 3],
    pub colour: [f32; 4],
    pub anim: [f32; 4],
    /// x: material; y: 1 if it stands on the seabed; z: how far it has faded
    /// in, as its biome comes and goes.
    pub extra: [f32; 4],
    /// x: texture layer; y: repeats per unit; z: 1 to texture in the model's
    /// own space, so the pattern moves with a swimming body.
    pub surface: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, bytemuck::Pod, bytemuck::Zeroable)]
pub struct Sprite {
    pub place: [f32; 4],
    pub tint: [f32; 4],
    /// x: how far the sprite is drawn out along the view's horizontal, for
    /// bubbles streaking past in a fast passage.
    pub stretch: [f32; 4],
}

#[derive(Clone, Copy, Debug)]
pub struct Light {
    pub pos: [f32; 3],
    pub range: f32,
    pub colour: [f32; 3],
}

/// Animations, as the vertex shader knows them.
pub mod anim {
    pub const SWAY: f32 = 1.0;
    pub const SWIM: f32 = 2.0;
    pub const FLUKE: f32 = 3.0;
    pub const WRITHE: f32 = 4.0;
    pub const PULSE: f32 = 5.0;
}

/// Materials, as the fragment shader knows them.
pub mod material {
    pub const LAVA: f32 = 1.0;
    pub const COPPER: f32 = 2.0;
    pub const SURFACE: f32 = 4.0;
    pub const VAULT: f32 = 5.0;
    pub const ICE: f32 = 6.0;
    pub const GOLD: f32 = 8.0;
}

// Sprite kinds.
const MOTE: f32 = 0.0;
const BUBBLE: f32 = 1.0;
const GLOW: f32 = 2.0;
const EMBER: f32 = 3.0;

pub fn hash(ix: i32, iz: i32, salt: u32) -> f32 {
    let mut h = (ix as u32).wrapping_mul(0x8da6_b343)
        ^ (iz as u32).wrapping_mul(0xd816_3841)
        ^ salt.wrapping_mul(0xcb1a_b31f);
    h = (h ^ (h >> 13)).wrapping_mul(0x5bd1_e995);
    h ^= h >> 15;
    (h & 0x00ff_ffff) as f32 / 16_777_216.0
}

pub fn vnoise(x: f32, z: f32, salt: u32) -> f32 {
    let (fx, fz) = (x.floor(), z.floor());
    let (ix, iz) = (fx as i32, fz as i32);
    let (tx, tz) = (x - fx, z - fz);
    let ux = tx * tx * (3.0 - 2.0 * tx);
    let uz = tz * tz * (3.0 - 2.0 * tz);
    let a = hash(ix, iz, salt);
    let b = hash(ix + 1, iz, salt);
    let c = hash(ix, iz + 1, salt);
    let d = hash(ix + 1, iz + 1, salt);
    let top = a + (b - a) * ux;
    let bottom = c + (d - c) * ux;
    top + (bottom - top) * uz
}

fn smooth(a: f32, b: f32, v: f32) -> f32 {
    let t = ((v - a) / (b - a)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// The line the voyage follows across the seabed: a gentle meander in z.
/// Everything — the biome borders, the far mountains, the cave's tunnel and
/// the lanes the swimmers keep to — is laid out about it.
pub fn corridor(x: f32) -> f32 {
    2.2 * (x * 0.11 + 0.7).sin() + 0.8 * (x * 0.031).sin()
}

/// The corridor's slope, dz/dx.
pub fn corridor_slope(x: f32) -> f32 {
    2.2 * 0.11 * (x * 0.11 + 0.7).cos() + 0.8 * 0.031 * (x * 0.031).cos()
}

pub fn voyage_x(x: f32, z: f32) -> f32 {
    let off = z - corridor(x);
    x + 1.2 * (off * 0.31).sin() + 0.8 * ((off * 0.73 + 1.3).sin() - 1.3f32.sin())
}

/// The lava cave is a tunnel through a massif: its radius, and how high the
/// rock rises either side of it.
pub const TUNNEL: f32 = 1.05;
pub const MASSIF: f32 = 4.5;
/// The cave's share of the biome mix from which its tunnel is built.
pub const TUNNEL_SHARE: f32 = 0.85;

/// Visual depth of each biome's seabed below the surface, which lies at y = 0
/// whatever the boat does. The voyage's own depths run from one to forty
/// units; here they are compressed, so the drop into the abyss is a steep
/// shelf rather than a gulf. The gauge undoes this (`helm::depth_reading`).
pub const SEABED: [f32; 7] = [1.7, 3.6, 4.9, 5.8, 6.6, 9.3, 3.5];
/// How strongly the heightmap's relief shows in each biome: rugged in the
/// abyss, nearly flat under the paved city of Atlantis.
pub const RELIEF: [f32; 7] = [0.25, 0.35, 0.35, 0.12, 0.20, 1.20, 0.40];

/// The seabed of one visit: its heightmap and the biome sequence's seed.
pub struct Seabed {
    pub map: Vec<[f32; 4]>,
    pub seed: f32,
}

impl Seabed {
    pub fn new(visit: u64, seed: f32) -> Self {
        Self {
            map: super::heightmap::generate(visit),
            seed,
        }
    }

    pub fn biome(&self, x: f32, z: f32) -> Biome {
        Biome::at(f64::from(voyage_x(x, z)), self.seed)
    }

    pub fn share(&self, x: f32, z: f32, id: usize) -> f32 {
        self.biome(x, z).weight(id) as f32
    }

    fn by_biome(b: &Biome, values: [f32; 7]) -> f32 {
        values[b.a] + (values[b.b] - values[b.a]) * b.w as f32
    }

    /// The seabed's height at (x, z), exactly as `ground` in the shader: the
    /// biome's depth, the heightmap's relief, a little fine roughness and the
    /// mountains that rise in the far distance.
    pub fn ground(&self, x: f32, z: f32) -> f32 {
        let b = self.biome(x, z);
        let base = -Self::by_biome(&b, SEABED);
        let relief = Self::by_biome(&b, RELIEF) * super::heightmap::sample(&self.map, x, z)[0];
        let rough = Self::by_biome(&b, [0.05, 0.07, 0.05, 0.02, 0.03, 0.08, 0.05]);
        let ridges = Self::by_biome(&b, [1.6, 2.2, 2.0, 2.6, 3.5, 5.0, 1.4]);
        let n = 0.65 * vnoise(x * 1.3, z * 1.3, 1) + 0.35 * vnoise(x * 3.1 + 5.0, z * 3.1, 2);
        let off = (z - corridor(x)).abs();
        let far = smooth(7.0, 22.0, off) * (0.3 + ridges * vnoise(x * 0.16, z * 0.16 + 3.0, 4));
        // The lava cave's massif, rising either side of its tunnel.
        let massif = b.weight(CAVE) as f32
            * MASSIF
            * smooth(TUNNEL * 1.05, TUNNEL * 1.8, off)
            * (0.85 + 0.3 * vnoise(x * 0.4, z * 0.4, 5));
        base + relief + rough * (n - 0.45) + far + massif
    }

    /// The tunnel's cross-section at `x`, where there is one: its centre, its
    /// sideways direction and how far the cave has taken over.
    pub fn tunnel_at(&self, x: f32) -> Option<([f32; 3], [f32; 3], f32)> {
        let zc = corridor(x);
        let share = self.share(x, zc, CAVE);
        if share < TUNNEL_SHARE {
            return None;
        }
        let floor = self.tunnel_floor(x);
        let s = corridor_slope(x);
        let n = (1.0 + s * s).sqrt();
        Some((
            [x, floor + 0.25 * TUNNEL, zc],
            [-s / n, 0.0, 1.0 / n],
            share,
        ))
    }

    /// The tunnel's floor at `x`: the cave's own floor, level through the
    /// massif, rising over the ground's real bumps but never following it down
    /// the shelf beyond, so the arch runs level to its mouths.
    pub fn tunnel_floor(&self, x: f32) -> f32 {
        let local = (0..5)
            .map(|i| {
                let ax = x - 0.5 + i as f32 * 0.25;
                self.ground(ax, corridor(ax))
            })
            .fold(f32::MIN, f32::max);
        local.max(-SEABED[CAVE] - 0.2)
    }

    /// The tunnel's ragged inner radius at `x` and angle `a` around it.
    pub fn tunnel_radius(x: f32, a: f32) -> f32 {
        TUNNEL * (0.91 + 0.18 * vnoise(x * 2.2, a * 3.0, 6))
    }

    /// Height of the lava cave's vault.
    pub fn vault(&self) -> f32 {
        -SEABED[CAVE] + CAVE_ROOF
    }

    /// The highest the boat may rise at `x`: a little under the surface or
    /// the ice, or well inside the tunnel's arch, which it settles under before
    /// it reaches the mouth.
    pub fn ceiling(&self, x: f32) -> f32 {
        let zc = corridor(x);
        let b = self.biome(x, zc);
        let ice = smooth(0.0, 0.3, b.weight(ARCTIC) as f32);
        let open = -0.30;
        let mut ceiling = open + (-ICE - 0.35 - open) * ice;
        let cave = smooth(TUNNEL_SHARE - 0.35, TUNNEL_SHARE, b.weight(CAVE) as f32);
        if cave > 0.0 {
            let floor = self.tunnel_floor(x);
            let arch = floor + 0.25 * TUNNEL + 0.85 * TUNNEL * 0.91 - 0.35;
            ceiling += (arch.min(ceiling) - ceiling) * cave;
        }
        ceiling
    }
}

/// The world around one frame of the voyage.
pub struct World<'a> {
    pub seabed: &'a Seabed,
    pub pose: Pose,
    pub t: f32,
}

impl World<'_> {
    pub fn biome(&self, x: f32, z: f32) -> Biome {
        self.seabed.biome(x, z)
    }

    pub fn share(&self, x: f32, z: f32, id: usize) -> f32 {
        self.seabed.share(x, z, id)
    }

    fn by_biome(b: &Biome, values: [f32; 7]) -> f32 {
        Seabed::by_biome(b, values)
    }

    pub fn ground(&self, x: f32, z: f32) -> f32 {
        self.seabed.ground(x, z)
    }

    pub fn camera(&self) -> f32 {
        self.pose.pos[0]
    }

    /// The window's height.
    pub fn eye(&self) -> f32 {
        self.pose.pos[1]
    }

    /// The sea's surface, at the same height wherever the boat goes.
    pub fn surface(&self) -> f32 {
        0.0
    }

    pub fn vault(&self) -> f32 {
        self.seabed.vault()
    }

    /// The underside of the polar ice, floating at the surface.
    pub fn ice(&self) -> f32 {
        -ICE
    }

    /// Whether a sphere of `radius` at `p` can be seen through the window,
    /// which is round: the corners of the square the sea is drawn into are
    /// behind the brass.
    pub fn visible(&self, p: [f32; 3], radius: f32) -> bool {
        let d = [
            p[0] - self.pose.pos[0],
            p[1] - self.pose.pos[1],
            p[2] - self.pose.pos[2],
        ];
        let dot = |a: [f32; 3]| a[0] * d[0] + a[1] * d[1] + a[2] * d[2];
        let [right, up, out] = self.pose.basis;
        let z = dot(out);
        if z + radius < 0.08 {
            return false;
        }
        // A sphere touches the cone of view up to a little past its radius,
        // as the cone's side slants.
        let reach = VIEW * z.max(0.0) * 1.08 + radius * 1.1;
        let (x, y) = (dot(right), dot(up));
        x * x + y * y < reach * reach
    }
}

/// The lengths of a frame's lists at one moment.
struct Mark {
    solid: Vec<usize>,
    sprites: usize,
    lights: usize,
}

/// Everything to draw in one frame.
pub struct Frame {
    /// Opaque instances per model.
    pub solid: Vec<Vec<Instance>>,
    /// Translucent instances per model.
    pub glass: Vec<Vec<Instance>>,
    pub sprites: Vec<Sprite>,
    pub lights: Vec<Light>,
    /// The tunnel through the lava cave's massif, in world space for the
    /// stretch in view. It depends only on the seabed and that stretch, so it
    /// is kept from frame to frame and rebuilt only when the stretch moves on
    /// by a ring, every few seconds rather than every frame.
    pub tunnel: Vec<super::mesh::Vertex>,
    pub tunnel_indices: Vec<u32>,
    /// Counts the tunnel's rebuilds, so whoever keeps a copy of it on the GPU
    /// can tell when theirs is out of date.
    pub tunnel_version: u64,
    /// The stretch of rings the tunnel was built for, and the seabed's seed.
    tunnel_span: Option<(i32, i32, u32)>,
    /// How far what is being placed has faded in, as its biome comes and goes.
    fade: f32,
    /// The texture for models that have none of their own, while placing one
    /// kind of thing: marble for the ruins, planking for the wrecks.
    skin: Option<[f32; 4]>,
}

impl Frame {
    pub fn new() -> Self {
        Self {
            solid: (0..Model::ALL.len()).map(|_| Vec::new()).collect(),
            glass: (0..Model::ALL.len()).map(|_| Vec::new()).collect(),
            sprites: Vec::new(),
            lights: Vec::new(),
            tunnel: Vec::new(),
            tunnel_indices: Vec::new(),
            tunnel_version: 0,
            tunnel_span: None,
            fade: 1.0,
            skin: None,
        }
    }

    pub fn clear(&mut self) {
        for list in self.solid.iter_mut().chain(self.glass.iter_mut()) {
            list.clear();
        }
        self.sprites.clear();
        self.lights.clear();
    }

    /// Build the tunnel afresh next time: the seabed has changed.
    pub fn forget_tunnel(&mut self) {
        self.tunnel_span = None;
    }

    fn instance(&self, model: Model, rows: [[f32; 4]; 3], colour: [f32; 3]) -> Instance {
        Instance {
            rows,
            colour: [colour[0], colour[1], colour[2], 0.0],
            extra: [0.0, 0.0, self.fade, 0.0],
            surface: own_skin(model)
                .or(self.skin)
                .unwrap_or([0.0, 1.0, 0.0, 0.0]),
            ..Default::default()
        }
    }

    fn put(&mut self, model: Model, rows: [[f32; 4]; 3], colour: [f32; 3]) -> &mut Instance {
        let instance = self.instance(model, rows, colour);
        let list = &mut self.solid[model.index()];
        list.push(instance);
        list.last_mut().unwrap()
    }

    fn glass(&mut self, model: Model, rows: [[f32; 4]; 3], colour: [f32; 3]) -> &mut Instance {
        let instance = self.instance(model, rows, colour);
        let list = &mut self.glass[model.index()];
        list.push(instance);
        list.last_mut().unwrap()
    }

    /// Where the lists stand now, to turn what is added after.
    fn mark(&self) -> Mark {
        Mark {
            solid: self.solid.iter().map(Vec::len).collect(),
            sprites: self.sprites.len(),
            lights: self.lights.len(),
        }
    }

    /// Turn everything placed since `mark` by `yaw` about `pivot`.
    fn turn_since(&mut self, mark: &Mark, pivot: [f32; 3], yaw: f32) {
        let (sy, cy) = yaw.sin_cos();
        let r = [[cy, 0.0, sy], [0.0, 1.0, 0.0], [-sy, 0.0, cy]];
        let spin = |p: [f32; 3]| {
            let d = [p[0] - pivot[0], p[1] - pivot[1], p[2] - pivot[2]];
            [0, 1, 2].map(|i| pivot[i] + r[i][0] * d[0] + r[i][1] * d[1] + r[i][2] * d[2])
        };
        for (list, &from) in self.solid.iter_mut().zip(&mark.solid) {
            for instance in &mut list[from..] {
                let m = instance.rows;
                let t = spin([m[0][3], m[1][3], m[2][3]]);
                for (i, row) in instance.rows.iter_mut().enumerate() {
                    *row = [0, 1, 2, 3].map(|j| {
                        if j == 3 {
                            t[i]
                        } else {
                            r[i][0] * m[0][j] + r[i][1] * m[1][j] + r[i][2] * m[2][j]
                        }
                    });
                }
            }
        }
        for sprite in &mut self.sprites[mark.sprites..] {
            let p = spin([sprite.place[0], sprite.place[1], sprite.place[2]]);
            sprite.place[..3].copy_from_slice(&p);
        }
        for light in &mut self.lights[mark.lights..] {
            light.pos = spin(light.pos);
        }
    }

    /// Start placing one kind of thing, with `skin` for its plain parts.
    fn style(&mut self, skin: Option<[f32; 4]>) {
        self.fade = 1.0;
        self.skin = skin;
    }

    fn sprite(&mut self, p: [f32; 3], size: f32, colour: [f32; 3], kind: f32) {
        self.sprites.push(Sprite {
            place: [p[0], p[1], p[2], size],
            tint: [colour[0], colour[1], colour[2], kind],
            stretch: [1.0, 0.0, 0.0, 0.0],
        });
    }

    fn streak(&mut self, p: [f32; 3], size: f32, colour: [f32; 3], stretch: f32) {
        self.sprites.push(Sprite {
            place: [p[0], p[1], p[2], size],
            tint: [colour[0], colour[1], colour[2], GLOW],
            stretch: [stretch, 0.0, 0.0, 0.0],
        });
    }

    fn light(&mut self, pos: [f32; 3], range: f32, colour: [f32; 3]) {
        self.lights.push(Light { pos, range, colour });
    }
}

trait Tweak {
    fn grounded(&mut self) -> &mut Self;
    fn moving(&mut self, kind: f32, phase: f32, amp: f32, freq: f32) -> &mut Self;
    fn material(&mut self, m: f32) -> &mut Self;
    fn glowing(&mut self, glow: f32) -> &mut Self;
    /// Textured in world space: for things that stay put.
    fn textured(&mut self, surface: Surface, repeats: f32) -> &mut Self;
}

impl Tweak for Instance {
    fn grounded(&mut self) -> &mut Self {
        self.extra[1] = 1.0;
        self
    }
    fn moving(&mut self, kind: f32, phase: f32, amp: f32, freq: f32) -> &mut Self {
        self.anim = [kind, phase, amp, freq];
        self
    }
    fn material(&mut self, m: f32) -> &mut Self {
        self.extra[0] = m;
        self
    }
    fn glowing(&mut self, glow: f32) -> &mut Self {
        self.colour[3] = glow;
        self
    }
    fn textured(&mut self, surface: Surface, repeats: f32) -> &mut Self {
        self.surface = [surface.layer(), repeats, 0.0, 0.0];
        self
    }
}

/// Texture in world space: for things that stay put, so the pattern has the
/// same scale everywhere.
fn fixed(surface: Surface, repeats: f32) -> Option<[f32; 4]> {
    Some([surface.layer(), repeats, 0.0, 0.0])
}

/// Texture in a model's own space: for things that move and bend.
fn carried(surface: Surface, repeats: f32) -> Option<[f32; 4]> {
    Some([surface.layer(), repeats, 1.0, 0.0])
}

/// The texture a model always wears; plain shapes take their context's.
fn own_skin(model: Model) -> Option<[f32; 4]> {
    match model {
        Model::Rock0 | Model::Rock1 | Model::Rock2 => fixed(Surface::Rock, 3.0),
        Model::Kelp => carried(Surface::Kelp, 3.0),
        Model::Fish => carried(Surface::Scales, 1.0),
        Model::Shark => carried(Surface::Skin, 2.0),
        Model::Whale => carried(Surface::Skin, 4.0),
        Model::Dolphin | Model::Narwhal => carried(Surface::Skin, 5.0),
        Model::Mantle | Model::Tube => carried(Surface::Skin, 4.0),
        Model::Shaft | Model::Capital | Model::Break | Model::Prism => fixed(Surface::Marble, 3.0),
        Model::Hull => carried(Surface::Wood, 3.0),
        Model::Sail => carried(Surface::Canvas, 1.5),
        Model::Fan | Model::Star | Model::Shell | Model::Sponge => carried(Surface::Coral, 2.5),
        Model::Keel => fixed(Surface::Ice, 1.5),
        Model::Hex => fixed(Surface::Basalt, 2.5),
        Model::Capsule => carried(Surface::Canvas, 1.0),
        _ => None,
    }
}

/// How far something has faded in, given its biome's share and the share it
/// needs: things grow out of the seabed and melt away rather than pop.
fn fade(share: f32, needs: f32) -> f32 {
    smooth(needs, needs + 0.2, share)
}

/// A transform: rotate by yaw (about y), pitch (about z) and roll (about x),
/// in that order outermost first, scale, and move to `pos`.
pub fn place(pos: [f32; 3], yaw: f32, pitch: f32, roll: f32, scale: [f32; 3]) -> [[f32; 4]; 3] {
    let (sy, cy) = yaw.sin_cos();
    let (sp, cp) = pitch.sin_cos();
    let (sr, cr) = roll.sin_cos();
    let ry = [[cy, 0.0, sy], [0.0, 1.0, 0.0], [-sy, 0.0, cy]];
    let rz = [[cp, -sp, 0.0], [sp, cp, 0.0], [0.0, 0.0, 1.0]];
    let rx = [[1.0, 0.0, 0.0], [0.0, cr, -sr], [0.0, sr, cr]];
    let m = mul(mul(ry, rz), rx);
    let mut rows = [[0.0; 4]; 3];
    for r in 0..3 {
        for c in 0..3 {
            rows[r][c] = m[r][c] * scale[c];
        }
        rows[r][3] = pos[r];
    }
    rows
}

fn mul(a: [[f32; 3]; 3], b: [[f32; 3]; 3]) -> [[f32; 3]; 3] {
    let mut m = [[0.0; 3]; 3];
    for (r, row) in m.iter_mut().enumerate() {
        for (c, cell) in row.iter_mut().enumerate() {
            *cell = (0..3).map(|k| a[r][k] * b[k][c]).sum();
        }
    }
    m
}

fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

fn norm(v: [f32; 3]) -> [f32; 3] {
    let l = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt().max(1e-6);
    [v[0] / l, v[1] / l, v[2] / l]
}

fn add(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

/// A model whose y axis runs from `a` to `b`, `radius` thick: limbs, masts,
/// trunks, tentacles.
pub fn between(a: [f32; 3], b: [f32; 3], radius: f32) -> [[f32; 4]; 3] {
    let d = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
    let length = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt().max(1e-5);
    let y = [d[0] / length, d[1] / length, d[2] / length];
    let reference = if y[1].abs() < 0.9 {
        [0.0, 1.0, 0.0]
    } else {
        [1.0, 0.0, 0.0]
    };
    let x = norm(cross(y, reference));
    let z = cross(x, y);
    let mut rows = [[0.0; 4]; 3];
    for r in 0..3 {
        rows[r] = [x[r] * radius, y[r] * length, z[r] * radius, a[r]];
    }
    rows
}

/// The cells of a grid of `size` that can be in view between `near` and
/// `far`, as (column, row, x, z) with the point jittered within its cell.
/// The view's footprint on the seabed between `near` and `far`, boxed in
/// world x and z: whichever way the boat is turned.
fn footprint(w: &World, near: f32, far: f32) -> ([f32; 4], [f32; 2], [f32; 2]) {
    let [right, _, out] = w.pose.basis;
    let flat = |v: [f32; 3]| {
        let l = (v[0] * v[0] + v[2] * v[2]).sqrt().max(1e-4);
        [v[0] / l, v[2] / l]
    };
    let (o, r) = (flat(out), flat(right));
    let c = w.pose.pos;
    let spread = VIEW * 1.3;
    let mut bounds = [f32::MAX, f32::MIN, f32::MAX, f32::MIN];
    for d in [near, far] {
        for side in [-1.0, 1.0] {
            let x = c[0] + o[0] * d + r[0] * side * d * spread;
            let z = c[2] + o[1] * d + r[1] * side * d * spread;
            bounds = [
                bounds[0].min(x),
                bounds[1].max(x),
                bounds[2].min(z),
                bounds[3].max(z),
            ];
        }
    }
    (bounds, o, r)
}

/// Where in its cell of `size` the thing of cell (ix, iz) stands.
fn cell_point(ix: i32, iz: i32, size: f32, salt: u32) -> (f32, f32) {
    (
        (ix as f32 + 0.5 + (hash(ix, iz, salt) - 0.5) * 0.8) * size,
        (iz as f32 + 0.5 + (hash(ix, iz, salt + 1) - 0.5) * 0.8) * size,
    )
}

/// The cells of a grid of `size` that can be in view between `near` and
/// `far` from the window, as (column, row, x, z) with the point jittered
/// within its cell. The grid is fixed in the world, so the same rock is
/// always in the same place however the boat turns.
fn cells(w: &World, size: f32, near: f32, far: f32, salt: u32) -> Vec<(i32, i32, f32, f32)> {
    let ([x0, x1, z0, z1], o, r) = footprint(w, near, far);
    let c = w.pose.pos;
    let mut out = Vec::new();
    for iz in ((z0 - size) / size).floor() as i32..=((z1 + size) / size).ceil() as i32 {
        for ix in ((x0 - size) / size).floor() as i32..=((x1 + size) / size).ceil() as i32 {
            let (x, z) = cell_point(ix, iz, size, salt);
            let (dx, dz) = (x - c[0], z - c[2]);
            let depth = dx * o[0] + dz * o[1];
            if depth < near || depth > far {
                continue;
            }
            if (dx * r[0] + dz * r[1]).abs() > depth * VIEW * 1.3 + size + 0.6 {
                continue;
            }
            out.push((ix, iz, x, z));
        }
    }
    out
}

/// Build the frame.
/// One kind of thing to place in the frame.
type Placer = fn(&World, &mut Frame);

pub fn build(world: &World, frame: &mut Frame) {
    frame.clear();
    let wood = carried(Surface::Wood, 1.5);
    let marble = fixed(Surface::Marble, 3.0);
    let rock = fixed(Surface::Rock, 3.0);
    let basalt = fixed(Surface::Basalt, 3.0);
    let algae = carried(Surface::Kelp, 2.0);
    let copper = carried(Surface::Copper, 1.0);
    let skin = carried(Surface::Skin, 3.0);
    let steps: [(Placer, Option<[f32; 4]>); 18] = [
        (planes, None),
        (rocks, None),
        (kelp, None),
        (forest, algae),
        (gardens, None),
        (wrecks, wood),
        (ruins, marble),
        (cave, rock),
        (tunnel, None),
        (vents, basalt),
        (ice, None),
        (divers, copper),
        (shoals, None),
        (great_swimmers, None),
        (squid, skin),
        (drifters, None),
        (polar, None),
        (particles, None),
    ];
    for (step, skin) in steps {
        frame.style(skin);
        step(world, frame);
    }
}

/// The surface, the cave's vault and the ice: great planes that follow the
/// camera; the shader fades them out where they do not belong. The surface is
/// the sea's, at the same height wherever the boat is.
fn planes(w: &World, f: &mut Frame) {
    // Centred on the boat, so they reach the horizon whichever way it turns.
    let c = w.pose.pos;
    let big = [40.0, 1.0, 40.0];
    if w.eye() > -6.0 {
        f.put(
            Model::Plane,
            place([c[0], w.surface(), c[2]], 0.0, 0.0, 0.0, big),
            [1.0; 3],
        )
        .material(material::SURFACE);
    }
    let near = (0..=4).any(|i| w.share(c[0] + i as f32 * 3.0 - 6.0, c[2], ARCTIC) > 0.0);
    if near && w.ice() > w.eye() {
        f.put(
            Model::Plane,
            place([c[0], w.ice(), c[2]], 0.0, 0.0, 0.0, big),
            [1.0; 3],
        )
        .material(material::ICE)
        .textured(Surface::Ice, 0.8);
    }
}

fn rocks(w: &World, f: &mut Frame) {
    for (ix, iz, x, z) in cells(w, 0.55, 0.6, 14.0, 10) {
        if hash(ix, iz, 12) < 0.55 {
            continue;
        }
        // Big boulders lie farther off; right by the glass only stones.
        let size = 0.05 + 0.20 * hash(ix, iz, 13).powf(2.0);
        // Boulders lie off the boat's course; only stones in its way.
        if size > 0.1 && (z - corridor(x)).abs() < 0.7 {
            continue;
        }
        let y = w.ground(x, z);
        let p = [x, y + size * 0.25, z];
        if !w.visible(p, size * 1.5) {
            continue;
        }
        let b = w.biome(x, z);
        let molten = b.weight(CAVE) as f32;
        let tone = World::by_biome(&b, [0.32, 0.24, 0.22, 0.30, 0.12, 0.14, 0.26]);
        let model =
            [Model::Rock0, Model::Rock1, Model::Rock2][(hash(ix, iz, 14) * 3.0) as usize % 3];
        let flat = 0.55 + 0.35 * hash(ix, iz, 15);
        let i = f.put(
            model,
            place(
                p,
                hash(ix, iz, 16) * TAU,
                0.0,
                0.0,
                [size * 1.2, size * flat, size],
            ),
            [tone, tone * 1.02, tone * 0.98],
        );
        i.grounded();
        if molten > 0.5 {
            i.textured(Surface::Basalt, 3.0);
        }
        if molten > 0.5 {
            i.material(material::LAVA);
        }
    }
}

fn kelp(w: &World, f: &mut Frame) {
    for (ix, iz, x, z) in cells(w, 0.30, 0.8, 15.0, 20) {
        let b = w.biome(x, z);
        let here = (b.weight(SHALLOWS) + 0.35 * b.weight(FOREST)) as f32 * 0.7;
        if here < hash(ix, iz, 21) * 0.9 + 0.1 {
            continue;
        }
        f.fade = fade(here, hash(ix, iz, 21) * 0.9 + 0.1);
        if (z - corridor(x)).abs() < 0.35 {
            continue;
        }
        let height = 0.5 + 1.0 * hash(ix, iz, 22);
        let y = w.ground(x, z);
        if !w.visible([x, y + height * 0.5, z], height * 0.6) {
            continue;
        }
        let gold = 0.8 + 0.4 * hash(ix, iz, 23);
        f.put(
            Model::Kelp,
            place(
                [x, y, z],
                hash(ix, iz, 24) * TAU,
                0.0,
                0.0,
                [0.9, height, 0.9],
            ),
            [0.46 * gold, 0.46 * gold, 0.14 * gold],
        )
        .moving(anim::SWAY, hash(ix, iz, 25) * TAU, 0.08, 0.8)
        .grounded();
    }
}

/// The submarine forest of Crespo: tall algae branching like trees.
fn forest(w: &World, f: &mut Frame) {
    for (ix, iz, x, z) in cells(w, 0.75, 1.0, 17.0, 30) {
        if w.share(x, z, FOREST) < hash(ix, iz, 31) * 0.7 + 0.2 {
            continue;
        }
        f.fade = fade(w.share(x, z, FOREST), hash(ix, iz, 31) * 0.7 + 0.2);
        if (z - corridor(x)).abs() < 0.6 {
            continue;
        }
        let h = 0.9 + 0.8 * hash(ix, iz, 32);
        let y = w.ground(x, z);
        if !w.visible([x, y + h * 0.5, z], h * 0.7) {
            continue;
        }
        let red = [0.46, 0.17, 0.12];
        let olive = [0.36, 0.34, 0.14];
        let t = hash(ix, iz, 33);
        let tint = [
            red[0] + (olive[0] - red[0]) * t,
            red[1] + (olive[1] - red[1]) * t,
            red[2] + (olive[2] - red[2]) * t,
        ];
        let lean = (hash(ix, iz, 34) - 0.5) * 0.15;
        let sway = (w.t * 0.35 + ix as f32).sin() * 0.02;
        let top = [x + (lean + sway) * h, y + h, z];
        f.put(
            Model::Cone,
            between([x, y - 0.02, z], top, 0.025),
            [tint[0] * 0.6, tint[1] * 0.6, tint[2] * 0.6],
        )
        .grounded();
        for k in 0..4 {
            let fk = k as f32;
            let by = 0.34 + 0.16 * fk;
            let root = [x + (lean + sway) * h * by, y + h * by, z];
            let a = hash(ix, iz, 35 + k) * TAU;
            let len = (0.24 - fk * 0.03) * h;
            let tip = [
                root[0] + a.cos() * len,
                root[1] + len * 0.6,
                root[2] + a.sin() * len * 0.6,
            ];
            f.put(
                Model::Cone,
                between(root, tip, 0.009),
                [tint[0] * 0.6, tint[1] * 0.6, tint[2] * 0.6],
            );
            f.put(
                Model::Sphere,
                place(tip, a, 0.0, 0.0, [0.09, 0.05, 0.07]),
                tint,
            )
            .moving(anim::SWAY, fk + ix as f32, 0.02, 0.6);
        }
        f.put(
            Model::Sphere,
            place(top, 0.0, 0.0, 0.0, [0.12, 0.08, 0.10]),
            tint,
        )
        .moving(anim::SWAY, iz as f32, 0.02, 0.5);
    }
}

/// Sea fans in the shallows and the forest, giant clams with their pearls,
/// and on the polar seabed sponges and starfish.
fn gardens(w: &World, f: &mut Frame) {
    for (ix, iz, x, z) in cells(w, 0.6, 0.8, 7.0, 40) {
        let b = w.biome(x, z);
        let green = (b.weight(SHALLOWS) + b.weight(FOREST)) as f32;
        let polar = b.weight(ARCTIC) as f32;
        let roll = hash(ix, iz, 41);
        let y = w.ground(x, z);
        if !w.visible([x, y + 0.1, z], 0.2) {
            continue;
        }
        f.fade = fade(green.max(polar), roll * 0.8 + 0.2);
        let yaw = hash(ix, iz, 42) * TAU;
        if green > roll * 0.8 + 0.2 {
            if b.weight(FOREST) > 0.5 && hash(ix, iz, 43) < 0.25 {
                // A tridacna gaping slowly, the pearl in its throat.
                let open = 0.35 + 0.2 * (w.t * 0.25 + ix as f32).sin();
                let s = 0.10;
                f.put(
                    Model::Shell,
                    place([x, y + 0.005, z], yaw, PI, 0.0, [s, s, s * 0.8]),
                    [0.55, 0.53, 0.47],
                )
                .grounded();
                f.put(
                    Model::Shell,
                    place([x, y + 0.01, z], yaw, open, 0.0, [s, s, s * 0.8]),
                    [0.55, 0.53, 0.47],
                );
                f.put(
                    Model::Sphere,
                    place([x, y + 0.03, z], 0.0, 0.0, 0.0, [0.018; 3]),
                    [0.9, 0.88, 0.82],
                )
                .glowing(0.35)
                .material(material::COPPER);
            } else {
                let hue = if hash(ix, iz, 44) < 0.5 {
                    [0.85, 0.28, 0.30]
                } else {
                    [0.90, 0.58, 0.20]
                };
                let s = 0.07 + 0.08 * hash(ix, iz, 45);
                f.put(Model::Fan, place([x, y, z], yaw, 0.0, 0.0, [s, s, s]), hue)
                    .moving(anim::SWAY, yaw, 0.15, 0.7)
                    .grounded();
            }
        } else if polar > roll * 0.8 + 0.2 {
            if hash(ix, iz, 46) < 0.5 {
                for k in 0..3 {
                    let a = k as f32 * 2.1 + yaw;
                    let h = 0.06 + 0.07 * hash(ix, iz, 47 + k);
                    f.put(
                        Model::Sponge,
                        place(
                            [x + a.cos() * 0.035, y - 0.005, z + a.sin() * 0.035],
                            a,
                            0.0,
                            0.0,
                            [0.018, h, 0.018],
                        ),
                        [0.80, 0.74, 0.52],
                    )
                    .grounded();
                }
            } else {
                let s = 0.035 + 0.02 * hash(ix, iz, 50);
                f.put(
                    Model::Star,
                    place([x, y + 0.005, z], yaw, 0.0, 0.0, [s, s, s]),
                    [0.85, 0.30, 0.16],
                )
                .grounded();
            }
        }
    }
}

/// The galleons of Vigo Bay, half buried, their treasure spilling out.
fn wrecks(w: &World, f: &mut Frame) {
    // A graveyard of ships: several in view at once, near and far.
    for (ix, iz, x, z) in cells(w, 2.4, 1.6, 10.0, 60) {
        if w.share(x, z, WRECKS) < 0.5 || hash(ix, iz, 61) < 0.35 {
            continue;
        }
        f.fade = fade(w.share(x, z, WRECKS), 0.5);
        if (z - corridor(x)).abs() < 1.2 {
            continue;
        }
        let length = 2.2 + 1.0 * hash(ix, iz, 62);
        let y = w.ground(x, z);
        if !w.visible([x, y + length * 0.25, z], length * 0.7) {
            continue;
        }
        let yaw = (hash(ix, iz, 63) - 0.5) * 1.2;
        let list = 0.25 * (hash(ix, iz, 64) - 0.3);
        let wood = [0.42, 0.31, 0.21];
        let p = [x, y + 0.04 * length, z];
        f.put(
            Model::Hull,
            place(p, yaw, 0.04, list, [length, length, length]),
            wood,
        )
        .grounded();
        // Castles fore and aft, masts and yards, a torn sail.
        let (s, c) = yaw.sin_cos();
        let along = |d: f32, up: f32| {
            [
                p[0] + c * d * length,
                p[1] + up * length,
                p[2] - s * d * length,
            ]
        };
        f.put(
            Model::Box,
            place(
                along(-0.38, 0.08),
                yaw,
                0.0,
                list,
                [0.10 * length, 0.05 * length, 0.10 * length],
            ),
            [0.27, 0.20, 0.14],
        );
        f.put(
            Model::Box,
            place(
                along(0.36, 0.07),
                yaw,
                0.0,
                list,
                [0.08 * length, 0.03 * length, 0.08 * length],
            ),
            [0.27, 0.20, 0.14],
        );
        let broken = hash(ix, iz, 65);
        for (k, d) in [-0.15f32, 0.15].iter().enumerate() {
            let h = if k == 1 && broken < 0.5 { 0.25 } else { 0.55 };
            let foot = along(*d, 0.04);
            let top = [foot[0] + list * 0.3, foot[1] + h * length, foot[2]];
            f.put(
                Model::Cylinder,
                between(foot, top, 0.008 * length),
                [0.24, 0.20, 0.15],
            );
            if h > 0.3 {
                let yard_y = top[1] - 0.05 * length;
                let half = [s * 0.16 * length, 0.0, c * 0.16 * length];
                let a = [top[0] - half[0], yard_y, top[2] - half[2]];
                let b = [top[0] + half[0], yard_y, top[2] + half[2]];
                f.put(
                    Model::Cylinder,
                    between(a, b, 0.005 * length),
                    [0.24, 0.20, 0.15],
                );
                f.put(
                    Model::Sail,
                    place(
                        [top[0], yard_y, top[2]],
                        yaw + PI * 0.5,
                        0.0,
                        0.0,
                        [0.15 * length, 0.22 * length, 0.15 * length],
                    ),
                    [0.42, 0.40, 0.32],
                )
                .moving(anim::SWAY, ix as f32, 0.02, 0.4);
            }
        }
        // A split chest by the bow, gold spilling over the sand.
        let chest = along(0.55, -0.02);
        let ground = w.ground(chest[0], chest[2]);
        let cp = [chest[0], ground + 0.025, chest[2]];
        f.put(
            Model::Box,
            place(cp, yaw + 0.4, 0.0, 0.0, [0.05, 0.03, 0.035]),
            [0.26, 0.18, 0.10],
        )
        .grounded();
        f.put(
            Model::Sphere,
            place(
                [cp[0] + 0.07, ground, cp[2] - 0.03],
                0.0,
                0.0,
                0.0,
                [0.08, 0.012, 0.05],
            ),
            [0.80, 0.58, 0.20],
        )
        .material(material::GOLD)
        .glowing(0.15);
        f.sprite(
            [cp[0] + 0.05, ground + 0.03, cp[2]],
            0.10,
            [0.20, 0.14, 0.04],
            GLOW,
        );
    }
}

/// The ruins of Atlantis: colonnades along avenues, many columns broken with
/// their drums fallen beside them, temple fronts farther off, and the volcano
/// whose fire lights the drowned city.
fn ruins(w: &World, f: &mut Frame) {
    let marble = [0.48, 0.46, 0.42];
    for (ix, iz, x, z) in cells(w, 0.55, 1.0, 16.0, 70) {
        // Colonnades stand in rows: whole rows of cells, and gaps between.
        if hash(0, iz, 71) < 0.45 || w.share(x, z, ATLANTIS) < hash(ix, iz, 72) * 0.5 + 0.4 {
            continue;
        }
        f.fade = fade(w.share(x, z, ATLANTIS), hash(ix, iz, 72) * 0.5 + 0.4);
        if hash(ix, iz, 73) < 0.15 {
            continue;
        }
        let x = (ix as f32 + 0.5) * 0.55;
        let z = (iz as f32 + 0.5) * 0.55;
        if (z - corridor(x)).abs() < 0.5 {
            continue;
        }
        let y = w.ground(x, z);
        let full = 0.65 + 0.15 * hash(ix, iz, 74);
        if !w.visible([x, y + full * 0.5, z], full * 0.6) {
            continue;
        }
        let r = 0.055;
        f.put(
            Model::Box,
            place([x, y + 0.02, z], 0.0, 0.0, 0.0, [r * 1.5, 0.025, r * 1.5]),
            marble,
        )
        .grounded();
        let broken = hash(ix, iz, 75);
        if broken < 0.45 {
            let h = full * (0.25 + broken);
            f.put(
                Model::Shaft,
                place([x, y + 0.04, z], 0.0, 0.0, 0.0, [r, h, r]),
                marble,
            )
            .grounded();
            f.put(
                Model::Break,
                place(
                    [x, y + 0.04 + h - 0.005, z],
                    broken * 9.0,
                    0.0,
                    0.0,
                    [r, r * 0.8, r],
                ),
                marble,
            );
            // A fallen drum at its foot.
            let a = hash(ix, iz, 76) * TAU;
            let foot = [
                x + a.cos() * 0.12,
                w.ground(x + a.cos() * 0.12, z + a.sin() * 0.12) + r * 0.9,
                z + a.sin() * 0.12,
            ];
            let end = [foot[0] + a.sin() * 0.12, foot[1], foot[2] - a.cos() * 0.12];
            f.put(Model::Cylinder, between(foot, end, r * 0.95), marble)
                .grounded();
        } else {
            f.put(
                Model::Shaft,
                place([x, y + 0.04, z], 0.0, 0.0, 0.0, [r, full, r]),
                marble,
            )
            .grounded();
            f.put(
                Model::Capital,
                place([x, y + 0.04 + full, z], 0.0, 0.0, 0.0, [r, 0.04, r]),
                marble,
            );
            f.put(
                Model::Box,
                place(
                    [x, y + 0.04 + full + 0.05, z],
                    0.0,
                    0.0,
                    0.0,
                    [r * 1.35, 0.012, r * 1.35],
                ),
                marble,
            );
        }
    }
    for (ix, iz, x, z) in cells(w, 5.0, 6.0, 22.0, 80) {
        if w.share(x, z, ATLANTIS) < 0.5 || hash(ix, iz, 81) < 0.3 {
            continue;
        }
        f.fade = fade(w.share(x, z, ATLANTIS), 0.5);
        if (z - corridor(x)).abs() < 2.0 {
            continue;
        }
        let s = 1.4 + 0.4 * hash(ix, iz, 82);
        let y = w.ground(x, z);
        if !w.visible([x, y + s * 0.4, z], s) {
            continue;
        }
        let yaw = (hash(ix, iz, 83) - 0.5) * 0.6;
        let (sn, cs) = yaw.sin_cos();
        let at = |d: f32, up: f32, depth: f32| {
            [
                x + cs * d * s + sn * depth * s,
                y + up * s,
                z - sn * d * s + cs * depth * s,
            ]
        };
        let stone = [0.46, 0.45, 0.41];
        f.put(
            Model::Box,
            place(
                at(0.0, 0.03, 0.0),
                yaw,
                0.0,
                0.0,
                [0.56 * s, 0.03 * s, 0.3 * s],
            ),
            stone,
        )
        .grounded();
        for k in 0..6 {
            let d = -0.40 + k as f32 * 0.16;
            let gone = hash(ix * 7 + k, iz, 84);
            let h = if gone < 0.3 { 0.12 + gone * 0.6 } else { 0.55 };
            f.put(
                Model::Shaft,
                place(
                    at(d, 0.06, -0.15),
                    yaw,
                    0.0,
                    0.0,
                    [0.03 * s, h * s, 0.03 * s],
                ),
                stone,
            );
        }
        f.put(
            Model::Box,
            place(
                at(0.0, 0.645, -0.15),
                yaw,
                0.0,
                0.0,
                [0.48 * s, 0.045 * s, 0.06 * s],
            ),
            stone,
        );
        f.put(
            Model::Prism,
            place(
                at(0.0, 0.69, -0.15),
                yaw,
                0.0,
                0.0,
                [0.50 * s, 0.17 * s, 0.06 * s],
            ),
            stone,
        );
    }
    // The volcano, far off across the city.
    f.style(fixed(Surface::Basalt, 0.6));
    for (ix, iz, x, z) in cells(w, 14.0, 18.0, 30.0, 90) {
        if w.share(x, z, ATLANTIS) < 0.4 || hash(ix, iz, 91) < 0.3 {
            continue;
        }
        f.fade = fade(w.share(x, z, ATLANTIS), 0.4);
        let y = w.ground(x, z) - 0.3;
        let h = 3.2;
        if !w.visible([x, y + h * 0.5, z], 5.0) {
            continue;
        }
        f.put(
            Model::Cone,
            place([x, y, z], 0.0, 0.0, 0.0, [4.5, h, 4.5]),
            [0.05, 0.04, 0.04],
        );
        let flicker = 0.85 + 0.15 * (w.t * 2.3 + ix as f32).sin() * (w.t * 1.3).sin();
        let crater = [x, y + h * 0.97, z];
        f.sprite(crater, 2.2, [0.9 * flicker, 0.30 * flicker, 0.06], GLOW);
        f.sprite(crater, 0.6, [1.0, 0.55, 0.15], GLOW);
        f.light(crater, 10.0, [0.7 * flicker, 0.24 * flicker, 0.07]);
        // A thread of lava down one flank.
        for k in 0..10 {
            let t = k as f32 / 10.0;
            let p = [
                x + t * 1.6 + (t * 9.0).sin() * 0.1,
                y + h * (0.95 - t * 0.9),
                z - 0.5,
            ];
            f.sprite(p, 0.18, [0.7 * flicker, 0.20, 0.04], GLOW);
        }
    }
}

/// The lava cave: basalt columns, stalactites dripping molten rock, and the
/// glow of the pools.
fn cave(w: &World, f: &mut Frame) {
    // Basalt columns stand against the tunnel's walls, clear of the boat's way.
    for (ix, iz, x, z) in cells(w, 0.16, 0.5, 9.0, 100) {
        let off = (z - corridor(x)).abs();
        if !(0.35..TUNNEL * 0.9).contains(&off) {
            continue;
        }
        let cluster = vnoise(x * 0.5, z * 0.5, 101);
        let share = w.share(x, z, CAVE);
        if cluster < 0.48 || share < 0.6 {
            continue;
        }
        f.fade = fade(share, 0.6);
        let y = w.ground(x, z);
        let h = 0.04 + 0.30 * smooth(0.48, 0.90, cluster) * (0.5 + 0.5 * hash(ix, iz, 102));
        if !w.visible([x, y + h * 0.5, z], h) {
            continue;
        }
        let v = 0.10 + 0.03 * hash(ix, iz, 103);
        f.put(
            Model::Hex,
            place(
                [x, y - 0.02, z],
                hash(ix, iz, 104),
                0.0,
                0.0,
                [0.08, h + 0.02, 0.08],
            ),
            [v, v * 0.95, v * 0.95],
        )
        .grounded();
    }
    // Stalactites hang from the tunnel's arch, some dripping molten rock.
    for (ix, iz, x, z) in cells(w, 0.2, 0.5, 12.0, 110) {
        let off = z - corridor(x);
        if off.abs() > TUNNEL * 0.75 || hash(ix, iz, 111) < 0.3 {
            continue;
        }
        let Some((c, _, share)) = w.seabed.tunnel_at(x) else {
            continue;
        };
        if share < 0.65 {
            continue;
        }
        f.fade = fade(share, 0.65);
        let a = (off / TUNNEL).clamp(-1.0, 1.0).acos();
        let top = c[1] + 0.85 * Seabed::tunnel_radius(x, a) * a.sin() + 0.08;
        let len = 0.05 + 0.32 * hash(ix, iz, 112).powf(1.5);
        if !w.visible([x, top - len * 0.5, z], len) {
            continue;
        }
        let r = 0.015 + len * 0.12;
        f.put(
            Model::Cone,
            between([x, top, z], [x, top - len - 0.08, z], r),
            [0.09, 0.08, 0.08],
        );
        if hash(ix, iz, 113) > 0.75 {
            let pulse = 0.7 + 0.3 * (w.t * 3.0 + ix as f32).sin();
            f.sprite(
                [x, top - len - 0.08, z],
                0.035,
                [1.0 * pulse, 0.45 * pulse, 0.10],
                GLOW,
            );
        }
    }
    // The pools that light the tunnel: a light over each of the nearest.
    for (ix, iz, x, z) in cells(w, 1.0, 0.5, 9.0, 120) {
        if (z - corridor(x)).abs() > TUNNEL * 0.8
            || w.share(x, z, CAVE) < 0.5
            || hash(ix, iz, 121) < 0.25
        {
            continue;
        }
        let y = w.ground(x, z);
        let flicker = 0.8 + 0.2 * (w.t * 1.9 + ix as f32 * 1.7).sin();
        f.light([x, y + 0.15, z], 1.6, [1.4 * flicker, 0.42 * flicker, 0.09]);
        for k in 0..4 {
            let rise = (w.t * 0.12 + hash(ix * 4 + k, iz, 122)).fract();
            let p = [
                x + (hash(ix, iz * 4 + k, 123) - 0.5) * 0.6,
                y + rise * 1.0,
                z + (hash(ix + k, iz, 124) - 0.5) * 0.6,
            ];
            let glow = (1.0 - rise) * smooth(0.0, 0.1, rise);
            f.sprite(p, 0.012, [1.0 * glow, 0.45 * glow, 0.1 * glow], EMBER);
        }
    }
}

/// The tunnel through the cave's massif: a ragged arch of rock inside, the
/// massif's dome over it outside, and at each mouth the cliff face with the
/// opening in it. Built for the stretch in view, from rings fixed in the
/// world, so it never shifts as the boat moves; and only when that stretch
/// gains or loses a ring, as nothing else about it changes.
fn tunnel(w: &World, f: &mut Frame) {
    use super::mesh::Vertex;
    const STEP: f32 = 0.2;
    const AROUND: usize = 20;
    let cam = w.camera();
    let first = ((cam - 16.0) / STEP).floor() as i32;
    let last = ((cam + 16.0) / STEP).ceil() as i32;
    let span = Some((first, last, w.seabed.seed.to_bits()));
    if f.tunnel_span == span {
        return;
    }
    f.tunnel_span = span;
    f.tunnel_version += 1;
    f.tunnel.clear();
    f.tunnel_indices.clear();
    let verts = &mut f.tunnel;
    let indices = &mut f.tunnel_indices;
    let vertex = |p: [f32; 3]| Vertex {
        pos: p,
        normal: [0.0; 3],
        colour: [1.0; 3],
        aux: [0.0; 2],
    };
    // Each ring: AROUND + 1 inner points then as many outer ones.
    let ring = |x: f32, c: [f32; 3], lat: [f32; 3], share: f32| -> Vec<[f32; 3]> {
        let mut points = Vec::with_capacity(2 * (AROUND + 1));
        for outer in [false, true] {
            for i in 0..=AROUND {
                let a = -0.2 * PI + i as f32 / AROUND as f32 * 1.4 * PI;
                let (across, up) = if outer {
                    (
                        TUNNEL * 1.9,
                        MASSIF * share * (0.85 + 0.3 * vnoise(x * 0.4, a, 5)),
                    )
                } else {
                    let r = Seabed::tunnel_radius(x, a);
                    (r, 0.85 * r)
                };
                points.push([
                    c[0] + lat[0] * across * a.cos(),
                    c[1] + up * a.sin(),
                    c[2] + lat[2] * across * a.cos(),
                ]);
            }
        }
        points
    };
    let n = AROUND as u32 + 1;
    let mut previous: Option<(u32, Vec<[f32; 3]>)> = None;
    let cap = |verts: &mut Vec<Vertex>, indices: &mut Vec<u32>, points: &[[f32; 3]]| {
        // A mouth: the cliff face between the arch and the dome.
        let base = verts.len() as u32;
        verts.extend(points.iter().map(|&p| vertex(p)));
        for i in 0..n - 1 {
            indices.extend_from_slice(&[
                base + i,
                base + i + 1,
                base + n + i + 1,
                base + i,
                base + n + i + 1,
                base + n + i,
            ]);
        }
    };
    for k in first..=last {
        let x = k as f32 * STEP;
        match w.seabed.tunnel_at(x) {
            Some((c, lat, share)) => {
                let points = ring(x, c, lat, share);
                let base = verts.len() as u32;
                verts.extend(points.iter().map(|&p| vertex(p)));
                match &previous {
                    Some((prev, _)) => {
                        for half in [0, n] {
                            for i in 0..n - 1 {
                                let (a, b) = (prev + half + i, prev + half + i + 1);
                                let (c, d) = (base + half + i, base + half + i + 1);
                                indices.extend_from_slice(&[a, c, b, b, c, d]);
                            }
                        }
                    }
                    None => cap(verts, indices, &points),
                }
                previous = Some((base, points));
            }
            None => {
                if let Some((_, points)) = previous.take() {
                    cap(verts, indices, &points);
                }
            }
        }
    }
    // Smooth normals, which the shader turns toward the viewer.
    let mut normals = vec![[0.0f32; 3]; verts.len()];
    for t in indices.as_chunks::<3>().0 {
        let [a, b, c] = [t[0] as usize, t[1] as usize, t[2] as usize];
        let (pa, pb, pc) = (verts[a].pos, verts[b].pos, verts[c].pos);
        let e1 = [pb[0] - pa[0], pb[1] - pa[1], pb[2] - pa[2]];
        let e2 = [pc[0] - pa[0], pc[1] - pa[1], pc[2] - pa[2]];
        let nn = cross(e1, e2);
        for i in [a, b, c] {
            for j in 0..3 {
                normals[i][j] += nn[j];
            }
        }
    }
    for (v, nn) in verts.iter_mut().zip(normals) {
        v.normal = norm(nn);
    }
}

/// Black smokers in the abyss, steaming vents in the lava cave.
fn vents(w: &World, f: &mut Frame) {
    for (ix, iz, x, z) in cells(w, 2.2, 1.0, 14.0, 130) {
        let b = w.biome(x, z);
        if (b.weight(ABYSS) + b.weight(CAVE)) < 0.5 || hash(ix, iz, 131) < 0.4 {
            continue;
        }
        f.fade = fade((b.weight(ABYSS) + b.weight(CAVE)) as f32, 0.5);
        let y = w.ground(x, z);
        let h = 0.25 + 0.25 * hash(ix, iz, 132);
        if !w.visible([x, y + 0.5, z], 0.9) {
            continue;
        }
        f.put(
            Model::Cone,
            place([x, y - 0.03, z], 0.0, 0.0, 0.0, [0.09, h, 0.09]),
            [0.10, 0.10, 0.10],
        )
        .grounded();
        let top = [x, y + h * 0.95, z];
        f.sprite(top, 0.08, [1.0, 0.44, 0.13], GLOW);
        f.light(top, 0.8, [0.9, 0.36, 0.10]);
        for k in 0..10 {
            let rise = (w.t * 0.25 + k as f32 / 10.0).fract();
            let wobble = (rise * 9.0 + w.t + ix as f32).sin() * 0.03 * (0.3 + rise * 2.0);
            let p = [top[0] + wobble, top[1] + rise * 0.8, top[2]];
            let fade = 1.0 - rise;
            f.sprite(
                p,
                0.03 + rise * 0.10,
                [0.12 * fade, 0.10 * fade, 0.09 * fade],
                GLOW,
            );
        }
    }
}

/// Keels of broken ice hanging from the pack.
fn ice(w: &World, f: &mut Frame) {
    let top = w.ice();
    if top <= w.eye() {
        return;
    }
    for (ix, iz, x, z) in cells(w, 1.8, 1.5, 16.0, 140) {
        if w.share(x, z, ARCTIC) < 0.65 || hash(ix, iz, 141) < 0.3 {
            continue;
        }
        f.fade = fade(w.share(x, z, ARCTIC), 0.65);
        let deep = 0.15 + 0.5 * hash(ix, iz, 142);
        let wide = 0.5 + 0.6 * hash(ix, iz, 143);
        if !w.visible([x, top - deep * 0.5, z], wide) {
            continue;
        }
        let v = 0.42 + 0.1 * hash(ix, iz, 144);
        // Daylight comes through the ice, so the keels glow rather than shade.
        f.put(
            Model::Keel,
            place(
                [x, top + 0.02, z],
                hash(ix, iz, 145) * TAU,
                0.0,
                0.0,
                [wide, deep, wide * 0.5],
            ),
            [v * 0.6, v * 1.1, v * 1.25],
        )
        .glowing(0.9);
    }
}

/// A party of divers in Rouquayrol-Denayrouze suits walking the seabed of the
/// forest and of Atlantis, the leader carrying a Ruhmkorff lamp.
fn divers(w: &World, f: &mut Frame) {
    const WALK: f32 = 0.02;
    let cam = w.camera();
    let u = cam - WALK * w.t;
    let k = (u / 7.0).floor();
    let lead = (k + 0.5) * 7.0 + WALK * w.t;
    if (lead - cam).abs() > 5.0 {
        return;
    }
    let side = side(k as i32, 150);
    let b = w.biome(lead, lane(lead, 1.5, side));
    let share = (b.weight(FOREST) + b.weight(ATLANTIS)) as f32;
    if share < 0.5 || hash(k as i32, 0, 150) < 0.2 {
        return;
    }
    f.fade = fade(share, 0.5);
    // The party walks a lane beside the corridor and steps round what stands
    // in it: each diver treads the same path as the one in front.
    let obstacles = obstacles(w, lead - 1.6, lead + 0.8, lane(lead, 1.5, side));
    let path = |x: f32| {
        let base = lane(x, 1.5, side);
        base + obstacles.iter().map(|&o| detour(o, x, base)).sum::<f32>()
    };
    for d in 0..3 {
        let fd = d as f32;
        let x = lead - fd * 0.30;
        let z = path(x);
        let heading = -((path(x + 0.04) - path(x - 0.04)) / 0.08).atan();
        let feet = [x, w.ground(x, z), z];
        let size = 0.17 * (1.0 - fd * 0.04);
        // Feet stay planted: a full stride covers twice a step of the legs.
        let stride = x / (size * 0.56) * TAU + fd * 1.9;
        let mark = f.mark();
        diver(f, feet, size, stride, d == 0, w.t);
        f.turn_since(&mark, feet, heading);
    }
}

/// What a diver must walk round near `lane` between `x0` and `x1`: boulders,
/// tree trunks and columns, as (x, z, radius). These are the same cells that
/// `rocks`, `forest` and `ruins` place.
fn obstacles(w: &World, x0: f32, x1: f32, lane: f32) -> Vec<(f32, f32, f32)> {
    let mut found = Vec::new();
    let mut scan =
        |size: f32, salt: u32, keep: &dyn Fn(i32, i32, f32, f32) -> Option<(f32, f32, f32)>| {
            for iz in ((lane - 1.2) / size).floor() as i32..=((lane + 1.2) / size).ceil() as i32 {
                for ix in ((x0 - 0.6) / size).floor() as i32..=((x1 + 0.6) / size).ceil() as i32 {
                    let (x, z) = cell_point(ix, iz, size, salt);
                    found.extend(keep(ix, iz, x, z));
                }
            }
        };
    scan(0.55, 10, &|ix, iz, x, z| {
        if hash(ix, iz, 12) < 0.55 {
            return None;
        }
        let size = 0.05 + 0.20 * hash(ix, iz, 13).powf(2.0);
        (size > 0.06 && !(size > 0.1 && (z - corridor(x)).abs() < 0.7)).then_some((
            x,
            z,
            size * 1.2,
        ))
    });
    scan(0.75, 30, &|ix, iz, x, z| {
        let grows = w.share(x, z, FOREST) >= hash(ix, iz, 31) * 0.7 + 0.2;
        (grows && (z - corridor(x)).abs() >= 0.6).then_some((x, z, 0.12))
    });
    // Columns stand at their cell's centre, in rows.
    scan(0.55, 70, &|ix, iz, _, _| {
        let (x, z) = ((ix as f32 + 0.5) * 0.55, (iz as f32 + 0.5) * 0.55);
        let stands = hash(0, iz, 71) >= 0.45
            && w.share(x, z, ATLANTIS) >= hash(ix, iz, 72) * 0.5 + 0.4
            && hash(ix, iz, 73) >= 0.15
            && (z - corridor(x)).abs() >= 0.5;
        stands.then_some((x, z, 0.13))
    });
    found
}

/// How far a path along `base` swings aside at `x` to pass obstacle `o`:
/// smoothly, starting a little before it and back after.
fn detour(o: (f32, f32, f32), x: f32, base: f32) -> f32 {
    let (ox, oz, r) = o;
    let reach = r + 0.4;
    let u = (x - ox) / reach;
    if u.abs() >= 1.0 {
        return 0.0;
    }
    let clearance = r + 0.18 - (base - oz).abs();
    if clearance <= 0.0 {
        return 0.0;
    }
    let away = if base >= oz { 1.0 } else { -1.0 };
    away * clearance * (0.5 + 0.5 * (PI * u).cos())
}

fn diver(f: &mut Frame, feet: [f32; 3], s: f32, stride: f32, lamp: bool, t: f32) {
    let at = |dx: f32, dy: f32, dz: f32| [feet[0] + dx * s, feet[1] + dy * s, feet[2] + dz * s];
    let canvas = [0.46, 0.40, 0.31];
    let bob = 0.012 * stride.cos().abs();
    let hip = at(0.0, 0.44 + bob, 0.0);
    for side in [-1.0f32, 1.0] {
        let ph = stride + if side > 0.0 { PI } else { 0.0 };
        let thigh = 0.34 * ph.sin();
        let knee = [
            hip[0] + thigh.sin() * 0.22 * s,
            hip[1] - thigh.cos() * 0.22 * s,
            hip[2] + side * 0.05 * s,
        ];
        let bend = 0.45 * (ph + 1.3).sin().max(0.0);
        let foot = [
            knee[0] + (thigh - bend).sin() * 0.2 * s,
            knee[1] - (thigh - bend).cos() * 0.2 * s,
            knee[2],
        ];
        f.put(
            Model::Capsule,
            between([hip[0], hip[1], knee[2]], knee, 0.05 * s),
            canvas,
        );
        f.put(Model::Capsule, between(knee, foot, 0.045 * s), canvas);
        f.put(
            Model::Box,
            place(
                [foot[0] + 0.03 * s, foot[1], foot[2]],
                0.0,
                0.0,
                0.0,
                [0.065 * s, 0.026 * s, 0.04 * s],
            ),
            [0.10, 0.10, 0.11],
        );
    }
    f.put(
        Model::Capsule,
        between(at(0.0, 0.42 + bob, 0.0), at(0.0, 0.78 + bob, 0.0), 0.10 * s),
        canvas,
    );
    f.put(
        Model::Cylinder,
        between(
            at(-0.13, 0.48 + bob, 0.0),
            at(-0.13, 0.76 + bob, 0.0),
            0.045 * s,
        ),
        [0.30, 0.31, 0.33],
    )
    .material(material::COPPER);
    // The copper helmet, its round faceplate and the corselet.
    let head = at(0.03, 0.87 + bob, 0.0);
    f.put(
        Model::Sphere,
        place(head, 0.0, 0.0, 0.0, [0.105 * s; 3]),
        [0.78, 0.42, 0.20],
    )
    .material(material::COPPER);
    f.put(
        Model::Sphere,
        place(
            at(0.12, 0.87 + bob, 0.0),
            0.0,
            0.0,
            0.0,
            [0.02 * s, 0.045 * s, 0.045 * s],
        ),
        [0.15, 0.25, 0.28],
    )
    .glowing(0.3);
    f.put(
        Model::Cylinder,
        between(
            at(0.01, 0.74 + bob, 0.0),
            at(0.01, 0.79 + bob, 0.0),
            0.13 * s,
        ),
        [0.62, 0.40, 0.22],
    )
    .material(material::COPPER);
    let shoulder = at(0.03, 0.71 + bob, 0.08);
    let swing = 0.07 * (stride + PI).sin();
    f.put(
        Model::Capsule,
        between(
            at(-0.01, 0.71 + bob, -0.08),
            at(-0.04 + swing, 0.46 + bob, -0.08),
            0.034 * s,
        ),
        canvas,
    );
    if lamp {
        let elbow = at(0.10, 0.57 + bob, 0.08);
        let hand = at(0.19, 0.58 + bob, 0.08);
        f.put(Model::Capsule, between(shoulder, elbow, 0.036 * s), canvas);
        f.put(Model::Capsule, between(elbow, hand, 0.032 * s), canvas);
        let light = at(0.24, 0.55 + bob, 0.08);
        f.put(
            Model::Box,
            place(light, 0.0, 0.0, 0.0, [0.03 * s, 0.04 * s, 0.03 * s]),
            [0.16, 0.15, 0.14],
        );
        f.sprite(light, 0.04, [1.0, 0.94, 0.78], GLOW);
        f.sprite(
            add(light, [0.06, -0.02, 0.0]),
            0.25,
            [0.12, 0.11, 0.09],
            GLOW,
        );
        f.light(add(light, [0.05, -0.02, 0.0]), 0.5, [1.4, 1.3, 1.05]);
    } else {
        let elbow = at(0.06, 0.55 + bob, 0.08);
        let hand = at(0.15, 0.55 + bob, 0.08);
        f.put(Model::Capsule, between(shoulder, elbow, 0.036 * s), canvas);
        f.put(Model::Capsule, between(elbow, hand, 0.032 * s), canvas);
        f.put(
            Model::Cylinder,
            between(
                at(-0.02, 0.50 + bob, 0.08),
                at(0.36, 0.63 + bob, 0.08),
                0.014 * s,
            ),
            [0.16, 0.15, 0.14],
        );
    }
    // A trail of bubbles from the helmet's exhaust.
    for b in 0..4 {
        let rise = (t * 0.35 + b as f32 / 4.0 + feet[0]).fract();
        f.sprite(
            add(head, [-0.02 - rise * 0.06, 0.1 * s + rise * 0.35, 0.0]),
            0.008 + rise * 0.006,
            [0.40, 0.62, 0.62],
            BUBBLE,
        );
    }
}

/// A species of schooling fish: where it lives, how big it is and how fast
/// it swims through the water (signed: negative swims against the voyage).
struct Species {
    z: f32,
    depth: f32,
    size: f32,
    speed: f32,
    band: (f32, f32),
    back: [f32; 3],
    gate: [f32; 7],
    salt: u32,
}

const SPECIES: [Species; 4] = [
    // Big fish high in open water, silversides mid-water, yellow grunts low
    // among the reefs and ruins, and fry right down by the seabed.
    Species {
        z: 5.0,
        depth: 2.5,
        size: 0.06,
        speed: 0.035,
        band: (0.6, 1.4),
        back: [0.22, 0.26, 0.28],
        gate: [1.0, 0.8, 1.0, 1.0, 0.0, 0.0, 0.5],
        salt: 200,
    },
    Species {
        z: 3.0,
        depth: 1.6,
        size: 0.032,
        speed: -0.045,
        band: (0.3, 0.9),
        back: [0.30, 0.36, 0.38],
        gate: [1.0, 1.0, 0.8, 0.6, 0.15, 0.0, 0.8],
        salt: 210,
    },
    Species {
        z: 2.0,
        depth: 1.0,
        size: 0.024,
        speed: 0.075,
        band: (0.10, 0.35),
        back: [0.48, 0.40, 0.20],
        gate: [1.0, 0.7, 0.3, 1.0, 0.0, 0.0, 0.0],
        salt: 220,
    },
    Species {
        z: 1.2,
        depth: 0.5,
        size: 0.016,
        speed: 0.030,
        band: (0.06, 0.25),
        back: [0.32, 0.42, 0.46],
        gate: [1.0, 1.0, 0.0, 0.5, 0.0, 0.0, 0.6],
        salt: 230,
    },
];

/// Schools: patchy, mostly small, now and then a great cloud; densest in the
/// core, thinning to stragglers; each fish beating its tail in step with its
/// speed, the school's heading rippling through it.
fn shoals(w: &World, f: &mut Frame) {
    let cam = w.camera();
    for sp in &SPECIES {
        let spacing = 3.0;
        let u = cam - sp.speed * w.t;
        // Far enough either way that no school comes or goes in view.
        let first = ((u - 10.0) / spacing).floor() as i32;
        let last = ((u + 10.0) / spacing).ceil() as i32;
        for k in first..=last {
            let centre_x =
                (k as f32 + 0.5 + (hash(k, 0, sp.salt) - 0.5) * 0.5) * spacing + sp.speed * w.t;
            let gathering = vnoise(centre_x * 0.25, 0.0, sp.salt + 1);
            let b = w.biome(centre_x, sp.z);
            let gate = World::by_biome(&b, sp.gate);
            if gate < 0.3 || gathering * 0.7 + hash(k, 1, sp.salt) * 0.5 < 0.5 {
                continue;
            }
            let big = hash(k, 2, sp.salt).powf(2.0);
            let rx = 0.08 + 0.75 * big;
            let ry = rx * (0.22 + 0.2 * hash(k, 3, sp.salt));
            let rz = rx * (0.4 + 0.3 * hash(k, 4, sp.salt));
            let cz = lane(
                centre_x,
                sp.z + (hash(k, 5, sp.salt) - 0.5) * sp.depth,
                side(k, sp.salt),
            );
            let floor = w.ground(centre_x, cz);
            let rise = sp.band.0 + (sp.band.1 - sp.band.0) * hash(k, 6, sp.salt);
            let cy = floor + rise.max(ry + 0.05) + 0.04 * (w.t * 0.09 + k as f32 * 2.0).sin();
            let centre = [centre_x, cy, cz];
            if !w.visible(centre, rx + 0.1) {
                continue;
            }
            // About one body length between neighbours, coarser in a big school
            // so it never runs to more than a few hundred fish.
            let length = sp.size * 2.0;
            let volume = rx * ry * rz * 4.2;
            let g = (length * 1.6).max((volume / 260.0).cbrt());
            let (nx, ny, nz) = (
                (rx / g).ceil() as i32,
                (ry / g).ceil() as i32,
                (rz / g).ceil() as i32,
            );
            let dir = if sp.speed >= 0.0 { 1.0 } else { -1.0 };
            let fade = smooth(0.3, 0.6, gate);
            for i in -nx..=nx {
                for j in -ny..=ny {
                    for l in -nz..=nz {
                        let r = hash(i * 31 + l, j * 17 + k, sp.salt + 7);
                        let jitter = [
                            (hash(i, j * 7 + l, sp.salt + 8) - 0.5) * 0.6,
                            (hash(i + l, j, sp.salt + 9) - 0.5) * 0.6,
                            (hash(i, j + l * 5, sp.salt + 10) - 0.5) * 0.6,
                        ];
                        let local = [
                            (i as f32 + jitter[0]) * g,
                            (j as f32 + jitter[1]) * g,
                            (l as f32 + jitter[2]) * g,
                        ];
                        let front = smooth(-rx, rx, local[0] * dir);
                        let e = ((local[0] / rx).powi(2)
                            + (local[1] / (ry * (0.75 + 0.35 * front))).powi(2)
                            + (local[2] / rz).powi(2))
                        .sqrt()
                            + (vnoise(
                                local[0] * 3.0 / rx,
                                local[2] * 3.0 / rx + k as f32,
                                sp.salt + 11,
                            ) - 0.5)
                                * 0.5;
                        let density = (0.05 + 0.90 * smooth(1.05, 0.40, e))
                            * if e < 1.6 { 1.0 } else { 0.0 }
                            * fade;
                        // Fish thin out at the school's edge by fading, not popping.
                        if r > density + 0.06 {
                            continue;
                        }
                        f.fade = smooth(0.06, -0.06, r - density);
                        let wob = [
                            (w.t * 0.7 + r * 20.0).sin() * 0.08 * g,
                            (w.t * 0.9 + r * 13.0).cos() * 0.05 * g,
                            (w.t * 0.6 + r * 7.0).sin() * 0.08 * g,
                        ];
                        // A bend in the school, and a wave of turning through it.
                        let bend = rx * 0.2 * (local[0] / rx * 2.0 + w.t * 0.25 + k as f32).sin();
                        let p = [
                            centre[0] + local[0] + wob[0],
                            centre[1] + local[1] + bend + wob[1],
                            centre[2] + local[2] + wob[2],
                        ];
                        let turn = 0.12 * (w.t * 0.6 - local[0] / rx * 4.0 + k as f32).sin()
                            + (r - 0.5) * 0.2;
                        let yaw = if dir > 0.0 { turn } else { PI + turn };
                        let fsize = sp.size * (0.85 + 0.3 * hash(i, j, sp.salt + 12));
                        let beat = sp.speed.abs() / (fsize * 1.4) * TAU * (0.85 + 0.3 * r);
                        let pitch = 0.08 * (w.t * 0.5 + r * 9.0).sin();
                        // A fish flashes silver as it turns into the light.
                        let flash = (w.t * 0.9 + r * 40.0 - local[0] / rx * 3.0)
                            .sin()
                            .max(0.0)
                            .powi(14);
                        f.put(Model::Fish, place(p, yaw, pitch, 0.0, [fsize; 3]), sp.back)
                            .moving(anim::SWIM, r * TAU, 0.22, beat)
                            .glowing(flash * 0.6);
                    }
                }
            }
        }
    }
}

/// A procession of creatures at `speed` through the water, one slot every
/// `spacing`: the slot centres around the camera.
fn procession(w: &World, speed: f32, spacing: f32, salt: u32) -> Vec<(i32, f32)> {
    // Far enough either way that nothing comes or goes in view.
    let reach = 10.0;
    let t = w.t;
    let u = w.camera() - speed * t;
    let first = ((u - reach) / spacing).floor() as i32;
    let last = ((u + reach) / spacing).ceil() as i32;
    (first..=last)
        .map(|k| {
            (
                k,
                (k as f32 + 0.5 + (hash(k, 0, salt) - 0.5) * 0.4) * spacing + speed * t,
            )
        })
        .collect()
}

/// Which side of the corridor procession slot `k` keeps to.
fn side(k: i32, salt: u32) -> f32 {
    if hash(k, 99, salt) < 0.5 { 1.0 } else { -1.0 }
}

/// A lane `offset` from the corridor on `side`.
fn lane(x: f32, offset: f32, side: f32) -> f32 {
    corridor(x) + side * offset
}

/// The right whale, the dolphins and the sharks.
fn great_swimmers(w: &World, f: &mut Frame) {
    // A right whale near the surface of the shallows, against the voyage.
    for (k, x) in procession(w, -0.035, 6.0, 300) {
        let z = lane(x, 4.5 + hash(k, 1, 300), side(k, 300));
        if w.share(x, z, SHALLOWS) < 0.4 || hash(k, 2, 300) < 0.3 {
            continue;
        }
        f.fade = fade(w.share(x, z, SHALLOWS), 0.4);
        let length = 1.35;
        let y = w.surface() - 0.35 - hash(k, 3, 300) * 0.25;
        if !w.visible([x, y, z], length) {
            continue;
        }
        let beat = 0.035 / (0.25 * length) * TAU;
        f.put(
            Model::Whale,
            place(
                [x, y, z],
                PI + 0.1 * (w.t * 0.1).sin(),
                0.02 * (w.t * 0.2).sin(),
                0.0,
                [length; 3],
            ),
            [0.075, 0.080, 0.088],
        )
        .moving(anim::FLUKE, k as f32, 0.035, beat);
    }
    // A pod of dolphins overtaking the boat.
    for (k, x) in procession(w, 0.13, 7.0, 310) {
        let pod = lane(x, 2.2, side(k, 310));
        if w.share(x, pod, SHALLOWS) < 0.4 || hash(k, 1, 310) < 0.3 {
            continue;
        }
        f.fade = fade(w.share(x, pod, SHALLOWS), 0.4);
        for d in 0..5 {
            let fd = d as f32;
            let phase = hash(k, d + 2, 310) * TAU;
            let size = 0.24 * (0.85 + 0.3 * hash(k, d + 10, 310));
            let p = [
                x + 0.5 - fd * 0.25 + 0.05 * (w.t * 0.3 + phase).sin(),
                w.eye() - 0.05 + hash(k, d + 20, 310) * 0.35 + 0.04 * (w.t * 0.6 + phase).sin(),
                pod + side(k, 310) * fd * 0.15,
            ];
            if !w.visible(p, size) {
                continue;
            }
            let beat = 0.13 / (0.9 * size) * TAU;
            f.put(
                Model::Dolphin,
                place(
                    p,
                    0.05 * (w.t * 0.4 + phase).sin(),
                    0.06 * (w.t * 0.6 + phase).cos(),
                    0.0,
                    [size; 3],
                ),
                [0.30, 0.35, 0.38],
            )
            .moving(anim::FLUKE, phase, 0.05, beat);
        }
    }
    // Sharks prowling the wrecks, against the voyage.
    for (k, x) in procession(w, -0.05, 2.4, 320) {
        let z = lane(x, 2.2 + hash(k, 1, 320) * 1.5, side(k, 320));
        if w.share(x, z, WRECKS) < 0.4 || hash(k, 2, 320) < 0.4 {
            continue;
        }
        f.fade = fade(w.share(x, z, WRECKS), 0.4);
        let length = 0.32;
        let y = w.ground(x, z) + 0.2 + hash(k, 3, 320) * 0.4;
        let p = [x, y + 0.03 * (w.t * 0.2 + k as f32).sin(), z];
        if !w.visible(p, length) {
            continue;
        }
        let beat = 0.05 / (0.35 * length) * TAU;
        f.put(
            Model::Shark,
            place(
                p,
                PI + 0.1 * (w.t * 0.3 + k as f32).sin(),
                0.0,
                0.0,
                [length * 0.5; 3],
            ),
            [0.30, 0.33, 0.35],
        )
        .moving(anim::SWIM, k as f32, 0.18, beat);
    }
}

/// The poulpe of chapter XVIII, met in the abyss.
fn squid(w: &World, f: &mut Frame) {
    for (k, x) in procession(w, -0.03, 4.5, 330) {
        let z = lane(x, 2.6 + hash(k, 1, 330), side(k, 330));
        if w.share(x, z, ABYSS) < 0.4 {
            continue;
        }
        f.fade = fade(w.share(x, z, ABYSS), 0.4);
        let s = 1.1;
        let y = w.eye() - 0.1 + 0.1 * (w.t * 0.13 + k as f32).sin();
        let head = [x, y, z];
        if !w.visible(head, s) {
            continue;
        }
        let jet = 0.5 + 0.5 * (w.t * 0.9).sin();
        let mottled = [0.40, 0.12, 0.08];
        // It swims mantle first, toward -x: the mantle runs from the head.
        f.put(
            Model::Mantle,
            place(head, PI, 0.0, 0.0, [s, s * (1.0 - 0.1 * jet), s]),
            mottled,
        );
        f.put(
            Model::Sphere,
            place(
                add(head, [0.07 * s, 0.0, 0.0]),
                0.0,
                0.0,
                0.0,
                [0.075 * s, 0.05 * s, 0.05 * s],
            ),
            mottled,
        );
        for side in [-1.0f32, 1.0] {
            let eye = add(head, [0.06 * s, 0.012 * s, side * 0.045 * s]);
            f.put(
                Model::Sphere,
                place(eye, 0.0, 0.0, 0.0, [0.026 * s; 3]),
                [0.70, 0.62, 0.40],
            )
            .glowing(0.15);
            f.put(
                Model::Sphere,
                place(
                    add(eye, [0.004 * s, 0.0, side * 0.018 * s]),
                    0.0,
                    0.0,
                    0.0,
                    [0.013 * s; 3],
                ),
                [0.01, 0.01, 0.015],
            );
        }
        for a in 0..10 {
            let fa = a as f32;
            let tentacle = a >= 8;
            let reach = if tentacle {
                0.70
            } else {
                0.24 + 0.16 * hash(a, k, 331)
            };
            let ang = fa / 10.0 * TAU;
            let spread = if tentacle {
                0.03
            } else {
                0.06 + 0.10 * (1.0 - jet)
            };
            let root = add(head, [0.11 * s, 0.0, 0.0]);
            let tip = add(
                root,
                [
                    reach * s,
                    ang.sin() * spread * s * 2.0,
                    ang.cos() * spread * s * 2.0,
                ],
            );
            let r = if tentacle { 0.006 * s } else { 0.014 * s };
            f.put(Model::Tube, between(root, tip, r), mottled).moving(
                anim::WRITHE,
                fa * 1.9,
                0.05 * s,
                1.6 + 0.3 * hash(a, 0, 332),
            );
        }
    }
    // Anglerfish, lit only by their lures.
    for (ix, iz, x, z) in cells(w, 1.4, 1.0, 6.0, 340) {
        if w.share(x, z, ABYSS) < 0.5 || hash(ix, iz, 341) < 0.5 {
            continue;
        }
        f.fade = fade(w.share(x, z, ABYSS), 0.5);
        let y =
            w.ground(x, z) + 0.15 + hash(ix, iz, 342) * 0.3 + 0.03 * (w.t * 1.3 + ix as f32).sin();
        let p = [x, y, z];
        if !w.visible(p, 0.1) {
            continue;
        }
        f.put(
            Model::Sphere,
            place(p, 0.0, 0.0, 0.0, [0.045, 0.035, 0.03]),
            [0.12, 0.10, 0.10],
        );
        let lure = add(p, [0.04, 0.03, 0.0]);
        let flicker = 0.75 + 0.25 * (w.t * 3.1 + ix as f32).sin() * (w.t * 1.7).sin();
        f.sprite(
            lure,
            0.012,
            [0.45 * flicker, 1.0 * flicker, 0.85 * flicker],
            GLOW,
        );
        f.sprite(
            lure,
            0.06,
            [0.05 * flicker, 0.12 * flicker, 0.10 * flicker],
            GLOW,
        );
        f.light(lure, 0.15, [0.3 * flicker, 0.8 * flicker, 0.7 * flicker]);
    }
}

/// Jellyfish drifting where the water carries them.
fn drifters(w: &World, f: &mut Frame) {
    for (ix, iz, x, z) in cells(w, 0.9, 1.0, 8.0, 350) {
        let b = w.biome(x, z);
        let here = (b.weight(ABYSS) + 0.6 * b.weight(WRECKS) + 0.5 * b.weight(ARCTIC)) as f32;
        if here < hash(ix, iz, 351) * 0.8 + 0.2 {
            continue;
        }
        f.fade = fade(here, hash(ix, iz, 351) * 0.8 + 0.2);
        let s = 0.04 + 0.05 * hash(ix, iz, 352);
        let phase = hash(ix, iz, 353) * TAU;
        let y = w.ground(x, z)
            + 0.3
            + hash(ix, iz, 354) * (0.3 * z + 0.3)
            + 0.04 * (w.t * 0.13 + phase).sin();
        let p = [x + 0.03 * (w.t * 0.09 + phase).sin(), y, z];
        if !w.visible(p, s * 3.0) {
            continue;
        }
        let tint = if hash(ix, iz, 355) < 0.5 {
            [0.35, 0.32, 0.80]
        } else {
            [0.70, 0.45, 0.80]
        };
        f.glass(Model::Bell, place(p, phase, 0.0, 0.0, [s, s, s]), tint)
            .moving(anim::PULSE, phase, 0.15, 1.35)
            .glowing(0.2);
        for a in 0..7 {
            let ang = a as f32 / 7.0 * TAU;
            let root = add(p, [ang.cos() * s * 0.8, 0.0, ang.sin() * s * 0.8]);
            let len = s * (2.5 + 1.5 * hash(ix * 7 + a, iz, 356));
            f.glass(
                Model::Tube,
                between(root, add(root, [0.0, -len, 0.0]), s * 0.03),
                [0.14, 0.55, 0.65],
            )
            .moving(anim::WRITHE, ang + phase, s * 0.4, 2.4);
        }
    }
}

/// Under the ice: penguins flying past, and narwhals.
fn polar(w: &World, f: &mut Frame) {
    let top = w.ice();
    for (k, x) in procession(w, 0.24, 3.2, 360) {
        let flight = lane(x, 1.7, side(k, 360));
        if w.share(x, flight, ARCTIC) < 0.4 || hash(k, 1, 360) < 0.25 {
            continue;
        }
        f.fade = fade(w.share(x, flight, ARCTIC), 0.4);
        for d in 0..4 {
            let fd = d as f32;
            let phase = hash(k, d + 2, 360) * TAU;
            let px = x + 0.4 - fd * 0.26;
            let wave = 0.06 * (px * 3.5 + phase).sin();
            let slope = 0.21 * (px * 3.5 + phase).cos();
            let p = [
                px,
                top - 0.25 - hash(k, d + 6, 360) * 0.45 + wave,
                flight + side(k, 360) * fd * 0.2,
            ];
            if !w.visible(p, 0.1) {
                continue;
            }
            let s = 0.075;
            let pitch = slope.atan();
            f.put(
                Model::Penguin,
                place(p, 0.0, pitch, 0.0, [s * 2.0; 3]),
                [0.12, 0.12, 0.13],
            );
            let flap = 0.9 * (w.t * TAU * 2.2 + phase).sin();
            for side in [-1.0f32, 1.0] {
                let root = add(p, [0.01, 0.0, side * 0.012]);
                f.put(
                    Model::Box,
                    place(
                        add(root, [-0.02, 0.0, side * 0.02]),
                        0.0,
                        pitch,
                        side * flap,
                        [0.03, 0.004, 0.022],
                    ),
                    [0.06, 0.07, 0.08],
                );
            }
            for b in 0..5 {
                let back = (b as f32 + (w.t * 0.24 / 0.05).fract()) * 0.05 + 0.04;
                f.sprite(
                    [p[0] - back, p[1] - slope * back, p[2]],
                    0.005 + 0.0015 * b as f32,
                    [0.45, 0.65, 0.70],
                    BUBBLE,
                );
            }
        }
    }
    for (k, x) in procession(w, 0.07, 6.0, 370) {
        let pod = lane(x, 2.9, side(k, 370));
        if w.share(x, pod, ARCTIC) < 0.4 || hash(k, 1, 370) < 0.3 {
            continue;
        }
        f.fade = fade(w.share(x, pod, ARCTIC), 0.4);
        for d in 0..3 {
            let fd = d as f32;
            let phase = hash(k, d + 2, 370) * TAU;
            let size = 0.42 * (0.85 + 0.3 * hash(k, d + 8, 370));
            let p = [
                x + 0.6 - fd * 0.55,
                top - 0.35 - hash(k, d + 12, 370) * 0.5 + 0.03 * (w.t * 0.3 + phase).sin(),
                pod + side(k, 370) * fd * 0.3,
            ];
            if !w.visible(p, size) {
                continue;
            }
            let beat = 0.07 / (0.5 * size) * TAU;
            f.put(
                Model::Narwhal,
                place(p, 0.05 * (w.t * 0.2 + phase).sin(), 0.0, 0.0, [size; 3]),
                [0.30, 0.32, 0.33],
            )
            .moving(anim::FLUKE, phase, 0.04, beat);
        }
    }
}

/// Marine snow, krill, plankton, embers and the bubbles shed by the hull.
fn particles(w: &World, f: &mut Frame) {
    let c = w.pose.pos;
    let b = w.biome(c[0], c[2]);
    let lava = (b.weight(ATLANTIS) + b.weight(CAVE)) as f32;
    let arctic = b.weight(ARCTIC) as f32;
    let abyss = b.weight(ABYSS) as f32;
    let eye = w.eye();
    // Snow hangs in a lattice fixed in the water, which sinks slowly as a
    // whole: no flake ever jumps, and the voyage sweeps the boat through it.
    let cell = 0.32;
    let sink = w.t * 0.006;
    let ([x0, x1, z0, z1], _, _) = footprint(w, 0.2, 5.0);
    for iz in (z0 / cell).floor() as i32..=(z1 / cell).ceil() as i32 {
        for ix in (x0 / cell).floor() as i32..=(x1 / cell).ceil() as i32 {
            for iy in ((eye - 2.4 + sink) / cell).floor() as i32
                ..=((eye + 2.4 + sink) / cell).ceil() as i32
            {
                let h = hash(ix, iy * 61 + iz, 400);
                if h < 0.55 {
                    continue;
                }
                let p = [
                    (ix as f32 + hash(iy, ix, 402)) * cell,
                    (iy as f32 + hash(ix + iz, iy, 403)) * cell - sink,
                    (iz as f32 + hash(ix, iz + iy, 404)) * cell,
                ];
                if !w.visible(p, 0.01) {
                    continue;
                }
                let warm = lava * 0.7;
                let tint = [0.34 + 0.66 * warm, 0.61 - 0.16 * warm, 0.63 - 0.48 * warm];
                f.sprite(
                    p,
                    0.004 + 0.003 * h,
                    [tint[0] * 0.35, tint[1] * 0.35, tint[2] * 0.35],
                    MOTE,
                );
                if arctic > 0.2 && h > 0.85 {
                    f.sprite(
                        add(p, [0.05, 0.03, 0.02]),
                        0.003,
                        [0.30 * arctic, 0.15 * arctic, 0.13 * arctic],
                        EMBER,
                    );
                }
                if abyss > 0.2 && h > 0.9 {
                    let blink = (w.t * 2.1 + h * 40.0).sin().max(0.0);
                    f.sprite(
                        add(p, [0.02, -0.04, 0.03]),
                        0.006,
                        [
                            0.05 * abyss * blink,
                            0.45 * abyss * blink,
                            0.35 * abyss * blink,
                        ],
                        GLOW,
                    );
                }
            }
        }
    }
    // Bubbles shed by the hull below the window: released into the water,
    // they rise into view from the bottom of the glass and drift aft, fading
    // in and out rather than appearing.
    let [right, up, out] = w.pose.basis;
    let v = w.pose.vel;
    let in_boat = |a: f32, b: f32, d: f32| {
        [
            c[0] + right[0] * a + up[0] * b + out[0] * d,
            c[1] + right[1] * a + up[1] * b + out[1] * d,
            c[2] + right[2] * a + up[2] * b + out[2] * d,
        ]
    };
    for k in 0..24 {
        let life = 7.0;
        let cycle = (w.t + k as f32 * life / 24.0) / life;
        let age = cycle.fract() * life;
        let n = cycle.floor() as i32;
        let d = 0.45 + hash(n, k, 411) * 0.5;
        let start = in_boat((hash(n, k, 410) - 0.4) * 1.2 * d, -VIEW * d - 0.06, d);
        let p = [
            start[0] - v[0] * age,
            start[1] + age * 0.06 - v[1] * age,
            start[2] - v[2] * age,
        ];
        let alpha = smooth(0.0, 0.8, age) * smooth(life, life - 1.5, age);
        if alpha > 0.0 && w.visible(p, 0.02) {
            f.sprite(
                p,
                0.004 + 0.006 * hash(k, 0, 412),
                [0.32 * alpha, 0.62 * alpha, 0.65 * alpha],
                BUBBLE,
            );
        }
    }
    // In a fast passage the hull throws off a rush of bubbles that streak past
    // the glass, and hides the sea rushing by behind them.
    let speed = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    let rush = smooth(0.08, 0.5, speed);
    if rush > 0.0 {
        let count = (rush * 220.0) as i32;
        for k in 0..count {
            let life = 0.9;
            let cycle = (w.t * 1.7 + k as f32 / count.max(1) as f32) / life;
            let age = cycle.fract() * life;
            let n = cycle.floor() as i32;
            let d = 0.25 + hash(n, k, 420) * 0.7;
            let start = in_boat(
                (hash(n, k, 421) - 0.3) * 1.6 * d,
                (hash(n, k, 422) - 0.5) * 1.6 * VIEW * d,
                d,
            );
            let p = [
                start[0] - v[0] * age,
                start[1] + age * 0.1,
                start[2] - v[2] * age,
            ];
            let alpha = rush * smooth(0.0, 0.15, age) * smooth(life, life * 0.5, age);
            f.streak(
                p,
                0.006 + 0.01 * hash(k, n, 423),
                [0.35 * alpha, 0.55 * alpha, 0.58 * alpha],
                1.0 + speed * 20.0,
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::voyage::Voyage;

    /// Mid-way through each leg of the voyage, its signature things are in view.
    #[test]
    fn every_biome_has_its_landmarks() {
        let landmarks = [
            (SHALLOWS, Model::Kelp),
            (FOREST, Model::Cone),
            (WRECKS, Model::Hull),
            (ATLANTIS, Model::Shaft),
            // The stalactites: the basalt columns stand low by the walls,
            // outside the round window from up the tunnel.
            (CAVE, Model::Cone),
            (ARCTIC, Model::Keel),
        ];
        for (leg, model) in landmarks {
            let mut found = 0;
            // A few moments through the leg, as any one could fall in a gap.
            for k in 0..6 {
                let seconds =
                    crate::voyage::LEG / crate::voyage::SPEED * (leg as f64 + 0.2 + 0.1 * k as f64);
                let mut voyage = Voyage::default();
                let position = voyage.advance(seconds, 0.0);
                let seabed = Seabed::new(3, 0.0);
                let x = position.x as f32;
                let mut pose = Pose::level(x, seabed.ground(x, 0.6) + 0.5);
                if leg == CAVE {
                    // In the cave the helm turns the window up the tunnel.
                    pose.pos[1] = seabed.tunnel_floor(x) + 0.4;
                    let (sin, cos) = super::super::helm::LOOK.sin_cos();
                    pose.basis = [[cos, 0.0, -sin], [0.0, 1.0, 0.0], [sin, 0.0, cos]];
                }
                let world = World {
                    seabed: &seabed,
                    pose,
                    t: seconds as f32,
                };
                let mut frame = Frame::new();
                build(&world, &mut frame);
                found += frame.solid[model.index()].len();
            }
            assert!(found > 0, "no {model:?} anywhere in leg {leg}");
        }
    }
}
