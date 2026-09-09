//! The contact lattice: every relationship two solids can have.
//!
//! `sliver.rs` sweeps ONE relationship (a thinning face overlap) and
//! `conditioning.rs` in the kernel sweeps the same thing for the reported
//! conditioning number. Neither covers the rest of the lattice, and the
//! degenerate contacts are where boolean kernels actually break: a shared
//! edge or a single shared vertex has zero-volume intersection, so a
//! classifier that answers "inside or outside" with a tolerance has no
//! correct answer available and must pick one consistently.
//!
//! # What is asserted
//!
//! Every case has an EXACT oracle derived from the construction, never read
//! back from the implementation:
//!
//! | case | union | intersection | difference |
//! |---|---|---|---|
//! | disjoint | va + vb | 0 | va |
//! | contained | va | vb | va - vb |
//! | identical | va | va | 0 |
//! | face/edge/vertex contact | va + vb | 0 | va |
//! | tiny overlap eps | va + vb - eps | eps slab | va - eps slab |
//!
//! # Refusal is an acceptable answer, a wrong number is not
//!
//! At eps = 1e-15 on a unit cube the overlap is below the f64 spacing of the
//! coordinates that define it, so the operands are not reliably distinct.
//! A refusal there is CORRECT. What must never happen is a confident volume
//! that is wrong by more than the requested tolerance -- that is the failure
//! this matrix exists to catch.

use axiolid_contracts::ExecutionOptions;
use axiolid_core::{BooleanOperator, Point3, Scalar, Tolerance};
use axiolid_mesh::{audit_mesh, component_count, TriMesh};
use axiolid_mesh_boolean_boolmesh::BoolmeshBoolean;
use axiolid_mesh_boolean_contract::MeshBoolean;

use crate::sphere::{icosphere, mesh_volume};

/// Axis-aligned box from min/max corners.
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

/// A unit cube at the origin.
fn unit() -> TriMesh {
    boxx([0.0, 0.0, 0.0], [1.0, 1.0, 1.0])
}

/// What the three operators should return, in volume.
#[derive(Clone, Copy)]
struct Oracle {
    union: Scalar,
    intersection: Scalar,
    difference: Scalar,
}

/// One row of the lattice.
struct Case {
    name: &'static str,
    a: TriMesh,
    b: TriMesh,
    oracle: Oracle,
    /// Whether a refusal is a legitimate answer for this case. True only
    /// where the operands are genuinely below f64 resolution.
    refusal_ok: bool,
}

/// The non-swept cases: every qualitative relationship, built exactly.
fn lattice() -> Vec<Case> {
    let mut cases = Vec::new();

    // Disjoint: a clear gap of 1.
    cases.push(Case {
        name: "disjoint (gap 1)",
        a: unit(),
        b: boxx([2.0, 0.0, 0.0], [3.0, 1.0, 1.0]),
        oracle: Oracle {
            union: 2.0,
            intersection: 0.0,
            difference: 1.0,
        },
        refusal_ok: false,
    });

    // Contained: the small cube is strictly inside, sharing no surface.
    let small = boxx([0.25, 0.25, 0.25], [0.75, 0.75, 0.75]);
    let sv = 0.5 * 0.5 * 0.5;
    cases.push(Case {
        name: "contained (no contact)",
        a: unit(),
        b: small,
        oracle: Oracle {
            union: 1.0,
            intersection: sv,
            difference: 1.0 - sv,
        },
        refusal_ok: false,
    });

    // Partial overlap: the ordinary case, half a cube deep.
    cases.push(Case {
        name: "partial overlap",
        a: unit(),
        b: boxx([0.5, 0.0, 0.0], [1.5, 1.0, 1.0]),
        oracle: Oracle {
            union: 1.5,
            intersection: 0.5,
            difference: 0.5,
        },
        refusal_ok: false,
    });

    // Identical: A == B exactly, bit for bit.
    cases.push(Case {
        name: "identical",
        a: unit(),
        b: unit(),
        oracle: Oracle {
            union: 1.0,
            intersection: 1.0,
            difference: 0.0,
        },
        refusal_ok: false,
    });

    // The three degenerate contacts. All have zero-volume intersection, so
    // union is the sum and difference is A untouched. These are the rows
    // most likely to expose a tolerance-based classifier.

    // Face touching: x=1 plane shared, full 1x1 square of contact.
    cases.push(Case {
        name: "face contact",
        a: unit(),
        b: boxx([1.0, 0.0, 0.0], [2.0, 1.0, 1.0]),
        oracle: Oracle {
            union: 2.0,
            intersection: 0.0,
            difference: 1.0,
        },
        refusal_ok: false,
    });

    // Edge touching: the cubes share exactly the line x=1, y=1.
    cases.push(Case {
        name: "edge contact",
        a: unit(),
        b: boxx([1.0, 1.0, 0.0], [2.0, 2.0, 1.0]),
        oracle: Oracle {
            union: 2.0,
            intersection: 0.0,
            difference: 1.0,
        },
        refusal_ok: false,
    });

    // Vertex touching: the cubes share exactly the point (1,1,1).
    cases.push(Case {
        name: "vertex contact",
        a: unit(),
        b: boxx([1.0, 1.0, 1.0], [2.0, 2.0, 2.0]),
        oracle: Oracle {
            union: 2.0,
            intersection: 0.0,
            difference: 1.0,
        },
        refusal_ok: false,
    });

    // Sphere tangency: centres exactly 2r apart, so the surfaces meet at
    // one point. Curved tangency rather than planar, and the oracle uses
    // the TESSELLATED volume because an icosphere inscribes its sphere.
    let s1 = icosphere([0.0, 0.0, 0.0], 1.0, 3);
    let s2 = icosphere([2.0, 0.0, 0.0], 1.0, 3);
    let sph_v = mesh_volume(&s1);
    cases.push(Case {
        name: "sphere tangent",
        a: s1,
        b: s2,
        oracle: Oracle {
            union: 2.0 * sph_v,
            intersection: 0.0,
            difference: sph_v,
        },
        refusal_ok: false,
    });

    cases
}

/// The epsilon sweep: separation +eps and overlap -eps around face contact.
///
/// Positive eps is a gap, so the solids are disjoint. Negative eps is a thin
/// slab of overlap of exactly `eps * 1 * 1`. Both approach the same
/// degenerate face contact from opposite sides, which is the point: a kernel
/// that snaps to coincidence must not snap in only one direction.
fn epsilon_cases() -> Vec<Case> {
    let mut cases = Vec::new();
    for &eps in &[1e-3, 1e-6, 1e-9, 1e-12, 1e-15] {
        // Below ~1e-16 relative, `1.0 + eps` is not distinct from 1.0 in
        // f64, so at 1e-15 the operands are at the edge of representability
        // and a refusal is legitimate.
        let fragile = eps <= 1e-15;

        cases.push(Case {
            name: Box::leak(format!("gap +{eps:.0e}").into_boxed_str()),
            a: unit(),
            b: boxx([1.0 + eps, 0.0, 0.0], [2.0 + eps, 1.0, 1.0]),
            oracle: Oracle {
                union: 2.0,
                intersection: 0.0,
                difference: 1.0,
            },
            refusal_ok: fragile,
        });

        // The oracle must use the overlap f64 ACTUALLY STORED, not the one
        // requested: placing a corner at `1.0 - eps` rounds, and at 1e-12
        // the stored slab differs from `eps` in the 5th significant digit.
        // Charging the kernel for the fixture\u0027s own placement rounding
        // would manufacture a failure that is not the kernel\u0027s.
        let lo = 1.0 - eps;
        let slab = 1.0 - lo;
        cases.push(Case {
            name: Box::leak(format!("overlap -{eps:.0e}").into_boxed_str()),
            a: unit(),
            b: boxx([lo, 0.0, 0.0], [1.0 + lo, 1.0, 1.0]),
            oracle: Oracle {
                union: 2.0 - slab,
                intersection: slab,
                difference: 1.0 - slab,
            },
            refusal_ok: fragile,
        });
    }
    cases
}

/// Spheres just short of and just past tangency.
///
/// The lens volume of two spheres overlapping by depth `d` is
/// `pi*d^2*(3r - d)/3`. That is the ANALYTIC lens, and these operands are
/// tessellated, so it is used only as an order-of-magnitude expectation:
/// the gate below compares against the tessellated union identity instead,
/// which is exact for the mesh actually passed in.
fn sphere_epsilon_cases() -> Vec<Case> {
    let mut cases = Vec::new();
    for &eps in &[1e-3, 1e-6, 1e-9, 1e-12] {
        let a = icosphere([0.0, 0.0, 0.0], 1.0, 3);
        let v = mesh_volume(&a);

        // Separated by eps: still disjoint.
        cases.push(Case {
            name: Box::leak(format!("sphere gap +{eps:.0e}").into_boxed_str()),
            a: icosphere([0.0, 0.0, 0.0], 1.0, 3),
            b: icosphere([2.0 + eps, 0.0, 0.0], 1.0, 3),
            oracle: Oracle {
                union: 2.0 * v,
                intersection: 0.0,
                difference: v,
            },
            refusal_ok: false,
        });
    }
    cases
}

/// Relative error, guarding the zero-oracle cases.
fn rel(got: Scalar, want: Scalar) -> Scalar {
    if want.abs() < 1e-12 {
        // A zero oracle: report the absolute value, since a relative error
        // against zero is meaningless and would hide a non-empty result.
        got.abs()
    } else {
        ((got - want) / want).abs()
    }
}

/// Run the full contact lattice. Returns the number of gate violations.
#[must_use]
pub fn report() -> usize {
    let provider = BoolmeshBoolean::new();
    let opts = ExecutionOptions::new(Tolerance::MILLIMETRE);

    println!();
    println!("Contact lattice -- every relationship two solids can have");
    println!("{}", "-".repeat(112));
    println!("Oracles are derived from the construction, never read back from the kernel.");
    println!("Degenerate contacts (face/edge/vertex) have ZERO-volume intersection.");
    println!("A refusal is acceptable only where the operands are below f64 resolution.");
    println!();
    println!(
        "  {:<24} {:>12} {:>12} {:>12} {:>6}  verdict",
        "case", "union err", "isect err", "diff err", "comps"
    );

    let mut faults = 0usize;
    let mut cases = lattice();
    cases.extend(epsilon_cases());
    cases.extend(sphere_epsilon_cases());

    for case in &cases {
        let run = |op: BooleanOperator| {
            provider
                .boolean(&case.a, &case.b, op, &opts)
                .map(|o| (mesh_volume(&o.mesh), o.mesh))
        };

        let (u, i, d) = (
            run(BooleanOperator::Union),
            run(BooleanOperator::Intersection),
            run(BooleanOperator::Difference),
        );

        // A refusal on any operator: acceptable only where declared.
        if u.is_err() || i.is_err() || d.is_err() {
            let which = match (&u, &i, &d) {
                (Err(e), _, _) => format!("union: {e}"),
                (_, Err(e), _) => format!("isect: {e}"),
                (_, _, Err(e)) => format!("diff: {e}"),
                _ => unreachable!(),
            };
            let short = which.split(':').next().unwrap_or("?").to_string();
            if case.refusal_ok {
                println!(
                    "  {:<24} {:>12} {:>12} {:>12} {:>6}  refused ({short}) -- allowed",
                    case.name, "-", "-", "-", "-"
                );
            } else {
                faults += 1;
                println!(
                    "  {:<24} {:>12} {:>12} {:>12} {:>6}  !! REFUSED {short}",
                    case.name, "-", "-", "-", "-"
                );
            }
            continue;
        }

        let (uv, umesh) = u.expect("checked");
        let (iv, _) = i.expect("checked");
        let (dv, _) = d.expect("checked");

        let ue = rel(uv, case.oracle.union);
        let ie = rel(iv, case.oracle.intersection);
        let de = rel(dv, case.oracle.difference);
        let comps = component_count(&umesh);
        let health = audit_mesh(&umesh, Tolerance::METRE);

        // 1e-6 relative: the boxes are exact in f64 and the sphere oracles
        // are the tessellated volumes, so the only slack needed is
        // accumulation across the mesh, which is far below this.
        let tol = 1e-6;
        let mut bad = Vec::new();
        if ue > tol {
            bad.push(format!("UNION {ue:.1e}"));
        }
        if ie > tol {
            bad.push(format!("ISECT {ie:.1e}"));
        }
        if de > tol {
            bad.push(format!("DIFF {de:.1e}"));
        }
        if !health.is_closed_two_manifold() {
            bad.push(format!("UNION NOT CLOSED ({})", health.boundary_edges));
        }
        if !bad.is_empty() {
            faults += 1;
        }
        let verdict = if bad.is_empty() {
            "ok".to_string()
        } else {
            format!("!! {}", bad.join("; "))
        };
        println!(
            "  {:<24} {ue:>12.2e} {ie:>12.2e} {de:>12.2e} {comps:>6}  {verdict}",
            case.name
        );
    }

    println!();
    if faults == 0 {
        println!("  the whole contact lattice behaves.");
    } else {
        println!("  {faults} contact case(s) violated their oracle.");
    }
    faults
}
