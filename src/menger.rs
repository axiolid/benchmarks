//! Menger sponge: the recursive-subtraction stress case.
//!
//! Every other workload here is a wall with openings: a few cutters against
//! one host. The sponge inverts that profile. At depth d the
//! host is cut by 20^d - 1 grid-aligned boxes, so it stresses cutter count
//! and coincident-plane handling rather than operand complexity.
//!
//! # The oracle
//!
//! Volume is closed-form: each level keeps 20 of 27 subcubes, so a unit
//! sponge at depth d has volume (20/27)^d exactly. That is a strong check --
//! it is sensitive to a single missing or doubled cutter, unlike a
//! self-consistency test that only compares kernels against each other.
//!
//! # Why the cutters are emitted flat, not recursively subtracted
//!
//! Building the sponge as d rounds of "subtract 20 sub-sponges" would measure
//! the harness's recursion as much as the kernel. Instead every void box is
//! generated up front and handed to one `subtract_many`, which is the same
//! call shape the other workloads use and keeps the columns comparable.

/// An axis-aligned void box, in the same `[min, max]` form `Box3` uses.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Void {
    pub min: [f64; 3],
    pub max: [f64; 3],
}

/// Every void box of a unit Menger sponge at `depth`, ordered deterministically.
///
/// The unit cube spans [0, 1]^3. At each level a cell is split into 27
/// subcells and the 7 with at least two centred coordinates are removed: the
/// 6 face centres and the body centre. Surviving cells recurse.
pub fn voids(depth: u32) -> Vec<Void> {
    let mut out = Vec::new();
    carve(&mut out, [0.0, 0.0, 0.0], 1.0, depth);
    out
}

/// Emit the voids of one cell, then recurse into the 20 survivors.
fn carve(out: &mut Vec<Void>, origin: [f64; 3], size: f64, depth: u32) {
    if depth == 0 {
        return;
    }
    let third = size / 3.0;
    for i in 0..3usize {
        for j in 0..3usize {
            for k in 0..3usize {
                // A subcell is removed when at least two of its indices are
                // the centre (1): the 6 face centres plus the core.
                let centred = usize::from(i == 1) + usize::from(j == 1) + usize::from(k == 1);
                let corner = [
                    origin[0] + third * i as f64,
                    origin[1] + third * j as f64,
                    origin[2] + third * k as f64,
                ];
                if centred >= 2 {
                    out.push(Void {
                        min: corner,
                        max: [corner[0] + third, corner[1] + third, corner[2] + third],
                    });
                } else {
                    carve(out, corner, third, depth - 1);
                }
            }
        }
    }
}

/// Closed-form volume of a unit sponge at `depth`: (20/27)^depth.
pub fn expected_volume(depth: u32) -> f64 {
    (20.0f64 / 27.0).powi(depth as i32)
}

/// Number of void boxes at `depth`: 7 * sum_{i<depth} 20^i.
pub fn void_count(depth: u32) -> usize {
    (0..depth).map(|i| 7 * 20usize.pow(i)).sum()
}

use crate::{axiolid_box, axiolid_obb, axiolid_volume, best_of, flat_cutters, Box3, Obb};
use axiolid_construct::polyhedron::{boolean_polyhedra_exact, triangulate, BooleanOp, Polyhedron};
use axiolid_contracts::ExecutionOptions;
use axiolid_core::{Point3, Tolerance};
use axiolid_mesh::TriMesh;
use axiolid_mesh_boolean_boolmesh::BoolmeshBoolean;
use axiolid_mesh_boolean_contract::MeshBoolean;

// Manifold's own sample algorithm, for a like-for-like comparison against the
// published depth-4 figure. Absent unless the Manifold shim was compiled.
#[cfg(has_manifold)]
extern "C" {
    fn bench_manifold_menger_composed(depth: i32) -> f64;
    fn bench_manifold_menger_holes(depth: i32) -> i32;
}

type CppFn = unsafe extern "C" fn(*const f64, *const f64, *const f64, i32) -> f64;

extern "C" {
    #[cfg(has_manifold)]
    fn bench_manifold_subtract(mn: *const f64, mx: *const f64, c: *const f64, n: i32) -> f64;
    #[cfg(has_cgal)]
    fn bench_cgal_subtract(mn: *const f64, mx: *const f64, c: *const f64, n: i32) -> f64;
}

/// The unit cube the sponge is carved from.
fn host() -> Box3 {
    Box3::new(0.5, 0.5, 0.5, 1.0, 1.0, 1.0)
}

/// Void boxes as `Obb`, the operand form every column already consumes.
fn cutters(depth: u32) -> Vec<Obb> {
    voids(depth)
        .into_iter()
        .map(|v| {
            Obb::aabb(Box3 {
                min: v.min,
                max: v.max,
            })
        })
        .collect()
}

/// Run the sponge at depths 1..=max_depth and report time and volume error.
///
/// Each row is one `subtract_many` with every void handed over at once, so
/// the columns mean the same thing they do in the wall table.
pub fn report(reps: usize, max_depth: u32) {
    println!("\n\nMenger sponge -- one host, 20^d grid-aligned cutters");
    println!("Volume oracle is closed-form (20/27)^d; error is relative.");
    println!(
        "\n  {:>5}  {:>7}  {:>12}  {:>10}  {:>12}  {:>10}  {:>12}  {:>10}",
        "depth", "cutters", "axiolid ms", "err", "manifold ms", "err", "cgal ms", "err"
    );

    let provider = BoolmeshBoolean::default();
    let options = ExecutionOptions::new(Tolerance::METRE);
    let wall = host();

    for depth in 1..=max_depth {
        let openings = cutters(depth);
        let expected = expected_volume(depth);
        let rel = |v: f64| ((v - expected) / expected).abs();

        let show = |ms: f64, e: f64| -> (String, String) {
            if ms.is_nan() {
                ("n/a".to_owned(), "-".to_owned())
            } else {
                (format!("{ms:.1}"), format!("{e:.2e}"))
            }
        };

        // boolmesh 0.1.9 PANICS (not errors) on this workload from depth 2:
        // an out-of-bounds index inside its own half-edge mesh. A panic would
        // take the whole harness down and lose every later row, so it is
        // caught here and reported as a failed column. This is a real upstream
        // defect, not a slow path: see README.
        let hook = std::panic::take_hook();
        std::panic::set_hook(Box::new(|_| {}));
        let ax_col = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let ax_host = axiolid_box(wall);
            let ax_tools: Vec<TriMesh> = openings.iter().map(|o| axiolid_obb(*o)).collect();
            best_of(reps, || {
                provider
                    .subtract_many(&ax_host, &ax_tools, &options)
                    .expect("axiolid subtract_many")
                    .mesh
            })
        }));
        std::panic::set_hook(hook);
        let (ax_ms, ax_err) = match ax_col {
            Ok((ms, out)) => (ms, rel(axiolid_volume(&out))),
            Err(_) => (f64::NAN, f64::NAN),
        };

        let flat = flat_cutters(&openings);
        let cpp_col = |f: CppFn| -> (f64, f64) {
            let (ms, v) = best_of(reps, || unsafe {
                f(
                    wall.min.as_ptr(),
                    wall.max.as_ptr(),
                    flat.as_ptr(),
                    openings.len() as i32,
                )
            });
            // Negative volume is the shims' documented failure signal. Passing
            // it through `rel` would render a failure as a large-but-finite
            // error, which reads as "inaccurate" rather than "did not answer".
            if v < 0.0 {
                (f64::NAN, f64::NAN)
            } else {
                (ms, rel(v))
            }
        };

        #[cfg(has_manifold)]
        let (mf_ms, mf_err) = cpp_col(bench_manifold_subtract);
        #[cfg(not(has_manifold))]
        let (mf_ms, mf_err) = (f64::NAN, f64::NAN);
        #[cfg(has_cgal)]
        let (cg_ms, cg_err) = cpp_col(bench_cgal_subtract);
        #[cfg(not(has_cgal))]
        let (cg_ms, cg_err) = (f64::NAN, f64::NAN);
        let _ = &cpp_col;

        let (mf_t, mf_e) = show(mf_ms, mf_err);
        let (cg_t, cg_e) = show(cg_ms, cg_err);

        let (ax_t, ax_e) = show(ax_ms, ax_err);
        println!(
            "  {:>5}  {:>7}  {:>12}  {:>10}  {:>12}  {:>10}  {:>12}  {:>10}",
            depth,
            openings.len(),
            ax_t,
            ax_e,
            mf_t,
            mf_e,
            cg_t,
            cg_e
        );
    }
}

/// Manifold's own recursive-composition algorithm, for a like-for-like
/// comparison against the published depth-4 timing.
///
/// The flattened table above subtracts every void box separately: 58947
/// operations at depth 4. This builds the sponge the way Manifold's
/// `samples/src/menger_sponge.cpp` does -- a Sierpinski carpet of holes,
/// batch-unioned once, then subtracted in three orthogonal orientations.
/// That is FOUR booleans regardless of depth, so it is a different algorithm
/// and the two timings must never be read as the same measurement.
///
/// Only Manifold has a column here: reproducing the composition form for the
/// other kernels means porting their batch-union paths too, which is a
/// separate piece of work. A one-kernel table is honest; a table with
/// invented peer numbers would not be.
pub fn composed_report(reps: usize, max_depth: u32) {
    println!("\n\nMenger sponge -- recursive composition (Manifold's own algorithm)");
    println!("Four booleans per depth, not one per void. Not comparable to the table above.");
    println!();
    println!(
        "{:>7}{:>9}{:>14}{:>12}",
        "depth", "holes", "manifold ms", "err"
    );

    for depth in 1..=max_depth {
        #[cfg(has_manifold)]
        {
            let holes = unsafe { bench_manifold_menger_holes(depth as i32) };
            let (ms, vol) = best_of(reps, || unsafe {
                bench_manifold_menger_composed(depth as i32)
            });
            let expected = expected_volume(depth);
            // The shims use a negative return as their failure signal; a
            // relative error computed from it would look like a real number.
            let err = if vol < 0.0 {
                "-".to_owned()
            } else {
                format!("{:.2e}", ((vol - expected) / expected).abs())
            };
            println!("{depth:>7}{holes:>9}{ms:>14.1}{err:>12}");
        }
        #[cfg(not(has_manifold))]
        {
            let _ = (reps, depth);
            println!("{depth:>7}{:>9}{:>14}{:>12}", "-", "n/a", "-");
        }
    }
}

/// The analytic cellular path on the sponge.
///
/// Menger is the ideal case for `cellular.rs`: every void is axis-aligned, so
/// every cutter face is a grid plane and each cell is wholly solid or
/// wholly void. No general boolean is needed.
pub fn cellular_report(reps: usize, max_depth: u32) {
    println!("\n\nMenger sponge -- analytic cellular path (axiolid)");
    println!("Grid decomposition, no general boolean. Same solid, same oracle.");
    println!();
    println!(
        "{:>7}{:>9}{:>11}{:>14}{:>12}",
        "depth", "voids", "cells", "cellular ms", "err"
    );

    for depth in 1..=max_depth {
        let boxes: Vec<([f64; 3], [f64; 3])> =
            voids(depth).iter().map(|v| (v.min, v.max)).collect();
        let expected = expected_volume(depth);

        // Budget generously: the point is to find where the grid stops being
        // viable, not to hide that limit behind a low ceiling.
        let (ms, out) = best_of(reps, || {
            crate::cellular::subtract_boxes((host().min, host().max), &boxes, 1 << 26)
        });

        match out {
            None => println!(
                "{depth:>7}{:>9}{:>11}{:>14}{:>12}",
                boxes.len(),
                "-",
                "declined",
                "-"
            ),
            Some(cells) => {
                let got = mesh_volume(&cells.positions, &cells.indices);
                let err = ((got - expected) / expected).abs();
                println!(
                    "{depth:>7}{:>9}{:>11}{ms:>14.1}{err:>12.2e}",
                    boxes.len(),
                    cells.indices.len() / 3
                );
            }
        }
    }
}

/// Volume of a closed triangle soup by the divergence theorem.
fn mesh_volume(positions: &[[f64; 3]], indices: &[u32]) -> f64 {
    let mut sum = 0.0;
    for t in indices.chunks_exact(3) {
        let a = positions[t[0] as usize];
        let b = positions[t[1] as usize];
        let c = positions[t[2] as usize];
        sum += a[0] * (b[1] * c[2] - b[2] * c[1]) - a[1] * (b[0] * c[2] - b[2] * c[0])
            + a[2] * (b[0] * c[1] - b[1] * c[0]);
    }
    (sum / 6.0).abs()
}

/// Does #77's exact boolean survive where boolmesh panics?
///
/// This measures ROBUSTNESS first and speed second. boolmesh dies at
/// depth 2 with an out-of-bounds index; the question is whether the exact
/// planar-faced boolean returns a correct answer on the same input.
///
/// It is a pairwise API, so this is one call per void: the flattened
/// shape, directly comparable to the `axiolid` column of the first table.
pub fn exact_report(max_depth: u32) {
    println!("\n\nMenger sponge -- #77 exact planar-faced boolean (axiolid)");
    println!("Pairwise, one call per void. Robustness probe: boolmesh panics here.");
    println!();
    println!(
        "{:>7}{:>9}{:>16}{:>12}",
        "depth", "voids", "exact ms", "err"
    );

    /// A `Polyhedron` for an axis-aligned box, outward-wound.
    fn poly_box(min: [f64; 3], max: [f64; 3]) -> Polyhedron {
        let p = |x: f64, y: f64, z: f64| Point3::new(x, y, z);
        let (n, x) = (min, max);
        let faces = vec![
            vec![
                p(n[0], n[1], n[2]),
                p(n[0], x[1], n[2]),
                p(x[0], x[1], n[2]),
                p(x[0], n[1], n[2]),
            ],
            vec![
                p(n[0], n[1], x[2]),
                p(x[0], n[1], x[2]),
                p(x[0], x[1], x[2]),
                p(n[0], x[1], x[2]),
            ],
            vec![
                p(n[0], n[1], n[2]),
                p(x[0], n[1], n[2]),
                p(x[0], n[1], x[2]),
                p(n[0], n[1], x[2]),
            ],
            vec![
                p(x[0], n[1], n[2]),
                p(x[0], x[1], n[2]),
                p(x[0], x[1], x[2]),
                p(x[0], n[1], x[2]),
            ],
            vec![
                p(x[0], x[1], n[2]),
                p(n[0], x[1], n[2]),
                p(n[0], x[1], x[2]),
                p(x[0], x[1], x[2]),
            ],
            vec![
                p(n[0], x[1], n[2]),
                p(n[0], n[1], n[2]),
                p(n[0], n[1], x[2]),
                p(n[0], x[1], x[2]),
            ],
        ];
        Polyhedron::new(faces).expect("axis-aligned box is a valid polyhedron")
    }

    for depth in 1..=max_depth {
        let boxes = cutters(depth);
        let expected = expected_volume(depth);
        let host_box = host();

        // Guarded: a panic here is the finding, not a crash to hide.
        let hook = std::panic::take_hook();
        std::panic::set_hook(Box::new(|_| {}));
        let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let start = std::time::Instant::now();
            let mut acc = poly_box(host_box.min, host_box.max);
            for b in &boxes {
                let aabb = b.as_aabb().expect("menger cutters are axis-aligned");
                let tool = poly_box(aabb.min, aabb.max);
                acc = boolean_polyhedra_exact(&acc, &tool, BooleanOp::Difference)?;
            }
            let ms = start.elapsed().as_secs_f64() * 1e3;
            Ok::<_, axiolid_contracts::GeomError>((ms, triangulate(&acc)))
        }));
        std::panic::set_hook(hook);

        match outcome {
            Err(_) => println!("{depth:>7}{:>9}{:>16}{:>12}", boxes.len(), "PANIC", "-"),
            // Print the refusal reason: "refused" alone does not say whether
            // this is a fixable degeneracy or an out-of-scope input.
            Ok(Err(e)) => println!(
                "{depth:>7}{:>9}{:>16}{:>12}   {e}",
                boxes.len(),
                "refused",
                "-"
            ),
            Ok(Ok((ms, mesh))) => {
                let got = axiolid_volume(&mesh);
                let err = ((got - expected) / expected).abs();
                println!("{depth:>7}{:>9}{:>16.1}{:>12.2e}", boxes.len(), ms, err);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn void_counts_match_the_closed_form() {
        for depth in 0..4 {
            assert_eq!(voids(depth).len(), void_count(depth), "depth {depth}");
        }
        // Depth 1 removes 7 of 27 subcubes; depth 2 adds 20 more rounds of 7.
        assert_eq!(void_count(1), 7);
        assert_eq!(void_count(2), 147);
    }

    #[test]
    fn voids_are_disjoint_and_sum_to_the_removed_volume() {
        for depth in 1..4 {
            let total: f64 = voids(depth)
                .iter()
                .map(|v| (v.max[0] - v.min[0]) * (v.max[1] - v.min[1]) * (v.max[2] - v.min[2]))
                .sum();
            let removed = 1.0 - expected_volume(depth);
            assert!(
                (total - removed).abs() < 1e-12,
                "depth {depth}: voids sum to {total}, expected {removed}"
            );
        }
    }

    #[test]
    #[ignore = "boolmesh 0.1.9 panics; see README"]
    fn depth_two_does_not_panic_the_backend() {
        // Regression guard for the crash found when this benchmark was added:
        // boolmesh 0.1.9 panics with an out-of-bounds index inside its own
        // half-edge mesh at depth 2 (147 grid-aligned cutters). A panic is worse
        // than a wrong answer here: the harness cannot report it as a mismatch,
        // it takes the whole process down.
        //
        // Marked #[ignore] so the suite stays green while the upstream bug is
        // open; run with `cargo test --release -- --ignored` to check whether a
        // newer boolmesh fixes it.
        let openings = cutters(2);
        assert_eq!(openings.len(), 147);

        let provider = BoolmeshBoolean::default();
        let options = ExecutionOptions::new(Tolerance::METRE);
        let ax_host = axiolid_box(host());
        let ax_tools: Vec<TriMesh> = openings.iter().map(|o| axiolid_obb(*o)).collect();

        let out = provider
            .subtract_many(&ax_host, &ax_tools, &options)
            .expect("subtract_many must return an error, not panic");
        let got = axiolid_volume(&out.mesh);
        let expected = expected_volume(2);
        assert!(
            ((got - expected) / expected).abs() < 1e-9,
            "depth 2 volume {got}, expected {expected}"
        );
    }

    #[test]
    fn voids_share_faces_from_depth_two() {
        // Why this workload is hard, stated as a fact rather than a guess:
        // a cell's +X face-centre void touches that cell's +X face, and the
        // neighbouring cell's -X face-centre void touches the same plane from
        // the other side. So from depth 2 the cutters are face-adjacent, and
        // the kernel must decide coincident planes rather than merely nest
        // disjoint boxes. Depth 1's 7 voids only touch at the core.
        let d2 = voids(2);
        let touching = d2
            .iter()
            .enumerate()
            .flat_map(|(i, a)| d2.iter().skip(i + 1).map(move |b| (a, b)))
            .filter(|(a, b)| {
                // Overlapping in two axes and exactly abutting in the third.
                let overlap = |k: usize| a.min[k] < b.max[k] - 1e-12 && b.min[k] < a.max[k] - 1e-12;
                let abut = |k: usize| {
                    (a.max[k] - b.min[k]).abs() < 1e-12 || (b.max[k] - a.min[k]).abs() < 1e-12
                };
                (0..3).any(|k| abut(k) && (0..3).filter(|&j| j != k).all(overlap))
            })
            .count();
        assert!(
            touching > 0,
            "depth 2 voids should include face-adjacent pairs; found none"
        );
    }
}
