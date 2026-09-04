# Axiolid kernel benchmarks

Cross-kernel comparison of mesh boolean performance on the workload that
dominates IFC: subtracting rectangular openings from a wall.

Three geometry cases are measured, because kernels rank differently on each:

| Workload | Geometry | Why it is here |
|---|---|---|
| `offset` | Cut planes strictly inside the wall | The easy, well-conditioned case |
| `flush` | Cut planes coincident with the wall's faces | The degenerate case real IFC hits constantly (a door at floor level). Kernels disagree most here |
| `rotated` | Openings rotated 30 deg in plan | No operand is axis-aligned, so every analytic fast path must decline and the general solver's real cost is visible |

A kernel that declines is reported as absent, never as a zero. A kernel whose
result volume disagrees with the derived ground truth is reported as `WRONG`
and emits no timing -- a fast wrong answer must not read as a win.

Every number here is measured on the machine that runs it. Nothing is
committed, cached, or hand-written into the UI.

## What is compared

| Column | What it is |
|---|---|
| `axiolid` | Axiolid's `MeshBoolean` provider (`subtract_many`, with cutter grouping) |
| `raw_boolmesh` | The upstream `boolmesh` crate called directly, no grouping |
| `lite_kernel` | ifc-lite's general mesh boolean |
| `lite_rectfast` | ifc-lite's axis-aligned fast path |
| `cellular` | Axiolid's opt-in analytic box path |
| `manifold` | Manifold (C++), via a C ABI shim |
| `cgal` | CGAL polygon-mesh processing (C++), exact predicates |
| `occt` | OpenCascade `BRepAlgoAPI_Cut` (C++), B-rep |

A kernel that is not installed does not appear. Columns are gated at compile
time (`HAS_MANIFOLD`, `HAS_CGAL`, `HAS_OCCT`), so a missing kernel produces no
column rather than a zero or a fabricated number.

## Correctness first

Every kernel's result volume is checked against a derived ground truth before
its timing is reported. A row flagged `!!` disagreed; the run prints the
mismatch count on stderr. A fast wrong answer is not a win.

## Running

```
cargo run --release -- 64          # table for n=64 openings
cargo run --release -- 64 --json   # machine-readable
```

The C++ kernels are optional. Point the build at them if you have them:

```
MANIFOLD_DIR=/path/to/manifold-install OCCT_DIR=/path/to/occt-install cargo build --release
```

## Viewer

```
cd viewer && pnpm install && pnpm build && node server.mjs
```

Serves on `127.0.0.1:8095`. `/api/results` shells out to the release binary on
every request, so the charts always show a real run.
