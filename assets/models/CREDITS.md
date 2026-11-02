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
