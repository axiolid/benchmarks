"""Per-kernel cost breakdown from one profiled comparison run.

The comparison harness exercises every built kernel in a single
process, so one `perf record` carries samples for all of them. Each
symbol is owned by exactly one kernel (CGAL::, ifc_lite_geometry::,
axiolid_/boolmesh, ...), so samples can be attributed by owner and
then broken down by cost category within that owner.

This is REAL profile data, not a model: a kernel with no samples is
reported as absent rather than as zero.
"""

import json
import re
import subprocess
from pathlib import Path

BENCH = Path("/home/friedrich/projects/axiolid/benchmarks")
DATA = Path("/tmp/kernels.data")

# Symbol owner -> kernel. First match wins, so narrower patterns lead.
OWNERS = [
    ("cgal", r"CGAL::|__gmpn_|__gmpz_|libgmp|libmpfr"),
    ("ifclite", r"ifc_lite_geometry::"),
    ("boolmesh", r"boolmesh::|raw_boolmesh"),
    ("axiolid", r"axiolid_|axiolid::"),
    ("manifold", r"manifold::|Manifold"),
    ("occt", r"BRep|TopoDS|gp_|Standard_"),
]

# Cost categories, shared with perf-areas.py so both charts speak the
# same language. Generic (not axiolid-specific): these describe what a
# geometry kernel spends time on regardless of whose code it is.
CATEGORIES = [
    ("exact arithmetic", r"gmp|mpfr|Interval_nt|interval::|orient2d|orient3d|lpi_|lambda|predicate"),
    ("allocation", r"_int_malloc|_int_free|malloc|free|operator new|operator delete|memmove|memcpy|clear_page|page_fault"),
    ("hashing", r"hash|Hash"),
    ("sorting", r"sort|quicksort|smallsort"),
    ("tree/graph traversal", r"btree|BTree|_Rb_tree|unordered|set<|map<|halfedge|Corefinement|patch"),
]


def record():
    """Profile one comparison run covering every built kernel."""
    binary = BENCH / "target/release/axiolid-benchmarks"
    subprocess.run(
        ["perf", "record", "-q", "-F", "999", "-o", str(DATA), "--",
         "taskset", "-c", "3", str(binary), "1", "--json"],
        cwd=BENCH, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
        check=False,
    )


def samples():
    """(percent, symbol) for every sample line perf reports."""
    out = subprocess.run(
        ["perf", "report", "-i", str(DATA), "--stdio", "--no-children",
         "-g", "none", "--percent-limit", "0.01"],
        capture_output=True, text=True, check=False,
    ).stdout
    rows = []
    for line in out.splitlines():
        m = re.match(r"\s+(\d+\.\d+)%\s+\S+\s+(\S+)\s+\[\.\]\s+(.*)", line)
        if m:
            rows.append((float(m.group(1)), m.group(2) + " " + m.group(3)))
    return rows


def classify(symbol, table):
    for name, pattern in table:
        if re.search(pattern, symbol):
            return name
    return None


def main():
    record()
    rows = samples()
    if not rows:
        raise SystemExit("no samples: is perf permitted here?")

    owned = {}
    for pct, symbol in rows:
        who = classify(symbol, OWNERS)
        if who is None:
            continue
        cat = classify(symbol, CATEGORIES) or "other kernel work"
        bucket = owned.setdefault(who, {"total": 0.0, "cats": {}})
        bucket["total"] += pct
        bucket["cats"][cat] = bucket["cats"].get(cat, 0.0) + pct

    kernels = []
    for name, data in sorted(owned.items(), key=lambda kv: -kv[1]["total"]):
        share = data["total"]
        cats = [
            {"name": c, "pct": round(v / share * 100, 2)}
            for c, v in sorted(data["cats"].items(), key=lambda kv: -kv[1])
        ]
        kernels.append({
            "kernel": name,
            "profile_share_pct": round(share, 2),
            # Below ~2% of samples a breakdown is shape-noise: the wall
            # clock is still sound, but the category split is not worth
            # plotting. The UI shows the total and says why.
            "breakdown_trustworthy": share >= 2.0,
            "categories": cats,
        })

    # Absolute per-kernel wall time comes from the harness, not from the
    # profile: sample share is a shape, milliseconds are a measurement.
    run = subprocess.run(
        [str(BENCH / "target/release/axiolid-benchmarks"), "1", "--json"],
        cwd=BENCH, capture_output=True, text=True, check=False,
    )
    totals, built, done, rowcount = {}, [], {}, 0
    try:
        doc = json.loads(run.stdout)
        built = doc.get("built", [])
        rows = doc.get("rows", [])
        rowcount = len(rows)
        for row in rows:
            for key, value in row.items():
                if key in ("workload", "n") or value is None:
                    continue
                totals[key] = totals.get(key, 0.0) + float(value)
                done[key] = done.get(key, 0) + 1
    except json.JSONDecodeError:
        pass

    # Harness kernel ids -> profiler owner ids, so the UI can join them.
    ALIAS = {"cgal": "cgal", "lite_kernel": "ifclite",
             "raw_boolmesh": "boolmesh", "axiolid": "axiolid",
             "manifold": "manifold", "occt": "occt"}
    for entry in kernels:
        for harness_id, owner in ALIAS.items():
            if owner == entry["kernel"] and harness_id in totals:
                entry["suite_ms"] = round(totals[harness_id], 3)
                # Rows the kernel actually completed. lite_kernel finishes
                # 2 of 12, so its small total is missing coverage, not
                # speed -- reporting it bare would invert the ranking.
                entry["rows_done"] = done.get(harness_id, 0)
                entry["rows_total"] = rowcount

    # Kernels the UI may offer but this binary cannot measure. Naming them
    # keeps a missing dependency visible instead of looking like a kernel
    # that simply lost.
    seen = {entry["kernel"] for entry in kernels}
    unavailable = [
        owner for owner in ("manifold", "occt", "cgal", "ifclite")
        if owner not in seen
    ]

    doc = {"kernels": kernels, "built": built, "unavailable": unavailable,
           "note": "Sample share is measured; a kernel absent from the "
                   "profile is reported as unavailable, never as zero."}
    print(json.dumps(doc, indent=1))


if __name__ == "__main__":
    main()
