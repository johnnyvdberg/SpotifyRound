//! The seabed's relief: a random heightmap, made afresh for every visit.
//!
//! One tile, [`SIZE`] texels square and [`PERIOD`] world units across, wraps
//! in both directions. It mixes domain-warped rolling ground, sharp ridges and
//! the occasional canyon cut through them. The vertex shader lifts the seabed
//! grid by it and `world.rs` places things on it, both by the same bilinear
//! lookup of the same numbers, so everything stands on the seabed.
//!
//! Each texel holds the height, in -1..1, its slope along x and z, so the
//! seabed's normals interpolate smoothly instead of showing the texel grid, and
//! one more field of fine grain that each biome folds into its own floor: bank
//! tops in the shallows, root mounds in the forest, the abyss's cut rock.

use super::world::hash;

pub const SIZE: usize = 512;
/// World units the tile spans before it repeats: twenty minutes of voyage.
pub const PERIOD: f32 = 64.0;

/// Value noise on a lattice that wraps every `period` cells.
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

fn fbm(u: f32, v: f32, cells: i32, octaves: u32, salt: u32) -> f32 {
    let (mut sum, mut amp, mut norm) = (0.0, 0.5, 0.0);
    for o in 0..octaves {
        let scale = (1 << o) as f32;
        let period = cells << o;
        sum += amp
            * wrapped(
                u * cells as f32 * scale,
                v * cells as f32 * scale,
                period,
                salt + o,
            );
        norm += amp;
        amp *= 0.5;
    }
    sum / norm
}

/// The heightmap for `seed`, as RGBA texels: height, slope along x, slope
/// along z (both per world unit), and the floor's fine grain in -1..1.
pub fn generate(seed: u64) -> Vec<[f32; 4]> {
    let salt = (seed % 1_000_003) as u32 * 16;
    let mut height = vec![0.0f32; SIZE * SIZE];
    for j in 0..SIZE {
        for i in 0..SIZE {
            let (u, v) = (i as f32 / SIZE as f32, j as f32 / SIZE as f32);
            // Warp the domain, so the ground wanders instead of lying in a grid.
            let wu = u + 0.06 * (fbm(u, v, 4, 3, salt + 10) - 0.5);
            let wv = v + 0.06 * (fbm(u, v, 4, 3, salt + 20) - 0.5);
            let rolling = fbm(wu, wv, 6, 6, salt + 30);
            // Ridges: the creases of folded noise, sharpened.
            let fold = 1.0 - (2.0 * fbm(wu, wv, 5, 4, salt + 40) - 1.0).abs();
            let ridge = fold * fold * fold;
            // A canyon now and then, where a slow noise crosses its middle.
            let course = fbm(wu * 0.7, wv * 0.7, 3, 3, salt + 50);
            let canyon = 1.0 - ((course - 0.5).abs() / 0.045).min(1.0);
            let h = (rolling - 0.5) * 1.4 + ridge * 0.8 - canyon * canyon * 0.9;
            height[j * SIZE + i] = h;
        }
    }
    // Soften the finest grain a touch, as silt would.
    let mut smoothed = height.clone();
    for j in 0..SIZE {
        for i in 0..SIZE {
            let at = |di: i32, dj: i32| {
                let x = (i as i32 + di).rem_euclid(SIZE as i32) as usize;
                let y = (j as i32 + dj).rem_euclid(SIZE as i32) as usize;
                height[y * SIZE + x]
            };
            smoothed[j * SIZE + i] =
                (at(0, 0) * 4.0 + at(1, 0) + at(-1, 0) + at(0, 1) + at(0, -1)) / 8.0;
        }
    }
    let texel = PERIOD / SIZE as f32;
    let mut out = vec![[0.0f32; 4]; SIZE * SIZE];
    for j in 0..SIZE {
        for i in 0..SIZE {
            let at = |di: i32, dj: i32| {
                let x = (i as i32 + di).rem_euclid(SIZE as i32) as usize;
                let y = (j as i32 + dj).rem_euclid(SIZE as i32) as usize;
                smoothed[y * SIZE + x]
            };
            let gx = (at(1, 0) - at(-1, 0)) / (2.0 * texel);
            let gz = (at(0, 1) - at(0, -1)) / (2.0 * texel);
            // Fine grain, at one to four units a feature: fine enough to read
            // as silt and pitted rock underfoot, coarse enough that every
            // terrain level still samples it rather than aliasing it.
            let (u, v) = (i as f32 / SIZE as f32, j as f32 / SIZE as f32);
            let grain = 2.0 * fbm(u, v, 16, 3, salt + 60) - 1.0;
            out[j * SIZE + i] = [at(0, 0).clamp(-1.0, 1.0), gx, gz, grain];
        }
    }
    out
}

/// Height, slopes and grain at world (x, z), bilinear, exactly as `relief` in
/// the shader.
pub fn sample(map: &[[f32; 4]], x: f32, z: f32) -> [f32; 4] {
    let u = x / PERIOD * SIZE as f32;
    let v = z / PERIOD * SIZE as f32;
    let (fu, fv) = (u.floor(), v.floor());
    let (tu, tv) = (u - fu, v - fv);
    let n = SIZE as i32;
    let at = |i: i32, j: i32| map[(j.rem_euclid(n) * n + i.rem_euclid(n)) as usize];
    let (i, j) = (fu as i32, fv as i32);
    let (a, b, c, d) = (at(i, j), at(i + 1, j), at(i, j + 1), at(i + 1, j + 1));
    let mut out = [0.0; 4];
    for k in 0..4 {
        let top = a[k] + (b[k] - a[k]) * tu;
        let bottom = c[k] + (d[k] - c[k]) * tu;
        out[k] = top + (bottom - top) * tv;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_seabed_has_relief_and_wraps() {
        let map = generate(42);
        let heights: Vec<f32> = map.iter().map(|t| t[0]).collect();
        let lowest = heights.iter().cloned().fold(f32::MAX, f32::min);
        let highest = heights.iter().cloned().fold(f32::MIN, f32::max);
        assert!(highest - lowest > 0.8, "{lowest}..{highest} is too flat");
        // The tile repeats seamlessly, grain and all.
        let a = sample(&map, 3.3, 7.7);
        let b = sample(&map, 3.3 + PERIOD, 7.7 - PERIOD);
        for k in 0..4 {
            assert!((a[k] - b[k]).abs() < 1e-5);
        }
        // The grain varies over every texel, in -1..1.
        let grains: Vec<f32> = map.iter().map(|t| t[3]).collect();
        let low = grains.iter().cloned().fold(f32::MAX, f32::min);
        let high = grains.iter().cloned().fold(f32::MIN, f32::max);
        assert!(low > -1.0 && high < 1.0, "grain {low}..{high} out of range");
        assert!(high - low > 0.6, "grain {low}..{high} is too flat");
        // Another visit, another seabed.
        assert_ne!(generate(43)[1000][0], map[1000][0]);
        assert_ne!(generate(43)[1000][3], map[1000][3]);
    }
}
