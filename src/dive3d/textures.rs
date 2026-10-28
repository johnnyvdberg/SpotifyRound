//! The surfaces of the 3D dive, painted when the scene is built.
//!
//! Each layer of one texture array is a tileable pattern made from wrapping
//! noise: grained rock, planking, veined marble, fish scales, wrinkled whale
//! skin, copper going green, canvas, coral, kelp, basalt, ice, sand, silt and
//! riveted iron plating.
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
}

impl Surface {
    pub const ALL: [Surface; 15] = [
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
        Surface::Kelp => {
            let rib = smooth(0.04, 0.0, (u - 0.5).abs());
            let veins = 0.9 + 0.1 * ((v * 60.0 + (u - 0.5).abs() * 20.0).sin());
            let spots = smooth(0.65, 0.8, fbm(u, v, 8, 3, 80));
            let c = grey(veins * (0.85 + 0.25 * fbm(u, v, 6, 3, 81)) * (1.0 - 0.3 * spots));
            mix(c, [1.2, 1.15, 0.7], rib * 0.5)
        }
        Surface::Basalt => {
            // Fine-grained dark rock, pocked with the vesicles of escaped gas.
            let vesicle = smooth(0.05, 0.02, cells(u, v, 48, 92));
            let streak = 0.9 + 0.1 * fbm(u * 0.2, v * 2.0, 16, 2, 91);
            grey((0.70 + 0.35 * fbm(u, v, 10, 4, 93)) * streak * (1.0 - 0.35 * vesicle))
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
    }
}

/// Every layer with its mip chain: `levels[mip][layer]` as RGBA8 texels.
pub fn paint_all() -> Vec<Vec<Vec<u8>>> {
    let base: Vec<Vec<[f32; 3]>> = Surface::ALL
        .iter()
        .map(|&surface| {
            (0..SIZE * SIZE)
                .map(|i| {
                    let (x, y) = (i % SIZE, i / SIZE);
                    paint(
                        surface,
                        (x as f32 + 0.5) / SIZE as f32,
                        (y as f32 + 0.5) / SIZE as f32,
                    )
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
                        .flat_map(|c| {
                            let q = |v: f32| (v.clamp(0.0, RANGE) / RANGE * 255.0).round() as u8;
                            [q(c[0]), q(c[1]), q(c[2]), 255]
                        })
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
                        let mut sum = [0.0f32; 3];
                        for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                            let c = layer[(y + dy) * size + x + dx];
                            for k in 0..3 {
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
