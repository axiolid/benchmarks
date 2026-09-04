//! Where exact arithmetic is supposed to matter: near-degenerate overlap.
//!
//! The drift table uses well-separated flat-faced boxes, where every
//! intersection point is rational and f64 is nearly exact. That is the
//! FRIENDLIEST case for a floating-point kernel, so "axiolid matches CGAL"
//! there is weak evidence.
//!
//! This sweeps a cutter's face toward exact coincidence with the subject's
//! face: overlap 1e-1 down to 1e-15. As the sliver thins, a floating-point
//! kernel must decide whether two nearly-identical planes are the same plane.
//! Getting that wrong produces a missing face, a doubled face, or a wildly
//! wrong volume -- the classic failure exact predicates exist to prevent.
//!
//! Ground truth is exact and trivial here (the overlap is an axis-aligned
//! box), so any deviation is the kernel's, not the oracle's.

use crate::{axiolid_obb, axiolid_volume, flat_cutters, Box3, Obb};
use axiolid_contracts::ExecutionOptions;
use axiolid_core::{BooleanOperator, Tolerance};
use axiolid_mesh_boolean_boolmesh::BoolmeshBoolean;
use axiolid_mesh_boolean_contract::MeshBoolean;

type CppFn = unsafe extern "C" fn(*const f64, *const f64, *const f64, i32) -> f64;

/// Subject and cutter overlapping by exactly `d` along +X.
fn operands(d: f64) -> (Obb, Obb) {
    let subject = Obb::aabb(Box3::new(0.0, 0.0, 0.0, 1.0, 1.0, 1.0));
    // Cutter is unit-sized and shifted so the overlap slab is d thick.
    let cutter = Obb::aabb(Box3::new(1.0 - d, 0.0, 0.0, 1.0, 1.0, 1.0));
    (subject, cutter)
}

/// Relative error of `A ^ B` against the exact overlap volume `d`.
fn err(got: Option<f64>, d: f64) -> Option<f64> {
    got.map(|v| ((v - d) / d).abs())
}

/// Sweep the overlap toward coincidence and report each kernel's relative error.
pub fn report() {
    println!("\n\nNear-degenerate overlap -- intersection of two unit cubes");
    println!("{}", "-".repeat(88));
    println!("Overlap d shrinks toward an exactly-coincident face. Exact volume is d.");
    println!("Relative error; `fail` = kernel returned nothing.\n");
    println!(
        "{:>10}{:>14}{:>14}{:>14}{:>14}",
        "overlap d", "axiolid", "manifold", "cgal", "occt"
    );

    let options = ExecutionOptions::new(Tolerance::MILLIMETRE);
    let provider = BoolmeshBoolean::new();

    for &d in &[1e-1, 1e-3, 1e-6, 1e-9, 1e-12, 1e-15] {
        let (a, b) = operands(d);
        print!("{d:>10.0e}");

        let ax = provider
            .boolean(
                &axiolid_obb(a),
                &axiolid_obb(b),
                BooleanOperator::Intersection,
                &options,
            )
            .ok()
            .map(|o| axiolid_volume(&o.mesh));
        show(err(ax, d));

        let aabb = a.as_aabb().expect("axis-aligned subject");
        let operand = flat_cutters(&[b]);
        let mut call = |f: CppFn| {
            let v = unsafe {
                f(
                    aabb.min.as_ptr(),
                    aabb.max.as_ptr(),
                    operand.as_ptr(),
                    2, // intersection
                )
            };
            if v < 0.0 {
                None
            } else {
                Some(v)
            }
        };

        #[cfg(has_manifold)]
        show(err(call(crate::bench_manifold_op), d));
        #[cfg(not(has_manifold))]
        print!("{:>14}", "n/a");
        #[cfg(has_cgal)]
        show(err(call(crate::bench_cgal_op), d));
        #[cfg(not(has_cgal))]
        print!("{:>14}", "n/a");
        #[cfg(has_occt)]
        show(err(call(crate::bench_occt_op), d));
        #[cfg(not(has_occt))]
        print!("{:>14}", "n/a");
        println!();
    }
}

fn show(e: Option<f64>) {
    match e {
        Some(v) => print!("{v:>14.2e}"),
        None => print!("{:>14}", "fail"),
    }
}
