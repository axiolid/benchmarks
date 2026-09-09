//! Remeshing invariance: geometry must not depend on triangulation.
//!
//! Each perturbation rewrites the index buffer or vertex order while
//! leaving the surface identical. Volume and topology must not move.
//!
//! A perturbation that changed the surface would make this a lie, so
//! each one is self-checked: volume before and after must agree to 1e-12.

use crate::sphere::{icosphere, mesh_volume};
use axiolid_contracts::ExecutionOptions;
use axiolid_core::{BooleanOperator, Point3, Scalar, Tolerance};
use axiolid_mesh::{component_count, TriMesh};
use axiolid_mesh_boolean_boolmesh::BoolmeshBoolean;
use axiolid_mesh_boolean_contract::MeshBoolean;

/// Deterministic 64-bit PRNG, so a failure is reproducible.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    fn upto(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
}

/// Shuffle triangle order. Pure index permutation.
fn shuffle_triangles(mesh: &TriMesh, rng: &mut Rng) -> TriMesh {
    let mut tris: Vec<[u32; 3]> = mesh.indices.chunks(3).map(|c| [c[0], c[1], c[2]]).collect();
    for i in (1..tris.len()).rev() {
        let j = rng.upto(i + 1);
        tris.swap(i, j);
    }
    let mut out = mesh.clone();
    out.indices = tris.into_iter().flatten().collect();
    out
}

/// Renumber vertices through a random permutation, rewriting indices to
/// match. The surface is untouched; only the identity of each slot moves.
fn reindex_vertices(mesh: &TriMesh, rng: &mut Rng) -> TriMesh {
    let n = mesh.positions.len();
    let mut perm: Vec<usize> = (0..n).collect();
    for i in (1..n).rev() {
        let j = rng.upto(i + 1);
        perm.swap(i, j);
    }
    // perm[old] = new, so positions move to their new slot and every
    // index is rewritten through the same map.
    let mut positions = vec![Point3::new(0.0, 0.0, 0.0); n];
    for (old, &new) in perm.iter().enumerate() {
        positions[new] = mesh.positions[old];
    }
    let indices = mesh
        .indices
        .iter()
        .map(|&i| perm[i as usize] as u32)
        .collect();
    TriMesh {
        positions,
        indices,
        ..Default::default()
    }
}

/// Split every triangle into four by its edge midpoints.
///
/// Midpoints are new vertices ON the existing edges, so the surface is
/// unchanged for a PLANAR facet. Vertices are duplicated per triangle
/// rather than shared: that is legitimate, and it also exercises the
/// seam-duplication case at the same time.
fn subdivide(mesh: &TriMesh) -> TriMesh {
    let mut out = TriMesh::default();
    for t in mesh.indices.chunks(3) {
        let (a, b, c) = (
            mesh.positions[t[0] as usize],
            mesh.positions[t[1] as usize],
            mesh.positions[t[2] as usize],
        );
        let ab = (a + b) * 0.5;
        let bc = (b + c) * 0.5;
        let ca = (c + a) * 0.5;
        let base = out.positions.len() as u32;
        out.positions.extend([a, b, c, ab, bc, ca]);
        // a-ab-ca, ab-b-bc, ca-bc-c, ab-bc-ca: same winding as the parent.
        let (a, b, c, ab, bc, ca) = (base, base + 1, base + 2, base + 3, base + 4, base + 5);
        out.indices
            .extend([a, ab, ca, ab, b, bc, ca, bc, c, ab, bc, ca]);
    }
    out
}

/// An axis-aligned box whose every quad face is split along a chosen
/// diagonal. `flip` picks the other diagonal, which is the same solid
/// with a different triangulation.
fn diag_box(c: [Scalar; 3], s: [Scalar; 3], flip: bool) -> TriMesh {
    let (x0, y0, z0) = (c[0] - s[0] * 0.5, c[1] - s[1] * 0.5, c[2] - s[2] * 0.5);
    let (x1, y1, z1) = (c[0] + s[0] * 0.5, c[1] + s[1] * 0.5, c[2] + s[2] * 0.5);
    let p = [
        Point3::new(x0, y0, z0),
        Point3::new(x1, y0, z0),
        Point3::new(x1, y1, z0),
        Point3::new(x0, y1, z0),
        Point3::new(x0, y0, z1),
        Point3::new(x1, y0, z1),
        Point3::new(x1, y1, z1),
        Point3::new(x0, y1, z1),
    ];
    // Outward-facing quads, each as (a,b,c,d) in CCW order seen from
    // outside. Splitting a-b-c/a-c-d or b-c-d/b-d-a are the two diagonals.
    let quads = [
        [0u32, 3, 2, 1],
        [4, 5, 6, 7],
        [0, 1, 5, 4],
        [1, 2, 6, 5],
        [2, 3, 7, 6],
        [3, 0, 4, 7],
    ];
    let mut out = TriMesh {
        positions: p.to_vec(),
        ..Default::default()
    };
    for q in quads {
        if flip {
            out.indices.extend([q[1], q[2], q[3], q[1], q[3], q[0]]);
        } else {
            out.indices.extend([q[0], q[1], q[2], q[0], q[2], q[3]]);
        }
    }
    out
}

/// Run one boolean per representation and compare against the baseline.
pub fn report() {
    let provider = BoolmeshBoolean::new();
    let opts = ExecutionOptions::new(Tolerance::MILLIMETRE);
    let mut rng = Rng(0x2545_F491_4F6C_DD1D);

    println!("Remesh invariance -- same solid, different triangulation");
    println!("{}", "-".repeat(104));
    println!("Perturbations preserve the SURFACE, so the boolean volume must not move.");
    println!("in vol err = perturbation self-check; out vol err = the actual claim.");
    println!();
    println!(
        "  {:<26} {:>7} {:>12} {:>12} {:>6} {:>8} {:>9}",
        "representation", "tris", "in vol err", "out vol err", "comps", "out tris", "verdict"
    );

    // Subject: a box. Tool: a sphere overlapping one corner region, so
    // the boolean does real cutting work and the intersection curve is
    // curved rather than a coincident plane.
    let base_subject = diag_box([0.0, 0.0, 0.0], [2.0, 2.0, 2.0], false);
    let tool = icosphere([0.7, 0.7, 0.7], 0.9, 2);
    let base_in_vol = mesh_volume(&base_subject);
    let base_out = provider
        .boolean(&base_subject, &tool, BooleanOperator::Difference, &opts)
        .expect("baseline difference")
        .mesh;
    let base_vol = mesh_volume(&base_out);
    let base_comps = component_count(&base_out);

    let flipped = diag_box([0.0, 0.0, 0.0], [2.0, 2.0, 2.0], true);
    let variants: Vec<(String, TriMesh)> = vec![
        ("baseline".to_string(), base_subject.clone()),
        ("flipped quad diagonals".to_string(), flipped),
        (
            "shuffled triangles".to_string(),
            shuffle_triangles(&base_subject, &mut rng),
        ),
        (
            "reindexed vertices".to_string(),
            reindex_vertices(&base_subject, &mut rng),
        ),
        (
            "subdivided 1x (seams dup)".to_string(),
            subdivide(&base_subject),
        ),
        (
            "subdivided 2x".to_string(),
            subdivide(&subdivide(&base_subject)),
        ),
        (
            "subdiv + shuffle + reindex".to_string(),
            reindex_vertices(
                &shuffle_triangles(&subdivide(&base_subject), &mut rng),
                &mut rng,
            ),
        ),
    ];

    let mut faults = 0usize;
    for (label, subject) in &variants {
        // Self-check: the perturbation must not have changed the solid.
        // Without this the whole table could pass by comparing two
        // different shapes that happen to agree.
        let in_err = ((mesh_volume(subject) - base_in_vol) / base_in_vol).abs();
        let out = provider
            .boolean(subject, &tool, BooleanOperator::Difference, &opts)
            .expect("difference");
        let vol = mesh_volume(&out.mesh);
        let comps = component_count(&out.mesh);
        let out_err = ((vol - base_vol) / base_vol).abs();

        let mut bad = Vec::new();
        // 1e-12 relative: the perturbations are exact midpoint splits and
        // permutations, so any real drift is far larger than rounding.
        if in_err > 1e-12 {
            bad.push(format!("PERTURBATION CHANGED INPUT {in_err:.2e}"));
        }
        if out_err > 1e-9 {
            bad.push(format!("VOLUME MOVED {out_err:.2e}"));
        }
        if comps != base_comps {
            bad.push(format!("COMPONENTS {base_comps} to {comps}"));
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
            "  {:<26} {:>7} {:>12.2e} {:>12.2e} {:>6} {:>8} {:>9}",
            label,
            subject.indices.len(),
            in_err,
            out_err,
            comps,
            out.mesh.indices.len(),
            verdict
        );
    }

    println!();
    if faults == 0 {
        println!("  all representations agree: the boolean is triangulation-invariant here.");
    } else {
        println!("  {faults} representation(s) disagree -- see verdicts above.");
    }
}

/// Does each representation land on the SAME vertex set run to run?
///
/// The invariance table above compares VOLUME, which is a scalar and
/// survives vertex reordering. The upstream nondeterminism moves vertex
/// ORDER and VALUES without moving volume, so it is invisible there.
/// This checks the stronger property, per representation, over repeats.
pub fn stability_probe(runs: usize) {
    use std::collections::BTreeSet;
    let provider = BoolmeshBoolean::new();
    let opts = ExecutionOptions::new(Tolerance::MILLIMETRE);
    let mut rng = Rng(0x9E37_79B9_7F4A_7C15);

    let base = diag_box([0.0, 0.0, 0.0], [2.0, 2.0, 2.0], false);
    let tool = icosphere([0.7, 0.7, 0.7], 0.9, 2);

    let variants: Vec<(String, TriMesh)> = vec![
        ("baseline".to_string(), base.clone()),
        (
            "flipped diagonals".to_string(),
            diag_box([0.0, 0.0, 0.0], [2.0, 2.0, 2.0], true),
        ),
        (
            "shuffled triangles".to_string(),
            shuffle_triangles(&base, &mut rng),
        ),
        (
            "reindexed vertices".to_string(),
            reindex_vertices(&base, &mut rng),
        ),
        ("subdivided 1x".to_string(), subdivide(&base)),
    ];

    println!();
    println!("Remesh stability -- is each representation itself reproducible?");
    println!("{}", "-".repeat(104));
    println!("Volume is permutation-invariant, so it CANNOT see ordering drift.");
    println!("This fingerprints positions AND indices over {runs} identical runs.");
    println!();
    println!(
        "  {:<26} {:>10} {:>12}  verdict",
        "representation", "out tris", "distinct"
    );

    for (label, subject) in &variants {
        let mut seen = BTreeSet::new();
        let mut tris = 0usize;
        for _ in 0..runs {
            let out = provider
                .boolean(subject, &tool, BooleanOperator::Difference, &opts)
                .expect("difference")
                .mesh;
            tris = out.indices.len();
            let mut key = String::new();
            for p in &out.positions {
                key.push_str(&format!("{:.17e},{:.17e},{:.17e};", p.x, p.y, p.z));
            }
            key.push('|');
            for i in &out.indices {
                key.push_str(&format!("{i},"));
            }
            seen.insert(key);
        }
        let verdict = if seen.len() == 1 {
            "STABLE".to_string()
        } else {
            format!("!! NONDETERMINISTIC ({} distinct)", seen.len())
        };
        println!(
            "  {:<26} {:>10} {:>12}  {}",
            label,
            tris,
            seen.len(),
            verdict
        );
    }
}
