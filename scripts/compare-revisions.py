#!/usr/bin/env python3
"""Compare two kernel checkouts on the same cross-kernel benchmark.

Answers one question: did THIS kernel change make axiolid faster or slower,
measured against fixed reference kernels on identical inputs?

Deliberately no git submodule. The kernel is located by path at build time,
so any checkout works -- a worktree, a clone, a release tarball. A submodule
would pin one commit and make 'benchmark an arbitrary revision' awkward.

Both sides run in the SAME process configuration against the SAME workload,
so the comparison is apples-to-apples. What differs is only the kernel source.

WALL-CLOCK LIMIT -- read before trusting a flag.

This measures elapsed milliseconds, so it inherits every source of noise a
shared machine has: frequency scaling, other processes, cache state. Measured
on an idle 20-core Xeon, v0.13.0 vs v0.14.0 -- a change that touched only a
curvature-law enum and CANNOT affect boolean performance -- individual rows
still moved by up to 24%, including `raw_boolmesh`, whose code is identical
in both revisions.

So: a flagged row is a HINT to investigate, never proof of a regression. The
default threshold is deliberately loose. For a trustworthy pass/fail signal
use the kernel repo's instruction-count benchmarks
(`scripts/bench-regression.sh`), which are deterministic; this script exists
for the thing those cannot do -- comparing against OTHER kernels.

Read the whole table, prefer the larger n rows, and re-run before believing
any single number.

Usage:
  compare-revisions.py --base v0.13.0 --head main
  compare-revisions.py --base HEAD~1 --head HEAD --workload 64
"""

from __future__ import annotations

import argparse
import json
import re
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

BENCHMARKS = Path(__file__).resolve().parent.parent
DEFAULT_KERNEL = BENCHMARKS.parent / "kernel"

# Rows compare only kernels whose timing comes from the checkout under test.
# manifold/cgal/occt are external and identical on both sides; including them
# would report noise as a difference.
KERNEL_COLUMNS = ("axiolid", "raw_boolmesh")


def run(command: list[str], cwd: Path) -> str:
    """Run a command, raising with its stderr when it fails."""
    result = subprocess.run(command, cwd=cwd, capture_output=True, text=True)
    if result.returncode != 0:
        raise SystemExit(
            f"command failed in {cwd}: {' '.join(command)}\n{result.stderr}"
        )
    return result.stdout


def materialise(kernel: Path, revision: str, workspace: Path) -> Path:
    """Check `revision` out into its own worktree.

    A worktree rather than a clone: it shares the object store, so materialising
    a second revision costs a checkout instead of a full copy. Detached, so the
    developer's branch state is never touched.
    """
    resolved = run(["git", "rev-parse", "--short", revision], kernel).strip()
    tree = workspace / f"kernel-{resolved}"
    run(["git", "worktree", "add", "--detach", str(tree), revision], kernel)
    return tree


def retarget(harness: Path, kernel: Path) -> None:
    """Point a copy of the harness manifest at `kernel`.

    Cargo's `paths` override does not apply to path dependencies, and the
    `AXIOLID_KERNEL_DIR` variable the manifest comment mentions was never
    implemented, so neither can redirect the build. Rewriting the manifest is
    the mechanism that actually works.

    Kernel paths are repointed at the requested revision. Every OTHER relative
    path (ifc-lite, and anything added later) is made absolute against the
    original checkout, or the copied manifest would resolve it relative to the
    scratch directory and fail to build.
    """
    manifest = harness / "Cargo.toml"
    text = manifest.read_text()

    def repoint(match: re.Match) -> str:
        relative = match.group(1)
        if relative.startswith("../kernel/"):
            return f'path = "{kernel}/{relative[len("../kernel/"):]}"'
        return f'path = "{(BENCHMARKS / relative).resolve()}"'

    rewritten = re.sub(r'path\s*=\s*"(\.\.?/[^"]+)"', repoint, text)
    if "../kernel/" not in text:
        raise SystemExit(f"no kernel path dependencies found in {manifest}")
    manifest.write_text(rewritten)


def measure(kernel: Path, workspace: Path, workload: str, reps: str) -> dict:
    """Build the harness against `kernel` and return its JSON report.

    The harness is copied so two revisions never share a manifest, and so the
    developer's checkout is left untouched.
    """
    harness = workspace / f"harness-{kernel.name}"
    shutil.copytree(
        BENCHMARKS,
        harness,
        ignore=shutil.ignore_patterns("target", ".git", "viewer", "*.lock"),
    )
    retarget(harness, kernel)
    output = run(
        ["cargo", "run", "--release", "--", workload, reps, "--json"],
        cwd=harness,
    )
    # The harness prints diagnostics before the JSON line; take the last one.
    for line in reversed(output.strip().splitlines()):
        if line.startswith("{"):
            return json.loads(line)
    raise SystemExit(f"no JSON row emitted by the harness for {kernel}")


def compare(base: dict, head: dict, threshold: float) -> int:
    """Print a base-vs-head table and return a process exit code."""
    base_rows = {(r["workload"], r["n"]): r for r in base["rows"]}
    head_rows = {(r["workload"], r["n"]): r for r in head["rows"]}
    shared = sorted(set(base_rows) & set(head_rows))
    if not shared:
        raise SystemExit("no comparable rows: the two revisions ran different workloads")

    print(f"{'workload':<12} {'n':>5} {'kernel':<14} {'base ms':>10} {'head ms':>10} {'delta':>9}")
    regressed = 0
    for key in shared:
        for column in KERNEL_COLUMNS:
            # A column is a bare millisecond number, or null when the kernel
            # declined or is not compiled in. Null must stay absent, never 0.
            before = base_rows[key].get(column)
            after = head_rows[key].get(column)
            if not before or not after:
                continue
            delta = (after - before) / before
            flag = ""
            if delta > threshold:
                flag = "  REGRESSED"
                regressed += 1
            print(
                f"{key[0]:<12} {key[1]:>5} {column:<14} "
                f"{before:>10.3f} {after:>10.3f} {delta:>+8.1%}{flag}"
            )
    return 1 if regressed else 0


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--base", required=True, help="git revision to compare from")
    parser.add_argument("--head", required=True, help="git revision to compare to")
    parser.add_argument(
        "--kernel", type=Path, default=DEFAULT_KERNEL, help="kernel checkout"
    )
    parser.add_argument("--workload", default="16", help="opening count")
    parser.add_argument("--reps", default="5", help="repetitions per row")
    parser.add_argument(
        "--threshold",
        type=float,
        default=0.25,
        help="fractional slowdown that counts as a regression",
    )
    parser.add_argument(
        "--keep", action="store_true", help="keep the scratch worktrees"
    )
    arguments = parser.parse_args()

    kernel = arguments.kernel.resolve()
    if not (kernel / ".git").exists():
        raise SystemExit(f"{kernel} is not a git checkout")

    workspace = Path(tempfile.mkdtemp(prefix="axiolid-compare-"))
    try:
        print(f"base {arguments.base} -> head {arguments.head}", file=sys.stderr)
        base_tree = materialise(kernel, arguments.base, workspace)
        head_tree = materialise(kernel, arguments.head, workspace)
        base = measure(base_tree, workspace, arguments.workload, arguments.reps)
        head = measure(head_tree, workspace, arguments.workload, arguments.reps)
        return compare(base, head, arguments.threshold)
    finally:
        # Worktrees are registered with the kernel repo, so they must be
        # removed through git or they linger in `git worktree list`.
        if not arguments.keep:
            for tree in workspace.glob("*"):
                if (tree / ".git").exists():
                    subprocess.run(
                        ["git", "worktree", "remove", "--force", str(tree)],
                        cwd=kernel,
                        capture_output=True,
                    )
            shutil.rmtree(workspace, ignore_errors=True)
        else:
            print(f"kept {workspace}", file=sys.stderr)


if __name__ == "__main__":
    raise SystemExit(main())
