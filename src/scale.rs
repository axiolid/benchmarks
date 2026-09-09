//! Scale and translation invariance of the boolean itself.
//!
//! `scale_disparity` in the kernel corpus checks that `bounds()` survives a
//! nine-order scale gap on ONE static mesh. This runs the SAME boolean at
//! many scales and world offsets and normalises the answer back, which is
//! the property a BIM caller actually depends on: a millimetre feature on a
//! building placed at a survey coordinate must behave like the same feature
//! at the origin.
//!
//! # What is compared
//!
//! Every result is mapped back by `(p - offset) / scale` before measuring,
//! so all rows are directly comparable to the baseline at scale 1, offset 0.
//! Volume is normalised by `scale^3`.
//!
//! # Why f64 predicts a cliff
//!
//! Doubles carry ~15.95 decimal digits. At world coordinate `x` the spacing
//! between representable neighbours is about `x * 2^-52`. At 1e9 that is
//! ~2.2e-7 m, so a 1 mm feature still has ~4500 distinct positions across
//! it. At 1e12 the spacing is ~2.2e-4 m and the same feature has only ~4
//! representable steps -- the geometry is quantised into nonsense before
//! any algorithm runs. A refusal there is CORRECT behaviour; a confident
//! wrong answer is not.

use std::time::Instant;

use axiolid_contracts::ExecutionOptions;
use axiolid_core::{BooleanOperator, Point3, Scalar, Tolerance};
use axiolid_mesh::{audit_mesh, component_count, TriMesh};
use axiolid_mesh_boolean_boolmesh::BoolmeshBoolean;
use axiolid_mesh_boolean_contract::MeshBoolean;

use crate::sphere::{icosphere, mesh_volume};

/// Axis-aligned unit-ish box as a closed triangle mesh.
fn boxx(min: [Scalar; 3], max: [Scalar; 3]) -> TriMesh {
    let ([x0, y0, z0], [x1, y1, z1]) = (min, max);
    let positions = vec![
        Point3::new(x0, y0, z0),
        Point3::new(x1, y0, z0),
        Point3::new(x1, y1, z0),
        Point3::new(x0, y1, z0),
        Point3::new(x0, y0, z1),
        Point3::new(x1, y0, z1),
        Point3::new(x1, y1, z1),
        Point3::new(x0, y1, z1),
    ];
    let indices = vec![
        0, 2, 1, 0, 3, 2, 4, 5, 6, 4, 6, 7, 0, 1, 5, 0, 5, 4, 1, 2, 6, 1, 6, 5, 2, 3, 7, 2, 7, 6,
        3, 0, 4, 3, 4, 7,
    ];
    TriMesh::new(positions, indices)
}

/// Map a local mesh into world space: scale about the origin, then translate.
fn place(mesh: &TriMesh, scale: Scalar, offset: Scalar) -> TriMesh {
    let mut out = mesh.clone();
    for p in &mut out.positions {
        *p = Point3::new(
            p.x * scale + offset,
            p.y * scale + offset,
            p.z * scale + offset,
        );
    }
    out
}

/// Map a world result back to local coordinates, so every row is comparable.
fn unplace(mesh: &TriMesh, scale: Scalar, offset: Scalar) -> TriMesh {
    let mut out = mesh.clone();
    for p in &mut out.positions {
        *p = Point3::new(
            (p.x - offset) / scale,
            (p.y - offset) / scale,
            (p.z - offset) / scale,
        );
    }
    out
}

/// Axis-aligned bounds as (min, max) triples.
fn bounds(mesh: &TriMesh) -> ([Scalar; 3], [Scalar; 3]) {
    let mut lo = [Scalar::INFINITY; 3];
    let mut hi = [Scalar::NEG_INFINITY; 3];
    for p in &mesh.positions {
        for (k, v) in [p.x, p.y, p.z].into_iter().enumerate() {
            lo[k] = lo[k].min(v);
            hi[k] = hi[k].max(v);
        }
    }
    (lo, hi)
}

/// Largest corner deviation of `m` bounds from `base` bounds.
fn bounds_error(m: &TriMesh, base: &TriMesh) -> Scalar {
    let (lo, hi) = bounds(m);
    let (blo, bhi) = bounds(base);
    let mut worst: Scalar = 0.0;
    for k in 0..3 {
        worst = worst.max((lo[k] - blo[k]).abs());
        worst = worst.max((hi[k] - bhi[k]).abs());
    }
    worst
}

/// Symmetric VERTEX-SET Hausdorff distance.
///
/// Honest naming: this is the Hausdorff distance between the two vertex
/// SETS, not between the two surfaces. A surface Hausdorff needs
/// point-to-triangle distance and is quadratic in triangles; the vertex
/// form is enough here because the perturbation under test moves vertices
/// rather than retriangulating, so a surface deviation would show up as a
/// vertex deviation too. Reported, not gated, for exactly that reason.
fn vertex_hausdorff(a: &TriMesh, b: &TriMesh) -> Scalar {
    let directed = |from: &TriMesh, to: &TriMesh| {
        let mut worst: Scalar = 0.0;
        for p in &from.positions {
            let mut best = Scalar::INFINITY;
            for q in &to.positions {
                let d = (p.x - q.x).powi(2) + (p.y - q.y).powi(2) + (p.z - q.z).powi(2);
                if d < best {
                    best = d;
                }
            }
            worst = worst.max(best.sqrt());
        }
        worst
    };
    directed(a, b).max(directed(b, a))
}

/// The local (unit-scale, origin-centred) operands, shared by every row.
fn operands() -> (TriMesh, TriMesh) {
    let subject = boxx([0.0, 0.0, 0.0], [2.0, 2.0, 2.0]);
    let tool = icosphere([0.7, 0.7, 0.7], 0.9, 2);
    (subject, tool)
}

/// Run the same boolean across scales and world offsets.
/// Returns the number of rows where the BOOLEAN lost accuracy.
///
/// Input-limited rows are deliberately NOT counted: placement destroyed the
/// operands before the kernel saw them, which is a property of f64 and the
/// callers coordinates, not a kernel defect to gate on.
#[must_use]
pub fn report() -> usize {
    let provider = BoolmeshBoolean::new();
    let opts = ExecutionOptions::new(Tolerance::METRE);
    let (subject, tool) = operands();

    // Baseline: scale 1, offset 0. Every other row normalises back to this.
    let base = provider
        .boolean(&subject, &tool, BooleanOperator::Difference, &opts)
        .expect("baseline difference")
        .mesh;
    let base_vol = mesh_volume(&base);
    let base_comps = component_count(&base);

    println!();
    println!("Scale + translation invariance -- same boolean, placed differently");
    println!("{}", "-".repeat(112));
    println!("Results are mapped back by (p - offset) / scale before measuring,");
    println!("so every row is directly comparable to scale 1 / offset 0.");
    println!("f64 spacing at coordinate x is ~x*2^-52: at 1e12 a 1 mm feature has ~4");
    println!("representable steps, so a REFUSAL there is correct, not a failure.");
    println!();
    println!(
        "  {:>8} {:>8} {:>10} {:>11} {:>11} {:>11} {:>5} {:>8}  verdict",
        "scale", "offset", "spacing", "in err", "vol err", "hausdorff", "comps", "ms"
    );

    let scales = [1e-9, 1e-6, 1e-3, 1.0, 1e3, 1e6, 1e9];
    let offsets = [0.0, 1e3, 1e6, 1e9, 1e12];

    let mut refused = 0usize;
    let mut wrong = 0usize;
    let mut input_limited = 0usize;
    for &scale in &scales {
        for &offset in &offsets {
            // Skip combinations where the offset is below the feature size:
            // the operand would straddle the origin and the row would not be
            // testing remote placement at all.
            if offset != 0.0 && offset < scale {
                continue;
            }
            let s = place(&subject, scale, offset);
            let t = place(&tool, scale, offset);

            // Self-check FIRST: round-trip the operands with no boolean at
            // all. Placement snaps every coordinate to the f64 grid at
            // `offset`, so at large offsets the kernel is handed a
            // DIFFERENT solid than the caller described. Blaming the
            // boolean for that would be wrong, so it is measured and
            // attributed separately.
            let sv = mesh_volume(&subject);
            let tv = mesh_volume(&tool);
            let in_err = ((mesh_volume(&unplace(&s, scale, offset)) - sv) / sv)
                .abs()
                .max(((mesh_volume(&unplace(&t, scale, offset)) - tv) / tv).abs());

            let start = Instant::now();
            let outcome = provider.boolean(&s, &t, BooleanOperator::Difference, &opts);
            let ms = start.elapsed().as_secs_f64() * 1e3;

            // ULP spacing at the far corner of the placed operand.
            let far = offset + 2.0 * scale;
            let spacing = if far == 0.0 {
                0.0
            } else {
                far.abs() * Scalar::EPSILON
            };

            let result = match outcome {
                Ok(o) => o.mesh,
                Err(e) => {
                    refused += 1;
                    let name = format!("{e}");
                    let short = name.split(':').next().unwrap_or("error").to_string();
                    println!(
                        "  {scale:>8.0e} {offset:>8.0e} {spacing:>10.1e} {:>11} {:>11} {:>11} {:>5} {ms:>8.2}  REFUSED {short}",
                        "-", "-", "-", "-"
                    );
                    continue;
                }
            };

            let local = unplace(&result, scale, offset);
            let vol = mesh_volume(&local);
            let vol_err = ((vol - base_vol) / base_vol).abs();
            let b_err = bounds_error(&local, &base);
            let haus = vertex_hausdorff(&local, &base);
            let comps = component_count(&local);
            let health = audit_mesh(&local, Tolerance::METRE);

            let mut bad = Vec::new();
            // 1e-9 relative: placement is exact in binary only when scale is a
            // power of two, so a few ulps of drift are expected and benign.
            // Anything at 1e-9 or worse is the geometry actually moving.
            // Attribute honestly. `in_err` is how much PLACEMENT alone
            // moved the operands, with no boolean involved. When it is at
            // least as large as the result error, the kernel was handed a
            // different solid than the caller described and is reproducing
            // it faithfully: an INPUT-LIMITED row, not a kernel defect.
            // Only an output error EXCEEDING the input damage is the
            // boolean losing accuracy on its own.
            if vol_err > 1e-9 {
                if vol_err > in_err * 2.0 {
                    bad.push(format!("VOLUME {vol_err:.1e} vs input {in_err:.1e}"));
                } else {
                    bad.push(format!("input-limited {in_err:.1e} -> {vol_err:.1e}"));
                }
            }

            // Bounds are gated rather than printed: a bounds shift that
            // volume misses (an equal-and-opposite move) is exactly the
            // silent failure this fixture exists to catch.
            if b_err > 1e-9 && b_err > in_err * 2.0 {
                bad.push(format!("BOUNDS {b_err:.1e}"));
            }
            if comps != base_comps {
                bad.push(format!("COMPONENTS {base_comps} to {comps}"));
            }
            if !health.is_closed_two_manifold() {
                bad.push(format!("NOT CLOSED ({} boundary)", health.boundary_edges));
            }
            // Count only rows where the BOOLEAN lost accuracy, not rows
            // where placement had already destroyed the operands.
            if bad.iter().any(|b| !b.starts_with("input-limited")) {
                wrong += 1;
            } else if !bad.is_empty() {
                input_limited += 1;
            }
            let verdict = if bad.is_empty() {
                "ok".to_string()
            } else {
                format!("!! {}", bad.join("; "))
            };

            println!(
                "  {scale:>8.0e} {offset:>8.0e} {spacing:>10.1e} {in_err:>11.2e} {vol_err:>11.2e} {haus:>11.2e} {comps:>5} {ms:>8.2}  {verdict}"
            );
        }
    }

    println!();
    println!("  {refused} refused, {input_limited} input-limited (placement quantised the");
    println!("  operands before the kernel saw them), {wrong} genuine kernel error(s).");
    wrong
}
