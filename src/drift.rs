//! Chain-length error growth: does f64 construction cost precision?

use crate::{
    axiolid_obb, axiolid_volume, expected_rotated_volume, flat_cutters, signed_volume,
    to_manifold_obb, wall_with_rotated_openings, Obb,
};

/// Signature every C ABI subtract entry point shares.
type CppFn = unsafe extern "C" fn(*const f64, *const f64, *const f64, i32) -> f64;
use axiolid_contracts::ExecutionOptions;
use axiolid_core::Tolerance;
use axiolid_mesh::TriMesh;
use axiolid_mesh_boolean_boolmesh::BoolmeshBoolean;
use axiolid_mesh_boolean_contract::MeshBoolean;

/// Does error ACCUMULATE as cuts are chained?
///
/// The timing table asks how fast; this asks whether speed was bought with
/// precision. Each kernel applies the same N sequential cuts and is compared
/// against analytically derived ground truth -- never against another kernel,
/// which would let a shared error pass silently.
///
/// The distinguishing hypothesis: a kernel that stores constructed points as
/// f64 must round every intersection, so error can compound as the output of
/// one boolean becomes the input of the next. A kernel with exact
/// constructions (CGAL's EPECK keeps coordinates as lazy exact algebraic
/// numbers) has no such drift -- its 64th cut is as exact as its first.
///
/// Rotated operands are essential: an axis-aligned cut produces intersection
/// coordinates that are already exactly representable in binary floating
/// point, so it cannot expose the difference. At 30 degrees they are not.
pub fn drift_report() {
    println!("\n\nError vs chain length -- rotated cuts, relative to derived ground truth");
    println!("{}", "-".repeat(88));
    println!("If f64 construction cost precision, error grows with n. Flat means it does not.\n");
    println!(
        "{:>5}  {:>12}  {:>12}  {:>12}  {:>12}  {:>12}",
        "n", "axiolid", "raw_bmesh", "manifold", "cgal", "occt"
    );

    for &n in &[1usize, 2, 4, 8, 16, 32, 64] {
        let (wall, openings) = wall_with_rotated_openings(n);
        let want = expected_rotated_volume(n);
        let rel = |v: f64| (v - want).abs() / want.abs().max(1e-12);

        let ax = {
            let host = axiolid_obb(Obb::aabb(wall));
            let tools: Vec<TriMesh> = openings.iter().map(|o| axiolid_obb(*o)).collect();
            let provider = BoolmeshBoolean::new();
            let options = ExecutionOptions::new(Tolerance::MILLIMETRE);
            provider
                .subtract_many(&host, &tools, &options)
                .ok()
                .map(|o| rel(axiolid_volume(&o.mesh)))
        };

        let raw = {
            let mut acc = to_manifold_obb(Obb::aabb(wall));
            let mut ok = true;
            for o in &openings {
                match boolmesh::prelude::compute_boolean(
                    &acc,
                    &to_manifold_obb(*o),
                    boolmesh::prelude::OpType::Subtract,
                ) {
                    Ok(next) => acc = next,
                    Err(_) => {
                        ok = false;
                        break;
                    }
                }
            }
            ok.then(|| {
                rel(signed_volume(
                    acc.get_indices()
                        .iter()
                        .map(|t| [t.x as u32, t.y as u32, t.z as u32]),
                    |i| {
                        let q = acc.ps[i as usize];
                        [q.x, q.y, q.z]
                    },
                )
                .abs())
            })
        };

        let flat = flat_cutters(&openings);
        let aabb = Obb::aabb(wall).as_aabb().expect("wall is axis aligned");
        let cpp = |f: Option<CppFn>| -> Option<f64> {
            let f = f?;
            let v = unsafe {
                f(
                    aabb.min.as_ptr(),
                    aabb.max.as_ptr(),
                    flat.as_ptr(),
                    n as i32,
                )
            };
            (v >= 0.0).then(|| rel(v))
        };

        #[cfg(has_manifold)]
        let mf = cpp(Some(crate::bench_manifold_subtract as CppFn));
        #[cfg(not(has_manifold))]
        let mf = cpp(None);
        #[cfg(has_cgal)]
        let cg = cpp(Some(crate::bench_cgal_subtract as CppFn));
        #[cfg(not(has_cgal))]
        let cg = cpp(None);
        #[cfg(has_occt)]
        let oc = cpp(Some(crate::bench_occt_subtract as CppFn));
        #[cfg(not(has_occt))]
        let oc = cpp(None);

        let cell = |v: Option<f64>| match v {
            Some(v) if v == 0.0 => "0".to_owned(),
            Some(v) => format!("{v:.2e}"),
            None => "n/a".to_owned(),
        };
        println!(
            "{n:>5}  {:>12}  {:>12}  {:>12}  {:>12}  {:>12}",
            cell(ax),
            cell(raw),
            cell(mf),
            cell(cg),
            cell(oc)
        );
    }
}
