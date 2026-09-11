#!/usr/bin/env python3
"""Fetch and partition the Thingi10K corpus for the nightly benchmark.

Never run by CI. The CI gate is the synthetic suite, which has known
ground truth; this corpus has none, so it can only assert BEHAVIOUR:
that each defect class provokes the right response instead of a
panic or a silent wrong answer.

Licence policy: the kernel is MPL-2.0. Non-commercial and
no-derivatives models are unusable here -- NC contradicts the MPL
grant, and ND forbids the repair and transformation the corpus
exists to exercise. Share-alike is excluded too: a modified fixture
would have to carry SA, which clashes with MPL file-copyleft.
Only permissive and public-domain models are admitted.

Nothing downloaded here is ever committed. The cache lives outside
the repository and the manifest records ids and licences only.
"""

import argparse
import json
import os
import sys

# Substrings that mark a licence as unusable in an MPL-2.0 project.
# Matched case-insensitively against the entry licence string.
DENY = (
    "non-commercial",
    "noncommercial",
    "no derivatives",
    "no-derivatives",
    "share alike",
    "share-alike",
    "gpl",
)

def allowed(licence):
    """True if a model may be used in an MPL-2.0 project.

    Unknown or missing licence is refused, not allowed: absent terms
    mean all rights reserved, not permission.
    """
    if not licence:
        return False
    low = licence.lower().replace("_", " ")
    if "unknown" in low:
        return False
    return not any(d in low for d in DENY)

def classify(entry):
    """Assign a model to the defect classes it exhibits.

    A model can belong to several: a mesh may be both non-manifold
    and self-intersecting. Each class names the BEHAVIOUR the kernel
    must show, which is all that can be asserted without an oracle.
    """
    classes = []
    closed = entry.get("closed")
    # Two separate manifold flags in the dataset; a model is manifold only
    # if BOTH hold. Edge-manifold alone still permits a pinched vertex.
    manifold = bool(entry.get("edge_manifold")) and bool(entry.get("vertex_manifold"))
    selfx = bool(entry.get("self_intersecting"))
    comps = entry.get("num_components", 1) or 1
    oriented = bool(entry.get("oriented"))

    if closed and manifold and oriented and not selfx:
        classes.append("clean")
    if closed is False:
        classes.append("open")
    if not manifold:
        classes.append("non_manifold")
    if selfx:
        classes.append("self_intersecting")
    if not oriented:
        classes.append("non_oriented")
    if comps > 1:
        classes.append("multi_component")
    return classes

def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--cache", default="/mnt/archive/corpus/thingi10k",
                    help="download cache, MUST be outside the repo")
    ap.add_argument("--out", default="/mnt/archive/corpus/manifest.json",
                    help="where to write the partition manifest")
    ap.add_argument("--limit", type=int, default=0,
                    help="cap models per class (0 = no cap)")
    args = ap.parse_args()

    if not os.path.isabs(args.cache):
        sys.exit("--cache must be an absolute path outside the repo")
    repo = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
    if os.path.abspath(args.cache).startswith(repo):
        sys.exit("refusing to cache corpus data inside the repository")

    import thingi10k
    thingi10k.init(cache_dir=args.cache)

    buckets = {}
    kept = skipped = 0
    licences = {}

    for entry in thingi10k.dataset():
        lic = entry.get("license")
        if not allowed(lic):
            skipped += 1
            continue
        kept += 1
        licences[lic] = licences.get(lic, 0) + 1

        for cls in classify(entry):
            bucket = buckets.setdefault(cls, [])
            if args.limit and len(bucket) >= args.limit:
                continue
            bucket.append({
                "file_id": entry.get("file_id"),
                "path": entry.get("file_path"),
                "license": lic,
                "author": entry.get("author"),
                "num_vertices": entry.get("num_vertices"),
                "num_facets": entry.get("num_facets"),
                "closed": entry.get("closed"),
            })

    manifest = {
        "source": "Thingi10K",
        "cache": args.cache,
        "policy": "MPL-2.0 compatible only: no NC, no ND, no SA, no GPL",
        "kept": kept,
        "skipped_by_licence": skipped,
        "licences": licences,
        "classes": {k: len(v) for k, v in sorted(buckets.items())},
        "models": buckets,
    }

    with open(args.out, "w", encoding="utf-8") as fh:
        json.dump(manifest, fh, indent=2, sort_keys=True)

    print("licence-admitted models:", kept)
    print("refused by licence:    ", skipped)
    print()
    for cls, items in sorted(buckets.items()):
        print("  {:<18} {:>6}".format(cls, len(items)))
    print()
    print("manifest:", args.out)


if __name__ == "__main__":
    main()
