# Imported models

3D scans of plaster casts in The Royal Cast Collection, Statens Museum for
Kunst (SMK, the National Gallery of Denmark), released into the public
domain (https://creativecommons.org/publicdomain/mark/1.0/) through
https://open.smk.dk. Each was reduced from about 400,000 triangles to 3,000
with meshoptimizer, set y up, a unit tall and centred on its footprint.

| File | Object | Source |
|---|---|---|
| sophocles.obj | KAS633, Portrait of the Greek tragedian Sophocles, standing | https://open.smk.dk/artwork/image/KAS633 |
| belvedere-torso.obj | KAS402, Belvedere Torso | https://open.smk.dk/artwork/image/KAS402 |
| bearded-head.obj | KMS5962, Head of a bearded man | https://open.smk.dk/artwork/image/KMS5962 |

`deploy/models.sh` downloads the full-resolution originals again (into
target/models/) and rebuilds these files from `models.tsv`; raise a
triangle count there for a higher quality level.

## Poly Haven

CC0 models from Poly Haven (https://polyhaven.com, released into the public
domain under https://creativecommons.org/publicdomain/zero/1.0/), fetched as
their 1k glTF through https://api.polyhaven.com. `tools/decimate` reduces
each and bakes its diffuse (base colour) texture into the vertices' colours,
`v x y z r g b`; `polyhaven:ASSET#PICK` in `models.tsv` keeps only some of a
kit's nodes.

| File | Object | Source |
|---|---|---|
| bronze-shark.obj | Bronze Shark Statue, by Tina | https://polyhaven.com/a/bronze_shark_statue |
| bronze-ray.obj | Bronze Ray Statue, by Tina | https://polyhaven.com/a/bronze_ray_statue |
| bronze-whale.obj | Bronze Whale Statue, by Tina | https://polyhaven.com/a/bronze_whale_statue |
| treasure-chest.obj | Treasure Chest, by Rico Cilliers | https://polyhaven.com/a/treasure_chest |
| barrel.obj | Wooden Barrels 01 (its first whole barrel), by James Ray Cock | https://polyhaven.com/a/wooden_barrels_01 |
| wreck-fluyt.obj | Dutch Ship Medium, its hull (no rigging or sails), by James Ray Cock, Rico Cilliers and Nicolò Zubbini | https://polyhaven.com/a/dutch_ship_medium |
| wreck-galleon.obj | Dutch Ship Large 01, its hull, by the same | https://polyhaven.com/a/dutch_ship_large_01 |
| wreck-pinas.obj | Dutch Ship Large 02, its hull, by the same | https://polyhaven.com/a/dutch_ship_large_02 |
| fort-wall.obj | Modular Fort 01, its `wall_thick_straight_01`, by Rico Cilliers | https://polyhaven.com/a/modular_fort_01 |
| fort-tower.obj | Modular Fort 01, its `tower_round` | https://polyhaven.com/a/modular_fort_01 |
| fort-gate.obj | Modular Fort 01, its `wall_thin_gate_01` | https://polyhaven.com/a/modular_fort_01 |
| fort-end.obj | Modular Fort 01, its `wall_thick_end_01` | https://polyhaven.com/a/modular_fort_01 |
| cannon.obj | Cannon 01 without its balls, by James Ray Cock (model) and Yann Kervran (rig) | https://polyhaven.com/a/cannon_01 |

## Smithsonian

CC0 3D scans from the Smithsonian Institution's National Museum of Natural
History, through Smithsonian Open Access (https://www.si.edu/openaccess,
released into the public domain under
https://creativecommons.org/publicdomain/zero/1.0/), fetched as their
non-Draco download glTF through https://3d-api.si.edu. `tools/decimate`
reduces each to a unit tall and bakes its base colour texture into the
vertices' colours, or, for the casts scanned without one, its occlusion.
The underwater museum streams them in as it comes near and drops them again
once it is left behind (`dive3d::museum`).

| File | Specimen | Source |
|---|---|---|
| museum-mammoth.obj | Mammuthus primigenius (Blumbach) | https://3d-api.si.edu/content/document/3d_package:341c96cd-f967-4540-8ed1-d3fc56d31f12/resources/woolly-mammoth-100k-4096_std.glb |
| museum-ground-sloth.obj | Megalonyx leptostomus Cope | https://3d-api.si.edu/content/document/3d_package:f05ed78e-c2b3-4270-a3e5-8ceacb6130cc/resources/Megalonyx_leptostomus_Cope-150k-4096_std.glb |
| museum-triceratops.obj | Triceratops horridus Marsh, 1889 | https://3d-api.si.edu/content/document/3d_package:d8c623be-4ebc-11ea-b77f-2e728ce88125/resources/Triceratops_horridus_Marsh_1889-150k-4096.glb |
| museum-megalodon.obj | Carcharocles megalodon | https://3d-api.si.edu/content/document/3d_package:106e215b-a829-40c6-8e4d-dfba37350f36/resources/carcharodon-megalodon-(usnm-v-10333)-150k-4096.glb |
| museum-archaeopteryx.obj | Archaeopteryx siemensii Dames | https://3d-api.si.edu/content/document/3d_package:391660da-7c49-499c-91f5-88a298686c09/resources/USNM_PAL_509743-100k-2048_std.glb |
| museum-walrus-whale.obj | Odobenocetops peruvianus De Muizon | https://3d-api.si.edu/content/document/3d_package:3b896f42-b8f5-4798-af90-ad46e5d9fcb3/resources/walrus_whale-100k-web.glb |
| museum-pilot-whale.obj | Globicephala melas melas (Traill, 1809): Skull and Mandible | https://3d-api.si.edu/content/document/3d_package:a8ab02ba-bcae-470a-9df4-db1265b25905/resources/USNM_VZ_20957_pilot_whale_skull-100k-2048_std.glb |
| museum-tortoise.obj | Geochelone carbonaria | https://3d-api.si.edu/content/document/3d_package:70b1ce97-2659-4878-b926-a6e75e7f0b35/resources/USNM_222486-100k-2048_std.glb |
| museum-porcupinefish.obj | Diodon hystrix | https://3d-api.si.edu/content/document/3d_package:e0bdb234-d604-41fd-8f47-27368159c476/resources/usnm_195928-100k-2048_std.glb |
| museum-murex.obj | Venus Comb Murex, Venus comb shell | https://3d-api.si.edu/content/document/3d_package:e4f2f155-de31-4b46-b769-2783571f86e2/resources/usnm_1127933-100k-2048_std.glb |
| museum-diplocaulus.obj | Diplocaulus magnicornis: Skull | https://3d-api.si.edu/content/document/3d_package:23a3cd17-8526-4b30-80e7-fdae0ecdd36a/resources/Diplocaulus_magnicornis-150k-4096_std.glb |
| museum-stegosaurus-spike.obj | Stegosaurus stenops Marsh, 1887: A: Tail Spike | https://3d-api.si.edu/content/document/3d_package:68adcbc1-84bc-4e3c-9f85-a08c81a5a820/resources/Stegosaurus_stenops_Marsh__1887-150k-4096_std.glb |
| museum-dinictis.obj | Dinictis cyclops (Cope): presentation | https://3d-api.si.edu/content/document/3d_package:71fede81-9be4-4fc8-9422-402aeaa82c4d/resources/Dinictis_cyclops_-Cope-150k-4096_std.glb |
| museum-trilobite.obj | Psychopyge elegans Termier & Termier, 1950 | https://3d-api.si.edu/content/document/3d_package:e4518faa-9603-4e1b-8375-e62d1fda079b/resources/Psychopyge_elegans_Termier_-_Termier__1950-150k-4096_std.glb |
| museum-deer-skull.obj | White-tailed Deer: Skull and Mandible | https://3d-api.si.edu/content/document/3d_package:fcc69841-d8be-4103-be0f-143e374c9ce6/resources/USNM_PAL_Deer_mandible-100k-2048_std.glb |
| museum-king-crab.obj | Lithodidae | https://3d-api.si.edu/content/document/3d_package:451bcb49-01f9-4030-9d89-6f976b64572b/resources/Lithodidae-150k-4096_std.glb |
| museum-allosaurus.obj | Allosaurus fragilis Marsh, 1877: Maxilla | https://3d-api.si.edu/content/document/3d_package:1a090f67-0f38-438b-b0ac-da2f5ab434ba/resources/Allosaurus_fragilis_Marsh__1877-150k-4096_std.glb |
| museum-seadragon.obj | Phyllopteryx taeniolatus | https://3d-api.si.edu/content/document/3d_package:4d6a5e3c-65a0-41e9-81d5-9f4116139471/resources/usnm_111086-100k-2048_std.glb |
