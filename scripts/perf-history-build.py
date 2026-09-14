"""Build history.json from the measured CSV.

The noise floor is measured, not assumed: repeated runs of the SAME binary
spread by ~7%, so any per-step delta below that is reported as "within
noise" rather than dressed up as a win.
"""
import csv
import json
import datetime

NOISE = 7.1
SRC = "/mnt/backup/perf-hist/history.csv"
OUT = "/home/friedrich/projects/axiolid/benchmarks/viewer/public/history.json"

REVS = [
    ("5e52dde", "Baseline", "Before any of this work."),
    ("a24f8a6", "Counting sort", "audit_mesh edge grouping: comparison sort to two-pass counting sort."),
    ("b47274d", "Hashed weld caches", "levelset, refine, decompose: BTreeMap weld caches to hash maps."),
]

rows = list(csv.DictReader(open(SRC)))
by = {}
for r in rows:
    # The CSV carries every sample, not a pre-reduced number, so the
    # median and the spread band are both derived from the same data.
    xs = sorted(float(x) * 1000 for x in r["samples"].split())
    if not xs:
        continue
    by.setdefault(r["area"], {})[r["rev"]] = {
        "ms": xs[len(xs) // 2],
        "min": xs[0],
        "max": xs[-1],
    }

areas = []
for area, revs in by.items():
    if len(revs) != len(REVS):
        continue
    first = revs[REVS[0][0]]["ms"]
    last = revs[REVS[-1][0]]["ms"]
    gain = (first - last) / first * 100 if first else 0.0
    areas.append({
        "area": area,
        "ms": {k: v["ms"] for k, v in revs.items()},
        "band": {k: [v["min"], v["max"]] for k, v in revs.items()},
        "gain": gain,
        "real": abs(gain) > NOISE,
    })

areas.sort(key=lambda a: -a["gain"])

doc = {
    "generatedAt": datetime.datetime.now().isoformat(timespec="seconds"),
    "noiseFloorPct": NOISE,
    "reps": 7,
    "revs": [{"rev": r, "label": l, "note": n} for r, l, n in REVS],
    "areas": areas,
}
json.dump(doc, open(OUT, "w"), indent=1)

print(f"{'area':11}{'base':>8}{'sort':>8}{'weld':>8}{'total':>9}  verdict")
for a in areas:
    m = a["ms"]
    verdict = "REAL" if a["real"] else "noise"
    print(f"{a['area']:11}{m['5e52dde']:7.0f}m{m['a24f8a6']:7.0f}m{m['b47274d']:7.0f}m{a['gain']:8.1f}%  {verdict}")
