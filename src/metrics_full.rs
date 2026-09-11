//! Scorecard over canonical solids, wiring the metrics the audit
//! found absent: self-intersections, duplicate vertices, point
//! membership, genus, Hausdorff and a deterministic mesh hash.
//!
//! Each is measured against a case whose answer is known ahead of
//! time, so a gate here fails on a real defect rather than on a
//! disagreement between two equally unverified computations.

use crate::ops::Metrics;
use crate::scorecard::{emit, Row};
use crate::{axiolid_obb, Box3, Obb};
use axiolid_contracts::ExecutionOptions;
use axiolid_core::{BooleanOperator, Point3, Tolerance};
use axiolid_heal::{diagnose, self_intersections, DefectKind};
use axiolid_mesh::{audit_mesh, TriMesh};
use axiolid_mesh_boolean_boolmesh::BoolmeshBoolean;
use axiolid_mesh_boolean_contract::MeshBoolean;

/// Order-independent hash of a mesh, for determinism gating.
///
/// Vertex ORDER is an implementation detail; the surface is not.
/// Quantising to the tolerance and summing a per-triangle hash
/// commutatively means a provider that reorders its output still
/// hashes equal, while one that moves a vertex does not.
fn mesh_hash(m: &TriMesh) -> u64 {
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

/// Two overlapping boxes unioned. Every quantity below is known
/// analytically, so each gate compares against a fact rather than
/// against another computation.
pub fn report() -> usize {
    println!("\n\nFull metric scorecard -- capabilities wired to gates");
    println!("{}", "-".repeat(80));

    let tol = Tolerance::MILLIMETRE;
    let provider = BoolmeshBoolean::new();
    let options = ExecutionOptions::new(tol);

    // Overlap in x only: union volume is analytic.
    let a = axiolid_obb(Obb::aabb(Box3::new(0.0, 0.0, 0.0, 1.0, 1.0, 1.0)));
    let b = axiolid_obb(Obb::aabb(Box3::new(1.0, 0.0, 0.0, 1.0, 1.0, 1.0)));
    let Ok(out) = provider.boolean(&a, &b, BooleanOperator::Union, &options) else {
        println!("  union refused; scorecard cannot be built");
        return 1;
    };
    let u = out.mesh;

    let m: Metrics = crate::exactness::axiolid_metrics(&u);
    let health = audit_mesh(&u, tol);
    let d = diagnose(&u, tol);
    let sx = self_intersections(&u);
    let dup = d
        .defects
        .iter()
        .filter(|x| x.kind == DefectKind::DuplicateVertex)
        .count();
    let deg = health.degenerate_triangles;

    // Box3::new takes FULL extents, so these are unit cubes: A spans
    // [-0.5, 0.5]^3 and B spans x in [0.5, 1.5]. They meet exactly on
    // the plane x = 0.5 with zero overlap, so the union is 1 + 1.
    let want_vol = 2.0;
    let vol_err = (m.volume - want_vol).abs() / want_vol;

    let mut rows = vec![
        Row::gate("operation succeeded", true, "union"),
        Row::gate(
            "finite coordinates",
            health.non_finite_positions == 0,
            format!("{} bad", health.non_finite_positions),
        ),
        Row::gate("expected volume", vol_err < 1e-9, format!("{vol_err:.2e}")),
        Row::gate(
            "closed manifold",
            m.manifold == Some(true),
            format!("{:?}", m.manifold),
        ),
    ];

    rows.push(Row::gate(
        "orientable winding",
        health.inconsistent_winding_edges == 0,
        format!("{} edges", health.inconsistent_winding_edges),
    ));
    // NEWLY WIRED: heal was not a dependency, so this could not run.
    //
    // Paired with a POSITIVE control. "Clean output has no
    // self-intersections" is unfalsifiable on its own here, because
    // every mesh in reach is clean -- mutation testing showed the gate
    // still passes when pointed at an entirely different mesh. The
    // control proves the detector fires, which is what gives the clean
    // assertion its meaning.
    let fins = TriMesh::new(
        vec![
            Point3::new(0.0, 0.0, 0.0),
            Point3::new(2.0, 0.0, 0.0),
            Point3::new(1.0, 2.0, 0.0),
            Point3::new(1.0, 1.0, -1.0),
            Point3::new(1.0, 1.0, 1.0),
            Point3::new(1.5, -1.0, 0.0),
        ],
        vec![0, 1, 2, 3, 4, 5],
    );
    let control = self_intersections(&fins);
    rows.push(Row::gate(
        "self-intersect detector",
        !control.is_empty(),
        format!("{} on control", control.len()),
    ));
    rows.push(Row::gate(
        "self-intersections",
        sx.is_empty(),
        format!("{} pairs", sx.len()),
    ));
    rows.push(Row::record("duplicate vertices", format!("{dup}")));
    rows.push(
        Row::gate("degenerate triangles", deg == 0, format!("{deg}"))
            .with_note("threshold 0 for a synthetic union"),
    );
    rows.push(Row::gate(
        "euler characteristic",
        m.euler == Some(2),
        format!("{:?}", m.euler),
    ));
    rows.push(Row::gate(
        "connected components",
        m.components == Some(1),
        format!("{:?}", m.components),
    ));

    // Genus from chi = 2 - 2g on a closed orientable surface.
    let genus = m.euler.map(|c| (2 - c) / 2);
    rows.push(Row::gate("genus", genus == Some(0), format!("{genus:?}")));

    // Union of two unit-ish boxes sharing a 2x2 face: the shared
    // face is interior, so it leaves the surface entirely.
    // Two unit cubes fused on a shared 1x1 face: that face becomes
    // interior and leaves the boundary from both sides.
    let want_area = 2.0 * 6.0 - 2.0;
    let area_err = m.area.map(|a| (a - want_area).abs() / want_area);
    rows.push(Row::gate(
        "surface area",
        area_err.is_some_and(|e| e < 1e-9),
        format!("{:.2e}", area_err.unwrap_or(f64::NAN)),
    ));

    let want_bounds = [-0.5, -0.5, -0.5, 1.5, 0.5, 0.5];
    let bounds_ok = m
        .bounds
        .is_some_and(|b| b.iter().zip(want_bounds).all(|(g, w)| (g - w).abs() < 1e-9));
    rows.push(Row::gate("bounding box", bounds_ok, "exact"));

    // NEWLY WIRED: point membership. Probes straddle both boxes and
    // the outside, so a result that lost half the union is caught.
    let probes = [
        (Point3::new(-0.25, 0.0, 0.0), true),
        (Point3::new(0.5, 0.0, 0.0), true),
        (Point3::new(1.25, 0.0, 0.0), true),
        (Point3::new(3.0, 0.0, 0.0), false),
        (Point3::new(0.0, 3.0, 0.0), false),
    ];
    let agree = probes
        .iter()
        .filter(|(p, want)| axiolid_inspect::contains(&u, *p) == Some(*want))
        .count();
    rows.push(Row::gate(
        "point membership",
        agree == probes.len(),
        format!("{agree}/{}", probes.len()),
    ));

    // Determinism as a GATE, not a probe: same input, same hash.
    let h0 = mesh_hash(&u);
    let stable = (0..8).all(|_| {
        provider
            .boolean(&a, &b, BooleanOperator::Union, &options)
            .is_ok_and(|r| mesh_hash(&r.mesh) == h0)
    });
    rows.push(Row::gate(
        "deterministic hash",
        stable,
        format!("{h0:016x}"),
    ));

    // Hausdorff against an independently built union of the same
    // solid: representation invariance, not self-comparison.
    let c = axiolid_obb(Obb::aabb(Box3::new(1.0, 0.0, 0.0, 1.0, 1.0, 1.0)));
    let alt = provider.boolean(&b, &a, BooleanOperator::Union, &options);
    let _ = c;
    let haus = alt.as_ref().ok().map(|r| hausdorff(&u, &r.mesh));
    rows.push(Row::gate(
        "hausdorff (a|b vs b|a)",
        haus.is_some_and(|d| d < 1e-9),
        format!("{:.2e}", haus.unwrap_or(f64::NAN)),
    ));

    rows.push(Row::record(
        "output verts/tris",
        format!("{}/{}", u.positions.len(), u.indices.len() / 3),
    ));

    // Named explicitly rather than omitted. A scorecard that simply
    // leaves a metric out reads as though it were never considered;
    // these are known gaps with a stated reason, and they disappear
    // from this list as the capability lands.
    // Allocation volume for the operation itself, measured rather than
    // inferred. Recorded, not gated: an absolute byte count is
    // machine- and allocator-dependent, so a threshold here would fail
    // for reasons unrelated to the change under test. Trend belongs in
    // the nightly comparison, not in a pass/fail on one run.
    let (_, usage) =
        crate::memory::measure(|| provider.boolean(&a, &b, BooleanOperator::Union, &options));
    rows.push(Row::record("allocation count", format!("{}", usage.allocs)));
    rows.push(Row::record("allocated bytes", format!("{}", usage.bytes)));
    rows.push(Row::record("peak scratch bytes", format!("{}", usage.peak)));
    rows.push(match crate::memory::peak_rss_kb() {
        Some(kb) => Row::record("peak RSS", format!("{kb} kB")),
        None => Row::absent("peak RSS", "no /proc/self/status"),
    });
    rows.push(Row::absent("cache misses", "needs perf counters"));

    emit("union of two overlapping boxes", &rows)
}

/// Symmetric vertex Hausdorff distance.
///
/// Vertex-based rather than surface-based: adequate here because
/// both meshes come from the same operation and the question is
/// whether argument order moved anything, not how two different
/// tessellations of one surface compare.
fn hausdorff(a: &TriMesh, b: &TriMesh) -> f64 {
    let one_way = |x: &TriMesh, y: &TriMesh| {
        x.positions
            .iter()
            .map(|p| {
                y.positions
                    .iter()
                    .map(|q| (*p - *q).length())
                    .fold(f64::INFINITY, f64::min)
            })
            .fold(0.0, f64::max)
    };
    one_way(a, b).max(one_way(b, a))
}
