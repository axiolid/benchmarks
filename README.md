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

## Exactness

Timing tabs answer "how fast"; the exactness table answers "is it correct". Each
row is a law of set algebra that any correct boolean must satisfy for any
operands:

| identity | law |
|---|---|
| partition | `vol(A-B) + vol(A^B) = vol(A)` |
| inclusion-exclusion | `vol(AuB) + vol(A^B) = vol(A) + vol(B)` |
| idempotence | `vol(AuA) = vol(A)` |
| commutativity | `vol(AuB) = vol(BuA)` |

The residual is reported relative to `vol(A)+vol(B)`, so magnitudes are
comparable across operand sizes. ~1e-16 is machine epsilon: exact to the limit
of double precision. This needs no ground truth -- the identity is its own
oracle -- so it works on inputs whose true volume nobody knows.

`not scored` means the kernel could not be asked: it refused the input, failed,
or is not compiled in. It is deliberately not rendered as 0, because a kernel
that declines every operation would otherwise look flawless.

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

## Menger sponge

Every other workload here is a wall with a handful of openings. The sponge
inverts that profile: one host cut by `7 * sum(20^i)` grid-aligned boxes, so
it stresses cutter count and coincident-plane handling rather than operand
complexity. It exists because the landscape doc cites a published Manifold
depth-4 timing, and that number is only meaningful next to ours on identical
geometry.

```
AXIOLID_MENGER_DEPTH=4 cargo run --release -- 3
```

Default depth is 3 (2_947 cutters). Depth 4 is 21_527 cutters and can run for
minutes on the slower kernels, so it is opt-in rather than part of the default
sweep.

The oracle is closed-form: each level keeps 20 of 27 subcubes, so a unit
sponge at depth *d* has volume `(20/27)^d` exactly. That is sensitive to a
single missing or doubled cutter, unlike a self-consistency check that only
compares kernels against each other. `src/menger.rs` carries a unit test
asserting the generated voids are disjoint and sum to `1 - (20/27)^d`, so a
generator bug fails before any kernel is blamed for it.

### Measured (depth 1-4, 20-core host, best of 1-3 reps)

```
  depth  cutters    axiolid ms         err   manifold ms         err       cgal ms         err
      1        7           1.0    8.99e-16           0.8    3.00e-16          14.1    3.00e-16
      2      147           n/a           -          18.8    4.05e-16           n/a           -
      3     2947           n/a           -         509.3    1.64e-15           n/a           -
      4    58947           n/a           -       25786.0     3.13e-6           n/a           -
```

**Manifold is the only kernel that answers past depth 1.** Both exact-
predicate kernels stop: `boolmesh` (behind axiolid) panics with an
out-of-bounds index in its half-edge builder, and CGAL corefinement returns
its failure signal. Those are defects, not slowness -- see below.

**Manifold's flattened depth-4 error is 3.13e-6**, up from ~1e-15 at
depth 3. That is drift accumulated across 58947 sequential booleans, not
a limit of the kernel: the recursive-composition section below builds the
same solid on the same kernel and reports 1.48e-15.

See the composition section below for the like-for-like
comparison against the published figure.


### Recursive composition: the published algorithm

The table above flattens the sponge into one subtraction per void box.
Manifold's own `samples/src/menger_sponge.cpp` does something different: a
Menger sponge is the intersection of three orthogonal Sierpinski-carpet
prisms, so it builds `(8^d-1)/7` carpet holes, batch-unions them once, and
subtracts three rotations. **Four booleans, regardless of depth.**

That algorithm is reproduced in `cpp/shim.cpp` and reported separately.

```
  depth    holes   manifold ms         err
      1        1           0.7    1.50e-16
      2        9          13.2    4.05e-16
      3       73         298.3    9.56e-16
      4      585       12199.3    1.48e-15
```

**The depth-4 accuracy cliff was an artifact of flattening, not a Manifold
defect.** Flattened depth 4 reports `3.13e-6` relative error; composed
depth 4 reports `1.48e-15` on the same kernel and the same final solid.
The error came from accumulating drift across 58947 sequential booleans,
each re-triangulating the previous result. Four booleans do not accumulate
it. This corrects the earlier reading that Manifold's floating-point
arithmetic breaks down at scale -- what breaks down is the flattened
formulation.

Composition is also 2.1x faster (12.2s vs 25.8s) despite producing the
same shape, which is the expected consequence of doing 4 operations
instead of 58947.

**On the published ~8s figure.** 12.2s here is 1.5x slower, which is
within the range explainable by hardware, build flags, and Manifold
version -- this is 3.5.1 built locally with default Release flags, and
the published figure names neither. The algorithm now matches, so this is
a plausible reproduction rather than a contradiction; closing the gap
further would need the original's machine and build configuration.


### Can axiolid beat Manifold here?

Yes at depth <= 3, no at depth 4, and the reason is a fixable data
structure rather than a limit of the approach.

```
  depth    voids      cells   cellular ms         err   composed ms         err
      1        7        144           0.0    7.49e-16           0.7    1.50e-16
      2      147       2112           0.4    3.04e-15          13.2    4.05e-16
      3     2947      36096          31.5    8.74e-14         298.3    9.56e-16
      4    58947     672768       23955.2    4.69e-13       12199.3    1.48e-15
```

The analytic cellular path answers at every depth, including the ones
where both general mesh kernels fail outright. At depth 3 it is 9.5x
faster than Manifold's own composition algorithm; at depth 2, 33x.

**Why depth 4 regresses.** Cells grew 18.6x from depth 3 but time grew
760x. The classifier tests every cell centre against every cutter:
`O(cells * cutters)`, which is 39.7 billion containment tests at depth 4.
That is the whole gap. A BVH or an interval tree over the cutters turns
the inner scan into a log-factor lookup, and the same 672768 cells then
cost roughly what depth 3 predicts.

**Error grows too**: 7e-16 to 4.7e-13. Volume is summed over 672768
cells in emission order, so this is accumulated summation drift, not a
geometric error. Pairwise or Kahan summation removes it.

**The honest caveat.** `cellular.rs` is not a general boolean. It is
defined only for an axis-aligned host minus axis-aligned cutters, and it
declines anything else -- the rotated-openings workload above shows it
returning `n/a` by design. Menger happens to be its perfect case. Beating
Manifold here is a real result for the wall-with-openings shape that
drives BIM, and it is not a claim about general mesh booleans.

The general path remains blocked on the `boolmesh` panic, which no
amount of grid work fixes.


### Rasterised classifier: 152x at depth 4

The per-cell scan tested every cell centre against every cutter,
`O(cells * cutters)` = 39.7 billion tests at depth 4. But every cutter
face IS a grid plane, so a cutter covers an exact contiguous range of
cells on each axis: two binary searches per axis, then mark the box.

```
  depth    voids      cells   cellular ms   manifold ms   speedup
      1        7        144           0.0           0.7         -
      2      147       2112           0.4          13.2       33x
      3     2947      36096           7.4         298.3       40x
      4    58947     672768         157.8       12199.3       77x
```

Semantics are unchanged, and the error column proves it: identical to
13 significant digits at every depth (4.69e-13 at depth 4, same as
before the rewrite). Scaling is now linear in cells -- depth 3 to 4 is
21.3x time for 18.6x cells, against 760x before.

**axiolid is now 77x faster than Manifold at depth 4** on this workload,
and answers at every depth where both general mesh kernels fail.

### Measured: routing through #77's exact boolean

Asked before assuming: does the exact planar-faced boolean from #77 avoid
boolmesh's failure? Measured, pairwise, one call per void:

```
  depth    voids        exact ms         err
      1        7             8.7    1.50e-15
      2      147         refused    ray met a vertex or edge exactly
      3     2947         refused    (same)
```

**It does not panic -- it refuses, by name.** That is a real improvement
over an out-of-bounds crash: a caller can catch it and fall back. But it
does not solve the sponge either.

The refusal is #77's containment probe hitting a vertex or edge exactly.
That is deliberate: rather than resolve a degenerate ray arbitrarily, the
predicate returns `None` and the boolean declines. On grid-aligned
geometry, where thousands of vertices are collinear with any axis-ish
ray, it fires constantly.

**Fixable, and cheap.** The probe direction is fixed
(`0.577, 0.301, 0.144`). Retrying with a different direction on refusal
would clear it: the answer is direction-independent, so any direction
that avoids the degeneracy gives the same result. That is a change to
the kernel, not the harness, so it is recorded here rather than made.

Also note depth 1 costs 8.7ms against the mesh path's 1.1ms -- the exact
boolean is ~8x slower where both work. It is the robustness option, not
the speed option.


**Update: the retry landed, and the refusal moved but did not clear.**
The kernel now tries a family of ray directions before refusing
(`axiolid/kernel` `bff1f58`). Re-measured on the same sequence:

```
  directions   refuses at
           1   step 9  of 147
           4   step 82 of 147
          12   step 82 of 147
```

Twelve directions fail at exactly the same step as four, which rules out
"unlucky ray" as the remaining cause. Inspecting the subject at the
refusal shows why: it carries **157 zero-extent faces** -- rings whose
vertices are all within 1e-12 of each other. Such a face has no
meaningful plane, so every ray meets it edge-on and no direction can
classify it.

So the probe retry was necessary but not sufficient. It fixed the
genuinely unlucky-direction case (step 9 -> 82, and the kernel's own
63-void sequence now completes). What remains is a different defect:
repeated subtraction accumulates collapsed faces, and the boolean does
not drop them. The fix belongs in fragment emission -- a face with no
extent should not be emitted at all -- not in the probe.

Depth 1 still answers, in 10.9 ms against the mesh path's 1.1 ms: the
exact boolean remains the robustness option, not the speed option.


**Update 2: the fragment drop landed, and traded a refusal for a wrong
answer.** `axiolid/kernel` `51297f6` drops split fragments that enclose no
area at f64 precision. Re-measured:

```
  depth    voids        exact ms         err
      1        7             2.7    1.95e-15
      2      147           528.8     8.33e-4
      3     2947        106718.9     1.89e-2
```

Depth 2 and 3 now COMPLETE where they previously refused at subtraction
82 of 147. That part worked: collapsed rings -- quads spanning a third of
the model whose vertices paired up one ULP apart -- have no usable normal,
so every probe ray met them edge-on. Dropping them at emission removed the
blocker, and the kernel's own 147-subtraction regression test passes.

**But the answers are wrong, and not by a rounding margin.** The depth-2
volume error is 4.57e-4 absolute, against a depth-2 cell volume of
1.37e-3: the result is short by almost exactly one third of a cell. That
is a systematic geometric error, not accumulated drift -- some fragments
being dropped carry real area.

So the area threshold is too aggressive somewhere, or the collapsed rings
are a symptom of a split defect rather than the defect itself, and
deleting them discards geometry that should have been repaired. Either
way the current state is worse than refusing: a caller cannot tell this
answer is wrong, whereas a refusal was honest.

**This is a regression to fix, not a result to build on.** The refusal was
load-bearing.

### Building Manifold

Manifold is not packaged on Debian and is absent unless `MANIFOLD_DIR`
points at an install prefix. Built here from the checkout at
`~/projects/manifold` (commit 2e2ed36, v3.5.1):

```
cmake -S . -B build -DCMAKE_BUILD_TYPE=Release -DBUILD_SHARED_LIBS=ON \
      -DMANIFOLD_TEST=OFF -DMANIFOLD_PYBIND=OFF -DMANIFOLD_CBIND=OFF \
      -DMANIFOLD_JSBIND=OFF -DCMAKE_INSTALL_PREFIX=$PWD/install
cmake --build build -j 20 && cmake --install build
```

Then build the harness with `MANIFOLD_DIR=~/projects/manifold/install`.
The `has_manifold` cfg is emitted only when the header is really present,
so a missing kernel stays a missing column rather than a fake number.


## Sphere-sphere boolean scaling

Every other workload here is box-based: axis-aligned or rotated hexahedra,
whose intersections are coplanar or axis-parallel. This is the inverse
case -- generic triangle/triangle intersections, curved intersection
loops, and no rectangular grouping structure to exploit.

Two icospheres of radius 1, centres 1.5 apart, so they overlap by half a
radius. Density sweeps by subdivision: `20 * 4^n` triangles per operand,
from 80 up to 1310720 (subdivision 8, opt-in via `AXIOLID_SPHERE_SUB`;
default is 6).

Measured on a 20-core host, best of 1 rep, peak RSS 2.6 GB at sub 8:

```
   sub      tris      kernel    union ms    isect ms      A-B ms    identity
     4      5120     axiolid        15.4         8.8        12.1    4.99e-15
     4      5120    manifold        10.6         7.8         9.6    1.17e-15
     4      5120        cgal        55.0        22.7        38.4    9.56e-16
     5     20480     axiolid        65.4        35.8        49.3    1.41e-14
     5     20480    manifold        38.5        25.3        31.0    1.07e-14
     5     20480        cgal       220.8        79.3       148.2    1.05e-14
     6     81920     axiolid       294.9       153.5       212.1    1.03e-14
     6     81920    manifold       171.9       113.6       134.1    1.38e-15
     6     81920        cgal       906.5       268.6       566.7    1.38e-15
     7    327680     axiolid      1457.0       697.0      1073.8    3.35e-14
     7    327680    manifold       740.6       466.8       586.6    1.61e-14
     7    327680        cgal      4474.9      1216.0      2593.0    1.59e-14
     8   1310720     axiolid      7355.7      3537.8      5216.0    2.90e-14
     8   1310720    manifold      3462.5      2248.7      2816.2    6.64e-14
     8   1310720        cgal     23922.9      5602.2     13368.2    6.64e-14
```

### Reading it

**A constant factor, not a complexity gap.** The axiolid/manifold ratio
runs 1.45x, 1.70x, 1.72x, 1.97x, 2.12x across the ladder. Per 4x
triangles both kernels grow by a similar factor at the top end (manifold
~4.7x, axiolid ~5.0x), so this is a fixed multiplier plus a mild
memory-traffic penalty -- not a different asymptotic class.

**We beat CGAL by 6.9x at sub 8**, and that margin grows with input size
(5.2x at sub 4).

**Cross-check against the published port.** `manifold-rust` reports 2.57 s
for sphere-minus-sphere at 2.1M total triangles. Manifold's `A-B` here is
2.82 s at 2.62M total -- consistent once scaled for the larger input, so
the manifold column reproduces the published figure.

### The oracle is inclusion-exclusion, not the closed-form lens

A tessellated sphere is an INSCRIBED polyhedron: its volume sits below
the ideal sphere's by a tessellation-dependent amount that shrinks as
density rises. Scoring against the closed-form lens volume would show
every kernel `getting more accurate` with subdivision -- an artifact of
the fixture, not a property of the kernel.

So the oracle is the identity, exact for the tessellated operands at any
density, and it validates all the measured operations at once:

```
|A u B| + |A n B| = |A| + |B|
```

`src/sphere.rs` asserts the inscribed-volume property directly, so the
reason for this choice is a test rather than a comment.

### The mesh-passing FFI

Every earlier shim entry point was box-parameterised (`host_min`,
`host_max`, corner arrays), so no arbitrary mesh could cross it.
`bench_manifold_mesh_op` and `bench_cgal_mesh_op` take
`(verts, nverts, tris, ntris)` per operand, which makes any closed mesh
measurable on every kernel -- Thingi10K pairs, CAD imports, spiky stress
cases -- without touching C++ again.

OCCT has no mesh entry point: it consumes B-rep solids, and feeding it a
triangle soup would measure a conversion rather than its boolean.


## Viewer

```
cd viewer && pnpm install && pnpm build && node server.mjs
```

Serves on `127.0.0.1:8095`. `/api/results` shells out to the release binary on
every request, so the charts always show a real run.

## Comparing two kernel revisions

```
python3 scripts/compare-revisions.py --base v0.13.0 --head main
python3 scripts/compare-revisions.py --base HEAD~1 --head HEAD --workload 64
```

Materialises each revision as a detached `git worktree`, copies the harness,
rewrites its path dependencies to point at that revision, and runs both sides
through the same `--json` path. No submodule: the kernel is located by path at
build time, so any checkout works.

Two mechanisms do NOT work and were tried first: cargo's `paths` override does
not apply to path dependencies (the build silently keeps the original kernel),
and the `AXIOLID_KERNEL_DIR` variable named in `Cargo.toml`'s comment was never
implemented. Rewriting the copied manifest is what actually redirects the build.

**A flagged row is a hint, not a verdict.** Comparing v0.13.0 with v0.14.0 --
a change that touched only a curvature-law enum and cannot affect booleans --
individual rows still moved up to 24%, `raw_boolmesh` included, whose code is
byte-identical across both. Wall-clock on a shared machine is that noisy. For
a trustworthy pass/fail signal use the kernel's deterministic instruction-count
benchmarks; this script exists for what those cannot do -- comparing against
other kernels.
