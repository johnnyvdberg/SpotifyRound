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
