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
