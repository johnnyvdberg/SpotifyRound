//! The ship's terminal and its incidents: orders carried out, courses
//! changed, and the console's pages kept up to date.

use super::*;

/// The ship's terminal and its incidents (`console`).
impl Solar {
    /// A jump's passage for trip `trip` toward a star its drive is `drive`
    /// for, with nothing that needs the model meshes until they are built.
    pub(super) fn ready_passage(&self, passage: voyage::Passage, drive: u32) -> voyage::Passage {
        if self.models.is_some() {
            passage
        } else {
            passage.without_models(drive)
        }
    }

    /// This visit's tour again, its jump through `passage` instead: the same
    /// up to the first leg, so swapped in without a cut until then.
    pub(super) fn with_passage(&self, passage: voyage::Passage) -> Tour {
        Tour::plan(
            self.tour.system.clone(),
            self.tour.route.clone(),
            self.tour.transit,
            self.tour.leg,
            self.tour.legs,
            passage,
            self.tour.arrival_light(),
        )
    }

    /// Carry out an order from the terminal, `seconds` into the scene. The
    /// terminal covers the frame while it does, so a cut does not show.
    pub(super) fn obey(&mut self, order: console::Order, seconds: f64) {
        let t = seconds - self.visit_start;
        let phase = self.tour.segment(t).0.phase;
        match order {
            console::Order::Stop(k) => {
                let Some(&stop) = self.tour.itinerary.get(k) else {
                    return;
                };
                if phase.deep() || self.tour.current(t) == Some(stop) {
                    return;
                }
                if let Some(start) = self
                    .tour
                    .start_of(|p| matches!(p, Phase::Transfer { to, .. } if to == stop))
                {
                    self.visit_start = seconds - start;
                }
            }
            console::Order::Star(target) => {
                if target <= self.visit || relay(self.base_seed, target) {
                    return;
                }
                let bound = self.visit + self.tour.route.len() as u64;
                if phase.deep() && target == bound {
                    return;
                }
                let destination = system_for(self.base_seed, target);
                let drive = tour::drive_to(&destination).0;
                let passage = self.ready_passage(
                    voyage::Passage::plan(self.base_seed, target - 1, drive),
                    drive,
                );
                let tour = Tour::plan(
                    self.tour.system.clone(),
                    vec![destination],
                    self.tour.transit,
                    1,
                    1,
                    passage,
                    self.tour.arrival_light(),
                );
                // Between the stars the jump turns for it at once; in a
                // system the ship goes to its jump point and spools up.
                let at = if phase.deep() {
                    tour.start_of(|p| p == Phase::Jump(0))
                } else {
                    tour.start_of(|p| p == Phase::Spool)
                };
                let Some(at) = at else {
                    return;
                };
                self.tour = tour;
                self.visit = target - 1;
                self.visit_start = seconds - at;
            }
            console::Order::Encounter(pick) => {
                let mut rng = system::Rng::new(rand::random::<u64>());
                let event = match pick {
                    Some(k) => voyage::Event::ALL[k],
                    None => *rng.pick(&voyage::Event::ALL),
                };
                let drive = tour::drive_to(&self.tour.next).0;
                let stops = vec![voyage::Stopover {
                    event,
                    variant: rng.next(),
                }];
                let passage = voyage::Passage::with_stops(stops, drive, &mut rng);
                let tour = self.with_passage(self.ready_passage(passage, drive));
                // A moment before the drop-out into it.
                let Some(at) = tour.start_of(|p| p == Phase::Stopover(0)) else {
                    return;
                };
                self.tour = tour;
                self.visit_start = seconds - (at - 2.0);
            }
            console::Order::Incident(pick) => {
                let k = pick.unwrap_or_else(|| {
                    (rand::random::<u64>() % console::INCIDENTS.len() as u64) as usize
                });
                self.raised = Some((seconds, k));
                if let Some(event) = console::INCIDENTS[k].divert {
                    self.divert(event, t);
                }
            }
        }
    }

    /// Meet `event` at the next jump's first drop-out, if the ship is not in
    /// the jump yet: the visit up to then stays as it is.
    pub(super) fn divert(&mut self, event: voyage::Event, t: f64) {
        let Some(jump) = self.tour.start_of(|p| p == Phase::Jump(0)) else {
            return;
        };
        if t >= jump || self.tour.passage.stops.is_empty() {
            return;
        }
        let mut rng = system::Rng::new(self.tour.system.seed ^ (event.id() as u64) << 32 ^ 0xd17e);
        let drive = tour::drive_to(&self.tour.next).0;
        let mut stops = self.tour.passage.stops.clone();
        stops[0] = voyage::Stopover {
            event,
            variant: rng.next(),
        };
        let passage = voyage::Passage::with_stops(stops, drive, &mut rng);
        self.tour = self.with_passage(self.ready_passage(passage, drive));
        tracing::info!(?event, "course altered by an incident");
    }

    /// The incident showing at `t` into the visit, if any, on the HUD with
    /// its alarm; a scheduled one that changes course does so as it begins.
    pub(super) fn update_incidents(&mut self, t: f64) {
        let seconds = t + self.visit_start;
        if self.incidents.0 != self.visit || self.incidents.1 != self.tour.system.seed {
            let inside = self
                .tour
                .start_of(|p| p == Phase::Spool)
                .unwrap_or(self.tour.total);
            let seed = self.tour.system.seed ^ self.visit.wrapping_mul(0x9e37_79b9);
            self.incidents = (
                self.visit,
                self.tour.system.seed,
                console::schedule(seed, inside),
            );
        }
        let raised = self
            .raised
            .filter(|&(at, _)| (0.0..console::SHOWN).contains(&(seconds - at)))
            .map(|(at, k)| (seconds - at, k));
        if raised.is_none() {
            self.raised = None;
        }
        let scheduled = self
            .incidents
            .2
            .iter()
            .find(|&&(at, _)| (0.0..console::SHOWN).contains(&(t - at)))
            .map(|&(at, k)| (t - at, k));
        let Some((into, k)) = raised.or(scheduled) else {
            return;
        };
        if raised.is_none()
            && self.diverted != Some((self.visit, k))
            && let Some(event) = console::INCIDENTS[k].divert
        {
            self.diverted = Some((self.visit, k));
            self.divert(event, t);
        }
        let notice = console::Notice::of(&console::INCIDENTS[k], into / console::SHOWN);
        self.hud.alert = self.hud.alert.max(notice.alarm());
        self.hud.notice = Some(notice);
    }

    /// What the terminal shows at `t` into the visit.
    pub(super) fn update_console(&mut self, t: f64) {
        let phase = self.tour.segment(t).0.phase;
        let current = self.tour.current(t);
        let here = current.and_then(|c| self.tour.itinerary.iter().position(|&s| s == c));
        let mut status = vec![format!(
            "LOCATION   {} SYSTEM",
            self.tour.system.star.name.to_uppercase()
        )];
        status.push(match phase {
            Phase::Transfer { to, .. } => {
                format!(
                    "STATUS     IN TRANSFER TO {}",
                    self.tour.world(to).name.to_uppercase()
                )
            }
            Phase::High(stop) => format!(
                "STATUS     IN ORBIT, {}",
                self.tour.world(stop).name.to_uppercase()
            ),
            Phase::Base(stop, stage) => format!(
                "STATUS     {}, {}",
                if stage == tour::BaseStage::Approach {
                    "ON APPROACH"
                } else {
                    "AT THE STARBASE"
                },
                self.tour.world(stop).name.to_uppercase()
            ),
            Phase::Depart(_) | Phase::Cruise { .. } => {
                "STATUS     OUTBOUND FOR THE JUMP POINT".into()
            }
            Phase::Spool => "STATUS     DRIVE SPOOLING".into(),
            Phase::Jump(_) | Phase::Rejump(_) => "STATUS     IN HYPERJUMP".into(),
            Phase::Stopover(_) => "STATUS     DROPPED OUT. CONTACT".into(),
        });
        status.push(format!(
            "COURSE     {} SYSTEM",
            self.tour.destination().star.name.to_uppercase()
        ));
        let mut destinations = Vec::new();
        if !phase.deep() {
            // Past the last stop every world is behind the ship.
            let passed = here.unwrap_or(match phase {
                Phase::Transfer { .. } | Phase::High(_) | Phase::Base(..) | Phase::Depart(_) => 0,
                _ => self.tour.itinerary.len(),
            });
            for (k, &stop) in self.tour.itinerary.iter().enumerate() {
                let world = self.tour.world(stop);
                let detail = match stop.moon {
                    Some(_) => format!(
                        "{}, moon of {}",
                        world.kind.name(),
                        self.tour.system.planets[stop.planet].name
                    ),
                    None => world.kind.name().to_string(),
                };
                destinations.push(console::Row {
                    label: world.name.to_uppercase(),
                    detail,
                    state: if Some(k) == here {
                        1
                    } else if k < passed {
                        2
                    } else {
                        0
                    },
                    code: k as i32,
                });
            }
        }
        if self.stars.0 != self.visit {
            self.stars = (self.visit, self.stars_ahead());
        }
        let bound = self.tour.destination().seed;
        destinations.extend(self.stars.1.iter().map(|(seed, row)| {
            let mut row = row.clone();
            if *seed == bound {
                row.state = 1;
            }
            row
        }));
        let console = console::Console {
            status,
            destinations,
            encounters: if self.console.encounters.is_empty() {
                console::encounter_rows()
            } else {
                std::mem::take(&mut self.console.encounters)
            },
            incidents: if self.console.incidents.is_empty() {
                console::incident_rows()
            } else {
                std::mem::take(&mut self.console.incidents)
            },
        };
        self.console = console;
    }

    /// The next stars out from this visit's, relays left out: a row each,
    /// with how far the ship has to go to reach it.
    pub(super) fn stars_ahead(&self) -> Vec<(u64, console::Row)> {
        let mut rows = Vec::new();
        let mut ly = 0.0;
        for n in 1..=12u64 {
            let system = system_for(self.base_seed, self.visit + n);
            ly += system.distance;
            if relay(self.base_seed, self.visit + n) {
                continue;
            }
            rows.push((
                system.seed,
                console::Row {
                    label: format!("{} SYSTEM", system.star.name.to_uppercase()),
                    detail: format!(
                        "{}, {} worlds, {:.1} ly",
                        system.star.title(),
                        system.planets.len(),
                        ly
                    ),
                    state: 0,
                    code: console::STAR_CODE + n as i32,
                },
            ));
            if rows.len() == 5 {
                break;
            }
        }
        rows
    }
}
