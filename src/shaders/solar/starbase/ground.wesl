// The ground of the world inside the base, painted a texel at a time
// (`world.wesl` bakes it): hills, rivers, lakes and the sea, fields in
// parcels, and the cities `city.wesl` works out, each lot's roof painted
// where its building stands so that one dropping out with distance leaves
// its roof behind; and its relief, levelled where it is built over, the
// roads in it for their traffic.

import package::solar::noise::TAU;
import package::solar::frame::hash_cell;
import package::solar::starbase::city::{
    CITY_ALONG, CITY_CORE, CITY_HIGH, CITY_LAKE, CITY_LOW, CITY_PARK, CITY_ROUND, CITY_WORKS,
    SHAPE_NEEDLE, SHAPE_ROUND, SHAPE_TAPER, city_cells, city_district, city_fbm, city_height,
    city_here, city_local, city_lot, city_lot_cell, city_metres, city_noise, city_outward, city_river,
    city_roads, city_sea, city_wrap1,
};

// How near texel `x` round is to one of the three strip suns, in texels,
// and which side of it (-1 or 1).
fn ground_sun(x: f32, width: f32) -> vec2<f32> {
    let u = fract(x / width * 3.0) - 0.5;
    return vec2<f32>(abs(u) * width / 3.0, sign(u));
}

// How much of a texel `w` metres across lies where `d` is under 0.
fn ground_cover(d: f32, w: f32) -> f32 {
    return 1.0 - smoothstep(-0.5 * w, 0.5 * w, d);
}

// How far a texel at `x` reaches on the ground, metres: 8 to 16 round (the
// map's coordinate round is not an angle: `around` turns `dθ` into
// `du = dθ / (4 (|cos θ| + |sin θ|)²)`, and the diamond point `q` has
// `|cos θ| + |sin θ| = 1 / |q|`), 13 along.
fn ground_width(x: vec2<f32>, size: vec2<f32>) -> f32 {
    let q = city_outward(x.x / size.x);
    return max(4.0 * CITY_ROUND / TAU / (size.x * dot(q, q)), CITY_ALONG / size.y);
}

// The ground at texel `x` of a map `size` texels: its colour by day, and
// how built over it is (for the fine blocks laid over it).
fn ground_colour(x: vec2<f32>, size: vec2<f32>) -> vec4<f32> {
    let m = city_metres(x, size);
    // The district, its block grid and the lot here, first: the rest is
    // worked out after, so little stays live across it (the V3D spills).
    let c = city_cells(m);
    let d = city_district(c);
    let l = city_local(m, c, d);
    let lc = city_lot_cell(l, d);
    let lot = city_lot(c, d, lc);
    let off = l - lc.middle;
    let w = ground_width(x, size);
    let sun = ground_sun(x.x, size.x);
    let h = city_height(m);
    let detail = 0.05 * city_fbm(m, 96.0, 2, 17.0);
    let lit = m - vec2<f32>(sun.y * 30.0, 0.0);
    let toward = city_height(lit) + 0.05 * city_fbm(lit, 96.0, 2, 17.0);
    var relief = clamp(0.75 + (toward - h - detail) * 16.0, 0.25, 1.6);
    let water = max(max(ground_cover(city_river(m), w), ground_cover(city_sea(m, h), w)), 1.0 - smoothstep(CITY_LAKE - 0.01, CITY_LAKE + 0.01, h));
    // The country: fields in parcels, a hedge or lane between, woods and
    // rock on the heights.
    let parcel = hash_cell(vec3<f32>(lc.block * 4.0 + lc.lot, c.id.x + c.id.y * 66.0 + 0.5));
    let fields = mix(vec3<f32>(0.04, 0.12, 0.045), vec3<f32>(0.15, 0.13, 0.05), step(0.55, parcel)) * (0.7 + 0.6 * fract(parcel * 7.0));
    let wild = mix(vec3<f32>(0.025, 0.075, 0.035), vec3<f32>(0.11, 0.095, 0.08), smoothstep(0.1, 0.35, h));
    var colour = mix(fields, wild, smoothstep(-0.02, 0.12, h + detail));
    colour *= 1.0 - 0.3 * ground_cover(lc.street - 1.5, w);
    if (d.kind == CITY_PARK) {
        colour = mix(vec3<f32>(0.035, 0.095, 0.04) * (0.8 + 0.4 * parcel), vec3<f32>(0.09, 0.09, 0.08), ground_cover(lc.street - d.street, w));
    }
    var built = 0.0;
    if (lot.developed) {
        // Paved, each district its own cast, the suburbs green between
        // their houses.
        var hue = vec3<f32>(1.05, 1.0, 0.94);
        if (d.kind == CITY_CORE) {
            hue = vec3<f32>(0.95, 0.98, 1.05);
        } else if (d.kind == CITY_WORKS) {
            hue = vec3<f32>(0.9, 0.9, 0.92);
        } else if (d.kind == CITY_HIGH) {
            hue = vec3<f32>(1.02, 1.0, 0.97);
        }
        var urban = vec3<f32>(0.055, 0.058, 0.066) * hue * (0.85 + 0.3 * lot.tone);
        if (d.kind == CITY_LOW) {
            urban = mix(urban, vec3<f32>(0.05, 0.085, 0.045), 0.55);
        }
        // The building's roof, cooler and darker than its walls, a round
        // tower's round; a dark footprint round the tall.
        var reach = abs(off) - lot.footprint;
        var out = max(reach.x, reach.y);
        if (lot.shape == SHAPE_TAPER || lot.shape == SHAPE_ROUND || lot.shape == SHAPE_NEEDLE) {
            out = length(off) - lot.footprint.x;
        }
        let roof = ground_cover(out, w) * select(0.0, 1.0, lot.built);
        let tall = smoothstep(30.0, 120.0, lot.height) * select(0.0, 1.0, lot.built);
        urban *= 1.0 - 0.4 * tall * ground_cover(out - lot.footprint.x * 0.8, w);
        var top = vec3<f32>(0.1, 0.105, 0.12);
        if (d.kind == CITY_WORKS) {
            top = vec3<f32>(0.12, 0.12, 0.125);
        } else if (d.kind == CITY_LOW) {
            top = vec3<f32>(0.12, 0.08, 0.068);
        }
        urban = mix(urban, top * (0.7 + 0.6 * lot.tone), roof);
        // Lit: the windows and yards between the buildings, a speckle of
        // them, thickest in the core; its streets, and the collector
        // streets along the district's edges, lamp-lit, brightest there.
        let lamps = select(select(0.07, 0.11, d.kind == CITY_HIGH), 0.17, d.kind == CITY_CORE);
        let speck = step(1.0 - 1.6 * lamps, hash_cell(vec3<f32>(floor(l / 9.0), lot.tone * 37.0)));
        urban += vec3<f32>(1.0, 0.8, 0.52) * speck * lamps * 0.35 * (1.0 - roof);
        let street = max(ground_cover(lc.street - d.street, w), ground_cover(c.edge - 7.0, w));
        urban = mix(urban, vec3<f32>(0.022, 0.023, 0.028) + vec3<f32>(1.0, 0.76, 0.46) * lamps, street);
        colour = urban;
        relief = 1.0;
        built = 0.5 + 0.5 * roof;
    }
    let day = 0.45 + 0.75 * exp(-sun.x / 150.0);
    colour *= relief * day;
    colour = mix(colour, vec3<f32>(0.015, 0.05, 0.09) * day + vec3<f32>(0.3, 0.33, 0.36) * max(day - 0.7, 0.0), water);
    // The arterials and highways over all, bridges over the water; the
    // highways lit.
    let roads = city_roads(m);
    let road = ground_cover(roads.near, w);
    let highway = ground_cover(roads.highway, w);
    colour = mix(colour, vec3<f32>(0.04, 0.04, 0.045) * day, road) + vec3<f32>(1.0, 0.76, 0.46) * (highway * 0.3 + road * 0.08);
    // The suns: a white strip, the trench it lies in aglow.
    colour = mix(colour, vec3<f32>(2.0, 1.95, 1.8), 1.0 - smoothstep(3.0, 5.0, sun.x));
    colour += vec3<f32>(0.5, 0.48, 0.42) * exp(-sun.x / 22.0) * 0.6;
    return vec4<f32>(colour, built * (1.0 - water) * smoothstep(8.0, 20.0, sun.x));
}

// The ground's relief at texel `x`: how high it stands (0 the lowlands and
// the water, 1 the peaks), level where it is built over, so the buildings
// stand on the ground the map draws; how much of a road is here, for its
// traffic; and a number that runs along the roads, for the lights on them.
fn ground_relief(x: vec2<f32>, size: vec2<f32>) -> vec4<f32> {
    let m = city_metres(x, size);
    let w = ground_width(x, size);
    let here = city_here(m);
    let urban = smoothstep(-0.12, 0.02, here.total - 0.35 + 0.22 * city_noise(m));
    let traffic = max(ground_cover(here.roads.highway, w), 0.6 * ground_cover(here.roads.near, w));
    // Low at the ends of the length, where the bulkheads stand over the
    // seams between lengths laid as they are and mirrored (`sections`).
    let seam = smoothstep(150.0, 1500.0, min(m.y, CITY_ALONG - m.y));
    return vec4<f32>(smoothstep(-0.05, 0.5, here.height) * (1.0 - urban) * seam, traffic, 0.5 + 0.5 * sin(here.roads.along * TAU / 1500.0), 1.0);
}
