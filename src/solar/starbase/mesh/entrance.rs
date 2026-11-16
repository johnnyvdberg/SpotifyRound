//! The entrance: from the mouth, a kilometre across the middle, a pit
//! stepped down a kilometre to the tunnel's bore, built for ships far
//! larger than this one. Each step's riser is a ring of lit floors and its
//! tread a landing with lamps along its edge; lines of lamps run down it to
//! the bore. Round the rim a hazard ring on a broad apron, and eight
//! pylons standing over it, a beacon on each.

use glam::Vec3;
use std::f32::consts::TAU;

use super::super::super::models::part;
use super::super::super::tour::starbase::{BORE, FUNNEL, FUNNEL_TOP};
use super::tunnel::{DECK, DIVISIONS, deck, division_angle};
use super::{Builder, neon, parts, round};

/// Steps down the pit.
const STEPS: usize = 9;
/// How far out the apron reaches, where the city's floor begins.
pub const APRON: f32 = FUNNEL_TOP as f32 + 260.0;

pub(super) fn entrance(b: &mut Builder) {
    let depth = FUNNEL as f32;
    let (top, bore) = (FUNNEL_TOP as f32, BORE as f32);
    let radius = |k: usize| bore + (top - bore) * (1.0 - k as f32 / STEPS as f32).powf(1.5);
    let level = |k: usize| depth * k as f32 / STEPS as f32;
    for k in 0..STEPS {
        let (z0, z1) = (level(k), level(k + 1));
        let (r0, r1) = (radius(k), radius(k + 1));
        b.tone = (37 + k * 53 % 200) as u8;
        for i in 0..DIVISIONS {
            let (a0, a1) = (division_angle(i), division_angle(i + 1));
            // The riser, leaning back so it shows from above: floors of
            // windows, a lip at its top.
            let lean = r0 - (r0 - r1) * 0.72;
            let lip = z0 + 14.0;
            let slope = (r0 - lean) / (z1 - z0);
            let n = [a0, a1, a1, a0].map(|a| -Vec3::new(a.cos(), a.sin(), slope));
            b.quad(
                [
                    round(r0, a0, z0),
                    round(r0, a1, z0),
                    round(r0, a1, lip),
                    round(r0, a0, lip),
                ],
                [a0, a1, a1, a0].map(|a| -Vec3::new(a.cos(), a.sin(), 0.0)),
                part::ACCENT,
            );
            b.quad(
                [
                    round(r0, a0, lip),
                    round(r0, a1, lip),
                    round(lean, a1, z1),
                    round(lean, a0, z1),
                ],
                n,
                parts::TOWER,
            );
            // The tread, a line of lamps along its inner edge.
            let edge = (r1 + 4.0).min(lean);
            b.flat(
                [
                    round(lean, a0, z1),
                    round(lean, a1, z1),
                    round(edge, a1, z1),
                    round(edge, a0, z1),
                ],
                -Vec3::Z,
                part::HULL,
            );
            b.flat(
                [
                    round(edge, a0, z1),
                    round(edge, a1, z1),
                    round(r1, a1, z1),
                    round(r1, a0, z1),
                ],
                -Vec3::Z,
                if i % 4 == 3 { part::DARK } else { parts::LIGHT },
            );
        }
        // The guide lines down it, eight of them: down the riser and in
        // across the tread.
        b.tone = 255;
        let lean = r0 - (r0 - r1) * 0.72;
        for g in 0..8 {
            let a = g as f32 * TAU / 8.0;
            let across = Vec3::new(-a.sin(), a.cos(), 0.0) * 3.5;
            let at = |r: f32, z: f32| round(r, a, z);
            b.flat(
                [
                    at(r0 - 0.8, z0) - across,
                    at(r0 - 0.8, z0) + across,
                    at(lean - 0.8, z1 - 0.8) + across,
                    at(lean - 0.8, z1 - 0.8) - across,
                ],
                -Vec3::new(a.cos(), a.sin(), 0.5),
                parts::LIGHT,
            );
            b.flat(
                [
                    at(lean, z1 - 0.8) - across,
                    at(lean, z1 - 0.8) + across,
                    at(r1, z1 - 0.8) + across,
                    at(r1, z1 - 0.8) - across,
                ],
                -Vec3::Z,
                parts::LIGHT,
            );
        }
    }
    // The end of the tunnel's deck, closing the bore below it.
    b.tone = 255;
    let (floor, _) = deck();
    for i in DECK.0..DECK.1 {
        let (p0, p1) = (
            round(bore, division_angle(i), depth),
            round(bore, division_angle(i + 1), depth),
        );
        b.flat(
            [
                p0,
                p1,
                Vec3::new(p1.x, floor, depth),
                Vec3::new(p0.x, floor, depth),
            ],
            -Vec3::Z,
            part::ACCENT,
        );
    }
    // The rim's hazard ring and the apron round it, facing out of the mouth.
    b.tone = 255;
    for i in 0..DIVISIONS {
        let (a0, a1) = (division_angle(i), division_angle(i + 1));
        for (r0, r1, part) in [
            (top, top + 70.0, parts::HAZARD),
            (top + 70.0, APRON, part::HULL),
        ] {
            b.flat(
                [
                    round(r0, a0, 0.0),
                    round(r1, a0, 0.0),
                    round(r1, a1, 0.0),
                    round(r0, a1, 0.0),
                ],
                -Vec3::Z,
                part,
            );
        }
    }
    // The pylons: a grey shaft stepped back toward its top, a lamp
    // under its head and a beacon over it.
    for g in 0..8 {
        let a = (g as f32 + 0.5) * TAU / 8.0;
        let u = Vec3::new(a.cos(), a.sin(), 0.0);
        let at = u * (top + 150.0);
        b.tone = (g * 23) as u8;
        b.block(at, u, [55.0, 80.0], [0.0, -360.0], parts::TOWER, part::HULL);
        b.block(
            at,
            u,
            [38.0, 56.0],
            [-360.0, -520.0],
            parts::TOWER,
            part::HULL,
        );
        b.tone = 255;
        b.block(
            at,
            u,
            [42.0, 60.0],
            [-520.0, -532.0],
            parts::LIGHT,
            part::DARK,
        );
        b.tone = neon::BEACON;
        b.block(
            at,
            u,
            [10.0, 10.0],
            [-532.0, -566.0],
            parts::NEON,
            parts::NEON,
        );
    }
    b.tone = 255;
}
