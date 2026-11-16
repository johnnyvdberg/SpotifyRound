//! The cities of the world inside the base, worked out the same here and in
//! `starbase/city.wesl`: the map paints them a texel at a time, and the
//! meshes of their buildings stand on the lots it paints
//! (`docs/starbase-city-algorithm.md`).
//!
//! Everything is in ground metres: round the cylinder (`R·θ`, θ the true
//! angle about its axis, not the map's diamond coordinate) and along it,
//! tiling every `ROUND` and `ALONG`. What both sides must agree on is
//! closed-form arithmetic over a table this side builds once (`table`):
//! the towns' density kernels, the arterial roads as straight pieces, and a
//! grid listing the pieces near each square of it.
//!
//! - Towns at quasi-random sites, sized by rank (Zipf), each a Clark kernel
//!   `exp(-r/b)`, the largest few with sub-centres.
//! - Arterials: each town joined to the earlier one that pulls it hardest
//!   (rank over distance squared), and to its nearest neighbour when close;
//!   each bent a little; those between the largest towns highways.
//! - Rivers as sums of sines along the axis, the sea a band round it.
//! - Built or not: the kernels, ribbons along the roads and the water, less
//!   the hills, cut against a correlated noise, so cores are solid, edges
//!   ragged, with leapfrog suburbs and strips along the roads.
//! - Districts: Voronoi cells about 400 m across, each classed at its own
//!   site; a block grid each, turned to its nearest road (or radial in an
//!   old core); lots in the blocks, a building on each lot that is built,
//!   its height lognormal about what the district and the density say.

use std::f32::consts::TAU;
use std::sync::OnceLock;

/// The ground's radius, metres (`WORLD_GROUND` in `world.wesl`).
pub const GROUND: f32 = 4200.0;
/// Metres round the cylinder, and along it before the map repeats.
pub const ROUND: f32 = TAU * GROUND;
pub const ALONG: f32 = 13_194.0;
/// Where round the cylinder its three strip suns lie, metres (the map's
/// `sun_off`: a sixth, a half and five sixths of the way round it).
pub const SUNS: [f32; 3] = [4650.0246, 13_194.689, 21_739.354];

/// The grid listing the roads near each square: squares round and along.
const GRID: [usize; 2] = [24, 12];
/// How far the roads are looked for, metres: the furthest any of them
/// reaches into the density or a district's turn.
const REACH: f32 = 700.0;
/// District cells round and along a tile (about 400 m each).
const CELLS: [f32; 2] = [66.0, 33.0];
/// The table's fixed sizes (`CityTable` in `city.wesl`).
pub const MAX_KERNELS: usize = 160;
pub const MAX_SEGMENTS: usize = 1024;

/// What a district is.
pub mod kind {
    pub const RURAL: u32 = 0;
    pub const PARK: u32 = 1;
    pub const CORE: u32 = 2;
    pub const WORKS: u32 = 3;
    pub const HIGH: u32 = 4;
    pub const LOW: u32 = 5;
}

/// A building's shape, its footprint per unit of its mesh's scale and how
/// tall that scale stands it (`shape_extent` in `city.wesl`).
pub mod shape {
    pub const BLOCK: u32 = 0;
    pub const SHED: u32 = 1;
    pub const SLAB: u32 = 2;
    pub const TAPER: u32 = 3;
    pub const ROUND: u32 = 4;
    pub const NEEDLE: u32 = 5;
    pub const HOUSE: u32 = 6;
}

fn fract(x: f32) -> f32 {
    x - x.floor()
}

fn smoothstep(e0: f32, e1: f32, x: f32) -> f32 {
    let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

fn mix(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

/// `d` brought into -p/2..p/2: the nearest image of a difference.
fn wrap1(d: f32, p: f32) -> f32 {
    d - p * (d / p + 0.5).floor()
}

/// The nearest image of a difference in metres, both ways.
pub fn wrap(d: [f32; 2]) -> [f32; 2] {
    [wrap1(d[0], ROUND), wrap1(d[1], ALONG)]
}

fn length(d: [f32; 2]) -> f32 {
    (d[0] * d[0] + d[1] * d[1]).sqrt()
}

/// `hash_cell` in `frame.wesl`, line for line.
pub fn hash_cell(p: [f32; 3]) -> f32 {
    let mut q = [
        fract(p[0] * 0.1031),
        fract(p[1] * 0.1030),
        fract(p[2] * 0.0973),
    ];
    let d = q[0] * (q[1] + 33.33) + q[1] * (q[2] + 33.33) + q[2] * (q[0] + 33.33);
    q = [q[0] + d, q[1] + d, q[2] + d];
    fract((q[0] + q[1]) * q[2])
}

/// Value noise tiling the map, `n` cells along it and twice that round
/// (the tile is twice as far round as along), 0..1 (`city_value`).
pub fn value(m: [f32; 2], n: f32, seed: f32) -> f32 {
    let g = [m[0] / ROUND * (2.0 * n), m[1] / ALONG * n];
    let i = [g[0].floor(), g[1].floor()];
    let f = [g[0] - i[0], g[1] - i[1]];
    let u = [
        f[0] * f[0] * (3.0 - 2.0 * f[0]),
        f[1] * f[1] * (3.0 - 2.0 * f[1]),
    ];
    let period = [2.0 * n, n];
    let at = |dx: f32, dz: f32| {
        let c = [i[0] + dx, i[1] + dz];
        let c = [
            c[0] - (c[0] / period[0]).floor() * period[0],
            c[1] - (c[1] / period[1]).floor() * period[1],
        ];
        hash_cell([c[0], c[1], seed])
    };
    mix(
        mix(at(0.0, 0.0), at(1.0, 0.0), u[0]),
        mix(at(0.0, 1.0), at(1.0, 1.0), u[0]),
        u[1],
    )
}

/// Fractal value noise, `octaves` from `n` cells, about -1..1 (`city_fbm`).
pub fn fbm(m: [f32; 2], n: f32, octaves: i32, seed: f32) -> f32 {
    let mut sum = 0.0;
    let mut amplitude = 0.5;
    let mut c = n;
    for i in 0..octaves {
        sum += amplitude * (value(m, c, seed + i as f32) * 2.0 - 1.0);
        c *= 2.0;
        amplitude *= 0.5;
    }
    sum
}

/// How high the ground stands, about -0.7..0.7 (`city_height`).
pub fn height(m: [f32; 2]) -> f32 {
    0.75 * fbm(m, 3.0, 5, 1.0)
}

/// The noise the density is cut against, about -1..1 (`city_noise`).
pub fn noise(m: [f32; 2]) -> f32 {
    1.6 * fbm(m, 16.0, 3, 9.0)
}

/// The rivers: where round each one runs at a place along, as sines along
/// the axis (a whole number of waves a tile): its middle and three waves'
/// size and phase.
const RIVERS: [[f32; 7]; 3] = [
    [0.0, 700.0, 0.3, 300.0, 2.1, 140.0, 4.0],
    [8920.0, 650.0, 1.7, 350.0, 5.2, 120.0, 0.9],
    [17_470.0, 750.0, 3.9, 280.0, 0.4, 150.0, 2.8],
];
const RIVER_HALF: f32 = 26.0;

/// How far from the nearest river's bank, metres (less than 0 in it).
pub fn river(m: [f32; 2]) -> f32 {
    let w = m[1] / ALONG * TAU;
    let mut best = 1e9f32;
    for r in RIVERS {
        let x = r[0]
            + r[1] * (w + r[2]).sin()
            + r[3] * (2.0 * w + r[4]).sin()
            + r[5] * (3.0 * w + r[6]).sin();
        let slope = (r[1] * (w + r[2]).cos()
            + 2.0 * r[3] * (2.0 * w + r[4]).cos()
            + 3.0 * r[5] * (3.0 * w + r[6]).cos())
            * (TAU / ALONG);
        let d = wrap1(m[0] - x, ROUND).abs() / (1.0 + slope * slope).sqrt();
        best = best.min(d);
    }
    best - RIVER_HALF
}

/// How far from the sea's shore, metres (less than 0 in it): a band round
/// the middle of the tile, its shores as the land lies.
pub fn sea(m: [f32; 2], h: f32) -> f32 {
    (m[1] - ALONG * 0.5).abs() - 520.0 - 3350.0 * h
}

/// Lakes lie where the ground is lower than this.
pub const LAKE: f32 = -0.27;

/// How far round from the nearest strip sun, metres.
pub fn sun_distance(x: f32) -> f32 {
    let mut best = 1e9f32;
    for s in SUNS {
        best = best.min(wrap1(x - s, ROUND).abs());
    }
    best
}

/// The roads near a place: how far to the nearest one's edge and the
/// nearest highway's (metres, `REACH` at most), the nearest one's way and
/// how far along the network that is, metres.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Roads {
    pub near: f32,
    pub highway: f32,
    pub way: [f32; 2],
    pub along: f32,
}

/// What a place is: its density and what goes into it.
#[derive(Clone, Copy, Debug)]
pub struct Here {
    /// The towns' kernels alone, and the one counting most here.
    pub kernel: f32,
    pub town: usize,
    /// All of it: kernels, ribbons, less the hills.
    pub total: f32,
    pub roads: Roads,
    /// How far from open water, metres; whether in it.
    pub water: f32,
    pub wet: bool,
}

/// A district cell: its site (metres, near the place asked about), its
/// cell on the map's grid of them, and how far the place is from its edge.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Cell {
    pub site: [f32; 2],
    pub id: [f32; 2],
    pub edge: f32,
}

/// A district: its kind, the turn of its block grid, its blocks' and
/// lots' size and its streets' half width, metres.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct District {
    pub kind: u32,
    pub turn: f32,
    pub block: f32,
    pub lot: f32,
    pub street: f32,
}

/// A lot and what stands on it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Lot {
    /// Its middle, metres; its block's place in the district's grid and its
    /// own in the block; its half size.
    pub centre: [f32; 2],
    pub key: [f32; 3],
    pub half: f32,
    pub developed: bool,
    /// Whether it is built over (paved, its streets laid), whether a
    /// building stands on it, how tall, of what shape, how far
    /// out its footprint reaches along the block's two ways, and a number
    /// of its own (its tint).
    pub built: bool,
    pub height: f32,
    pub shape: u32,
    pub footprint: [f32; 2],
    pub tone: f32,
}

/// The table both sides read.
pub struct City {
    /// Each town's kernel: where (metres), `b` and how strong.
    pub kernels: Vec<[f32; 4]>,
    /// The roads as straight pieces: ends (metres, the second near the
    /// first, not wrapped), and half width, highway or not, how far along
    /// the network at its start and its length.
    pub segments: Vec<[f32; 4]>,
    pub roads: Vec<[f32; 4]>,
    /// For each square of the grid, its first entry in `list` and how many.
    pub cells: Vec<[u32; 2]>,
    pub list: Vec<u32>,
    /// Each district cell's kind and the turn of its block grid, classed
    /// once at its own site (`classify`), a row a cell.
    pub districts: Vec<[f32; 2]>,
}

/// The city, made once.
pub fn city() -> &'static City {
    static CITY: OnceLock<City> = OnceLock::new();
    CITY.get_or_init(City::new)
}

fn wet_at(m: [f32; 2], h: f32) -> bool {
    river(m) < 0.0 || sea(m, h) < 0.0 || h < LAKE
}

impl City {
    fn new() -> City {
        // Towns at quasi-random sites, none on a sun, in the water or high
        // in the hills; by rank, each smaller than the last.
        let mut towns: Vec<[f32; 2]> = Vec::new();
        for k in 0..110 {
            let i = k as f32;
            let m = [
                fract(i * 0.618034 + 0.13) * ROUND,
                fract(i * 0.754878 + 0.31) * ALONG,
            ];
            let h = height(m);
            if sun_distance(m[0]) > 1100.0 && !wet_at(m, h) && h < 0.2 {
                towns.push(m);
            }
        }
        let mut kernels = Vec::new();
        for (r, m) in towns.iter().enumerate() {
            let b = 1600.0 * ((r + 1) as f32).powf(-0.45);
            kernels.push([m[0], m[1], b, 1.0]);
            // The largest few have sub-centres round them.
            let subs = [4, 3, 2, 2].get(r).copied().unwrap_or(0);
            for s in 0..subs {
                let u = hash_cell([r as f32, s as f32, 7.0]);
                let a = TAU * (s as f32 / subs as f32 + 0.3 * u);
                let d = b * (1.3 + 0.9 * hash_cell([r as f32, s as f32, 8.0]));
                let p = [
                    (m[0] + a.cos() * d).rem_euclid(ROUND),
                    (m[1] + a.sin() * d).rem_euclid(ALONG),
                ];
                if !wet_at(p, height(p)) && sun_distance(p[0]) > 600.0 {
                    kernels.push([p[0], p[1], b * 0.38, 0.75]);
                }
            }
        }
        assert!(kernels.len() <= MAX_KERNELS);
        // The arterials: each town to the earlier one pulling it hardest,
        // and to its nearest neighbour when that is close.
        let apart = |a: usize, b: usize| {
            length(wrap([towns[b][0] - towns[a][0], towns[b][1] - towns[a][1]]))
        };
        let mut edges: Vec<(usize, usize)> = Vec::new();
        for i in 1..towns.len() {
            let pull = |j: usize| 1.0 / ((j + 1) as f32 * apart(i, j).powi(3));
            let j = (0..i)
                .max_by(|&a, &b| pull(a).total_cmp(&pull(b)))
                .unwrap_or(0);
            edges.push((j, i));
            let near = (0..towns.len())
                .filter(|&n| n != i)
                .min_by(|&a, &b| apart(i, a).total_cmp(&apart(i, b)));
            if let Some(n) = near {
                let e = (n.min(i), n.max(i));
                if apart(i, n) < 3200.0 && !edges.contains(&e) && !edges.contains(&(e.1, e.0)) {
                    edges.push(e);
                }
            }
        }
        let mut segments = Vec::new();
        let mut roads = Vec::new();
        let mut along = 0.0;
        for &(a, b) in &edges {
            let p0 = towns[a];
            let d = wrap([towns[b][0] - p0[0], towns[b][1] - p0[1]]);
            let len = length(d);
            let normal = [-d[1] / len, d[0] / len];
            let bend = (hash_cell([a as f32, b as f32, 11.0]) - 0.5) * 0.3 * len;
            let p1 = [
                p0[0] + d[0] * 0.5 + normal[0] * bend,
                p0[1] + d[1] * 0.5 + normal[1] * bend,
            ];
            let p2 = [p0[0] + d[0], p0[1] + d[1]];
            let pieces = (len / 600.0).ceil().clamp(2.0, 8.0) as usize;
            let point = |t: f32| {
                let s = 1.0 - t;
                [
                    s * s * p0[0] + 2.0 * s * t * p1[0] + t * t * p2[0],
                    s * s * p0[1] + 2.0 * s * t * p1[1] + t * t * p2[1],
                ]
            };
            let highway = a.max(b) < 14;
            let half = if highway { 13.0 } else { 8.0 };
            for k in 0..pieces {
                let s = point(k as f32 / pieces as f32);
                let e = point((k + 1) as f32 / pieces as f32);
                let start = [s[0].rem_euclid(ROUND), s[1].rem_euclid(ALONG)];
                let end = [start[0] + e[0] - s[0], start[1] + e[1] - s[1]];
                let piece = length([e[0] - s[0], e[1] - s[1]]);
                segments.push([start[0], start[1], end[0], end[1]]);
                roads.push([half, if highway { 1.0 } else { 0.0 }, along, piece]);
                along += piece;
            }
        }
        assert!(
            segments.len() <= MAX_SEGMENTS,
            "{} road pieces",
            segments.len()
        );
        // The grid: each square lists the pieces that might be within
        // `REACH` of a point in it.
        let size = [ROUND / GRID[0] as f32, ALONG / GRID[1] as f32];
        let corner = 0.5 * length(size);
        let mut cells = Vec::new();
        let mut list = Vec::new();
        for gz in 0..GRID[1] {
            for gx in 0..GRID[0] {
                let c = [(gx as f32 + 0.5) * size[0], (gz as f32 + 0.5) * size[1]];
                let first = list.len() as u32;
                for (k, s) in segments.iter().enumerate() {
                    if segment_distance(c, *s).0 < REACH + corner {
                        list.push(k as u32);
                    }
                }
                cells.push([first, list.len() as u32 - first]);
            }
        }
        let mut city = City {
            kernels,
            segments,
            roads,
            cells,
            list,
            districts: Vec::new(),
        };
        city.districts = (0..(CELLS[0] * CELLS[1]) as usize)
            .map(|i| {
                let id = [
                    (i % CELLS[0] as usize) as f32,
                    (i / CELLS[0] as usize) as f32,
                ];
                let (kind, turn) = city.classify(id);
                [kind as f32, turn]
            })
            .collect();
        city
    }

    /// The table as `city.wesl`'s `CityTable` lays it out.
    pub fn table(&self) -> Vec<u8> {
        let mut out = Vec::new();
        let mut word = |w: u32| out.extend_from_slice(&w.to_le_bytes());
        word(self.kernels.len() as u32);
        word(self.segments.len() as u32);
        word(0);
        word(0);
        let floats = |rows: &[[f32; 4]], size: usize, out: &mut Vec<u8>| {
            for k in 0..size {
                for v in rows.get(k).copied().unwrap_or([0.0; 4]) {
                    out.extend_from_slice(&v.to_le_bytes());
                }
            }
        };
        floats(&self.kernels, MAX_KERNELS, &mut out);
        floats(&self.segments, MAX_SEGMENTS, &mut out);
        floats(&self.roads, MAX_SEGMENTS, &mut out);
        for c in &self.cells {
            for w in [c[0], c[1], 0, 0] {
                out.extend_from_slice(&w.to_le_bytes());
            }
        }
        for d in &self.districts {
            for v in [d[0], d[1], 0.0, 0.0] {
                out.extend_from_slice(&v.to_le_bytes());
            }
        }
        for w in &self.list {
            out.extend_from_slice(&w.to_le_bytes());
        }
        out
    }

    /// The roads near `m` (`city_roads`).
    pub fn roads_at(&self, m: [f32; 2]) -> Roads {
        let gx =
            ((m[0] - ROUND * (m[0] / ROUND).floor()) / ROUND * GRID[0] as f32).floor() as usize;
        let gz =
            ((m[1] - ALONG * (m[1] / ALONG).floor()) / ALONG * GRID[1] as f32).floor() as usize;
        let cell = self.cells[gz.min(GRID[1] - 1) * GRID[0] + gx.min(GRID[0] - 1)];
        let mut out = Roads {
            near: REACH,
            highway: REACH,
            way: [1.0, 0.0],
            along: 0.0,
        };
        for k in cell[0]..cell[0] + cell[1] {
            let i = self.list[k as usize] as usize;
            let s = self.segments[i];
            let r = self.roads[i];
            let (d, t) = segment_distance(m, s);
            let edge = d - r[0];
            if edge < out.near {
                out.near = edge;
                let len = r[3].max(1e-3);
                out.way = [(s[2] - s[0]) / len, (s[3] - s[1]) / len];
                out.along = r[2] + t * r[3];
            }
            if r[1] > 0.5 {
                out.highway = out.highway.min(edge);
            }
        }
        out
    }

    /// What place `m` is (`city_here`).
    pub fn here(&self, m: [f32; 2]) -> Here {
        let h = height(m);
        let roads = self.roads_at(m);
        let mut kernel = 0.0;
        let mut town = 0;
        let mut most = 0.0;
        for (k, c) in self.kernels.iter().enumerate() {
            let r = length(wrap([m[0] - c[0], m[1] - c[1]]));
            if r < 8.0 * c[2] {
                let e = c[3] * (-r / c[2]).exp();
                kernel += e;
                if e > most {
                    most = e;
                    town = k;
                }
            }
        }
        let water = river(m).min(sea(m, h)).min((h - LAKE) * 3000.0);
        let mut total = kernel
            + 0.28 * (-roads.highway.max(0.0) / 180.0).exp()
            + 0.18 * (-roads.near.max(0.0) / 120.0).exp()
            + 0.16 * (-water.max(0.0) / 220.0).exp()
            - 1.3 * (h - 0.12).max(0.0);
        total *= smoothstep(250.0, 700.0, sun_distance(m[0]));
        Here {
            kernel,
            town,
            total,
            roads,
            water,
            wet: water < 0.0,
        }
    }

    /// A district cell's kind and the turn of its block grid, classed at its
    /// own site: rural, a park, a commercial core, works along the highways
    /// and the water, or homes, high or low by the density.
    fn classify(&self, id: [f32; 2]) -> (u32, f32) {
        let site = site_of(id);
        let here = self.here(site);
        let n = noise(site);
        let u = hash_cell([id[0], id[1], 63.0]);
        let urban = !here.wet && here.total - 0.35 + 0.22 * n > -0.05;
        let kind = if !urban {
            kind::RURAL
        } else if here.kernel > 0.62 {
            kind::CORE
        } else if u < 0.08 {
            kind::PARK
        } else if (here.roads.highway < 350.0 || here.water < 300.0)
            && u < 0.55
            && here.kernel < 0.5
        {
            kind::WORKS
        } else if here.total > 0.55 {
            kind::HIGH
        } else {
            kind::LOW
        };
        // An old core's streets run out from its middle; elsewhere a grid
        // turned to the nearest road, or its own way.
        let t = self.kernels[here.town];
        let from = wrap([site[0] - t[0], site[1] - t[1]]);
        let turn = if kind == kind::CORE && length(from) < 1.5 * t[2] && length(from) > 1.0 {
            from[1].atan2(from[0])
        } else if here.roads.near < 600.0 {
            here.roads.way[1].atan2(here.roads.way[0])
        } else {
            u * 7.0
        };
        (kind, turn)
    }

    /// The district of cell `c`, from the table (`city_district`).
    pub fn district(&self, c: &Cell) -> District {
        let row = self.districts[c.id[0] as usize + c.id[1] as usize * CELLS[0] as usize];
        let kind = row[0] as u32;
        let k = kind as usize;
        District {
            kind,
            turn: row[1],
            block: [170.0, 120.0, 130.0, 190.0, 120.0, 145.0][k],
            lot: [80.0, 60.0, 55.0, 80.0, 38.0, 28.0][k],
            street: [2.0, 3.0, 7.0, 7.0, 6.0, 5.0][k],
        }
    }
}

/// A district cell's own site, metres.
fn site_of(id: [f32; 2]) -> [f32; 2] {
    let j = [
        hash_cell([id[0], id[1], 61.0]) * 0.7 + 0.15,
        hash_cell([id[0], id[1], 62.0]) * 0.7 + 0.15,
    ];
    [
        (id[0] + j[0]) * ROUND / CELLS[0],
        (id[1] + j[1]) * ALONG / CELLS[1],
    ]
}

/// How far `p` is from piece `s`, and how far along it (0..1) the nearest
/// point is (`city_segment`).
fn segment_distance(p: [f32; 2], s: [f32; 4]) -> (f32, f32) {
    let d = wrap([p[0] - s[0], p[1] - s[1]]);
    let e = [s[2] - s[0], s[3] - s[1]];
    let t = ((d[0] * e[0] + d[1] * e[1]) / (e[0] * e[0] + e[1] * e[1]).max(1e-3)).clamp(0.0, 1.0);
    (length([d[0] - e[0] * t, d[1] - e[1] * t]), t)
}

/// The district cell `m` is in (`city_cells`).
pub fn cell_at(m: [f32; 2]) -> Cell {
    let size = [ROUND / CELLS[0], ALONG / CELLS[1]];
    let g = [m[0] / ROUND * CELLS[0], m[1] / ALONG * CELLS[1]];
    let cell = [g[0].floor(), g[1].floor()];
    let f = [g[0] - cell[0], g[1] - cell[1]];
    let mut first = 1e9f32;
    let mut second = 1e9f32;
    let mut out = Cell {
        site: m,
        id: [0.0, 0.0],
        edge: 0.0,
    };
    for k in 0..9 {
        let o = [(k % 3) as f32 - 1.0, (k / 3) as f32 - 1.0];
        let c = [cell[0] + o[0], cell[1] + o[1]];
        let id = [
            c[0] - (c[0] / CELLS[0]).floor() * CELLS[0],
            c[1] - (c[1] / CELLS[1]).floor() * CELLS[1],
        ];
        let j = [
            hash_cell([id[0], id[1], 61.0]) * 0.7 + 0.15,
            hash_cell([id[0], id[1], 62.0]) * 0.7 + 0.15,
        ];
        let r = [
            (o[0] + j[0] - f[0]) * size[0],
            (o[1] + j[1] - f[1]) * size[1],
        ];
        let d = length(r);
        if d < first {
            second = first;
            first = d;
            out.site = [m[0] + r[0], m[1] + r[1]];
            out.id = id;
        } else if d < second {
            second = d;
        }
    }
    out.edge = (second - first) * 0.5;
    out
}

#[cfg(test)]
/// `m` in a district's frame about its site, its blocks' grid's: turned,
/// and in low suburbs warped a little so the streets curve (`city_local`).
pub fn local(m: [f32; 2], c: &Cell, d: &District) -> [f32; 2] {
    let r = wrap([m[0] - c.site[0], m[1] - c.site[1]]);
    let (s, k) = d.turn.sin_cos();
    let l = [r[0] * k + r[1] * s, -r[0] * s + r[1] * k];
    if d.kind == kind::LOW {
        [
            l[0] + 9.0 * (l[1] / 140.0).sin(),
            l[1] + 9.0 * (l[0] / 140.0).sin(),
        ]
    } else {
        l
    }
}

/// Back from a district's frame to metres, the warp undone near enough
/// (`city_unlocal`).
pub fn unlocal(l: [f32; 2], c: &Cell, d: &District) -> [f32; 2] {
    let l = if d.kind == kind::LOW {
        [
            l[0] - 9.0 * (l[1] / 140.0).sin(),
            l[1] - 9.0 * (l[0] / 140.0).sin(),
        ]
    } else {
        l
    };
    let (s, k) = d.turn.sin_cos();
    [
        c.site[0] + l[0] * k - l[1] * s,
        c.site[1] + l[0] * s + l[1] * k,
    ]
}

#[cfg(test)]
/// The lot's place in its block, the street's distance and the block's
/// half inside the streets, for a point `l` in a district's frame: its
/// block, its lot in it, and that lot's middle in the same frame
/// (`city_lot_cell`).
pub fn lot_cell(l: [f32; 2], d: &District) -> ([f32; 2], [f32; 2], [f32; 2], f32, f32) {
    let inner = d.block * 0.5 - d.street - 2.0;
    let n = (2.0 * inner / d.lot).floor().max(1.0);
    let size = 2.0 * inner / n;
    let mut block = [0.0; 2];
    let mut lot = [0.0; 2];
    let mut middle = [0.0; 2];
    let mut street = d.block;
    for a in 0..2 {
        let b = (l[a] / d.block + 0.5).floor();
        let within = l[a] - b * d.block;
        street = street.min(d.block * 0.5 - within.abs());
        let i = ((within + inner) / size).floor().clamp(0.0, n - 1.0);
        block[a] = b;
        lot[a] = i;
        middle[a] = b * d.block - inner + (i + 0.5) * size;
    }
    (block, lot, middle, street, size * 0.5)
}

/// How far a shape reaches across per unit of its mesh's scale, both ways,
/// and how tall that scale stands it (`city_extent`).
pub fn extent(s: u32) -> [f32; 3] {
    match s {
        shape::BLOCK => [0.5, 0.35, 1.0],
        shape::SHED => [0.5, 0.32, 0.3],
        shape::SLAB => [0.106, 0.053, 1.0],
        shape::TAPER => [0.12, 0.12, 1.0],
        shape::ROUND => [0.15, 0.15, 1.0],
        shape::NEEDLE => [0.055, 0.055, 1.0],
        _ => [0.0, 0.0, 1.0],
    }
}

impl City {
    /// The lot in district `d` of cell `c` whose middle is `middle` in the
    /// district's frame (block `block`, lot `lot` in it, `half` its half
    /// size), and what stands on it (`city_lot`).
    pub fn lot(
        &self,
        c: &Cell,
        d: &District,
        block: [f32; 2],
        lot: [f32; 2],
        middle: [f32; 2],
        half: f32,
    ) -> Lot {
        let centre = unlocal(middle, c, d);
        let key = [
            block[0] * 8.0 + lot[0],
            block[1] * 8.0 + lot[1],
            c.id[0] + c.id[1] * CELLS[0],
        ];
        let u = |s: f32| hash_cell([key[0], key[1], key[2] + 3000.0 * s]);
        let mut out = Lot {
            centre,
            key,
            half,
            developed: false,
            built: false,
            height: 0.0,
            shape: shape::HOUSE,
            footprint: [0.0, 0.0],
            tone: u(3.0),
        };
        if d.kind < kind::CORE {
            return out;
        }
        // Built over: dry, dense enough, clear of the roads.
        let here = self.here(centre);
        let n = noise(centre);
        out.developed = !here.wet && here.total - 0.35 + 0.22 * n > 0.0 && here.roads.near > half;
        // A building, wholly in its own cell, clear of the collector
        // streets along the cell's edges, unless the lot stands empty.
        let own = cell_at(centre);
        let vacant = [0.0, 0.0, 0.06, 0.25, 0.12, 0.15][d.kind as usize];
        if !out.developed || own.id != c.id || own.edge < half + d.street || u(0.0) < vacant {
            return out;
        }
        // Lognormal about the district's height for the density.
        let g = (-2.0 * u(1.0).max(1e-4).ln()).sqrt() * (TAU * u(2.0)).cos();
        let k = here.kernel;
        let pick = u(4.0);
        let (mut h, mut s) = match d.kind {
            kind::CORE => ((30.0 + 520.0 * k * k) * (0.45 * g).exp(), shape::TAPER),
            kind::HIGH => ((12.0 + 45.0 * k) * (0.35 * g).exp(), shape::ROUND),
            kind::WORKS => (8.0 + 14.0 * pick, shape::SHED),
            _ => (5.0 + 4.0 * pick, shape::HOUSE),
        };
        h = h.clamp(5.0, 700.0);
        if d.kind == kind::CORE {
            s = if h > 150.0 {
                shape::NEEDLE
            } else if h > 45.0 {
                [shape::TAPER, shape::ROUND, shape::SLAB][((pick * 2.99) as usize).min(2)]
            } else {
                shape::BLOCK
            };
        } else if d.kind == kind::HIGH {
            s = if h > 45.0 {
                if pick < 0.5 {
                    shape::ROUND
                } else {
                    shape::SLAB
                }
            } else {
                shape::BLOCK
            };
        }
        // Within its lot: too broad, it stands lower.
        let e = extent(s);
        let mut footprint = if s == shape::HOUSE {
            [half * 0.7, half * 0.7]
        } else {
            [e[0] * h / e[2], e[1] * h / e[2]]
        };
        let room = half * 0.92;
        let over = footprint[0].max(footprint[1]) / room;
        if over > 1.0 {
            h /= over;
            footprint = [footprint[0] / over, footprint[1] / over];
        }
        out.built = true;
        out.height = h;
        out.shape = s;
        out.footprint = footprint;
        out
    }

    #[cfg(test)]
    /// The lot under `m`, as the map works it out a texel at a time
    /// (`ground_texel`): its cell, district, the lot and the street's
    /// distance; the lot's own whether or not `m` is on it.
    pub fn lot_under(&self, m: [f32; 2]) -> (Cell, District, Lot, [f32; 2], f32) {
        let c = cell_at(m);
        let d = self.district(&c);
        let l = local(m, &c, &d);
        let (block, lot, middle, street, half) = lot_cell(l, &d);
        let out = self.lot(&c, &d, block, lot, middle, half);
        (c, d, out, [l[0] - middle[0], l[1] - middle[1]], street)
    }

    /// Every building tall enough ever to stand as a mesh: those of the
    /// core, the high-rise and the works, 12 m and up.
    pub fn buildings(&self) -> Vec<Building> {
        let mut out = Vec::new();
        let size = [ROUND / CELLS[0], ALONG / CELLS[1]];
        for iz in 0..CELLS[1] as usize {
            for ix in 0..CELLS[0] as usize {
                let id = [ix as f32, iz as f32];
                let j = [
                    hash_cell([id[0], id[1], 61.0]) * 0.7 + 0.15,
                    hash_cell([id[0], id[1], 62.0]) * 0.7 + 0.15,
                ];
                let site = [(id[0] + j[0]) * size[0], (id[1] + j[1]) * size[1]];
                // As the map finds it from a point at its site.
                let c = cell_at(site);
                let d = self.district(&c);
                if !matches!(d.kind, kind::CORE | kind::HIGH | kind::WORKS) {
                    continue;
                }
                let reach = (1.3 * size[0].max(size[1]) / d.block).ceil() as i32;
                let inner = d.block * 0.5 - d.street - 2.0;
                let n = (2.0 * inner / d.lot).floor().max(1.0);
                let lot_size = 2.0 * inner / n;
                for bz in -reach..=reach {
                    for bx in -reach..=reach {
                        for lz in 0..n as i32 {
                            for lx in 0..n as i32 {
                                let block = [bx as f32, bz as f32];
                                let lot = [lx as f32, lz as f32];
                                let middle = [
                                    block[0] * d.block - inner + (lot[0] + 0.5) * lot_size,
                                    block[1] * d.block - inner + (lot[1] + 0.5) * lot_size,
                                ];
                                let l = self.lot(&c, &d, block, lot, middle, lot_size * 0.5);
                                if l.built && l.height >= 12.0 && l.shape != shape::HOUSE {
                                    out.push(Building {
                                        at: [
                                            l.centre[0].rem_euclid(ROUND),
                                            l.centre[1].rem_euclid(ALONG),
                                        ],
                                        height: l.height,
                                        shape: l.shape,
                                        turn: d.turn,
                                        kind: d.kind,
                                        tone: l.tone,
                                    });
                                }
                            }
                        }
                    }
                }
            }
        }
        out
    }
}

/// A building of the city: where (metres), how tall, its shape, the turn of
/// its block, its district and a number of its own.
#[derive(Clone, Copy, Debug)]
pub struct Building {
    pub at: [f32; 2],
    pub height: f32,
    pub shape: u32,
    pub turn: f32,
    pub kind: u32,
    pub tone: f32,
}

#[cfg(test)]
/// Where round the map (0..1) a point across the cylinder lies: `around`
/// in `pane.wesl`.
pub fn around(q: [f32; 2]) -> f32 {
    let s = q[0] / (q[0].abs() + q[1].abs());
    (if q[1] >= 0.0 { 1.0 - s } else { 3.0 + s }) * 0.25
}

#[cfg(test)]
/// The way out from the axis at `u` round the map (`city_outward`), not
/// normalised: the inverse of `around`.
pub fn outward(u: f32) -> [f32; 2] {
    let d = fract(u) * 4.0;
    if d <= 2.0 {
        [1.0 - d, 1.0 - (1.0 - d).abs()]
    } else {
        [d - 3.0, -(1.0 - (d - 3.0).abs())]
    }
}

#[cfg(test)]
/// A texel's place on the ground, metres (`city_metres`).
pub fn metres(texel: [f32; 2], width: f32, height: f32) -> [f32; 2] {
    let q = outward(texel[0] / width);
    [
        fract(q[1].atan2(q[0]) / TAU) * ROUND,
        texel[1] / height * ALONG,
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hash_cell_is_the_shaders() {
        // Worked out in single precision a step at a time, as WGSL does
        // (Python's `struct` rounding each step to f32).
        for (p, want) in [
            ([0.0, 0.0, 0.0], 0.0),
            ([1.0, 2.0, 61.0], 0.541_992),
            ([12.0, 7.0, 3.5], 0.298_828),
            ([65.0, 32.0, 2240.0], 0.237_305),
        ] {
            let got = hash_cell(p);
            assert!((got - want).abs() < 2e-5, "{p:?}: {got} not {want}");
        }
    }

    #[test]
    fn the_noise_tiles_both_ways() {
        for m in [[100.0, 200.0], [9000.0, 1.0], [25_000.0, 13_000.0]] {
            for (a, b) in [(m, [m[0] + ROUND, m[1]]), (m, [m[0], m[1] + ALONG])] {
                assert!((fbm(a, 3.0, 5, 1.0) - fbm(b, 3.0, 5, 1.0)).abs() < 2e-3);
                assert!((noise(a) - noise(b)).abs() < 2e-3);
                assert!((river(a) - river(b)).abs() < 0.5);
            }
        }
    }

    #[test]
    fn metres_round_are_the_true_angle() {
        // A point on the ground straight out from the axis at angle θ is
        // painted at the texel `around` gives, which is `R·θ` metres round.
        for theta in [0.1f32, 1.0, 2.5, 3.3, 4.7, 6.1] {
            let u = around([theta.cos(), theta.sin()]);
            let m = metres([u * 2048.0, 0.0], 2048.0, 1024.0);
            assert!((m[0] - theta * GROUND).abs() < 0.5, "{theta}: {}", m[0]);
        }
        // And `outward` undoes `around`.
        for u in [0.05f32, 0.2, 0.4, 0.55, 0.7, 0.93] {
            assert!((around(outward(u)) - u).abs() < 1e-4);
        }
        // The suns where the map puts them.
        for (k, s) in SUNS.iter().enumerate() {
            let m = metres([(k as f32 * 2.0 + 1.0) / 6.0 * 2048.0, 0.0], 2048.0, 1024.0);
            assert!((m[0] - s).abs() < 0.5, "{k}: {}", m[0]);
        }
    }

    #[test]
    fn towns_are_ranked_and_joined() {
        let c = city();
        assert!(c.kernels.len() > 60, "{} kernels", c.kernels.len());
        assert!(c.kernels[0][2] > 1500.0);
        assert!(c.segments.len() > 100 && c.segments.len() <= MAX_SEGMENTS);
        // Every piece is listed in the square it starts in.
        for (k, s) in c.segments.iter().enumerate() {
            let gx = (s[0] / ROUND * GRID[0] as f32).floor() as usize;
            let gz = (s[1] / ALONG * GRID[1] as f32).floor() as usize;
            let cell = c.cells[gz.min(GRID[1] - 1) * GRID[0] + gx.min(GRID[0] - 1)];
            assert!(c.list[cell[0] as usize..(cell[0] + cell[1]) as usize].contains(&(k as u32)));
        }
        // A road is found on itself.
        let s = c.segments[5];
        let r = c.roads_at([(s[0] + s[2]) * 0.5, (s[1] + s[3]) * 0.5]);
        assert!(r.near < 0.0, "{r:?}");
    }

    #[test]
    fn the_largest_town_is_a_city() {
        let c = city();
        let t = c.kernels[0];
        let middle = c.here([t[0], t[1]]);
        assert!(middle.total > 0.9 && !middle.wet, "{middle:?}");
        let cell = cell_at([t[0], t[1]]);
        assert_eq!(c.district(&cell).kind, kind::CORE);
        // Far out in the country, nothing.
        let mut rural = 0;
        for k in 0..400 {
            let m = [
                hash_cell([k as f32, 1.0, 5.0]) * ROUND,
                hash_cell([k as f32, 2.0, 5.0]) * ALONG,
            ];
            if c.district(&cell_at(m)).kind == kind::RURAL {
                rural += 1;
            }
        }
        assert!(rural > 150 && rural < 380, "{rural} of 400 rural");
    }

    #[test]
    fn each_building_stands_on_the_lot_the_map_paints() {
        // The map finds a lot from the texel it paints; the buildings are
        // listed from the cells' own grids. Every building is found again
        // from a point on it, the same lot with the same building.
        let c = city();
        let buildings = c.buildings();
        assert!(
            buildings.len() > 3000 && buildings.len() < 60_000,
            "{}",
            buildings.len()
        );
        eprintln!(
            "{} buildings, {} kernels, {} road pieces, {} listed",
            buildings.len(),
            c.kernels.len(),
            c.segments.len(),
            c.list.len()
        );
        let mut tall = 0;
        for b in buildings.iter().step_by(7) {
            let (_, d, lot, off, _) = c.lot_under(b.at);
            assert!(lot.built, "{b:?}");
            assert!(
                (lot.height - b.height).abs() < 0.5 && lot.shape == b.shape,
                "{b:?} {lot:?}"
            );
            assert!(
                length(off) < 0.5 && (d.turn - b.turn).abs() < 5e-3,
                "{b:?} {off:?}"
            );
            assert!(b.height < 710.0);
            if b.height > 150.0 {
                tall += 1;
            }
        }
        assert!(tall > 5, "{tall} towers over 150 m");
    }
}

#[cfg(test)]
mod picture {
    use super::*;

    /// A picture of a tile of the city from above, the Rust side's view:
    /// `SOLAR_SHOTS_DIR/city-map.png`.
    #[test]
    #[ignore = "writes a picture"]
    fn city_map() {
        let c = city();
        let (w, h) = (1024u32, 512u32);
        let mut img = image::RgbImage::new(w, h);
        for y in 0..h {
            for x in 0..w {
                let m = [
                    (x as f32 + 0.5) / w as f32 * ROUND,
                    (y as f32 + 0.5) / h as f32 * ALONG,
                ];
                let here = c.here(m);
                let cell = cell_at(m);
                let d = c.district(&cell);
                let mut px = match d.kind {
                    kind::RURAL => [40, 90, 40],
                    kind::PARK => [60, 160, 60],
                    kind::CORE => [230, 230, 240],
                    kind::WORKS => [150, 120, 90],
                    kind::HIGH => [180, 170, 150],
                    _ => [140, 120, 100],
                };
                if here.wet {
                    px = [30, 60, 140];
                }
                if here.roads.near < 15.0 {
                    px = [250, 200, 80];
                }
                img.put_pixel(x, y, image::Rgb(px));
            }
        }
        for b in c.buildings() {
            let x = (b.at[0] / ROUND * w as f32) as u32;
            let y = (b.at[1] / ALONG * h as f32) as u32;
            let v = (60.0 + b.height).min(255.0) as u8;
            img.put_pixel(x.min(w - 1), y.min(h - 1), image::Rgb([v, 0, v / 2]));
        }
        let dir = std::env::var("SOLAR_SHOTS_DIR").unwrap_or_else(|_| "target/solar-shots".into());
        img.save(std::path::Path::new(&dir).join("city-map.png"))
            .unwrap();
    }
}
