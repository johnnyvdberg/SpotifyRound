# The Microscope

One of the idle visualisers (see the [feature overview](features.md)): a naturalist's slides under
the objective. The code is `src/microscope.rs`, its shader `src/shaders/lens/microscope.wgsl`; the
plumbing it shares with the [observatory](observatory.md) is in `src/lens.rs`.

<p align="center"><img src="microscope.jpg" width="640" alt="The fourteen slides"></p>

## The slides

| Slide | Lighting | On it |
| --- | --- | --- |
| Pond water | Bright field | Paramecia, amoebae, Closterium and diatoms |
| Diatoms | Dark field | Pennate and centric diatoms, their glass edges shining in colour |
| Diatoms, Rheinberg | Rheinberg | The same, gold on a deep blue ground |
| Volvox | Bright field | Rolling colonies with daughter colonies inside |
| Desmids | Bright field | Closterium, Pediastrum and diatoms |
| Hay infusion | Phase contrast | A crowd of paramecia, bacteria darting between them |
| Amoebae | Bright field | Amoebae creeping among diatoms |
| Euglena bloom | Bright field | A swarm of Euglena |
| Vorticella on a twig | Phase contrast | Bells on stalks that snap short and stretch out again |
| Spirogyra | Bright field | Filaments wound with spiral chloroplasts, Euglena among them |
| Moss water | Dark field | A tardigrade lumbering through, with diatoms and amoebae |
| Pediastrum | Rheinberg | Star-shaped colonies, gold on green |
| Onion skin, iodine | Bright field | Long walled cells, their nuclei stained brown |
| Blood smear, Giemsa | Bright field | Red cells with pale middles, the odd purple white cell, platelets |

A slide stays under the objective for 85 seconds, in an order shuffled on every visit, and each
visit puts a different number of each organism on it. Changing it takes five seconds: the view
blurs and the stage slides away, then the next slide slides in and comes into focus. A **swipe**
changes to the next slide or back; a tap returns to the player.

Partway through a slide the **nosepiece** turns to a stronger objective: the hand on the stage first
brings the nearest organism to the middle and keeps it there as it moves, and the view comes back
into focus closer in. The stage also drifts slowly across the slide. The HUD names the slide, its
lighting, what is in view and the objective in use.

## What lives there

Each organism is a small signed-distance shape with its own fine structure:

- **Paramecium**: a slipper with an oral groove, cilia beating round the edge in a travelling wave,
  granular cytoplasm and two contractile vacuoles filling and emptying in turn. It swims on a
  curving path, swaying.
- **Euglena**: a green spindle that squirms as it swims, with chloroplasts, a red eyespot and a
  whipping flagellum ahead of it.
- **Pennate diatoms** (*Navicula*): glass boats with rows of pores across them and the raphe down
  the middle, gliding back and forth along it.
- **Centric diatoms** (*Cyclotella*): pill boxes of radial ribs and rings of pores.
- **Volvox**: a sphere of cells that rolls as it drifts, three daughter colonies turning inside.
- **Amoeba**: a blob whose pseudopods slowly reach out and pull back, with a nucleus and granules.
- **Vorticella**: a bell with a crown of cilia on a stalk anchored to the slide; every ten seconds
  or so the stalk coils and snaps short, then stretches out again.
- **Closterium**: a tapering green crescent with clear tips.
- **Spirogyra**: a filament of cells, each wound with a ribbon of chloroplast.
- **Pediastrum**: a flat star of cells in rings, the outer ones horned.
- **A tardigrade**: a plump segmented body on legs that swing as it walks, its last pair pointing
  back.
- **Bacteria and debris**: rods darting about, and specks jiggling in Brownian motion.

They live on a torus a little wider than the field, centred on the view, so one that swims out of
sight comes back on the other side, and wraps only outside the field stop. Every pose is a pure
function of the clock and the visit's seed.

## Light and focus

The **fine focus** drifts slowly down through the drop and back, once a minute, so one organism
sharpens as another blurs: away from the focal plane an outline spreads and its fine structure
fades. Tissue lies flat, so on it the focus hardly moves. In **bright field** pigment absorbs the
lamp's light, outlines are darkest, and a refracting edge throws a faint halo just outside itself.
In **dark field** only scattered light shows: bright outlines on black, a diatom's glass edge in
colour; **Rheinberg** lighting puts them on a coloured ground. In **phase contrast** whatever slows
the light shows dark, with a bright halo round it.

## Cost

One draw a frame from one 608-byte uniform block: up to sixteen organisms, each skipped by a
bounding test for pixels nowhere near it, one hashed cell each of debris, bacteria and tissue.
`cargo test --release lens::tests::frame_time -- --ignored` times it; on the same adapter it
costs about a third of the Earth's frame.
