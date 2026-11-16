# A city generator for the world inside the starbase: research notes

Implemented 2026-11-16 in `starbase/city.rs` (Rust: the table, the buildings) and
`shaders/solar/starbase/city.wesl` + `ground.wesl` (the map). These are the findings of a research pass (2026-11-16) into an algorithm for
realistic cities and urban sprawl that fits how the world inside the base is drawn
([starbase-plan.md](starbase-plan.md) §7): a ground map painted one texel at a time by a shader
(`starbase/world.wesl`), and building meshes placed by Rust as a pure function of the seed
(`starbase/world.rs`). Whatever both must agree on has to be closed-form arithmetic written the
same in WGSL and Rust.

## A fault to fix first

The map's coordinate round the cylinder is not an angle (`around` in `pane.wesl` is a diamond
mapping), so a texel covers about 8 m of ground at the axis directions and 16 m at the diagonals.
`town_at` sizes towns in texels and `world.rs` in metres, so a painted town is an ellipse under a
circle of meshes. From reading the code; not checked in a render. The fix, and the basis of the
rest: do all city arithmetic in ground metres `(R·θ, z)` on both sides.

## What was compared

| Approach | Fits a per-texel, closed-form map mirrored in Rust? |
|---|---|
| Correlated percolation in a density gradient (Makse, Havlin & Stanley) | Yes. The core of the recommendation: a compact centre, a ragged fractal edge, satellite fragments |
| Clark's negative-exponential density, summed over centres | Yes. The gradient the percolation is cut against |
| Zipf rank-size for town sizes | Yes. Size from the site's index |
| Gravity-model arterial roads between town sites | Yes, as a table Rust builds and both read |
| Voronoi district cells, a street template each | Yes. Districts become irregular polygons, not pie slices |
| Distance to road and water as attraction | Yes, once roads and rivers are analytic |
| Tensor-field streets (Chen et al.) | No (traced streamlines). Approximate: one orientation a district cell |
| L-system roads (Parish & Müller) | No (sequential growth) |
| DLA, agents, cellular automata | No (simulations) |

## The recommended algorithm, in short

1. **Shared primitives**: `hash_cell` ported line for line, and a tiling value noise built on it,
   so Rust can evaluate what the shader does. The GPU's noise texture stays for cosmetic terrain.
2. **Sites and sizes**: the present quasi-random sites, sized by rank (`1/(k+1)`, Zipf), each a
   Clark kernel `exp(-r/b)` with `b` from about 1600 m for the largest to 190 m for villages; the
   largest few with two to four sub-centres.
3. **Arterials**: each town joined to the earlier town that pulls it hardest (rank over distance
   squared), plus its nearest neighbour when close: a tree with loops. Edges bent a little, stored
   as straight sub-segments in a table. Rivers as sums of sines along the axis.
4. **Built or not**: density `D` = the kernels + ribbons along highways, arterials and waterfronts
   − hills; a texel is built where `D − 0.35 + 0.22·noise > 0`. Cores are always built, the fringe
   half built in correlated patches, leapfrog suburbs beyond it, strips along the roads.
5. **Districts**: Voronoi cells about 400 m across, each classified once at its own site
   (rural, park, commercial core, industrial along highways and docks, residential by density);
   cell edges are collector streets.
6. **Streets**: a block grid a cell, turned to the nearest arterial (or radial in an old core, or
   by a hash), block sizes by district, a slight warp so suburbs curve.
7. **Buildings**: lots from the same grid, kept where the point is built and clear of roads and
   water; height from `D` with a lognormal spread; height, footprint and spacing by district;
   every lot's roof painted into the map too, so a building that drops out with distance leaves
   its roof behind.

**Detail with distance**: keep a building while it stands about three pixels tall, tallest first,
fading rather than popping: under 6 km everything from 12 m up; 6 to 10 km, 20 m up as plain
prisms; 10 to 20 km, towers and landmarks only; beyond, the map alone.

**A skyline that reads as large**: several height peaks with gaps between, not one cone; neighbours
differing by a fifth or more; mostly slender towers, some slabs, a landmark or two; a carpet of
small buildings at their feet; low saturation, roofs cooler and darker than walls; haze that
desaturates and lifts the blacks with distance, the same on meshes and map; a dark footprint and a
lit street grid under each cluster.

## Sources

Read in full: Makse, Andrade, Batty, Havlin & Stanley 1998, <https://arxiv.org/abs/cond-mat/9809431>.
From abstracts or summaries: Parish & Müller 2001; Chen et al. 2008
(<https://www.sci.utah.edu/~chengu/street_sig08/street_project.htm>); Clark's law
(<https://arxiv.org/pdf/1609.07697>); Gabaix on Zipf; Batty, Longley & Fotheringham 1989; Batty et
al. on London's building heights (<https://arxiv.org/pdf/0804.2442>); Schläpfer, Lee & Bettencourt
(<https://arxiv.org/pdf/1512.00946>).

The lengths, thresholds, block sizes and heights above are calibrations for this world, not sourced
figures, and the skyline advice is reasoning from those sources rather than findings in them.

## As built

- Districts are classed once in Rust and handed to the bake in the table with the kernels and the
  roads (`City::table`), not re-derived per texel: the shader's own classing made the ground
  layer's bake spill badly on the V3D.
- The gravity pull is rank over distance cubed, not squared: squared joined nearly every town to the
  largest one, a star of long roads.
- The noise the density is cut against is value noise scaled ×1.6, as its spread is narrower than
  the ±1 the 0.22 assumed.
- Houses (low suburbs) are roofs on the map only, never meshes; the sprawl patch is no longer placed.
- The cores were made more prominent after: blocks of 130 m and lots of 55 (not 110 and 45), heights
  `(30 + 520 k²) e^(0.45 g)` up to 700 m, lighter hulls and brighter windows, and the tallest kept to
  32 km. Cars are lights on the road table's pieces (`roads.rs`), not the map alone.
- The haze is now exponential on both, two-thirds at 11 km, for the 2,000 km world (its far end
  out of sight); the map's lengths are mirrored past the port and their relief lowered at their
  ends, under the bulkheads (`sections.rs`).
- The map's haze was brought to the buildings' (9 km), not theirs to the map's (14 km): the
  per-pixel test to tell a world building from a port structure cost `fs_base` its four threads.
