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

use crate::ops::{identities, score, Kernel, Metrics, Op, Verdict};
use crate::{axiolid_obb, axiolid_volume, Box3, Obb};
use axiolid_mesh::TriMesh;

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

/// The three operands every kernel is scored on.
///
/// `A` is a wall-like slab. `B` is rotated 30 degrees in plan and
/// straddles A face rather than sitting cleanly inside or outside, so
/// the intersection is a genuine wedge with near-parallel faces. `C`
/// is a third box overlapping BOTH, which the associativity laws need:
/// a C disjoint from A or B would make both groupings trivially equal
/// and prove nothing.
fn operands() -> (Obb, Obb, Obb) {
    let a = Obb::aabb(Box3::new(0.0, 0.0, 1.5, 4.0, 0.4, 3.0));
    let b = Obb {
        centre: [1.0, 0.1, 1.5],
        half: [0.6, 0.5, 0.7],
        angle: std::f64::consts::FRAC_PI_6,
    };
    let c = Obb {
        centre: [1.4, 0.15, 1.8],
        half: [0.7, 0.45, 0.6],
        angle: std::f64::consts::FRAC_PI_4,
    };
    (a, b, c)
}

/// Axiolid provider as an expression-tree kernel.
///
/// `Solid = TriMesh`, so an intermediate result feeds the next
/// operation as real geometry rather than as a measurement.
struct Axiolid {
    operands: [TriMesh; 3],
}

impl Kernel for Axiolid {
    type Solid = TriMesh;

    fn operand(&mut self, index: usize) -> Option<TriMesh> {
        self.operands.get(index).cloned()
    }

    fn apply(&mut self, op: Op, a: &TriMesh, b: &TriMesh) -> Option<TriMesh> {
        use axiolid_contracts::ExecutionOptions;
        use axiolid_core::{BooleanOperator, Tolerance};
        use axiolid_mesh_boolean_boolmesh::BoolmeshBoolean;
        use axiolid_mesh_boolean_contract::MeshBoolean;

        let operator = match op {
            Op::Difference => BooleanOperator::Difference,
            Op::Union => BooleanOperator::Union,
            Op::Intersection => BooleanOperator::Intersection,
        };
        let options = ExecutionOptions::new(Tolerance::MILLIMETRE);
        BoolmeshBoolean::new()
            .boolean(a, b, operator, &options)
            .ok()
            .map(|o| o.mesh)
    }

    fn measure(&mut self, solid: &TriMesh) -> Metrics {
        axiolid_metrics(solid)
    }
}

/// Upstream `boolmesh` directly, no provider wrapper.
///
/// Native solid is `Manifold`, so nesting costs no conversion.
struct RawBoolmesh {
    operands: [crate::Obb; 3],
}

impl Kernel for RawBoolmesh {
    type Solid = boolmesh::prelude::Manifold;

    fn operand(&mut self, index: usize) -> Option<Self::Solid> {
        self.operands.get(index).map(|o| crate::to_manifold_obb(*o))
    }

    fn apply(&mut self, op: Op, a: &Self::Solid, b: &Self::Solid) -> Option<Self::Solid> {
        use boolmesh::prelude::{compute_boolean, OpType};
        let kind = match op {
            Op::Difference => OpType::Subtract,
            Op::Union => OpType::Add,
            Op::Intersection => OpType::Intersect,
        };
        compute_boolean(a, b, kind).ok()
    }

    /// Volume only, deliberately. Measuring topology here would need a
    /// second half-edge implementation over `Manifold`, and a column
    /// that disagreed with the axiolid one for its own reasons would
    /// be worse than an honest absence.
    fn measure(&mut self, solid: &Self::Solid) -> Metrics {
        let volume = crate::signed_volume(
            solid
                .get_indices()
                .iter()
                .map(|t| [t.x as u32, t.y as u32, t.z as u32]),
            |i| {
                let q = solid.ps[i as usize];
                [q.x, q.y, q.z]
            },
        )
        .abs();
        Metrics {
            volume,
            ..Metrics::default()
        }
    }
}

/// Score every kernel on every identity, returning JSON rows.
///
/// A residual near machine epsilon means the law held to the limit of
/// double precision. `null` means the kernel could not be scored --
/// refused, failed, or not compiled -- and is deliberately NOT zero,
/// because a kernel that declines everything must not look flawless.
///
/// The C ABI kernels are absent from this table by construction: their
/// entry point takes OBB corners and returns a bare double, so it
/// cannot accept an intermediate RESULT as an operand. Nested laws
/// like associativity are unreachable through that interface.
pub fn report(json: bool) -> Vec<String> {
    let (a, b, c) = operands();
    let table = identities();

    if !json {
        println!();
        println!();
        println!("Algebraic exactness -- residual of each identity");
        println!("{}", "-".repeat(78));
        println!("A = slab, B = 30 deg rotated box straddling A, C = 45 deg box over both");
        println!();
        println!("{:<22}{:>16}{:>16}", "identity", "axiolid", "raw_boolmesh");
    }

    let mut rows = Vec::new();
    for identity in &table {
        let mut ax = Axiolid {
            operands: [axiolid_obb(a), axiolid_obb(b), axiolid_obb(c)],
        };
        let mut raw = RawBoolmesh {
            operands: [a, b, c],
        };
        let va = score(identity, &mut ax);
        let vr = score(identity, &mut raw);

        if !json {
            print!("{:<22}", identity.name);
            for v in [&va, &vr] {
                match v.as_ref().and_then(Verdict::worst) {
                    Some(r) => {
                        let m = v.as_ref().and_then(Verdict::worst_metric).unwrap_or("");
                        print!("{:>16}", format!("{r:.1e} {m}"));
                    }
                    None => print!("{:>16}", "n/a"),
                }
            }
            println!();
        }

        let cell = |v: &Option<Verdict>| match v.as_ref().and_then(Verdict::worst) {
            Some(r) => {
                let m = v.as_ref().and_then(Verdict::worst_metric).unwrap_or("");
                format!("{{\"residual\":{r:.6e},\"metric\":\"{m}\"}}")
            }
            None => "null".to_string(),
        };
        rows.push(format!(
            "{{\"identity\":\"{}\",\"law\":\"{}\",\"axiolid\":{},\"raw_boolmesh\":{}}}",
            identity.name,
            identity.law,
            cell(&va),
            cell(&vr)
        ));
    }

    if !json {
        println!();
        println!("  law reference:");
        for identity in &table {
            println!("    {:<22} {}", identity.name, identity.law);
        }
    }
    rows
}
