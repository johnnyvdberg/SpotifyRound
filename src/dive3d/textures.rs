//! The surfaces of the 3D dive, painted when the scene is built.
//!
//! Each layer of one texture array is a tileable pattern made from wrapping
//! noise: grained rock, planking, veined marble, fish scales, wrinkled whale
//! skin, copper going green, canvas, coral, kelp, basalt, ice, sand, silt and
//! riveted iron plating, and the meanders of a brain coral.
//! They are mostly tone, close to white on average, so a model's own colour
//! still decides its hue and the texture adds the detail. The shader projects
//! them along three axes, so no model needs texture coordinates.

use super::world::hash;

pub const SIZE: usize = 256;

/// The layers, in array order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Surface {
    Plain,
    Rock,
    Wood,
    Marble,
    Scales,
    Skin,
    Copper,
    Canvas,
    Coral,
    Kelp,
    Basalt,
    Ice,
    Sand,
    Silt,
    Plates,
    Brain,
    /// The lava cave's tunnel, which is not a colour at all: its rock's tone,
    /// the slopes of its relief and its molten cracks, one per channel
    /// ([`paint_vault`]), so the tunnel's single texture tap carries all
    /// three.
    Vault,
}

impl Surface {
    pub const ALL: [Surface; 17] = [
        Surface::Plain,
        Surface::Rock,
        Surface::Wood,
        Surface::Marble,
        Surface::Scales,
        Surface::Skin,
        Surface::Copper,
        Surface::Canvas,
        Surface::Coral,
        Surface::Kelp,
        Surface::Basalt,
        Surface::Ice,
        Surface::Sand,
        Surface::Silt,
        Surface::Plates,
        Surface::Brain,
        Surface::Vault,
    ];

    pub fn layer(self) -> f32 {
        self as usize as f32
    }
}

/// Value noise that wraps every `period` lattice cells.
fn wrapped(x: f32, y: f32, period: i32, salt: u32) -> f32 {
    let (fx, fy) = (x.floor(), y.floor());
    let (ix, iy) = (fx as i32, fy as i32);
    let (tx, ty) = (x - fx, y - fy);
    let (ux, uy) = (tx * tx * (3.0 - 2.0 * tx), ty * ty * (3.0 - 2.0 * ty));
    let at = |i: i32, j: i32| hash(i.rem_euclid(period), j.rem_euclid(period), salt);
    let top = at(ix, iy) + (at(ix + 1, iy) - at(ix, iy)) * ux;
    let bottom = at(ix, iy + 1) + (at(ix + 1, iy + 1) - at(ix, iy + 1)) * ux;
    top + (bottom - top) * uy
}

/// Fractal noise over the tile, `u` and `v` in 0..1, starting at `cells`.
fn fbm(u: f32, v: f32, cells: i32, octaves: u32, salt: u32) -> f32 {
    let (mut sum, mut amp, mut norm) = (0.0, 0.5, 0.0);
    for o in 0..octaves {
        let c = cells << o;
        sum += amp * wrapped(u * c as f32, v * c as f32, c, salt + o * 7);
        norm += amp;
        amp *= 0.5;
    }
    sum / norm
}

/// Distance to the nearest of a wrapped grid of jittered points: cells,
/// pores, scales and bubbles.
fn cells(u: f32, v: f32, n: i32, salt: u32) -> f32 {
    let (x, y) = (u * n as f32, v * n as f32);
    let (ix, iy) = (x.floor() as i32, y.floor() as i32);
    let mut best = 9.0f32;
    for dj in -1..=1 {
        for di in -1..=1 {
            let (ci, cj) = (ix + di, iy + dj);
            let px = ci as f32 + hash(ci.rem_euclid(n), cj.rem_euclid(n), salt);
            let py = cj as f32 + hash(ci.rem_euclid(n), cj.rem_euclid(n), salt + 1);
            best = best.min(((x - px).powi(2) + (y - py).powi(2)).sqrt());
        }
    }
    best
}

/// How near (u, v) is to the crack between two cells of a wrapped jittered
/// grid: 0 on the crack, larger inside a cell. The same cells as [`cells`],
/// so a texture can use both the pits and their edges.
fn cell_edge(u: f32, v: f32, n: i32, salt: u32) -> f32 {
    let (x, y) = (u * n as f32, v * n as f32);
    let (ix, iy) = (x.floor() as i32, y.floor() as i32);
    let (mut best, mut second) = (9.0f32, 9.0f32);
    for dj in -1..=1 {
        for di in -1..=1 {
            let (ci, cj) = (ix + di, iy + dj);
            let px = ci as f32 + hash(ci.rem_euclid(n), cj.rem_euclid(n), salt);
            let py = cj as f32 + hash(ci.rem_euclid(n), cj.rem_euclid(n), salt + 1);
            let d = ((x - px).powi(2) + (y - py).powi(2)).sqrt();
            if d < best {
                second = best;
                best = d;
            } else if d < second {
                second = d;
            }
        }
    }
    second - best
}

fn grey(v: f32) -> [f32; 3] {
    [v, v, v]
}

fn mix(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] {
    [
        a[0] + (b[0] - a[0]) * t,
        a[1] + (b[1] - a[1]) * t,
        a[2] + (b[2] - a[2]) * t,
    ]
}

fn smooth(a: f32, b: f32, v: f32) -> f32 {
    let t = ((v - a) / (b - a)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// One texel of `surface` at (u, v) in 0..1.
fn paint(surface: Surface, u: f32, v: f32) -> [f32; 3] {
    match surface {
        Surface::Plain => grey(1.0),
        Surface::Rock => {
            let grain = fbm(u, v, 8, 5, 1);
            let crack = smooth(0.0, 0.04, (fbm(u, v, 4, 3, 2) - 0.5).abs());
            let lichen = smooth(0.62, 0.70, fbm(u, v, 6, 3, 3));
            let base = grey(0.62 + 0.55 * grain).map(|c| c * (0.80 + 0.20 * crack));
            mix(base, [0.70, 0.85, 0.55], lichen * 0.35)
        }
        Surface::Wood => {
            // Planks across v, grain along u, dark seams and the odd nail.
            let plank = (v * 8.0).fract();
            let row = (v * 8.0).floor() as i32;
            let seam = smooth(0.0, 0.06, plank) * smooth(1.0, 0.94, plank);
            let grain = fbm(u * 0.25 + row as f32 * 0.37, v * 4.0, 8, 4, 10 + row as u32);
            let tone = 0.75 + 0.35 * grain + 0.1 * (hash(row, 0, 11) - 0.5);
            let nail = smooth(
                0.02,
                0.0,
                ((u * 4.0).fract() - 0.1).hypot(plank - 0.5) - 0.02,
            );
            let rot = smooth(0.6, 0.75, fbm(u, v, 4, 3, 12));
            let c = grey(tone * (0.3 + 0.7 * seam)).map(|c| c * (1.0 - 0.6 * nail));
            mix(
                [c[0], c[1] * 0.95, c[2] * 0.85],
                [0.45, 0.55, 0.35],
                rot * 0.4,
            )
        }
        Surface::Marble => {
            let swirl = fbm(u, v, 4, 5, 20);
            let vein = smooth(
                0.06,
                0.0,
                ((u * 4.0 + swirl * 6.0) * std::f32::consts::PI).sin().abs() * 0.5,
            );
            let stain = smooth(0.55, 0.8, fbm(u, v, 3, 4, 21));
            let c = grey(0.92 + 0.08 * swirl).map(|c| c * (1.0 - 0.35 * vein));
            mix(c, [0.62, 0.72, 0.55], stain * 0.45)
        }
        Surface::Scales => {
            // Overlapping scales in rows offset by half a scale.
            let (rows, across) = (16.0, 24.0);
            let row = (v * rows).floor();
            let su = (u * across + row * 0.5).fract();
            let sv = (v * rows).fract();
            let edge = ((su - 0.5) * 2.0).hypot(sv * 1.6);
            let rim = smooth(0.75, 1.0, edge);
            let sheen = 0.85 + 0.25 * fbm(u, v, 8, 3, 30);
            grey(sheen * (1.0 - 0.35 * rim))
        }
        Surface::Skin => {
            let wrinkle = fbm(u * 0.3, v * 3.0, 16, 3, 40);
            let mottle = smooth(0.4, 0.8, fbm(u, v, 6, 4, 41));
            let scar = smooth(0.012, 0.0, (fbm(u, v, 3, 3, 42) - 0.5).abs());
            grey(0.80 + 0.25 * wrinkle - 0.18 * mottle + 0.35 * scar)
        }
        Surface::Copper => {
            let brush = fbm(u * 0.1, v * 6.0, 32, 2, 50);
            let patina = smooth(0.55, 0.75, fbm(u, v, 5, 4, 51));
            let rivet = smooth(
                0.06,
                0.03,
                ((u * 6.0).fract() - 0.5).hypot((v * 6.0).fract() - 0.1),
            );
            let c = grey(0.85 + 0.2 * brush + 0.3 * rivet);
            mix(c, [0.45, 0.85, 0.70], patina * 0.55)
        }
        Surface::Canvas => {
            let weave = 0.85 + 0.15 * ((u * 160.0).sin() * (v * 160.0).sin());
            let stain = smooth(0.5, 0.8, fbm(u, v, 4, 4, 60));
            grey(weave * (1.0 - 0.3 * stain) * (0.9 + 0.15 * fbm(u, v, 16, 2, 61)))
        }
        Surface::Coral => {
            let pore = smooth(0.12, 0.30, cells(u, v, 24, 70));
            let blotch = fbm(u, v, 5, 4, 71);
            grey((0.55 + 0.45 * pore) * (0.8 + 0.35 * blotch))
        }
        Surface::Brain => {
            // A brain coral's meanders. Waves of one wavelength running every
            // which way, with random phases, add up to a field whose zero
            // line winds, forks and dead-ends at an even spacing: the
            // labyrinth of a brain coral's valleys, with rounded ridges
            // between. Every wave fits the tile a whole number of times, so
            // the tile joins itself.
            let tau = std::f32::consts::TAU;
            let (mut field, mut waves) = (0.0f32, 0.0f32);
            for b in 0..=5i32 {
                for a in -5..=5i32 {
                    let k = ((a * a + b * b) as f32).sqrt();
                    if !(4.5..=5.5).contains(&k) || (b == 0 && a < 0) {
                        continue;
                    }
                    let phase = hash(a, b, 151) * tau;
                    let weight = 0.6 + 0.8 * hash(a, b, 152);
                    field += weight * (tau * (a as f32 * u + b as f32 * v) + phase).cos();
                    waves += weight * weight;
                }
            }
            let field = field / (waves * 0.5).sqrt();
            // Dark, narrow valleys along the zero line, and the pale ridges
            // rounding up between them.
            let valley = smooth(0.42, 0.0, field.abs());
            let ridge = smooth(0.2, 1.2, field.abs());
            let grain = 0.92 + 0.12 * fbm(u, v, 32, 2, 153);
            let c = grey((0.78 + 0.40 * ridge) * (1.0 - 0.62 * valley) * grain);
            mix(c, [0.60, 0.66, 0.56], valley * 0.35)
        }
        Surface::Kelp => {
            let rib = smooth(0.04, 0.0, (u - 0.5).abs());
            let veins = 0.9 + 0.1 * ((v * 60.0 + (u - 0.5).abs() * 20.0).sin());
            let spots = smooth(0.65, 0.8, fbm(u, v, 8, 3, 80));
            let c = grey(veins * (0.85 + 0.25 * fbm(u, v, 6, 3, 81)) * (1.0 - 0.3 * spots));
            mix(c, [1.2, 1.15, 0.7], rib * 0.5)
        }
        Surface::Basalt => {
            // Basalt laid down flow on flow: pale ledges over darker
            // underhangs, finer laminae within a flow, cross-cut by cooling
            // cracks, pocked with the vesicles of escaped gas and stained by
            // minerals and soot. Every field wraps, so the tile is seamless
            // however the shader projects it.
            let flow = fbm(u, v * 2.0, 8, 4, 91);
            let band = 0.5 + 0.5 * (std::f32::consts::TAU * (v * 6.0 + flow * 2.0)).sin();
            let laminae = 0.5 + 0.5 * (std::f32::consts::TAU * (v * 19.0 + flow * 3.0)).sin();
            let grain = fbm(u, v, 24, 4, 93);
            let major = smooth(0.055, 0.0, cell_edge(u, v, 7, 94));
            let minor = smooth(0.040, 0.0, cell_edge(u, v, 13, 95));
            let fracture = smooth(0.012, 0.0, (fbm(u * 3.0, v, 9, 3, 96) - 0.5).abs());
            let crack = (0.55 * major).max(0.45 * minor).max(0.35 * fracture);
            let vesicle = smooth(0.06, 0.015, cells(u, v, 36, 92))
                .max(0.65 * smooth(0.045, 0.01, cells(u, v, 64, 96)));
            let stain = smooth(0.55, 0.82, fbm(u, v, 4, 3, 97));
            let mineral = smooth(0.62, 0.86, fbm(u * 2.0, v, 7, 3, 98));
            let ledge = smooth(0.55, 0.92, band);
            let hollow = smooth(0.45, 0.08, band);
            let mut tone = (0.85 + 0.55 * grain)
                * (0.95 + 0.10 * band)
                * (0.94 + 0.10 * laminae)
                * (0.94 + 0.14 * ledge)
                * (1.0 - 0.16 * hollow);
            tone *= 1.0 - 0.70 * crack;
            tone *= 1.0 - 0.45 * vesicle;
            tone *= 1.0 - 0.24 * stain;
            // Calcite crust in some seams, a cold damp stain in others.
            mix(
                mix(grey(tone), [1.10, 1.03, 0.87], mineral * 0.4),
                [0.72, 0.82, 0.72],
                stain * 0.3,
            )
        }
        Surface::Ice => {
            let bubble = smooth(0.08, 0.04, cells(u, v, 20, 100));
            let fracture = smooth(0.015, 0.0, (fbm(u, v, 3, 4, 101) - 0.5).abs());
            let c = grey(0.85 + 0.15 * fbm(u, v, 6, 3, 102));
            mix(c, [1.3, 1.4, 1.45], (bubble * 0.6 + fracture).min(1.0))
        }
        Surface::Sand => {
            let grain = hash((u * 256.0) as i32, (v * 256.0) as i32, 110);
            let ripple = 0.92 + 0.08 * ((u + v * 0.3 + fbm(u, v, 4, 3, 111) * 0.2) * 40.0).sin();
            let shell = smooth(0.94, 0.97, hash((u * 64.0) as i32, (v * 64.0) as i32, 112));
            grey((0.85 + 0.2 * grain) * ripple + 0.3 * shell)
        }
        Surface::Silt => {
            let soft = fbm(u, v, 6, 5, 120);
            let pebble = smooth(0.10, 0.06, cells(u, v, 18, 121));
            grey((0.78 + 0.35 * soft) * (1.0 - 0.25 * pebble))
        }
        Surface::Plates => {
            // Iron plates in courses, each course offset by half a plate,
            // lapped at dark seams and fastened by a line of rivets along
            // each edge; streaked with rust below the rivets.
            let (rows, across) = (4.0, 3.0);
            let row = (v * rows).floor();
            let pu = (u * across + row * 0.5).fract();
            let pv = (v * rows).fract();
            let plate = hash(
                ((u * across + row * 0.5).floor() as i32).rem_euclid(across as i32),
                row as i32,
                130,
            );
            let seam = smooth(0.0, 0.03, pu.min(1.0 - pu)) * smooth(0.0, 0.04, pv.min(1.0 - pv));
            // Lines of rivets just in from each plate's edges, eight texels
            // apart.
            let dot = |du: f32, dv: f32| smooth(2.2, 1.0, du.hypot(dv) * SIZE as f32);
            let spaced = |t: f32| ((t * 32.0).fract() - 0.5) / 32.0;
            let rows_v = (pv - 0.14).abs().min((pv - 0.86).abs()) / rows;
            let rows_u = (pu - 0.05).abs().min((pu - 0.95).abs()) / across;
            let rivets = dot(spaced(u), rows_v).max(dot(rows_u, spaced(v)));
            let grain = 0.88 + 0.2 * fbm(u, v, 16, 3, 131) + 0.12 * (plate - 0.5);
            let rust = smooth(0.55, 0.8, fbm(u, v, 4, 3, 132)) * (0.4 + 0.6 * pv);
            let c = grey(grain * (0.35 + 0.65 * seam) + 0.45 * rivets);
            mix(c, [0.85, 0.45, 0.25], rust * 0.35)
        }
        // Not a colour: painted whole, with its relief, by `paint_vault`.
        Surface::Vault => grey(1.0),
    }
}

/// How steep a slope of the tunnel's relief its channel holds, in heights
/// per tile: `SLOPE` is stored as 1, `-SLOPE` as 0, and anything steeper is
/// clipped. `VAULT_RELIEF` in `scene.wesl` reads them back.
pub const SLOPE: f32 = 10.0;

/// The tunnel's rock, as stored and not as seen: r its tone, as in any other
/// layer; g and b the slopes of its relief across the tile and up it; a the
/// cracks that run molten. The tunnel's walls fill the window from a hand's
/// breadth away, and one colour there reads as a smear; with the slopes the
/// shader lights a rough face, ledges and joints and all, from the same one
/// sample.
///
/// The rock is lava laid down flow on flow: each flow rises slowly to a sharp
/// lip, so the walls are ledged; pillowed and grained within a flow, broken
/// into blocks by cooling joints, pocked with vesicles, and here and there
/// glazed over by pale flowstone that has run down the face and smoothed the
/// cracks it crossed. Every field wraps, so the tile is seamless.
fn paint_vault() -> Vec<[f32; 4]> {
    use std::f32::consts::TAU;
    let n = SIZE;
    let mut height = vec![0.0f32; n * n];
    let mut texels = vec![[0.0f32; 4]; n * n];
    for (i, texel) in texels.iter_mut().enumerate() {
        let (u, v) = (
            ((i % n) as f32 + 0.5) / n as f32,
            ((i / n) as f32 + 0.5) / n as f32,
        );
        let flow = fbm(u, v, 4, 3, 140);
        // Two flows a tile, each a slow rise to a lip at its top.
        let rise = (v * 2.0 + 0.6 * flow).fract();
        let terrace = 0.8 * rise - 0.8 * smooth(0.84, 1.0, rise);
        let laminae = 0.03 * (TAU * (v * 9.0 + flow * 2.0)).sin();
        let pillows = fbm(u, v, 6, 3, 142);
        let grain = fbm(u, v, 16, 4, 141);
        let grit = fbm(u, v, 48, 2, 147);
        // The joints wander: the cells are bent by the flow, so the blocks
        // are not a mosaic of straight-edged tiles.
        let edge = cell_edge(
            u + 0.10 * (fbm(u, v, 5, 2, 148) - 0.5),
            v + 0.10 * (fbm(u, v, 5, 2, 149) - 0.5),
            6,
            143,
        );
        // Not every joint has opened: some blocks are still one with the
        // next, so the face is not a mosaic either.
        let opened = smooth(0.30, 0.55, fbm(u, v, 3, 2, 150));
        let edge = edge + 0.08 * (1.0 - opened);
        let joint = smooth(0.06, 0.0, edge);
        let fine = smooth(0.03, 0.0, cell_edge(u, v, 11, 144));
        let vesicle = smooth(0.045, 0.015, cells(u, v, 28, 145));
        // Flowstone: streaks six times as long as they are wide, down the face.
        let streak = smooth(0.60, 0.74, fbm(u * 6.0, v, 4, 3, 146));
        let body = 0.30 * terrace + 0.12 * pillows;
        let rough = body + 0.08 * grain + 0.04 * grit + laminae - 0.20 * joint - 0.08 * vesicle;
        height[i] = rough + (body + 0.04 - rough) * streak * 0.8;
        let mut tone = (0.74 + 0.55 * grain)
            * (0.90 + 0.20 * grit)
            * (0.88 + 0.20 * pillows)
            * (0.82 + 0.30 * rise)
            * (1.0 - 0.60 * joint)
            * (1.0 - 0.15 * fine)
            * (1.0 - 0.40 * vesicle);
        tone += (1.30 - tone) * streak * 0.55;
        let molten = smooth(0.035, 0.0, edge) * (1.0 - streak);
        *texel = [tone / RANGE, 0.0, 0.0, molten];
    }
    let at = |x: usize, y: usize| height[(y % n) * n + x % n];
    for (i, texel) in texels.iter_mut().enumerate() {
        let (x, y) = (i % n, i / n);
        let across = (at(x + 1, y) - at(x + n - 1, y)) * n as f32 * 0.5;
        let up = (at(x, y + 1) - at(x, y + n - 1)) * n as f32 * 0.5;
        texel[1] = 0.5 + 0.5 * (across / SLOPE).clamp(-1.0, 1.0);
        texel[2] = 0.5 + 0.5 * (up / SLOPE).clamp(-1.0, 1.0);
    }
    texels
}

/// Every layer with its mip chain: `levels[mip][layer]` as RGBA8 texels.
pub fn paint_all() -> Vec<Vec<Vec<u8>>> {
    // Every layer as stored, 0..1 a channel: colours scaled down by `RANGE`,
    // and an opaque alpha but for the tunnel's own.
    let base: Vec<Vec<[f32; 4]>> = Surface::ALL
        .iter()
        .map(|&surface| {
            if surface == Surface::Vault {
                return paint_vault();
            }
            (0..SIZE * SIZE)
                .map(|i| {
                    let (x, y) = (i % SIZE, i / SIZE);
                    let c = paint(
                        surface,
                        (x as f32 + 0.5) / SIZE as f32,
                        (y as f32 + 0.5) / SIZE as f32,
                    );
                    [c[0] / RANGE, c[1] / RANGE, c[2] / RANGE, 1.0]
                })
                .collect()
        })
        .collect();
    let mut levels = Vec::new();
    let mut current = base;
    let mut size = SIZE;
    loop {
        levels.push(
            current
                .iter()
                .map(|layer| {
                    layer
                        .iter()
                        .flat_map(|c| c.map(|v| (v.clamp(0.0, 1.0) * 255.0).round() as u8))
                        .collect()
                })
                .collect(),
        );
        if size == 1 {
            break;
        }
        let half = size / 2;
        current = current
            .iter()
            .map(|layer| {
                (0..half * half)
                    .map(|i| {
                        let (x, y) = (i % half * 2, i / half * 2);
                        let mut sum = [0.0f32; 4];
                        for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                            let c = layer[(y + dy) * size + x + dx];
                            for k in 0..4 {
                                sum[k] += c[k] * 0.25;
                            }
                        }
                        sum
                    })
                    .collect()
            })
            .collect();
        size = half;
    }
    levels
}

/// The texture array stores 0..1.6 as 0..255, so textures can brighten as well
/// as darken; the shader scales back by this.
pub const RANGE: f32 = 1.6;

#[cfg(test)]
mod tests {
    use super::*;

    /// The model shader gives the reef's two layers their glow by number.
    #[test]
    fn the_reef_layers_are_where_the_shader_looks() {
        assert_eq!(Surface::Coral.layer(), 8.0, "CORAL_LAYER in scene.wesl");
        assert_eq!(Surface::Brain.layer(), 15.0, "BRAIN_LAYER in scene.wesl");
    }

    /// The cave's cliffs name the tunnel's layer in the shader by number.
    #[test]
    fn the_shader_knows_the_vaults_layer() {
        let shader = include_str!("../shaders/scene.wesl");
        let line = format!("const VAULT_LAYER: i32 = {};", Surface::Vault as usize);
        assert!(shader.contains(&line), "scene.wesl wants `{line}`");
        assert!(
            shader.contains(&format!("const VAULT_RELIEF: f32 = 2.0 * {SLOPE:.1} *")),
            "VAULT_RELIEF reads the slopes back by `SLOPE`"
        );
    }

    /// The tunnel's relief is stored as slopes, clipped at `SLOPE`: if most
    /// of the tile were clipped, its ledges would light as one flat facet.
    #[test]
    fn the_vaults_slopes_mostly_fit() {
        let texels = paint_vault();
        let clipped = texels
            .iter()
            .filter(|t| t[1] <= 0.0 || t[1] >= 1.0 || t[2] <= 0.0 || t[2] >= 1.0)
            .count();
        assert!(
            clipped * 10 < texels.len(),
            "{clipped} of {} texels clipped",
            texels.len()
        );
    }

    /// The tunnel and the cave floor project this one layer in world space, so
    /// a seam in it would run across the rock wherever the tile repeats.
    #[test]
    fn the_basalt_tile_joins_itself() {
        let eps = 2e-3;
        for i in 0..SIZE {
            let t = (i as f32 + 0.5) / SIZE as f32;
            for (a, b) in [
                (
                    paint(Surface::Basalt, 0.0, t),
                    paint(Surface::Basalt, 1.0, t),
                ),
                (
                    paint(Surface::Basalt, t, 0.0),
                    paint(Surface::Basalt, t, 1.0),
                ),
            ] {
                for k in 0..3 {
                    assert!(
                        (a[k] - b[k]).abs() <= eps,
                        "a seam at {t}: {a:?} meets {b:?}"
                    );
                }
            }
        }
    }
}
