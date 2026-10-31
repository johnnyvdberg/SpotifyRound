#!/usr/bin/env python3
"""Make every mouse cursor on the kiosk transparent.

The round panel's touch controller sometimes enumerates as a mouse. cage then
draws its arrow (`left_ptr`) on the display, and it stays there: touches never
move it into the app, so the app's own `mouse-cursor: none` never gets a say.
Both cage (wlroots) and the app (winit) load their cursors from the user's
`default` cursor theme, which every loader looks up in ~/.icons before
/usr/share/icons (not all of them search ~/.local/share/icons), and the first
image found under a name wins. So a 1x1 transparent image under every name
the system's themes know leaves nothing to draw, whatever the controller
claims to be.

Run it on the Pi as the kiosk's user (no root needed); cage picks it up the
next time it starts:

    ssh <user>@<pi> python3 - < deploy/invisible-cursors.py
"""

import os
import struct

THEMES = "/usr/share/icons"
OUT = os.path.expanduser("~/.icons/default/cursors")
# The ones cage and winit ask for, in case no installed theme lists them.
NAMES = {"left_ptr", "default", "pointer", "hand1", "hand2", "text", "xterm",
         "crosshair", "grab", "grabbing", "wait", "watch", "progress", "not-allowed"}


def transparent():
    """An Xcursor file with one 1x1 fully transparent image at size 24."""
    size = 24
    header = struct.pack("<4sIII", b"Xcur", 16, 0x10000, 1)
    image_type = 0xFFFD0002
    toc = struct.pack("<III", image_type, size, 16 + 12)
    image = struct.pack("<IIIIIIIII", 36, image_type, size, 1, 1, 1, 0, 0, 0)
    pixel = struct.pack("<I", 0)
    return header + toc + image + pixel


def main():
    names = set(NAMES)
    for theme in os.listdir(THEMES) if os.path.isdir(THEMES) else []:
        cursors = os.path.join(THEMES, theme, "cursors")
        if os.path.isdir(cursors):
            names.update(os.listdir(cursors))
    os.makedirs(OUT, exist_ok=True)
    data = transparent()
    for name in sorted(names):
        path = os.path.join(OUT, name)
        if os.path.islink(path) or os.path.exists(path):
            os.remove(path)
        with open(path, "wb") as f:
            f.write(data)
    print(f"{len(names)} transparent cursors in {OUT}")


main()
