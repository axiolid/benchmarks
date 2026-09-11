//! Algebraic exactness comparison across kernels.
//!
//! Speed rankings invert between workloads; exactness does not. A kernel that
//! violates a law of set algebra is wrong at any speed, and the violation is
//! measurable without knowing the true answer -- the identity IS the oracle.
//!
//! Operands are deliberately awkward: a rotated tool straddling a wall face
//! produces coincident and near-coincident faces, which is where floating-point
//! kernels actually break. Well-separated boxes would score every kernel 0 and
//! prove nothing.

use crate::ops::{score, Metrics, Op, Pair, IDENTITIES};
use crate::{axiolid_obb, axiolid_volume, flat_cutters, Box3, Obb};
use axiolid_mesh::TriMesh;

/// The operand pair every kernel is scored on.
///
/// `A` is a wall-like slab. `B` is rotated 30 degrees in plan and positioned so
/// it straddles A's face rather than sitting cleanly inside or outside: the
/// intersection is a genuine wedge, and several of B's faces come near-parallel
/// to A's. That is the configuration that separates an exact kernel from one
/// that merely rounds well.
fn operands() -> (Obb, Obb) {
    let a = Obb::aabb(Box3::new(0.0, 0.0, 1.5, 4.0, 0.4, 3.0));
    let b = Obb {
        centre: [1.0, 0.1, 1.5],
        half: [0.6, 0.5, 0.7],
        angle: std::f64::consts::FRAC_PI_6,
    };
    (a, b)
}

/// Evaluate one operation with axiolid's provider contract.
fn resolve(a: Obb, b: Obb, pair: Pair) -> (Obb, Obb) {
    match pair {
        Pair::Ab => (a, b),
        Pair::Ba => (b, a),
        Pair::Aa => (a, a),
    }
}

/// Full metric bundle for a mesh the axiolid provider produced.
///
/// Only this kernel can report topology: the C ABI shims return a bare
/// double, so their columns stay volume-only and the comparison skips
/// what they cannot supply.
fn axiolid_metrics(mesh: &TriMesh) -> Metrics {
    use axiolid_mesh::component_count;
    let volume = axiolid_volume(mesh);
    let area = mesh
        .indices
        .chunks_exact(3)
        .map(|t| {
            let p = &mesh.positions;
            let (a, b, c) = (p[t[0] as usize], p[t[1] as usize], p[t[2] as usize]);
            (b - a).cross(c - a).length() * 0.5
        })
        .sum::<f64>();
    let mut lo = [f64::INFINITY; 3];
    let mut hi = [f64::NEG_INFINITY; 3];
    for p in &mesh.positions {
        let v = [p.x, p.y, p.z];
        for i in 0..3 {
            lo[i] = lo[i].min(v[i]);
            hi[i] = hi[i].max(v[i]);
        }
    }
    let bounds = if mesh.positions.is_empty() {
        None
    } else {
        Some([lo[0], lo[1], lo[2], hi[0], hi[1], hi[2]])
    };
    Metrics {
        volume,
        area: Some(area),
        bounds,
        euler: Some(euler_of(mesh)),
        components: Some(component_count(mesh)),
        manifold: Some(is_closed_manifold(mesh)),
    }
}

/// Euler characteristic from the half-edge counts.
///
/// Computed here rather than via `genus()` because genus REFUSES a
/// multi-component or open result (kernel #98/#99), and a damaged
/// result is exactly the case this scoring exists to detect. The raw
/// characteristic is always defined.
fn euler_of(mesh: &TriMesh) -> i64 {
    use axiolid_mesh::EdgeAdjacency;
    EdgeAdjacency::build(mesh).euler_characteristic()
}

/// Whether the mesh is a closed two-manifold.
fn is_closed_manifold(mesh: &TriMesh) -> bool {
    use axiolid_mesh::EdgeAdjacency;
    let adjacency = EdgeAdjacency::build(mesh);
    adjacency.boundary_edges().count() == 0 && adjacency.non_manifold_edges().count() == 0
}

fn axiolid_op(a: Obb, b: Obb, op: Op, pair: Pair) -> Option<Metrics> {
    use axiolid_contracts::ExecutionOptions;
    use axiolid_core::{BooleanOperator, Tolerance};
    use axiolid_mesh_boolean_boolmesh::BoolmeshBoolean;
    use axiolid_mesh_boolean_contract::MeshBoolean;

    let (subject, tool) = resolve(a, b, pair);
    let operator = match op {
        Op::Difference => BooleanOperator::Difference,
        Op::Union => BooleanOperator::Union,
        Op::Intersection => BooleanOperator::Intersection,
    };
    let options = ExecutionOptions::new(Tolerance::MILLIMETRE);
    BoolmeshBoolean::new()
        .boolean(
            &axiolid_obb(subject),
            &axiolid_obb(tool),
            operator,
            &options,
        )
        .ok()
        .map(|o| axiolid_metrics(&o.mesh))
}

/// Evaluate one operation with upstream `boolmesh` directly, no provider.
fn raw_op(a: Obb, b: Obb, op: Op, pair: Pair) -> Option<Metrics> {
    use crate::to_manifold_obb;
    use boolmesh::prelude::{compute_boolean, OpType};

    let (subject, tool) = resolve(a, b, pair);
    let kind = match op {
        Op::Difference => OpType::Subtract,
        Op::Union => OpType::Add,
        Op::Intersection => OpType::Intersect,
    };
    let out = compute_boolean(&to_manifold_obb(subject), &to_manifold_obb(tool), kind).ok()?;
    // Reuse the shared divergence-theorem helper so this column is measured the
    // same way as every other, rather than by a second implementation that
    // could disagree for its own reasons.
    let volume = crate::signed_volume(
        out.get_indices()
            .iter()
            .map(|t| [t.x as u32, t.y as u32, t.z as u32]),
        |i| {
            let q = out.ps[i as usize];
            [q.x, q.y, q.z]
        },
    )
    .abs();
    Some(Metrics {
        volume,
        ..Metrics::default()
    })
}

/// Evaluate one operation through a C ABI entry point.
///
/// The C side takes the host as `[min,max]` and the operand as 8 corners, so
/// the swapped case needs the subject's own corners: an OBB subject cannot be
/// expressed as a min/max pair without losing its rotation. Both operands are
/// therefore passed as corners, with the host's AABB supplied only for the
/// axis-aligned `A`.
#[allow(unused_variables)]
fn cpp_op(
    f: unsafe extern "C" fn(*const f64, *const f64, *const f64, i32) -> f64,
    a: Obb,
    b: Obb,
    op: Op,
    pair: Pair,
) -> Option<Metrics> {
    let (subject, tool) = resolve(a, b, pair);
    // The C shim builds its host from min/max, which is only exact for an
    // unrotated subject. Refuse rather than silently squaring off a rotation.
    let sub_aabb = subject.as_aabb()?;
    let operand = flat_cutters(&[tool]);
    let v = unsafe {
        f(
            sub_aabb.min.as_ptr(),
            sub_aabb.max.as_ptr(),
            operand.as_ptr(),
            op as i32,
        )
    };
    // Negative is the shim's unambiguous failure signal.
    if v < 0.0 {
        None
    } else {
        // Volume only: the C ABI returns a bare double, so every
        // topological field stays `None` and is skipped when scoring.
        Some(Metrics {
            volume: v,
            ..Metrics::default()
        })
    }
}

/// Score every kernel on every identity, returning JSON rows.
///
/// A residual near machine epsilon means the kernel satisfied the law to the
/// limit of double precision. A residual of 1e-3 on a unit-scaled problem is a
/// real geometric error, not rounding. `null` means the kernel could not be
/// scored -- refused, failed, or not compiled -- and is deliberately NOT zero,
/// because a kernel that declines everything must not appear flawless.
pub fn report(json: bool) -> Vec<String> {
    let (a, b) = operands();
    let metrics_a = axiolid_metrics(&axiolid_obb(a));
    let metrics_b = axiolid_metrics(&axiolid_obb(b));

    // (label, evaluator). Each closure adapts one kernel to the shared
    // `(Op, Pair) -> Option<Metrics>` signature the identity checker calls.
    type Eval = Box<dyn FnMut(Op, Pair) -> Option<Metrics>>;
    let mut kernels: Vec<(&str, Eval)> = vec![
        ("axiolid", Box::new(move |op, pr| axiolid_op(a, b, op, pr))),
        ("raw_boolmesh", Box::new(move |op, pr| raw_op(a, b, op, pr))),
    ];
    #[cfg(has_manifold)]
    kernels.push((
        "manifold",
        Box::new(move |op, pr| cpp_op(crate::bench_manifold_op, a, b, op, pr)),
    ));
    #[cfg(has_cgal)]
    kernels.push((
        "cgal",
        Box::new(move |op, pr| cpp_op(crate::bench_cgal_op, a, b, op, pr)),
    ));
    #[cfg(has_occt)]
    kernels.push((
        "occt",
        Box::new(move |op, pr| cpp_op(crate::bench_occt_op, a, b, op, pr)),
    ));

    if !json {
        println!("\n\nAlgebraic exactness -- residual of each identity, relative to vol(A)+vol(B)");
        println!("{}", "-".repeat(80));
        println!("A = axis-aligned slab, B = 30 deg rotated box straddling A's face\n");
        print!("{:<22}", "identity");
        for (name, _) in &kernels {
            print!("{name:>14}");
        }
        println!();
    }

    let mut rows = Vec::new();
    for identity in IDENTITIES.iter() {
        if !json {
            print!("{:<22}", identity.name);
        }
        let mut cells = Vec::new();
        for (name, eval) in kernels.iter_mut() {
            let verdict = score(identity, &metrics_a, &metrics_b, eval);
            let r = verdict.as_ref().and_then(super::ops::Verdict::worst);
            let which = verdict.as_ref().and_then(super::ops::Verdict::worst_metric);
            if !json {
                match (r, which) {
                    // Name the metric that failed: "which invariant broke"
                    // is the diagnostic, not the magnitude alone.
                    (Some(v), Some(m)) if v > 1e-12 => print!("{:>14}", format!("{v:.0e} {m}")),
                    (Some(v), _) => print!("{v:>14.2e}"),
                    _ => print!("{:>14}", "n/a"),
                }
            }
            cells.push(match (r, which) {
                (Some(v), Some(m)) => {
                    format!("\"{name}\":{{\"residual\":{v:.6e},\"metric\":\"{m}\"}}")
                }
                _ => format!("\"{name}\":null"),
            });
        }
        if !json {
            println!();
        }
        rows.push(format!(
            "{{\"identity\":\"{}\",\"law\":\"{}\",{}}}",
            identity.name,
            identity.law,
            cells.join(",")
        ));
    }
    if !json {
        println!("\n  law reference:");
        for identity in IDENTITIES.iter() {
            println!("    {:<22} {}", identity.name, identity.law);
        }
    }
    rows
}
