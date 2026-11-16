# Textures

Photographed metals from ambientCG (https://ambientcg.com), released into the public domain under
CC0 (https://creativecommons.org/publicdomain/zero/1.0/). Each is the colour map of the 1K JPG set,
scaled to 512 pixels; the sets' other maps (normal, roughness, metalness, displacement) are not
used. They tile, and are laid on the starbase's structures as shades alone (near grey as they are), a
colour channel each in one layer of the base's maps (`bake::write_metals`, `base_metal` in
`src/shaders/solar/starbase/surface.wesl`).

| File | Asset | Used for | Source |
|---|---|---|---|
| metal-plates-008.jpg | Metal Plates 008 | walls and hulls | https://ambientcg.com/a/MetalPlates008 |
| metal-plates-004.jpg | Metal Plates 004 | frames and ribs | https://ambientcg.com/a/MetalPlates004 |
| diamond-plate-008c.jpg | Diamond Plate 008 C | decks and catwalks | https://ambientcg.com/a/DiamondPlate008C |
