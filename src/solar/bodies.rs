//! The bodies on the frame: which passes draw each, their meshes, the
//! rectangles they cover, their spin and shadows.

use super::*;

/// Whether another body of `system` could pass in front of the star as seen
/// from a body of `radius` at `at`: only then is its eclipse pass drawn.
pub(super) fn eclipsed(
    system: &System,
    t: f64,
    at: [f64; 3],
    radius: f64,
    planet: Option<usize>,
) -> bool {
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

/// The part of the frame, as a scissor rectangle in pixels, where the shadows
/// of body `r`'s shadowers (`shade`) can fall on it and be seen: for each,
/// where the line from the star through it meets body `r`, widened by its
/// size and its penumbra. None if no shadow falls on the side in view, so the
/// eclipse pass is skipped.
pub(super) fn shadow_scissor(params: &Params, r: usize) -> Option<[u32; 4]> {
    let v3 = |a: [f32; 4]| [a[0] as f64, a[1] as f64, a[2] as f64];
    let body = &params.bodies[r];
    let centre = v3(body.place);
    let radius = body.place[3] as f64;
    let star = v3(params.star);
    let (right, up, forward) = (v3(params.right), v3(params.up), v3(params.forward));
    let tan_half = params.frame[1] as f64;
    let (mut lo, mut hi) = ([f64::MAX; 2], [f64::MIN; 2]);
    let mut any = false;
    for j in body.shade {
        if j < 0.0 {
            break;
        }
        let other = &params.bodies[j as usize];
        let o = v3(other.place);
        let d = normalize([o[0] - star[0], o[1] - star[1], o[2] - star[2]]);
        // The point on the body nearest the shadow's axis.
        let rel = [centre[0] - o[0], centre[1] - o[1], centre[2] - o[2]];
        let along = dot(rel, d).max(0.0);
        let axis_point = [
            o[0] + d[0] * along,
            o[1] + d[1] * along,
            o[2] + d[2] * along,
        ];
        let towards = normalize([
            axis_point[0] - centre[0],
            axis_point[1] - centre[1],
            axis_point[2] - centre[2],
        ]);
        let p = [
            centre[0] + towards[0] * radius,
            centre[1] + towards[1] * radius,
            centre[2] + towards[2] * radius,
        ];
        // The shadow's reach on the surface: the shadower's size and its
        // penumbra, as an angle round the body's centre.
        let star_distance = length([p[0] - star[0], p[1] - star[1], p[2] - star[2]]).max(1.0);
        let reach = other.place[3] as f64 + along * params.star[3] as f64 / star_distance;
        let miss = length([
            axis_point[0] - centre[0],
            axis_point[1] - centre[1],
            axis_point[2] - centre[2],
        ]);
        if miss > radius + reach {
            continue;
        }
        let spread = (reach / radius).min(std::f64::consts::PI);
        let e1 = normalize(cross(
            towards,
            if towards[1].abs() < 0.9 {
                [0.0, 1.0, 0.0]
            } else {
                [1.0, 0.0, 0.0]
            },
        ));
        let e2 = cross(towards, e1);
        for n in 0..=12 {
            let (a, b) = if n == 12 {
                (0.0, 0.0)
            } else {
                let angle = n as f64 / 12.0 * std::f64::consts::TAU;
                (spread.min(1.5) * angle.cos(), spread.min(1.5) * angle.sin())
            };
            // A point of the shadow's footprint, round the sphere.
            let dir = normalize([
                towards[0] + e1[0] * a + e2[0] * b,
                towards[1] + e1[1] * a + e2[1] * b,
                towards[2] + e1[2] * a + e2[2] * b,
            ]);
            let q = [
                centre[0] + dir[0] * radius,
                centre[1] + dir[1] * radius,
                centre[2] + dir[2] * radius,
            ];
            // Only the side facing the camera can show it.
            if dot(dir, q) > radius * 0.05 && spread < 1.5 {
                continue;
            }
            let z = dot(q, forward);
            if z <= 1e-6 {
                return Some([0, 0, SIZE, SIZE]);
            }
            let x = dot(q, right) / z / tan_half;
            let y = dot(q, up) / z / tan_half;
            lo = [lo[0].min(x), lo[1].min(y)];
            hi = [hi[0].max(x), hi[1].max(y)];
            any = true;
        }
        if spread >= 1.5 {
            // A shadower as big as the body (or a giant's shadow): all of it.
            return Some([0, 0, SIZE, SIZE]);
        }
    }
    if !any {
        return None;
    }
    let half = SIZE as f64 / 2.0;
    let px = |v: f64| ((v * half + half).clamp(0.0, SIZE as f64)) as u32;
    let margin = 6;
    let x0 = px(lo[0]).saturating_sub(margin);
    let x1 = (px(hi[0]) + margin).min(SIZE);
    // Clip space y is up; the frame's rows go down.
    let y0 = (SIZE - px(hi[1])).saturating_sub(margin);
    let y1 = (SIZE - px(lo[1]) + margin).min(SIZE);
    (x1 > x0 && y1 > y0).then_some([x0, y0, x1 - x0, y1 - y0])
}

/// Which of `bodies` (camera-relative, far to near) can cast a shadow on body
/// `index`: those whose disc, seen from it, comes near the star's. Up to four,
/// as indices; -1 for the rest.
pub(super) fn shadowers(
    bodies: &[(f64, Body, Vec<Pipe>)],
    index: usize,
    star: [f64; 3],
    star_radius: f64,
) -> [f32; 4] {
    let mut out = [-1.0f32; 4];
    let place = bodies[index].1.place;
    let at = [place[0] as f64, place[1] as f64, place[2] as f64];
    let radius = place[3] as f64;
    let to_star = [star[0] - at[0], star[1] - at[1], star[2] - at[2]];
    let star_distance = length(to_star);
    let to_star = normalize(to_star);
    let star_angle = (star_radius / star_distance).min(1.0).asin();
    let mut n = 0;
    for (j, (_, other, _)) in bodies.iter().enumerate() {
        if j == index || other.look[0] < 0.0 || n == 4 {
            continue;
        }
        let rel = [
            other.place[0] as f64 - at[0],
            other.place[1] as f64 - at[1],
            other.place[2] as f64 - at[2],
        ];
        let d = length(rel);
        if d < 1e-6 || d > star_distance {
            continue;
        }
        let separation = dot(normalize(rel), to_star).clamp(-1.0, 1.0).acos();
        let reach =
            (other.place[3] as f64 / d).min(1.0).asin() + star_angle + (radius / d).min(1.0).asin();
        if separation < reach {
            out[n] = j as f32;
            n += 1;
        }
    }
    out
}

/// The passes that draw a planet-sized body of `kind` from map slot `slot`
/// (or flat, if its maps are not in), in order.
pub(super) fn surface_passes(
    kind: Kind,
    slot: Option<usize>,
    eclipsed: bool,
    rings: bool,
    close: bool,
) -> Vec<Pipe> {
    if let Some(s) = slot
        && !close
    {
        // From afar, the ground, clouds and added light as a mesh in one
        // pass (two for a world with seas: its added light over it). The
        // rings' shadow is in it too; the air past the limb is a pass of its
        // own (`limb`, made `limbring` in `params`).
        let look = SurfaceLook::of(kind, rings);
        let mut passes = vec![Pipe::Globe(look, s)];
        if look.sea {
            passes.push(Pipe::GlobeGlow(look, s));
        }
        if eclipsed {
            passes.push(Pipe::Eclipse);
        }
        if kind.air()[0] > 0.0 {
            passes.push(Pipe::Limb);
        }
        if rings {
            passes.push(Pipe::Ring);
        }
        return passes;
    }
    let mut passes: Vec<Pipe> = match slot {
        Some(s) => {
            // Close up, the ground with its detail (one pass); higher up the
            // map alone is fine enough.
            // Close up, the ground with its lookups made crisp, then the
            // detail the map is too coarse for; higher up the map alone.
            let mut passes = vec![if close && kind.has_ground() {
                Pipe::GroundDetail(s)
            } else {
                Pipe::Ground(s)
            }];
            if close && kind.has_ground() {
                passes.push(Pipe::Detail(s));
            }
            if kind.has_clouds() {
                passes.push(Pipe::Clouds(s));
                // In low orbit, clouds that stand up from the deck.
                if close {
                    for layer in 1..=CLOUD_LAYERS {
                        passes.push(Pipe::CloudLayer(s, layer));
                    }
                }
            }
            passes
        }
        // Its maps not in: as a mesh with its rings' shadow in it.
        None => vec![Pipe::GlobeFlat(FlatLook::of(kind, rings))],
    };
    if eclipsed {
        passes.push(Pipe::Eclipse);
    }
    if rings && slot.is_some() {
        passes.push(Pipe::RingShadow);
    }
    if let Some(s) = slot {
        // The added light: night lights, the sea's glint, the air's sheen.
        passes.push(if kind.has_sea() {
            Pipe::GlowSea(s)
        } else {
            Pipe::Glow(s)
        });
    }
    if kind.air()[0] > 0.0 {
        passes.push(Pipe::Limb);
    }
    if rings {
        passes.push(Pipe::Ring);
    }
    passes
}

/// What a kind's world from afar needs drawn (`fs_globe`'s constants).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) struct SurfaceLook {
    pub(super) sea: bool,
    pub(super) clouds: bool,
    pub(super) night: u8,
    pub(super) air: bool,
    pub(super) giant: bool,
    pub(super) rings: bool,
}

impl SurfaceLook {
    /// For a world of `kind`, with rings or without.
    pub(super) fn of(kind: Kind, rings: bool) -> Self {
        Self {
            sea: kind.has_sea(),
            clouds: kind.has_clouds(),
            night: night_mode(kind) as u8,
            air: kind.air()[0] > 0.0,
            giant: matches!(kind, Kind::GasGiant | Kind::IceGiant | Kind::HotJupiter),
            rings,
        }
    }

    /// The look's flags after the slot: `sc0a--` and the like.
    pub(super) fn flags(self, slot: usize) -> String {
        let flag = |on: bool, c: char| if on { c } else { '-' };
        format!(
            "{slot}{}{}{}{}{}{}",
            flag(self.sea, 's'),
            flag(self.clouds, 'c'),
            self.night,
            flag(self.air, 'a'),
            flag(self.giant, 'g'),
            flag(self.rings, 'r'),
        )
    }

    pub(super) fn constants(self) -> [(&'static str, f64); 6] {
        [
            ("SEA", f64::from(u8::from(self.sea))),
            ("CLOUDS", f64::from(u8::from(self.clouds))),
            ("NIGHT", f64::from(self.night)),
            ("AIR", f64::from(u8::from(self.air))),
            ("GIANT", f64::from(u8::from(self.giant))),
            ("RINGS", f64::from(u8::from(self.rings))),
        ]
    }
}

/// What a world whose maps are not in yet needs drawn as a mesh
/// (`fs_globe_flat`'s constants).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) struct FlatLook {
    pub(super) air: bool,
    pub(super) giant: bool,
    pub(super) rings: bool,
}

impl FlatLook {
    pub(super) fn of(kind: Kind, rings: bool) -> Self {
        Self {
            air: kind.air()[0] > 0.0,
            giant: matches!(kind, Kind::GasGiant | Kind::IceGiant | Kind::HotJupiter),
            rings,
        }
    }

    pub(super) fn constants(self) -> [(&'static str, f64); 4] {
        [
            ("GLOBE", 1.0),
            ("AIR", f64::from(u8::from(self.air))),
            ("GIANT", f64::from(u8::from(self.giant))),
            ("RINGS", f64::from(u8::from(self.rings))),
        ]
    }
}

pub(super) fn night_mode(kind: Kind) -> f32 {
    if kind == Kind::Starbase {
        // A station's own lights, coloured (`station_lights` in the frame).
        3.0
    } else if kind.has_cities() {
        1.0
    } else if kind.glows() {
        2.0
    } else {
        0.0
    }
}

pub(super) fn flat_colour(planet: &Planet) -> [f32; 4] {
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
pub(super) fn spin_angle(planet: &Planet, t: f64, centre: [f64; 3]) -> f64 {
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
pub(super) fn rotation(planet: &Planet, angle: f64, clouds: f64) -> [[f32; 4]; 3] {
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
pub(super) fn sky_rotation(_sky: [f32; 4]) -> [[f32; 4]; 3] {
    [
        [1.0, 0.0, 0.0, 0.0],
        [0.0, 1.0, 0.0, 0.0],
        [0.0, 0.0, 1.0, 0.0],
    ]
}

/// For a sphere around or close beside the camera, whose outline reaches
/// behind it: the part of the frame whose rays hit it, from a grid of rays,
/// a cell wider all round. In low orbit that leaves out the sky above the
/// horizon, which every pass over the planet would otherwise trace for
/// nothing. None if no ray hits it.
pub(super) fn close_rect(view: &View, centre: [f64; 3], radius: f64) -> Option<[f32; 4]> {
    const CELLS: usize = 32;
    let right = cross(view.forward, view.up);
    let inside = length(centre) <= radius;
    let (mut lo, mut hi) = ([usize::MAX; 2], [0usize; 2]);
    for gy in 0..=CELLS {
        for gx in 0..=CELLS {
            let x = gx as f64 / CELLS as f64 * 2.0 - 1.0;
            let y = gy as f64 / CELLS as f64 * 2.0 - 1.0;
            let dir = normalize(
                [0, 1, 2]
                    .map(|i| view.forward[i] + (right[i] * x + view.up[i] * y) * view.half_fov),
            );
            let along = dot(centre, dir);
            let miss = length([0, 1, 2].map(|i| centre[i] - dir[i] * along));
            if inside || (along > 0.0 && miss <= radius) {
                lo = [lo[0].min(gx), lo[1].min(gy)];
                hi = [hi[0].max(gx), hi[1].max(gy)];
            }
        }
    }
    if lo[0] == usize::MAX {
        return None;
    }
    let edge = |g: usize, grow: i64| {
        ((g as i64 + grow).clamp(0, CELLS as i64) as f64 / CELLS as f64 * 2.0 - 1.0) as f32
    };
    Some([
        edge(lo[0], -1),
        edge(lo[1], -1),
        edge(hi[0], 1),
        edge(hi[1], 1),
    ])
}

/// A mapped world's mesh (`vs_globe`): the patch of its latitude-longitude
/// grid the camera can see, as `Body::window`, and `Body::mesh`: the grid's
/// cells a side, few enough that a far body costs little and many enough
/// that the silhouette keeps within 0.2 px of the circle, and how far the
/// mesh stands proud, 1.5 px at the limb, for the soft edge. `pixel` is the
/// tangent a pixel spans (`frame.z`).
pub(super) fn globe_mesh(body: &Body, pixel: f32) -> ([f32; 4], [f32; 4]) {
    use std::f64::consts::{FRAC_PI_2, PI};
    let c = [0, 1, 2].map(|i| f64::from(body.place[i]));
    let r = f64::from(body.place[3]);
    let d = length(c).max(r * 1.001);
    // The way to the camera in the body's turning frame.
    let row = |k: usize| [0, 1, 2].map(|i| f64::from(body.rot[k][i]));
    let a = [0, 1, 2].map(|k| -dot(row(k), c) / d);
    let lat_c = a[1].clamp(-1.0, 1.0).asin();
    let lon_c = a[0].atan2(a[2]);
    let tangent = (d * d - r * r).sqrt();
    let pixel = f64::from(pixel);
    // The limb's radius in pixels, and the widest cell that keeps the
    // silhouette (a cell's sag, about R c^2 / 2) within 0.2 px.
    let limb = r / (tangent * pixel);
    let cell = (0.4 / limb.max(1e-6)).sqrt();
    // The cap in view, two cells wider.
    let alpha = (r / d).acos() + 2.0 * cell;
    let lat_lo = (lat_c - alpha).max(-FRAC_PI_2);
    let lat_hi = (lat_c + alpha).min(FRAC_PI_2);
    let reach = alpha.sin() / lat_c.cos().max(1e-6);
    let pole = lat_c + alpha >= FRAC_PI_2 || lat_c - alpha <= -FRAC_PI_2 || reach >= 1.0;
    // With a pole in view, every longitude.
    let half = if pole { PI } else { reach.asin() };
    let (lon_lo, lon_span) = (lon_c - half, 2.0 * half);
    // The window's widest side as an arc, at the latitude nearest the equator.
    let mut widest = lat_lo.cos().max(lat_hi.cos());
    if lat_lo <= 0.0 && lat_hi >= 0.0 {
        widest = 1.0;
    }
    let span = (lat_hi - lat_lo).max(lon_span * widest);
    let cells = GRIDS
        .into_iter()
        .find(|&n| span / f64::from(n) <= cell)
        .unwrap_or(GRIDS[GRIDS.len() - 1]);
    let proud = 1.5 * pixel * tangent / r;
    let window = [lon_lo, lat_lo, lon_span, lat_hi - lat_lo];
    let mesh = [cells as f32, proud as f32, 0.0, 0.0];
    (window.map(|v| v as f32), mesh)
}

/// The quad covering a sphere of `radius` at `centre` (camera-relative), in
/// clip space, or none if it is out of view.
/// Whether a body's silhouette lies wholly ahead of the camera, well clear
/// of it, so its depth mask can be the polygon inscribed in it.
pub(super) fn inscribable(body: &Body, params: &Params) -> bool {
    wholly_ahead(body, params, 1.0)
}

/// How the star's glare is drawn: as a polygon round the star out to where
/// it falls under `GLARE_FLOOR` (`glare`), over the whole frame when that
/// reaches too far round the sky (`glarefull`), or not at all when the
/// star's disc is hidden. The same sums as `vs_glare`.
pub(super) fn glare_pass(params: &Params) -> Option<Pipe> {
    let shown = f64::from(params.star_colour[3]);
    let colour = (0..3).map(|i| f64::from(params.star_colour[i]) * shown);
    let bright = colour.fold(0.0, f64::max);
    if bright <= 0.0 {
        return None;
    }
    let distance = length([0, 1, 2].map(|i| f64::from(params.star[i]))).max(1e-3);
    let chord = f64::from(params.star[3]) / distance * 0.012 * bright / GLARE_FLOOR;
    let wide = chord >= 2.0 * (GLARE_POLYGON / 2.0).sin();
    Some(if wide { Pipe::GlareFull } else { Pipe::Glare })
}

/// Whether everything within `scale` of its radii of the body's centre is
/// wholly ahead of the camera, its silhouette a circle on the cone of
/// tangents (for `scale` 1.08, its air's: the limb's annulus).
pub(super) fn wholly_ahead(body: &Body, params: &Params, scale: f64) -> bool {
    let c = [0, 1, 2].map(|i| f64::from(body.place[i]));
    let r = f64::from(body.place[3]) * scale;
    let distance = length(c);
    if distance <= r * 1.05 {
        return false;
    }
    let forward = [0, 1, 2].map(|i| f64::from(params.forward[i]));
    let spread = r * distance / (distance * distance - r * r).sqrt();
    dot(c, forward) - spread > distance * 0.05
}

pub(super) fn rect(view: &View, centre: [f64; 3], radius: f64) -> Option<[f32; 4]> {
    let distance = length(centre);
    let ahead = dot(centre, view.forward);
    if distance <= radius * 1.02 || ahead <= radius {
        // Around or beside the camera: what of the frame its rays hit.
        return close_rect(view, centre, radius);
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
            return close_rect(view, centre, radius);
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
