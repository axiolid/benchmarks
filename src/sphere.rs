//! Sphere-sphere boolean scaling.
//!
//! Every other workload here is box-based: axis-aligned or rotated hexahedra,
//! whose intersections are coplanar or axis-parallel. This is the inverse
//! case. Two overlapping spheres meet in a circle that no input plane
//! contains, so every intersection is a generic triangle-triangle crossing and
//! the intersection curve is a loop of new vertices. It stresses the broad
//! phase and the intersection curve tracer, neither of which the wall
//! workloads exercise hard.
//!
//! # The oracle
//!
//! NOT the closed-form lens volume. A tessellated sphere is an inscribed
//! polyhedron whose volume is below the ideal sphere's by an amount that
//! depends on tessellation density, so comparing against the smooth lens
//! would confound tessellation error with kernel error -- and the error would
//! shrink as density rises, which reads exactly like a kernel getting more
//! accurate.
//!
//! Inclusion-exclusion holds exactly for the polyhedra actually passed in, at
//! any density:
//!
//! ```text
//! |A u B| + |A n B| = |A| + |B|
//! |A - B| = |A| - |A n B|
//! |B - A| = |B| - |A n B|
//! ```
//!
//! So all four operations are checked against each other with no reference to
//! the ideal sphere at all.

use axiolid_core::Point3;
use axiolid_mesh::TriMesh;
use std::collections::HashMap;

/// A closed icosphere: subdivided icosahedron, projected onto the sphere.
///
/// Icosahedral rather than UV: a UV sphere clusters triangles at the poles,
/// so "triangle count" would not mean "uniform resolution" and the two
/// operands would disagree about where detail sits. Subdividing an
/// icosahedron `n` times gives 20 * 4^n near-equilateral triangles of nearly
/// equal area.
///
/// Vertices are shared through an edge-midpoint cache, so the result is a
/// closed two-manifold rather than a triangle soup -- every kernel here
/// refuses an open shell, so this must be closed by construction.
pub fn icosphere(center: [f64; 3], radius: f64, subdivisions: u32) -> TriMesh {
    let t = (1.0 + 5.0f64.sqrt()) / 2.0;
    let mut verts: Vec<[f64; 3]> = vec![
        [-1.0, t, 0.0],
        [1.0, t, 0.0],
        [-1.0, -t, 0.0],
        [1.0, -t, 0.0],
        [0.0, -1.0, t],
        [0.0, 1.0, t],
        [0.0, -1.0, -t],
        [0.0, 1.0, -t],
        [t, 0.0, -1.0],
        [t, 0.0, 1.0],
        [-t, 0.0, -1.0],
        [-t, 0.0, 1.0],
    ];
    let mut faces: Vec<[u32; 3]> = vec![
        [0, 11, 5],
        [0, 5, 1],
        [0, 1, 7],
        [0, 7, 10],
        [0, 10, 11],
        [1, 5, 9],
        [5, 11, 4],
        [11, 10, 2],
        [10, 7, 6],
        [7, 1, 8],
        [3, 9, 4],
        [3, 4, 2],
        [3, 2, 6],
        [3, 6, 8],
        [3, 8, 9],
        [4, 9, 5],
        [2, 4, 11],
        [6, 2, 10],
        [8, 6, 7],
        [9, 8, 1],
    ];

    for _ in 0..subdivisions {
        // Midpoints are cached by ORDERED edge key so the two faces sharing an
        // edge get the same new vertex. Without this the shell splits into
        // unwelded triangles and every kernel refuses it.
        let mut midpoint: HashMap<(u32, u32), u32> = HashMap::new();
        let mut next: Vec<[u32; 3]> = Vec::with_capacity(faces.len() * 4);
        for f in &faces {
            let mut mid = [0u32; 3];
            for e in 0..3 {
                let (a, b) = (f[e], f[(e + 1) % 3]);
                let key = (a.min(b), a.max(b));
                mid[e] = *midpoint.entry(key).or_insert_with(|| {
                    let (pa, pb) = (verts[a as usize], verts[b as usize]);
                    verts.push([
                        (pa[0] + pb[0]) * 0.5,
                        (pa[1] + pb[1]) * 0.5,
                        (pa[2] + pb[2]) * 0.5,
                    ]);
                    (verts.len() - 1) as u32
                });
            }
            next.push([f[0], mid[0], mid[2]]);
            next.push([f[1], mid[1], mid[0]]);
            next.push([f[2], mid[2], mid[1]]);
            next.push([mid[0], mid[1], mid[2]]);
        }
        faces = next;
    }

    // Project every vertex onto the sphere only at the end: projecting during
    // subdivision would move shared vertices twice and open seams.
    let positions: Vec<Point3> = verts
        .iter()
        .map(|v| {
            let len = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
            let k = radius / len;
            Point3::new(
                center[0] + v[0] * k,
                center[1] + v[1] * k,
                center[2] + v[2] * k,
            )
        })
        .collect();
    let indices: Vec<u32> = faces.iter().flat_map(|f| [f[0], f[1], f[2]]).collect();
    TriMesh::new(positions, indices)
}

/// Volume of a closed triangle mesh by the divergence theorem.
pub fn mesh_volume(mesh: &TriMesh) -> f64 {
    let mut sum = 0.0;
    for t in mesh.indices.chunks_exact(3) {
        let a = mesh.positions[t[0] as usize];
        let b = mesh.positions[t[1] as usize];
        let c = mesh.positions[t[2] as usize];
        sum += a.x * (b.y * c.z - b.z * c.y) - a.y * (b.x * c.z - b.z * c.x)
            + a.z * (b.x * c.y - b.y * c.x);
    }
    (sum / 6.0).abs()
}

use crate::best_of;
use axiolid_contracts::ExecutionOptions;
use axiolid_core::BooleanOperator;
use axiolid_core::Tolerance;
use axiolid_mesh_boolean_boolmesh::BoolmeshBoolean;
use axiolid_mesh_boolean_contract::MeshBoolean;

#[cfg(has_manifold)]
extern "C" {
    fn bench_manifold_mesh_op(
        verts_a: *const f64,
        nverts_a: i32,
        tris_a: *const u32,
        ntris_a: i32,
        verts_b: *const f64,
        nverts_b: i32,
        tris_b: *const u32,
        ntris_b: i32,
        op: i32,
    ) -> f64;
}

#[cfg(has_cgal)]
extern "C" {
    fn bench_cgal_mesh_op(
        verts_a: *const f64,
        nverts_a: i32,
        tris_a: *const u32,
        ntris_a: i32,
        verts_b: *const f64,
        nverts_b: i32,
        tris_b: *const u32,
        ntris_b: i32,
        op: i32,
    ) -> f64;
}

/// Flatten positions for the C ABI: 3 doubles per vertex, no conversion cost
/// inside the timed region.
fn flat_verts(mesh: &TriMesh) -> Vec<f64> {
    mesh.positions
        .iter()
        .flat_map(|p| [p.x, p.y, p.z])
        .collect()
}

/// Op codes shared with the C++ shim.
const OP_DIFFERENCE: i32 = 0;
const OP_UNION: i32 = 1;
const OP_INTERSECTION: i32 = 2;

/// Two spheres overlapping by half a radius, at `subdivisions` density.
///
/// Centres 1.5 apart with radius 1.0: a substantial lens, so the intersection
/// curve is long and neither operand is nearly contained in the other.
fn operands(subdivisions: u32) -> (TriMesh, TriMesh) {
    (
        icosphere([0.0, 0.0, 0.0], 1.0, subdivisions),
        icosphere([1.5, 0.0, 0.0], 1.0, subdivisions),
    )
}

/// Run the sphere-sphere ladder and report time and inclusion-exclusion error.
pub fn report(reps: usize, max_subdivisions: u32) {
    println!("\n\nSphere-sphere boolean -- two icospheres overlapping by 0.5r");
    println!("Oracle is inclusion-exclusion on the TESSELLATED operands, not the");
    println!("closed-form lens: an inscribed polyhedron is smaller than its sphere.");
    println!();
    println!(
        "{:>6}{:>10}{:>12}{:>12}{:>12}{:>12}{:>12}",
        "sub", "tris", "kernel", "union ms", "isect ms", "A-B ms", "identity"
    );

    for sub in 1..=max_subdivisions {
        let (a, b) = operands(sub);
        let tris = a.indices.len() / 3;
        let (va, vb) = (flat_verts(&a), flat_verts(&b));
        let (vol_a, vol_b) = (mesh_volume(&a), mesh_volume(&b));

        // axiolid: the mesh path. `cellular` declines non-axis-aligned input
        // by design and #77's exact boolean would need every face split
        // against every plane of the other, so this is the only axiolid
        // column that can run here.
        let provider = BoolmeshBoolean::default();
        let options = ExecutionOptions::new(Tolerance::METRE);
        let run_ax = |op: &str| -> (f64, f64) {
            let hook = std::panic::take_hook();
            std::panic::set_hook(Box::new(|_| {}));
            let out = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                best_of(reps, || {
                    let operator = match op {
                        "union" => BooleanOperator::Union,
                        "isect" => BooleanOperator::Intersection,
                        _ => BooleanOperator::Difference,
                    };
                    let r = provider.boolean(&a, &b, operator, &options);
                    r.map(|o| mesh_volume(&o.mesh)).unwrap_or(f64::NAN)
                })
            }));
            std::panic::set_hook(hook);
            out.unwrap_or((f64::NAN, f64::NAN))
        };

        let (ax_u_ms, ax_u) = run_ax("union");
        let (ax_i_ms, ax_i) = run_ax("isect");
        let (ax_d_ms, ax_d) = run_ax("diff");
        // |A u B| + |A n B| = |A| + |B|, exact for these polyhedra.
        let ax_id = if ax_u.is_nan() || ax_i.is_nan() {
            f64::NAN
        } else {
            ((ax_u + ax_i - vol_a - vol_b) / (vol_a + vol_b)).abs()
        };
        let _ = (ax_d, ax_d_ms);
        show_row(sub, tris, "axiolid", ax_u_ms, ax_i_ms, ax_d_ms, ax_id);

        #[cfg(has_manifold)]
        {
            let call = |op: i32| -> (f64, f64) {
                best_of(reps, || unsafe {
                    bench_manifold_mesh_op(
                        va.as_ptr(),
                        a.positions.len() as i32,
                        a.indices.as_ptr(),
                        tris as i32,
                        vb.as_ptr(),
                        b.positions.len() as i32,
                        b.indices.as_ptr(),
                        (b.indices.len() / 3) as i32,
                        op,
                    )
                })
            };
            let (u_ms, u) = call(OP_UNION);
            let (i_ms, i) = call(OP_INTERSECTION);
            let (d_ms, _) = call(OP_DIFFERENCE);
            let id = if u < 0.0 || i < 0.0 {
                f64::NAN
            } else {
                ((u + i - vol_a - vol_b) / (vol_a + vol_b)).abs()
            };
            show_row(sub, tris, "manifold", u_ms, i_ms, d_ms, id);
        }

        #[cfg(has_cgal)]
        {
            let call = |op: i32| -> (f64, f64) {
                best_of(reps, || unsafe {
                    bench_cgal_mesh_op(
                        va.as_ptr(),
                        a.positions.len() as i32,
                        a.indices.as_ptr(),
                        tris as i32,
                        vb.as_ptr(),
                        b.positions.len() as i32,
                        b.indices.as_ptr(),
                        (b.indices.len() / 3) as i32,
                        op,
                    )
                })
            };
            let (u_ms, u) = call(OP_UNION);
            let (i_ms, i) = call(OP_INTERSECTION);
            let (d_ms, _) = call(OP_DIFFERENCE);
            let id = if u < 0.0 || i < 0.0 {
                f64::NAN
            } else {
                ((u + i - vol_a - vol_b) / (vol_a + vol_b)).abs()
            };
            show_row(sub, tris, "cgal", u_ms, i_ms, d_ms, id);
        }
    }
}

/// One table row, rendering a failure as `n/a` rather than a number.
///
/// A kernel that declined returns NaN (axiolid) or a negative volume (the C++
/// shims): feeding either into the identity formula would print a plausible
/// figure for an answer that was never produced.
fn show_row(sub: u32, tris: usize, kernel: &str, u: f64, i: f64, d: f64, id: f64) {
    let ms = |v: f64| {
        if v.is_nan() {
            "n/a".to_owned()
        } else {
            format!("{v:.1}")
        }
    };
    let err = if id.is_nan() {
        "-".to_owned()
    } else {
        format!("{id:.2e}")
    };
    println!(
        "{sub:>6}{tris:>10}{kernel:>12}{:>12}{:>12}{:>12}{err:>12}",
        ms(u),
        ms(i),
        ms(d)
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use axiolid_core::Tolerance;
    use axiolid_mesh::audit_mesh;

    #[test]
    fn icospheres_are_closed_two_manifolds_at_every_density() {
        // Every kernel here refuses an open shell, so a seam in the generator
        // would show up as "the kernel failed" rather than "the fixture is
        // broken". Prove the fixture first.
        for sub in 0..=4 {
            let s = icosphere([0.0, 0.0, 0.0], 1.0, sub);
            let expected_tris = 20 * 4usize.pow(sub);
            assert_eq!(
                s.indices.len() / 3,
                expected_tris,
                "subdivision {sub} should give 20*4^n triangles"
            );
            let health = audit_mesh(&s, Tolerance::METRE);
            assert!(
                health.is_closed_two_manifold(),
                "subdivision {sub} is not a closed two-manifold: {health:?}"
            );
        }
    }

    #[test]
    fn tessellated_volume_approaches_the_ideal_sphere_from_below() {
        // An inscribed polyhedron is strictly smaller than the sphere. This is
        // exactly why the benchmark uses inclusion-exclusion and NOT the
        // closed-form lens volume: the gap below is real, density-dependent,
        // and would otherwise be misread as kernel error.
        let ideal = 4.0 / 3.0 * std::f64::consts::PI;
        let mut previous = 0.0;
        for sub in 0..=4 {
            let v = mesh_volume(&icosphere([0.0, 0.0, 0.0], 1.0, sub));
            assert!(v < ideal, "subdivision {sub}: {v} must be below {ideal}");
            assert!(v > previous, "subdivision {sub}: volume must increase");
            previous = v;
        }
        // Still short at the top of the ladder: the oracle cannot be the ideal.
        assert!(ideal - previous > 1e-4, "gap should remain measurable");
    }
}
