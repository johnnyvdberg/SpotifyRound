// The solar tour's map generator: every kind of planet and moon, painted
// into equirectangular maps from 3D noise on the sphere. Prepended with
// `noise.wgsl`.
//
// `fs_height` paints elevations; `fs_planet` and `fs_moon` paint the maps
// from them; `fs_sky` paints the system's sky (the galactic band).

struct Gen {
    // x, y: target size in texels; z: sea level; w: the kind.
    size: vec4<f32>,
    offsets: array<vec4<f32>, 4>,
    plates: array<vec4<f32>, 12>,
    storms: array<vec4<f32>, 12>,
    palette: array<vec4<f32>, 4>,
    // x: clouds, y: cities, z: unused, w: warmth (+ hotter, - colder).
    flags: vec4<f32>,
};

@group(0) @binding(0) var<uniform> gen: Gen;
@group(0) @binding(1) var terrain: texture_2d<f32>;
@group(0) @binding(2) var terrain_sampler: sampler;

const TERRAN: i32 = 0;
const DESERT: i32 = 1;
const OCEAN: i32 = 2;
const ICE: i32 = 3;
const LAVA: i32 = 4;
const BARREN: i32 = 5;
const GAS: i32 = 6;
const ICE_GIANT: i32 = 7;
const VOLCANIC: i32 = 8;
const ICY_MOON: i32 = 9;
const HAZY: i32 = 10;
const CAPTURED: i32 = 11;
const CLOUDY: i32 = 12;
const EYEBALL: i32 = 13;
const HOT: i32 = 14;
const JUNGLE: i32 = 15;
const GEYSER: i32 = 16;
const TWO_FACED: i32 = 17;
const NITROGEN: i32 = 18;
const RUSTY: i32 = 19;

fn kind() -> i32 {
    return i32(gen.size.w + 0.5);
}

struct Varying {
    @builtin(position) position: vec4<f32>,
};

@vertex
fn vs_main(@builtin(vertex_index) index: u32) -> Varying {
    var corners = array<vec2<f32>, 3>(
        vec2<f32>(-1.0, -1.0),
        vec2<f32>(3.0, -1.0),
        vec2<f32>(-1.0, 3.0),
    );
    var out: Varying;
    out.position = vec4<f32>(corners[index], 0.0, 1.0);
    return out;
}

fn sphere_at(uv: vec2<f32>) -> vec3<f32> {
    let lon = (uv.x - 0.5) * TAU;
    let lat = (0.5 - uv.y) * PI;
    return vec3<f32>(cos(lat) * sin(lon), sin(lat), cos(lat) * cos(lon));
}

fn ridged(p: vec3<f32>, octaves: i32) -> f32 {
    var q = p;
    var sum = 0.0;
    var amplitude = 0.5;
    var weight = 1.0;
    for (var i = 0; i < octaves; i++) {
        var r = 1.0 - abs(noise(q));
        r = r * r * weight;
        weight = clamp(r * 1.6, 0.0, 1.0);
        sum += amplitude * r;
        q = OCTAVE * q * 2.07;
        amplitude *= 0.5;
    }
    return sum;
}

fn eroded(p: vec3<f32>, octaves: i32) -> f32 {
    var q = p;
    var sum = 0.0;
    var amplitude = 0.5;
    var slope = vec3<f32>(0.0);
    var frequency = 1.0;
    var turn = mat3x3<f32>(vec3<f32>(1.0, 0.0, 0.0), vec3<f32>(0.0, 1.0, 0.0), vec3<f32>(0.0, 0.0, 1.0));
    for (var i = 0; i < octaves; i++) {
        let n = noised(q);
        slope += (transpose(turn) * n.yzw) * frequency * amplitude;
        sum += amplitude * n.x / (1.0 + dot(slope, slope) * 0.35);
        q = OCTAVE * q * 2.01;
        turn = OCTAVE * turn;
        frequency *= 2.01;
        amplitude *= 0.5;
    }
    return sum;
}

// Craters at `scale` cells per radius: bowls with raised rims, kept with
// probability `density`. Returns the height change and the bright fresh
// ejecta around the younger ones.
fn craters(p: vec3<f32>, scale: f32, density: f32, seed: i32) -> vec2<f32> {
    let x = p * scale;
    let cell = vec3<i32>(floor(x));
    var h = 0.0;
    var rays = 0.0;
    for (var k = 0; k < 27; k++) {
        let id = cell + vec3<i32>(k % 3 - 1, (k / 3) % 3 - 1, k / 9 - 1);
        if (hash3(id + vec3<i32>(seed, 5, 9)) > density) {
            continue;
        }
        let centre = vec3<f32>(id) + vec3<f32>(
            hash3(id + vec3<i32>(seed, 1, 7)),
            hash3(id + vec3<i32>(seed, 2, 7)),
            hash3(id + vec3<i32>(seed, 3, 7)),
        );
        let size = hash3(id + vec3<i32>(seed, 4, 7));
        let radius = 0.15 + 0.45 * size * size;
        let d = distance(x, centre) / radius;
        if (d < 1.0) {
            h += (d * d - 1.0) * 0.6 * radius;
        }
        let rim = (d - 1.0) / 0.22;
        h += exp(-rim * rim) * 0.25 * radius;
        let young = step(0.8, hash3(id + vec3<i32>(seed, 6, 7)));
        rays += young * exp(-max(d - 1.0, 0.0) * 1.6) * step(0.9, d) * 0.6;
    }
    return vec2<f32>(h / scale, rays);
}

fn height(p: vec3<f32>) -> vec4<f32> {
    let k = kind();
    if (k == GAS || k == ICE_GIANT || k == HAZY || k == HOT || k == CLOUDY) {
        return vec4<f32>(0.0, 0.0, 0.0, 1.0);
    }
    let o = gen.offsets;
    let warp = vec3<f32>(
        fbm(p * 1.4 + o[0].xyz, 4),
        fbm(p * 1.4 + o[1].xyz, 4),
        fbm(p * 1.4 + o[2].xyz, 4),
    );
    let q = normalize(p + warp * 0.32);

    var weights = 0.0;
    var bias = 0.0;
    var d1 = 9.0;
    var d2 = 9.0;
    for (var i = 0; i < 12; i++) {
        let plate = gen.plates[i];
        let d = distance(q, plate.xyz);
        let w = exp(-d * d * 14.0);
        weights += w;
        bias += w * plate.w;
        if (d < d1) {
            d2 = d1;
            d1 = d;
        } else if (d < d2) {
            d2 = d;
        }
    }
    let plates = bias / max(weights, 1e-5);
    let boundary = d2 - d1;

    let shapes = fbm(q * 2.2 + o[3].xyz, 6);
    var base = 0.42 * plates + 0.55 * shapes;
    let landish = smoothstep(-0.15, 0.25, base);
    let belt = exp(-boundary * boundary / (0.07 * 0.07));
    let ridges = ridged(q * 5.5 + o[0].zxy, 6);
    let scattered = smoothstep(0.15, 0.55, fbm(q * 3.0 + o[1].zxy, 3));
    var mountains = ridges * (belt * (0.35 + 0.65 * landish) * 0.42 + scattered * 0.12 * landish);
    var detail = eroded(q * 9.0 + o[2].zxy, 7) * (0.05 + 0.10 * landish);

    // Airless and icy worlds: little tectonics, a cratered face.
    var pits = 0.0;
    if (k == BARREN || k == CAPTURED || k == ICY_MOON || k == DESERT || k == VOLCANIC
        || k == GEYSER || k == TWO_FACED || k == NITROGEN || k == RUSTY) {
        var amount = 1.0;
        if (k == ICY_MOON || k == GEYSER || k == NITROGEN) { amount = 0.25; }
        if (k == DESERT) { amount = 0.4; }
        if (k == VOLCANIC) { amount = 0.0; }
        base *= 0.5;
        mountains *= 0.35;
        let big = craters(p, 7.0, 0.55 * amount, 3);
        let mid = craters(p, 22.0, 0.6 * amount, 17);
        let small = craters(p, 70.0, 0.6 * amount, 41);
        pits = big.x * 1.6 + mid.x * 1.4 + small.x * 1.2;
        detail *= 0.6;
    }
    return vec4<f32>(base + mountains + detail + pits, mountains, base, 1.0);
}

@fragment
fn fs_height(in: Varying) -> @location(0) vec4<f32> {
    return height(sphere_at(in.position.xy / gen.size.xy));
}

fn srgb(r: f32, g: f32, b: f32) -> vec3<f32> {
    return pow(vec3<f32>(r, g, b) / 255.0, vec3<f32>(2.2));
}

fn wide(uv: vec2<f32>, level: f32) -> f32 {
    return textureSampleLevel(terrain, terrain_sampler, uv, level).r - gen.size.z;
}

fn elevation_at(texel: vec2<i32>) -> f32 {
    let w = i32(gen.size.x);
    let h = i32(gen.size.y);
    let t = vec2<i32>((texel.x + w) % w, clamp(texel.y, 0, h - 1));
    return textureLoad(terrain, t, 0).r - gen.size.z;
}

fn settlements(p: vec3<f32>, scale: f32, density: f32, seed: i32) -> f32 {
    let x = p * scale;
    let cell = vec3<i32>(floor(x));
    var light = 0.0;
    for (var k = 0; k < 27; k++) {
        let id = cell + vec3<i32>(k % 3 - 1, (k / 3) % 3 - 1, k / 9 - 1);
        if (hash3(id + vec3<i32>(seed, 17, 3)) > density) {
            continue;
        }
        let centre = vec3<f32>(id) + vec3<f32>(
            hash3(id + vec3<i32>(seed, 1, 0)),
            hash3(id + vec3<i32>(seed, 2, 0)),
            hash3(id + vec3<i32>(seed, 3, 0)),
        );
        let size = hash3(id + vec3<i32>(seed, 4, 0));
        let population = size * size * size;
        let radius = 0.08 + 0.42 * population;
        let d = distance(x, centre) / radius;
        light += (0.25 + 0.75 * population) * exp(-d * d * 2.5);
    }
    return light;
}

fn turn_about(v: vec3<f32>, axis: vec3<f32>, angle: f32) -> vec3<f32> {
    let c = cos(angle);
    let s = sin(angle);
    return v * c + cross(axis, v) * s + axis * dot(axis, v) * (1.0 - c);
}

// Thin lines where a noise field crosses zero: cracks in ice and crust.
fn cracks(p: vec3<f32>, width: f32) -> f32 {
    let a = abs(noise(p) + 0.5 * noise(p * 2.1 + vec3<f32>(3.1)));
    return 1.0 - smoothstep(0.0, width, a);
}

fn pal(i: i32) -> vec3<f32> {
    return gen.palette[i].rgb;
}

struct Surface {
    day: vec3<f32>,
    bump: vec2<f32>,
    // Water for the glint; 0 elsewhere.
    water: f32,
    // City lights, or molten rock's glow.
    night: f32,
    cloud: f32,
};

// Bands of a giant: latitude warped by turbulence and wound by storms.
fn giant(p: vec3<f32>, k: i32) -> Surface {
    let o = gen.offsets;
    var c = p;
    for (var i = 0; i < 6; i++) {
        let storm = gen.storms[i];
        let d = distance(c, storm.xyz);
        c = turn_about(c, storm.xyz, storm.w * 1.4 * exp(-d * d / (0.12 * 0.12)));
    }
    let streaks = vec3<f32>(c.x * 3.0, c.y * 34.0, c.z * 3.0);
    let turbulence = fbm(streaks + o[0].xyz, 6) * 0.9 + fbm(c * 9.0 + o[1].xyz, 4) * 0.35;
    let y = c.y + turbulence * 0.05;
    let bands = sin(y * (18.0 + o[2].w) + o[3].w) * 0.5 + 0.5;
    let fine = sin(y * 61.0 + turbulence * 3.0) * 0.5 + 0.5;
    var colour = mix(pal(0), pal(1), smoothstep(0.25, 0.75, bands));
    colour = mix(colour, pal(2), smoothstep(0.55, 0.95, fine) * 0.45);
    colour = mix(colour, pal(3), smoothstep(0.85, 0.98, abs(p.y)) * 0.6);
    // One great storm: an oval of the darker colour with a pale rim.
    let spot = gen.storms[0].xyz;
    let rel = p - spot;
    let oval = length(vec2<f32>(dot(rel, normalize(cross(vec3<f32>(0.0, 1.0, 0.0), spot))) * 0.55, rel.y)) / 0.07;
    colour = mix(colour, pal(1) * 0.85, (1.0 - smoothstep(0.6, 1.0, oval)) * 0.85);
    colour = mix(colour, pal(3), exp(-pow((oval - 1.1) / 0.15, 2.0)) * 0.35);
    if (k == ICE_GIANT) {
        colour = mix(pal(0), colour, 0.3);
    }
    var s: Surface;
    s.day = colour * (0.94 + 0.12 * fbm(c * 40.0, 3));
    s.bump = vec2<f32>(0.0);
    s.water = 0.0;
    s.night = 0.0;
    if (k == HOT) {
        // A hot Jupiter glows dull red from its own heat, brightest in the
        // hotter bands.
        s.night = 0.35 + 0.35 * bands + 0.2 * turbulence;
    }
    s.cloud = 0.0;
    return s;
}

// A deck of sulphuric cloud: soft streaks swept along the latitudes into
// broad chevrons, as Venus's are in ultraviolet.
fn cloudy(p: vec3<f32>) -> Surface {
    let o = gen.offsets;
    let lon = atan2(p.x, p.z);
    let chevron = p.y * 3.0 + abs(sin(lon * 0.5)) * 0.9 * sign(p.y);
    let swirl = fbm(vec3<f32>(p.x * 2.0, p.y * 9.0 + chevron, p.z * 2.0) + o[0].xyz, 6);
    let tone = smoothstep(-0.35, 0.45, swirl + 0.25 * sin(chevron * 3.0));
    var s: Surface;
    s.day = mix(pal(1), pal(2), tone) * (0.95 + 0.1 * fbm(p * 30.0 + o[1].xyz, 3));
    s.day = mix(s.day, pal(3), smoothstep(0.8, 0.98, abs(p.y)) * 0.3);
    s.bump = vec2<f32>(0.0);
    s.water = 0.0;
    s.night = 0.0;
    s.cloud = 0.0;
    return s;
}

fn surface(position: vec2<f32>) -> Surface {
    let k = kind();
    let uv = position / gen.size.xy;
    let p = sphere_at(uv);
    if (k == GAS || k == ICE_GIANT || k == HOT) {
        return giant(p, k);
    }
    if (k == CLOUDY) {
        return cloudy(p);
    }
    let o = gen.offsets;
    let texel = vec2<i32>(position);
    let e = textureLoad(terrain, texel, 0).r - gen.size.z;
    let lat = asin(clamp(p.y, -1.0, 1.0));
    let cos_lat = max(cos(lat), 0.02);
    let low = 1.0 - smoothstep(-0.0015, 0.0015, e);
    let land = 1.0 - low;

    let east_step = cos_lat * TAU / gen.size.x;
    let north_step = PI / gen.size.y;
    let floor_at = select(-1.0, 0.0, k == TERRAN || k == OCEAN || k == LAVA || k == ICE || k == JUNGLE || k == EYEBALL);
    let de = (max(elevation_at(texel + vec2<i32>(1, 0)), floor_at) - max(elevation_at(texel - vec2<i32>(1, 0)), floor_at)) / (2.0 * east_step);
    let dn = (max(elevation_at(texel - vec2<i32>(0, 1)), floor_at) - max(elevation_at(texel + vec2<i32>(0, 1)), floor_at)) / (2.0 * north_step);
    let bump = clamp(-vec2<f32>(de, dn) * 0.0025 * 22.0, vec2<f32>(-0.45), vec2<f32>(0.45));
    let steep = length(bump);
    let grain = fbm(p * 60.0 + o[1].yxz, 4);
    let wobble = fbm(p * 4.0 + o[3].zyx, 4);
    var warmth = pow(cos_lat, 0.85) + wobble * 0.10 + gen.flags.w;
    if (k == EYEBALL) {
        // Locked to its star, which always stands over longitude 0 (+z):
        // warm under it, frozen past the terminator.
        warmth = mix(-0.35, 1.2, smoothstep(-0.25, 0.95, p.z)) + wobble * 0.10;
    } else if (k == JUNGLE) {
        warmth += 0.08;
    }
    let temperature = warmth - max(e, 0.0) * 1.5;

    var s: Surface;
    s.bump = bump;
    s.water = 0.0;
    s.night = 0.0;
    s.cloud = 0.0;

    if (k == TERRAN || k == OCEAN || k == JUNGLE || k == EYEBALL) {
        let cells = cos(6.0 * lat);
        let degrees = abs(lat) * 57.2958;
        let from_east = select(-1.0, 1.0, degrees < 30.0 || degrees > 62.0);
        var ocean_upwind = 0.0;
        var barrier = 0.0;
        var weights = 0.0;
        for (var i = 1; i <= 5; i++) {
            let reach = f32(i) * 0.0105 / cos_lat;
            let up = wide(uv + vec2<f32>(from_east * reach, 0.0), 4.5);
            let w = 1.0 / f32(i);
            ocean_upwind += w * (1.0 - smoothstep(-0.02, 0.03, up));
            barrier = max(barrier, up - max(e, 0.0));
            weights += w;
        }
        ocean_upwind /= weights;
        let near_coast = 1.0 - smoothstep(0.0, 0.12, max(wide(uv, 5.5), 0.0));
        let interior = smoothstep(0.03, 0.25, wide(uv, 7.5));
        let shadow = smoothstep(0.05, 0.3, barrier);
        let wetter = select(0.0, 0.28, k == JUNGLE);
        let moisture = clamp(
            wetter + 0.40 + 0.26 * cells + 0.30 * ocean_upwind + 0.12 * near_coast - 0.18 * interior
                - 0.35 * shadow + 0.22 * fbm(p * 6.0 + o[2].yzx, 5),
            0.0,
            1.0,
        );
        let sand = mix(srgb(218.0, 184.0, 136.0), srgb(170.0, 118.0, 78.0), 0.45 * smoothstep(-0.2, 0.3, fbm(p * 14.0 + o[0].yxz, 4)));
        let dry = mix(sand, srgb(150.0, 136.0, 94.0), smoothstep(0.10, 0.32, moisture));
        let warm_green = mix(srgb(122.0, 114.0, 66.0), srgb(26.0, 54.0, 20.0), smoothstep(0.48, 0.80, moisture));
        let green = mix(srgb(32.0, 48.0, 34.0), mix(srgb(48.0, 70.0, 32.0), warm_green, smoothstep(0.62, 0.85, temperature)), smoothstep(0.42, 0.58, temperature));
        var ground = mix(dry, green, smoothstep(0.25, 0.62, moisture));
        ground = mix(ground, srgb(108.0, 102.0, 88.0), smoothstep(0.46, 0.34, temperature));
        ground = mix(ground, srgb(100.0, 88.0, 76.0), max(smoothstep(0.12, 0.40, steep) * 0.75, smoothstep(0.16, 0.34, e) * 0.55));
        ground = mix(ground, srgb(236.0, 240.0, 245.0), smoothstep(0.26, 0.15, temperature));
        ground *= 0.86 + 0.28 * (grain + 0.5);
        let depth = max(-e, 0.0);
        var sea = mix(srgb(30.0, 82.0, 112.0), srgb(10.0, 28.0, 66.0), smoothstep(0.0, 0.035, depth));
        sea = mix(sea, srgb(5.0, 13.0, 40.0), smoothstep(0.035, 0.22, depth));
        let ice = smoothstep(0.17, 0.10, warmth + 0.04 + 0.04 * fbm(p * 20.0, 3));
        sea = mix(sea, srgb(218.0, 226.0, 236.0), ice);
        s.day = mix(ground, sea, low);
        s.water = low * (1.0 - ice);
        s.bump = bump * land;
        let liveable = land
            * smoothstep(0.30, 0.52, temperature) * (1.0 - smoothstep(0.30, 0.50, e))
            * smoothstep(0.10, 0.35, moisture) * (0.45 + 0.9 * near_coast)
            * smoothstep(-0.25, 0.20, fbm(p * 3.0 + o[3].xzy, 3));
        if (liveable > 0.01 && gen.flags.y > 0.5) {
            let lights = liveable * (settlements(p, 120.0, 0.7, 11) * 1.8 + settlements(p, 420.0, 0.6, 29) * 0.7);
            s.night = lights * (1.0 - smoothstep(0.08, 0.25, steep));
        }
        // Clouds.
        var c = p;
        for (var i = 0; i < 12; i++) {
            let storm = gen.storms[i];
            let d = distance(c, storm.xyz);
            c = turn_about(c, storm.xyz, storm.w * exp(-d * d / (0.22 * 0.22)));
        }
        let stretched = vec3<f32>(c.x, c.y * 1.9, c.z);
        let cwarp = vec3<f32>(
            fbm(stretched * 2.0 + o[1].xyz, 3),
            fbm(stretched * 2.0 + o[2].xyz, 3),
            fbm(stretched * 2.0 + o[3].xyz, 3),
        );
        let sheet = fbm(stretched * 3.5 + cwarp * 0.9 + o[0].xyz, 8);
        let itcz = exp(-pow((lat * 57.2958 - 4.0) / 7.0, 2.0));
        var more = select(0.0, 0.08, k == OCEAN) + select(0.0, 0.06, k == JUNGLE);
        var bands = 0.10 * cells + 0.12 * itcz;
        if (k == EYEBALL) {
            // One great storm system boils up under the star; the far side is
            // too cold for cloud.
            bands = 0.35 * smoothstep(0.55, 1.0, p.z) - 0.3 * (1.0 - smoothstep(-0.2, 0.3, p.z));
        }
        let cover = sheet + bands + 0.16 * (moisture - 0.5) * land - 0.10 + more;
        var cloud = smoothstep(0.0, 0.42, cover);
        cloud *= smoothstep(-0.45, 0.25, fbm(c * 26.0 + o[3].yzx, 4) + cloud * 0.6);
        s.cloud = cloud;
    } else if (k == DESERT) {
        // Rust and ochre, darker basalt lowlands, polar caps, dust streaks.
        let tone = smoothstep(-0.3, 0.4, fbm(p * 5.0 + o[0].xyz, 5));
        var ground = mix(pal(0), pal(2), tone * 0.6);
        ground = mix(ground, pal(1), low * 0.7 + smoothstep(0.1, 0.35, steep) * 0.3);
        ground = mix(ground, pal(3), smoothstep(0.2, 0.6, fbm(p * 18.0 + o[2].xyz, 4)) * 0.25);
        let cap = smoothstep(0.2, 0.12, warmth + 0.05 * fbm(p * 12.0, 3));
        ground = mix(ground, srgb(236.0, 232.0, 226.0), cap);
        s.day = ground * (0.88 + 0.24 * (grain + 0.5));
        s.cloud = smoothstep(0.35, 0.6, fbm(vec3<f32>(p.x, p.y * 2.0, p.z) * 4.0 + o[3].xyz, 5)) * 0.25;
    } else if (k == ICE) {
        let lines = max(cracks(p * 9.0 + o[0].xyz, 0.035), cracks(p * 23.0 + o[1].xyz, 0.03) * 0.6);
        var ground = mix(pal(0), pal(2), smoothstep(-0.2, 0.4, fbm(p * 6.0 + o[2].xyz, 4)));
        ground = mix(ground, pal(1), low * 0.6);
        ground = mix(ground, pal(3), lines * 0.6);
        s.day = ground * (0.94 + 0.12 * (grain + 0.5));
        s.bump = bump * 0.5;
        s.cloud = smoothstep(0.25, 0.6, fbm(vec3<f32>(p.x, p.y * 1.6, p.z) * 3.5 + o[3].xyz, 6)) * 0.5;
    } else if (k == LAVA) {
        let lines = max(cracks(p * 7.0 + o[0].xyz, 0.03), cracks(p * 19.0 + o[1].xyz, 0.025) * 0.7);
        let crust = mix(pal(0), pal(2), smoothstep(-0.2, 0.4, fbm(p * 12.0 + o[2].xyz, 4)));
        let molten = max(low, lines * land);
        // By day the lava reads as dark red rock with a crust on it; its
        // light is mostly the emissive glow, which shows best at night.
        s.day = mix(crust * (0.8 + 0.4 * (grain + 0.5)), pal(1) * 0.25, molten);
        s.night = clamp(molten * (0.6 + 0.4 * fbm(p * 30.0, 3) + 0.4), 0.0, 1.0);
        s.bump = bump * land;
    } else if (k == VOLCANIC) {
        // Sulphur yellow and orange, white frost, black calderas with red
        // rims that glow at their hearts.
        let tone = fbm(p * 5.0 + o[0].xyz, 5);
        var ground = mix(pal(0), pal(1), smoothstep(-0.1, 0.35, tone));
        ground = mix(ground, pal(2), smoothstep(0.25, 0.5, fbm(p * 9.0 + o[1].xyz, 4)) * 0.6);
        let cal = craters(p, 9.0, 0.5, 23);
        let spot = smoothstep(0.1, 0.5, cal.y + max(-cal.x * 30.0, 0.0));
        ground = mix(ground, pal(3), spot * 0.85);
        ground = mix(ground, srgb(170.0, 60.0, 30.0), smoothstep(0.0, 0.3, cal.y) * (1.0 - spot) * 0.5);
        s.day = ground * (0.9 + 0.2 * (grain + 0.5));
        s.night = spot * smoothstep(0.6, 0.9, hash3(vec3<i32>(floor(p * 9.0)))) * 0.8;
        s.bump = bump * 0.4;
    } else if (k == ICY_MOON) {
        let lines = max(cracks(p * 6.0 + o[0].xyz, 0.03), max(cracks(p * 14.0 + o[1].xyz, 0.025) * 0.8, cracks(p * 31.0 + o[2].xyz, 0.02) * 0.5));
        var ground = mix(pal(0), pal(2), smoothstep(-0.2, 0.4, fbm(p * 4.0 + o[3].xyz, 4)));
        ground = mix(ground, pal(1), lines * 0.75);
        ground = mix(ground, pal(3), smoothstep(0.2, 0.5, fbm(p * 3.0 + o[0].zyx, 3)) * 0.3);
        s.day = ground;
        s.bump = bump * 0.6;
    } else if (k == GEYSER) {
        // Bright fresh ice, and blue fissures across the south pole.
        var ground = mix(pal(0), pal(2), smoothstep(-0.2, 0.4, fbm(p * 5.0 + o[0].xyz, 4)));
        let lon = atan2(p.x, p.z);
        let stripes = 1.0 - smoothstep(0.0, 0.05, abs(sin(lon * 2.0 + p.y * 3.0 + 0.4 * noise(p * 6.0 + o[1].xyz))));
        let south = smoothstep(-0.55, -0.8, p.y);
        ground = mix(ground, pal(1), stripes * south * 0.85);
        ground = mix(ground, pal(3), cracks(p * 8.0 + o[2].xyz, 0.02) * 0.4 * (1.0 - south));
        s.day = ground;
        s.bump = bump * 0.4;
    } else if (k == TWO_FACED) {
        // A dark leading hemisphere with a ragged edge, a bright trailing one.
        let edge = p.z + 0.3 * fbm(p * 3.0 + o[0].xyz, 5);
        let dark = 1.0 - smoothstep(-0.12, 0.12, edge);
        var ground = mix(pal(0), pal(2), smoothstep(-0.2, 0.4, fbm(p * 8.0 + o[1].xyz, 4)) * 0.5);
        ground = mix(ground, mix(pal(1), pal(3), smoothstep(-0.3, 0.3, fbm(p * 6.0 + o[2].xyz, 4))), dark);
        s.day = ground * (0.9 + 0.2 * (grain + 0.5));
    } else if (k == NITROGEN) {
        // Cantaloupe terrain: a close mesh of dimples; dark wind streaks; a
        // bright pinkish polar cap.
        let cells = craters(p, 28.0, 1.0, 51).x * 28.0;
        var ground = mix(pal(0), pal(1), smoothstep(-0.2, 0.2, cells + 0.3 * fbm(p * 7.0 + o[0].xyz, 4)) * 0.5);
        let streak = smoothstep(0.35, 0.6, fbm(vec3<f32>(p.x * 14.0, p.y * 3.0, p.z * 14.0) + o[1].xyz, 4));
        ground = mix(ground, pal(3), streak * 0.5 * smoothstep(-0.2, 0.3, p.y));
        ground = mix(ground, pal(2), smoothstep(-0.25, -0.55, p.y + 0.1 * fbm(p * 5.0, 3)));
        s.day = ground;
        s.bump = bump * 0.5;
    } else if (k == HAZY) {
        let tone = smoothstep(-0.3, 0.3, fbm(vec3<f32>(p.x, p.y * 2.5, p.z) * 3.0 + o[0].xyz, 5));
        s.day = mix(pal(0), pal(1), tone * 0.35);
        s.day = mix(s.day, pal(3), smoothstep(0.75, 0.95, abs(p.y)) * 0.4);
        s.bump = vec2<f32>(0.0);
    } else {
        // Barren and captured rock: greys, dark lowland plains, bright young
        // craters' rays.
        let rays = craters(p, 22.0, 0.6, 17).y + craters(p, 7.0, 0.55, 3).y;
        var ground = mix(pal(0), pal(2), smoothstep(-0.2, 0.4, fbm(p * 8.0 + o[0].xyz, 4)) * 0.5);
        ground = mix(ground, pal(1), low * 0.75);
        ground = mix(ground, pal(2), clamp(rays, 0.0, 1.0) * 0.5);
        if (k == CAPTURED) {
            ground *= 0.55;
        }
        s.day = ground * (0.85 + 0.3 * (grain + 0.5));
    }
    return s;
}

struct Planet {
    @location(0) day: vec4<f32>,
    @location(1) relief: vec4<f32>,
    @location(2) extra: vec4<f32>,
};

@fragment
fn fs_planet(in: Varying) -> Planet {
    let s = surface(in.position.xy);
    var out: Planet;
    out.day = vec4<f32>(s.day, 1.0);
    out.relief = vec4<f32>(vec2<f32>(0.5) + s.bump, s.water, 1.0);
    out.extra = vec4<f32>(clamp(s.night, 0.0, 1.0), clamp(s.cloud, 0.0, 1.0), 0.0, 1.0);
    return out;
}

struct Moon {
    @location(0) day: vec4<f32>,
    @location(1) relief: vec4<f32>,
};

// Moons have no clouds, cities or seas: their relief's third channel carries
// the glow of molten rock instead.
@fragment
fn fs_moon(in: Varying) -> Moon {
    let s = surface(in.position.xy);
    var out: Moon;
    out.day = vec4<f32>(s.day, 1.0);
    out.relief = vec4<f32>(vec2<f32>(0.5) + s.bump, clamp(s.night, 0.0, 1.0), 1.0);
    return out;
}

// The sky: the galaxy's band along a great circle (its pole in palette[0]),
// dust lanes, and faint glowing clouds.
@fragment
fn fs_sky(in: Varying) -> @location(0) vec4<f32> {
    let p = sphere_at(in.position.xy / gen.size.xy);
    let pole = gen.palette[0].xyz;
    let o = gen.offsets;
    let off = dot(p, pole);
    let warp = fbm(p * 3.0 + o[0].xyz, 4) * 0.12;
    let band = exp(-pow((off + warp) / 0.16, 2.0));
    let core = exp(-pow((off + warp) / 0.05, 2.0));
    let dust = smoothstep(-0.1, 0.35, fbm(p * 9.0 + o[1].xyz, 6));
    let glow = fbm(p * 5.0 + o[2].xyz, 5) * 0.5 + 0.5;
    var sky = vec3<f32>(0.55, 0.52, 0.62) * band * (0.25 + 0.75 * glow) * 0.05;
    sky += vec3<f32>(0.85, 0.72, 0.55) * core * 0.04 * glow;
    sky *= 1.0 - dust * 0.75 * band;
    // A few faint nebulae, tinted.
    let cloud = smoothstep(0.25, 0.65, fbm(p * 2.2 + o[3].xyz, 5));
    let tint = mix(vec3<f32>(0.6, 0.2, 0.35), vec3<f32>(0.2, 0.35, 0.6), fbm(p * 1.3 + o[0].zyx, 3) + 0.5);
    sky += tint * cloud * 0.012;
    return vec4<f32>(sky, 1.0);
}
