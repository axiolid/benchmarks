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
use axiolid_core::{BooleanOperator, Frame2, Frame3, Point2, Point3, Tolerance, Vec2, Vec3};
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

/// Per-triangle AABB items for a BVH.
///
/// The ray arms already needed this; the pair-query arms need the same
/// thing, so it lives here rather than being written twice with a
/// chance of the two drifting apart.
fn tri_items(m: &TriMesh) -> Vec<SpatialItem<usize>> {
    (0..m.triangle_count())
        .map(|i| {
            let idx = &m.indices[i * 3..i * 3 + 3];
            let t = [
                m.positions[idx[0] as usize],
                m.positions[idx[1] as usize],
                m.positions[idx[2] as usize],
            ];
            let mut lo = t[0];
            let mut hi = t[0];
            for p in [t[1], t[2]] {
                lo = Point3::new(lo.x.min(p.x), lo.y.min(p.y), lo.z.min(p.z));
                hi = Point3::new(hi.x.max(p.x), hi.y.max(p.y), hi.z.max(p.z));
            }
            SpatialItem::new(i, Aabb { min: lo, max: hi })
        })
        .collect()
}

/// Annulus profile: an outer ring with `holes` circular holes punched
/// out. Holes matter because they force real triangulation rather than
/// a fan, which is what production profiles look like.
fn ring_with_holes(segments: usize, holes: usize) -> axiolid_construct::profile::Rings {
    let circle = |cx: f64, cy: f64, r: f64, n: usize, cw: bool| -> Vec<Point2> {
        (0..n)
            .map(|i| {
                let mut t = i as f64 / n as f64 * std::f64::consts::TAU;
                if cw {
                    t = -t;
                }
                Point2::new(cx + r * t.cos(), cy + r * t.sin())
            })
            .collect()
    };
    axiolid_construct::profile::Rings {
        outer: circle(0.0, 0.0, 4.0, segments, false),
        holes: (0..holes)
            .map(|i| {
                let a = i as f64 / holes.max(1) as f64 * std::f64::consts::TAU;
                circle(a.cos() * 2.0, a.sin() * 2.0, 0.5, 12, true)
            })
            .collect(),
    }
}

/// Axis-aligned cube as a face-list solid, for the offset arms.
fn box_polyhedron(h: f64) -> axiolid_construct::polyhedron::Polyhedron {
    let v = |x: f64, y: f64, z: f64| Point3::new(x * h, y * h, z * h);
    let faces = vec![
        vec![v(-1.0, -1.0, -1.0), v(-1.0, 1.0, -1.0), v(1.0, 1.0, -1.0), v(1.0, -1.0, -1.0)],
        vec![v(-1.0, -1.0, 1.0), v(1.0, -1.0, 1.0), v(1.0, 1.0, 1.0), v(-1.0, 1.0, 1.0)],
        vec![v(-1.0, -1.0, -1.0), v(1.0, -1.0, -1.0), v(1.0, -1.0, 1.0), v(-1.0, -1.0, 1.0)],
        vec![v(1.0, -1.0, -1.0), v(1.0, 1.0, -1.0), v(1.0, 1.0, 1.0), v(1.0, -1.0, 1.0)],
        vec![v(1.0, 1.0, -1.0), v(-1.0, 1.0, -1.0), v(-1.0, 1.0, 1.0), v(1.0, 1.0, 1.0)],
        vec![v(-1.0, 1.0, -1.0), v(-1.0, -1.0, -1.0), v(-1.0, -1.0, 1.0), v(-1.0, 1.0, 1.0)],
    ];
    axiolid_construct::polyhedron::Polyhedron::new(faces).expect("unit cube is a valid solid")
}

/// Cubic Bezier in 3D, offset in y so two of them do not coincide.
fn bezier3(dy: f64) -> axiolid_curve::BSplineCurve<Point3> {
    axiolid_curve::BSplineCurve {
        degree: 3,
        control_points: vec![
            Point3::new(0.0, dy, 0.0),
            Point3::new(1.0, dy + 1.0, 0.5),
            Point3::new(2.0, dy - 1.0, 1.0),
            Point3::new(3.0, dy, 1.5),
        ],
        knots: vec![0.0, 1.0],
        multiplicities: vec![4, 4],
        weights: None,
        closed: false,
        self_intersect: Some(false),
        knot_spec: axiolid_curve::KnotSpec::PiecewiseBezier,
    }
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
        "handleray" => {
            // The zero-bookkeeping path: build once, cast many. No
            // digest per call, no lock.
            use axiolid::ray_index::MeshRayIndex;
            let m = sphere(6, 1.0, 0.0);
            let index = MeshRayIndex::build(&m, tol);
            let mut hits = 0usize;
            for i in 0..2000 {
                let t = i as f64 * 0.001;
                let ray = axiolid_core::Ray3 {
                    origin: Point3::new(3.0 * t.cos(), 3.0 * t.sin(), 0.25),
                    direction: Point3::new(-t.cos(), -t.sin(), 0.0) - Point3::ZERO,
                };
                if matches!(index.nearest_hit(&ray, tol), Ok(Some(_))) {
                    hits += 1;
                }
            }
            println!("handleray hits={hits}");
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
        "orient3" => {
            // Exact orientation on clean input: the filter should settle
            // almost everything, so this is the fast path.
            use axiolid_predicates::scene::{orient3_scene, DegeneracyRate};
            let scene = orient3_scene(40_000, DegeneracyRate::None, 0x51ED_2701);
            for _ in 0..120 {
                for c in &scene {
                    std::hint::black_box(axiolid_predicates::orient3d(c[0], c[1], c[2], c[3]));
                }
            }
        }
        "orient3degen" => {
            // Same predicate, frequently degenerate input: the filter
            // defers and exact arithmetic dominates. Paired with orient3
            // so the cost of certainty is visible rather than averaged
            // into one number.
            use axiolid_predicates::scene::{orient3_scene, DegeneracyRate};
            let scene = orient3_scene(40_000, DegeneracyRate::Frequent, 0x51ED_2701);
            for _ in 0..120 {
                for c in &scene {
                    std::hint::black_box(axiolid_predicates::orient3d(c[0], c[1], c[2], c[3]));
                }
            }
        }
        "orient2" => {
            // 2D orientation: the most-called predicate in planar code.
            use axiolid_predicates::scene::{orient2_scene, DegeneracyRate};
            let scene = orient2_scene(60_000, DegeneracyRate::Occasional, 0x51ED_2701);
            for _ in 0..120 {
                for c in &scene {
                    std::hint::black_box(axiolid_predicates::orient2d(c[0], c[1], c[2]));
                }
            }
        }
        "incircle" => {
            // Delaunay in-circle test on a 2D scene, reusing the orient2
            // generator: four points, so the last is the query.
            use axiolid_predicates::scene::{orient2_scene, DegeneracyRate};
            let scene = orient2_scene(30_000, DegeneracyRate::Occasional, 0xC0FF_EE01);
            for _ in 0..120 {
                for w in scene.windows(2) {
                    let (a, b) = (w[0], w[1]);
                    std::hint::black_box(axiolid_predicates::incircle(a[0], a[1], a[2], b[0]));
                }
            }
        }
        "insphere" => {
            // 3D in-sphere: the most expensive certified predicate, and
            // the one that drives Delaunay tetrahedralisation.
            use axiolid_predicates::scene::{orient3_scene, DegeneracyRate};
            let scene = orient3_scene(20_000, DegeneracyRate::Occasional, 0xC0FF_EE01);
            for _ in 0..120 {
                for w in scene.windows(2) {
                    let (a, b) = (w[0], w[1]);
                    std::hint::black_box(axiolid_predicates::insphere(
                        a[0], a[1], a[2], a[3], b[0],
                    ));
                }
            }
        }
        "hull" => {
            // Convex hull of a point cloud: the classic incremental
            // construction, dominated by orientation predicates.
            let mut pts = Vec::new();
            for i in 0..3_000u32 {
                let f = i as f64 * 0.7391;
                pts.push(Point3::new(f.sin() * 2.0, (f * 1.7).cos() * 2.0, (f * 2.3).sin() * 2.0));
            }
            for _ in 0..8 {
                let h = axiolid_construct::hull::convex_hull(&pts);
                debug_assert!(h.is_ok(), "hull arm must produce a hull");
                std::hint::black_box(h.ok());
            }
        }
        "extrude" => {
            // Profile with holes swept to a solid: triangulation plus
            // side-wall generation, the bread and butter of BIM geometry.
            let rings = ring_with_holes(96, 16);
            for _ in 0..4_000 {
                let m = axiolid_construct::extrude::extrude_profile(
                    &rings,
                    Vec3::new(0.0, 0.0, 1.0),
                    3.0,
                    Tolerance::MILLIMETRE,
                );
                debug_assert!(m.is_ok(), "extrude arm must produce a solid");
                std::hint::black_box(m.ok());
            }
        }
        "revolve" => {
            // Sweep a profile about an axis: trigonometry per station
            // times profile size, a different shape from extrude.
            let rings = ring_with_holes(48, 8);
            for _ in 0..200 {
                let m = axiolid_construct::revolve::revolve(
                    &rings,
                    Point3::new(0.0, 0.0, 0.0),
                    Vec3::new(0.0, 0.0, 1.0),
                    std::f64::consts::PI * 1.5,
                    Tolerance::MILLIMETRE,
                );
                debug_assert!(m.is_ok(), "revolve arm must produce a solid");
                std::hint::black_box(m.ok());
            }
        }
        "loft" => {
            // Blend a profile through stations: purely topological
            // stitching between rings, almost no arithmetic.
            let rings = ring_with_holes(64, 10);
            let stations: Vec<_> = (0..24)
                .map(|i| {
                    let t = i as f64 * 0.25;
                    axiolid_construct::loft::place(&rings, move |p| {
                        Point3::new(p.x + t * 0.1, p.y, t)
                    })
                })
                .collect();
            for _ in 0..3_000 {
                let m = axiolid_construct::loft::loft(&rings, &stations, false);
                debug_assert!(m.is_ok(), "loft arm must produce a solid");
                std::hint::black_box(m.ok());
            }
        }
        "offsetsolid" => {
            // Grow a solid by a distance: plane offsetting plus face
            // re-intersection, where the work is planar algebra.
            let solid = box_polyhedron(1.0);
            for _ in 0..40_000 {
                let r = axiolid_construct::offset::offset_solid(
                    &solid,
                    0.1,
                    axiolid_construct::offset::OffsetDirection::Outward,
                );
                debug_assert!(r.is_ok(), "offset arm must produce a solid");
                std::hint::black_box(r.ok());
            }
        }
        "shell" => {
            // Hollow a solid to a wall thickness: the inward offset plus
            // cavity construction, so roughly twice the offset work.
            let solid = box_polyhedron(1.0);
            for _ in 0..2_000 {
                let r = axiolid_construct::offset::shell_solid(&solid, 0.1);
                debug_assert!(r.is_ok(), "shell arm must produce a solid");
                std::hint::black_box(r.ok());
            }
        }
        "frenet" => {
            // Frame transport along a curved 3D spine: numerical
            // integration per evaluation, so cost scales with arc length
            // rather than control-point count.
            use axiolid_curve::{CurvatureLaw, Intrinsic3};
            let curve = Intrinsic3::new(
                Frame3 {
                    origin: Point3::new(0.0, 0.0, 0.0),
                    x: Vec3::X,
                    y: Vec3::Y,
                    z: Vec3::Z,
                },
                CurvatureLaw::circular(0.35),
                CurvatureLaw::circular(0.12),
                40.0,
            );
            for i in 0..16_000 {
                let s = (i % 400) as f64 * 0.1;
                let f = axiolid_evaluate::frenet::frenet_frame(&curve, s);
                debug_assert!(f.is_ok(), "frenet arm must evaluate");
                std::hint::black_box(f.ok());
            }
        }
        "arclength" => {
            // Arc-length parameterisation of a 2D intrinsic curve: the
            // Gauss-Legendre panels that frenet also pays, isolated in 2D
            // so the integration cost shows without frame transport.
            use axiolid_curve::{CurvatureLaw, Intrinsic2};
            let curve = Intrinsic2::new(
                Frame2 {
                    origin: Point2::new(0.0, 0.0),
                    x: Vec2::X,
                    y: Vec2::Y,
                },
                CurvatureLaw::circular(0.25),
                40.0,
            );
            for i in 0..60_000 {
                let s = (i % 400) as f64 * 0.1;
                let p = axiolid_evaluate::arc_length::intrinsic_point(&curve, s);
                debug_assert!(p.is_ok(), "arclength arm must evaluate");
                std::hint::black_box(p.ok());
            }
        }
        "curvedist" => {
            // Certified distance between two curves: subdivision to a
            // proven bound, so the cost is refinement depth, not a fixed
            // sample count. The exact-arithmetic end of the kernel.
            use axiolid_nurbs::CertifiedProjectionOptions;
            let a = bezier3(0.0);
            let b = bezier3(1.3);
            let opts = CertifiedProjectionOptions::new(
                Tolerance::new(1e-4, 1e-10).unwrap(),
                65_536,
                64,
            )
            .unwrap();
            for _ in 0..40 {
                let d = axiolid_nurbs::distance_curve3_certified(&a, &b, opts);
                debug_assert!(d.is_ok(), "curvedist arm must certify a distance: {:?}", d.as_ref().err());
                std::hint::black_box(d.ok());
            }
        }
        "curveproject" => {
            // Certified point projection onto a curve: same refinement
            // machinery as curvedist but against a fixed target, so the
            // two separate search cost from pair-subdivision cost.
            use axiolid_nurbs::CertifiedProjectionOptions;
            let c = bezier3(0.0);
            let opts = CertifiedProjectionOptions::new(
                Tolerance::new(1e-4, 1e-10).unwrap(),
                65_536,
                64,
            )
            .unwrap();
            for i in 0..400 {
                let f = i as f64 * 0.01;
                let t = Point3::new(0.5 + f, 0.4, 0.3);
                let r = axiolid_nurbs::project_curve3_certified(&c, t, opts);
                debug_assert!(r.is_ok(), "curveproject arm must certify");
                std::hint::black_box(r.ok());
            }
        }
        "minkowski" => {
            // Convex sum: every face pair of two solids, so cost grows
            // multiplicatively rather than additively.
            let a = sphere(3, 1.0, 0.0);
            let b = sphere(1, 0.25, 0.0);
            for _ in 0..2 {
                let r = axiolid_minkowski::minkowski_sum(&a, &b, tol);
                debug_assert!(r.is_ok(), "minkowski arm must do real work");
                std::hint::black_box(r.ok());
            }
        }
        "pointindex" => {
            // Build a k-d style point index then query it: build cost is
            // a sort, query cost is pointer chasing.
            let m = sphere(6, 1.0, 0.0);
            let pts: Vec<Point3> = m.positions.clone();
            for _ in 0..8 {
                let idx = axiolid_spatial::PointIndex::build(&pts);
                std::hint::black_box(idx.len());
            }
        }
        "pointnear" => {
            // Nearest-neighbour queries against a prebuilt index: pure
            // traversal, no build cost in the measurement.
            let m = sphere(6, 1.0, 0.0);
            let pts: Vec<Point3> = m.positions.clone();
            let idx = axiolid_spatial::PointIndex::build(&pts);
            for i in 0..4000 {
                let f = i as f64 * 0.001;
                std::hint::black_box(idx.nearest(Point3::new(f, 0.2, 0.3)).ok());
            }
        }
        "bvhpairs" => {
            // Self-intersection candidate pairs: quadratic in the worst
            // case, and the reason a BVH exists at all.
            let m = sphere(5, 1.0, 0.0);
            let bvh = Bvh::build(tri_items(&m));
            for _ in 0..6 {
                std::hint::black_box(bvh.overlap_pairs(0.0).pairs.len());
            }
        }
        "overlay" => {
            // Planar boolean on rings: sweep-line plus intersection
            // tests, a different cost shape from the mesh booleans.
            use axiolid_overlay::Ring;
            let mut rings = Vec::new();
            for i in 0..24 {
                let f = i as f64 * 0.35;
                rings.push(Ring {
                    points: vec![
                        Point2::new(f, 0.0),
                        Point2::new(f + 1.0, 0.0),
                        Point2::new(f + 1.0, 1.0),
                        Point2::new(f, 1.0),
                    ],
                });
            }
            for _ in 0..400 {
                let r = axiolid_overlay::union_soup(&rings, tol);
                debug_assert!(r.is_ok(), "overlay arm must do real work");
                std::hint::black_box(r.ok());
            }
        }
        "route" => {
            // Visibility-graph shortest path around barriers: graph
            // search cost, not geometry cost.
            use axiolid_overlay::{Polygon, Ring};
            let region = vec![Polygon {
                outer: Ring {
                    points: vec![
                        Point2::new(0.0, 0.0),
                        Point2::new(20.0, 0.0),
                        Point2::new(20.0, 20.0),
                        Point2::new(0.0, 20.0),
                    ],
                },
                holes: Vec::new(),
            }];
            let mut barriers = Vec::new();
            for i in 0..10 {
                let f = 1.0 + i as f64 * 1.8;
                barriers.push(vec![
                    Point2::new(f, 2.0),
                    Point2::new(f + 0.6, 2.0),
                    Point2::new(f + 0.6, 16.0),
                    Point2::new(f, 16.0),
                ]);
            }
            for _ in 0..120 {
                std::hint::black_box(axiolid_route::shortest_path(
                    &region,
                    &barriers,
                    Point2::new(0.5, 1.0),
                    Point2::new(19.5, 19.0),
                ).ok());
            }
        }
        "fieldsample" => {
            // Rasterise triangles into a layered height field: the
            // scatter-heavy counterpart to the levelset arm.
            use axiolid_field_ops::{sample_triangles_cpu, Triangle3};
            use axiolid_field::{FieldBounds, FieldConfig, FieldResourceBudget};
            let m = sphere(5, 1.0, 0.0);
            let tris: Vec<Triangle3> = (0..m.triangle_count())
                .map(|i| {
                    let idx = &m.indices[i * 3..i * 3 + 3];
                    Triangle3 {
                        a: m.positions[idx[0] as usize],
                        b: m.positions[idx[1] as usize],
                        c: m.positions[idx[2] as usize],
                    }
                })
                .collect();
            let bounds = FieldBounds::new(
                Point3::new(-1.2, -1.2, -1.2),
                Point3::new(1.2, 1.2, 1.2),
            )
            .expect("field bounds");
            let cfg = FieldConfig::new(
                Frame3 {
                    origin: Vec3::ZERO,
                    x: Vec3::X,
                    y: Vec3::Y,
                    z: Vec3::Z,
                },
                bounds,
                0.02,
                tol,
                // Explicit: the crate deliberately has no default budget,
                // since a library must not decide an application's memory cap.
                FieldResourceBudget::new(1 << 22, 1 << 24),
            )
            .expect("field config");
            for _ in 0..3 {
                std::hint::black_box(sample_triangles_cpu(&cfg, &tris).is_ok());
            }
        }
        "components" => {
            // Connected-component labelling over the adjacency: pure
            // union-find / traversal, no arithmetic.
            let m = sphere(7, 1.0, 0.0);
            for _ in 0..40 {
                std::hint::black_box(axiolid_mesh::component_count(&m));
            }
        }
        "minkdiff" => {
            // Difference rather than sum: same machinery, opposite
            // sweep direction, and a separate clipping path.
            let a = sphere(3, 1.0, 0.0);
            let b = sphere(1, 0.2, 0.0);
            for _ in 0..2 {
                std::hint::black_box(axiolid_minkowski::minkowski_difference_with(&a, &b, tol, &BoolmeshBoolean::new()).ok());
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
