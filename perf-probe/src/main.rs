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
use axiolid_core::Aabb;
use axiolid_core::{BooleanOperator, Point3, Tolerance};
use axiolid_mesh::{audit_mesh, TriMesh};
use axiolid_mesh_boolean_boolmesh::BoolmeshBoolean;
use axiolid_mesh_boolean_contract::MeshBoolean;
use axiolid_spatial::{Bvh, SpatialIndex, SpatialItem};
use std::ops::ControlFlow;

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
        "decimate" => {
            // Edge-collapse simplification: a priority queue over
            // candidate collapses, so heap churn and topology updates
            // rather than float work.
            let m = sphere(6, 1.0, 0.0);
            for _ in 0..6 {
                let r = axiolid_decimate::decimate(
                    &m,
                    axiolid_decimate::DecimateTarget::TriangleBudget(8000),
                    tol,
                );
                if let Err(e) = &r {
                    eprintln!("decimate FAILED: {e:?}");
                }
                std::hint::black_box(&r);
            }
        }
        "refine" => {
            // Uniform subdivision: allocation-dominated by construction,
            // every pass quadruples the triangle count.
            let m = sphere(5, 1.0, 0.0);
            for _ in 0..4 {
                let r = axiolid_refine::refine(
                    &m,
                    axiolid_refine::RefineTarget::Uniform { levels: 2 },
                    None,
                    tol,
                );
                std::hint::black_box(&r);
            }
        }
        "raymesh" => {
            // NOT a BVH workload: nearest_hit deliberately scans every
            // triangle -- the broad phase lives in axiolid-spatial and is
            // the caller's job. So this measures the narrow-phase
            // ray-triangle test, which is float work, not pointer chasing.
            // Labelling it "BVH" would have misread its 43s as a tree
            // problem when it is an O(rays * triangles) scan.
            let m = sphere(6, 1.0, 0.0);
            let mut acc = 0usize;
            let mut dsum = 0.0f64;
            let mut isum = 0u64;
            for i in 0..2000 {
                let t = i as f64 * 0.001;
                let ray = axiolid_core::Ray3 {
                    origin: Point3::new(3.0 * t.cos(), 3.0 * t.sin(), 0.25),
                    direction: Point3::new(-t.cos(), -t.sin(), 0.0) - Point3::ZERO,
                };
                if let Ok(Some(h)) = axiolid_ray_mesh::nearest_hit(&m, &ray, tol) {
                    acc += 1;
                    dsum += h.t;
                    isum += h.triangle as u64;
                }
            }
            eprintln!("raymesh hits={acc} tsum={dsum:.9} isum={isum}");
            std::hint::black_box(acc);
        }
        "raybvh" => {
            // Same rays as "raymesh", but through the broad phase that
            // already exists in axiolid-spatial. Build cost is INSIDE
            // the timed region: a per-query BVH that is rebuilt each
            // call would be a regression, and hiding the build would
            // make the comparison flattering rather than useful.
            let m = sphere(6, 1.0, 0.0);
            let tol = Tolerance::MILLIMETRE;
            let tris: Vec<_> = (0..m.triangle_count())
                .map(|i| {
                    let pts = &m.positions;
                    let idx = &m.indices[i * 3..i * 3 + 3];
                    let t = [
                        pts[idx[0] as usize],
                        pts[idx[1] as usize],
                        pts[idx[2] as usize],
                    ];
                    let mut lo = t[0];
                    let mut hi = t[0];
                    for p in [t[1], t[2]] {
                        lo = Point3::new(lo.x.min(p.x), lo.y.min(p.y), lo.z.min(p.z));
                        hi = Point3::new(hi.x.max(p.x), hi.y.max(p.y), hi.z.max(p.z));
                    }
                    SpatialItem::new(i, Aabb { min: lo, max: hi })
                })
                .collect();
            let bvh = Bvh::build(tris);
            let mut hits = 0usize;
            let mut dsum = 0.0f64;
            let mut isum = 0u64;
            for i in 0..2000 {
                let t = i as f64 * 0.001;
                let ray = axiolid_core::Ray3 {
                    origin: Point3::new(3.0 * t.cos(), 3.0 * t.sin(), 0.25),
                    direction: Point3::new(-t.cos(), -t.sin(), 0.0) - Point3::ZERO,
                };
                // Front-to-back: the first candidate whose triangle is
                // actually hit is the nearest, so stop there. Feeding
                // every candidate would rebuild the brute-force scan.
                let mut best: Option<axiolid_ray_mesh::RayHit3> = None;
                bvh.visit_ray(&ray, &mut |cand| {
                    let i = *cand.key;
                    if let Ok(Some(h)) =
                        axiolid_ray_mesh::nearest_hit_among(&m, &ray, tol, i..i + 1)
                    {
                        best = Some(h);
                        return ControlFlow::Break(());
                    }
                    ControlFlow::Continue(())
                });
                if let Some(h) = best {
                    hits += 1;
                    dsum += h.t;
                    isum += h.triangle as u64;
                }
            }
            // A faster wrong answer is worthless, so print the hit
            // count for comparison against the brute-force arm.
            eprintln!("raybvh hits={hits} tsum={dsum:.9} isum={isum}");
            std::hint::black_box(hits);
        }
        "facaderay" => {
            // The win as a CALLER sees it: same public entry point,
            // repeated casts against one mesh.
            let app = axiolid::application::Application::portable().expect("app");
            let m = sphere(6, 1.0, 0.0);
            let mut acc = 0usize;
            for i in 0..2000 {
                let t = i as f64 * 0.001;
                let ray = axiolid_core::Ray3 {
                    origin: Point3::new(3.0 * t.cos(), 3.0 * t.sin(), 0.25),
                    direction: Point3::new(-t.cos(), -t.sin(), 0.0) - Point3::ZERO,
                };
                if matches!(app.nearest_mesh_hit(&m, &ray, tol), Ok(Some(_))) {
                    acc += 1;
                }
            }
            println!("facaderay hits={acc}");
        }
        "project" => {
            // Planar projection + 2D overlay: sorting and predicate
            // evaluation rather than mesh topology.
            let m = sphere(6, 1.0, 0.0);
            let plane = axiolid_core::PlaneFrame::ground();
            for _ in 0..40 {
                let r = axiolid_project::project_mesh(&m, plane, tol);
                std::hint::black_box(&r);
            }
        }
        "decompose" => {
            // Convex decomposition: repeated plane splits, each one a
            // boolean-flavoured topology rebuild.
            let m = sphere(5, 1.0, 0.0);
            for _ in 0..4 {
                let r = axiolid_decompose::convex_decompose(
                    &m,
                    axiolid_decompose::Strategy::Exact,
                    tol,
                );
                std::hint::black_box(&r);
            }
        }
        "refinehash" => {
            // Digest of the refined mesh. Printed so separate PROCESSES can be
            // compared: a per-process hash seed leaking into output ordering
            // would show up here and nowhere else.
            let m = sphere(3, 1.0, 0.0);
            let r = axiolid_refine::refine(
                &m,
                axiolid_refine::RefineTarget::Uniform { levels: 1 },
                None,
                tol,
            );
            let (mesh, _) = r.expect("refine");
            let mut acc: u64 = 1469598103934665603;
            for p in &mesh.positions {
                for v in [p.x, p.y, p.z] {
                    for b in v.to_bits().to_le_bytes() {
                        acc ^= u64::from(b);
                        acc = acc.wrapping_mul(1099511628211);
                    }
                }
            }
            for i in &mesh.indices {
                acc ^= u64::from(*i);
                acc = acc.wrapping_mul(1099511628211);
            }
            println!("refine_digest,{acc:016x}");
        }
        other => {
            eprintln!("unknown area: {other}");
            std::process::exit(2);
        }
    }
}
