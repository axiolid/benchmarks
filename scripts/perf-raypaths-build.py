"""Ray-path comparison for the benchmark page.

Three ways to cast the same 2000 rays at one mesh, at ONE kernel
revision. Not a history: a choice the caller makes. Median of 7,
pinned core.
"""

import csv
import json
import statistics as st

NOISE = 7.1
SRC = "/mnt/backup/perf-hist/rayvariants.csv"
OUT = "/home/friedrich/projects/axiolid/benchmarks/viewer/public/raypaths.json"

VARIANTS = [
    (
        "raymesh",
        "Full scan",
        "nearest_hit tests every triangle. No index, no setup, no",
        "bookkeeping.",
    ),
    (
        "facaderay",
        "Automatic cache",
        "Application::nearest_mesh_hit. Keyed by content hash, so it",
        "costs one hash per cast and cannot serve stale geometry.",
    ),
    (
        "handleray",
        "Caller-held index",
        "MeshRayIndex::build once, cast many. The borrow checker",
        "pins the mesh, so no key is needed at all.",
    ),
]

samples = {}
for row in csv.DictReader(open(SRC)):
    xs = sorted(float(x) for x in row["samples"].split())
    samples[row["variant"]] = xs

base = st.median(samples["raymesh"])
rows = []
for key, label, *desc in VARIANTS:
    xs = samples[key]
    ms = st.median(xs)
    rows.append(
        {
            "variant": label,
            "note": " ".join(desc),
            "ms": round(ms, 1),
            "band": [round(xs[0], 1), round(xs[-1], 1)],
            "speedup": round(base / ms, 1),
            "real": abs(base - ms) / base * 100 > NOISE,
        }
    )

doc = {
    "noiseFloorPct": NOISE,
    "reps": 7,
    "rays": 2000,
    "triangles": 81920,
    "crossoverRays": 22,
    "variants": rows,
}
json.dump(doc, open(OUT, "w"), indent=2)
for r in rows:
    print(f"{r['variant']:20} {r['ms']:7.0f} ms  {r['speedup']:5.1f}x")
