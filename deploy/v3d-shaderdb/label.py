"""Turn the raw log of inner.sh into one labelled line per compiled V3D shader.

Mesa (patched by patch-mesa.sh) prints, for each pipeline, its SHADER-DB lines
while compiling, then `PIPELINE-NAME <handle> <label>` when wgpu names the
pipeline right after creating it. So every SHADER-DB line belongs to the next
PIPELINE-NAME. Pipelines sharing a label are numbered #0, #1, ... in creation
order.
"""
import re
import sys
from collections import Counter

SHADERDB = re.compile(
    r"SHADER-DB-(?P<name>.*?) - MESA_SHADER_(?P<stage>\w+) shader: (?P<inst>\d+) inst, "
    r"(?P<threads>\d+) threads, (?P<loops>\d+) loops, (?P<uniforms>\d+) uniforms, "
    r"(?P<temps>\d+) max-temps, (?P<spills>\d+):(?P<fills>\d+) spills:fills, "
    r"(?P<sfu>\d+) sfu-stalls, (?P<stalls>\d+) inst-and-stalls, (?P<nops>\d+) nops"
)
NAME = re.compile(r"PIPELINE-NAME 0x[0-9a-f]+ (?P<label>.*)$")

STAGES = {"VERTEX": "VS", "VERTEX_BIN": "VS-bin", "FRAGMENT": "FS", "COMPUTE": "CS"}

rows, pending, seen = [], [], Counter()
status = None
for line in open(sys.argv[1], errors="replace"):
    line = line.rstrip("\n")
    if m := SHADERDB.search(line):
        if re.match(r"[0-9a-f]{64}:", m["name"]):
            pending.append(m)
        else:
            # Mesa's own meta shaders (queries, events), named by purpose.
            rows.append(("(mesa internal)", m))
    elif m := NAME.search(line):
        label = m["label"]
        n = seen[label]
        seen[label] += 1
        if not pending:
            rows.append((f"{label} #{n}", None))
        for p in pending:
            rows.append((f"{label} #{n}", p))
        pending = []
    elif line.startswith("== test exit status"):
        status = line
for p in pending:
    # Mesa-internal (meta) shaders and anything compiled without a name after it.
    rows.append(("(unnamed)", p))

print(f"{'pipeline':32} {'entry':14} {'stage':6} {'inst':>5} {'thr':>3} {'loops':>5} "
      f"{'unif':>4} {'temps':>5} {'spill:fill':>10} {'sfu':>4} {'nops':>4}")
for label, p in rows:
    if p is None:
        print(f"{label:32} (no shaders compiled for this pipeline)")
        continue
    entry = p["name"].rsplit(":", 1)[-1] if ":" in p["name"] else p["name"]
    stage = STAGES.get(p["stage"], p["stage"])
    print(f"{label:32} {entry:14} {stage:6} {p['inst']:>5} {p['threads']:>3} {p['loops']:>5} "
          f"{p['uniforms']:>4} {p['temps']:>5} {p['spills'] + ':' + p['fills']:>10} "
          f"{p['sfu']:>4} {p['nops']:>4}")
if not rows:
    print("no SHADER-DB lines found; see the raw log", file=sys.stderr)
    sys.exit(1)
print(status or "== test did not finish (timeout or crash); stats above are what compiled",
      file=sys.stderr)
