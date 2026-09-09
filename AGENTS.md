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

## Gyroid (whole-kernel chain)

A triply-periodic minimal surface -- `sin x cos y + sin y cos z + sin z cos x`
-- meshed by `level_set`, then audited, sectioned, and cut. The only fixture
here that is smooth-dominated: Menger and the box tests are planar and
coplanar-heavy, so this is the opposite workload.

One row exercises: level-set -> audit -> genus -> section -> boolean ->
measurement, so a regression anywhere in that chain surfaces as one failing
row instead of a silently different number.

```
  edge    tris  closed  comps  genus  contours   sect ms   bool ms  out tris     volume
   0.4    2040  true        1     12         6      0.19      5.17       864     0.9366
   0.3    4320  true        1     34         6      0.31      9.48      1812     0.9686
   0.2    8784  true        1     27         4      0.33     15.47      3138     1.0387
  0.15   16908  true        1     27         4      0.59     24.87      5550     1.0536
  0.12   24360  true        1     27         5      0.80     34.31      8238     1.0603
   0.1   33180  true        1     27         4      0.95     47.59     10842     1.0639
  0.08   51864  true        1     27         4      1.56     70.16     17388     1.0669
```

### Gated

- **closure + two-manifoldness** -- an open shell makes every later stage
  meaningless, so it is checked before anything else.
- **component count = 1** -- also what makes the genus number trustworthy
  (see below).
- **genus = 27**, but only for `edge <= 0.2`. It holds across a 6x
  refinement (8,784 to 51,864 triangles). The two coarsest rows are
  genuinely under-resolved -- 2,040 triangles cannot carry 27 handles -- so
  they are reported and excluded rather than silently passed.
- **section contour count > 0** on z=0.
- **non-empty boolean result**.

### NOT gated: volume

Deliberate. The gyroid is cut by its bounding box, so the solid depends on
where grid planes fall relative to the surface, and volume OSCILLATES with
resolution rather than converging:

```
  edge   0.40  0.30  0.20  0.15  0.12  0.10
  vol    6.03  6.21  5.01  5.42  4.86  4.50
```

That is the FIXTURE, not the mesher. The same extractor on a sphere, which
no bound cuts, converges monotonically over the same ladder (8.2e-2 to
5.0e-3 relative error). Measured both ways before deciding. Gating on
gyroid volume would fail whenever the resolution changed, for no defect.

### Genus is computed here, not taken from the kernel

`axiolid_inspect::genus` assumes ONE component: it applies `chi = 2 - 2g`
and clamps a negative result to zero (kernel issue #98), so a two-piece
solid reports genus 0 and is indistinguishable from a sphere. This fixture
uses the general `g = (2c - chi)/2`, and asserts `comps == 1` separately, so
the number stays meaningful if a future variant becomes multi-component.

### Mutation-proven

Both gates were shown to fail before being trusted:

- Perturbing the field frequency 15% (a different gyroid, still a valid
  closed surface): caught on 6 of 7 rows -- genus 27 to 56, components 1 to
  4. Volume barely moved, confirming it would have been a useless gate.
- Moving the section plane off the solid: `NO CONTOURS` on every row.

## Swiss cheese

A cube minus a `k^3` grid of curved cutters, `n` = 1, 8, 27, 64, 125.
`AXIOLID_CHEESE_MAX` caps it (default 125). Two variants, because they test
different topology and only one has a cheap exact oracle:

| | cavity | bore |
|---|---|---|
| cutters | strictly inside the host | pierce both faces |
| components | `n + 1` | 1 |
| genus | 0 | `n` |
| volume oracle | exact (host - sum of cutters) | none; reported only |
| determinism | STABLE | **NONDETERMINISTIC** |

Cutter shapes: `sphere` (icosphere), `cyl` (capped z-cylinder), `alt`
(alternating by cell parity). Bores are cylinders by definition, so the
sphere and alternating variants are skipped there rather than faked.

Measured (best of 3, `subops=1` throughout — the disjoint-fusion path):

| n | sphere | cyl | alt | out tris (sphere) |
|---|---|---|---|---|
| 1 | 0.3 ms | 0.2 ms | 0.4 ms | 332 |
| 8 | 2.4 ms | 1.0 ms | 1.9 ms | 2572 |
| 27 | 9.5 ms | 3.1 ms | 5.9 ms | 8652 |
| 64 | 22.6 ms | 7.7 ms | 15.5 ms | 20492 |
| 125 | 46.8 ms | 15.9 ms | 29.7 ms | 40012 |

Volume error holds at 1e-13..1e-15 across every cavity row.

### ⚠️ The oracle is TESSELLATED, never analytic

An icosphere INSCRIBES its sphere, so it is strictly smaller. Measured
against `4/3 pi r^3` at r=1:

| subdivisions | triangles | relative error |
|---|---|---|
| 1 | 80 | 1.3e-1 |
| 2 | 320 | 3.4e-2 |
| 3 | 1280 | 8.6e-3 |
| 4 | 5120 | 2.2e-3 |

Gating on the analytic value would charge the kernel with the FIXTURE's
discretisation error — a 3.4% 'failure' that is really the benchmark being
wrong. Summing the tessellated cutter volumes instead lands at ~1e-14,
which is the kernel's actual accuracy.

### ⚠️ Interior cavities are genus 0, not high genus

A natural assumption is that drilling holes in a cube raises its genus. It
does not, if the holes do not break the surface:

```
cube minus 8 INTERIOR spheres    chi=18   comps=9   genus=0
cube minus 9 THROUGH cylinders   chi=-16  comps=1   genus=9
```

A cavity adds a SHELL (one more component, genus unchanged); only a cutter
that pierces the boundary adds a HANDLE. That is why both variants exist —
`cavity` exercises high component count with an exact oracle, `bore`
exercises high genus without one. Neither substitutes for the other.

Both gates are mutation-proven: shortening a bore so it stops short of the
far face (`reach = extent/2 - 0.3`) makes it a cavity, and every bore row
fails with both `!! COMPONENTS want 1` and `!! GENUS want n`.

### Determinism: the intersection-curve theory, independently confirmed

Cavity rows are STABLE; bore rows are NONDETERMINISTIC (3 distinct in 3
runs). Same batch path, same cutter shape, same fusion — the only
difference is whether the cutter's surface crosses the host's. This is an
independent confirmation of the finding under "Determinism probe": the
trigger is a non-empty intersection curve, not a multi-component operand.
Note the direction — the MULTI-component result (cavity, `n+1` components)
is the stable one, and the SINGLE-component result (bore) is not.

### Found while building this

`axiolid_inspect::genus` silently returns `Ok(0)` for any multi-component
mesh: it assumes `chi = 2 - 2g` (one component) and clamps the resulting
negative genus through `u32::try_from(...).unwrap_or(0)`. Two disjoint
cubes report `Ok(0)`, indistinguishable from a sphere. Filed as
axiolid/kernel#98. This harness computes its own `euler_genus` with the
general `g = (2c - chi)/2`, which is why the cavity rows can report
genus 0 against 126 components honestly.
## Remesh invariance

Does the boolean depend on how the input was triangulated rather than on its
geometry? Seven representations of the SAME box, one difference against the
same icosphere tool.

Perturbations: flipped quad diagonals, shuffled triangle order, randomly
reindexed vertices, 1x and 2x midpoint subdivision (which also duplicates
seam vertices per-triangle), and the three composed.

### Coverage

Eleven representations: baseline, flipped quad diagonals, shuffled triangles,
reindexed vertices, uniform subdivision (1x and 2x), a combined
subdivide+shuffle+reindex, two SKINNY triangulations (edges cut at t=0.02 and
t=0.002 instead of the midpoint), UNEVEN refinement dense only near the cut,
and uneven+skinny combined.

Two reported shape metrics make "skinny" and "uneven" measured rather than
asserted: `min qual` is normalised triangle quality (1.0 equilateral, 0
degenerate) and `spread` is largest triangle area over smallest.

| representation | tris | min qual | spread |
|---|---|---|---|
| baseline | 12 | 0.866 | 1.0 |
| skinny (t=0.02) | 48 | 0.018 | 2401 |
| skinny (t=0.002) | 48 | 0.002 | 249001 |
| uneven: dense near cut | 368 | 0.495 | 4.0 |
| uneven + skinny | 1472 | 0.009 | 9604 |

### Result: invariant

All eleven agree. Volume error against the baseline is at most 1.4e-15
relative, component count is identical, and output triangle count is
identical within each refinement level.

So the P0 worry does NOT reproduce for boolean difference: the provider does
not depend on vertex order, triangle order, or diagonal choice.

### Two fixture bugs this extension exposed

Both were caught by the kernel refusing the input, not by a wrong number --
worth recording because both would have looked like kernel findings.

1. **Unwelded splits.** The split helpers emit six vertices per triangle, so
   a shared edge got one copy per side and EVERY edge was a boundary edge.
   The kernel refused it: `NotManifold("Input mesh must not contain boundary
   edges")`. `subdivide` had this latent from the start and passed only
   because nothing checked closure. Fixed with an exact bitwise `weld`; a
   tolerance-based weld was rejected as it could merge genuinely distinct
   points and change the surface.

2. **Inconsistent cut orientation.** The first `skinny_subdivide` cut edge
   `(i,j)` at `lerp(p,q,t)` from one side and `lerp(q,p,1-t)` from the other,
   intending them to cancel. They do not: on edge (0,0,0)-(2,0,0) at t=0.02
   that is 0.04 versus 1.96. Both sides must interpolate FROM the
   lower-indexed endpoint by the SAME t.

Neither is visible from volume alone, which is why the closure self-check
(`audit_mesh` on the perturbed INPUT) was added alongside the volume one.

### The self-check is load-bearing

Each row reports `in vol err` BEFORE the boolean runs: the perturbed input
must enclose the same volume as the baseline. Without it, a buggy
perturbation would look like a kernel defect. Proven by mutation -- nudging
one vertex of the flipped box reports PERTURBATION CHANGED INPUT rather than
blaming the boolean, and inverting the reindex map is refused by the kernel
as a zero-volume solid.


Two further mutations target the new perturbations specifically:

- **naive uneven refinement** (split only the selected triangles, skipping
  red-green) reintroduces T-junctions -> kernel refuses with `NotManifold`.
- **inconsistent skinny cut** (`1.0 - t` on one side) tears the surface ->
  same refusal.

Both fail loudly rather than returning a plausible wrong number, which is why
red-green refinement and the canonical cut direction are not optional detail.

### Stability probe, and what it corrects

Volume is permutation-invariant, so the table above CANNOT see the upstream
ordering drift. `stability_probe` fingerprints positions AND indices over 20
identical runs per representation. All SEVEN are STABLE, 1 distinct result --
including the t=0.002 skinny mesh and the uneven one, which are the most
likely to expose an ordering dependency if one existed.

That MATTERS for an earlier claim in this file: the sphere-grid section says
a non-empty intersection curve is what triggers the drift. These booleans all
have one, and none of them drift.

So an intersection curve is NECESSARY but NOT SUFFICIENT, and the open
question is narrowed: what the drifting cases add is curved-vs-curved
operands (icosphere against icosphere), not merely intersecting ones. A box
against an icosphere is stable. Not yet closed.

### Running it

Included in the default run. `remesh::report()` for the table,
`remesh::stability_probe(20)` for the fingerprints.

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
cases share is a **non-empty intersection curve** -- necessary, but NOT
sufficient: see "Remesh invariance", where five representations of a box cut by
an icosphere all have an intersection curve and all stay STABLE over 20 runs.
The drifting cases additionally have CURVED-VS-CURVED operands. Fusing disjoint cutters
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
sufficient" as REFUTED -- the remesh stability probe closes that half: a box
against an icosphere has an intersection curve and does not drift. The
narrowed hypothesis is curved-vs-curved operands.

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

