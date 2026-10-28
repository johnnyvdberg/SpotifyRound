"""Compare two shaderdb tables: inst/threads/spills per pipeline and stage."""
import sys


def load(path):
    rows = {}
    for line in open(path):
        parts = line.split()
        if "FS" not in parts and "VS" not in parts and "VS-bin" not in parts:
            continue
        for stage in ("FS", "VS", "VS-bin"):
            if stage in parts:
                i = parts.index(stage)
                break
        key = (" ".join(parts[: i - 1]), parts[i - 1], stage)
        rows[key] = (int(parts[i + 1]), int(parts[i + 2]), parts[i + 6])
    return rows


a, b = load(sys.argv[1]), load(sys.argv[2])
for key in sorted(set(a) | set(b)):
    x, y = a.get(key), b.get(key)
    if x == y:
        continue
    print(f"{key[0]:30} {key[1]:14} {key[2]:7} {x} -> {y}")
