//! Where does auto-vectorisation actually help?
//!
//! The kernel contains NO hand-written SIMD: crates/execution/cpu
//! detects instruction sets but no compute path consults
//! InstructionPolicy. So the only SIMD in play is what LLVM finds
//! on its own, and the honest experiment is to compile the SAME
//! source at baseline x86-64 (SSE2) versus target-cpu=native and
//! compare like for like.
//!
//! Each case prints median ns so the two builds can be diffed.
//! Median rather than mean: a single scheduler preemption skews a
//! mean far more than it moves a median.

use axiolid_core::{Point3, Tolerance};
use axiolid_mesh::{audit_mesh, TriMesh};
use std::time::Instant;

/// Median wall time of `reps` runs, in nanoseconds.
///
/// `std::hint::black_box` on the result stops LLVM deleting the
/// whole computation as dead when the value is unused -- without
/// it the native build can "win" by optimising the work away.
fn median_ns<T>(reps: usize, mut f: impl FnMut() -> T) -> f64 {
    let mut samples = Vec::with_capacity(reps);
    for _ in 0..reps {
        let t = Instant::now();
        let out = f();
        let ns = t.elapsed().as_nanos() as f64;
        std::hint::black_box(out);
        samples.push(ns);
    }
    samples.sort_by(f64::total_cmp);
    samples[samples.len() / 2]
}

/// Icosphere, subdivided `n` times.
fn sphere(n: u32, r: f64, dx: f64) -> TriMesh {
    let mut m = axiolid_reference_sphere(n, r);
    for p in m.positions.iter_mut() {
        p.x += dx;
    }
    m
}

/// Icosahedron subdivided in place; avoids depending on a
/// generator crate whose API may differ from the benchmark suite.
fn axiolid_reference_sphere(subdiv: u32, radius: f64) -> TriMesh {
    let t = (1.0 + 5.0f64.sqrt()) / 2.0;
    let mut ps: Vec<Point3> = vec![
        Point3::new(-1.0, t, 0.0),
        Point3::new(1.0, t, 0.0),
        Point3::new(-1.0, -t, 0.0),
        Point3::new(1.0, -t, 0.0),
        Point3::new(0.0, -1.0, t),
        Point3::new(0.0, 1.0, t),
        Point3::new(0.0, -1.0, -t),
        Point3::new(0.0, 1.0, -t),
        Point3::new(t, 0.0, -1.0),
        Point3::new(t, 0.0, 1.0),
        Point3::new(-t, 0.0, -1.0),
        Point3::new(-t, 0.0, 1.0),
    ];
    let mut idx: Vec<u32> = vec![
        0, 11, 5, 0, 5, 1, 0, 1, 7, 0, 7, 10, 0, 10, 11, 1, 5, 9, 5, 11, 4, 11, 10, 2, 10, 7, 6, 7,
        1, 8, 3, 9, 4, 3, 4, 2, 3, 2, 6, 3, 6, 8, 3, 8, 9, 4, 9, 5, 2, 4, 11, 6, 2, 10, 8, 6, 7, 9,
        8, 1,
    ];

    for _ in 0..subdiv {
        let mut next = Vec::with_capacity(idx.len() * 4);
        let mut mid = std::collections::HashMap::new();
        for tri in idx.chunks_exact(3) {
            let (a, b, c) = (tri[0], tri[1], tri[2]);
            let mut edge = |x: u32, y: u32| -> u32 {
                let key = (x.min(y), x.max(y));
                *mid.entry(key).or_insert_with(|| {
                    let p = ps[x as usize] + (ps[y as usize] - ps[x as usize]) * 0.5;
                    ps.push(p);
                    (ps.len() - 1) as u32
                })
            };
            let (ab, bc, ca) = (edge(a, b), edge(b, c), edge(c, a));
            next.extend_from_slice(&[a, ab, ca, b, bc, ab, c, ca, bc, ab, bc, ca]);
        }
        idx = next;
    }
    for p in ps.iter_mut() {
        let len = (p.x * p.x + p.y * p.y + p.z * p.z).sqrt();
        *p = Point3::new(p.x / len * radius, p.y / len * radius, p.z / len * radius);
    }
    TriMesh::new(ps, idx)
}

fn main() {
    let tol = Tolerance::MILLIMETRE;
    let big = axiolid_reference_sphere(6, 1.0);
    let a = sphere(5, 1.0, 0.0);
    let b = sphere(5, 1.0, 0.6);
    println!(
        "# mesh: {} verts {} tris",
        big.positions.len(),
        big.indices.len() / 3
    );
    println!("case,median_ns");

    // 1. Pure float reduction over contiguous data. The best case
    //    for vectorisation: no branches, unit stride, fused ops.
    let ns = median_ns(200, || axiolid_measure::volume_properties(&big, tol));
    println!("volume_properties,{ns:.0}");

    let ns = median_ns(200, || axiolid_measure::surface_properties(&big, tol));
    println!("surface_properties,{ns:.0}");

    // 2. Second moments: more arithmetic per triangle than either
    //    of the above, same access pattern.
    let ns = median_ns(200, || axiolid_measure::second_moments(&big, tol));
    println!("second_moments,{ns:.0}");

    // 3. Audit: hash-heavy and branchy. Edge bookkeeping dominates
    //    the float work, so vectorisation has little to grip.
    let ns = median_ns(50, || audit_mesh(&big, tol));
    println!("audit_mesh,{ns:.0}");

    // 4. Winding number: transcendental-ish per triangle, but a
    //    scalar accumulation with a data-dependent branch.
    let ns = median_ns(50, || {
        axiolid_inspect::winding_number(&big, Point3::new(0.1, 0.2, 0.3))
    });
    println!("winding_number,{ns:.0}");

    // 5. Self-intersection: BVH traversal, pointer chasing.
    let medium = axiolid_reference_sphere(4, 1.0);
    let ns = median_ns(20, || axiolid_heal::self_intersections(&medium));
    println!("self_intersections,{ns:.0}");

    // 6. Boolean: the headline operation, allocation-heavy.
    use axiolid_contracts::ExecutionOptions;
    use axiolid_core::BooleanOperator;
    use axiolid_mesh_boolean_boolmesh::BoolmeshBoolean;
    use axiolid_mesh_boolean_contract::MeshBoolean;
    let provider = BoolmeshBoolean::new();
    let opts = ExecutionOptions::new(tol);
    let ns = median_ns(20, || {
        provider.boolean(&a, &b, BooleanOperator::Union, &opts)
    });
    println!("boolean_union,{ns:.0}");

    // The measure entry points each call audit_mesh first, so
    // timing them measures the audit, not the float reduction --
    // which is why volume/surface/second_moments all landed within
    // noise of audit_mesh itself. These isolate the arithmetic.
    let ns = median_ns(200, || {
        let mut vol = 0.0f64;
        for t in big.indices.chunks_exact(3) {
            let a = big.positions[t[0] as usize];
            let b = big.positions[t[1] as usize];
            let c = big.positions[t[2] as usize];
            vol += a.dot(b.cross(c)) / 6.0;
        }
        vol
    });
    println!("raw_volume_loop,{ns:.0}");

    // Flat array of the same data: contiguous, no index
    // indirection. Separates "the maths does not vectorise" from
    // "the gather through an index buffer defeats it".
    let flat: Vec<f64> = big
        .indices
        .chunks_exact(3)
        .flat_map(|t| {
            let a = big.positions[t[0] as usize];
            let b = big.positions[t[1] as usize];
            let c = big.positions[t[2] as usize];
            [a.x, a.y, a.z, b.x, b.y, b.z, c.x, c.y, c.z]
        })
        .collect();
    let ns = median_ns(200, || {
        let mut acc = 0.0f64;
        for w in flat.chunks_exact(9) {
            acc += w[0] * (w[4] * w[8] - w[5] * w[7]) - w[1] * (w[3] * w[8] - w[5] * w[6])
                + w[2] * (w[3] * w[7] - w[4] * w[6]);
        }
        acc / 6.0
    });
    println!("flat_array_loop,{ns:.0}");

    // Pure elementwise f64: the textbook vectorisable kernel.
    let xs: Vec<f64> = (0..1_000_000).map(|i| f64::from(i) * 0.5).collect();
    let ns = median_ns(200, || xs.iter().map(|v| v * v + 1.0).sum::<f64>());
    println!("elementwise_fma,{ns:.0}");

    // Same maths, four independent accumulators. f64 addition is not
    // associative, so LLVM may not legally re-associate a single
    // accumulator into lanes -- that is the usual reason a "textbook
    // vectorisable" reduction stays scalar. Splitting the chain by hand
    // grants the re-association explicitly. If THIS speeds up under
    // native while the naive sum does not, associativity was the block,
    // not the instruction set.
    let ns = median_ns(200, || {
        let mut acc = [0.0f64; 4];
        for w in xs.chunks_exact(4) {
            for k in 0..4 {
                acc[k] += w[k] * w[k] + 1.0;
            }
        }
        acc[0] + acc[1] + acc[2] + acc[3]
    });
    println!("elementwise_4acc,{ns:.0}");

    // How much of the entry point is the reduction at all? If audit
    // dominates, halving the arithmetic is worth almost nothing at the
    // API boundary, and optimising it would be effort spent where the
    // time is not.
    let ns = median_ns(50, || audit_mesh(&big, tol));
    println!("audit_only,{ns:.0}");

    // Profiling says ~17% of audit_mesh is sort_unstable_by_key over
    // 3*triangle_count edge records, and the audit is ~94% of the
    // measure entry points. So the sort -- not the float maths -- is
    // where the time is.
    //
    // The key is a (u64, u64) pair of VERTEX INDICES, bounded by
    // positions.len(), which a comparison sort cannot exploit but an
    // LSD radix sort can. Prototyped here before touching the kernel:
    // if the win does not show on representative data it is not worth
    // perturbing a structure ten test suites depend on.
    let nv = big.positions.len() as u64;
    let mut records: Vec<(u64, u64, i8)> = Vec::with_capacity(big.indices.len());
    for t in big.indices.chunks_exact(3) {
        for (x, y) in [(t[0], t[1]), (t[1], t[2]), (t[2], t[0])] {
            let (l, h) = (u64::from(x.min(y)), u64::from(x.max(y)));
            records.push((l, h, 1));
        }
    }

    let base = records.clone();
    let ns = median_ns(50, || {
        let mut v = base.clone();
        v.sort_unstable_by_key(|e| (e.0, e.1));
        v.len()
    });
    println!("edge_sort_comparison,{ns:.0}");

    let ns = median_ns(50, || {
        let mut v = base.clone();
        radix_sort_edges(&mut v, nv);
        v.len()
    });
    println!("edge_sort_radix,{ns:.0}");

    // Correctness, not just speed: a faster sort that orders
    // differently would silently change every downstream edge count.
    let mut want = base.clone();
    want.sort_unstable_by_key(|e| (e.0, e.1));
    let mut got = base.clone();
    radix_sort_edges(&mut got, nv);
    let keys_match = want.iter().zip(&got).all(|(a, b)| (a.0, a.1) == (b.0, b.1));
    println!("# radix key order matches comparison sort: {keys_match}");
}

/// LSD radix sort on the (low, high) vertex-index key.
///
/// Two stable counting passes, least-significant field first, so the
/// final order is by `low` then `high` -- identical to the comparison
/// sort's key order. Counting sort is stable, which is what makes the
/// two-pass composition produce a correct lexicographic ordering.
fn radix_sort_edges(v: &mut Vec<(u64, u64, i8)>, buckets: u64) {
    let n = buckets as usize + 1;
    let mut out = vec![(0u64, 0u64, 0i8); v.len()];
    for pass in 0..2 {
        let key = |e: &(u64, u64, i8)| if pass == 0 { e.1 } else { e.0 } as usize;
        let mut counts = vec![0usize; n + 1];
        for e in v.iter() {
            counts[key(e) + 1] += 1;
        }
        for i in 0..n {
            counts[i + 1] += counts[i];
        }
        for e in v.iter() {
            let k = key(e);
            out[counts[k]] = *e;
            counts[k] += 1;
        }
        std::mem::swap(v, &mut out);
    }
}
