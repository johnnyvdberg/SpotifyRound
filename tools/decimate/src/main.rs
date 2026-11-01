//! decimate IN OUT TRIANGLES UP FIT [TURN]
//!
//! Reads a binary STL or an OBJ (with its .mtl beside it), welds it,
//! simplifies it with meshoptimizer to at most TRIANGLES, and writes an OBJ
//! for assets/models: y up (UP names the source's up axis, 0 x, 1 y, 2 z),
//! the files' own handedness (the game flips z back), and
//!   FIT = height: a unit tall, standing on y = 0, centred on its footprint;
//!   FIT = length: a unit long along x, centred on its bounds.
//! TURN (degrees about y, after the axes are set) points an animal's nose
//! to +x. An OBJ's material colours come out as the vertices' shade, `v x y
//! z s`, scaled so the darkest material is 0.75 and the lightest 1.6, the
//! countershading the game's own swimmers use.
use std::collections::HashMap;

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let want: usize = a[3].parse().unwrap();
    let up: usize = a[4].parse().unwrap();
    let fit = a[5].as_str();
    let turn: f32 = a.get(6).map(|s| s.parse().unwrap()).unwrap_or(0.0);
    let (verts, shade, idx) = if a[1].ends_with(".obj") { obj(&a[1]) } else { stl(&a[1]) };
    let n = idx.len() / 3;
    let flat: Vec<f32> = verts.iter().flatten().copied().collect();
    let adapter = meshopt::VertexDataAdapter::new(bytes(&flat), 12, 0).unwrap();
    let mut err = 0.0;
    let out = if want < n {
        meshopt::simplify(&idx, &adapter, want * 3, 1.0, meshopt::SimplifyOptions::None, Some(&mut err))
    } else {
        idx.clone()
    };
    // The source's up to y, the axis after it to x and the one before to z.
    let (s1, s2) = ((up + 1) % 3, (up + 2) % 3);
    let (ts, tc) = turn.to_radians().sin_cos();
    let place = |p: [f32; 3]| {
        let q = [p[s1], p[up], p[s2]];
        [tc * q[0] - ts * q[2], q[1], ts * q[0] + tc * q[2]]
    };
    let mut lo = [f32::MAX; 3];
    let mut hi = [f32::MIN; 3];
    for &i in &out {
        let p = place(verts[i as usize]);
        for c in 0..3 {
            lo[c] = lo[c].min(p[c]);
            hi[c] = hi[c].max(p[c]);
        }
    }
    let (size, base) = match fit {
        "height" => (hi[1] - lo[1], [(lo[0] + hi[0]) / 2.0, lo[1], (lo[2] + hi[2]) / 2.0]),
        _ => (hi[0] - lo[0], [(lo[0] + hi[0]) / 2.0, (lo[1] + hi[1]) / 2.0, (lo[2] + hi[2]) / 2.0]),
    };
    let (dark, light) = shade.iter().fold((f32::MAX, f32::MIN), |(d, l), &s| (d.min(s), l.max(s)));
    let mut used: HashMap<u32, usize> = HashMap::new();
    let mut text = String::new();
    let mut faces = String::new();
    for tri in out.chunks(3) {
        let mut f = vec![];
        for &i in tri {
            let len = used.len();
            let j = *used.entry(i).or_insert_with(|| {
                let p = place(verts[i as usize]);
                let q: Vec<f32> = (0..3).map(|c| (p[c] - base[c]) / size).collect();
                text += &format!("v {:.4} {:.4} {:.4}", q[0], q[1], q[2]);
                if light > dark {
                    let s = 0.75 + 0.85 * (shade[i as usize] - dark) / (light - dark);
                    text += &format!(" {s:.3}");
                }
                text += "\n";
                len + 1
            });
            f.push(j);
        }
        faces += &format!("f {} {} {}\n", f[0], f[1], f[2]);
    }
    std::fs::write(&a[2], text + &faces).unwrap();
    eprintln!(
        "{}: {n} tris -> {} tris, {} verts, error {err:.4}",
        a[2],
        out.len() / 3,
        used.len()
    );
}

/// A binary STL, welded on exact positions.
fn stl(path: &str) -> (Vec<[f32; 3]>, Vec<f32>, Vec<u32>) {
    let b = std::fs::read(path).unwrap();
    let n = u32::from_le_bytes(b[80..84].try_into().unwrap()) as usize;
    let mut verts = vec![];
    let mut seen: HashMap<[u32; 3], u32> = HashMap::new();
    let mut idx = vec![];
    for t in 0..n {
        let o = 84 + t * 50 + 12;
        for k in 0..3 {
            let at = |c: usize| o + k * 12 + c * 4;
            let p: [f32; 3] = std::array::from_fn(|c| f32::from_le_bytes(b[at(c)..at(c) + 4].try_into().unwrap()));
            let i = *seen.entry(p.map(f32::to_bits)).or_insert_with(|| {
                verts.push(p);
                verts.len() as u32 - 1
            });
            idx.push(i);
        }
    }
    let shade = vec![1.0; verts.len()];
    (verts, shade, idx)
}

/// An OBJ, its faces fanned into triangles, each vertex shaded by the
/// luminance of its face's material (Kd in the .mtl beside it). A vertex
/// on two materials is split, so the seam stays sharp.
fn obj(path: &str) -> (Vec<[f32; 3]>, Vec<f32>, Vec<u32>) {
    let text = std::fs::read_to_string(path).unwrap();
    let mut kd: HashMap<String, f32> = HashMap::new();
    if let Ok(mtl) = std::fs::read_to_string(path.replace(".obj", ".mtl")) {
        let mut name = String::new();
        for line in mtl.lines() {
            let w: Vec<&str> = line.split_whitespace().collect();
            match w.first() {
                Some(&"newmtl") => name = w[1..].join(" "),
                Some(&"Kd") => {
                    let c: Vec<f32> = w[1..4].iter().map(|s| s.parse().unwrap()).collect();
                    kd.insert(name.clone(), 0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2]);
                }
                _ => {}
            }
        }
    }
    let mut positions = vec![];
    let mut verts = vec![];
    let mut shade = vec![];
    let mut split: HashMap<(usize, u32), u32> = HashMap::new();
    let mut idx = vec![];
    let mut current = 1.0f32;
    for line in text.lines() {
        let w: Vec<&str> = line.split_whitespace().collect();
        match w.first() {
            Some(&"v") => positions.push(std::array::from_fn(|c| w[1 + c].parse().unwrap())),
            Some(&"usemtl") => current = *kd.get(&w[1..].join(" ")).unwrap_or(&1.0),
            Some(&"f") => {
                let corner: Vec<u32> = w[1..]
                    .iter()
                    .map(|s| {
                        let p = s.split('/').next().unwrap().parse::<usize>().unwrap() - 1;
                        *split.entry((p, current.to_bits())).or_insert_with(|| {
                            verts.push(positions[p]);
                            shade.push(current);
                            verts.len() as u32 - 1
                        })
                    })
                    .collect();
                for k in 1..corner.len() - 1 {
                    idx.extend([corner[0], corner[k], corner[k + 1]]);
                }
            }
            _ => {}
        }
    }
    (verts, shade, idx)
}

fn bytes(f: &[f32]) -> &[u8] {
    unsafe { std::slice::from_raw_parts(f.as_ptr() as *const u8, f.len() * 4) }
}
