//! The ship's computer: the terminal a long press on the solar tour opens,
//! and the shipboard incidents it reports on its own.
//!
//! The terminal has three pages. NAVIGATION lists where the ship can go: the
//! worlds of the system it is in, still to be visited or already behind it,
//! and the next stars out (relays left out), with how far each is. Choosing a
//! world sets the tour on to the transfer that reaches it; choosing a star
//! takes the ship to its jump point with the drive spooling up for that
//! star, or, already between the stars, turns the jump toward it.
//! ENCOUNTERS lists everything the ship can meet between two legs of a jump,
//! and choosing one drops the ship out of a jump into it a moment later.
//! INCIDENTS lists what can happen aboard; choosing one raises it now.
//!
//! Incidents also happen by themselves, now and then while the ship is in a
//! system: a transmission on the HUD, the alarm raised to its level, and for
//! some a change of course, the next jump's first stopover replaced by what
//! the incident found (a distress beacon leads to a derelict, a tractor lock
//! to a hive cube). That change is made without a cut: the passage after the
//! jump point is all it touches.

use super::system::Rng;
use super::voyage::Event;

/// A row of the terminal's lists: what to call it, a line about it, its state
/// (0 ahead, 1 where the ship is or makes for, 2 behind it), and the code the
/// UI sends back when it is chosen.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Row {
    pub label: String,
    pub detail: String,
    pub state: i32,
    pub code: i32,
}

/// What the terminal shows: where the ship is, and its three lists.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Console {
    /// The ship's position and status, a line each.
    pub status: Vec<String>,
    pub destinations: Vec<Row>,
    pub encounters: Vec<Row>,
    pub incidents: Vec<Row>,
}

/// An order from the terminal.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Order {
    /// To the world at this place in the itinerary.
    Stop(usize),
    /// To the star of this visit.
    Star(u64),
    /// Into this encounter (`Event::ALL`), or one at random.
    Encounter(Option<usize>),
    /// This incident (`INCIDENTS`), or one at random.
    Incident(Option<usize>),
}

/// Codes for the terminal's rows: a star's is its visit past this one plus
/// `STAR_CODE`; a world's its place in the itinerary; `RANDOM` the roll of
/// the dice on the encounters' and incidents' pages.
pub const STAR_CODE: i32 = 1000;
pub const RANDOM: i32 = -1;

impl Order {
    /// The order the UI's page and row code stand for.
    pub fn from_ui(page: i32, code: i32, visit: u64) -> Option<Self> {
        let pick = |count: usize| match code {
            RANDOM => Some(None),
            c if c >= 0 && (c as usize) < count => Some(Some(c as usize)),
            _ => None,
        };
        match page {
            0 if code >= STAR_CODE => Some(Order::Star(visit + (code - STAR_CODE) as u64)),
            0 if code >= 0 => Some(Order::Stop(code as usize)),
            1 => pick(Event::ALL.len()).map(Order::Encounter),
            2 => pick(INCIDENTS.len()).map(Order::Incident),
            _ => None,
        }
    }
}

/// Something that happens aboard: its title and report, how loud the alarm
/// goes (`Hud::alert`, amber from 0.3 and red from 0.7) and what, if
/// anything, the ship turns aside to meet at the next drop-out.
#[derive(Clone, Copy, Debug)]
pub struct Incident {
    pub title: &'static str,
    pub text: &'static str,
    pub alert: f32,
    pub divert: Option<Event>,
}

pub const INCIDENTS: [Incident; 16] = [
    Incident {
        title: "SIGNAL OF UNKNOWN ORIGIN",
        text: "Acoustic beacon, repeating at intervals. Standing orders require investigation. \
               Course altered for the source.",
        alert: 0.45,
        divert: Some(Event::Derelict),
    },
    Incident {
        title: "MOTION TRACKER",
        text: "Intermittent movement in the air ducts, deck C. Sensor fault suspected. \
               Do not go alone.",
        alert: 0.6,
        divert: None,
    },
    Incident {
        title: "HYPERSLEEP CYCLE COMPLETE",
        text: "Crew revival in progress. Pods one to seven opening. Galley heating up.",
        alert: 0.0,
        divert: None,
    },
    Incident {
        title: "COOLANT LEAK",
        text: "Pressure falling in reactor coolant loop B. Venting to space. \
               Maintenance drone dispatched.",
        alert: 0.45,
        divert: None,
    },
    Incident {
        title: "PROXIMITY ALERT",
        text: "Uncharted mass on the jump line. Plotting a way through the field \
               at the next drop-out.",
        alert: 0.6,
        divert: Some(Event::Asteroids),
    },
    Incident {
        title: "HULL BREACH",
        text: "Micrometeoroid strike, cargo bay two. Bulkheads sealed. Atmosphere holding.",
        alert: 0.8,
        divert: None,
    },
    Incident {
        title: "LIFE SIGNS",
        text: "Scanner reads one more lifeform aboard than the crew manifest. Recount ordered.",
        alert: 0.75,
        divert: None,
    },
    Incident {
        title: "PRIORITY DIRECTIVE",
        text: "Sealed order from the company. Science division only. All other priorities \
               rescinded. Access denied.",
        alert: 0.35,
        divert: None,
    },
    Incident {
        title: "STELLAR FLARE",
        text: "Particle front from the star. Shielding at 140 percent. Crew to the storm shelter.",
        alert: 0.5,
        divert: None,
    },
    Incident {
        title: "NAVIGATION DRIFT",
        text: "Inertial platform out of true by 0.03 degrees. Recalibrating on the pulsar grid.",
        alert: 0.2,
        divert: None,
    },
    Incident {
        title: "TRACTOR LOCK",
        text: "Something ahead is pulling on the jump line. Drive will drop out short. \
               Brace for contact.",
        alert: 0.7,
        divert: Some(Event::HiveCube),
    },
    Incident {
        title: "AUTOPILOT FAULT",
        text: "Flight computer rebooting. Manual control for forty seconds. Hands on the yoke.",
        alert: 0.35,
        divert: None,
    },
    Incident {
        title: "SHIP'S CAT LOOSE",
        text: "Cat missing from the mess again. Last seen near the landing struts. \
               Check the lockers.",
        alert: 0.1,
        divert: None,
    },
    Incident {
        title: "CARGO SHIFT",
        text: "Refinery tow shifting on its clamps. Thrust trimmed until it settles.",
        alert: 0.3,
        divert: None,
    },
    Incident {
        title: "UNKNOWN TRANSMISSION",
        text: "Narrow-band burst from the deep. Pattern repeats every twelve seconds. \
               Not natural. Intercept plotted.",
        alert: 0.4,
        divert: Some(Event::Crystal),
    },
    Incident {
        title: "WARSHIPS ON THE LINE",
        text: "Two fleets on the jump line, IFF unknown. Running silent. Drop-out among them likely.",
        alert: 0.65,
        divert: Some(Event::Battle),
    },
];

/// How long an incident's transmission stays on the HUD, its alarm
/// following it, in seconds.
pub const SHOWN: f64 = 11.0;

/// An incident on the HUD: its title and report, how far through its time
/// there, 0..1, and its alarm.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Notice {
    pub title: String,
    pub text: String,
    pub progress: f32,
    pub alert: f32,
}

impl Notice {
    pub fn of(incident: &Incident, progress: f64) -> Self {
        Notice {
            title: incident.title.into(),
            text: incident.text.into(),
            progress: progress.clamp(0.0, 1.0) as f32,
            alert: incident.alert,
        }
    }

    /// Its alarm at this moment: up quickly, held, and down at the end.
    pub fn alarm(&self) -> f32 {
        let p = self.progress;
        self.alert * (p * 12.0).min(1.0) * ((1.0 - p) * 6.0).min(1.0)
    }
}

/// A name for an encounter on the terminal, and a line about it.
pub fn describe(event: Event) -> (&'static str, &'static str) {
    match event {
        Event::Asteroids => ("ASTEROID FIELD", "Thread a field of tumbling rock"),
        Event::HiveCube => ("HIVE CUBE", "Torn from the jump by a tractor beam"),
        Event::BlackHole => ("BLACK HOLE", "Sling past a lensed accretion disc"),
        Event::Nebula => ("STORM NEBULA", "Cross a nebula lit by lightning"),
        Event::RoguePlanet => ("ROGUE PLANET", "A starless world, cracked with lava"),
        Event::Pulsar => ("PULSAR", "Ride out the sweeping beams"),
        Event::Derelict => ("DERELICT STATION", "A broken ring station, adrift"),
        Event::Comet => ("COMET", "Fly through the tails"),
        Event::Leviathans => ("LEVIATHANS", "A pod of glowing giants"),
        Event::Remnant => ("SUPERNOVA REMNANT", "The filaments of a dead star"),
        Event::Gate => ("STAR GATE", "Thrown down a golden conduit"),
        Event::Wormhole => ("WORMHOLE", "Dive into another sky"),
        Event::Battle => ("FLEET BATTLE", "Slip past a fight between fleets"),
        Event::PlasmaStorm => ("PLASMA STORM", "Skirt a lightning-lit eyewall"),
        Event::DysonSwarm => ("DYSON SWARM", "Rings of collectors round a dim star"),
        Event::Crystal => ("CRYSTAL ENTITY", "Something vast that notices the ship"),
    }
}

/// The encounters page: a roll of the dice, then every encounter.
pub fn encounter_rows() -> Vec<Row> {
    std::iter::once(Row {
        label: "RANDOM CONTACT".into(),
        detail: "Let the deep decide".into(),
        state: 0,
        code: RANDOM,
    })
    .chain(Event::ALL.iter().enumerate().map(|(k, &event)| {
        let (label, detail) = describe(event);
        Row {
            label: label.into(),
            detail: detail.into(),
            state: 0,
            code: k as i32,
        }
    }))
    .collect()
}

/// The incidents page: a roll of the dice, then every incident.
pub fn incident_rows() -> Vec<Row> {
    std::iter::once(Row {
        label: "RANDOM INCIDENT".into(),
        detail: "Whatever happens next".into(),
        state: 0,
        code: RANDOM,
    })
    .chain(INCIDENTS.iter().enumerate().map(|(k, incident)| Row {
        label: incident.title.into(),
        detail: match incident.divert {
            Some(event) => format!("Diverts: {}", describe(event).0.to_lowercase()),
            None => format!(
                "Alarm {}",
                match incident.alert {
                    a if a >= 0.7 => "red",
                    a if a >= 0.3 => "amber",
                    _ => "none",
                }
            ),
        },
        state: 0,
        code: k as i32,
    }))
    .collect()
}

/// The incidents of a visit by themselves: when each begins, seconds into the
/// visit, and which. One or two, sometimes none, spread through the time in
/// the system (`inside`, the start of the flight out); never in its first
/// half minute, and never two at once.
pub fn schedule(seed: u64, inside: f64) -> Vec<(f64, usize)> {
    let mut rng = Rng::new(seed ^ 0x1c1d_e7a5);
    let count = match rng.unit() {
        u if u < 0.25 => 0,
        u if u < 0.75 => 1,
        _ => 2,
    };
    let room = inside - 30.0 - SHOWN;
    if room <= SHOWN * 2.0 {
        return Vec::new();
    }
    let mut out: Vec<(f64, usize)> = Vec::new();
    for k in 0..count {
        // Each in its own share of the time, so two never overlap.
        let share = room / count as f64;
        let at = 30.0 + share * k as f64 + rng.range(0.0, share - SHOWN);
        let pick = rng.int(0, INCIDENTS.len() as u64 - 1) as usize;
        if out.iter().all(|&(_, p)| p != pick) {
            out.push((at, pick));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn orders_come_back_from_the_ui() {
        assert_eq!(Order::from_ui(0, 2, 7), Some(Order::Stop(2)));
        assert_eq!(Order::from_ui(0, STAR_CODE + 3, 7), Some(Order::Star(10)));
        assert_eq!(Order::from_ui(1, RANDOM, 7), Some(Order::Encounter(None)));
        assert_eq!(Order::from_ui(1, 4, 7), Some(Order::Encounter(Some(4))));
        assert_eq!(Order::from_ui(1, 99, 7), None);
        assert_eq!(Order::from_ui(2, 0, 7), Some(Order::Incident(Some(0))));
        assert_eq!(Order::from_ui(5, 0, 7), None);
    }

    #[test]
    fn scheduled_incidents_never_overlap() {
        for seed in 0..200 {
            let incidents = schedule(seed, 500.0);
            assert!(incidents.len() <= 2);
            for &(at, pick) in &incidents {
                assert!(at >= 30.0 && at + SHOWN <= 500.0, "{seed}: {at}");
                assert!(pick < INCIDENTS.len());
            }
            if let [a, b] = incidents[..] {
                assert!((a.0 - b.0).abs() >= SHOWN, "{seed}: {a:?} {b:?}");
            }
        }
    }

    #[test]
    fn every_encounter_and_incident_is_listed() {
        assert_eq!(encounter_rows().len(), Event::ALL.len() + 1);
        assert_eq!(incident_rows().len(), INCIDENTS.len() + 1);
    }
}
