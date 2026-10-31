//! A survey of the helm over minutes of voyage: what a hull 20 m long and 3 m
//! high would feel, and how near it would come to the ground, the tunnel's
//! walls and the surface. The pose only says where the window is and which
//! way it faces, so everything here is measured from that, whatever the helm
//! does inside.
//!
//! `HELM_LOG_DIR=<dir> cargo test --release survey -- --ignored --nocapture`
//! writes a CSV of each voyage, a line every quarter second, and prints the
//! summary.

use super::*;
use crate::voyage::Voyage;
use std::f32::consts::PI;
use std::fmt::Write as _;

/// Metres in a world unit, and in a knot a second.
const METRES: f32 = 11.0;
const KNOT: f32 = 0.5144;

/// One voyage to survey: a seabed, a stretch of time and the swipes in it.
pub struct Voyaging {
    pub name: &'static str,
    pub visit: u64,
    pub seed: f32,
    pub from: f64,
    pub seconds: f64,
    /// (seconds into the voyage, legs to swipe on).
    pub swipes: &'static [(f64, i32)],
}

/// The worst of a voyage, cruising and in a swipe's passage apart.
#[derive(Debug, Default, Clone, Copy)]
pub struct Summary {
    /// Cruising: knots, least and most.
    pub slowest: f32,
    pub fastest: f32,
    /// Cruising: the most the boat speeds up or slows, m/s².
    pub surge: f32,
    /// Cruising: the fastest the nose swings, degrees a second, and the
    /// tightest turn that makes, in hull lengths (speed over rate of turn).
    pub yaw_rate: f32,
    pub radius: f32,
    /// Cruising: the fastest climb or dive, m/s, and vertical acceleration, m/s².
    pub climb: f32,
    pub heave: f32,
    /// The most the boat pitches and rolls, degrees, and how fast, degrees a second.
    pub pitch: f32,
    pub roll: f32,
    pub pitch_rate: f32,
    pub roll_rate: f32,
    /// Jitter: the largest frame-to-frame change in the window's angular
    /// velocity, degrees a second per frame.
    pub jitter: f32,
    /// A swipe's passage: top speed in knots, and its acceleration in g.
    pub dash: f32,
    pub dash_g: f32,
    /// The least clearance, in metres, of any point of the hull: above the
    /// ground, inside the tunnel's walls and vault, and under the surface or ice.
    pub keel: f32,
    pub wall: f32,
    pub top: f32,
    /// The window's least height above the ground right under it, metres.
    pub eye: f32,
    /// Cruising: the fastest the window closes on what it looks at, or draws
    /// away from it, m/s, and the most the boat's way is off its nose, either
    /// way, in degrees: how far it crabs.
    pub toward: f32,
    pub crab: f32,
    /// How far any point of the hull comes from the corridor's middle, in
    /// world units, in each biome (the one it is mostly in): the lane the
    /// world keeps clear must be wider.
    pub sweep: [f32; crate::voyage::BIOMES],
    /// The helm's mean and worst time a step, microseconds.
    pub step_us: f32,
    pub worst_us: f32,
}

/// Points over the hull: a spindle LENGTH long and HEIGHT round, its window
/// on the port side at mid-length, mid-height.
fn hull(pose: &Pose) -> Vec<[f32; 3]> {
    const LONG: f32 = 1.8;
    const ROUND: f32 = 0.27;
    let [nose, up, out] = pose.basis;
    let r0 = ROUND * 0.5;
    let centre = [
        pose.pos[0] - out[0] * r0,
        pose.pos[1] - out[1] * r0,
        pose.pos[2] - out[2] * r0,
    ];
    let mut points = Vec::new();
    for i in 0..=12 {
        let a = (i as f32 / 12.0 - 0.5) * LONG;
        let r = r0 * (1.0 - (2.0 * a / LONG).powi(2)).max(0.0).sqrt();
        for k in 0..12 {
            let (s, c) = (k as f32 * TAU / 12.0).sin_cos();
            points.push([
                centre[0] + nose[0] * a + (up[0] * c + out[0] * s) * r,
                centre[1] + nose[1] * a + (up[1] * c + out[1] * s) * r,
                centre[2] + nose[2] * a + (up[2] * c + out[2] * s) * r,
            ]);
        }
    }
    points
}

/// How far `p` is inside the tunnel's ragged arch, if the tunnel is there.
fn inside_tunnel(seabed: &Seabed, p: [f32; 3]) -> Option<f32> {
    let (c, lat, _) = seabed.tunnel_at(p[0])?;
    let across = (p[0] - c[0]) * lat[0] + (p[2] - c[2]) * lat[2];
    let dy = p[1] - c[1];
    let (wide, high) = super::super::world::tunnel_stretch(p[0]);
    let a = (dy / (0.85 * high)).atan2(across / wide);
    // Below the arch's foot is the floor, which the ground covers.
    if a < -0.2 * PI || a > 1.2 * PI {
        return None;
    }
    let r = Seabed::tunnel_radius(p[0], a);
    let rho = ((across / (wide * r)).powi(2) + (dy / (0.85 * high * r)).powi(2)).sqrt();
    Some((1.0 - rho) * 0.85 * r)
}

fn angles(pose: &Pose) -> [f32; 3] {
    let [nose, _, out] = pose.basis;
    let yaw = (-nose[2]).atan2(nose[0]);
    let pitch = nose[1].clamp(-1.0, 1.0).asin();
    let roll = (-out[1]).clamp(-1.0, 1.0).asin();
    [yaw, pitch, roll]
}

/// Sail a voyage at 60 frames a second, logging it to `log` if given.
pub fn sail(trip: &Voyaging, mut log: Option<&mut String>) -> Summary {
    let seabed = Seabed::new(trip.visit, trip.seed);
    let mut voyage = Voyage::default();
    let mut helm = Helm::default();
    let frames = (trip.seconds * 60.0) as usize;
    let mut poses = Vec::with_capacity(frames);
    let mut paces = Vec::with_capacity(frames);
    let mut last_x = None;
    let mut swipes = trip.swipes.iter().peekable();
    let (mut spent, mut worst) = (0.0f32, 0.0f32);
    for frame in 0..frames {
        let seconds = trip.from + frame as f64 / 60.0;
        while let Some(&&(at, legs)) = swipes.peek() {
            if at > seconds - trip.from {
                break;
            }
            voyage.travel(legs, seconds);
            swipes.next();
        }
        let at = voyage.advance(seconds, trip.seed);
        let pace = last_x.map_or(SPEED, |x| (at.x - x) * 60.0).abs() as f32;
        last_x = Some(at.x);
        let begun = std::time::Instant::now();
        poses.push(helm.steer(&seabed, at, seconds));
        let took = begun.elapsed().as_secs_f32() * 1e6;
        spent += took;
        worst = worst.max(took);
        paces.push(pace);
    }

    let mut s = Summary {
        slowest: f32::MAX,
        radius: f32::MAX,
        keel: f32::MAX,
        wall: f32::MAX,
        top: f32::MAX,
        eye: f32::MAX,
        step_us: spent / frames as f32,
        worst_us: worst,
        ..Summary::default()
    };
    if let Some(log) = log.as_deref_mut() {
        log.push_str(
            "t,x,y,z,knots,yaw_deg,pitch_deg,roll_deg,yaw_rate_dps,radius_lengths,\
             climb_ms,keel_m,wall_m,top_m,passage,toward_ms,crab_deg\n",
        );
    }
    let angle_rate = |i: usize, j: usize, k: usize| {
        let a = angles(&poses[j])[k];
        let b = angles(&poses[i])[k];
        let mut d = a - b;
        d -= TAU * (d / TAU).round();
        d * 60.0 / (j - i) as f32
    };
    // Derivatives over a second either side, so f32 positions do not swamp them.
    let w = 30;
    let mut previous_spin: Option<[f32; 3]> = None;
    for i in w..frames.saturating_sub(w) {
        let pose = &poses[i];
        // Within a passage, or a few seconds either side of one: the course
        // comes round through as much as the corridor's bends in a leg or
        // two, and finishes coming round as the passage settles.
        let near = |k: usize| paces[k] > (SPEED as f32) * 1.3;
        let passage = (i.saturating_sub(360)..(i + 360).min(frames)).any(near);
        let (a, b) = (&poses[i - w], &poses[i + w]);
        let v = [
            (b.pos[0] - a.pos[0]) * 60.0 / (2 * w) as f32,
            (b.pos[1] - a.pos[1]) * 60.0 / (2 * w) as f32,
            (b.pos[2] - a.pos[2]) * 60.0 / (2 * w) as f32,
        ];
        let horizontal = v[0].hypot(v[2]);
        let knots = horizontal * METRES / KNOT;
        let accel =
            |k: usize| (b.pos[k] - 2.0 * pose.pos[k] + a.pos[k]) * (60.0 / w as f32).powi(2);
        // Along the track: the surge.
        let along = (accel(0) * v[0] + accel(2) * v[2]) / horizontal.max(1e-6);
        let yaw_rate = angle_rate(i - w, i + w, 0);
        let pitch_rate = angle_rate(i - w, i + w, 1);
        let roll_rate = angle_rate(i - w, i + w, 2);
        let radius = horizontal / yaw_rate.abs().max(1e-6) / 1.8;
        let [_, pitch, roll] = angles(pose);
        // Frame-to-frame angular velocity, for jitter.
        let spin = [
            angle_rate(i - 1, i, 0),
            angle_rate(i - 1, i, 1),
            angle_rate(i - 1, i, 2),
        ];
        if let Some(p) = previous_spin {
            for k in 0..3 {
                s.jitter = s.jitter.max((spin[k] - p[k]).abs().to_degrees());
            }
        }
        previous_spin = Some(spin);

        let mut keel = f32::MAX;
        let mut wall = f32::MAX;
        let mut top = f32::MAX;
        let ice = if seabed.share(
            pose.pos[0],
            corridor(pose.pos[0]),
            super::super::world::ARCTIC,
        ) > 0.3
        {
            -super::super::world::ICE
        } else {
            0.0
        };
        for p in hull(pose) {
            keel = keel.min(p[1] - seabed.ground(p[0], p[2]));
            if let Some(m) = inside_tunnel(&seabed, p) {
                wall = wall.min(m);
            }
            top = top.min(ice - p[1]);
        }
        let eye = pose.pos[1] - seabed.ground(pose.pos[0], pose.pos[2]);
        // The way, in the window's own axes: along the hull and out of it.
        let [nose, _, out] = pose.basis;
        let ahead = v[0] * nose[0] + v[2] * nose[2];
        let toward = v[0] * out[0] + v[1] * out[1] + v[2] * out[2];
        let crab = toward.atan2(ahead);
        let biome = seabed.biome(pose.pos[0], corridor(pose.pos[0]));
        let biome = if biome.w < 0.5 { biome.a } else { biome.b };
        for p in hull(pose) {
            let off = (p[2] - corridor(p[0])).abs();
            s.sweep[biome] = s.sweep[biome].max(off);
        }
        s.keel = s.keel.min(keel * METRES);
        s.wall = s.wall.min(wall * METRES);
        s.top = s.top.min(top * METRES);
        s.eye = s.eye.min(eye * METRES);
        s.pitch = s.pitch.max(pitch.abs().to_degrees());
        s.roll = s.roll.max(roll.abs().to_degrees());
        if passage {
            s.dash = s.dash.max(knots);
            s.dash_g = s.dash_g.max(along.abs() * METRES / 9.81);
        } else {
            s.slowest = s.slowest.min(knots);
            s.fastest = s.fastest.max(knots);
            s.surge = s.surge.max(along.abs() * METRES);
            s.yaw_rate = s.yaw_rate.max(yaw_rate.abs().to_degrees());
            s.radius = s.radius.min(radius);
            s.climb = s.climb.max(v[1].abs() * METRES);
            s.heave = s.heave.max(accel(1).abs() * METRES);
            s.pitch_rate = s.pitch_rate.max(pitch_rate.abs().to_degrees());
            s.roll_rate = s.roll_rate.max(roll_rate.abs().to_degrees());
            s.toward = s.toward.max(toward.abs() * METRES);
            s.crab = s.crab.max(crab.abs().to_degrees());
        }
        if let Some(log) = log.as_deref_mut()
            && i % 15 == 0
        {
            let [yaw, ..] = angles(pose);
            let wall = if wall == f32::MAX {
                f32::NAN
            } else {
                wall * METRES
            };
            let _ = writeln!(
                log,
                "{:.2},{:.3},{:.3},{:.3},{:.2},{:.2},{:.2},{:.2},{:.3},{:.1},{:.3},{:.2},{:.2},{:.2},{},{:.3},{:.1}",
                trip.from + i as f64 / 60.0,
                pose.pos[0],
                pose.pos[1],
                pose.pos[2],
                knots,
                yaw.to_degrees(),
                pitch.to_degrees(),
                roll.to_degrees(),
                yaw_rate.to_degrees(),
                radius.min(999.0),
                v[1] * METRES,
                keel * METRES,
                wall,
                top * METRES,
                u8::from(passage),
                toward * METRES,
                crab.to_degrees(),
            );
        }
    }
    s
}

/// The voyages surveyed: whole circuits cruising on two seabeds, one swiped
/// through leg after leg, there and back, and one through the lava cave's
/// halls and narrows, set across the tunnel from before its mouth to past
/// the far one.
pub const VOYAGES: [Voyaging; 4] = [
    Voyaging {
        name: "cruise-7",
        visit: 7,
        seed: 0.0,
        from: 5.0,
        seconds: 7.0 * 62.0 + 30.0,
        swipes: &[],
    },
    Voyaging {
        name: "cruise-3",
        visit: 3,
        seed: 0.37,
        from: 5.0,
        seconds: 7.0 * 62.0 + 30.0,
        swipes: &[],
    },
    Voyaging {
        name: "swipes-7",
        visit: 7,
        seed: 0.0,
        from: 5.0,
        seconds: 330.0,
        swipes: &[
            (20.0, 1),
            (45.0, 1),
            (70.0, 1),
            (95.0, 1),
            (120.0, 1),
            (145.0, 1),
            (170.0, 1),
            (195.0, 2),
            (230.0, -1),
            (260.0, -1),
            (290.0, 1),
        ],
    },
    // The cave is the fourth leg of this circuit: from a little over half a
    // leg before it to most of a leg past it.
    Voyaging {
        name: "cave-3",
        visit: 3,
        seed: 0.37,
        from: 684.0,
        seconds: 560.0,
        swipes: &[],
    },
];

#[test]
#[ignore = "writes a log of the helm's voyages; set HELM_LOG_DIR"]
fn survey() {
    let dir = std::env::var("HELM_LOG_DIR").ok();
    for v in &VOYAGES {
        let mut log = String::new();
        let s = sail(v, dir.as_ref().map(|_| &mut log));
        if let Some(dir) = &dir {
            std::fs::create_dir_all(dir).expect("a directory for the log");
            std::fs::write(
                std::path::Path::new(dir).join(format!("{}.csv", v.name)),
                log,
            )
            .expect("a log");
        }
        println!("{}: {s:#?}", v.name);
        println!(
            "{}: the helm takes {:.1} µs a step, at worst {:.0} µs",
            v.name, s.step_us, s.worst_us
        );
    }
}

/// A whole circuit cruising, and swipes through leg after leg: the boat moves
/// like a heavy hull of its size, and no point of the hull ever touches.
#[test]
fn the_hull_moves_like_a_heavy_boat() {
    for trip in [&VOYAGES[0], &VOYAGES[2], &VOYAGES[3]] {
        let s = sail(trip, None);
        let name = trip.name;
        assert!(
            s.slowest > 1.0 && s.fastest < 1.4,
            "{name}: cruising at {:.2} to {:.2} knots",
            s.slowest,
            s.fastest
        );
        assert!(s.surge < 0.02, "{name}: surging at {:.3} m/s²", s.surge);
        assert!(
            s.radius > 2.8,
            "{name}: turning in {:.2} hull lengths",
            s.radius
        );
        // Under a metre a second on its planes; blowing its tanks for a
        // shelf it could not otherwise clear, half as fast again.
        assert!(s.climb < 1.2, "{name}: climbing at {:.2} m/s", s.climb);
        assert!(s.heave < 0.3, "{name}: heaving at {:.2} m/s²", s.heave);
        assert!(
            // Banking under 2.5°, and the heel over the trench on top.
            s.pitch < 6.5 && s.roll < 2.5 + super::PEER.to_degrees(),
            "{name}: pitched or rolled too far"
        );
        assert!(
            s.jitter < 2.0,
            "{name}: a jolt of {:.2}°/s in a frame",
            s.jitter
        );
        assert!(s.keel > 0.05, "{name}: the keel within {:.2} m", s.keel);
        assert!(s.wall > 1.0, "{name}: the walls within {:.2} m", s.wall);
        assert!(s.top > 1.5, "{name}: the surface within {:.2} m", s.top);
    }
}
