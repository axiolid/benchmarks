# Where SIMD helps in Axiolid, and where it does not

Measured on a Xeon w7-3565X (20 cores, AVX-512F/BW/DQ/VL/IFMA/VBMI) by
compiling the same source twice -- `-C target-cpu=x86-64` versus
`-C target-cpu=native` -- and timing identical workloads.

## The headline: there is no hand-written SIMD to evaluate

`crates/execution/cpu/src/features.rs` is the only file in the kernel
that touches `core::arch::x86_64`, and it performs detection only:

```
$ grep -rln "CpuExecution|InstructionPolicy|CpuInstructionSet" crates/ \
    --include=*.rs | grep -v execution/cpu
(no results)
```

No compute path consults the instruction policy. `compiled_feature!
(features, "simd")` reports a build flag, not a code path. So every
result below is about AUTO-vectorisation: what LLVM finds unaided.

This is the mechanical reason SIMD "seemed not helpful". There is no
SIMD dispatch to help.

## Result: enabling AVX-512 changes almost nothing

Every workload measured came in flat -- within run-to-run noise -- with
native no faster than baseline:

| workload | shape | verdict |
| --- | --- | --- |
| raw volume loop | float reduction over indexed tris | flat |
| flat array loop | same maths, contiguous | flat |
| elementwise `v*v+1` | textbook vectorisable | flat |
| `volume_properties` | measure entry point | flat |
| `surface_properties` | measure entry point | flat |
| `second_moments` | measure entry point | flat |
| `audit_mesh` | hash/branch bound | flat |
| `winding_number` | scalar accumulation | flat |
| `self_intersections` | BVH pointer chasing | flat |
| `boolean_union` | allocation bound | flat |

The flag did change the emitted code, so this is a real null result and
not a broken experiment:

```
avx512 instructions in baseline: 0
avx512 instructions in native  : 201
```

Both binaries already vectorise to a similar degree; native simply uses
VEX/EVEX encodings for work the baseline does with legacy SSE:

```
baseline : addpd 142  addsd 233  mulpd 321   (legacy SSE)
native   : vaddpd 150 vaddsd 312             (VEX/EVEX)
```

Wider registers do not help when the bottleneck is elsewhere.

## The real finding: associativity, not instruction set

`vfmadd` count is ZERO in both builds despite FMA being available, and
the reduction loops emit scalar `vaddsd` rather than packed `vaddpd`.

The cause is that f64 addition is not associative, so LLVM may not
legally re-associate a single accumulator chain into lanes. Granting
that re-association by hand -- four independent accumulators over the
same data -- is worth about 2x, in BOTH builds:

| case | baseline | native |
| --- | --- | --- |
| `elementwise_fma` (1 accumulator) | 510587 ns | 582933 ns |
| `elementwise_4acc` (4 accumulators) | 259603 ns | 260234 ns |

A ~2x gap that appears identically with and without AVX-512 is not an
instruction-set effect. The serial dependency chain on the accumulator
is the limit; the vector units were never the constraint.

This is the actionable result: reduction loops in the measure crate are
written as single-accumulator sums, and splitting those chains is worth
more than any `target-cpu` flag.

## Method note, and a correction to an earlier run

The first pass timed `volume_properties`, `surface_properties` and
`second_moments` as "pure float reductions". They are not: each calls
`audit_mesh` first, so all three landed within noise of `audit_mesh`
itself -- around 8.5 ms each, which was the clue. `raw_volume_loop`
isolates the arithmetic and costs ~168 us, roughly fifty times less.
Timing a public entry point measures its preconditions too.

Run-to-run noise was +-10% pinned to one core and worse unpinned, so
small effects here are not resolvable by wall clock. The 2x accumulator
result is well outside that band; the `target-cpu` differences are not,
which is why they are reported as flat rather than as small wins.

## What would actually pay off

1. Split accumulator chains in the measure reductions. Machine-
   independent, ~2x on the arithmetic, no unsafe code, no intrinsics.
2. Attack allocation on the boolean path before considering SIMD there:
   it is the slowest case by an order of magnitude (~50 ms) and already
   measured spending ~11% of instructions in allocation.
3. Do NOT add `target-cpu=native` to the build expecting a win. It
   changes encodings, not bottlenecks, and it costs portability.
4. If hand-written SIMD is ever wanted, `InstructionPolicy` must first
   be wired through a provider -- today nothing reads it, which is also
   why the benchmark scorecard refuses to gate SIMD equivalence.

## Correction: the accumulator split is the wrong target

The SIMD probe found a 2x win from splitting a single f64 accumulator
into four, and the obvious next step looked like applying that to the
`measure` reductions. Profiling first says no.

Breakdown of one `volume_properties` call on 81920 triangles:

| part | time | share |
| --- | --- | --- |
| `audit_mesh` | 9645779 ns | 93.8% |
| the float reduction | 156910 ns | 1.5% |
| entry point total | 10282368 ns | 100% |

Halving the reduction would make the entry point 0.76% faster, in
exchange for changing float summation order in a function that roughly
ten downstream test suites assert against. Not worth it. The 2x is real
but it applies to 1.5% of the work.

## Where the time actually is

`perf record --call-graph=dwarf` over the probe:

```
34.35%  core::slice::sort::unstable::quicksort::quicksort
22.44%  core::slice::sort::shared::smallsort::small_sort_general
10.73%  axiolid_mesh::audit::audit_mesh
 4.74%  axiolid_measure::mesh::surface_properties
```

57% of total runtime is sorting, and the caller-attributed tree puts
~17% of each `audit_mesh` call in `sort_unstable_by_key`:

```
--21.39%--axiolid_mesh::audit::audit_mesh
   --17.27%--VecEdgeSink::summarize
      --16.69%--core::slice::sort_unstable_by_key
```

`VecEdgeSink::summarize` sorts `3 * triangle_count` edge records to
group them by undirected key. The sort is a comparison sort over a
`(u64, u64)` pair of VERTEX INDICES, which are bounded by
`positions.len()`. A comparison sort cannot exploit that bound; a radix
sort can.

Note the existing code is already the good version -- `audit_mesh` uses
the sort-based `VecEdgeSink` and falls back to the `BTreeMap` sink only
when the exact allocation fails. The remaining cost is the sort itself,
not a data-structure mistake.

## Measured prototype

LSD radix, two stable counting passes over `high` then `low`, on 245760
edge records:

| sort | median |
| --- | --- |
| `sort_unstable_by_key` | 7690829 ns |
| two-pass radix | 5071692 ns |

Roughly 34% off the sort, reproducible across three runs, with the
resulting key order asserted identical to the comparison sort's. Since
the sort is ~17% of the audit and the audit is ~94% of the measure
entry points, that is worth several times more than the accumulator
change it replaces.

Prototype lives in `simd-probe`; it has NOT been applied to the kernel.
Doing so means touching `MeshHealth`'s edge pipeline, which every
`is_closed_two_manifold` caller depends on, so it wants its own change
with the full gate behind it.

## Outcome: the counting sort landed

Kernel commits e077c92 and a24f8a6 replaced the comparison sort in
the mesh audit with a two-pass counting sort on vertex ids.

Controlled A/B, forced rebuild per arm, same core, median of five:

| area | before | after | gain |
|---|---|---|---|
| audit | 582 ms | 449 ms | 23% |
| measure | 1204 ms | 968 ms | 20% |

A first measurement claimed 37% and 41%. It was wrong: the two arms
straddled an incremental rebuild, so one of them ran a stale binary.
The lesson is the same one this repo keeps relearning -- verify the
binary under test actually contains the change before trusting a
delta.

Sorting is now absent from both areas. The remaining cost is branch
bookkeeping plus the page faults for the scratch buffer, which is why
the classifier now counts kernel paging as allocation.

## The BTreeMap weld pattern

Four areas showed the same shape: a BTreeMap used as a vertex or midpoint
weld cache, dominating the profile. Three were safe to hash, one was not.

  levelset   79 ms -> 45 ms   43% faster
  refine    147 ms -> 47 ms   68% faster
  decompose 670 ms -> 645 ms   3.7% faster

The split is whether map ITERATION order is observable. refine and
levelset only ever query by key, so their ordering cannot reach the
output. EdgeAdjacency (the genus hotspot) is iterated by six methods and
decompose feeds that order into loop stitching, so it stays a BTreeMap:
hashing it would silently change results.

refine's docs claimed its cross-process determinism came FROM the
BTreeMap. That was wrong even before the change: ordering comes from the
triangle walk. The guarantee is now checked by digesting output in five
separate processes, and mutation-tested by making vertex numbering
depend on cache iteration order, which yields five different digests.

decompose barely moved. Its cost is worst_concavity, an O(n^2) scan of
every vertex for every triangle per split iteration, so the weld was
never its bottleneck. That is a structural fix, not a container swap.

Two CAPDIAG eprintln! calls were also found in decompose, formatting and
allocating on every call in release. They were the only eprintln! in any
kernel library.
