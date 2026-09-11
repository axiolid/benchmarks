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

## Heavy tiers (measured, not extrapolated)

The default run caps the sphere ladder at subdivision 6 and the sphere grid at
125 spheres, because the heavy tiers cost minutes each. Both were run to the
top of their range; these are the actual numbers.

Use `--only=<section>` to run one section: without it the whole suite runs, so
reaching one heavy tier no longer costs an hour of unrelated work. Sections:
`wall`, `exactness`, `drift`, `sliver`, `menger`, `sphere`, `contact`,
`gyroid`, `remesh`, `scale`, `sphere_grid`, `cheese`. Comma-separated, e.g.
`--only=sphere,cheese`.

`wall` is the opening-count table at the top. It was initially left OUT of the
filter, so `--only=cheese` still ran it and exited 1 on the known ifc-lite
mismatches -- the filter appeared to work because the section it failed to
skip printed ABOVE the requested one. A section-filtered run should exit 0
when the requested sections pass; if it does not, check what else ran.

### Sphere-sphere to 1.3M triangles per operand

`AXIOLID_SPHERE_SUB=8 cargo run --release -- --only=sphere 1`

```
   sub      tris      kernel    union ms    isect ms      A-B ms
     6     81920     axiolid       207.1       134.1       159.8
     6     81920    manifold       164.8       110.9       138.3
     7    327680     axiolid       874.8       571.2       703.8
     7    327680    manifold       745.9       474.7       628.9
     8   1310720     axiolid      4246.6      2800.1      3499.1
     8   1310720    manifold      3453.6      2258.3      2867.9
     8   1310720        cgal     23860.4      5651.5     13403.9
```

This is the sphere-minus-sphere comparison the Manifold-Rust port uses, at the
same input sizes.

**Axiolid is 1.23x Manifold at the top tier, and the ratio is FLAT** -- 1.26x,
1.17x, 1.23x at subdivisions 6, 7, 8. A constant factor, not a scaling defect.

Log-log slope of axiolid union time against triangle count:

```
  sub 4->5   1.04
  sub 5->6   1.13
  sub 6->7   1.04
  sub 7->8   1.14
```

Essentially linear across a 256x range of input size. Nothing blows up at 2M
triangles. CGAL is 5.6x axiolid at the top tier.

Identity error (inclusion-exclusion on the tessellated operands) stays at
2.5e-14 at subdivision 8, so the extra triangles do not accumulate error.

### Profiling the Manifold gap (no fix shipped)

`perf record -g --call-graph=dwarf` on subdivision 7 union, bench profile
(`release` sets `strip = true`, which silently produces a symbol-free
profile -- use `--profile bench`).

Flat profile, grouped by phase:

```
mesh CONSTRUCTION (Manifold/Hmesh/collider::new)  22.3%
boolean ALGORITHM (kernels/tri/simplification)    18.1%
runtime/libc/sort/memmove/page-faults             51.8%
```

**Building the half-edge structure costs more than the boolean itself.**
There is no hotspot: the largest single entry is `Manifold::new` at 7.95%,
and it is a constructor. Micro-optimisation cannot close a 23% gap against
a profile shaped like this.

That construction share initially looked like a lead -- amortise
`Manifold::new` across `union_many` nodes. **It is not one**, and the
measurement that killed it is below under "The construction-reuse option,
sized and declined": a single boolean has no intermediates to reuse, so
that idea helps batch paths only and leaves this gap untouched. The
construction cost here is irreducible: each operand must be built once.

Both sides are genuinely single-threaded here, measured rather than assumed:
sampling `/proc/<pid>/task` during a subdivision-8 run gives **1 thread** for
the axiolid-only binary AND for the whole cross-kernel suite. boolmesh has a
`parallel` feature but it is OFF by default. So the 1.23x is a real
single-threaded difference, not a parallelism artefact.

#### One candidate tested and rejected

`__ieee754_acos_fma` at 2.14% is angle-weighted vertex normals in
`Hmesh::new`. Those normals are consumed only by
`shadows(p, q, dir) -> if p == q { dir < 0. } else { p < q }` -- the SIGN of
one component, nothing else. Positive angle weights cannot change that sign,
so uniform weights should be equivalent for the boolean.

Removing the `acos` passed all 9 boolmesh test suites. It was still reverted:

```
baseline, 4 separate processes: 780.4 814.5 796.4 787.0  (spread 34.1 ms)
probe,    3 separate processes: 766.0 781.0 785.0
apparent effect 14.4 ms; baseline noise 34.1 ms -> noise is 2.4x the effect
```

**The effect is below this harness noise floor and must not be quoted as a
1.8% win.** Deleting a correctness-relevant weighting for an unresolvable
gain is a bad trade. Reverted; nothing committed.

Process note: run-to-run spread on this box is ~4% ACROSS PROCESSES, while
`profile_sphere` best-of-N within one process understates it. Any perf claim
here needs repeated process launches, not repeated iterations.

#### The construction-reuse option, sized and declined

Construction being 22.3% of a single boolean prompted an obvious idea:
`union_many` rebuilds every intermediate, so reuse it. That was measured
rather than assumed, by instrumenting `to_manifold` against
`compute_boolean` on a disjoint sphere grid:

```
  n   wall_ms  build_ms  build%  triangles rebuilt vs input
  8       8.9       3.9   43.8%   3.00x
 27      51.2      22.2   43.4%   4.85x
 64     145.7      65.2   44.7%   6.00x
125     344.1     147.5   42.9%   6.98x
```

Construction is ~43% of `union_many` wall time, and the waste grows with n:
at 125 solids the same triangles are rebuilt seven times, because each tree
level hands the next a `TriMesh` and the half-edge structure is rebuilt from
scratch above it.

Upper bound if every triangle were built exactly once, at zero conversion
cost: **1.41x at n=8, 1.52x at 27, 1.59x at 64, 1.58x at 125.** Real would
be lower.

**It does not touch the Manifold gap.** The sphere-sphere benchmark is a
SINGLE boolean: one call, both operands are leaves, rebuild ratio 1.0x. Reuse
saves exactly zero there. An earlier note in this file suggested amortising
`Manifold::new` as the lead on the 1.23x -- that was wrong, and this
paragraph is the correction. The two are separate problems:

| | benefit | applies to |
|---|---|---|
| construction reuse | ~1.6x ceiling | `union_many` / batch paths |
| single-boolean 1.23x gap | unaffected | one `boolean()` call |

Cost of doing it: a new internal type boundary threading the built structure
between levels, heavier intermediates (the structure carries a collider and
a planar grid), and it touches the path with recorded ordering
nondeterminism -- which is currently STABLE at 1000 solids. Declined for now
on that basis, and recorded on `union_tree` in the kernel so the next reader
finds the numbers at the code rather than here.

### Sphere grid to 1000 spheres

`AXIOLID_SPHERE_GRID_MAX=1000 cargo run --release -- --only=sphere_grid 1`

```
arrange   spheres  strat  comp   union ms  out tris  peak MB vol err   determinism
disjoint       125    seq   125     2453.8     40000     51.2 6.7e-15  STABLE
disjoint       125   tree   125      271.8     40000     48.0 9.3e-15  STABLE
disjoint       512    seq   512    46260.7    163840    173.0 3.4e-14  STABLE
disjoint       512   tree   512     1558.9    163840    160.5 1.8e-14  STABLE
disjoint      1000    seq  1000   186175.6    320000    319.3 6.3e-14  STABLE
disjoint      1000   tree  1000     3484.8    320000    300.3 6.6e-14  STABLE
overlap       1000    seq     1   159048.1    276800    295.8       -  STABLE
overlap       1000   tree     1     2023.6    142336    149.8       -  STABLE
```

**The tree reduction is an ASYMPTOTIC win, not a constant factor.** Log-log
slope against sphere count:

```
       8->27   27->64   64->125   125->512   512->1000
  seq   1.93     2.04      2.15       2.08        2.08
  tree  1.39     1.36      1.20       1.24        1.20
```

Sequential is quadratic; tree is near-linear. The speedup therefore keeps
widening with n rather than settling:

```
  n        8    27    64   125    512   1000
  x      1.4x  2.7x  4.8x  9.0x  29.7x  53.4x
```

At 1000 spheres that is 3.5 seconds versus 3.1 minutes. This is what the
`union_many` tree-reduction work bought, measured at the top of the range
rather than extrapolated from 125.

Peak RSS grows sub-linearly (319 MB for 1000 spheres, 51 MB for 125) and the
tree path uses slightly LESS memory than sequential despite being 53x faster.

Every row at every size is STABLE: the boolmesh nondeterminism recorded
elsewhere in this file does not reappear at scale on this path.

### Swiss cheese at 125 cavities

`AXIOLID_CHEESE_MAX=125 cargo run --release -- --only=cheese 1`

All 15 cavity rows hold their analytic oracle to 1.4e-13 or better, with
`comps = n+1` and `genus 0` exactly as constructed, across sphere, cylinder
and alternating cutters. Bore rows give `genus n` with one component. All
STABLE.

## Contact lattice

Every qualitative relationship two solids can have, with an oracle derived
from the construction rather than read back from the kernel. `sliver.rs`
sweeps one relationship (a thinning face overlap); this covers the rest.

22 cases: disjoint, contained, partial overlap, identical, face/edge/vertex
contact, sphere tangency, an epsilon sweep at 1e-3/1e-6/1e-9/1e-12/1e-15 in
BOTH directions (gap +eps and overlap -eps), and sphere near-tangency.

### Result

The whole lattice behaves. The three degenerate contacts are the interesting
rows and all are correct:

```
  case                        union err    isect err     diff err  comps
  face contact                   0.00e0       0.00e0       0.00e0      1
  edge contact                   0.00e0       0.00e0       0.00e0      2
  vertex contact                 0.00e0       0.00e0       0.00e0      2
  sphere tangent               4.49e-15       0.00e0     2.14e-16      2
```

Zero-volume intersection on all three, and `comps` is right: face contact
fuses into ONE solid, while edge and vertex contact leave TWO. A kernel that
welded on any shared feature would report 1 everywhere.

### The epsilon sweep degrades gracefully

```
  overlap -1e-3     isect err 1.8e-14
  overlap -1e-6     isect err 1.9e-11
  overlap -1e-9     isect err 1.9e-8
  overlap -1e-12    isect err 1.7e-13
  overlap -1e-15    isect err 1.7e-16
```

Relative error grows as the slab thins, which is expected, and stays far
inside the 1e-6 gate. Nothing refuses. Both directions were swept because a
kernel that snaps near-coincident planes must not snap in only one of them.

### The oracle had to be corrected, and it mattered

The first version asserted the intersection equals the REQUESTED eps. It does
not: placing a corner at `1.0 - eps` rounds in f64, and at 1e-12 the stored
slab is 9.99978e-13, differing in the 5th significant digit. That produced a
spurious `!! ISECT 8.3e-1` that looked like a serious kernel bug.

The oracle now derives the slab from the stored coordinate (`1.0 - lo`), so
the fixture is not charging the kernel for its own placement rounding. Worth
recording: the failure looked entirely plausible, and reporting it would have
been wrong.

### Mutation-proven

Falsifying the vertex-contact oracle (claiming it fuses with 0.5 shared
volume) yields `!! UNION 3.3e-1; ISECT 1.0e0` and exit 1.

Note: the first mutation attempt silently failed to apply because `cargo fmt`
had reflowed the struct literal the edit matched on, and the run reported a
PASS. Always confirm a mutation actually landed (`grep MUTANT`) before
trusting a mutation test.

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

## Scale + translation invariance

The kernel corpus checks that `bounds()` survives a nine-order scale gap on ONE
static mesh (`scale_disparity`). This runs the SAME boolean at 7 scales x 5
world offsets and maps every result back by `(p - offset) / scale`, which is
the property a BIM caller actually depends on: a millimetre feature on a
building placed at a survey coordinate must behave like that feature at the
origin.

Reported per row: f64 spacing at the operand, input round-trip error, volume
error, vertex Hausdorff, component count, runtime, verdict.

### The headline number

`14 refused, 2 input-limited, 0 genuine kernel errors.`

The boolean is scale- and translation-invariant wherever the input survives
placement at all. It never silently returned a wrong answer that could be
blamed on the kernel.

### Attribution is the whole design

Two rows DO come back materially different — `1e-9 @ 1e6` (3.9e-2) and
`1e0 @ 1e12` (2.6e-7). Neither is a kernel defect.

Each row round-trips the OPERANDS with no boolean involved first. Placement
snaps every coordinate to the f64 grid at `offset`, so at large offsets the
kernel is handed a different solid than the caller described:

```
row            input err   output err
1e-9 @ 1e6      3.9e-2      3.1e-2
1e0  @ 1e12     7.1e-6      2.6e-7
```

Output error is SMALLER than input error in both cases — the boolean is more
accurate than the geometry it was given. A gate that skipped this check would
report two kernel bugs that do not exist.

### Why f64 predicts the cliff

Spacing at coordinate `x` is about `x * 2^-52`:

```
coordinate   spacing    steps across a 2e-9 feature
1e6          2.2e-10    9
1e12         2.2e-4     ~0
```

Nine representable steps across the whole feature is not enough to carry the
geometry, so `1e-9 @ 1e6` is quantised into a different shape before any
algorithm runs. A refusal there would be preferable to a confident answer, but
the answer it gives is faithful to the damaged input it received.

### Refusals are correct, and non-monotonic

14 of 33 rows refuse, in four distinct classes: `invalid geometry input`,
`numerically degenerate input`, and `backend boolmesh violated its contract`.
The last is the most interesting — a contract violation caught by the wrapper
rather than a clean refusal from the backend. It is diagnosed below under
"Why two scale-sweep rows report a contract vi...[truncated]

The refusal boundary is NOT monotonic: `1e-3 @ 1e3` succeeds while
`1e-6 @ 1e3` refuses, and `1e9 @ 1e12` succeeds while `1e6 @ 1e12` refuses.
Feature size relative to coordinate magnitude drives it, not either alone.

### Mutation-proven

Injecting a scale-dependent output drift (`x *= 1 + 1e-7` when `scale > 1`)
fails every affected row with the correct attribution:
`!! VOLUME 1.0e-7 vs input 1.5e-16; BOUNDS 2.0e-7`. The gate separates a real
kernel error from input quantisation rather than lumping them together.

### Hausdorff is vertex-set, not surface

`vertex_hausdorff` compares vertex SETS, not surfaces. A surface Hausdorff
needs point-to-triangle distance and is quadratic in triangles. The vertex form
is enough here because placement moves vertices rather than retriangulating.
Reported, never gated, for exactly that reason.


### These fixtures are gates, not reports

`gyroid::report`, `remesh::report`, `remesh::stability_probe` and
`scale::report` each RETURN their fault count, and `main` sums them into the
process exit code. A violated invariant fails the run rather than printing
`!!` into a log nobody reads.

Verified by mutation: changing the gyroid genus gate to expect 99 produces
`5 gyroid row(s) violated a gated invariant` / `5 invariant violation(s)` and
exit 1. On a clean tree these four contribute 0.

Note the harness also exits 1 for the pre-existing ifc-lite volume mismatches
described at the top of this file; those are a separate, known signal.

#### Why two scale-sweep rows report a contract violation

Two rows refuse with `backend boolmesh violated its contract` rather
than a plain refusal: scale 1e-6 at offset 1e9, and scale 1e-3 at
offset 1e6. Both are the extreme-quantisation regime -- about nine
representable f64 steps across the whole feature.

Reproduced directly: the placed SUBJECT is rejected before any
boolean runs, with

    subject: mesh is inside-out (signed volume -85333333333.33 < 0)

The box is not collapsed -- its extent survives placement (9.5e-7 at
scale 1e-6). The sign is wrong because the signed volume is summed as
triple products of ABSOLUTE coordinates, about the origin. At offset
1e9 each term is ~1e27 while the true 6V is ~6e-18:

    ratio term/true = 1.7e44  ->  needs ~44 significant digits
    f64 provides    = 16

The true volume is far below the rounding error of the sum, so the
sign is noise. Refusing is CORRECT -- nothing downstream could trust
a solid whose orientation cannot be determined.

Two honest caveats:

- The DIAGNOSIS is misleading. `mesh is inside-out` names a modelling
  error the caller could fix by reversing winding; the real cause is
  that the coordinates cannot represent the solid. A caller acting on
  the message as written would flip the winding and get nowhere.
- Which message you see depends on tolerance. At `Tolerance::METRE`
  (what the sweep uses) input validation passes and the failure
  surfaces later as `BackendContractViolation`; at `MILLIMETRE` it is
  caught up front as `invalid geometry input`. Same root cause, two
  different error classes -- so the error kind here is a function of
  tolerance, not of the defect.

Filed as axiolid/kernel#99.

**Update (kernel #99 fixed).** The two contract-violation rows are gone.
Orientation is now decided about the centroid rather than the origin, so a
small solid far from the origin is no longer misread as inside-out. The
sweep changes shape substantially:

```
                  before #99   after #99
  refused              14           3
  input-limited         2           9
  genuine kernel        0           2
```

The two "genuine kernel" rows are NOT a regression. Both previously
refused with `invalid geometry input` -- the false rejection #99 describes
-- and now compute:

```
  1e-6 @ 1e6   REFUSED  ->  vol err 4.9e-5 (input 2.3e-5)
  1e0  @ 1e9   REFUSED  ->  vol err 4.6e-9 (input 6.6e-10)
```

At ~9 representable steps across the feature, an output error a few times
the input damage is the precision limit showing through, not a defect. The
fixture reports them as kernel error because output exceeds input error,
which is the correct rule -- but the honest reading is that these rows sit
at the edge of what f64 can represent.
 The suggested fix is to compute the signed
volume about the centroid rather than the origin, which is translation-
invariant and removes the cancellation.


## Exactness scoring: metrics beyond volume

Four identities exist in ops.rs and every law is literally vol(...):
partition, inclusion-exclusion, idempotence, commutativity. The
scoring hook returns Option<f64> -- a volume -- so the harness
cannot express a non-volume check at all.

The gap is real and provable without a kernel. Let A be the unit cube
and X the same cube translated to x=100. Both have volume 1, so a
kernel returning X for A u A scores residual 0 and PASSES. The same
hole exists for A ^ A = A and commutativity.

Which metrics discriminate, for that counterexample:

| metric | catches A vs X |
|---|---|
| bounds | yes |
| Hausdorff distance | yes |
| point containment | yes |
| surface area | no |
| Euler / genus | no |
| component count | no |

Bounds, Hausdorff and containment are POSITIONAL; area, genus and
component count are not. A scoring upgrade that adds only the
topological metrics would still pass the counterexample above, so
at least one positional metric is required.

The scoring hook now returns a metric bundle, not a bare volume.
Metrics carries volume plus optional area, bounds, Euler
characteristic, component count and a closed-manifold flag.
Optional because the C ABI kernels return a bare double: a metric
one side cannot report is SKIPPED, never scored as a passing zero.

Two classes of law, and the distinction is load-bearing:

- ADDITIVE (partition, inclusion-exclusion): terms sum, so only
  volume is scored. Surface area is NOT additive across a cut --
  each piece gains a face the original never had -- so scoring area
  on a partition law would report a large residual for a kernel
  that did everything right.
- EQUIVALENCE (idempotence, commutativity): both sides denote the
  SAME solid, so every metric both sides report must agree.

Verified by mutation, not by inspection. Translating the A-u-A
result 100 units in x leaves volume, area, Euler and component
count identical -- all four are translation-invariant. Before the
change the identity scored a perfect 1.37e-16. After it:

```
idempotence      2e1 bounds      1.37e-16      1.37e-16
                 ^ axiolid       ^ volume-only kernels, unmoved
```

The volume-only columns stay clean under the same mutation, which
is the proof that the OLD scoring could not have caught it.

Bounds is currently the only positional metric, so it is the one
that makes the suite able to detect a right-shaped answer in the
wrong place. Do not remove it in favour of topology alone.


Missing identities: A-A=0, (A-B)^B=0, (A-B)u(A^B)=A, associativity
of union and of intersection, and both absorption laws. Only
idempotence and commutativity of the listed set are covered.

drift.rs is volume-only too: it maps axiolid_volume over the chain
and never audits topology, so a chain that accumulates
self-intersections while preserving volume scores clean. That is
precisely the CGAL failure mode -- exact predicates keep the
combinatorial decisions right while inexact CONSTRUCTIONS place
intersection points badly, which shows up as topology damage over
consecutive operations rather than as a volume error.

## Rotation invariance

`rotate.rs`. A rigid motion cannot change a solid, so rotating both operands,
running the boolean, and mapping the result back through the inverse rotation
must reproduce the unrotated answer.

Angles: 0, 30, 45, 89.999999, 90, 90.000001 degrees about z, plus four fixed
arbitrary 3D axes (body diagonal at 30 and 45, and two skew axes). The
89.999999 / 90 / 90.000001 trio is the point of the sweep: exactly 90 is
representable and restores axis alignment, a hair either side does not, so a
kernel that special-cases axis-aligned input would show a discontinuity.

Measured invariants: volume, area, Euler characteristic, component count,
vertex spread, and max vertex drift from the unrotated result.

```
  case                 vol err    area err     chi   comps       drift  verdict
  z            0        0.00e0      0.00e0       2       1      0.00e0  ok
  z           30        0.00e0      0.00e0       2       1    2.22e-16  ok
  z           45        0.00e0      0.00e0       2       1    1.57e-16  ok
  z    89.999999      1.69e-16      0.00e0       2       1    2.22e-16  ok
  z           90        0.00e0      0.00e0       2       1    1.11e-16  ok
  z    90.000001      1.69e-16      0.00e0       2       1    2.48e-16  ok
  diag 30             3.38e-16      0.00e0       2       1    2.48e-16  ok
  diag 45               0.00e0      0.00e0       2       1    1.57e-16  ok
  tilt 60               0.00e0    1.48e-16       2       1    4.00e-16  ok
  skew 120            1.69e-16      0.00e0       2       1    2.42e-16  ok
```

Every case holds to machine epsilon, and there is NO discontinuity across the
90-degree trio.

### The trap this fixture nearly fell into

The first version compared volume, area, chi, components and spread. It
reported "invariant" on all ten cases -- and it also reported "invariant"
when `unrotate` was mutated into a no-op, which means the round trip was
never actually being verified.

Every one of those metrics is rotation-invariant BY CONSTRUCTION. Comparing
them after a round trip cannot distinguish "correctly un-rotated" from "never
un-rotated". The fixture needed one metric that is NOT rotation-invariant:
`max_vertex_drift`, comparing actual positions against the unrotated
reference. With it, the same mutation is caught on 9 of 10 cases (0 degrees
correctly still passes -- there is no rotation to invert) and the run exits 1.

Generalisation worth keeping: **an invariance fixture must include at least
one quantity that the transform does not preserve**, or it only proves the
transform is a transform.

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


## Analytical geometry coverage (item 15 audit)

The benchmark suite is entirely mesh-boolean: every fixture in `src/` runs
`MeshBoolean`. No fixture exercises analytical/parametric geometry, even
though the kernel has NURBS, sectioning, offset and tessellation crates.

Audited against the 15 canonical cases. Kernel implementation vs BENCHMARK
coverage are very different things -- most of these exist in the kernel with
unit tests, and none are in the benchmark suite:

| case | kernel code | kernel tests | benchmark |
|------|-------------|--------------|-----------|
| Steinmetz / two perpendicular cylinders | none found | none | none |
| Sphere-sphere lens | none found | none | none |
| Tangent sphere-sphere | via mesh boolean | `contact.rs` lattice | `contact` |
| Cylinder-plane | `compile/src/brep.rs` | `exact_extrusion.rs` | none |
| Cone-plane conics | `revolve_exact.rs`, `measure/src/exact.rs` | partial | none |
| Sphere-plane | `levelset`, `evaluate/tests/surface.rs` | yes | none |
| Torus-plane | `analytic_directrix.rs`, `brep_tessellation.rs` | yes | none |
| Cylinder-cylinder near tangent | none found | none | none |
| NURBS circle (rational) | `nurbs` | `surface_curve_sweep.rs` | none |
| NURBS knot insertion | `nurbs` | `surface_insertion.rs`, `knot_removal.rs` | none |
| NURBS degree elevation | `nurbs` | `degree_elevation.rs`, `degree_reduction.rs` | none |
| Curve evaluate -> invert | `evaluate` | `evaluate/tests/invert.rs` | none |
| Surface tessellation sweep | `tessellate` contract | `tessellate/tests/output.rs` | none |
| Offset star polygon | `construct/src/offset.rs` | `overlay/tests/offset.rs` | none |
| High-genus section | `mesh-section` | thin -- see below | `menger`, `gyroid` volume only |

The three with NO kernel implementation found (Steinmetz, sphere-sphere
lens, cylinder-cylinder near tangent) are the genuinely missing capability;
the rest are an unexercised-in-benchmarks gap, not an absent one.

### The section conformance contract was the weakest link

`mesh-section`'s `ConformanceSuite` is what every section provider must
satisfy to be registered via `register_conformant`. It sectioned a UNIT
CUBE through the middle and asserted only:

```rust
if a.contours.is_empty() { /* fail */ }
```

Non-emptiness, evidence, determinism. Nothing about the returned geometry.
A provider returning one contour of the wrong size, wrong shape, or wrong
count passed the contract -- and this is the gate on the registration path,
not an optional benchmark.

Now checked against an analytic oracle on a sphere, including the
degenerate planes:

| case | plane | expected |
|------|-------|----------|
| `sphere central` | z=0 | 1 contour, area pi |
| `sphere h=0.5` | z=0.5 | 1 contour, area pi*(r^2-h^2) |
| `sphere above pole` | z=1.5 | 0 contours |
| `sphere tangent pole` | z=1.0 | 0 contours, zero area |

The tangent case is also the VERTEX-HIT case: an icosphere has a vertex at
the pole, so the plane touches exactly one vertex.

The oracle is `pi*(r^2 - h^2)` with a tolerance sized to the TESSELLATION,
not to the provider: an icosphere inscribes its sphere, so the measured
polygon is legitimately a little smaller than the true circle. Charging the
provider for the caller's subdivision choice would be wrong -- the same
principle as the swiss-cheese oracle using tessellated cutter volume.

Two different heights are checked deliberately: a provider returning a
constant area cannot satisfy both.

Mutation-verified against the real `ScalarSection` provider:

- inflating output coordinates by 5% -> `WrongSectionArea` on both heights
- emitting a duplicate contour -> `WrongContourCount` on both heights

Both mutations were caught and the test exits non-zero; reverting restores
green.

### Still open

Contour COUNT on a high-genus section (Menger/gyroid plane cut) is the
obvious next step -- `menger` and `gyroid` currently check volume only, and
a plane through a Menger sponge has a known contour count that would
exercise multi-contour handling far harder than a sphere does.
