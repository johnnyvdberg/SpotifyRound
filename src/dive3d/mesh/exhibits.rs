//! The drowned museum's own models (`world::museum`): its specimens, the
//! scans the Smithsonian's natural history museum made of them (assets/models
//! /CREDITS.md), and what the halls are built and hung with that no other sea
//! has: the whale skeleton under the great hall's vault, the canvas every
//! painting is drawn on, the arches of its arcades and the minerals in the
//! second hall's cases. None of them is in the library the dive starts with:
//! the renderer builds them as the museum comes near and drops them once it is
//! left behind ([`Model::is_streamed`]).

use super::*;

/// The specimens, each a unit tall, centred on its footprint and standing on
/// y = 0, the casts' and bones' crust of the sea's growth at the foot left as
/// it is: the hall has been under water as long as the city round it.
pub(super) fn specimen(obj: &str) -> Mesh {
    cast(obj, plinth_post())
}

const MAMMOTH: &str = include_str!("../../../assets/models/museum-mammoth.obj");
const GROUND_SLOTH: &str = include_str!("../../../assets/models/museum-ground-sloth.obj");
const TRICERATOPS: &str = include_str!("../../../assets/models/museum-triceratops.obj");
const MEGALODON: &str = include_str!("../../../assets/models/museum-megalodon.obj");
const ARCHAEOPTERYX: &str = include_str!("../../../assets/models/museum-archaeopteryx.obj");
const WALRUS_WHALE: &str = include_str!("../../../assets/models/museum-walrus-whale.obj");
const PILOT_WHALE: &str = include_str!("../../../assets/models/museum-pilot-whale.obj");
const TORTOISE: &str = include_str!("../../../assets/models/museum-tortoise.obj");
const PORCUPINEFISH: &str = include_str!("../../../assets/models/museum-porcupinefish.obj");
const MUREX: &str = include_str!("../../../assets/models/museum-murex.obj");
const DIPLOCAULUS: &str = include_str!("../../../assets/models/museum-diplocaulus.obj");
const STEGOSAURUS_SPIKE: &str = include_str!("../../../assets/models/museum-stegosaurus-spike.obj");
const DINICTIS: &str = include_str!("../../../assets/models/museum-dinictis.obj");
const TRILOBITE: &str = include_str!("../../../assets/models/museum-trilobite.obj");
const DEER_SKULL: &str = include_str!("../../../assets/models/museum-deer-skull.obj");
const KING_CRAB: &str = include_str!("../../../assets/models/museum-king-crab.obj");
const ALLOSAURUS: &str = include_str!("../../../assets/models/museum-allosaurus.obj");
const SEADRAGON: &str = include_str!("../../../assets/models/museum-seadragon.obj");

/// The file a specimen is cast from.
pub(super) fn scan(model: Model) -> &'static str {
    match model {
        Model::Mammoth => MAMMOTH,
        Model::GroundSloth => GROUND_SLOTH,
        Model::Triceratops => TRICERATOPS,
        Model::Megalodon => MEGALODON,
        Model::Archaeopteryx => ARCHAEOPTERYX,
        Model::WalrusWhale => WALRUS_WHALE,
        Model::PilotWhale => PILOT_WHALE,
        Model::Tortoise => TORTOISE,
        Model::Porcupinefish => PORCUPINEFISH,
        Model::Murex => MUREX,
        Model::Diplocaulus => DIPLOCAULUS,
        Model::StegosaurusSpike => STEGOSAURUS_SPIKE,
        Model::Dinictis => DINICTIS,
        Model::Trilobite => TRILOBITE,
        Model::DeerSkull => DEER_SKULL,
        Model::KingCrab => KING_CRAB,
        Model::Allosaurus => ALLOSAURUS,
        _ => SEADRAGON,
    }
}

/// Bone, old and cream, a little different bone by bone.
fn bone(k: u32) -> [f32; 3] {
    let v = 0.86 + 0.14 * jitter(k, 301);
    let warm = 0.04 * jitter(k, 302);
    [0.88 * v + warm, 0.82 * v, 0.70 * v - warm]
}

/// A great whale's skeleton, the hall's centrepiece, hung from the vault as
/// the blue whale hangs over the Natural History Museum's hall: a unit long
/// along x, the skull to +x and the flukes' last vertebrae to -x, the spine
/// at y = 0 and bowed as if diving. Each vertebra a body with its spine and
/// side processes, the ribs bowing down and round from the chest, the broad
/// flat skull with its bowed lower jaws, and the flippers' bones.
pub(super) fn whale_skeleton() -> Mesh {
    let mut m = Mesh::default();
    // The spine from the tail (t = 0) to the back of the skull (t = 1).
    let spine = |t: f32| {
        Vec3::new(
            -0.5 + 0.80 * t,
            0.035 * (t * PI).sin() - 0.05 * (1.0 - t).powi(3),
            0.0,
        )
    };
    let along = |t: f32| (spine((t + 0.01).min(1.0)) - spine((t - 0.01).max(0.0))).normalize();
    // How big a vertebra is along the spine: small at the tail, greatest
    // over the chest.
    let size = |t: f32| 0.006 + 0.020 * smooth(0.0, 0.7, t) * (1.0 - 0.35 * smooth(0.8, 1.0, t));
    let vertebrae = fewer(46, 20);
    for i in 0..vertebrae {
        let t = i as f32 / (vertebrae - 1) as f32;
        let (p, d, s) = (spine(t), along(t), size(t));
        let c = bone(i as u32);
        m.append(rod(
            (p - d * s * 0.45).to_array(),
            (p + d * s * 0.45).to_array(),
            s,
            s * 0.95,
            c,
        ));
        let up = Vec3::Y - d * d.y;
        if t > 0.2 {
            // The neural spine, raked back toward the tail.
            let top = p + (up * 2.4 - d * 0.8) * s;
            m.append(rod(p.to_array(), top.to_array(), s * 0.30, s * 0.12, c));
            // The transverse processes, out to either side.
            for side in [-1.0, 1.0] {
                let out = p + Vec3::new(0.0, -0.2 * s, side * 2.0 * s);
                m.append(rod(p.to_array(), out.to_array(), s * 0.28, s * 0.14, c));
            }
        } else if i % 2 == 0 {
            // The chevrons under the tail.
            let under = p - up * 1.6 * s;
            m.append(rod(p.to_array(), under.to_array(), s * 0.25, s * 0.12, c));
        }
    }
    // The ribs, fourteen pairs over the chest, the longest midway.
    let ribs = fewer(14, 8);
    for k in 0..ribs {
        let u = k as f32 / (ribs - 1) as f32;
        let t = 0.60 + 0.30 * u;
        let p = spine(t) + Vec3::new(0.0, -0.004, 0.0);
        let reach = 0.055 + 0.050 * (u * PI).sin().powf(0.7);
        let c = bone(100 + k as u32);
        for side in [-1.0f32, 1.0] {
            let knots: Vec<Knot> = (0..8)
                .map(|j| {
                    let a = j as f32 / 7.0 * 2.3;
                    let at = p + Vec3::new(
                        -0.012 * a,
                        -reach * (1.0 - a.cos()) * 0.95 + 0.012 * a.sin(),
                        side * (0.016 + reach * a.sin()),
                    );
                    Knot {
                        at,
                        radius: 0.0055 * (1.0 - 0.45 * j as f32 / 7.0),
                        colour: c,
                        aux: [0.0; 2],
                    }
                })
                .collect();
            m.append(tube(&knots, 5, 3));
        }
    }
    // The skull: the braincase behind, the broad flat rostrum to the snout.
    let skull = bone(200);
    m.append(
        sphere_with(2, |_| 1.0, |_| skull)
            .scaled([0.055, 0.040, 0.065])
            .map(|p| [p[0] + 0.31, p[1] + 0.01, p[2]], |n| n),
    );
    m.append(
        rod([0.30, 0.0, 0.0], [0.50, -0.012, 0.0], 0.062, 0.010, skull).map(
            |p| [p[0], p[1] * 0.35 + 0.012 * (1.0 - (p[0] - 0.3) / 0.2), p[2]],
            |n| [n[0], n[1] / 0.35, n[2]],
        ),
    );
    // The lower jaws, bowed out round the mouth.
    for side in [-1.0f32, 1.0] {
        let knots: Vec<Knot> = (0..9)
            .map(|j| {
                let s = j as f32 / 8.0;
                Knot {
                    at: Vec3::new(
                        0.30 + 0.205 * s,
                        -0.035 + 0.012 * s,
                        side * (0.052 * (1.0 - s) + 0.006 + 0.03 * (s * PI).sin()),
                    ),
                    radius: 0.008 * (1.0 - 0.4 * s),
                    colour: bone(210),
                    aux: [0.0; 2],
                }
            })
            .collect();
        m.append(tube(&knots, 6, 3));
    }
    // The flippers: the shoulder blade, the arm and four fingers, swept back.
    for side in [-1.0f32, 1.0] {
        let shoulder = spine(0.86) + Vec3::new(0.0, -0.03, side * 0.05);
        let elbow = shoulder + Vec3::new(-0.03, -0.045, side * 0.04);
        let wrist = elbow + Vec3::new(-0.04, -0.03, side * 0.035);
        let c = bone(220);
        m.append(
            sphere_with(1, |_| 1.0, |_| c)
                .scaled([0.03, 0.03, 0.006])
                .map(move |p| (shoulder + Vec3::from_array(p)).to_array(), |n| n),
        );
        m.append(rod(shoulder.to_array(), elbow.to_array(), 0.008, 0.006, c));
        m.append(rod(elbow.to_array(), wrist.to_array(), 0.006, 0.005, c));
        for f in 0..4 {
            let spread = (f as f32 - 1.5) * 0.012;
            let tip = wrist + Vec3::new(-0.05 - 0.01 * (f % 2) as f32, -0.02 + spread, side * 0.02);
            m.append(rod(wrist.to_array(), tip.to_array(), 0.004, 0.002, c));
        }
    }
    m
}

/// The canvas a painting is drawn on: a unit square in xy facing +z, its
/// corners carrying the painting's own coordinates in `aux` (u to the right,
/// v down, as the image runs), which `fs_canvas` reads it by.
pub(super) fn canvas() -> Mesh {
    let mut m = Mesh::default();
    let corner = |m: &mut Mesh, x: f32, y: f32| m.vertex([x, y, 0.0], [1.0; 3], [x + 0.5, 0.5 - y]);
    let a = corner(&mut m, -0.5, -0.5);
    let b = corner(&mut m, 0.5, -0.5);
    let c = corner(&mut m, 0.5, 0.5);
    let d = corner(&mut m, -0.5, 0.5);
    m.quad(a, b, c, d);
    m.smooth()
}

/// A round arch of the arcades, as the Natural History Museum's are built:
/// a band of voussoirs, alternately buff and slate-blue terracotta, springing
/// from x = ±1 at y = 0 over a unit radius, a fifth deep in z, its key a
/// little proud.
pub(super) fn arch() -> Mesh {
    let mut m = Mesh::default();
    let stones = fewer(15, 7);
    let (inner, outer, deep) = (1.0, 1.24, 0.1);
    let buff = [0.86, 0.72, 0.52];
    let blue = [0.52, 0.58, 0.64];
    for k in 0..stones {
        let (a0, a1) = (
            PI * k as f32 / stones as f32,
            PI * (k + 1) as f32 / stones as f32,
        );
        let key = k == stones / 2;
        let colour = if key || k % 2 == 1 { blue } else { buff };
        let r1 = if key { outer + 0.06 } else { outer };
        let at = |a: f32, r: f32, z: f32| [r * a.cos(), r * a.sin(), z];
        let faces = [
            // Soffit, extrados, front and back.
            [
                at(a0, inner, -deep),
                at(a1, inner, -deep),
                at(a1, inner, deep),
                at(a0, inner, deep),
            ],
            [
                at(a0, r1, deep),
                at(a1, r1, deep),
                at(a1, r1, -deep),
                at(a0, r1, -deep),
            ],
            [
                at(a0, inner, deep),
                at(a1, inner, deep),
                at(a1, r1, deep),
                at(a0, r1, deep),
            ],
            [
                at(a0, r1, -deep),
                at(a1, r1, -deep),
                at(a1, inner, -deep),
                at(a0, inner, -deep),
            ],
        ];
        for p in faces {
            m.facet(p, [colour; 4], [0.0; 2]);
        }
        // The joint to the next stone, a dark line on the face.
        let j = at(a1, inner - 0.002, deep + 0.002);
        let jo = at(a1, r1, deep + 0.002);
        let (s, c) = (a1.sin(), a1.cos());
        let w = 0.012;
        m.facet(
            [
                [j[0] - s * w, j[1] + c * w, j[2]],
                [j[0] + s * w, j[1] - c * w, j[2]],
                [jo[0] + s * w, jo[1] - c * w, jo[2]],
                [jo[0] - s * w, jo[1] + c * w, jo[2]],
            ],
            [[0.30, 0.27, 0.24]; 4],
            [0.0; 2],
        );
    }
    m
}

/// Pyrite: brassy cubes grown into one another on a dark matrix, each face
/// striated, the way the Navajún cubes come out of their marl.
pub(super) fn pyrite() -> Mesh {
    let mut m = block([0.5, 0.08, 0.36], [0.0, 0.08, 0.0], [0.22, 0.20, 0.18]);
    let cubes = [
        (0.24, [0.0, 0.30, 0.0], 0.2),
        (0.15, [0.28, 0.24, 0.10], 0.9),
        (0.12, [-0.26, 0.20, -0.06], 0.4),
        (0.09, [0.10, 0.17, 0.24], 1.3),
        (0.10, [-0.12, 0.52, 0.08], 0.7),
        (0.07, [0.22, 0.44, -0.12], 1.8),
    ];
    for (k, (s, c, turn)) in cubes.into_iter().enumerate() {
        let shade = 0.85 + 0.25 * jitter(k as u32, 310);
        let brass = [0.86 * shade, 0.70 * shade, 0.34 * shade];
        let cube = turned(block([s; 3], [0.0; 3], brass), turn);
        m.append(moved(cube, c));
    }
    m
}

/// Bismuth: stepped hopper crystals, square frames each a little smaller and
/// lower than the last, their oxide skin running gold, magenta, blue and
/// green.
pub(super) fn bismuth() -> Mesh {
    let mut m = block([0.42, 0.05, 0.42], [0.0, 0.05, 0.0], [0.62, 0.62, 0.64]);
    let skin = [
        [1.0, 0.78, 0.25],
        [0.95, 0.30, 0.65],
        [0.35, 0.40, 1.0],
        [0.25, 0.90, 0.70],
        [0.85, 0.85, 0.35],
        [0.70, 0.35, 0.95],
    ];
    let hoppers = [
        ([0.0, 0.0], 0.36, 0.0),
        ([0.22, 0.16], 0.22, 0.6),
        ([-0.2, 0.14], 0.18, 1.1),
    ];
    let steps = fewer(6, 3);
    for (h, (at, size, turn)) in hoppers.into_iter().enumerate() {
        let mut hopper = Mesh::default();
        for k in 0..steps {
            let s = size * (1.0 - k as f32 / (steps as f32 + 1.0));
            let y = 0.12 + size * 0.9 - k as f32 * size * 0.14;
            let w = size * 0.07;
            let colour = skin[(k + h * 2) % skin.len()];
            for (cx, cz, hx, hz) in [
                (0.0, s, s, w),
                (0.0, -s, s, w),
                (s, 0.0, w, s),
                (-s, 0.0, w, s),
            ] {
                hopper.append(block([hx, w, hz], [cx, y, cz], colour));
            }
        }
        m.append(moved(turned(hopper, turn), [at[0], 0.0, at[1]]));
    }
    m
}

/// Malachite: a botryoidal crust, grape-like domes on domes, banded in
/// rings of light and dark green.
pub(super) fn malachite() -> Mesh {
    let band = |y: f32| {
        let t = 0.5 + 0.5 * (y * 70.0).sin();
        [0.05 + 0.10 * t, 0.30 + 0.40 * t, 0.16 + 0.20 * t]
    };
    let mut m = sphere_with(
        2,
        |p| 1.0 + 0.05 * (p[0] * 9.0).sin() * (p[2] * 8.0).cos(),
        |p| band(p[1] * 0.3 + p[0] * 0.1),
    )
    .scaled([0.5, 0.35, 0.4]);
    for k in 0..fewer(14, 6) as u32 {
        let a = jitter(k, 320) * TAU;
        let up = 0.15 + 0.75 * jitter(k, 321);
        let r = 0.08 + 0.10 * jitter(k, 322);
        let p = [
            0.45 * up.cos() * a.cos(),
            0.30 * up.sin(),
            0.36 * up.cos() * a.sin(),
        ];
        let bump = sphere_with(1, |_| 1.0, move |q| band(q[1] * 0.4 + k as f32))
            .scaled([r, r, r])
            .map(move |q| [q[0] + p[0], q[1] + p[1], q[2] + p[2]], |n| n);
        m.append(bump);
    }
    // Cut flat underneath, so it sits on its stand.
    for v in &mut m.vertices {
        v.pos[1] = v.pos[1].max(0.0);
    }
    m
}

/// A geode, cracked in half: a rough grey-brown rind, a band of agate, and a
/// hollow lined with amethyst points, its opening turned to +z.
pub(super) fn geode() -> Mesh {
    let rind = lathe(
        &[
            (0.0, -0.5),
            (0.30, -0.46),
            (0.46, -0.28),
            (0.5, -0.04),
            (0.5, 0.0),
        ],
        18,
        |y, a| 1.0 + 0.06 * (a * 5.0 + y * 7.0).sin(),
        |y, a| {
            let v = 0.32 + 0.06 * (a * 11.0 + y * 13.0).sin();
            [v * 1.15, v, v * 0.85]
        },
        |_| [0.0; 2],
    );
    let agate = lathe(
        &[(0.5, 0.0), (0.44, 0.005), (0.40, 0.0)],
        18,
        |_, _| 1.0,
        |_, a| {
            let t = 0.5 + 0.5 * (a * 7.0).sin();
            [0.80 + 0.15 * t, 0.78 + 0.1 * t, 0.86]
        },
        |_| [0.0; 2],
    );
    let lining = lathe(
        &[(0.40, 0.0), (0.38, -0.16), (0.28, -0.32), (0.0, -0.38)],
        18,
        |_, _| 1.0,
        |_, _| [0.26, 0.10, 0.36],
        |_| [0.0; 2],
    );
    let mut m = rind;
    m.append(agate);
    m.append(lining);
    // The amethyst points, standing out of the lining toward the middle.
    for k in 0..fewer(22, 10) as u32 {
        let a = jitter(k, 330) * TAU;
        let down = 0.15 + 0.65 * jitter(k, 331);
        let base = Vec3::new(
            0.36 * (1.0 - down * 0.5) * a.cos(),
            -0.36 * down,
            0.36 * (1.0 - down * 0.5) * a.sin(),
        );
        let tip =
            base + (Vec3::new(0.0, 0.05, 0.0) - base).normalize() * (0.07 + 0.06 * jitter(k, 332));
        crystal_point(
            &mut m,
            base.to_array(),
            tip.to_array(),
            0.028,
            k as f32,
            [0.30, 0.08, 0.42],
            [0.92, 0.80, 1.0],
            k + 400,
        );
    }
    // Turned so the open face looks along +z, its foot on y = 0.
    m.map(|p| [p[0], 0.5 - p[2], p[1]], |n| [n[0], -n[2], n[1]])
}

/// Tourmaline: long striated prisms in a spray, watermelon-coloured, pink at
/// the core of the foot running to green at the terminations.
pub(super) fn tourmaline() -> Mesh {
    let mut m = block([0.3, 0.06, 0.24], [0.0, 0.06, 0.0], [0.78, 0.76, 0.72]);
    let sprays = [
        ([0.0, 0.08, 0.0], [0.04, 1.0, 0.02], 0.09),
        ([0.10, 0.08, 0.04], [0.30, 0.78, 0.10], 0.07),
        ([-0.10, 0.08, -0.02], [-0.28, 0.70, 0.06], 0.065),
        ([0.03, 0.08, 0.10], [0.10, 0.62, 0.30], 0.05),
        ([-0.04, 0.08, -0.10], [-0.10, 0.55, -0.28], 0.045),
    ];
    for (k, (base, tip, r)) in sprays.into_iter().enumerate() {
        crystal_point(
            &mut m,
            base,
            tip,
            r,
            k as f32 * 0.7,
            [0.85, 0.20, 0.42],
            [0.20, 0.62, 0.30],
            k as u32 + 420,
        );
    }
    m
}
