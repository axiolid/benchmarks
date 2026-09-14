//! Per-area workloads for cost-category profiling.
//!
//! Each area runs ONE kind of work in a loop under its own process, so
//! `perf` attributes samples to that area alone. The categories
//! (sorting, allocation, pointer chasing, hashing, branch bookkeeping,
//! maths) are derived afterwards by classifying the symbols perf
//! reports -- see scripts/perf-areas.py.
//!
//! Usage: perf-probe <area> [threads]

use axiolid_contracts::ExecutionOptions;
use axiolid_core::{BooleanOperator, Point3, Tolerance};
use axiolid_mesh::{audit_mesh, TriMesh};
use axiolid_mesh_boolean_boolmesh::BoolmeshBoolean;
use axiolid_mesh_boolean_contract::MeshBoolean;

/// Icosphere built in-crate so the probe depends on no fixture files.
fn sphere(subdiv: u32, radius: f64, dx: f64) -> TriMesh {
    let t = (1.0 + 5f64.sqrt()) / 2.0;
    let mut pos = vec![
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
            let mut split = |x: u32, y: u32| -> u32 {
                let key = (x.min(y), x.max(y));
                *mid.entry(key).or_insert_with(|| {
                    let m = (pos[x as usize] + pos[y as usize]) * 0.5;
                    pos.push(m);
                    (pos.len() - 1) as u32
                })
            };
            let (ab, bc, ca) = (split(a, b), split(b, c), split(c, a));
            next.extend_from_slice(&[a, ab, ca, b, bc, ab, c, ca, bc, ab, bc, ca]);
        }
        idx = next;
    }
    for p in pos.iter_mut() {
        let n = (p.x * p.x + p.y * p.y + p.z * p.z).sqrt();
        *p = Point3::new(p.x / n * radius + dx, p.y / n * radius, p.z / n * radius);
    }
    TriMesh::new(pos, idx)
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let area = args.get(1).map(String::as_str).unwrap_or("all");
    let threads: usize = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(0);

    // A private pool when a thread count is given, so scaling runs are
    // not at the mercy of the ambient global pool.
    if threads > 0 {
        rayon::ThreadPoolBuilder::new()
            .num_threads(threads)
            .build_global()
            .ok();
    }

    let tol = Tolerance::MILLIMETRE;
    match area {
        "boolean" => {
            let a = sphere(5, 1.0, 0.0);
            let b = sphere(5, 1.0, 0.6);
            let provider = BoolmeshBoolean::new();
            let opts = ExecutionOptions::new(tol);
            for _ in 0..12 {
                let r = provider.boolean(&a, &b, BooleanOperator::Union, &opts);
                std::hint::black_box(&r);
            }
        }
        "audit" => {
            let m = sphere(6, 1.0, 0.0);
            for _ in 0..60 {
                std::hint::black_box(audit_mesh(&m, tol));
            }
        }
        "measure" => {
            let m = sphere(6, 1.0, 0.0);
            for _ in 0..40 {
                std::hint::black_box(axiolid_measure::volume_properties(&m, tol).ok());
                std::hint::black_box(axiolid_measure::surface_properties(&m, tol).ok());
                std::hint::black_box(axiolid_measure::second_moments(&m, tol).ok());
            }
        }
        "levelset" => {
            // Dense field sampling: the one shape predicted to favour
            // vectorisation, and previously never measured.
            use axiolid_core::Aabb;
            let mut b = Aabb::empty();
            b.extend(Point3::new(-1.5, -1.5, -1.5));
            b.extend(Point3::new(1.5, 1.5, 1.5));
            for _ in 0..4 {
                let r = axiolid_levelset::level_set(
                    |p: Point3| (p.x * p.x + p.y * p.y + p.z * p.z).sqrt() - 1.0,
                    b,
                    0.05,
                    0.0,
                );
                std::hint::black_box(&r);
            }
        }
        "inspect" => {
            // Point queries: winding number and containment, dominated by
            // per-triangle tests with a data-dependent branch.
            let m = sphere(6, 1.0, 0.0);
            for i in 0..120 {
                let f = i as f64 * 0.01;
                let p = Point3::new(0.1 + f, 0.2, 0.3);
                std::hint::black_box(axiolid_inspect::winding_number(&m, p));
                std::hint::black_box(axiolid_inspect::contains(&m, p));
            }
        }
        "heal" => {
            // Defect detection: BVH build plus traversal, pointer heavy.
            let m = sphere(5, 1.0, 0.0);
            for _ in 0..20 {
                std::hint::black_box(axiolid_heal::self_intersections(&m));
                std::hint::black_box(axiolid_heal::diagnose(&m, tol));
            }
        }
        "genus" => {
            // Topology: euler characteristic over the edge structure.
            let m = sphere(6, 1.0, 0.0);
            for _ in 0..40 {
                std::hint::black_box(axiolid_inspect::genus(&m).ok());
            }
        }
        other => {
            eprintln!("unknown area: {other}");
            std::process::exit(2);
        }
    }
}
