//! What the ship's panels say through a starbase visit: the docking panel
//! on the way in, the service panel in the dock, the HUD's lines at every
//! stage and what approach control says.

use super::super::super::system::{KM, Rng, Stop};
use super::super::{Manoeuvre, Phase, Tour, add, clock, length, scale, sub, thousands};
use super::{
    BAY_SPEED, BERTH, Base, BaseStage, CONTACT, FLIP, HOLD, LOCK, METRE, SERVICE, Step, TUNNEL,
    WORLD_LENGTH, doors, ease, far_wall, lock_depth, tunnel_end,
};

impl Tour {
    /// Metres to the mouth from where the camera is at `t`, outside.
    fn range_to_mouth(&self, base: &Base, t: f64) -> f64 {
        let frame = self.base_frame(base);
        let mouth = add(
            self.system.centre(base.stop, t),
            scale(frame.out, frame.radius),
        );
        length(sub(self.view(t).position, mouth)) / METRE
    }

    /// How fast the camera moves at `t`, metres a second.
    fn speed_at(&self, t: f64) -> f64 {
        let h = 0.05;
        length(sub(self.view(t + h).position, self.view(t - h).position)) / (2.0 * h) / METRE
    }

    /// What approach control says during step `step`, `into` seconds into
    /// it.
    fn atc_line(&self, base: &Base, step: Step, into: f64) -> String {
        let name = &base.name;
        match (step, base.traffic) {
            (Step::Contact, _) if into < CONTACT * 0.5 => {
                format!("“{name} approach, inbound to dock, requesting a bay.”")
            }
            (Step::Contact, _) => "“Read you. Stand by for a bay.”".into(),
            (Step::Clearance, None) => format!(
                "“Cleared to dock, bay {}. Corridor {}.”",
                base.bay, base.corridor
            ),
            (Step::Clearance, Some(ship)) => format!(
                "“Bay {} is yours, but hold your orbit: {} has priority.”",
                base.bay, ship.name
            ),
            (Step::Hold, Some(ship)) if into < HOLD * 0.6 => {
                format!("“{} going in ahead of you: {}.”", ship.name, ship.reason)
            }
            (Step::Hold, _) => "“Traffic is in. You are cleared, corridor's yours.”".into(),
            (Step::Align, _) => format!("“Line up on the corridor {} beacon.”", base.corridor),
            (Step::Final, _) => format!("“Guidance lights on. Welcome to {name}.”"),
        }
    }

    /// The docking panel `local` seconds into the way in.
    fn approach_panel(&self, base: &Base, local: f64, t: f64) -> Manoeuvre {
        let total = base.approach();
        let steps = base.steps();
        let step = base.step(local);
        let bay = base.bay;
        let stages = steps
            .iter()
            .map(|&(s, _)| {
                let (label, value) = match s {
                    Step::Contact => ("CONTACT", format!("hailing {} approach control", base.name)),
                    Step::Clearance => match base.traffic {
                        None => ("CLEARANCE", format!("cleared to dock, bay {bay}")),
                        Some(_) => ("CLEARANCE", format!("bay {bay}, after priority traffic")),
                    },
                    Step::Hold => {
                        let ship = base.traffic.expect("a hold for traffic");
                        ("HOLD", format!("{} in first, {}", ship.name, ship.class))
                    }
                    Step::Align => (
                        "ALIGNMENT",
                        format!("bow on the corridor {} beacon", base.corridor),
                    ),
                    Step::Final => (
                        "FINAL",
                        format!(
                            "{} km to the mouth",
                            thousands(self.range_to_mouth(base, t) / 1000.0)
                        ),
                    ),
                };
                (label.to_string(), value)
            })
            .collect();
        let tag = match step {
            Step::Contact => "HAIL",
            Step::Clearance => "CLEARED",
            Step::Hold => "HOLD",
            Step::Align => "ALIGN",
            Step::Final => "DOCK",
        };
        // The range closed, from the orbit's height to the mouth.
        let start = (Tour::height(self.world(base.stop)) - self.world(base.stop).radius) / METRE;
        let range = self.range_to_mouth(base, t);
        Manoeuvre {
            name: "DOCKING APPROACH".into(),
            text: format!("{} · bay {bay} · corridor {}", base.name, base.corridor),
            progress: (local / total) as f32,
            stages,
            starts: steps.iter().map(|&(_, s)| s / total).collect(),
            charge: (1.0 - range / start).clamp(0.0, 1.0) as f32,
            dock: true,
            tag: tag.into(),
            gauge: "RNG".into(),
            label: "BERTH".into(),
            hot: (steps.len() - 1) as u32,
            ..Default::default()
        }
    }

    /// The service panel `local` seconds into the service.
    fn service_panel(&self, base: &Base, local: f64) -> Manoeuvre {
        let u = local / SERVICE;
        let mut rng = Rng::new(self.system.seed ^ 0x5e41_11ce);
        let hull = rng.int(58, 91);
        let mass = rng.int(140, 620);
        let (_, drive) = self.drive();
        let stages = vec![
            (
                "HULL PLATING".to_string(),
                format!("welding the seams, {hull}% to 100%"),
            ),
            (
                "COOLANT".to_string(),
                "loops flushed and refilled".to_string(),
            ),
            (
                "REACTION MASS".to_string(),
                format!("{mass} t pumped aboard"),
            ),
            (
                "DRIVE COIL".to_string(),
                format!("the {drive} recalibrated"),
            ),
            (
                "NAV SYNC".to_string(),
                "charts and beacons brought up to date".to_string(),
            ),
        ];
        let tag = if u >= 0.8 { "READY" } else { "SERVICE" };
        Manoeuvre {
            name: "SERVICE".into(),
            text: format!("{} · bay {}", base.name, base.bay),
            progress: u as f32,
            stages,
            starts: vec![0.0, 0.2, 0.4, 0.6, 0.8],
            charge: u as f32,
            dock: true,
            tag: tag.into(),
            gauge: "SVC".into(),
            label: "BAY".into(),
            hot: 4,
            ..Default::default()
        }
    }

    /// The panel `local` seconds into stage `stage` at the base at `stop`.
    pub(in crate::solar::tour) fn base_manoeuvre(
        &self,
        stop: Stop,
        stage: BaseStage,
        local: f64,
        t: f64,
    ) -> Manoeuvre {
        let base = self.base_at(stop);
        let (start, end) = self.base_span(stop, stage);
        let u = (local / (end - start).max(1e-6)) as f32;
        let coast = |name: &str, text: String| Manoeuvre {
            name: name.into(),
            text,
            progress: u,
            ..Default::default()
        };
        let (name, bay, corridor) = (&base.name, base.bay, base.corridor);
        match stage {
            BaseStage::Approach => self.approach_panel(base, local, t),
            BaseStage::Airlock => coast(
                "AIRLOCK",
                format!(
                    "Down the entrance into {name}'s lock: the outer door shut behind, the inner opened ahead."
                ),
            ),
            BaseStage::Tunnel => coast(
                "TRANSIT TUNNEL",
                format!(
                    "Down corridor {corridor} into {name}, the city's lights through the windows either side."
                ),
            ),
            BaseStage::Berth => coast(
                "BERTHING",
                format!("Guided into the cradle of bay {bay}, the clamps reaching for the hull."),
            ),
            BaseStage::Service => self.service_panel(base, local),
            BaseStage::Undock => coast(
                "UNDOCKING",
                format!(
                    "Clamps released: lifting out of bay {bay} on thrusters, cleared on through."
                ),
            ),
            BaseStage::Crossing => coast(
                "CROSSING",
                format!(
                    "On along the axis of {name}'s world, all {} km of it, to the lock in its far wall.",
                    thousands(WORLD_LENGTH / 1000.0)
                ),
            ),
            BaseStage::Egress => coast(
                "FAR AIRLOCK",
                format!(
                    "Through the far wall's lock and up its entrance, out of the far side of {name}."
                ),
            ),
            BaseStage::Climb => coast(
                "DEPARTURE",
                format!(
                    "Out of {name}'s far side, climbing round it into orbit for {}.",
                    self.after_base(stop)
                ),
            ),
        }
    }

    /// Where the tour goes after the base.
    fn after_base(&self, stop: Stop) -> String {
        let (_, end) = self.base_span(stop, BaseStage::Climb);
        self.segments
            .iter()
            .find(|s| s.start >= end - 1e-6)
            .and_then(|s| match s.phase {
                Phase::Transfer { to, .. } => Some(self.world(to).name.clone()),
                _ => None,
            })
            .unwrap_or_else(|| format!("the jump to {}", self.next.star.name))
    }

    /// The HUD `local` seconds into stage `stage` at the base at `stop`.
    pub(in crate::solar::tour) fn base_hud(
        &self,
        stop: Stop,
        stage: BaseStage,
        local: f64,
        t: f64,
    ) -> (String, Vec<(String, String)>) {
        let base = self.base_at(stop);
        let (_, end) = self.base_span(stop, stage);
        let left = end - t;
        let bay = format!("bay {} · corridor {}", base.bay, base.corridor);
        let speed = || format!("{} m/s", thousands(self.speed_at(t)));
        let name = ("Starbase".to_string(), base.name.clone());
        match stage {
            BaseStage::Approach => {
                let step = base.step(local);
                let into = local
                    - base
                        .steps()
                        .into_iter()
                        .find(|&(s, _)| s == step)
                        .map_or(0.0, |(_, at)| at);
                let giant = &self.system.planets[stop.planet].name;
                let across = self.world(stop).radius * 2.0 * KM;
                let traffic = match base.traffic {
                    None => "clear".to_string(),
                    Some(ship) => match step {
                        Step::Contact | Step::Clearance => {
                            format!("{} · {} inbound", ship.name, ship.class)
                        }
                        Step::Hold => format!("{} entering ahead · {}", ship.name, ship.reason),
                        _ => format!("{} docked", ship.name),
                    },
                };
                (
                    "APPROACH CONTROL".into(),
                    vec![
                        name,
                        (
                            "".into(),
                            format!(
                                "Starbase · {} km across · orbits {giant}",
                                thousands(across)
                            ),
                        ),
                        (
                            "Range".into(),
                            format!(
                                "{} km to the mouth",
                                thousands(self.range_to_mouth(base, t) / 1000.0)
                            ),
                        ),
                        ("Berth".into(), bay),
                        ("Traffic".into(), traffic),
                        ("ATC".into(), self.atc_line(base, step, into)),
                    ],
                )
            }
            BaseStage::Airlock => {
                let [outer, inner] = doors(stage, local);
                let door = |open: f64| {
                    if open >= 1.0 {
                        "open"
                    } else if open <= 0.0 {
                        "sealed"
                    } else {
                        "cycling"
                    }
                };
                (
                    "AIRLOCK".into(),
                    vec![
                        name,
                        (
                            "Depth".into(),
                            format!("{} m", thousands(lock_depth(local))),
                        ),
                        ("Outer door".into(), door(outer).into()),
                        ("Inner door".into(), door(inner).into()),
                        ("Berth".into(), bay),
                    ],
                )
            }
            BaseStage::Tunnel => {
                let z = ease(LOCK, 0.0, tunnel_end(), BAY_SPEED, TUNNEL, local);
                (
                    "TRANSIT TUNNEL".into(),
                    vec![
                        name,
                        ("Depth".into(), format!("{} m", thousands(z))),
                        ("Speed".into(), speed()),
                        ("Berth".into(), bay),
                        ("Docking in".into(), clock(left + BERTH)),
                    ],
                )
            }
            BaseStage::Berth => (
                "BERTHING".into(),
                vec![
                    name,
                    ("Berth".into(), bay),
                    ("Speed".into(), speed()),
                    (
                        "Clamps".into(),
                        if left < 1.5 {
                            "locked".into()
                        } else {
                            "open".into()
                        },
                    ),
                ],
            ),
            BaseStage::Service => {
                let done = ((local / SERVICE) * 5.0).floor().min(5.0) as u32;
                (
                    "DOCKED".into(),
                    vec![
                        name,
                        ("Berth".into(), bay),
                        ("Service".into(), format!("{done} of 5 done")),
                        ("Undock in".into(), clock(left)),
                    ],
                )
            }
            BaseStage::Undock => (
                "UNDOCKING".into(),
                vec![
                    name,
                    ("Berth".into(), bay),
                    ("Cleared by".into(), format!("{} control", base.name)),
                ],
            ),
            BaseStage::Crossing => {
                let scene = self.base_view(stop, stage, local, t).scene;
                let (ship, fold) = (scene.ship, scene.fold);
                (
                    "CROSSING".into(),
                    vec![
                        name,
                        (
                            "Far wall in".into(),
                            format!(
                                "{} km",
                                thousands((far_wall() - ship[2] - fold).max(0.0) / 1000.0)
                            ),
                        ),
                        ("Speed".into(), speed()),
                        (
                            "Corridor".into(),
                            format!("axis · {corridor} cleared", corridor = base.corridor),
                        ),
                    ],
                )
            }
            BaseStage::Egress => {
                let [outer, inner] = doors(stage, local);
                let door = |open: f64| {
                    if open >= 1.0 {
                        "open"
                    } else if open <= 0.0 {
                        "sealed"
                    } else {
                        "cycling"
                    }
                };
                // Still in the lock as the scene turns: no speed to show.
                let speed = if (local - FLIP).abs() < 0.3 {
                    "0 m/s".into()
                } else {
                    speed()
                };
                (
                    "FAR AIRLOCK".into(),
                    vec![
                        name,
                        (
                            "Door behind".into(),
                            door(if local < FLIP { inner } else { 0.0 }).into(),
                        ),
                        ("Door ahead".into(), door(outer).into()),
                        ("Speed".into(), speed),
                    ],
                )
            }
            BaseStage::Climb => (
                "DEPARTURE".into(),
                vec![
                    ("Leaving".into(), base.name.clone()),
                    ("Next".into(), self.after_base(stop)),
                    ("Speed".into(), speed()),
                ],
            ),
        }
    }
}
