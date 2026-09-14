#!/usr/bin/env python3
"""Profile each axiolid area and classify time into cost categories.

Method: run one area per process under `perf record`, then classify each
sampled SYMBOL into a category. Classification is by symbol name, which
is a heuristic -- so anything unmatched is reported as "unclassified"
rather than folded into a bucket. A category chart that silently absorbs
unknown symbols would look authoritative while being unfalsifiable.

Output: JSON on stdout, consumed by the viewer.
"""
import json
import re
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
PROBE = ROOT / "perf-probe/target/release/perf-probe"
PERF_DIR = Path("/mnt/archive/corpus/perf")

AREAS = ["boolean", "audit", "measure", "levelset", "inspect", "heal", "genus"]

# Ordered: first match wins, so narrower patterns must precede broader
# ones. Each entry is (category, regex, why) -- the reason is carried
# into the output so a reader can judge the attribution themselves.
# Domain rules come FIRST and are grounded in what the code does, not in
# what its name suggests. Each was checked by reading the function:
#   EdgeAdjacency::build  -> BTreeMap<EdgeKey, Vec<EdgeUse>> insertion
#   emit_tetrahedron      -> two BTreeMaps for vertex welding
#   MortonCollider        -> spatial index build over morton codes
#   Hmesh::new / Manifold::new -> half-edge construction, index shuffling
# Without these the boolean area reported 62% "unclassified", which is
# honest but useless; guessing from names would have been useless AND
# misleading.
DOMAIN_RULES = [
    ("pointer_chasing", r"EdgeAdjacency::build|emit_tetrahedron|MortonCollider|find_collisions|collider",
     "BTreeMap or spatial-index traversal (verified by reading the source)"),
    ("branch_bookkeeping", r"Hmesh::new|Manifold::new|level_set|euler_characteristic|shadows0|Kernel0",
     "topology construction and index bookkeeping"),
]

RULES = [
    ("sorting", r"::sort|sort_unstable|quicksort|small_sort|merge_sort|median3|partition",
     "comparison sort machinery"),
    ("hashing", r"hashbrown|HashMap|HashSet|SipHash|::hash|rustc_hash|FxHash",
     "hash table probe or hashing"),
    # Kernel-side memory work counts as allocation: faulting in and
    # zeroing a large buffer is the real cost of asking for it, even
    # though no userspace allocator symbol appears in the trace.
    ("allocation",
     r"malloc|free|realloc|calloc|alloc::|__rust_alloc|RawVec|memmove|"
     r"memcpy|memset|page_fault|_int_free|tcache|arena|clear_page|"
     r"unmap_page|handle_mm_fault|__do_fault|zap_pte|free_pgtables|"
     r"__list_del_entry|_raw_spin_lock|sync_regs|vma_|tlb_|__folio|"
     r"release_pages|lru_add",
     "allocator, zeroing, bulk copy, page fault or kernel memory management"),
    ("pointer_chasing", r"bvh|Bvh|tree|Tree|node|Node|traverse|btree|BTree|::next|Iterator",
     "tree or node traversal"),
    ("math", r"predicate|orient|incircle|expansion|sqrt|::dot|::cross|normal|length|volume|area|moment|winding|crossing_sign|intersect|distance|f64|fma",
     "geometric or floating-point arithmetic"),
    ("branch_bookkeeping", r"audit|health|summarize|record|validate|check|verify|count|classify|diagnose",
     "structural bookkeeping and validation"),
]


def classify(symbol):
    """Return (category, reason) for a symbol, or (None, None) if unknown."""
    for cat, pattern, why in DOMAIN_RULES + RULES:
        if re.search(pattern, symbol):
            return cat, why
    return None, None


def profile_area(area, threads=0):
    """Record one area and return per-symbol overhead percentages."""
    PERF_DIR.mkdir(parents=True, exist_ok=True)
    data = PERF_DIR / f"{area}.data"
    cmd = ["perf", "record", "-q", "-F", "3000", "-o", str(data), "--",
           str(PROBE), area]
    if threads:
        cmd.append(str(threads))
    subprocess.run(cmd, capture_output=True, check=False)

    out = subprocess.run(
        ["perf", "report", "-i", str(data), "--no-children", "--stdio",
         "-F", "overhead,symbol", "--percent-limit", "0.05"],
        capture_output=True, text=True, check=False).stdout

    syms = []
    for line in out.splitlines():
        m = re.match(r"\s*([0-9.]+)%\s+\[[.k]\]\s+(.+)", line)
        if m:
            pct, sym = float(m.group(1)), m.group(2).strip()
            syms.append((pct, sym))
    return syms


def wall_ms(area, threads=0):
    """Median wall time of the area binary, in milliseconds."""
    import time
    times = []
    for _ in range(3):
        cmd = [str(PROBE), area] + ([str(threads)] if threads else [])
        t0 = time.perf_counter()
        subprocess.run(cmd, capture_output=True, check=False)
        times.append((time.perf_counter() - t0) * 1000)
    times.sort()
    return round(times[len(times) // 2], 1)


def cpu_ratio(area, threads):
    """CPU-seconds divided by wall-seconds.

    This is the honest check on a "does not scale" verdict. Wall time
    alone cannot distinguish "the code is serial" from "the harness
    never applied the thread count". If N threads were genuinely busy
    this ratio approaches N; when it sits at ~1 the work really is
    serial, whatever the thread argument said.
    """
    import resource
    import time
    before = resource.getrusage(resource.RUSAGE_CHILDREN)
    t0 = time.perf_counter()
    subprocess.run([str(PROBE), area, str(threads)],
                   capture_output=True, check=False)
    wall = time.perf_counter() - t0
    after = resource.getrusage(resource.RUSAGE_CHILDREN)
    cpu = ((after.ru_utime - before.ru_utime)
           + (after.ru_stime - before.ru_stime))
    return round(cpu / wall, 2) if wall > 0 else 0.0


def main():
    threads_sweep = [1, 2, 4, 8, 16]
    areas = []

    for area in AREAS:
        syms = profile_area(area)
        if not syms:
            continue

        cats = {}
        reasons = {}
        unclassified = []
        for pct, sym in syms:
            cat, why = classify(sym)
            if cat is None:
                unclassified.append({"symbol": sym, "pct": pct})
                cat = "unclassified"
            else:
                reasons[cat] = why
            cats[cat] = cats.get(cat, 0.0) + pct

        total = sum(cats.values()) or 1.0
        # Renormalise to the SAMPLED total, not to 100: perf drops
        # samples below the percent limit, so the raw sum is under 100
        # and pretending otherwise would overstate every share.
        categories = [
            {"name": k,
             "pct": round(v / total * 100, 2),
             "reason": reasons.get(k, "no rule matched")}
            for k, v in sorted(cats.items(), key=lambda kv: -kv[1])
        ]

        # Drill-down: the individual symbols behind each category.
        drill = {}
        for pct, sym in syms:
            cat, _ = classify(sym)
            cat = cat or "unclassified"
            drill.setdefault(cat, []).append(
                {"symbol": sym, "pct": round(pct / total * 100, 2)})
        for k in drill:
            drill[k] = sorted(drill[k], key=lambda d: -d["pct"])[:12]

        areas.append({
            "area": area,
            "wall_ms": wall_ms(area),
            "sampled_pct": round(sum(p for p, _ in syms), 1),
            "categories": categories,
            "symbols": drill,
            "unclassified": sorted(unclassified, key=lambda d: -d["pct"])[:8],
        })

    return areas, threads_sweep


def scaling(areas):
    """Thread sweep per area: does this area benefit from parallelism?"""
    out = []
    for area in [a["area"] for a in areas]:
        points = []
        base = None
        for t in [1, 2, 4, 8, 16]:
            ms = wall_ms(area, t)
            if base is None:
                base = ms
            speedup = round(base / ms, 2) if ms else 0.0
            points.append({
                "threads": t,
                "ms": ms,
                "speedup": speedup,
                "efficiency": round(speedup / t * 100, 1),
                "cpu_ratio": cpu_ratio(area, t),
            })
        peak = max(p["speedup"] for p in points)
        # A verdict, not just numbers: the point of the exercise is
        # deciding where parallelism is worth engineering effort.
        if peak >= 2.0:
            verdict = "scales"
        elif peak >= 1.3:
            verdict = "partial"
        else:
            verdict = "does not scale"
        # Carry the evidence for the verdict, not just the verdict: the
        # CPU ratio at the widest width shows whether the threads were
        # ever actually busy.
        widest = points[-1]["cpu_ratio"]
        out.append({"area": area, "points": points,
                    "peak_speedup": peak, "verdict": verdict,
                    "cpu_ratio_at_max": widest,
                    "evidence": ("threads were busy but gave no speedup"
                                 if widest > 1.5 else
                                 "work is serial: CPU time never exceeded wall time")})
    return out


if __name__ == "__main__":
    areas, _ = main()
    doc = {
        "generatedAt": __import__("datetime").datetime.now().isoformat(timespec="seconds"),
        "areas": areas,
        "scaling": scaling(areas),
        "rules": [{"category": c, "pattern": p, "reason": w}
                  for c, p, w in DOMAIN_RULES + RULES],
    }
    json.dump(doc, sys.stdout, indent=1)
