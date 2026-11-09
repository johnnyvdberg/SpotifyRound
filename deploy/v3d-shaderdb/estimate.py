"""Estimate what each frame costs on the Pi 5's GPU, without a Pi.

    estimate.py counts.tsv [more.tsv ...] [--table baseline.txt]
                [--ms-per-inst 0.025] [--fixed 0.5] [--top 5]

Joins two things measured exactly elsewhere:

- each draw's pixels, from the `pi_estimate` tests (fragment counts on the
  machine at hand: `covered` is every pixel the draw rasterises, `kept` those
  that survive its discards);
- each shader's cost on the Pi, from the shader-db table (`shaderdb.sh`): its
  instructions per pixel and the threads it runs with, from the Pi's own
  compiler.

The Pi 5's V3D runs about 46 billion shader instructions a second, which
measured on the Pi (`gpu_cost`) comes to `--ms-per-inst` milliseconds for one
instruction over every pixel of a 1080 x 1080 frame. A shader at two threads
hides less texture latency and costs about twice as much per pixel; at one
thread, four times. `--fixed` adds a per-frame overhead for storing the frame
(tile writes to memory) and the submission.

On the V3D a discard does not end the shader: the pixel's output is masked
and the rest runs anyway (Mesa's nir_to_vir.c, nir_intrinsic_terminate). So
the estimate is `covered` pixels x the whole shader. Work wrapped in an `if`
is skipped for groups of 16 pixels that all leave it, so for a shader written
that way the cost falls toward `kept` pixels; the floor column shows that
best case.

The numbers are only as good as `--ms-per-inst`: compare one moment against a
real Pi run (`frame_time`, or the app's own frame stats) and scale it.
"""
import argparse
import collections
import sys

THREAD_FACTOR = {4: 1.0, 2: 2.0, 1: 4.0}
# Entry points that branch instead of discarding (`if (visible) { ... }`):
# their work after the branch runs only for 16-pixel groups with something to
# draw. Their cost: the prefix (the ray's hit and map lookups, about
# `--prefix` instructions) for every pixel reached, the rest for the pixels
# kept, rounded up to whole groups by `--groups`.
BRANCHED = {"fs_star", "fs_flat", "fs_ground", "fs_detail", "fs_clouds",
            "fs_cloud_layer", "fs_glow", "fs_moon", "fs_limb", "fs_ring",
            "fs_eclipse", "fs_ring_shadow"}
FULL = 1080 * 1080


def load_table(path):
    """Fragment shaders by pipeline label (the first pipeline of each name)."""
    shaders = {}
    for line in open(path):
        parts = line.split()
        if "FS" not in parts:
            continue
        i = parts.index("FS")
        name = " ".join(parts[: i - 1])
        label, _, number = name.rpartition(" #")
        if not label:
            label = name
        if label in shaders and number != "0":
            continue
        shaders[label] = (parts[i - 1], int(parts[i + 1]), int(parts[i + 2]))
    return shaders


def load_counts(paths):
    """Rows of (scene, moment, frame, label, covered, kept)."""
    rows = []
    for path in paths:
        for n, line in enumerate(open(path)):
            if n == 0:
                continue
            scene, moment, frame, label, covered, kept = line.rstrip("\n").split("\t")
            rows.append((scene, moment, int(frame), label, int(covered), int(kept)))
    return rows


def main():
    parser = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    parser.add_argument("counts", nargs="+")
    parser.add_argument("--table", default="deploy/v3d-shaderdb/baseline.txt")
    parser.add_argument("--ms-per-inst", type=float, default=0.025)
    parser.add_argument("--fixed", type=float, default=0.5)
    parser.add_argument("--top", type=int, default=5)
    parser.add_argument("--prefix", type=int, default=140)
    parser.add_argument("--groups", type=float, default=1.25)
    parser.add_argument("--no-branches", action="store_true",
                        help="cost every shader as if it discarded (the code before branching)")
    args = parser.parse_args()

    shaders = load_table(args.table)
    per_pixel = args.ms_per_inst / FULL
    missing = set()

    def cost(label, pixels, kept=None):
        shader = shaders.get(label)
        if shader is None:
            missing.add(label)
            return 0.0
        entry, inst, threads = shader
        factor = THREAD_FACTOR.get(threads, 4.0) * per_pixel
        if kept is not None and entry in BRANCHED and not args.no_branches:
            prefix = min(inst, args.prefix)
            busy = min(pixels, kept * args.groups)
            return (pixels * prefix + busy * (inst - prefix)) * factor
        return pixels * inst * factor

    frames = collections.OrderedDict()
    for scene, moment, frame, label, covered, kept in load_counts(args.counts):
        frames.setdefault((scene, moment, frame), []).append((label, covered, kept))

    # Frames while a planet's maps stream in: what the painting adds a frame.
    streaming = collections.defaultdict(list)
    print(f"{'moment':48} {'est ms':>7} {'floor':>6} {'fps':>5}   heaviest draws (ms)")
    for (scene, moment, frame), draws in frames.items():
        total = args.fixed + sum(cost(l, c, k) for l, c, k in draws)
        floor = args.fixed + sum(cost(l, k) for l, _, k in draws)
        if moment.endswith("streaming"):
            painting = sum(cost(l, c) for l, c, _ in draws if not is_frame(l))
            streaming[(scene, moment)].append(painting)
            continue
        by_label = collections.defaultdict(float)
        for l, c, k in draws:
            by_label[short(l)] += cost(l, c, k)
        heavy = sorted(by_label.items(), key=lambda x: -x[1])[: args.top]
        heavy = ", ".join(f"{name} {ms:.1f}" for name, ms in heavy)
        print(f"{scene + ' ' + moment:48.48} {total:7.1f} {floor:6.1f} {1000 / total:5.0f}   {heavy}")
    for (scene, moment), costs in streaming.items():
        costs.sort()
        print(
            f"{scene} {moment}: painting maps adds {sum(costs) / len(costs):.2f} ms a frame on average, "
            f"{costs[len(costs) // 2]:.2f} median, {costs[-1]:.2f} at worst, over {len(costs)} frames"
        )
    drawn = {l for draws in frames.values() for l, _, _ in draws}
    two = sorted({l for l in drawn if l in shaders and shaders[l][2] < 4})
    if two:
        print("\nshaders below four threads:", ", ".join(two))
    if missing:
        print("\nno shader-db row for:", ", ".join(sorted(missing)), file=sys.stderr)
        print("(rerun shaderdb.sh so the table has every pipeline)", file=sys.stderr)


GENERATOR = {"probe", "height", "erosion", "craters0", "craters1", "craters2", "sky",
             "climate", "relief", "towns", "clouds0", "clouds1", "glow", "mip chain"}
GENERATOR |= {f"day{k}" for k in range(13)} | {f"lights{k}" for k in range(5)}


def is_frame(label):
    """Whether a draw belongs to the frame rather than map painting (the
    streamer's pipelines are named without a space before their number)."""
    return label.removeprefix("Solar ") not in GENERATOR


def short(label):
    return label.removeprefix("Solar ").removeprefix("Planet ")


if __name__ == "__main__":
    main()
