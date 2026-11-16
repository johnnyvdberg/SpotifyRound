//! The stopovers between a jump's legs, worked out on the CPU for the frame:
//! their rows of the shader's parameters, a battle's fleets and effects, a
//! Dyson swarm's panels, the models each stopover places and its label.

use super::*;

/// The parts of an event drawn bounded instead of over the whole frame (see
/// `with_seed`), in the order drawn: each one's name, vertices and
/// instances. A Dyson swarm's panels are `panels` (`dyson_panels`): the
/// near side's up to the second, the far side's from the first. Parts with
/// nothing to show collapse in their vertex shaders.
pub(super) fn event_parts(
    event: voyage::Event,
    panels: [u32; 3],
) -> Option<Vec<(&'static str, u32, std::ops::Range<u32>)>> {
    match event {
        // The field, then the glows over it (`fs_gate_glint`).
        voyage::Event::Gate => Some(vec![
            ("field", GATE_SIDES * 3, 0..1),
            ("buoys", 6, 0..GATE_BUOYS),
            ("nodes", 6, 0..GATE_NODES),
        ]),
        // The hive cube's haze and tractor beam (`vs_cube_glow`): after its
        // meshes instead (`hive_glows`), tested against them.
        voyage::Event::HiveCube => Some(Vec::new()),
        // The near panels first, so nothing they hide is shaded.
        voyage::Event::DysonSwarm => {
            let ([near, far, all], n) = (panels, DYSON_PANEL_VERTICES);
            Some(vec![
                ("near", n, 0..near),
                ("near lit", n, 0..near),
                ("far", n, far..all),
                ("far lit", n, far..all),
                ("glow", 6, 0..1),
                ("star", 6, 0..1),
            ])
        }
        _ => None,
    }
}

/// A Dyson swarm's panel, placed for the frame (`PanelIn` in
/// `encounter.wesl`).
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub(super) struct Panel {
    pub(super) span: [f32; 4],
    pub(super) cell: [f32; 4],
}

/// A Dyson swarm's collector panels this frame, from the stopover's rows
/// (`ev`): three shells, two rings each about tilted axes of their own,
/// three rows of panels round each, a place for a panel a cell (its hash
/// may leave it out, in `vs_dyson_panel`); each ring turning, its cells
/// counted from where the longitude wraps, so the seam cuts a panel in two
/// as the trace once did. The near side's first, then those on the limb or
/// close (in both draws: the culling sorts their triangles), then the far
/// side's: the panels and where the limb's begin and end.
pub(super) fn dyson_panels(ev: &[[f32; 4]; EV_ROWS]) -> (Vec<Panel>, [u32; 3]) {
    use glam::Vec3;
    use std::f32::consts::{PI, TAU};
    let (t, seed) = (ev[EV_EVENT][1], ev[EV_EXTRA][0]);
    let ship = Vec3::new(ev[EV_SHIP][0], ev[EV_SHIP][1], ev[EV_SHIP][2]);
    let star = Vec3::new(ev[EV_OBJECT][0], ev[EV_OBJECT][1], ev[EV_OBJECT][2]);
    let mut sides: [Vec<Panel>; 3] = Default::default();
    for shell in 0..3 {
        let fs = shell as f32;
        let (rr, cells) = (ev[EV_OBJECT][3] * (0.7 + 0.2 * fs), 90.0 + 30.0 * fs);
        for k in 0..2 {
            let fk = k as f32 + fs * 2.0;
            let h = |n: f32| hash_cell([fk, seed, n]);
            let axis = (Vec3::new(h(1.0) - 0.5, 1.0, h(2.0) - 0.5)
                + Vec3::new(0.0, 0.0, 0.6 * (h(3.0) - 0.5)))
            .normalize();
            let width = 0.13 + 0.05 * h(4.0);
            let e1 = axis.cross(Vec3::Z).normalize();
            let e2 = axis.cross(e1);
            let turned = t * (0.02 + 0.02 * fs) * if k == 1 { -1.0 } else { 1.0 };
            let (lo, hi) = ((turned - PI) / TAU * cells, (turned + PI) / TAU * cells);
            let at = |g: f32, gy: f32| {
                let (lat, lon) = ((gy / 3.0 * 2.0 - 1.0) * width, g / cells * TAU - turned);
                (e1 * lon.cos() + e2 * lon.sin()) * (1.0 - lat * lat).sqrt() + axis * lat
            };
            for row in 0..3 {
                let fr = row as f32;
                for slot in 0..=cells as u32 {
                    let id = lo.floor() + slot as f32;
                    let (g0, g1) = ((id + 0.12).max(lo), (id + 0.88).min(hi));
                    if g1 <= g0 {
                        continue;
                    }
                    // Which way its corners face the ship, clearly.
                    let facing = [(g0, 0.14), (g1, 0.14), (g0, 0.86), (g1, 0.86)].map(|(g, y)| {
                        let n = at(g, fr + y);
                        n.dot((ship - star - n * rr).normalize())
                    });
                    let side = if facing.iter().all(|f| *f > 0.02) {
                        0
                    } else if facing.iter().all(|f| *f < -0.02) {
                        2
                    } else {
                        1
                    };
                    sides[side].push(Panel {
                        span: [g0, g1, cells, turned],
                        cell: [id, fr, fk, seed],
                    });
                }
            }
        }
    }
    let near = sides[0].len() as u32;
    let limb = near + sides[1].len() as u32;
    let panels: Vec<Panel> = sides.concat();
    let all = panels.len() as u32;
    (panels, [limb, near, all])
}

/// The index ranges of a placing's clusters that can show this frame
/// (`models::Cluster`), those next to each other merged: the camera as
/// `vs_model` places it, at the ship, its axes `params.right`, `up` and
/// `forward` in the world, the encounter frame's in `params.ev` rows `EV_X` to `EV_Z`.
pub(super) fn visible_runs(
    clusters: &[models::Cluster],
    instance: &models::Instance,
    params: &Params,
) -> Vec<std::ops::Range<u32>> {
    use glam::{Quat, Vec3};
    let v = |a: [f32; 4]| Vec3::new(a[0], a[1], a[2]);
    let ev = [v(params.ev[EV_X]), v(params.ev[EV_Y]), v(params.ev[EV_Z])];
    let camera = [v(params.right), v(params.up), v(params.forward)];
    // A direction in the encounter frame into the camera's.
    let to_camera = |d: Vec3| {
        let w = ev[0] * d.x + ev[1] * d.y + ev[2] * d.z;
        Vec3::new(w.dot(camera[0]), w.dot(camera[1]), w.dot(camera[2]))
    };
    let turn = Quat::from_array(instance.turn);
    let scale = instance.place[3];
    let place = v(instance.place) - v(params.ev[EV_SHIP]);
    let tan_half = params.frame[1];
    let mut runs: Vec<std::ops::Range<u32>> = Vec::new();
    for c in clusters {
        let centre = to_camera(turn * c.centre * scale + place);
        let axis = to_camera(turn * c.axis);
        if !c.visible(centre, axis, scale, tan_half) {
            continue;
        }
        match runs.last_mut() {
            Some(last) if last.end == c.first => last.end = c.first + c.count,
            _ => runs.push(c.first..c.first + c.count),
        }
    }
    runs
}

/// Every model the stopover in view has, placed for this moment, into
/// `out`: the asteroid field's rocks (each at the level of detail its size
/// on the frame calls for, too small ones left out), a star gate, a hive
/// cube and its escorts, a ring station and its debris, a battle's
/// wreckage. Each a mesh and where it goes (`fs_model`).
pub(super) fn model_instances(view: &View, out: &mut Vec<(models::Key, models::Instance)>) {
    use glam::{Mat3, Quat, Vec3};
    use models::{Instance, Key};
    out.clear();
    let Some(scene) = &view.scene else {
        return;
    };
    if scene.fade < 0.01 {
        return;
    }
    let shot = &scene.shot;
    let v3 = |a: [f64; 4]| Vec3::new(a[0] as f32, a[1] as f32, a[2] as f32);
    let ship = Vec3::new(
        shot.ship[0] as f32,
        shot.ship[1] as f32,
        shot.ship[2] as f32,
    );
    let t = scene.local as f32;
    // A pixel's angle, for the levels of detail.
    let pixel = (2.0 * view.half_fov / SIZE as f64) as f32;
    let seed = (scene.variant * 1e6) as u64;
    let unit = |rng: &mut system::Rng| rng.unit() as f32;
    let tumble = |rng: &mut system::Rng, rate: f32| {
        let axis =
            Vec3::new(unit(rng) - 0.5, unit(rng) - 0.5, unit(rng) - 0.5).normalize_or(Vec3::Y);
        Quat::from_axis_angle(
            axis,
            unit(rng) * std::f32::consts::TAU + t * rate * (0.5 + unit(rng)),
        )
    };
    let instance = |at: Vec3, size: f32, turn: Quat, look: [f32; 4]| Instance {
        place: [at.x, at.y, at.z, size],
        turn: turn.to_array(),
        look,
    };
    // The level of detail for a rock of `radius` at `at`: none if it would
    // be under a pixel.
    let lod = |at: Vec3, radius: f32| {
        let px = radius / (at - ship).length().max(1.0) / pixel;
        if px < 0.6 {
            None
        } else if px > 60.0 {
            Some(0)
        } else if px > 10.0 {
            Some(1)
        } else {
            Some(2)
        }
    };
    let o = |k: usize| shot.objects[k];
    match scene.event {
        voyage::Event::Asteroids => {
            // The light in each rock's own frame (`ev_light` turned back by
            // it: turned in its vertex shader, it cost the V3D half its
            // threads), and its kind of stone and shape (`bake_craters`)
            // in one.
            let kind = shot.extra[0] as f32;
            let light = Vec3::new(
                shot.extra[2] as f32,
                shot.extra[3] as f32,
                shot.rot[0] as f32,
            )
            .normalize();
            let look = |turn: Quat, variant: u32| {
                let l = turn.inverse() * light;
                [l.x, l.y, l.z, kind * models::ROCKS as f32 + variant as f32]
            };
            for k in 0..3 {
                let at = v3(o(k));
                let radius = o(k)[3] as f32;
                let mut rng = system::Rng::new(seed ^ 0x4e40 ^ k as u64);
                if let Some(level) = lod(at, radius) {
                    let variant = (k as u32 + 3) % models::ROCKS;
                    let key = Key::Rock {
                        variant,
                        lod: level,
                    };
                    let turn = tumble(&mut rng, 0.05);
                    out.push((key, instance(at, radius, turn, look(turn, variant))));
                }
            }
            let span = Vec3::new(460.0, 280.0, 820.0);
            let density = shot.extra[1] as f32;
            for i in 3..220u64 {
                let mut rng = system::Rng::new(seed ^ i.wrapping_mul(0x9e37_79b9));
                let h = [
                    unit(&mut rng),
                    unit(&mut rng),
                    unit(&mut rng),
                    unit(&mut rng),
                ];
                let radius = 0.7 + 12.3 * h[3] * h[3] * h[3];
                let mut xy =
                    glam::Vec2::new(h[0] - 0.5, h[1] - 0.5) * glam::Vec2::new(span.x, span.y);
                let clear = radius + 12.0 + 26.0 * h[2];
                if xy.length() < clear {
                    xy = xy.normalize_or(glam::Vec2::X) * clear;
                }
                let z = ship.z - 60.0
                    + ((h[2] * span.z - ship.z + 60.0) / span.z).rem_euclid(1.0) * span.z;
                let at = Vec3::new(xy.x, xy.y, z);
                // Rocks gather in clumps, thinning out between them.
                let cell = |scale: f32| {
                    let c = (at / scale).floor();
                    system::Rng::new(
                        seed ^ ((c.x as i64 as u64).wrapping_mul(73_856_093)
                            ^ (c.y as i64 as u64).wrapping_mul(19_349_663)
                            ^ (c.z as i64 as u64).wrapping_mul(83_492_791)),
                    )
                    .unit() as f32
                };
                let clump = cell(150.0) * 0.6 + cell(70.0) * 0.4;
                if unit(&mut rng) > density * (0.35 + 1.15 * clump) {
                    continue;
                }
                if let Some(level) = lod(at, radius) {
                    let variant = (i % models::ROCKS as u64) as u32;
                    let key = Key::Rock {
                        variant,
                        lod: level,
                    };
                    let turn = tumble(&mut rng, 0.08);
                    out.push((key, instance(at, radius, turn, look(turn, variant))));
                }
            }
        }
        voyage::Event::Gate => {
            let style = (shot.extra[0] as u32).min(2);
            let normal =
                Vec3::new(shot.rot[0] as f32, shot.rot[1] as f32, shot.rot[2] as f32).normalize();
            let spin = shot.rot[3] as f32;
            let e1 = normal.cross(Vec3::Y).normalize_or(Vec3::X);
            let e2 = normal.cross(e1);
            let frame = |spin: f32| {
                let (s, c) = spin.sin_cos();
                Quat::from_mat3(&Mat3::from_cols(e1 * c + e2 * s, -e1 * s + e2 * c, normal))
            };
            let look = [
                style as f32,
                shot.extra[1] as f32,
                shot.extra[2] as f32,
                0.0,
            ];
            out.push((
                Key::Gate { style, part: 0 },
                instance(v3(o(0)), o(0)[3] as f32, frame(spin), look),
            ));
            if style == 2 {
                out.push((
                    Key::Gate { style, part: 1 },
                    instance(v3(o(0)), o(0)[3] as f32, frame(-spin * 2.0), look),
                ));
            }
        }
        voyage::Event::HiveCube => {
            let axis =
                Vec3::new(shot.rot[0] as f32, shot.rot[1] as f32, shot.rot[2] as f32).normalize();
            let look = [shot.extra[0] as f32, shot.extra[1] as f32, 0.0, 0.0];
            for k in 0..3 {
                if o(k)[3] <= 0.0 {
                    continue;
                }
                let turn = Quat::from_axis_angle(axis, shot.rot[3] as f32 * (1.0 + k as f32 * 0.6));
                // Its finest machinery is 0.037 of its half-size across: the
                // coarser mesh while that is under two pixels (the escorts,
                // mostly). A third of a million triangles a cube cost the
                // V3D's binner whatever their size on the frame.
                let at = v3(o(k));
                let px = o(k)[3] as f32 / (at - ship).length().max(1.0) / pixel;
                let key = Key::Hive {
                    variant: (shot.extra[3] as u32 + k as u32) % 3,
                    lod: if px * 0.037 < 2.0 { 1 } else { 0 },
                };
                out.push((key, instance(v3(o(k)), o(k)[3] as f32, turn, look)));
            }
        }
        voyage::Event::Derelict => {
            let axis =
                Vec3::new(shot.rot[0] as f32, shot.rot[1] as f32, shot.rot[2] as f32).normalize();
            let spin = shot.rot[3] as f32;
            let e1 = axis.cross(Vec3::Y).normalize_or(Vec3::X);
            let e2 = axis.cross(e1);
            let (s, c) = spin.sin_cos();
            let turn = Quat::from_mat3(&Mat3::from_cols(e1 * c + e2 * s, -e1 * s + e2 * c, axis));
            let gaps = (o(1)[1] as u32).clamp(2, 4);
            let look = [
                shot.extra[0] as f32,
                shot.extra[1] as f32,
                shot.extra[2] as f32,
                0.0,
            ];
            let centre = v3(o(0));
            let radius = o(0)[3] as f32;
            out.push((
                Key::Ring {
                    gaps,
                    variant: (shot.extra[3] as u32) % 2,
                },
                instance(centre, radius, turn, look),
            ));
            for i in 0..80u64 {
                let mut rng = system::Rng::new(seed ^ 0xdeb ^ i.wrapping_mul(0x51af));
                let h = [
                    unit(&mut rng),
                    unit(&mut rng),
                    unit(&mut rng),
                    unit(&mut rng),
                ];
                let a = h[0] * std::f32::consts::TAU + spin * (0.8 + 0.4 * h[2]);
                let r = radius * (0.75 + 0.5 * h[1]);
                let fi = i as f32;
                let drift = Vec3::new(
                    (t * 0.1 + fi).sin(),
                    (t * 0.13 + fi * 1.7).cos(),
                    (t * 0.07 + fi * 2.3).sin(),
                ) * 3.0;
                let at = centre
                    + (e1 * a.cos() + e2 * a.sin()) * r
                    + axis * (h[2] - 0.5) * 120.0
                    + drift;
                let size = 0.5 + 5.0 * h[3] * h[3] * h[3];
                if lod(at, size).is_some() {
                    let key = Key::Debris {
                        variant: (i % models::DEBRIS as u64) as u32,
                    };
                    out.push((
                        key,
                        instance(
                            at,
                            size,
                            tumble(&mut rng, 0.15),
                            [-1.0, 0.0, shot.extra[0] as f32, 0.0],
                        ),
                    ));
                }
            }
        }
        voyage::Event::Battle => {
            let kill = o(1)[2] as f32;
            let age = t - kill - 0.35;
            if age > 0.0 {
                let lost = o(2)[3] as u32;
                let faction = if lost < 4 { o(1)[0] } else { o(1)[1] } as f32;
                let blast = Vec3::new(shot.rot[0] as f32, shot.rot[1] as f32, shot.rot[2] as f32);
                let heat = (-age / 4.0).exp();
                for i in 0..60u64 {
                    let mut rng = system::Rng::new(seed ^ 0x3ec ^ i.wrapping_mul(0x2545));
                    let dir = Vec3::new(
                        unit(&mut rng) - 0.5,
                        (unit(&mut rng) - 0.5) * 0.6,
                        unit(&mut rng) - 0.5,
                    )
                    .normalize_or(Vec3::X);
                    let speed = 10.0 + 24.0 * unit(&mut rng);
                    let at = blast + dir * speed * age;
                    let h = unit(&mut rng);
                    let size = 0.3 + 2.2 * h * h * h;
                    if lod(at, size).is_some() {
                        let key = Key::Debris {
                            variant: (i % models::DEBRIS as u64) as u32,
                        };
                        out.push((
                            key,
                            instance(at, size, tumble(&mut rng, 0.6), [faction, heat, 0.0, 0.0]),
                        ));
                    }
                }
            }
        }
        _ => {}
    }
}

/// The label on a stopover's main object, where it is in view and clear of
/// the panels: its name and range, a leader up from its edge.
pub(super) fn encounter_label(view: &View, panels: &[[f64; 4]]) -> Option<labels::BodyLabel> {
    let scene = view.scene.as_ref()?;
    if view.reveal < 1.0 || view.warp > 0.02 || scene.fade < 0.5 {
        return None;
    }
    let shot = &scene.shot;
    let o = shot.objects[0];
    let name = match scene.event {
        voyage::Event::Asteroids | voyage::Event::Nebula | voyage::Event::Remnant => return None,
        voyage::Event::HiveCube => "Hive cube".to_string(),
        voyage::Event::BlackHole => "Black hole".into(),
        voyage::Event::RoguePlanet => "Rogue planet".into(),
        voyage::Event::Derelict => "Derelict station".into(),
        voyage::Event::Comet => "Comet nucleus".into(),
        voyage::Event::Leviathans => "Leviathan pod".into(),
        voyage::Event::Gate => "Star gate".into(),
        voyage::Event::Wormhole => "Wormhole".into(),
        voyage::Event::Battle => "Battle".into(),
        voyage::Event::PlasmaStorm => "Storm eye".into(),
        voyage::Event::DysonSwarm => "Dyson swarm".into(),
    };
    let [x, y, z] = scene.basis;
    let rel = sub([o[0], o[1], o[2]], shot.ship);
    let world = add(add(scale(x, rel[0]), scale(y, rel[1])), scale(z, rel[2]));
    let depth = length(world);
    let size = match scene.event {
        voyage::Event::HiveCube => o[3] * 1.4,
        voyage::Event::Leviathans => 40.0,
        voyage::Event::Comet => 12.0,
        voyage::Event::Battle => 30.0,
        _ => o[3],
    };
    let ahead = dot(world, view.forward);
    if ahead <= size * 1.2 || depth <= size * 1.2 {
        return None;
    }
    let right = cross(view.forward, view.up);
    let half = SIZE as f64 / 2.0;
    let sx = dot(world, right) / ahead / view.half_fov;
    let sy = dot(world, view.up) / ahead / view.half_fov;
    let radius = (size / depth).asin().tan() / view.half_fov * half;
    // Too big to point at, or off the round frame.
    if radius > half * 0.55 || sx.hypot(sy) > 0.8 {
        return None;
    }
    let centre = [half + sx * half, half - sy * half];
    let left = centre[0] > half * 1.3;
    let lean = std::f64::consts::FRAC_1_SQRT_2 * radius;
    let at = [
        centre[0] + if left { -lean } else { lean },
        centre[1] - lean,
    ];
    if panels.iter().any(|p| {
        at[0] > p[0] - 20.0 && at[0] < p[2] + 20.0 && at[1] > p[1] - 70.0 && at[1] < p[3] + 20.0
    }) {
        return None;
    }
    Some(labels::BodyLabel {
        at: [at[0] as f32, at[1] as f32],
        left,
        name,
        distance: format!("{} km", tour::thousands((depth - size).max(0.0) * 10.0)),
    })
}

/// WGSL's `hash_cell` (`frame.wesl`), to the same bits.
pub(super) fn hash_cell(p: [f32; 3]) -> f32 {
    let fract = |x: f32| x - x.floor();
    let mut q = [
        fract(p[0] * 0.1031),
        fract(p[1] * 0.1030),
        fract(p[2] * 0.0973),
    ];
    let k = q[0] * (q[1] + 33.33) + q[1] * (q[2] + 33.33) + q[2] * (q[0] + 33.33);
    q = q.map(|x| x + k);
    fract((q[0] + q[1]) * q[2])
}

/// A battle's ships where they are this frame, as `battle_ship` in
/// `encounter.wesl` would place them, into rows 12 on of the stopover's
/// parameters: two rows a ship, where it is and its size, which way it heads
/// and how hard the shock strikes it. Worked out once here rather than for
/// every pixel and vertex.
pub(super) fn fleet_rows(ev: &mut [[f32; 4]; EV_ROWS]) {
    if ev[EV_EVENT][0] as u32 != voyage::Event::Battle.id() {
        return;
    }
    let t = ev[EV_EVENT][1];
    let (o, seed) = (ev[EV_OBJECT], ev[EV_EXTRA][0]);
    let (fa, fb) = (ev[EV_OBJECT + 1][0] as usize, ev[EV_OBJECT + 1][1] as usize);
    let faction = |i: usize| if i >= 4 { fb } else { fa };
    let size = |i: usize| {
        const BIG: [f32; 6] = [13.0, 32.0, 9.5, 27.0, 9.5, 40.0];
        const SMALL: [f32; 6] = [7.0, 4.0, 2.4, 13.0, 6.5, 9.0];
        let f = faction(i).min(5);
        if i == 0 || i == 4 { BIG[f] } else { SMALL[f] }
    };
    let norm = |v: [f32; 3]| {
        let l = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt().max(1e-9);
        v.map(|x| x / l)
    };
    let course = |i: usize, t: f32| -> ([f32; 3], [f32; 3]) {
        let fi = i as f32;
        let fleet = if i >= 4 { -1.0 } else { 1.0 };
        let capital = i == 0 || i == 4;
        let h = [1.0, 2.0, 3.0, 4.0].map(|k| hash_cell([fi, seed, k]));
        let rate = if capital {
            0.015 * fleet
        } else {
            (0.06 + 0.05 * h[0]) * fleet
        };
        let a = t * rate
            + if capital {
                0.0
            } else {
                h[1] * std::f32::consts::TAU
            }
            + if i >= 4 { std::f32::consts::PI } else { 0.0 }
            + 0.6;
        let r = if capital {
            o[3] * 0.55
        } else {
            o[3] * (0.4 + 0.5 * h[2])
        };
        let y = if capital {
            (h[3] - 0.5) * o[3] * 0.15
        } else {
            (h[3] - 0.5) * o[3] * 0.45
        } + (t * 0.4 + fi).sin() * 3.0;
        let p = [o[0] + a.cos() * r, o[1] + y, o[2] + a.sin() * r * 0.65];
        let heading = if capital {
            norm([-a.sin(), 0.0, a.cos()])
        } else {
            norm([
                -a.sin() * fleet,
                0.05 * (t * 0.4 + fi).cos(),
                a.cos() * 0.65 * fleet,
            ])
        };
        (p, heading)
    };
    let (kill, lost) = (ev[EV_OBJECT + 1][2], ev[EV_OBJECT + 2][3] as i32);
    let (blast, speed) = ([ev[EV_ROT][0], ev[EV_ROT][1], ev[EV_ROT][2]], ev[EV_ROT][3]);
    let ship = [ev[EV_SHIP][0], ev[EV_SHIP][1], ev[EV_SHIP][2]];
    for i in 0..BATTLE_SHIPS {
        let s = size(i);
        let (at, heading, jolt) = if ev[EV_EXTRA][1] > 0.5 {
            // The screenshots' showcase: the one ship asked for, alone,
            // ahead of the camera at a distance by its size, three-quarters.
            let shown = i as f32 == ev[EV_EXTRA][2];
            let at = [
                ship[0],
                ship[1],
                ship[2] + if shown { s * 3.3 } else { -1e4 },
            ];
            (at, norm([-0.78, -0.18, -0.6]), 0.0)
        } else if i as i32 == lost {
            // Flown to the blast as it dies.
            let (p, heading) = course(i, t);
            let then = course(i, kill).0;
            (
                std::array::from_fn(|k| p[k] + blast[k] - then[k]),
                heading,
                0.0,
            )
        } else {
            // Shoved aside by its shock as it passes.
            let (p, heading) = course(i, t);
            let away: [f32; 3] = std::array::from_fn(|k| p[k] - blast[k]);
            let dist = (away[0] * away[0] + away[1] * away[1] + away[2] * away[2]).sqrt();
            let away = norm(away);
            let age = t - kill - dist / speed;
            let strength = 26.0 / (1.0 + s / 6.0) * (-dist / 260.0).exp();
            let x = (age / 2.2).clamp(0.0, 1.0);
            let shove = strength * x * x * (3.0 - 2.0 * x);
            let jolt = (-(age / 0.5).powi(2)).exp() * if age >= -1.0 { 1.0 } else { 0.0 };
            let at = std::array::from_fn(|k| p[k] + away[k] * shove);
            (
                at,
                norm(std::array::from_fn(|k| heading[k] + away[k] * 0.35 * jolt)),
                jolt,
            )
        };
        ev[EV_FLEET + 2 * i] = [at[0], at[1], at[2], s];
        ev[EV_FLEET + 1 + 2 * i] = [heading[0], heading[1], heading[2], jolt];
    }
}

/// One of a battle's effects this frame, drawn on its own bounded geometry
/// (`vs_fx_*` in `encounter.wesl`): four rows its kind reads its own way,
/// the last how far its light reaches (pixels round it, units round it, how
/// far its front stands towards the camera, and the part of that within
/// which its light is bright, [`FX_BRIGHT`]).
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, bytemuck::Pod, bytemuck::Zeroable)]
pub(super) struct Fx {
    pub(super) a: [f32; 4],
    pub(super) b: [f32; 4],
    pub(super) colour: [f32; 4],
    pub(super) reach: [f32; 4],
}

/// The most effects drawn in a battle's frame.
pub(super) const FX_INSTANCES: usize = 128;

/// The kinds of a battle's effects, a pipeline each (`battlefx<kind>`): its
/// label's part, its vertex and fragment shaders and vertices an instance.
pub(super) const FX_KINDS: [(&str, &str, &str, u32); 6] = [
    ("beams", "vs_fx_line", "fs_fx_line", 6 * 8),
    ("glints", "vs_fx_spot", "fs_fx_glint", 6),
    ("bursts", "vs_fx_spot", "fs_fx_burst", 6),
    ("shields", "vs_fx_spot", "fs_fx_shield", 6),
    ("shock shields", "vs_fx_spot", "fs_fx_jolt", 6),
    ("shock wave", "vs_fx_ring", "fs_fx_ring", 6 * 64),
];
pub(super) const FX_BEAM: usize = 0;
pub(super) const FX_GLINT: usize = 1;
pub(super) const FX_BURST: usize = 2;
pub(super) const FX_SHIELD: usize = 3;
pub(super) const FX_JOLT: usize = 4;
pub(super) const FX_WAVE: usize = 5;

/// Linear light under which the sRGB frame rounds a pixel to black: where
/// each effect's glow is cut.
pub(super) const FX_DIM: f32 = 0.5 / 255.0 / 12.92;
/// Light under which an effect is dim (`FX_BRIGHT` in `encounter.wesl`):
/// drawn behind every mesh, not at its own depth.
pub(super) const FX_BRIGHT: f32 = 0.015;

/// How far out a glow `peak` bright at its centre stays above `floor`, in
/// its falloff's own lengths: for `exp(-x)`, `x` itself.
pub(super) fn fx_falloff(peak: f32, floor: f32) -> f32 {
    (peak / floor).max(1.0).ln()
}

/// A battle's effects this frame, by kind ([`FX_KINDS`]), as
/// `encounter.wesl` once worked them out for every pixel: the ships'
/// engines, shields flaring where the shock strikes them; fire between the
/// fleets, each its own way, the target's shield flaring where it lands;
/// torpedoes arcing over and bursting on the shields; fighters weaving round
/// the wedge fleet; the lost ship's blast and its shock; stray fire past the
/// ship. From the stopover's rows, `fleet_rows` already filled in.
pub(super) fn battle_effects(ev: &[[f32; 4]; EV_ROWS]) -> [Vec<Fx>; 6] {
    use std::f32::consts::{PI, TAU};
    type V = [f32; 3];
    let mut out: [Vec<Fx>; 6] = Default::default();
    let fract = |x: f32| x - x.floor();
    let v3 = |r: [f32; 4]| [r[0], r[1], r[2]];
    let add = |a: V, b: V| [a[0] + b[0], a[1] + b[1], a[2] + b[2]];
    let sub = |a: V, b: V| [a[0] - b[0], a[1] - b[1], a[2] - b[2]];
    let scale = |a: V, k: f32| a.map(|x| x * k);
    let length = |a: V| (a[0] * a[0] + a[1] * a[1] + a[2] * a[2]).sqrt();
    let norm = |a: V| scale(a, 1.0 / length(a).max(1e-9));
    let mix = |a: V, b: V, k: f32| add(a, scale(sub(b, a), k));
    let white = [1.0; 3];
    let brightest = |c: V| c[0].max(c[1]).max(c[2]);
    // A beam or bolt from `a` to `b`, `w` pixels wide; dashes at `dash`, or
    // steady if negative.
    let beam = |out: &mut [Vec<Fx>; 6], a: V, b: V, w: f32, colour: V, dash: f32| {
        let peak = brightest(colour);
        if peak < FX_DIM {
            return;
        }
        let span = length(sub(b, a));
        let m = scale(sub(b, a), 1.0 / span.max(1e-3));
        let reach = |floor| {
            (5.0 * w * fx_falloff(0.12 * peak, floor))
                .max(w * (2.0 * fx_falloff(1.6 * peak, floor)).sqrt())
                + 1.0
        };
        out[FX_BEAM].push(Fx {
            a: [a[0], a[1], a[2], w],
            b: [m[0], m[1], m[2], span],
            colour: [colour[0], colour[1], colour[2], dash],
            reach: [reach(FX_DIM), 0.0, 0.0, reach(FX_BRIGHT) / reach(FX_DIM)],
        });
    };
    // A point's glint at `p`, `size` pixels.
    let glint = |out: &mut [Vec<Fx>; 6], p: V, size: f32, colour: V| {
        let peak = brightest(colour);
        if peak < FX_DIM {
            return;
        }
        let reach = |floor| {
            (6.0 * size * fx_falloff(0.15 * peak, floor))
                .max(size * (2.0 * fx_falloff(2.0 * peak, floor)).sqrt())
                + 1.0
        };
        out[FX_GLINT].push(Fx {
            a: [p[0], p[1], p[2], size],
            colour: [colour[0], colour[1], colour[2], 0.0],
            reach: [reach(FX_DIM), 0.0, 0.0, reach(FX_BRIGHT) / reach(FX_DIM)],
            ..Default::default()
        });
    };
    // A burst at `c` of radius `r`, `age` 0..1 through it, its light times
    // `k`, its front `front` units nearer than its centre.
    let burst = |out: &mut [Vec<Fx>; 6], c: V, r: f32, age: f32, k: f32, front: f32| {
        if age <= 0.0 || age >= 1.0 {
            return;
        }
        let size = r * (0.25 + 0.75 * age.sqrt());
        let fade = (1.0 - age) * (1.0 - age) * k;
        let reach = |floor| {
            (size * 0.55 * fx_falloff(2.2 * fade, floor).sqrt())
                .max(size * (1.0 + 0.18 * fx_falloff(0.35 * fade, floor).sqrt()))
                .max(r * fx_falloff(0.04 * fade, floor))
                * 1.02
        };
        out[FX_BURST].push(Fx {
            a: [c[0], c[1], c[2], r],
            b: [0.0, 0.0, 0.0, age],
            colour: [k, k, k, 0.0],
            reach: [0.0, reach(FX_DIM), front, reach(FX_BRIGHT) / reach(FX_DIM)],
        });
    };
    // A shield of radius `r` round `c` flaring towards `toward`, `power`
    // how tightly; or, `power` 0, struck by the shock (`fs_fx_jolt`).
    let shield = |out: &mut [Vec<Fx>; 6], c: V, r: f32, toward: V, power: f32, colour: V| {
        if brightest(colour) < FX_DIM {
            return;
        }
        out[if power > 0.0 { FX_SHIELD } else { FX_JOLT }].push(Fx {
            a: [c[0], c[1], c[2], r],
            b: [toward[0], toward[1], toward[2], power],
            colour: [colour[0], colour[1], colour[2], 0.0],
            reach: [0.0, r, r, 1.0],
        });
    };
    let fire = |f: usize| {
        [
            [1.0, 0.5, 0.12],
            [0.3, 1.0, 0.3],
            [0.45, 1.0, 0.25],
            [0.3, 1.0, 0.55],
            [1.0, 0.8, 0.2],
            [0.25, 1.0, 0.4],
        ][f]
    };
    let shield_colour = |f: usize| {
        [
            [0.4, 0.7, 1.0],
            [0.7, 0.8, 1.0],
            [0.7, 1.0, 0.4],
            [0.3, 1.0, 0.5],
            [1.0, 0.7, 0.3],
            [0.3, 1.0, 0.4],
        ][f]
    };
    let faction = |i: usize| {
        ((if i >= 4 {
            ev[EV_OBJECT + 1][1]
        } else {
            ev[EV_OBJECT + 1][0]
        }) as i32)
            .clamp(0, 5) as usize
    };
    let at = |i: usize| ev[EV_FLEET + 2 * i];
    let ship = v3(ev[EV_SHIP]);
    let t = ev[EV_EVENT][1];
    let seed = ev[EV_EXTRA][0];
    let (kill, lost) = (ev[EV_OBJECT + 1][2], ev[EV_OBJECT + 2][3] as i32);
    let blast = v3(ev[EV_ROT]);
    for i in 0..BATTLE_SHIPS {
        if i as i32 == lost && t > kill {
            continue;
        }
        let (c, size) = (v3(at(i)), at(i)[3]);
        let (heading, jolt) = (v3(ev[EV_FLEET + 1 + 2 * i]), ev[EV_FLEET + 1 + 2 * i][3]);
        let f = faction(i);
        if jolt > 0.01 {
            let toward = norm(sub(blast, c));
            shield(
                &mut out,
                c,
                size * 1.45,
                toward,
                0.0,
                scale(shield_colour(f), jolt),
            );
        }
        let engine = sub(c, scale(heading, size * 1.02));
        let colour = scale(mix(shield_colour(f), white, 0.5), 0.9);
        glint(&mut out, engine, 1.0 + size * 0.03, colour);
    }
    // Fire between the fleets: beams, or bolts in short dashes for the wedge
    // fleet and the raiders. Not for the screenshots' one ship.
    for k in (0..7).filter(|_| ev[EV_EXTRA][1] <= 0.5) {
        let fk = k as f32;
        let clock = t * 1.6 + fk * 0.37;
        let on = fract(clock);
        if on > 0.35 {
            continue;
        }
        let h = hash_cell([clock.floor(), fk, seed]);
        let shooter = (h * 7.99) as i32;
        let aim = (shooter / 4 * 4 + 4 + (fract(h * 13.0) * 3.99) as i32) % 8;
        if (shooter == lost || aim == lost) && t > kill {
            continue;
        }
        let (a, b) = (at(shooter as usize), at(aim as usize));
        let f = faction(shooter as usize);
        let missing = if fract(h * 7.0) >= 0.6 { 1.0 } else { 0.0 };
        let miss = [31.0, 57.0, 91.0].map(|n| (fract(h * n) - 0.5) * b[3] * 3.0 * missing);
        let from = add(v3(a), scale(norm(sub(v3(b), v3(a))), a[3] * 0.6));
        let to = add(v3(b), miss);
        let pulse = (on / 0.35 * PI).sin();
        let colour = scale(mix(fire(f), white, 0.4), 1.4 * pulse);
        let dash = if f == 1 || f == 2 { on * 9.0 } else { -1.0 };
        beam(
            &mut out,
            from,
            to,
            if f == 5 { 1.8 } else { 1.1 },
            colour,
            dash,
        );
        // Where a beam lands, its target's shield flares.
        if fract(h * 7.0) < 0.6 && on > 0.05 {
            let toward = norm(sub(v3(a), v3(b)));
            let colour = scale(shield_colour(faction(aim as usize)), 0.7 * pulse);
            shield(&mut out, v3(b), b[3] * 1.45, toward, 5.0, colour);
        }
    }
    // Torpedoes: glowing slugs arcing over with their trails, bursting on
    // the target's shield.
    for k in 0..5 {
        let fk = k as f32;
        let clock = t / 2.6 + fk * 0.23;
        let phase = fract(clock);
        let h = hash_cell([clock.floor(), fk + 10.0, seed]);
        let shooter = (h * 7.99) as i32;
        let aim = (shooter / 4 * 4 + 4 + (fract(h * 17.0) * 3.99) as i32) % 8;
        if (shooter == lost || aim == lost) && t > kill - 2.0 {
            continue;
        }
        let a = v3(at(shooter as usize));
        let goal = at(aim as usize);
        let arc = |flight: f32| {
            add(
                mix(a, v3(goal), flight),
                [0.0, (flight * PI).sin() * 25.0, 0.0],
            )
        };
        let tint = mix(fire(faction(shooter as usize)), [1.0, 0.25, 0.1], 0.5);
        if phase < 0.7 {
            let flight = (phase / 0.7).clamp(0.0, 1.0);
            let p = arc(flight);
            let pulse = 0.75 + 0.25 * (t * 30.0 + fk).sin();
            glint(&mut out, p, 2.0, scale(tint, 1.6 * pulse));
            let back = arc((flight - 0.12).max(0.0));
            beam(&mut out, back, p, 1.0, scale(tint, 0.35), -1.0);
        } else {
            let age = (phase - 0.7) / 0.3;
            let toward = norm(sub(a, v3(goal)));
            let landing = add(v3(goal), scale(toward, goal[3] * 1.45));
            burst(
                &mut out,
                landing,
                (goal[3] * 0.7).max(4.0),
                age,
                0.6,
                goal[3],
            );
            let colour = scale(
                shield_colour(faction(aim as usize)),
                (1.0 - age).powi(2) * 0.9,
            );
            shield(&mut out, v3(goal), goal[3] * 1.45, toward, 4.0, colour);
        }
    }
    // Fighters weaving round the wedge fleet, bolts flicking between them.
    if ev[EV_OBJECT + 1][0] as i32 == 1 || ev[EV_OBJECT + 1][1] as i32 == 1 {
        let o = ev[EV_OBJECT];
        for k in 0..10 {
            let fk = k as f32;
            let h = hash_cell([fk, seed, 21.0]);
            let a = t * (0.5 + 0.4 * h) + h * TAU;
            let p = add(
                v3(o),
                [
                    a.cos() * o[3] * (0.3 + 0.5 * h),
                    (a * 1.7 + fk).sin() * o[3] * 0.25,
                    a.sin() * o[3] * 0.5 * (0.4 + 0.6 * fract(h * 7.0)),
                ],
            );
            let way = norm([
                -a.sin() * o[3],
                (a * 1.7 + fk).cos() * 1.7 * o[3] * 0.25,
                a.cos() * o[3] * 0.5,
            ]);
            let tint = if k < 5 {
                [0.6, 0.85, 1.0]
            } else {
                [1.0, 0.5, 0.3]
            };
            glint(&mut out, p, 1.0, scale(tint, 1.2));
            beam(
                &mut out,
                sub(p, scale(way, 8.0)),
                p,
                0.6,
                scale(tint, 0.25),
                -1.0,
            );
            if fract(t * 2.0 + h * 5.0) <= 0.15 {
                let shot = if k < 5 {
                    [1.0, 0.25, 0.2]
                } else {
                    [0.3, 1.0, 0.3]
                };
                let (from, to) = (add(p, scale(way, 2.0)), add(p, scale(way, 30.0)));
                beam(&mut out, from, to, 0.8, shot, -1.0);
            }
        }
    }
    // The ship lost: a flash, a fireball and a ring of shock in its plane.
    let age = (t - kill) / 5.0;
    burst(&mut out, blast, 30.0, age, 1.4, 30.0 * age.max(0.0).sqrt());
    if t >= kill {
        let flash = scale([1.0, 0.95, 0.9], (-(t - kill) * 3.0).exp() * 3.0);
        glint(&mut out, blast, 6.0, flash);
    }
    if age > 0.0 && age < 1.0 {
        let wave = (t - kill) * ev[EV_ROT][3];
        let colour = scale([0.7, 0.85, 1.0], (1.0 - age).powi(2) * 0.7);
        // Bright within this part of its band (`bump`).
        let bright = (1.0 - (FX_BRIGHT / brightest(colour)).min(1.0).sqrt()).sqrt();
        out[FX_WAVE].push(Fx {
            a: [blast[0], blast[1], blast[2], wave],
            b: [4.0 + wave * 0.05, 0.0, 0.0, 0.0],
            colour: [colour[0], colour[1], colour[2], 0.0],
            reach: [0.0, 0.0, 0.0, (bright + 0.05).min(1.0)],
        });
    }
    // Stray fire: a beam past the ship, a torpedo bursting close by.
    let (laser_at, torpedo_at, side) = (
        ev[EV_OBJECT + 2][0],
        ev[EV_OBJECT + 2][1],
        ev[EV_OBJECT + 1][3],
    );
    if t > laser_at && t < laser_at + 0.6 {
        let a = v3(at(4));
        let near = add(ship, [side * 14.0, 6.0, 28.0]);
        let end = add(near, scale(sub(near, a), 0.5));
        let pulse = ((t - laser_at) / 0.6 * PI).sin();
        let colour = scale(mix(fire(faction(4)), white, 0.5), 2.0 * pulse);
        beam(&mut out, a, end, 1.8, colour, -1.0);
    }
    if t > torpedo_at && t < torpedo_at + 3.5 {
        let a = v3(at(0));
        let near = add(ship, [-side * 22.0, -8.0, 40.0]);
        let flight = (t - torpedo_at) / 1.1;
        if flight < 1.0 {
            glint(&mut out, mix(a, near, flight), 2.4, [2.0, 0.8, 0.4]);
        } else {
            burst(
                &mut out,
                near,
                22.0,
                (t - torpedo_at - 1.1) / 2.4,
                1.2,
                10.0,
            );
        }
    }
    out
}

/// The stopover's parameters for the shaders (the `EV_*` rows of [`layout`]):
/// the event, how far into it and its draw; the ship in its frame, and the
/// frame's axes; its objects, rotation and own numbers; the alarm, how much
/// of it shows and what is seen inside a leg; and where a hit lands.
pub(super) fn encounter_rows(view: &View) -> [[f32; 4]; EV_ROWS] {
    let mut ev = [[0.0f32; 4]; EV_ROWS];
    let v4 = |v: [f64; 3], w: f64| [v[0] as f32, v[1] as f32, v[2] as f32, w as f32];
    let x4 = |v: [f64; 4]| v.map(|x| x as f32);
    if let Some(scene) = &view.scene {
        let shot = &scene.shot;
        ev[EV_EVENT] = [
            scene.event.id() as f32,
            scene.local as f32,
            scene.duration as f32,
            scene.variant as f32,
        ];
        ev[EV_SHIP] = v4(shot.ship, 1.0);
        ev[EV_X] = v4(scene.basis[0], shot.hit);
        ev[EV_Y] = v4(scene.basis[1], shot.beam);
        ev[EV_Z] = v4(scene.basis[2], shot.flash);
        for (k, object) in shot.objects.iter().enumerate() {
            ev[EV_OBJECT + k] = x4(*object);
        }
        ev[EV_ROT] = x4(shot.rot);
        ev[EV_EXTRA] = x4(shot.extra);
        ev[EV_STATE] = [shot.alert as f32, scene.fade as f32, 0.0, 0.0];
        ev[EV_HIT] = [
            shot.hit_at[0] as f32,
            shot.hit_at[1] as f32,
            shot.shake as f32,
            0.0,
        ];
    }
    if let Some((kind, amount, local)) = view.flux {
        ev[EV_STATE][2] = kind as f32;
        ev[EV_STATE][3] = amount as f32;
        ev[EV_HIT][3] = local as f32;
    }
    if let Some(base) = &view.base {
        super::starbase::rows(base, &mut ev);
    }
    ev[EV_Z][3] = ev[EV_Z][3].max(view.flash as f32);
    ev[EV_SHIP][3] = if view.deep { 1.0 } else { 0.0 };
    ev
}
