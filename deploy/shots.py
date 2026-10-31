#!/usr/bin/env python3
"""Pages that show what a change did to the Nautilus' pictures, at a glance.

Used by deploy/shots.sh; run it through that, which finds an interpreter with
numpy and Pillow.

  shots.py index RUN_DIR
      Write RUN_DIR/index.html: a contact sheet per sea, every place of the
      run in the order the voyage lists them (the places come from
      dive3d::places::all, which follows the voyage's legs, so a new leg shows
      up by itself).
  shots.py compare A_DIR B_DIR OUT_HTML
      Compare two runs place by place. Writes OUT_HTML (next to the two run
      directories), an amplified difference image per changed place in
      OUT_HTML's sibling directory, and prints the table. Sorted most-changed
      first; each place shows before | after | difference, and a score:
      the share of pixels that moved by more than 2/255 and the mean change.
"""

import html
import re
import sys
from pathlib import Path

import numpy as np
from PIL import Image

# A pixel counts as changed above this many levels (of 255) in some channel.
# The difference image is amplified so a change of one level shows.
THRESHOLD = 2
AMPLIFY = 16
THUMB = 360


def sea_of(name):
    """The sea a place belongs to: 3-cave-middle -> cave, helm-swipe-x -> helm."""
    m = re.match(r"^\d+-([a-z]+)-", name)
    if m:
        return m.group(1)
    m = re.match(r"^([a-z]+)", name)
    return m.group(1) if m else name


def places(run):
    """The run's pictures, by place, in the order the test wrote its index."""
    order = []
    index = run / "index.html"
    if index.exists():
        order = re.findall(r'<a href="([^"]+\.png)"', index.read_text())
    found = {p.name for p in run.glob("*.png") if p.name != "sheet.png"}
    ordered = [n for n in order if n in found]
    return ordered + sorted(found - set(ordered))


def load(path):
    return np.asarray(Image.open(path).convert("RGB"), dtype=np.int16)


def thumb(path, out, size=THUMB):
    img = Image.open(path).convert("RGB")
    img.thumbnail((size, size))
    img.save(out, "JPEG", quality=85)


PAGE = """<!doctype html><meta charset=utf-8><title>{title}</title>
<style>
body{{background:#111;color:#ddd;font:14px/1.4 sans-serif;margin:16px}}
h1,h2{{font-weight:500}} h2{{margin-top:28px;border-bottom:1px solid #333}}
a{{color:#8cf}} .grid{{display:flex;flex-wrap:wrap;gap:12px}}
figure{{margin:0}} figcaption{{max-width:{w}px;font-size:12px}}
img{{width:{w}px;display:block;background:#000}}
.row{{display:flex;gap:6px;margin:10px 0}} .row figure{{flex:none}}
.score{{font-variant-numeric:tabular-nums;color:#fc8}}
table{{border-collapse:collapse}} td,th{{padding:2px 10px;text-align:right}}
td:first-child,th:first-child{{text-align:left}}
details{{margin:12px 0}}
</style>
"""


def index(run):
    run = Path(run)
    names = places(run)
    thumbs = run / "thumbs"
    thumbs.mkdir(exist_ok=True)
    seas = {}
    for n in names:
        thumb(run / n, thumbs / (n[:-4] + ".jpg"))
        seas.setdefault(sea_of(n), []).append(n)
    out = [PAGE.format(title=f"Shots: {run.name}", w=THUMB)]
    out.append(f"<h1>{html.escape(run.name)}: {len(names)} places</h1>")
    out.append("<p>" + " ".join(f'<a href="#{s}">{s}</a>' for s in seas) + "</p>")
    for sea, group in seas.items():
        out.append(f'<h2 id="{sea}">{sea} ({len(group)})</h2><div class="grid">')
        for n in group:
            t = n[:-4]
            out.append(
                f'<figure><a href="{n}"><img loading="lazy" src="thumbs/{t}.jpg"></a>'
                f"<figcaption>{html.escape(t)}</figcaption></figure>"
            )
        out.append("</div>")
    (run / "index.html").write_text("\n".join(out))
    print(f"wrote {run / 'index.html'}")


def compare(a_dir, b_dir, out_html):
    a_dir, b_dir, out_html = Path(a_dir), Path(b_dir), Path(out_html)
    work = out_html.with_suffix("")
    work.mkdir(parents=True, exist_ok=True)
    a_names, b_names = places(a_dir), places(b_dir)
    both = [n for n in a_names if n in set(b_names)]
    rows = []
    for n in both:
        a, b = load(a_dir / n), load(b_dir / n)
        if a.shape != b.shape:
            rows.append((n, None, None, None))
            continue
        d = np.abs(a - b)
        share = (d.max(axis=2) > THRESHOLD).mean() * 100
        mean = d.mean()
        peak = int(d.max())
        stem = n[:-4]
        if peak > 0:
            diff = np.clip(d * AMPLIFY, 0, 255).astype(np.uint8)
            Image.fromarray(diff).save(work / f"{stem}-diff.png")
        thumb(a_dir / n, work / f"{stem}-a.jpg")
        thumb(b_dir / n, work / f"{stem}-b.jpg")
        thumb_diff = work / f"{stem}-diff.png"
        if peak > 0:
            im = Image.open(thumb_diff)
            im.thumbnail((THUMB, THUMB))
            im.save(work / f"{stem}-d.jpg", "JPEG", quality=85)
        rows.append((n, share, mean, peak))
    changed = sorted((r for r in rows if r[1] is not None and r[3] > 0), key=lambda r: -r[1])
    same = [r for r in rows if r[1] is not None and r[3] == 0]
    odd = [r for r in rows if r[1] is None]
    added = [n for n in b_names if n not in set(a_names)]
    removed = [n for n in a_names if n not in set(b_names)]

    print(f"{'place':44} {'moved>2':>8} {'mean':>7} {'max':>4}")
    for n, share, mean, peak in changed:
        print(f"{n[:-4]:44} {share:7.2f}% {mean:7.3f} {peak:4d}")
    print(
        f"{len(both)} places in both: {len(changed)} changed, {len(same)} identical, "
        f"{len(odd)} different size, {len(added)} only in {b_dir.name}, "
        f"{len(removed)} only in {a_dir.name}"
    )

    seas = {}
    for n, share, mean, peak in changed:
        seas.setdefault(sea_of(n), []).append(share)
    out = [PAGE.format(title=f"Compare {a_dir.name} / {b_dir.name}", w=THUMB)]
    out.append(f"<h1>{html.escape(a_dir.name)} &rarr; {html.escape(b_dir.name)}</h1>")
    out.append(
        f"<p>{len(both)} places in both: <b>{len(changed)} changed</b>, {len(same)} identical. "
        f"Score = share of pixels that moved by more than {THRESHOLD}/255; the third picture shows "
        f"the difference times {AMPLIFY}.</p>"
    )
    if seas:
        out.append("<table><tr><th>sea</th><th>places changed</th><th>worst</th><th>mean</th></tr>")
        for sea, shares in sorted(seas.items(), key=lambda kv: -max(kv[1])):
            out.append(
                f"<tr><td>{sea}</td><td>{len(shares)}</td><td>{max(shares):.2f}%</td>"
                f"<td>{sum(shares) / len(shares):.2f}%</td></tr>"
            )
        out.append("</table>")
    for n, share, mean, peak in changed:
        s = n[:-4]
        cap = f"{html.escape(s)}"
        out.append(
            f'<div class="row" id="{html.escape(s)}">'
            f'<figure><a href="{a_dir.name}/{n}"><img loading="lazy" src="{work.name}/{s}-a.jpg"></a>'
            f"<figcaption>before: {cap}</figcaption></figure>"
            f'<figure><a href="{b_dir.name}/{n}"><img loading="lazy" src="{work.name}/{s}-b.jpg"></a>'
            f'<figcaption>after <span class="score">{share:.2f}% moved, mean {mean:.3f}, max {peak}'
            f"</span></figcaption></figure>"
            f'<figure><a href="{work.name}/{s}-diff.png"><img loading="lazy" src="{work.name}/{s}-d.jpg"></a>'
            f"<figcaption>difference x{AMPLIFY}</figcaption></figure></div>"
        )
    if same:
        out.append(
            f"<details><summary>{len(same)} identical places</summary><p>"
            + ", ".join(html.escape(r[0][:-4]) for r in same)
            + "</p></details>"
        )
    for label, items in (("only in after", added), ("only in before", removed)):
        if items:
            out.append(
                f"<details><summary>{len(items)} places {label}</summary><p>"
                + ", ".join(html.escape(i[:-4]) for i in items)
                + "</p></details>"
            )
    if odd:
        out.append("<p>Different size: " + ", ".join(html.escape(r[0]) for r in odd) + "</p>")
    out_html.write_text("\n".join(out))
    print(f"wrote {out_html}")


if __name__ == "__main__":
    cmd = sys.argv[1] if len(sys.argv) > 1 else ""
    if cmd == "index" and len(sys.argv) == 3:
        index(sys.argv[2])
    elif cmd == "compare" and len(sys.argv) == 5:
        compare(*sys.argv[2:])
    else:
        sys.exit(__doc__)
