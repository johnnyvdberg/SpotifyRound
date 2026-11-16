// The cities of the world inside the base, as `starbase::city` works them
// out, line for line: the map (`world.wesl`) paints them a texel at a time,
// and the meshes of their buildings stand on the lots painted here. All in
// ground metres: round the cylinder (R·θ, θ its true angle) and along it,
// tiling every `CITY_ROUND` and `CITY_ALONG`. The towns' kernels and the
// roads come from a table built once (`City::table`), read by the bake
// alone.

import package::solar::noise::TAU;
import package::solar::frame::hash_cell;

const CITY_ROUND: f32 = 26389.379;
const CITY_ALONG: f32 = 13194.0;
const CITY_REACH: f32 = 700.0;
const CITY_GRID: vec2<f32> = vec2<f32>(24.0, 12.0);
const CITY_CELLS: vec2<f32> = vec2<f32>(66.0, 33.0);
const CITY_LAKE: f32 = -0.27;
const CITY_RIVER_HALF: f32 = 26.0;

// A district's kinds and a building's shapes (`city::kind`, `city::shape`).
const CITY_RURAL: u32 = 0u;
const CITY_PARK: u32 = 1u;
const CITY_CORE: u32 = 2u;
const CITY_WORKS: u32 = 3u;
const CITY_HIGH: u32 = 4u;
const CITY_LOW: u32 = 5u;
const SHAPE_BLOCK: u32 = 0u;
const SHAPE_SHED: u32 = 1u;
const SHAPE_SLAB: u32 = 2u;
const SHAPE_TAPER: u32 = 3u;
const SHAPE_ROUND: u32 = 4u;
const SHAPE_NEEDLE: u32 = 5u;
const SHAPE_HOUSE: u32 = 6u;

struct CityTable {
    // How many kernels and road pieces.
    counts: vec4<u32>,
    // A town's kernel: where, b, how strong.
    kernels: array<vec4<f32>, 160>,
    // A road piece's ends; its half width, highway or not, how far along
    // the network at its start, and its length.
    segments: array<vec4<f32>, 1024>,
    roads: array<vec4<f32>, 1024>,
    // A square of the grid: its first entry in `list` and how many.
    cells: array<vec4<u32>, 288>,
    // A district cell's kind and its block grid's turn, classed once.
    districts: array<vec4<f32>, 2178>,
    list: array<u32>,
};
@group(3) @binding(0) var<storage, read> city_table: CityTable;

// One of six by a district's kind: no arrays indexed at run time, which
// the V3D keeps in memory.
fn city_by_kind(k: u32, a: vec3<f32>, b: vec3<f32>) -> f32 {
    return select(select(select(b.z, b.y, k == 4u), b.x, k == 3u), select(select(a.z, a.y, k == 1u), a.x, k == 0u), k < 3u);
}

fn city_wrap1(d: f32, p: f32) -> f32 {
    return d - p * floor(d / p + 0.5);
}

fn city_wrap(d: vec2<f32>) -> vec2<f32> {
    return vec2<f32>(city_wrap1(d.x, CITY_ROUND), city_wrap1(d.y, CITY_ALONG));
}

// Value noise tiling the map, `n` cells along and twice that round, 0..1.
fn city_value(m: vec2<f32>, n: f32, seed: f32) -> f32 {
    let g = vec2<f32>(m.x / CITY_ROUND * (2.0 * n), m.y / CITY_ALONG * n);
    let i = floor(g);
    let f = g - i;
    let u = f * f * (3.0 - 2.0 * f);
    let period = vec2<f32>(2.0 * n, n);
    let c0 = i - floor(i / period) * period;
    let c1 = i + 1.0 - floor((i + 1.0) / period) * period;
    let v00 = hash_cell(vec3<f32>(c0.x, c0.y, seed));
    let v10 = hash_cell(vec3<f32>(c1.x, c0.y, seed));
    let v01 = hash_cell(vec3<f32>(c0.x, c1.y, seed));
    let v11 = hash_cell(vec3<f32>(c1.x, c1.y, seed));
    return mix(mix(v00, v10, u.x), mix(v01, v11, u.x), u.y);
}

fn city_fbm(m: vec2<f32>, n: f32, octaves: i32, seed: f32) -> f32 {
    var sum = 0.0;
    var amplitude = 0.5;
    var c = n;
    for (var i = 0; i < octaves; i++) {
        sum += amplitude * (city_value(m, c, seed + f32(i)) * 2.0 - 1.0);
        c *= 2.0;
        amplitude *= 0.5;
    }
    return sum;
}

fn city_height(m: vec2<f32>) -> f32 {
    return 0.75 * city_fbm(m, 3.0, 5, 1.0);
}

fn city_noise(m: vec2<f32>) -> f32 {
    return 1.6 * city_fbm(m, 16.0, 3, 9.0);
}

// One river: its middle and its waves' sizes and phases.
fn city_river_one(m: vec2<f32>, w: f32, a: vec4<f32>, b: vec3<f32>) -> f32 {
    let x = a.x + a.y * sin(w + a.z) + a.w * sin(2.0 * w + b.x) + b.y * sin(3.0 * w + b.z);
    let slope = (a.y * cos(w + a.z) + 2.0 * a.w * cos(2.0 * w + b.x) + 3.0 * b.y * cos(3.0 * w + b.z)) * (TAU / CITY_ALONG);
    return abs(city_wrap1(m.x - x, CITY_ROUND)) / sqrt(1.0 + slope * slope);
}

// How far from the nearest river's bank (less than 0 in it).
fn city_river(m: vec2<f32>) -> f32 {
    let w = m.y / CITY_ALONG * TAU;
    var best = city_river_one(m, w, vec4<f32>(0.0, 700.0, 0.3, 300.0), vec3<f32>(2.1, 140.0, 4.0));
    best = min(best, city_river_one(m, w, vec4<f32>(8920.0, 650.0, 1.7, 350.0), vec3<f32>(5.2, 120.0, 0.9)));
    best = min(best, city_river_one(m, w, vec4<f32>(17470.0, 750.0, 3.9, 280.0), vec3<f32>(0.4, 150.0, 2.8)));
    return best - CITY_RIVER_HALF;
}

fn city_sea(m: vec2<f32>, h: f32) -> f32 {
    return abs(m.y - CITY_ALONG * 0.5) - 520.0 - 3350.0 * h;
}

fn city_sun_distance(x: f32) -> f32 {
    let a = abs(city_wrap1(x - 4650.0246, CITY_ROUND));
    let b = abs(city_wrap1(x - 13194.689, CITY_ROUND));
    let c = abs(city_wrap1(x - 21739.354, CITY_ROUND));
    return min(a, min(b, c));
}

// How far `p` is from piece `s`, and how far along it the nearest point.
fn city_segment(p: vec2<f32>, s: vec4<f32>) -> vec2<f32> {
    let d = city_wrap(p - s.xy);
    let e = s.zw - s.xy;
    let t = clamp(dot(d, e) / max(dot(e, e), 1e-3), 0.0, 1.0);
    return vec2<f32>(length(d - e * t), t);
}

struct CityRoads {
    near: f32,
    highway: f32,
    way: vec2<f32>,
    along: f32,
};

fn city_roads(m: vec2<f32>) -> CityRoads {
    let g = vec2<u32>(min(floor((m - vec2<f32>(CITY_ROUND, CITY_ALONG) * floor(m / vec2<f32>(CITY_ROUND, CITY_ALONG))) / vec2<f32>(CITY_ROUND, CITY_ALONG) * CITY_GRID), CITY_GRID - 1.0));
    let cell = city_table.cells[g.y * 24u + g.x];
    var out = CityRoads(CITY_REACH, CITY_REACH, vec2<f32>(1.0, 0.0), 0.0);
    for (var k = cell.x; k < cell.x + cell.y; k++) {
        let i = city_table.list[k];
        let s = city_table.segments[i];
        let r = city_table.roads[i];
        let dt = city_segment(m, s);
        let edge = dt.x - r.x;
        if (edge < out.near) {
            out.near = edge;
            out.way = (s.zw - s.xy) / max(r.w, 1e-3);
            out.along = r.z + dt.y * r.w;
        }
        if (r.y > 0.5) {
            out.highway = min(out.highway, edge);
        }
    }
    return out;
}

struct CityHere {
    kernel: f32,
    total: f32,
    height: f32,
    roads: CityRoads,
    water: f32,
    wet: bool,
};

fn city_here(m: vec2<f32>) -> CityHere {
    let h = city_height(m);
    let roads = city_roads(m);
    var kernel = 0.0;
    for (var k = 0u; k < city_table.counts.x; k++) {
        let c = city_table.kernels[k];
        let r = length(city_wrap(m - c.xy));
        if (r < 8.0 * c.z) {
            kernel += c.w * exp(-r / c.z);
        }
    }
    let water = min(min(city_river(m), city_sea(m, h)), (h - CITY_LAKE) * 3000.0);
    var total = kernel
        + 0.28 * exp(-max(roads.highway, 0.0) / 180.0)
        + 0.18 * exp(-max(roads.near, 0.0) / 120.0)
        + 0.16 * exp(-max(water, 0.0) / 220.0)
        - 1.3 * max(h - 0.12, 0.0);
    total *= smoothstep(250.0, 700.0, city_sun_distance(m.x));
    return CityHere(kernel, total, h, roads, water, water < 0.0);
}

struct CityCell {
    site: vec2<f32>,
    id: vec2<f32>,
    edge: f32,
};

fn city_cells(m: vec2<f32>) -> CityCell {
    let size = vec2<f32>(CITY_ROUND, CITY_ALONG) / CITY_CELLS;
    let g = vec2<f32>(m.x / CITY_ROUND * CITY_CELLS.x, m.y / CITY_ALONG * CITY_CELLS.y);
    let cell = floor(g);
    let f = g - cell;
    var first = 1e9;
    var second = 1e9;
    var out = CityCell(m, vec2<f32>(0.0), 0.0);
    for (var k = 0; k < 9; k++) {
        let o = vec2<f32>(f32(k % 3) - 1.0, f32(k / 3) - 1.0);
        let c = cell + o;
        let id = c - floor(c / CITY_CELLS) * CITY_CELLS;
        let j = vec2<f32>(hash_cell(vec3<f32>(id, 61.0)), hash_cell(vec3<f32>(id, 62.0))) * 0.7 + 0.15;
        let r = (o + j - f) * size;
        let d = length(r);
        if (d < first) {
            second = first;
            first = d;
            out.site = m + r;
            out.id = id;
        } else if (d < second) {
            second = d;
        }
    }
    out.edge = (second - first) * 0.5;
    return out;
}

struct CityDistrict {
    kind: u32,
    turn: f32,
    block: f32,
    lot: f32,
    street: f32,
};

fn city_district(c: CityCell) -> CityDistrict {
    let row = city_table.districts[u32(c.id.x) + u32(c.id.y) * 66u];
    let kind = u32(row.x + 0.5);
    let turn = row.y;
    let block = city_by_kind(kind, vec3<f32>(170.0, 120.0, 110.0), vec3<f32>(190.0, 120.0, 145.0));
    let lot = city_by_kind(kind, vec3<f32>(80.0, 60.0, 45.0), vec3<f32>(80.0, 38.0, 28.0));
    let street = city_by_kind(kind, vec3<f32>(2.0, 3.0, 7.0), vec3<f32>(7.0, 6.0, 5.0));
    return CityDistrict(kind, turn, block, lot, street);
}

fn city_local(m: vec2<f32>, c: CityCell, d: CityDistrict) -> vec2<f32> {
    let r = city_wrap(m - c.site);
    let s = sin(d.turn);
    let k = cos(d.turn);
    let l = vec2<f32>(r.x * k + r.y * s, -r.x * s + r.y * k);
    if (d.kind == CITY_LOW) {
        return vec2<f32>(l.x + 9.0 * sin(l.y / 140.0), l.y + 9.0 * sin(l.x / 140.0));
    }
    return l;
}

fn city_unlocal(w: vec2<f32>, c: CityCell, d: CityDistrict) -> vec2<f32> {
    var l = w;
    if (d.kind == CITY_LOW) {
        l = vec2<f32>(w.x - 9.0 * sin(w.y / 140.0), w.y - 9.0 * sin(w.x / 140.0));
    }
    let s = sin(d.turn);
    let k = cos(d.turn);
    return vec2<f32>(c.site.x + l.x * k - l.y * s, c.site.y + l.x * s + l.y * k);
}

struct CityLotCell {
    block: vec2<f32>,
    lot: vec2<f32>,
    middle: vec2<f32>,
    street: f32,
    half: f32,
};

fn city_lot_cell(l: vec2<f32>, d: CityDistrict) -> CityLotCell {
    let inner = d.block * 0.5 - d.street - 2.0;
    let n = max(floor(2.0 * inner / d.lot), 1.0);
    let size = 2.0 * inner / n;
    let block = floor(l / d.block + 0.5);
    let within = l - block * d.block;
    let street = min(d.block * 0.5 - abs(within.x), d.block * 0.5 - abs(within.y));
    let i = clamp(floor((within + inner) / size), vec2<f32>(0.0), vec2<f32>(n - 1.0));
    return CityLotCell(block, i, block * d.block - inner + (i + 0.5) * size, min(street, d.block), size * 0.5);
}

fn city_extent(s: u32) -> vec3<f32> {
    var e = vec3<f32>(0.0, 0.0, 1.0);
    if (s == SHAPE_BLOCK) {
        e = vec3<f32>(0.5, 0.35, 1.0);
    } else if (s == SHAPE_SHED) {
        e = vec3<f32>(0.5, 0.32, 0.3);
    } else if (s == SHAPE_SLAB) {
        e = vec3<f32>(0.106, 0.053, 1.0);
    } else if (s == SHAPE_TAPER) {
        e = vec3<f32>(0.12, 0.12, 1.0);
    } else if (s == SHAPE_ROUND) {
        e = vec3<f32>(0.15, 0.15, 1.0);
    } else if (s == SHAPE_NEEDLE) {
        e = vec3<f32>(0.055, 0.055, 1.0);
    }
    return e;
}

struct CityLot {
    centre: vec2<f32>,
    developed: bool,
    built: bool,
    height: f32,
    shape: u32,
    footprint: vec2<f32>,
    tone: f32,
};

fn city_lot_hash(key: vec3<f32>, s: f32) -> f32 {
    return hash_cell(vec3<f32>(key.x, key.y, key.z + 3000.0 * s));
}

fn city_lot(c: CityCell, d: CityDistrict, lc: CityLotCell) -> CityLot {
    let centre = city_unlocal(lc.middle, c, d);
    let key = vec3<f32>(lc.block.x * 8.0 + lc.lot.x, lc.block.y * 8.0 + lc.lot.y, c.id.x + c.id.y * CITY_CELLS.x);
    var out = CityLot(centre, false, false, 0.0, SHAPE_HOUSE, vec2<f32>(0.0), city_lot_hash(key, 3.0));
    if (d.kind < CITY_CORE) {
        return out;
    }
    let half = lc.half;
    let here = city_here(centre);
    let n = city_noise(centre);
    out.developed = !here.wet && here.total - 0.35 + 0.22 * n > 0.0 && here.roads.near > half;
    let own = city_cells(centre);
    let vacancy = city_by_kind(d.kind, vec3<f32>(0.0, 0.0, 0.06), vec3<f32>(0.25, 0.12, 0.15));
    if (!out.developed || any(own.id != c.id) || own.edge < half + d.street || city_lot_hash(key, 0.0) < vacancy) {
        return out;
    }
    let g = sqrt(-2.0 * log(max(city_lot_hash(key, 1.0), 1e-4))) * cos(TAU * city_lot_hash(key, 2.0));
    let k = here.kernel;
    let pick = city_lot_hash(key, 4.0);
    var h = 5.0 + 4.0 * pick;
    var s = SHAPE_HOUSE;
    if (d.kind == CITY_CORE) {
        h = (22.0 + 300.0 * k * k) * exp(0.5 * g);
    } else if (d.kind == CITY_HIGH) {
        h = (12.0 + 45.0 * k) * exp(0.35 * g);
    } else if (d.kind == CITY_WORKS) {
        h = 8.0 + 14.0 * pick;
        s = SHAPE_SHED;
    }
    h = clamp(h, 5.0, 460.0);
    if (d.kind == CITY_CORE) {
        if (h > 150.0) {
            s = SHAPE_NEEDLE;
        } else if (h > 45.0) {
            // Taper, round, slab, as `city::lot` picks them.
            let p = min(u32(pick * 2.99), 2u);
            s = select(select(SHAPE_SLAB, SHAPE_ROUND, p == 1u), SHAPE_TAPER, p == 0u);
        } else {
            s = SHAPE_BLOCK;
        }
    } else if (d.kind == CITY_HIGH) {
        s = select(SHAPE_BLOCK, select(SHAPE_SLAB, SHAPE_ROUND, pick < 0.5), h > 45.0);
    }
    let e = city_extent(s);
    var footprint = vec2<f32>(half * 0.7);
    if (s != SHAPE_HOUSE) {
        footprint = e.xy * h / e.z;
    }
    let over = max(footprint.x, footprint.y) / (half * 0.92);
    if (over > 1.0) {
        h /= over;
        footprint /= over;
    }
    out.built = true;
    out.height = h;
    out.shape = s;
    out.footprint = footprint;
    return out;
}

// The way out from the axis at `u` round the map: the inverse of `around`
// (`pane.wesl`), not normalised.
fn city_outward(u: f32) -> vec2<f32> {
    let d = fract(u) * 4.0;
    if (d <= 2.0) {
        return vec2<f32>(1.0 - d, 1.0 - abs(1.0 - d));
    }
    return vec2<f32>(d - 3.0, -(1.0 - abs(d - 3.0)));
}

// A texel's place on the ground, metres.
fn city_metres(x: vec2<f32>, size: vec2<f32>) -> vec2<f32> {
    let q = city_outward(x.x / size.x);
    return vec2<f32>(fract(atan2(q.y, q.x) / TAU) * CITY_ROUND, x.y / size.y * CITY_ALONG);
}
