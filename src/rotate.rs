//! Rotation invariance: a rigid transform must not change the answer.
//!
//! `scale.rs` covers translation and scaling. This covers the third rigid
//! motion, which is the one that actually stresses a boolean: rotation makes
//! every operand face non-axis-aligned, so no analytic fast path applies and
//! the general solver runs on coordinates that are no longer exact in binary.
//!
//! The method is a round trip. Rotate both operands by R, run the boolean,
//! then map the result back through R inverse and compare against the answer
//! computed with no rotation at all. A rigid motion preserves volume, area,
//! genus, component count and every internal distance, so any difference is
//! the kernel reacting to coordinate representation rather than to shape.
//!
//! The angles are chosen to sit on the hard cases: 0 and 90 degrees are
//! exactly representable and turn the operands back into axis-aligned boxes,
//! while 89.999999 and 90.000001 are a hair off and produce faces that are
//! nearly-but-not-quite coincident -- the configuration that separates a
//! robust kernel from one that merely rounds well.

use axiolid_contracts::ExecutionOptions;
use axiolid_core::{BooleanOperator, Point3, Scalar, SpaceFrame, Tolerance, Vec3};
use axiolid_mesh::{component_count, EdgeAdjacency, TriMesh};
use axiolid_mesh_boolean_boolmesh::BoolmeshBoolean;
use axiolid_mesh_boolean_contract::MeshBoolean;

/// A rotation about a normalised axis by `angle` radians, as a `SpaceFrame`.
///
/// Built by Rodrigues rotation of the world axes. `SpaceFrame::new` re-checks
/// orthonormality and handedness, so a construction bug surfaces here rather
/// than as a mysterious volume error later.
fn rotation(axis: [Scalar; 3], angle: Scalar) -> SpaceFrame {
    let n = (axis[0] * axis[0] + axis[1] * axis[1] + axis[2] * axis[2]).sqrt();
    let k = Vec3::new(axis[0] / n, axis[1] / n, axis[2] / n);
    let (s, c) = angle.sin_cos();
    // Rodrigues: v cos + (k x v) sin + k (k . v)(1 - cos).
    let rot = |v: Vec3| -> Vec3 {
        let kv = k.cross(v);
        let kd = k.dot(v);
        Vec3::new(
            v.x * c + kv.x * s + k.x * kd * (1.0 - c),
            v.y * c + kv.y * s + k.y * kd * (1.0 - c),
            v.z * c + kv.z * s + k.z * kd * (1.0 - c),
        )
    };
    let x = rot(Vec3::new(1.0, 0.0, 0.0));
    let y = rot(Vec3::new(0.0, 1.0, 0.0));
    let z = rot(Vec3::new(0.0, 0.0, 1.0));
    // Tolerance::MILLIMETRE for the orthonormality check: Rodrigues on a unit
    // axis is orthonormal to a few ulps, and ZERO would reject on rounding.
    SpaceFrame::new(Point3::new(0.0, 0.0, 0.0), x, y, z, Tolerance::MILLIMETRE)
        .expect("Rodrigues rotation of the world axes is orthonormal")
}

/// Map every vertex through the frame into world space.
fn rotate(mesh: &TriMesh, f: &SpaceFrame) -> TriMesh {
    let p = mesh
        .positions
        .iter()
        .map(|v| f.to_world(Vec3::new(v.x, v.y, v.z)))
        .collect();
    TriMesh::new(p, mesh.indices.clone())
}

/// Map every vertex back through the frame. Exact inverse of `rotate`.
fn unrotate(mesh: &TriMesh, f: &SpaceFrame) -> TriMesh {
    let p = mesh
        .positions
        .iter()
        .map(|v| {
            let l = f.to_local(*v);
            Point3::new(l.x, l.y, l.z)
        })
        .collect();
    TriMesh::new(p, mesh.indices.clone())
}

/// Six times the signed volume, about the centroid (kernel #99).
fn six_volume(m: &TriMesh) -> Scalar {
    if m.indices.is_empty() {
        return 0.0;
    }
    let n = m.positions.len() as Scalar;
    let (mut cx, mut cy, mut cz) = (0.0, 0.0, 0.0);
    for p in &m.positions {
        cx += p.x;
        cy += p.y;
        cz += p.z;
    }
    let (cx, cy, cz) = (cx / n, cy / n, cz / n);
    let s = |i: u32| {
        let p = m.positions[i as usize];
        Vec3::new(p.x - cx, p.y - cy, p.z - cz)
    };
    m.indices
        .chunks_exact(3)
        .map(|t| s(t[0]).dot(s(t[1]).cross(s(t[2]))))
        .sum()
}

/// Total surface area. Rotation preserves it exactly in real arithmetic.
fn area(m: &TriMesh) -> Scalar {
    m.indices
        .chunks_exact(3)
        .map(|t| {
            let a = m.positions[t[0] as usize];
            let b = m.positions[t[1] as usize];
            let c = m.positions[t[2] as usize];
            (b - a).cross(c - a).length() * 0.5
        })
        .sum()
}

/// Euler characteristic V - E + F, counting only referenced vertices.
fn euler(m: &TriMesh) -> i64 {
    EdgeAdjacency::build(m).euler_characteristic()
}

/// The largest distance between any two vertices, a rotation invariant.
///
/// Sampled rather than exhaustive: the full pairwise set is O(n^2) and this
/// runs on meshes of a few thousand vertices. A stride keeps it honest at a
/// bounded cost -- it is a witness for gross distortion, not a proof.
fn spread(m: &TriMesh) -> Scalar {
    let step = (m.positions.len() / 64).max(1);
    let pts: Vec<_> = m.positions.iter().step_by(step).collect();
    let mut worst: Scalar = 0.0;
    for (i, a) in pts.iter().enumerate() {
        for b in &pts[i + 1..] {
            worst = worst.max((**b - **a).length());
        }
    }
    worst
}

/// Worst distance from a vertex of `m` to the nearest vertex of `want`.
///
/// This is the ONLY metric here that is not itself rotation-invariant, and it
/// is what makes the round trip meaningful: volume, area, genus, components
/// and spread are all preserved by ANY rotation, so comparing them after a
/// round trip cannot tell a correct inverse from a missing one. Comparing
/// positions can. A mutation test that skipped `unrotate` passed every other
/// column and was caught only by this one.
fn max_vertex_drift(m: &TriMesh, want: &TriMesh) -> Scalar {
    let mut worst: Scalar = 0.0;
    for p in &m.positions {
        let mut best = Scalar::INFINITY;
        for q in &want.positions {
            best = best.min((*q - *p).length());
        }
        worst = worst.max(best);
    }
    worst
}

/// Operands: a unit cube and an overlapping cube offset along the diagonal.
///
/// Deliberately axis-aligned at zero rotation, so the 0 and 90 degree cases
/// reproduce the axis-aligned configuration exactly and any other angle is a
/// genuine departure from it.
fn operands() -> (TriMesh, TriMesh) {
    (
        boxx([0.0, 0.0, 0.0], [1.0, 1.0, 1.0]),
        boxx([0.5, 0.5, 0.5], [1.5, 1.5, 1.5]),
    )
}

fn boxx(mn: [Scalar; 3], mx: [Scalar; 3]) -> TriMesh {
    let ([x0, y0, z0], [x1, y1, z1]) = (mn, mx);
    let p = vec![
        Point3::new(x0, y0, z0),
        Point3::new(x1, y0, z0),
        Point3::new(x1, y1, z0),
        Point3::new(x0, y1, z0),
        Point3::new(x0, y0, z1),
        Point3::new(x1, y0, z1),
        Point3::new(x1, y1, z1),
        Point3::new(x0, y1, z1),
    ];
    let i = vec![
        0, 2, 1, 0, 3, 2, 4, 5, 6, 4, 6, 7, 0, 1, 5, 0, 5, 4, 3, 7, 6, 3, 6, 2, 0, 4, 7, 0, 7, 3,
        1, 2, 6, 1, 6, 5,
    ];
    TriMesh::new(p, i)
}

/// One row of the sweep.
struct Row {
    label: String,
    /// Relative volume error of the round trip against the unrotated answer.
    vol: Scalar,
    /// Relative area error.
    area: Scalar,
    /// Euler characteristic of the rotated result.
    euler: i64,
    /// Component count of the rotated result.
    comps: usize,
    /// Relative error in the vertex spread.
    spread: Scalar,
    /// Worst vertex distance from the unrotated reference, in model units.
    drift: Scalar,
    /// Set when the kernel refused rather than returning a solid.
    refused: Option<String>,
}

/// The angle sweep, in degrees, plus fixed arbitrary 3D axes.
///
/// 89.999999 / 90 / 90.000001 is the trio the spec calls out: exactly 90 is
/// representable and restores axis alignment, while a hair either side does
/// not, so a kernel that special-cases axis-aligned input shows a
/// discontinuity across the three.
fn cases() -> Vec<(String, [Scalar; 3], Scalar)> {
    let z = [0.0, 0.0, 1.0];
    let mut v: Vec<(String, [Scalar; 3], Scalar)> = Vec::new();
    for d in [0.0_f64, 30.0, 45.0, 89.999_999, 90.0, 90.000_001] {
        v.push((format!("z {d:>12}"), z, d.to_radians()));
    }
    // Arbitrary fixed axes: not axis-aligned, not a nice fraction of pi, and
    // hard-coded so the suite is deterministic rather than randomised.
    v.push(("diag 30".into(), [1.0, 1.0, 1.0], 30.0_f64.to_radians()));
    v.push(("diag 45".into(), [1.0, 1.0, 1.0], 45.0_f64.to_radians()));
    v.push((
        "tilt 60".into(),
        [0.267_261, 0.534_522, 0.801_784],
        60.0_f64.to_radians(),
    ));
    v.push((
        "skew 120".into(),
        [0.371_391, -0.557_086, 0.742_781],
        120.0_f64.to_radians(),
    ));
    v
}

/// Run the sweep. Returns the number of gate violations.
#[must_use]
pub fn report() -> usize {
    let provider = BoolmeshBoolean;
    let opts = ExecutionOptions::new(Tolerance::MILLIMETRE);
    let (a, b) = operands();

    // The reference: the same boolean with no rotation at all. Every rotated
    // run is compared against THIS, not against an analytic constant, so the
    // test is invariance rather than accuracy.
    let base = provider
        .boolean(&a, &b, BooleanOperator::Difference, &opts)
        .expect("the unrotated difference must succeed")
        .mesh;
    let (bv, ba, be, bc, bs) = (
        six_volume(&base),
        area(&base),
        euler(&base),
        component_count(&base),
        spread(&base),
    );

    let mut rows: Vec<Row> = Vec::new();
    for (label, axis, angle) in cases() {
        let f = rotation(axis, angle);
        let (ra, rb) = (rotate(&a, &f), rotate(&b, &f));
        match provider.boolean(&ra, &rb, BooleanOperator::Difference, &opts) {
            Err(e) => rows.push(Row {
                label,
                vol: 0.0,
                area: 0.0,
                euler: 0,
                comps: 0,
                spread: 0.0,
                drift: 0.0,
                refused: Some(format!("{e}")),
            }),
            Ok(out) => {
                // Back to the original frame before comparing. Without this
                // the volume would match but the vertex positions would not,
                // and a rotation-dependent DISTORTION would be invisible.
                let back = unrotate(&out.mesh, &f);
                let rel = |got: Scalar, want: Scalar| {
                    if want.abs() > 0.0 {
                        (got - want).abs() / want.abs()
                    } else {
                        got.abs()
                    }
                };
                rows.push(Row {
                    label,
                    vol: rel(six_volume(&back), bv),
                    area: rel(area(&back), ba),
                    euler: euler(&back),
                    comps: component_count(&back),
                    spread: rel(spread(&back), bs),
                    drift: max_vertex_drift(&back, &base),
                    refused: None,
                });
            }
        }
    }

    println!();
    println!("Rotation invariance -- rigid transform, round trip, compare to unrotated");
    println!("{}", "-".repeat(100));
    println!("A rigid motion preserves volume, area, genus, components and distances.");
    println!("Result is mapped back through the inverse rotation before comparison.");
    println!(
        "Reference: chi={be}, components={bc}, volume={:.6}",
        bv / 6.0
    );
    println!();
    println!(
        "  {:<16}{:>12}{:>12}{:>8}{:>8}{:>12}  verdict",
        "case", "vol err", "area err", "chi", "comps", "drift"
    );

    let mut faults = 0usize;
    for r in &rows {
        if let Some(why) = &r.refused {
            // A rigid motion cannot make a valid operand invalid. Refusing
            // one is therefore a real failure, not a conditioning excuse.
            println!(
                "  {:<16}{:>12}{:>12}{:>8}{:>8}{:>12}  !! REFUSED {why}",
                r.label, "-", "-", "-", "-", "-"
            );
            faults += 1;
            continue;
        }
        // 1e-9 relative: rotation introduces irrational coordinates, so the
        // result is not bit-identical. It must still agree to well beyond
        // any plausible modelling tolerance.
        let bad_v = r.vol > 1e-9;
        let bad_a = r.area > 1e-9;
        let bad_s = r.spread > 1e-9;
        // Absolute, in model units: the operands are unit-sized, so 1e-9 is
        // a nanometre on a metre. This is the column that proves the inverse
        // rotation actually ran.
        let bad_d = r.drift > 1e-9;
        // Topology is integer-valued: it either matches or the kernel
        // changed the shape. No tolerance applies.
        let bad_t = r.euler != be || r.comps != bc;
        let mut notes: Vec<String> = Vec::new();
        if bad_v {
            notes.push(format!("VOL {:.1e}", r.vol));
        }
        if bad_a {
            notes.push(format!("AREA {:.1e}", r.area));
        }
        if bad_s {
            notes.push(format!("SPREAD {:.1e}", r.spread));
        }
        if bad_d {
            notes.push(format!("DRIFT {:.1e}", r.drift));
        }
        if bad_t {
            notes.push(format!("TOPOLOGY chi={} comps={}", r.euler, r.comps));
        }
        let verdict = if notes.is_empty() {
            "ok".to_string()
        } else {
            faults += 1;
            format!("!! {}", notes.join("; "))
        };
        println!(
            "  {:<16}{:>12.2e}{:>12.2e}{:>8}{:>8}{:>12.2e}  {verdict}",
            r.label, r.vol, r.area, r.euler, r.comps, r.drift
        );
    }
    println!();
    if faults == 0 {
        println!("  rotation is invariant across every angle and axis tested.");
    } else {
        println!("  {faults} rotation case(s) changed a rigid-motion invariant.");
    }
    faults
}
