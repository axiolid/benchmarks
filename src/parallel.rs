//! Thread scaling, parallel efficiency, and the equivalence gates.
//!
//! The equivalence rows matter more than the timing ones. The
//! kernel already runs boolean03, triangulation and hmesh under
//! rayon, so until now parallel code had no test proving it agrees
//! with itself at one thread. A scaling curve on a wrong answer is
//! worthless, so equivalence gates and timings only record.

use crate::scorecard::{emit, Row};
use axiolid_contracts::ExecutionOptions;
use axiolid_core::{BooleanOperator, Tolerance};
use axiolid_mesh::TriMesh;
use axiolid_mesh_boolean_boolmesh::BoolmeshBoolean;
use axiolid_mesh_boolean_contract::MeshBoolean;
use std::time::Instant;

/// A workload big enough that parallelism can actually show.
///
/// Two offset spheres: the intersection curve is long, so the
/// parallel stages have real work rather than being dominated by
/// pool setup.
fn operands() -> (TriMesh, TriMesh) {
    let a = crate::sphere::icosphere([0.0, 0.0, 0.0], 1.0, 5);
    let b = crate::sphere::icosphere([0.6, 0.0, 0.0], 1.0, 5);
    (a, b)
}

/// Order-independent hash, matching metrics_full.
fn hash(m: &TriMesh) -> u64 {
    let q = |v: f64| (v * 1.0e9).round() as i64;
    let mut acc = 0u64;
    for t in m.indices.chunks_exact(3) {
        let mut h = 1469598103934665603u64;
        for &i in t {
            let p = m.positions[i as usize];
            for c in [q(p.x), q(p.y), q(p.z)] {
                h ^= c as u64;
                h = h.wrapping_mul(1099511628211);
            }
        }
        acc = acc.wrapping_add(h);
    }
    acc
}

/// Run the union inside a pool of exactly `threads` workers.
///
/// A private pool rather than the global one: the global pool can
/// only be sized once per process, so sweeping thread counts in a
/// single run requires scoped pools.
fn run_with(threads: usize) -> Option<(u64, f64, usize)> {
    let (a, b) = operands();
    let provider = BoolmeshBoolean::new();
    let options = ExecutionOptions::new(Tolerance::MILLIMETRE);

    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(threads)
        .build()
        .ok()?;

    pool.install(|| {
        let t = Instant::now();
        let out = provider
            .boolean(&a, &b, BooleanOperator::Union, &options)
            .ok()?;
        let ms = t.elapsed().as_secs_f64() * 1000.0;
        Some((hash(&out.mesh), ms, out.mesh.indices.len() / 3))
    })
}

/// Sweep thread counts, gate equivalence, record scaling.
pub fn report() -> usize {
    println!("\n\nParallel scaling and equivalence");
    println!("{}", "-".repeat(80));

    let counts = [1usize, 2, 4, 8, 16];
    let mut results = Vec::new();
    for n in counts {
        match run_with(n) {
            Some(r) => results.push((n, r)),
            None => {
                println!("  {n} threads: pool unavailable or boolean refused");
            }
        }
    }
    if results.is_empty() {
        println!("  no thread count produced a result");
        return 1;
    }

    println!();
    println!(
        "  {:>8} {:>10} {:>10} {:>10}",
        "threads", "ms", "speedup", "efficiency"
    );
    let base_ms = results[0].1 .1;
    for (n, (_, ms, _)) in &results {
        let speedup = base_ms / ms;
        let eff = speedup / *n as f64;
        println!(
            "  {:>8} {:>10.2} {:>10.2} {:>9.0}%",
            n,
            ms,
            speedup,
            eff * 100.0
        );
    }

    // THE row this module exists for. Identical input through the
    // same code at different widths must give an identical surface.
    // Hashes are order-independent, so a provider that merely
    // reorders output still passes; one that moves a vertex does
    // not.
    let h0 = results[0].1 .0;
    let agree = results.iter().filter(|(_, (h, _, _))| *h == h0).count();
    let mut rows = vec![Row::gate(
        "seq/parallel equivalence",
        agree == results.len(),
        format!("{agree}/{} widths agree", results.len()),
    )];

    let tris0 = results[0].1 .2;
    let same_size = results.iter().all(|(_, (_, _, t))| *t == tris0);
    rows.push(Row::gate(
        "triangle count stable",
        same_size,
        format!("{tris0} tris"),
    ));

    // SIMD equivalence is NOT gated here, and the reason is
    // recorded rather than the row quietly omitted: the boolean
    // provider never consults InstructionPolicy. Nothing in
    // boolmesh reads CpuExecution, so toggling Auto against
    // Portable would compare a path against itself and report a
    // guaranteed pass. That is worse than no gate, because it
    // looks like coverage. Wiring the policy through the provider
    // is the prerequisite, and it is provider work, not harness
    // work.
    rows.push(Row::absent(
        "SIMD on/off equivalence",
        "provider ignores InstructionPolicy; gate would be vacuous",
    ));

    let best = results
        .iter()
        .map(|(_, (_, ms, _))| *ms)
        .fold(f64::INFINITY, f64::min);
    rows.push(Row::record("best wall clock", format!("{best:.2} ms")));
    let peak_speedup = base_ms / best;
    rows.push(Row::record("peak speedup", format!("{peak_speedup:.2}x")));

    emit("two offset spheres, union", &rows)
}
