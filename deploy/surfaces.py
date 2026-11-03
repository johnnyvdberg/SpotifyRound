#!/usr/bin/env python3
"""The photographed surface layers in assets/surfaces, from ambientCG.

    python3 -I deploy/surfaces.py [FILE ...]

downloads each material listed in assets/surfaces/surfaces.tsv from
ambientCG (CC0, https://ambientcg.com) at 1K and bakes it into the layer
`textures::paint_all` reads: 512 texels a side, the colour in RGB and the
displacement in alpha as the relief's height and the ambient occlusion
multiplied into the colour; and beside it `<file>-normal.png`, the normal
map's x and y, both scaled by the row's `relief`. A `tone` layer keeps only the
material's brightness, so the model's own colour gives the hue, as the
painted layers do; a `keep` layer keeps its colours. Either is scaled to a
mean of one and stored divided by the texture range (`textures::RANGE`),
exactly as the GPU takes it. Given FILEs, only those are made again.
"""

import io
import sys
import tempfile
import urllib.request
import zipfile
from pathlib import Path

from PIL import Image, ImageChops, ImageFilter

SIZE = 512
RANGE = 1.6
ROOT = Path(__file__).resolve().parent.parent / "assets" / "surfaces"


def fetch(material: str, into: Path) -> tuple[Image.Image, Image.Image, Image.Image, Image.Image]:
    url = f"https://ambientcg.com/get?file={material}_1K-JPG.zip"
    request = urllib.request.Request(url, headers={"User-Agent": "SpotifyRound-surfaces"})
    with urllib.request.urlopen(request, timeout=120) as response:
        archive = zipfile.ZipFile(io.BytesIO(response.read()))
    archive.extractall(into)
    colour = Image.open(into / f"{material}_1K-JPG_Color.jpg").convert("RGB")
    height = Image.open(into / f"{material}_1K-JPG_Displacement.jpg").convert("L")
    # Not every material has an occlusion map (polished marble has none to
    # speak of): without one, nothing is shaded.
    ao = into / f"{material}_1K-JPG_AmbientOcclusion.jpg"
    occlusion = Image.open(ao).convert("L") if ao.exists() else Image.new("L", colour.size, 255)
    normal = Image.open(into / f"{material}_1K-JPG_NormalGL.jpg").convert("RGB")
    return colour, height, occlusion, normal


def bake(
    colour: Image.Image,
    height: Image.Image,
    occlusion: Image.Image,
    normal: Image.Image,
    keep: bool,
    relief: float,
) -> tuple[Image.Image, Image.Image]:
    """The layer (colour, its occlusion multiplied in, and height) and its
    normals (the tangent-space x and y, GL's green up, scaled by `relief`)."""
    # The occlusion softened a little: the crevices' shade, not black.
    shade = occlusion.point(lambda a: round(255 * (a / 255) ** 0.8)).convert("RGB")
    colour = ImageChops.multiply(colour, shade).resize((SIZE, SIZE), Image.LANCZOS)
    # The height a little softened: a scan's grain, embossed texel by
    # texel, glitters instead of standing up.
    height = height.resize((SIZE, SIZE), Image.LANCZOS).filter(ImageFilter.GaussianBlur(1.0))
    normal = normal.resize((SIZE, SIZE), Image.LANCZOS)
    pixels = list(colour.get_flattened_data())
    tone = [0.30 * r + 0.55 * g + 0.15 * b for r, g, b in pixels]
    mean = sum(tone) / len(tone)
    # The height stretched over its 2nd to 98th percentiles, its middle at
    # one half, so every layer's relief spans the same range.
    heights = sorted(height.get_flattened_data())
    low, high = heights[len(heights) // 50], heights[-len(heights) // 50 - 1]
    mid = sum(heights) / len(heights)
    span = max(high - low, 1)

    def byte(v: float) -> int:
        return max(0, min(255, round(v * 255)))

    out = []
    for (r, g, b), t, h in zip(pixels, tone, height.get_flattened_data()):
        if keep:
            rgb = (r / mean, g / mean, b / mean)
        else:
            rgb = (t / mean,) * 3
        rise = 0.5 + relief * (h - mid) / span
        out.append(tuple(byte(c / RANGE) for c in rgb) + (byte(rise),))
    baked = Image.new("RGBA", (SIZE, SIZE))
    baked.putdata(out)
    bent = [
        (byte(0.5 + relief * (x / 255 - 0.5)), byte(0.5 + relief * (y / 255 - 0.5)), 128)
        for x, y, _ in normal.get_flattened_data()
    ]
    normals = Image.new("RGB", (SIZE, SIZE))
    normals.putdata(bent)
    return baked, normals


def main() -> None:
    only = set(sys.argv[1:])
    for line in (ROOT / "surfaces.tsv").read_text().splitlines():
        if not line.strip() or line.startswith("#"):
            continue
        file, material, colour, relief, _ = line.split("\t")
        if only and file not in only:
            continue
        with tempfile.TemporaryDirectory() as tmp:
            baked, normals = bake(
                *fetch(material, Path(tmp)), keep=colour == "keep", relief=float(relief)
            )
        baked.save(ROOT / file, optimize=True)
        normals.save(ROOT / file.replace(".png", "-normal.png"), optimize=True)
        print(f"{file} <- {material} ({colour})")


if __name__ == "__main__":
    main()
