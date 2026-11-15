//! The frame's pipelines, by what they draw.
//!
//! Every pipeline the frame uses is a [`Pipe`]: the frame looks them up by
//! value, so a misspelt name is a compile error and no lookup allocates. Each
//! one prints as the name it had when pipelines were keyed by strings, which
//! the benchmarks and the screenshot notes still show.

use std::fmt;

use super::{FlatLook, SurfaceLook};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum Pipe {
    /// The depth mask: each body's solid disc (traced, or a polygon wholly
    /// inscribed in it).
    Mask,
    MaskInscribed,
    /// The frame's corners off the round display.
    Corners,
    /// For the Pi estimate only: what survives the mask, shaded by nothing.
    Probe,
    ProbeStrict,
    ProbeFull,
    ProbeGlare,
    ProbeGlobe,
    ProbeLimbRing,
    ProbeLeviathan(u32),
    /// The starfield and the galaxy alone, baked once a system.
    Bake,
    BakeSky,
    /// The background: procedural, baked, or baked with the next jump's star
    /// marked.
    Space,
    SpaceBaked,
    Target,
    /// The star's glare round it, or over the whole frame.
    Glare,
    GlareFull,
    /// An engine burn's dust and the glow round the rim.
    Dust,
    Rim,
    /// A drive starting up, a pipeline a stage, and its light rising.
    Spool {
        drive: u32,
        stage: usize,
    },
    SpoolLight(u32),
    /// A drive's light (look `look`), with the flare at the heading or not.
    Warp {
        look: u32,
        flare: bool,
    },
    /// The drop-out of look `look`: ahead of its front, and behind it.
    DropAhead(u32),
    DropOut(u32),
    /// A stopover's own draw (by event id), one of its parts, its rocks, and
    /// its gas drawn coarse.
    Event(u32),
    EventPart(u32, &'static str),
    Rocks(u32),
    Coarse(u32),
    /// What is seen inside a leg (by flux id).
    Flux(u32),
    /// The leviathans' parts (`PART`, 4 the halos) and their glow.
    Leviathan(u32),
    LeviathanGlow,
    /// The battle's ships, and every other model family (from 1).
    Ships,
    Model(u32),
    /// A battle's effects of kind `kind`, near or dim.
    BattleFx {
        kind: usize,
        far: bool,
    },
    OverlayFlash,
    OverlayHit,
    OverlayRim,
    OverlayBeam,
    /// Over a body: its eclipse, its rings' shadow, its rings and its limb.
    Eclipse,
    RingShadow,
    Ring,
    Limb,
    LimbRing,
    /// The bodies as meshes: the star, worlds whose maps are not in, moons,
    /// and mapped worlds from afar with their seas' light.
    GlobeStar,
    GlobeFlat(FlatLook),
    GlobeMoon {
        slot: usize,
        air: bool,
    },
    Globe(SurfaceLook, usize),
    GlobeGlow(SurfaceLook, usize),
    /// A mapped world close by, per map slot.
    Ground(usize),
    GroundDetail(usize),
    Detail(usize),
    Clouds(usize),
    CloudLayer(usize, u32),
    Glow(usize),
    GlowSea(usize),
}

impl Pipe {
    /// Drawn as a mesh: its grid's triangles (`vs_globe`).
    pub(crate) fn is_globe(self) -> bool {
        matches!(
            self,
            Pipe::GlobeStar
                | Pipe::GlobeFlat(_)
                | Pipe::GlobeMoon { .. }
                | Pipe::Globe(..)
                | Pipe::GlobeGlow(..)
        )
    }
}

impl fmt::Display for Pipe {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let flag = |on: bool, c: char| if on { c } else { '-' };
        match *self {
            Pipe::Mask => f.write_str("mask"),
            Pipe::MaskInscribed => f.write_str("maskinscribed"),
            Pipe::Corners => f.write_str("corners"),
            Pipe::Probe => f.write_str("probe"),
            Pipe::ProbeStrict => f.write_str("probestrict"),
            Pipe::ProbeFull => f.write_str("probefull"),
            Pipe::ProbeGlare => f.write_str("probeglare"),
            Pipe::ProbeGlobe => f.write_str("probeglobe"),
            Pipe::ProbeLimbRing => f.write_str("probelimbring"),
            Pipe::ProbeLeviathan(part) => write!(f, "probeleviathan{part}"),
            Pipe::Bake => f.write_str("bake"),
            Pipe::BakeSky => f.write_str("bakesky"),
            Pipe::Space => f.write_str("space"),
            Pipe::SpaceBaked => f.write_str("spacebaked"),
            Pipe::Target => f.write_str("target"),
            Pipe::Glare => f.write_str("glare"),
            Pipe::GlareFull => f.write_str("glarefull"),
            Pipe::Dust => f.write_str("dust"),
            Pipe::Rim => f.write_str("rim"),
            Pipe::Spool { drive, stage } => write!(f, "spool{drive}-{stage}"),
            Pipe::SpoolLight(drive) => write!(f, "spoollight{drive}"),
            Pipe::Warp { look, flare } => {
                write!(f, "warp{}{look}", if flare { "flare" } else { "" })
            }
            Pipe::DropAhead(look) => write!(f, "dropahead{look}"),
            Pipe::DropOut(look) => write!(f, "dropout{look}"),
            Pipe::Event(id) => write!(f, "event{id}"),
            Pipe::EventPart(id, part) => write!(f, "event{id} {part}"),
            Pipe::Rocks(id) => write!(f, "rocks{id}"),
            Pipe::Coarse(id) => write!(f, "coarse{id}"),
            Pipe::Flux(id) => write!(f, "flux{id}"),
            Pipe::Leviathan(part) => write!(f, "leviathan{part}"),
            Pipe::LeviathanGlow => f.write_str("leviathanglow"),
            Pipe::Ships => f.write_str("ships"),
            Pipe::Model(family) => write!(f, "model{family}"),
            Pipe::BattleFx { kind, far } => {
                write!(f, "battlefx{kind}{}", if far { "far" } else { "" })
            }
            Pipe::OverlayFlash => f.write_str("overlayflash"),
            Pipe::OverlayHit => f.write_str("overlayhit"),
            Pipe::OverlayRim => f.write_str("overlayrim"),
            Pipe::OverlayBeam => f.write_str("overlaybeam"),
            Pipe::Eclipse => f.write_str("eclipse"),
            Pipe::RingShadow => f.write_str("ringshadow"),
            Pipe::Ring => f.write_str("ring"),
            Pipe::Limb => f.write_str("limb"),
            Pipe::LimbRing => f.write_str("limbring"),
            Pipe::GlobeStar => f.write_str("globestar"),
            Pipe::GlobeFlat(look) => write!(
                f,
                "globeflat{}{}{}",
                flag(look.air, 'a'),
                flag(look.giant, 'g'),
                flag(look.rings, 'r'),
            ),
            Pipe::GlobeMoon { slot, air } => write!(f, "globemoon{slot}{}", flag(air, 'a')),
            Pipe::Globe(look, slot) => write!(f, "globe{}", look.flags(slot)),
            Pipe::GlobeGlow(look, slot) => write!(f, "globeglow{}", look.flags(slot)),
            Pipe::Ground(slot) => write!(f, "ground{slot}"),
            Pipe::GroundDetail(slot) => write!(f, "grounddetail{slot}"),
            Pipe::Detail(slot) => write!(f, "detail{slot}"),
            Pipe::Clouds(slot) => write!(f, "clouds{slot}"),
            Pipe::CloudLayer(slot, layer) => write!(f, "cloudlayer{slot}x{layer}"),
            Pipe::Glow(slot) => write!(f, "glow{slot}"),
            Pipe::GlowSea(slot) => write!(f, "glowsea{slot}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Pipe;

    /// The names the benchmarks and screenshot notes have always shown.
    #[test]
    fn pipelines_print_their_old_names() {
        for (pipe, name) in [
            (
                Pipe::Warp {
                    look: 3,
                    flare: true,
                },
                "warpflare3",
            ),
            (
                Pipe::Warp {
                    look: 2,
                    flare: false,
                },
                "warp2",
            ),
            (Pipe::Spool { drive: 4, stage: 2 }, "spool4-2"),
            (Pipe::EventPart(7, "near lit"), "event7 near lit"),
            (Pipe::BattleFx { kind: 1, far: true }, "battlefx1far"),
            (
                Pipe::GlobeMoon {
                    slot: 1,
                    air: false,
                },
                "globemoon1-",
            ),
            (Pipe::CloudLayer(0, 1), "cloudlayer0x1"),
            (Pipe::SpaceBaked, "spacebaked"),
        ] {
            assert_eq!(pipe.to_string(), name);
        }
    }
}
