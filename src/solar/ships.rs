//! The battle's starships as real meshes, built procedurally and cached.
//!
//! Each fleet design (`voyage::FACTIONS`) is a distance field with
//! parameters of its own, drawn per ship from a seed, so no two variants of
//! a design are quite alike: a saucer cruiser's saucer size, its nacelles'
//! number, height and spread; a wedge's tiers, its deck's machinery and its
//! engine bank; a raider's wings raised to cruise or lowered to attack; and
//! so on. Each is meshed once by surface nets — a vertex in every cell of a
//! lattice the surface crosses, at the mean of the crossings on its edges,
//! then pulled onto the surface along the field's gradient — with the field's
//! gradient for normals, the field's distance a little way out for ambient
//! occlusion, and which part of the ship each vertex lies on (hull, windows,
//! nacelle glow, engines) for the shader to colour.
//!
//! A mesh takes a second or so to build (several on the Pi), so they are
//! built on a thread of their own and cached on disk, keyed by the
//! generator's version: once built, never again until the shapes change.
//! Until the library is ready, the battle traces the ships per pixel as it
//! did before (`hull_sdf` in `encounter.wgsl`).

use std::collections::HashMap;
use std::path::PathBuf;

use glam::Vec3;

use super::system::Rng;

/// Bumped whenever a design's shape changes, so stale caches are rebuilt.
pub const GENERATOR: u32 = 4;
/// Variants of each design and role.
pub const VARIANTS: u32 = 3;
/// Designs: as `voyage::FACTIONS`.
pub const DESIGNS: u32 = 6;
/// The extent meshed, about the ship's middle, in its own units (a ship
/// spans about -1..1 along its length).
const EXTENT: f32 = 1.2;

/// Which part of a ship a vertex lies on, for the shader's colours and
/// lights (`fs_ship`).
pub mod part {
    pub const HULL: u8 = 0;
    pub const ACCENT: u8 = 1;
    pub const DARK: u8 = 2;
    pub const NACELLE: u8 = 3;
    pub const BUSSARD: u8 = 4;
    pub const DEFLECTOR: u8 = 5;
    pub const ENGINE: u8 = 6;
    pub const WINDOWS: u8 = 7;
    pub const GREEN: u8 = 8;
}

/// A mesh vertex: where, its normal, and its occlusion, part and window
/// flag (unorm8 each).
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, bytemuck::Pod, bytemuck::Zeroable)]
pub struct ShipVertex {
    pub position: [f32; 3],
    pub normal: [f32; 3],
    pub extra: [u8; 4],
}

/// Which mesh: a design, a role (0 capital, 1 escort) and a variant.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Key {
    pub design: u32,
    pub role: u32,
    pub variant: u32,
}

impl Key {
    pub fn all() -> impl Iterator<Item = Key> {
        (0..DESIGNS).flat_map(|design| {
            (0..2).flat_map(move |role| {
                (0..VARIANTS).map(move |variant| Key {
                    design,
                    role,
                    variant,
                })
            })
        })
    }

    fn seed(self) -> u64 {
        0x5417_0000 ^ (self.design as u64) << 16 ^ (self.role as u64) << 8 ^ self.variant as u64
    }

    /// Lattice cells along each side: finer for capitals.
    fn resolution(self) -> usize {
        if self.role == 0 { 120 } else { 100 }
    }
}

pub struct Mesh {
    pub vertices: Vec<ShipVertex>,
    pub indices: Vec<u32>,
}

/// Every mesh, one after another: where each one's indices lie, and its
/// first vertex.
pub struct Library {
    pub vertices: Vec<ShipVertex>,
    pub indices: Vec<u32>,
    pub ranges: HashMap<Key, (u32, u32, i32)>,
}

impl Library {
    /// The mesh for ship `index` (0-7) of a battle seeded `seed`, of design
    /// `design`.
    pub fn for_ship(&self, design: u32, index: u32, seed: u32) -> Option<(u32, u32, i32)> {
        let role = if index.is_multiple_of(4) { 0 } else { 1 };
        let mut rng = Rng::new(((seed as u64) << 8) ^ index as u64 ^ 0x5a1);
        let variant = (rng.next() % VARIANTS as u64) as u32;
        self.ranges
            .get(&Key {
                design,
                role,
                variant,
            })
            .copied()
    }
}

/// Where the cache lives: `SOLAR_SHIP_CACHE`, or the user's cache directory.
fn cache_dir() -> Option<PathBuf> {
    if let Ok(dir) = std::env::var("SOLAR_SHIP_CACHE") {
        return Some(PathBuf::from(dir));
    }
    directories::ProjectDirs::from("", "", "spotify-round").map(|d| d.cache_dir().join("ships"))
}

fn cache_path(key: Key) -> Option<PathBuf> {
    cache_dir().map(|d| {
        d.join(format!(
            "g{GENERATOR}-d{}-r{}-v{}.mesh",
            key.design, key.role, key.variant
        ))
    })
}

const MAGIC: &[u8; 4] = b"SHP1";

fn load(key: Key) -> Option<Mesh> {
    let bytes = std::fs::read(cache_path(key)?).ok()?;
    if bytes.len() < 12 || &bytes[..4] != MAGIC {
        return None;
    }
    let vertices = u32::from_le_bytes(bytes[4..8].try_into().ok()?) as usize;
    let indices = u32::from_le_bytes(bytes[8..12].try_into().ok()?) as usize;
    let vsize = vertices * std::mem::size_of::<ShipVertex>();
    let isize = indices * 4;
    if bytes.len() != 12 + vsize + isize {
        return None;
    }
    let vertices: Vec<ShipVertex> = bytemuck::pod_collect_to_vec(&bytes[12..12 + vsize]);
    let indices: Vec<u32> = bytemuck::pod_collect_to_vec(&bytes[12 + vsize..]);
    Some(Mesh { vertices, indices })
}

fn store(key: Key, mesh: &Mesh) {
    let Some(path) = cache_path(key) else { return };
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let mut bytes = Vec::with_capacity(12 + mesh.vertices.len() * 28 + mesh.indices.len() * 4);
    bytes.extend_from_slice(MAGIC);
    bytes.extend_from_slice(&(mesh.vertices.len() as u32).to_le_bytes());
    bytes.extend_from_slice(&(mesh.indices.len() as u32).to_le_bytes());
    bytes.extend_from_slice(bytemuck::cast_slice(&mesh.vertices));
    bytes.extend_from_slice(bytemuck::cast_slice(&mesh.indices));
    // Written aside and moved into place, so a half-written file is never read.
    let partial = path.with_extension("part");
    if std::fs::write(&partial, &bytes).is_ok() {
        let _ = std::fs::rename(&partial, &path);
    }
}

/// The mesh for `key`: from the cache, or built (and cached).
pub fn mesh(key: Key) -> Mesh {
    if let Some(mesh) = load(key) {
        return mesh;
    }
    let mesh = build(key);
    store(key, &mesh);
    mesh
}

/// Older generations' meshes out of the cache, so it never grows past one
/// set.
fn prune() {
    let Some(dir) = cache_dir() else { return };
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return;
    };
    let current = format!("g{GENERATOR}-");
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.ends_with(".mesh") && !name.starts_with(&current) {
            let _ = std::fs::remove_file(entry.path());
        }
    }
}

/// Every mesh, loaded or built, into one library.
pub fn library() -> Library {
    prune();
    let mut lib = Library {
        vertices: Vec::new(),
        indices: Vec::new(),
        ranges: HashMap::new(),
    };
    for key in Key::all() {
        let m = mesh(key);
        let base = lib.vertices.len() as i32;
        let first = lib.indices.len() as u32;
        lib.indices.extend_from_slice(&m.indices);
        lib.vertices.extend_from_slice(&m.vertices);
        lib.ranges
            .insert(key, (first, m.indices.len() as u32, base));
    }
    lib
}

/// The library, built on a thread of its own: its receiver.
pub fn spawn_library() -> std::sync::mpsc::Receiver<Library> {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::Builder::new()
        .name("ship meshes".into())
        .spawn(move || {
            let _ = tx.send(library());
        })
        .expect("a thread for the ship meshes");
    rx
}

// ---------------------------------------------------------------------------
// The designs.
// ---------------------------------------------------------------------------

/// The field at a point: the distance, and the part nearest.
#[derive(Clone, Copy)]
struct Sample {
    d: f32,
    part: u8,
}

impl Sample {
    fn new(d: f32, part: u8) -> Self {
        Sample { d, part }
    }
    fn or(self, d: f32, part: u8) -> Self {
        if d < self.d { Sample { d, part } } else { self }
    }
    /// Cut away where `d` is inside, the cut's face of part `part`.
    fn cut(self, d: f32, part: u8) -> Self {
        if -d > self.d {
            Sample { d: -d, part }
        } else {
            self
        }
    }
}

fn hash(p: Vec3, seed: f32) -> f32 {
    let mut q = (p * Vec3::new(0.1031, 0.1030, 0.0973) + Vec3::splat(seed * 0.137)).fract();
    q += Vec3::splat(q.dot(Vec3::new(q.y, q.z, q.x) + Vec3::splat(33.33)));
    ((q.x + q.y) * q.z).fract()
}

fn sd_ellipsoid(p: Vec3, r: Vec3) -> f32 {
    let k0 = (p / r).length();
    let k1 = (p / (r * r)).length();
    k0 * (k0 - 1.0) / k1.max(1e-5)
}

fn sd_capsule(p: Vec3, a: Vec3, b: Vec3, r: f32) -> f32 {
    let pa = p - a;
    let ba = b - a;
    let h = (pa.dot(ba) / ba.dot(ba)).clamp(0.0, 1.0);
    (pa - ba * h).length() - r
}

fn sd_box(p: Vec3, b: Vec3) -> f32 {
    let q = p.abs() - b;
    q.max(Vec3::ZERO).length() + q.x.max(q.y.max(q.z)).min(0.0)
}

/// A cylinder along z, radius `r`, `h` either side of its middle.
fn sd_cylinder_z(p: Vec3, r: f32, h: f32) -> f32 {
    let d = glam::Vec2::new(glam::Vec2::new(p.x, p.y).length() - r, p.z.abs() - h);
    d.x.max(d.y).min(0.0) + d.max(glam::Vec2::ZERO).length()
}

/// A wedge's tier: its tip at `tip` ahead, `half_width` either side at the
/// stern (z = -1), its deck `top` high falling a little to its edges, its
/// floor at `floor`.
fn wedge_tier(p: Vec3, tip: f32, half_width: f32, top: f32, floor: f32) -> f32 {
    let m = glam::Vec2::new(p.x.abs(), p.z);
    let n = glam::Vec2::new(1.0 + tip, half_width).normalize();
    let edge = (m - glam::Vec2::new(0.0, tip)).dot(n);
    let back = -p.z - 1.0;
    let deck = p.y - (top - p.x.abs() * 0.18);
    edge.max(back).max(deck).max(floor - p.y)
}

/// A design's parameters for one variant, drawn from its seed.
struct Params {
    rng: [f32; 16],
    role: u32,
}

impl Params {
    fn new(key: Key) -> Self {
        let mut rng = Rng::new(key.seed());
        Params {
            rng: [(); 16].map(|_| rng.unit() as f32),
            role: key.role,
        }
    }
    /// The `i`th draw, between `a` and `b`.
    fn get(&self, i: usize, a: f32, b: f32) -> f32 {
        a + (b - a) * self.rng[i]
    }
    fn capital(&self) -> bool {
        self.role == 0
    }
}

/// The field of design `design` with parameters `k` at `p`.
fn field(design: u32, k: &Params, p: Vec3) -> Sample {
    match design {
        0 => saucer_cruiser(k, p),
        1 => wedge_destroyer(k, p),
        2 => raider(k, p),
        3 => warbird(k, p),
        4 => trader(k, p),
        _ => collective(k, p),
    }
}

/// Saucer cruisers: a saucer in decks with its bridge dome, impulse engines
/// astern; a neck down to the engineering hull, its deflector dish ahead and
/// shuttle bay astern; nacelles on swept pylons, two or four, high or low,
/// glowing along their inboard sides, bussard caps red at their bows.
fn saucer_cruiser(k: &Params, p: Vec3) -> Sample {
    let m = Vec3::new(p.x.abs(), p.y, p.z);
    let radius = k.get(0, 0.46, 0.58) * if k.capital() { 1.0 } else { 0.95 };
    let centre = Vec3::new(0.0, 0.06, 1.0 - radius - 0.04);
    let thick = k.get(1, 0.045, 0.065);
    let sc = p - centre;
    let mut s = Sample::new(
        sd_ellipsoid(sc, Vec3::new(radius, thick, radius)),
        part::HULL,
    );
    s = s.or(
        sd_ellipsoid(
            sc - Vec3::new(0.0, thick * 0.6, 0.0),
            Vec3::new(radius * 0.68, thick * 0.9, radius * 0.68),
        ),
        part::HULL,
    );
    s = s.or(
        sd_ellipsoid(
            sc - Vec3::new(0.0, -thick * 0.55, 0.0),
            Vec3::new(radius * 0.75, thick * 0.8, radius * 0.75),
        ),
        part::HULL,
    );
    s = s.or(
        sd_ellipsoid(
            sc - Vec3::new(0.0, thick * 1.65, 0.0),
            Vec3::new(0.08, 0.035, 0.08),
        ),
        part::WINDOWS,
    );
    // The saucer's rim: a band of windows.
    let rim = (glam::Vec2::new(sc.x, sc.z).length() - radius * 0.9).abs();
    if rim < 0.05 && sc.y.abs() < thick * 0.6 && s.d < 0.01 {
        s.part = part::WINDOWS;
    }
    // Impulse engines on the saucer's after edge.
    s = s.or(
        sd_box(
            sc - Vec3::new(0.0, thick * 0.2, -radius * 0.92),
            Vec3::new(0.11, 0.022, 0.03),
        ),
        part::BUSSARD,
    );
    // The neck and the engineering hull.
    let hull_len = k.get(2, 0.5, 0.72);
    let hull_r = k.get(3, 0.07, 0.1);
    let hull_front = centre.z - radius * 0.45;
    let hull_y = -0.15 - hull_r * 0.4;
    s = s.or(
        sd_box(
            p - Vec3::new(0.0, (hull_y + centre.y) * 0.5, hull_front - 0.08),
            Vec3::new(0.03, (centre.y - hull_y) * 0.5, 0.1),
        ),
        part::HULL,
    );
    let a = Vec3::new(0.0, hull_y, hull_front);
    let b = Vec3::new(0.0, hull_y + 0.03, hull_front - hull_len);
    s = s.or(sd_capsule(p, a, b, hull_r), part::ACCENT);
    s = s.or(
        sd_ellipsoid(
            p - (a + Vec3::new(0.0, 0.0, hull_r * 0.7)),
            Vec3::new(hull_r * 0.75, hull_r * 0.75, hull_r * 0.25),
        ),
        part::DEFLECTOR,
    );
    s = s.cut(
        sd_box(
            p - (b + Vec3::new(0.0, hull_r * 0.3, -hull_r * 0.6)),
            Vec3::new(hull_r * 0.5, hull_r * 0.35, hull_r * 0.8),
        ),
        part::DARK,
    );
    if (p.y - hull_y).abs() < hull_r * 0.25
        && p.z < hull_front
        && p.z > b.z
        && s.d < 0.01
        && s.part == part::ACCENT
    {
        s.part = part::WINDOWS;
    }
    // Nacelles: two, or four (a pair above and a pair below).
    let four = k.rng[4] > 0.7;
    let spread = k.get(5, 0.28, 0.4);
    let height = if k.rng[6] > 0.25 {
        k.get(7, 0.12, 0.24)
    } else {
        -k.get(7, 0.2, 0.3)
    };
    let length = k.get(8, 0.55, 0.8);
    let front = b.z + length * k.get(9, 0.55, 0.85);
    let pairs: &[f32] = if four { &[0.0, 1.0] } else { &[0.0] };
    for &which in pairs {
        let h = if which > 0.5 { -height * 0.9 } else { height };
        let x = if which > 0.5 { spread * 0.85 } else { spread };
        let na = Vec3::new(x, h, front);
        let nb = Vec3::new(x, h, front - length);
        let r = 0.048;
        s = s.or(sd_capsule(m, na, nb, r), part::HULL);
        // The inboard glow, and the bussard cap.
        let rel = m - na;
        if s.d < 0.01
            && rel.z < -0.06
            && rel.z > -length + 0.04
            && sd_capsule(m, na, nb, r + 0.01) < 0.01
            && (m.x - x) < -r * 0.3
        {
            s.part = part::NACELLE;
        }
        s = s.or(
            sd_ellipsoid(
                m - na - Vec3::new(0.0, 0.0, 0.01),
                Vec3::new(r * 1.05, r * 1.05, r * 1.3),
            ),
            part::BUSSARD,
        );
        // The pylon, swept back from the hull to the nacelle.
        let root = Vec3::new(0.02, hull_y + 0.02 * h.signum(), b.z + hull_len * 0.35);
        s = s.or(
            sd_capsule(m, root, Vec3::new(x, h, front - length * 0.45), 0.016),
            part::HULL,
        );
    }
    s
}

/// Wedge destroyers: a dagger in tiers, each deck crowded with machinery,
/// a spine along the top; the bridge tower astern with its two sensor
/// globes; a bank of engines across the stern.
fn wedge_destroyer(k: &Params, p: Vec3) -> Sample {
    let m = Vec3::new(p.x.abs(), p.y, p.z);
    let half_width = k.get(0, 0.52, 0.64) * if k.capital() { 1.0 } else { 0.8 };
    let tiers = if k.capital() {
        3 + (k.rng[1] * 1.99) as usize
    } else {
        1 + (k.rng[1] * 1.99) as usize
    };
    let mut d = wedge_tier(p, 1.0, half_width, 0.05 + 0.07 * (1.0 - p.z) * 0.5, -0.05);
    // Its belly: a shallower wedge below.
    d = d.min(wedge_tier(
        Vec3::new(p.x, -p.y, p.z),
        1.0,
        half_width * 0.92,
        0.035 + 0.04 * (1.0 - p.z) * 0.5,
        0.0,
    ));
    let mut s = Sample::new(d, part::HULL);
    let mut top = 0.05 + 0.07 * (1.0 - p.z) * 0.5;
    for t in 1..tiers {
        let ft = t as f32 / tiers as f32;
        let tip = 1.0 - ft * k.get(2, 1.1, 1.5);
        let width = half_width * (1.0 - ft * 0.62);
        let height = 0.1 + 0.07 * ft * k.get(3, 0.8, 1.2);
        s = s.or(wedge_tier(p, tip, width, height, 0.0), part::HULL);
        if p.z < tip && p.x.abs() < width && p.y > height - 0.06 {
            top = top.max(height - p.x.abs() * 0.18);
        }
    }
    // Machinery on the decks: blocks of their own heights, a cell each.
    let cell = 0.045;
    let c = (glam::Vec2::new(p.x, p.z) / cell).floor();
    let h = hash(Vec3::new(c.x, c.y, 3.0), k.rng[10] * 100.0);
    if h > 0.55 {
        let centre = (c + glam::Vec2::splat(0.5)) * cell;
        let size = 0.3 + 0.35 * hash(Vec3::new(c.x, c.y, 7.0), 1.0);
        let block = sd_box(
            p - Vec3::new(centre.x, top + (h - 0.55) * 0.03, centre.y),
            Vec3::new(
                cell * size,
                (h - 0.55) * 0.03 + 0.004,
                cell * size * (1.0 + h),
            ),
        );
        if wedge_tier(p, 1.0, half_width, 0.3, -1.0) < 0.0 {
            s = s.or(block, part::ACCENT);
        }
    }
    // The spine and the trench.
    s = s.or(
        sd_box(
            p - Vec3::new(0.0, top + 0.01, -0.1),
            Vec3::new(0.012, 0.018, 0.65),
        ),
        part::ACCENT,
    );
    // The tower: a neck, the bridge across it with windows, two globes.
    let tower = k.get(4, 0.85, 1.15) * if k.capital() { 1.0 } else { 0.7 };
    let ty = 0.2 * tower + 0.05;
    s = s.or(
        sd_box(
            p - Vec3::new(0.0, ty - 0.03, -0.82),
            Vec3::new(0.05, 0.08 * tower, 0.05),
        ),
        part::HULL,
    );
    s = s.or(
        sd_box(
            p - Vec3::new(0.0, ty + 0.06 * tower, -0.83),
            Vec3::new(0.19 * tower, 0.022, 0.04),
        ),
        part::WINDOWS,
    );
    s = s.or(
        (m - Vec3::new(0.15 * tower, ty + 0.105 * tower, -0.83)).length() - 0.026 * tower,
        part::HULL,
    );
    // Engines across the stern.
    let engines = 3 + (k.rng[5] * 2.99) as usize;
    for e in 0..engines {
        let f = if engines == 1 {
            0.0
        } else {
            e as f32 / (engines - 1) as f32 * 2.0 - 1.0
        };
        let r = if e == engines / 2 { 0.07 } else { 0.048 };
        let at = Vec3::new(f * half_width * 0.62, 0.05, -1.02);
        let bell = sd_cylinder_z(p - at, r, 0.04);
        s = s.or(bell, part::DARK);
        if (p.z - (at.z - 0.04)).abs() < 0.01
            && glam::Vec2::new(p.x - at.x, p.y - at.y).length() < r * 0.85
        {
            s.part = part::ENGINE;
        }
    }
    s.d *= 0.75;
    s
}

/// Winged raiders: an armoured head on a long neck, a body with its engine
/// block, wings raised to cruise or lowered to attack, their trailing edges
/// cut into feathers, cannons at their tips. The capitals heavier: a
/// shorter, thicker neck, broader wings set lower.
fn raider(k: &Params, p: Vec3) -> Sample {
    let m = Vec3::new(p.x.abs(), p.y, p.z);
    let heavy = k.capital();
    let neck_len = if heavy {
        k.get(0, 0.35, 0.5)
    } else {
        k.get(0, 0.5, 0.7)
    };
    let head_z = -0.05 + neck_len + 0.18;
    let mut s = Sample::new(
        sd_ellipsoid(p - Vec3::new(0.0, 0.0, head_z), Vec3::new(0.11, 0.07, 0.17)),
        part::HULL,
    );
    s = s.or(
        sd_ellipsoid(
            p - Vec3::new(0.0, 0.05, head_z - 0.04),
            Vec3::new(0.07, 0.03, 0.1),
        ),
        part::ACCENT,
    );
    if p.z > head_z + 0.05 && p.y.abs() < 0.02 && s.d < 0.01 {
        s.part = part::WINDOWS;
    }
    s = s.or(
        sd_capsule(
            p,
            Vec3::new(0.0, 0.03, head_z - 0.12),
            Vec3::new(0.0, 0.06, 0.05),
            if heavy { 0.06 } else { 0.04 },
        ),
        part::HULL,
    );
    s = s.or(
        sd_ellipsoid(p - Vec3::new(0.0, 0.05, -0.15), Vec3::new(0.25, 0.09, 0.32)),
        part::HULL,
    );
    s = s.or(
        sd_box(p - Vec3::new(0.0, 0.03, -0.42), Vec3::new(0.12, 0.05, 0.08)),
        part::DARK,
    );
    if p.z < -0.49 && p.x.abs() < 0.1 && s.d < 0.01 {
        s.part = part::ENGINE;
    }
    // Wings: raised (cruise) or lowered (attack).
    let angle = k.get(1, -0.15, 0.55) * if heavy { 0.5 } else { 1.0 };
    let span = k.get(2, 0.75, 0.95) * if heavy { 1.15 } else { 1.0 };
    let w = m - Vec3::new(0.12, 0.0, -0.18);
    let (sn, c) = angle.sin_cos();
    let along = w.x * c + w.y * sn;
    let across = -w.x * sn + w.y * c;
    let chord = 0.22 - along * 0.12;
    let mut wing = sd_box(
        Vec3::new(along - span * 0.5, across, w.z + along * 0.12),
        Vec3::new(span * 0.5, 0.016, chord.max(0.05)),
    );
    // Feathers along the trailing edge.
    let feather = (along * 9.0).fract() - 0.5;
    let trailing = -(w.z + along * 0.12) - chord + 0.08;
    wing = wing.max(-(feather.abs() - 0.07).max(-trailing));
    s = s.or(wing, part::HULL);
    s = s.or(
        sd_ellipsoid(w - Vec3::new(0.08, 0.0, 0.0), Vec3::new(0.1, 0.05, 0.16)),
        part::ACCENT,
    );
    let tip = Vec3::new(0.12 + span * c, -span * sn, -0.08);
    s = s.or(
        sd_capsule(
            m,
            tip + Vec3::new(0.0, 0.0, 0.16),
            tip - Vec3::new(0.0, 0.0, 0.22),
            0.024,
        ),
        part::DARK,
    );
    if (m - (tip + Vec3::new(0.0, 0.0, 0.16))).length() < 0.03 {
        s.part = part::GREEN;
    }
    s
}

/// Crescent warbirds: from the side, two great wings arching above and
/// below with open space between, broadest at their middles and narrowing
/// fore and aft, thin at their edges; a long neck to the command section
/// ahead; nacelles under the lower wing's tips.
fn warbird(k: &Params, p: Vec3) -> Sample {
    let m = Vec3::new(p.x.abs(), p.y, p.z);
    let span = k.get(0, 0.62, 0.78);
    let tall = k.get(1, 1.0, 1.3);
    let q = p - Vec3::new(0.0, 0.0, -0.25);
    let ang = (q.y * 1.15 * tall).atan2(q.z * 0.85);
    let around = glam::Vec2::new(q.y * 1.15 * tall, q.z * 0.85).length() - 0.52;
    let width = span * (0.25 + 0.75 * ang.sin().abs());
    let thin = 0.034 * (1.0 - 0.7 * (q.x / width).clamp(-1.0, 1.0).powi(2));
    let w2 = glam::Vec2::new(around.abs() - thin, q.x.abs() - width);
    let wings = (w2.max(glam::Vec2::ZERO).length() + w2.x.max(w2.y).min(0.0)) * 0.7;
    let mut s = Sample::new(wings, part::HULL);
    let neck = k.get(2, 0.75, 0.88);
    s = s.or(
        sd_capsule(
            p,
            Vec3::new(0.0, 0.0, 0.2),
            Vec3::new(0.0, -0.01, neck),
            0.045,
        ),
        part::HULL,
    );
    s = s.or(
        sd_ellipsoid(
            p - Vec3::new(0.0, -0.01, neck + 0.06),
            Vec3::new(0.15, 0.075, 0.12),
        ),
        part::ACCENT,
    );
    s = s.or(
        sd_ellipsoid(
            p - Vec3::new(0.0, 0.05, neck + 0.06),
            Vec3::new(0.06, 0.03, 0.05),
        ),
        part::WINDOWS,
    );
    if p.z > neck && (p.y + 0.01).abs() < 0.02 && s.d < 0.01 {
        s.part = part::WINDOWS;
    }
    let nacelle_x = width.min(span) * 0.75;
    let na = Vec3::new(nacelle_x, -0.47 / tall, -0.1);
    let nb = Vec3::new(nacelle_x, -0.47 / tall, -0.55);
    s = s.or(sd_capsule(m, na, nb, 0.035), part::DARK);
    // Only the nacelles' own sterns glow.
    if (m - nb).length() < 0.045 && p.z < nb.z {
        s.part = part::GREEN;
    }
    s
}

/// Horseshoe traders: a broad horseshoe in two or three decks, open ahead,
/// its prongs ending in claws; a raised bridge astern with engines behind.
fn trader(k: &Params, p: Vec3) -> Sample {
    let q = p - Vec3::new(0.0, 0.0, -0.05);
    let opening = k.get(0, 0.2, 0.28);
    let open = sd_box(q - Vec3::new(0.0, 0.0, 0.6), Vec3::new(opening, 0.3, 0.32));
    let decks = if k.rng[1] > 0.5 { 3 } else { 2 };
    let radial = glam::Vec2::new(q.x, q.z).length();
    let mut s = Sample::new(1e9, part::HULL);
    for deck in 0..decks {
        let f = deck as f32;
        let r = 0.52 - f * 0.06;
        let half = 0.25 - f * 0.07;
        let y = f * 0.07;
        let h = 0.075 - f * 0.02;
        let d = (glam::Vec2::new((radial - r) / half, (q.y - y) / h).length() - 1.0) * h;
        s = s.or(
            d.max(-open - f * 0.04),
            if deck == 0 { part::HULL } else { part::ACCENT },
        );
    }
    if (radial - 0.75).abs() < 0.04 && q.y.abs() < 0.03 && s.d < 0.01 {
        s.part = part::WINDOWS;
    }
    let claw = k.get(2, 0.85, 1.25);
    s = s.or(
        sd_ellipsoid(
            Vec3::new(q.x.abs(), q.y, q.z) - Vec3::new(0.4, -0.01, 0.42),
            Vec3::new(0.07, 0.05, 0.12) * claw,
        ),
        part::ACCENT,
    );
    if (Vec3::new(q.x.abs(), q.y, q.z) - Vec3::new(0.4, -0.01, 0.54 * claw)).length() < 0.04 {
        s.part = part::GREEN;
    }
    s = s.or(
        sd_ellipsoid(
            q - Vec3::new(0.0, 0.12 + (decks as f32 - 2.0) * 0.05, -0.42),
            Vec3::new(0.2, 0.07, 0.14),
        ),
        part::WINDOWS,
    );
    let engines = sd_cylinder_z(
        Vec3::new(q.x.abs(), q.y, q.z) - Vec3::new(0.14, 0.02, -0.66),
        0.05,
        0.05,
    );
    s = s.or(engines, part::DARK);
    if q.z < -0.7 && s.d < 0.01 && s.part == part::DARK {
        s.part = part::ENGINE;
    }
    s
}

/// The collective: capitals cubes, escorts spheres or smaller cubes, each
/// face built up of blocks of machinery standing out and sinking in, at two
/// scales, green light deep in the gaps.
fn collective(k: &Params, p: Vec3) -> Sample {
    let sphere = !k.capital() && k.rng[0] > 0.4;
    let base = if sphere {
        p.length() - 0.7
    } else {
        sd_box(p, Vec3::splat(0.62))
    };
    let mut relief = 0.0;
    let mut deep = false;
    for (scale, height) in [(0.11, 0.03), (0.04, 0.012)] {
        let c = (p / scale).floor();
        let h = hash(c, k.rng[1] * 50.0 + scale);
        relief += (h - 0.45) * height;
        if h < 0.05 {
            deep = true;
        }
    }
    let d = base - relief;
    let mut s = Sample::new(d, if deep { part::GREEN } else { part::HULL });
    if !deep && hash((p / 0.025).floor(), 3.0) > 0.93 {
        s.part = part::DARK;
    }
    s.d *= 0.8;
    s
}

// ---------------------------------------------------------------------------
// Meshing.
// ---------------------------------------------------------------------------

/// The mesh of `key`, by surface nets over a lattice of its field.
pub fn build(key: Key) -> Mesh {
    let params = Params::new(key);
    let n = key.resolution();
    let step = 2.0 * EXTENT / n as f32;
    let at = |i: usize, j: usize, k: usize| {
        Vec3::new(i as f32, j as f32, k as f32) * step - Vec3::splat(EXTENT)
    };
    let f = |p: Vec3| field(key.design, &params, p);
    let side = n + 1;
    // The field at every lattice point, a slab of z per thread; whole blocks
    // far from the surface filled from their middle alone.
    let mut values = vec![0.0f32; side * side * side];
    let block = 6;
    let threads = std::thread::available_parallelism()
        .map_or(2, |n| n.get())
        .clamp(1, 8);
    let chunk = side.div_ceil(threads);
    std::thread::scope(|scope| {
        for (t, slab) in values.chunks_mut(chunk * side * side).enumerate() {
            let f = &f;
            scope.spawn(move || {
                let k0 = t * chunk;
                let depth = slab.len() / (side * side);
                let mut kb = 0;
                while kb < depth {
                    for jb in (0..side).step_by(block) {
                        for ib in (0..side).step_by(block) {
                            let middle = at(ib + block / 2, jb + block / 2, k0 + kb + block / 2);
                            let d = f(middle).d;
                            let far = d.abs() > step * block as f32 * 1.6;
                            for kk in kb..(kb + block).min(depth) {
                                for jj in jb..(jb + block).min(side) {
                                    for ii in ib..(ib + block).min(side) {
                                        slab[(kk * side + jj) * side + ii] =
                                            if far { d } else { f(at(ii, jj, k0 + kk)).d };
                                    }
                                }
                            }
                        }
                    }
                    kb += block;
                }
            });
        }
    });
    let value = |i: usize, j: usize, k: usize| values[(k * side + j) * side + i];
    // A vertex in each cell the surface crosses.
    let mut cell_vertex = vec![u32::MAX; n * n * n];
    let mut positions: Vec<Vec3> = Vec::new();
    const CORNERS: [[usize; 3]; 8] = [
        [0, 0, 0],
        [1, 0, 0],
        [0, 1, 0],
        [1, 1, 0],
        [0, 0, 1],
        [1, 0, 1],
        [0, 1, 1],
        [1, 1, 1],
    ];
    const EDGES: [[usize; 2]; 12] = [
        [0, 1],
        [2, 3],
        [4, 5],
        [6, 7],
        [0, 2],
        [1, 3],
        [4, 6],
        [5, 7],
        [0, 4],
        [1, 5],
        [2, 6],
        [3, 7],
    ];
    for k in 0..n {
        for j in 0..n {
            for i in 0..n {
                let v: [f32; 8] = CORNERS.map(|c| value(i + c[0], j + c[1], k + c[2]));
                let inside = v.iter().filter(|&&x| x < 0.0).count();
                if inside == 0 || inside == 8 {
                    continue;
                }
                let mut sum = Vec3::ZERO;
                let mut count = 0.0;
                for e in EDGES {
                    let (a, b) = (v[e[0]], v[e[1]]);
                    if (a < 0.0) != (b < 0.0) {
                        let t = a / (a - b);
                        let pa = Vec3::new(
                            CORNERS[e[0]][0] as f32,
                            CORNERS[e[0]][1] as f32,
                            CORNERS[e[0]][2] as f32,
                        );
                        let pb = Vec3::new(
                            CORNERS[e[1]][0] as f32,
                            CORNERS[e[1]][1] as f32,
                            CORNERS[e[1]][2] as f32,
                        );
                        sum += pa + (pb - pa) * t;
                        count += 1.0;
                    }
                }
                cell_vertex[(k * n + j) * n + i] = positions.len() as u32;
                positions.push(at(i, j, k) + sum / count * step);
            }
        }
    }
    // Quads across every lattice edge the surface crosses, joining the four
    // cells round it.
    let mut indices = Vec::new();
    let cv = |i: usize, j: usize, k: usize| cell_vertex[(k * n + j) * n + i];
    for k in 1..n {
        for j in 1..n {
            for i in 1..n {
                let here = value(i, j, k) < 0.0;
                for axis in 0..3 {
                    let (ni, nj, nk) = match axis {
                        0 => (i + 1, j, k),
                        1 => (i, j + 1, k),
                        _ => (i, j, k + 1),
                    };
                    if ni > n || nj > n || nk > n || here == (value(ni, nj, nk) < 0.0) {
                        continue;
                    }
                    let quad = match axis {
                        0 => [
                            cv(i, j - 1, k - 1),
                            cv(i, j, k - 1),
                            cv(i, j, k),
                            cv(i, j - 1, k),
                        ],
                        1 => [
                            cv(i - 1, j, k - 1),
                            cv(i - 1, j, k),
                            cv(i, j, k),
                            cv(i, j, k - 1),
                        ],
                        _ => [
                            cv(i - 1, j - 1, k),
                            cv(i, j - 1, k),
                            cv(i, j, k),
                            cv(i - 1, j, k),
                        ],
                    };
                    if quad.contains(&u32::MAX) {
                        continue;
                    }
                    let [a, b, c, d] = quad;
                    if here {
                        indices.extend_from_slice(&[a, b, c, a, c, d]);
                    } else {
                        indices.extend_from_slice(&[a, c, b, a, d, c]);
                    }
                }
            }
        }
    }
    // Each vertex pulled onto the surface along the field's gradient, its
    // normal the gradient, its occlusion from the field a little way out,
    // and the part it lies on.
    let e = step * 0.35;
    let gradient = |p: Vec3| {
        Vec3::new(
            f(p + Vec3::X * e).d - f(p - Vec3::X * e).d,
            f(p + Vec3::Y * e).d - f(p - Vec3::Y * e).d,
            f(p + Vec3::Z * e).d - f(p - Vec3::Z * e).d,
        )
        .normalize_or_zero()
    };
    let vertices: Vec<ShipVertex> = positions
        .iter()
        .map(|&p0| {
            let mut p = p0;
            for _ in 0..2 {
                let d = f(p).d;
                let g = gradient(p);
                let moved = p - g * d;
                // Never further than a cell from where the net put it.
                if (moved - p0).length() < step {
                    p = moved;
                }
            }
            let normal = gradient(p);
            let ao = ((f(p + normal * 0.03).d / 0.03).clamp(0.0, 1.0) * 0.5
                + (f(p + normal * 0.09).d / 0.09).clamp(0.0, 1.0) * 0.5)
                .clamp(0.25, 1.0);
            let part = f(p - normal * step * 0.2).part;
            ShipVertex {
                position: p.to_array(),
                normal: normal.to_array(),
                extra: [(ao * 255.0) as u8, part, 0, 255],
            }
        })
        .collect();
    Mesh { vertices, indices }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_design_meshes_closed_and_sound() {
        for key in Key::all().filter(|k| k.variant == 0) {
            let mesh = build(key);
            assert!(
                mesh.indices.len() > 3000,
                "{key:?}: {} indices",
                mesh.indices.len()
            );
            assert_eq!(mesh.indices.len() % 3, 0);
            assert!(
                mesh.indices
                    .iter()
                    .all(|&i| (i as usize) < mesh.vertices.len())
            );
            for v in &mesh.vertices {
                assert!(
                    v.position
                        .iter()
                        .all(|x| x.is_finite() && x.abs() <= EXTENT + 0.01),
                    "{key:?}"
                );
                let n = Vec3::from_array(v.normal);
                assert!((n.length() - 1.0).abs() < 0.01 || n.length() == 0.0);
            }
        }
    }

    #[test]
    fn variants_differ() {
        for design in 0..DESIGNS {
            let a = build(Key {
                design,
                role: 0,
                variant: 0,
            });
            let b = build(Key {
                design,
                role: 0,
                variant: 1,
            });
            assert_ne!(a.vertices.len(), b.vertices.len(), "design {design}");
        }
    }

    #[test]
    fn the_cache_round_trips() {
        let dir = std::env::temp_dir().join(format!("ship-cache-test-{}", std::process::id()));
        // SAFETY: set before any other thread reads it in this test binary's
        // use of the cache, and only to a directory of this test's own.
        unsafe { std::env::set_var("SOLAR_SHIP_CACHE", &dir) };
        let key = Key {
            design: 4,
            role: 1,
            variant: 2,
        };
        let built = mesh(key);
        let loaded = load(key).expect("cached");
        assert_eq!(built.vertices.len(), loaded.vertices.len());
        assert_eq!(built.indices, loaded.indices);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
