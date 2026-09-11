#!/usr/bin/env python3
"""Pack a sample of the corpus into a format the Rust harness can read.

The harness must not depend on a JSON or npz reader, so this writes a
flat binary per model plus a tab-separated index. Nothing here leaves
the cache directory: the index stores paths into the cache, never
geometry, and the cache lives outside the repository.

Binary layout per model, little-endian:

    u32 n_vertices
    u32 n_facets
    f64 * 3 * n_vertices
    u32 * 3 * n_facets

Sampling is deterministic (sorted by file id, strided) so a nightly run
is comparable to the previous one rather than a fresh random draw.
"""

import argparse
import json
import os
import struct
import sys

import numpy as np


def pack_one(path, out_path):
    """Write one model as a flat binary. Returns (n_verts, n_facets)."""
    data = np.load(path)
    verts = np.asarray(data["vertices"], dtype="<f8")
    facets = np.asarray(data["facets"], dtype="<u4")
    if verts.ndim != 2 or verts.shape[1] != 3:
        return None
    if facets.ndim != 2 or facets.shape[1] != 3:
        return None
    # A facet index past the vertex array is exactly the kind of damage
    # the corpus exists to carry, but it cannot be expressed in the
    # packed form, so it is filtered here and counted as a skip.
    if facets.size and int(facets.max()) >= verts.shape[0]:
        return None
    with open(out_path, "wb") as fh:
        fh.write(struct.pack("<II", verts.shape[0], facets.shape[0]))
        fh.write(verts.tobytes(order="C"))
        fh.write(facets.tobytes(order="C"))
    return verts.shape[0], facets.shape[0]


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--manifest", default="/mnt/archive/corpus/manifest.json")
    ap.add_argument("--out", default="/mnt/archive/corpus/pack")
    ap.add_argument("--per-class", type=int, default=40)
    ap.add_argument("--max-facets", type=int, default=200000)
    args = ap.parse_args()

    with open(args.manifest, encoding="utf-8") as fh:
        manifest = json.load(fh)

    repo = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
    if os.path.abspath(args.out).startswith(repo + os.sep):
        sys.exit("refusing to pack corpus data inside the repository")

    os.makedirs(args.out, exist_ok=True)
    index_path = os.path.join(args.out, "index.tsv")
    rows = []
    skipped = 0

    for cls, entries in sorted(manifest["models"].items()):
        usable = [e for e in entries if (e.get("num_facets") or 0) <= args.max_facets]
        usable.sort(key=lambda e: int(e["file_id"]))
        if not usable:
            continue
        # Stride rather than take the head: the head of a sorted id list
        # is biased toward one era of uploads.
        step = max(1, len(usable) // args.per_class)
        chosen = usable[::step][: args.per_class]
        for entry in chosen:
            name = "{}_{}.bin".format(cls, entry["file_id"])
            dest = os.path.join(args.out, name)
            try:
                result = pack_one(entry["path"], dest)
            except Exception:
                result = None
            if result is None:
                skipped += 1
                continue
            rows.append((cls, str(entry["file_id"]), entry.get("license", ""), name))

    with open(index_path, "w", encoding="utf-8") as fh:
        for row in rows:
            fh.write("\t".join(row) + "\n")

    per_class = {}
    for cls, _, _, _ in rows:
        per_class[cls] = per_class.get(cls, 0) + 1
    for cls in sorted(per_class):
        print("  {:<18} {:>4}".format(cls, per_class[cls]))
    print()
    print("packed:", len(rows), "skipped:", skipped)
    print("index:", index_path)


if __name__ == "__main__":
    main()
