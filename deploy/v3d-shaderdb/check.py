"""Check a shaderdb table against the committed baseline.

    check.py baseline.txt new.txt

Fails (exit 1) when any pipeline's shader runs at fewer threads than in the
baseline or spills where it did not: on the Pi's V3D a shader that drops from
four threads to two costs about twice as much for every pixel or vertex it
shades, and that is the cliff a change can fall off without a visible
difference in the code. Instruction counts are reported, not enforced.
"""
import sys

STAGES = ("FS", "VS", "VS-bin")


def load(path):
    rows = {}
    for line in open(path):
        parts = line.split()
        stage = next((s for s in STAGES if s in parts), None)
        if stage is None:
            continue
        i = parts.index(stage)
        key = (" ".join(parts[: i - 1]), parts[i - 1], stage)
        spills = int(parts[i + 6].split(":")[0])
        rows[key] = (int(parts[i + 1]), int(parts[i + 2]), spills)
    return rows


base, new = load(sys.argv[1]), load(sys.argv[2])
if not new:
    sys.exit("no shaders in the new table: the build or the run failed")
failures, changes = [], []
for key in sorted(set(base) | set(new)):
    name = f"{key[0]:32} {key[1]:16} {key[2]:6}"
    if key not in new:
        changes.append(f"{name} gone")
        continue
    if key not in base:
        changes.append(f"{name} new: {new[key][0]} inst, {new[key][1]} threads, {new[key][2]} spills")
        continue
    (bi, bt, bs), (ni, nt, ns) = base[key], new[key]
    if nt < bt or ns > bs:
        failures.append(f"{name} {bi} inst {bt} threads {bs} spills -> {ni} inst {nt} threads {ns} spills")
    elif (bi, bt, bs) != (ni, nt, ns):
        changes.append(f"{name} {bi} -> {ni} inst, {bt} -> {nt} threads, {bs} -> {ns} spills")

for stage in STAGES:
    b = sum(v[0] for k, v in base.items() if k[2] == stage)
    n = sum(v[0] for k, v in new.items() if k[2] == stage)
    print(f"{stage:6} instructions, all pipelines: {b} -> {n} ({n - b:+})")
if changes:
    print(f"\n{len(changes)} changed:")
    print("\n".join(changes))
if failures:
    print(f"\nFAILED: {len(failures)} shaders lost threads or started spilling:")
    print("\n".join(failures))
    sys.exit(1)
print("\nok: no shader lost a thread or started spilling")
