# benchmarks

Head-to-head mesh-boolean benchmark. Sibling of `kernel/`; deliberately NOT a
workspace member, so it can point at an arbitrary axiolid worktree (absolute
paths in `Cargo.toml`) and compare commits without editing that workspace.

```
cargo run --release -- [reps]     # default 5
```

Exit 1 with exactly one flagged row (`lite-kernel` at n=64, zero volume) is the
**known-good** state — a real ifc-lite failure the harness refuses to report as
a win. Any other mismatch is new.

## Why it exists

Every earlier comparison went through `IfcConvert`: STEP parsing, mapping,
serialisation, and IfcOpenShell's own triangulation all landed in the number.
This calls each kernel's Rust API directly on identical in-memory geometry, so
a difference is the kernel.

## Columns

| column | what it is |
|---|---|
| `axiolid` | `BoolmeshBoolean::subtract_many` (grouped batch path) |
| `raw boolmesh` | the same backend, sequential — isolates axiolid's provider overhead |
| `lite-kernel` | `ifc_lite_geometry::kernel::mesh_bridge::subtract_many` |
| `lite-rectfast` | `ifc_lite_geometry::rect_fast` analytic path |
| `cellular` | our analytic path (`cellular.rs`), **opt-in** |
| `manifold` | Manifold C++ kernel via `cpp/shim.cpp` |
| `cgal` | CGAL 6.0.1 corefinement PMP via `cpp/shim.cpp` |

Two kernels are deliberately ABSENT, both for evidence-backed reasons:

- **`passthrough`** is not a boolean kernel. `passthrough_shape::subtract`
  throws `"Not implemented"` and the plugin declares
  `supports_boolean_operations = false`. Its old "fastest kernel" IfcConvert
  numbers were booleans being *skipped*. Including it would be a fake row.
- **OpenCascade** could not be built here. Debian's `libocct-*-dev` 7.8.1 ships
  `Poly_ArrayOfNodes.hxx` but omits its required `NCollection_AliasedArray.hxx`
  (a packaging bug); the copy in `~/occt-research/occt` is a newer, ABI-
  incompatible API (8 compile errors when mixed). Adding it needs a full OCCT
  source build. Not faked, not estimated — simply absent.

## The analytic path (`cellular.rs`)

Axis-aligned box host minus axis-aligned box cutters, solved in closed form:
cutter faces become grid planes, each cell is wholly solid or wholly void, and
faces are emitted only where solid meets void. Watertight by construction —
adjacent cells share grid vertices by integer index, so no T-junctions.

**Opt-in by design.** It is never auto-dispatched: the caller picks fast-vs-exact
explicitly, so a run's topology is predictable. It returns `None` (declines)
when out of its competence — no cutters, none overlapping, or a cell count over
the caller's budget (the grid is `O(n^3)` in cutter count).

**Deterministic by design.** Vertex identity is a `BTreeMap` keyed by integer
grid index, not a `HashMap` — `std`'s `RandomState` seeds each map instance
differently, which is exactly the upstream defect documented below. Measured
STABLE across 20 runs where the general boolean is not.

Measured (n = openings on one wall, best-of-7):

| n | axiolid | cellular | speedup |
|---|---|---|---|
| 16 | 1.44 ms | 0.050 ms | ~29x |
| 64 | 8.09 ms | 0.318 ms | ~25x |

## Cross-kernel results (best-of-7, one wall, n openings)

| n | axiolid | cellular | manifold | cgal | lite-rectfast |
|---|---|---|---|---|---|
| 1 | 0.094 ms | 0.003 ms | 0.070 ms | 0.544 ms | 0.002 ms |
| 4 | 0.360 ms | 0.010 ms | 0.298 ms | 4.823 ms | 0.005 ms |
| 16 | 1.539 ms | 0.057 ms | 1.140 ms | 45.97 ms | 0.015 ms |
| 64 | 8.091 ms | 0.355 ms | 4.985 ms | 991.1 ms | 0.116 ms |

Manifold is consistently ~1.6x faster than axiolid on the general boolean path.
CGAL's exact-predicate corefinement is 2-3 orders of magnitude slower and scales
badly (991 ms at n=64) — correctness guarantees, not speed. The analytic paths
(`cellular`, `lite-rectfast`) are in a different complexity class entirely,
which is the whole point: the win is algorithmic, not micro-optimisation.

## Pitfalls (learned the hard way)

- **`deferred` is not a win.** `rect_fast` returns `None` when its preconditions
  fail; `mesh_bridge` returns `None` when its batch is untrustworthy. A kernel
  that declines to answer is not faster than one that answers.
- **Check volumes, always.** At n=64 `lite-kernel` returns in ~0.2ms with **zero
  volume** — it failed, it did not win. Every column is validated against
  `expected_volume()`, a *derived* ground truth, and the run exits non-zero on
  mismatch. Anchoring parity on axiolid instead would let a shared error pass.
- **The verifier is mutation-tested.** Perturbing `expected_volume` by +0.05
  must flag all five columns. Re-run that probe if you touch the fixture.
- **f32 vs f64.** ifc-lite stores positions as `f32`, axiolid as `f64`, so
  volumes agree only to ~1e-7 relative. Tolerance is set accordingly.
- **`IfcConvert` skips work if the output file exists** — reusing an output path
  across runs silently produces fake ~2ms timings.
- **`passthrough` is not a boolean kernel.** `passthrough_shape::subtract`
  throws `"Not implemented"` and the plugin declares
  `supports_boolean_operations = false`. Its old "fastest kernel" IfcConvert
  numbers were booleans being skipped, never a like-for-like comparison.

## Sphere-grid union

`k^3` grids of icospheres unioned into one solid, in two arrangements: spaced
3.0r apart (disjoint, no contact) and 1.5r apart (overlapping, neighbours fuse).
Run with `cargo run --release -- 3`; `AXIOLID_SPHERE_GRID_MAX=1000` opts into
the 512 and 1000 cases, default cap is 125.

Each case is run under two reduction strategies, which is the point of the
table — the *order* of the reduction dominates, not the per-boolean cost:

| spheres | seq | tree | speedup |
|---|---|---|---|
| 8 | 9.4 ms | 6.9 ms | 1.4x |
| 27 | 103.2 ms | 40.6 ms | 2.5x |
| 64 | 596.6 ms | 113.9 ms | 5.2x |
| 125 | 2249.5 ms | 287.3 ms | 7.8x |
| 512 | 44358.5 ms | 1536.0 ms | **28.9x** |

Sequential fold (`((a∪b)∪c)∪d`) makes step *i* pay for an accumulator holding
*i* spheres — quadratic total work. Balanced pairwise reduction makes the same
number of boolean calls on operands that stay small until the last levels. The
gap widens with n (1.4x → 28.9x), so it is a complexity difference, not a
constant factor.

Same answer either way: at every size both strategies return identical triangle
counts and component counts, with volume error ~1e-14 against the oracle. Only
the reduction order differs.

**This is a provider-level opportunity.** `subtract_many` already owns grouping
for differences; unions currently leave reduction order to the caller, and the
default a caller reaches for is the fold. 44 seconds versus 1.5 seconds for the
same 512-sphere answer is the cost of that omission.

Columns:

- `comp` — `component_count` of the result. Disjoint grids must return exactly
  the input sphere count; a mismatch prints `!! COMPONENTS want N`. Overlapping
  grids collapse to 1.
- `vol err` — disjoint only, against `concat` (no boolean at all: an
  independent oracle, not the same path asked twice). Holds at ~1e-15.
  Overlapping has no cheap closed form, so it prints `-` rather than a guess.
- `peak MB` — absolute `VmHWM` after a `/proc/self/clear_refs` reset.
  ⚠️ Read it as a ceiling, not per-case growth. Up to 125 spheres it reads a
  flat ~68 MB: the allocator arena, already grown by earlier sections of the
  harness, absorbs these unions entirely. It only starts tracking once a case
  genuinely exceeds that arena (512 spheres: 180.2 MB seq vs 166.4 MB tree).
  A delta-over-baseline column was tried first and printed 0.0 everywhere for
  the same reason. Real per-case numbers need one process per case.
- `determinism` — fingerprint over positions AND indices across `reps` runs.

## Sphere-grid blame probe

Appended after the ladder. Narrows the determinism fault to the smallest
operand that exhibits it, and **corrected the standing conclusion** — see the
correction under "Determinism probe" above.

## Determinism probe

`IfcConvert --kernel axiolid` yields different vertex counts across identical
runs (9664/9665) where `--kernel manifold` is stable. The probe (appended to
every run) reproduces this **without IfcOpenShell**, then isolates the layer:

```
n=64
axiolid subtract_many (grouped)  verts=520  tris=1292  !! NONDETERMINISTIC (20 distinct, ordering/value)
axiolid single boolean           verts=16   tris=32    !! NONDETERMINISTIC (2 distinct, ordering/value)
raw boolmesh (sequential)        verts=520  tris=1292  STABLE
raw boolmesh (FUSED tool)        verts=520  tris=1292  !! NONDETERMINISTIC  <-- upstream
cellular (analytic, opt-in)      verts=1040 tris=2332  STABLE
```

**The fault is upstream in `boolmesh`, not axiolid's grouping.** `boolmesh` uses
randomly-seeded `std::collections::HashMap` in `boolean45/` and its vertex dedup.
Counts stay fixed, so only ordering/values drift.

**Correction (sphere-grid work):** the multi-component operand is NOT the
trigger. That was the narrowest case this box-based probe could reach, so the
correlation looked causal. The sphere-grid blame probe reaches a smaller one:

```
single boolean, 2 overlapping spheres   verts=314  tris=624  !! NONDETERMINISTIC (7 distinct, ordering/value)
single boolean, 2 disjoint spheres      verts=324  tris=640  STABLE
8-sphere grid, disjoint, tree           verts=1296 tris=2560 STABLE
8-sphere grid, overlap, tree            verts=1176 tris=2368 !! NONDETERMINISTIC (20 distinct)
```

One boolean, two single-component operands, no fusion and no grouping, still
drifts. So a multi-component operand is **not required**. What the drifting
cases share is a **non-empty intersection curve**: operands that genuinely cut
each other drift, operands that merely coexist do not. Fusing disjoint cutters
was implicated only because that box fixture's fused tool was also the one that
actually intersected the subject.

The `axiolid single boolean` row above is also **stale**: it now reads
`!! NONDETERMINISTIC (2 distinct)`, not STABLE. Its cutter is a through-hole
(the opening spans y −0.15..0.35 through a wall of y 0.0..0.2), so it does have
an intersection curve, and it drifts — rarely, but it drifts.

Not yet explained: `raw boolmesh (sequential)` stays STABLE while performing
intersecting booleans. It differs from the axiolid path by conversion and
vertex dedup, so the trigger may be narrower than "any intersection curve".
Treat "intersection curve required" as established and "intersection curve
sufficient" as open.

⚠️ **Methodology warning — this conclusion inverted once.** The first probe
fingerprinted raw results over positions only, while the axiolid fingerprint
covered positions *and* indices. That made both raw paths look STABLE and
wrongly indicted axiolid's grouping. Permuted triangle order with fixed vertices
is invisible to a position-only hash. **When comparing implementations, the
fingerprints must cover identical data or the comparison is meaningless.**

⚠️ **And it narrowed once more.** The multi-component attribution above was
sound for the evidence available but wrong in general. A probe can only blame
layers it can separate: if every drifting case in your fixture shares two
properties, you cannot tell which one is the cause. Adding a fixture that
separates them (overlapping vs disjoint spheres) is what settled it.

## Findings

`rect_fast` is why ifc-lite looked ~5x faster end-to-end: for the common IFC
case it skips the 3D boolean entirely. The gap is **algorithmic, not threading**
— confirmed by the reverted rayon experiment (see `kernel/docs/architecture/
threading.md`). `cellular.rs` closes it on that case at ~25x.

Provider overhead is *not* the problem: at n≥4 axiolid's grouped `subtract_many`
beats a naive sequential loop over the same backend (8.1ms vs 49.4ms at n=64),
so the grouping optimisation is already earning its keep.

