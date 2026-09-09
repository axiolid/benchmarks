//! Remeshing invariance: geometry must not depend on triangulation.
//!
//! Each perturbation rewrites the index buffer, the vertex order, or the
//! triangle SHAPES while leaving the surface identical. Volume and topology
//! must not move.
//!
//! Two of these are pathological on purpose: `skinny_subdivide` drives the
//! worst triangle quality from 0.866 down to 0.002, and `uneven_refine`
//! makes the largest triangle 4x the smallest. Both stay closed and valid,
//! so a disagreement would be a real finding rather than malformed input.
//!
//! A perturbation that changed the surface would make this a lie, so
//! each one is self-checked: volume before and after must agree to 1e-12.

use crate::sphere::{icosphere, mesh_volume};
use axiolid_contracts::ExecutionOptions;
use axiolid_core::{BooleanOperator, Point3, Scalar, Tolerance};
use axiolid_mesh::{audit_mesh, component_count, TriMesh};
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
    weld(&out)
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
        "  {:<26} {:>7} {:>9} {:>8} {:>11} {:>11} {:>5} {:>8} {:>9}",
        "representation",
        "tris",
        "min qual",
        "spread",
        "in vol err",
        "out vol err",
        "comps",
        "out tris",
        "verdict"
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
        (
            "skinny 4-way (t=0.02)".to_string(),
            skinny_subdivide(&base_subject, 0.02),
        ),
        (
            "skinny 4-way (t=0.002)".to_string(),
            skinny_subdivide(&base_subject, 0.002),
        ),
        (
            "uneven: dense near cut".to_string(),
            // Uniformly subdivided twice FIRST, so there are enough triangles
            // for "dense here, coarse there" to be a real contrast; then only
            // the region near the sphere is refined again.
            uneven_refine(
                &subdivide(&subdivide(&base_subject)),
                Point3::new(0.7, 0.7, 0.7),
                1.1,
            ),
        ),
        (
            "uneven + skinny".to_string(),
            skinny_subdivide(
                &uneven_refine(
                    &subdivide(&subdivide(&base_subject)),
                    Point3::new(0.7, 0.7, 0.7),
                    1.1,
                ),
                0.02,
            ),
        ),
    ];

    let mut faults = 0usize;
    for (label, subject) in &variants {
        // Self-check A: the perturbed input must still be a closed
        // two-manifold. Uneven refinement done naively leaves T-junctions,
        // which hand the boolean an OPEN shell -- a broken FIXTURE, not a
        // kernel finding, so it is caught and named separately.
        let health = audit_mesh(subject, Tolerance::METRE);
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
        if !health.is_closed_two_manifold() {
            bad.push(format!(
                "INPUT NOT CLOSED ({} boundary)",
                health.boundary_edges
            ));
        }
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
            "  {:<26} {:>7} {:>9.3} {:>8.1} {:>11.2e} {:>11.2e} {:>5} {:>8} {:>9}",
            label,
            subject.indices.len() / 3,
            min_quality(subject),
            area_spread(subject),
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
        (
            "skinny (t=0.002)".to_string(),
            skinny_subdivide(&base, 0.002),
        ),
        (
            "uneven near cut".to_string(),
            uneven_refine(
                &subdivide(&subdivide(&base)),
                Point3::new(0.7, 0.7, 0.7),
                1.1,
            ),
        ),
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

/// Normalised triangle quality: `4*sqrt(3)*area / sum of squared edges`.
///
/// 1.0 for equilateral, approaching 0 as a triangle degenerates. A standard
/// shape measure, used here so "skinny" is a reported number rather than an
/// adjective.
fn min_quality(mesh: &TriMesh) -> Scalar {
    let mut worst = 1.0;
    for t in mesh.indices.chunks(3) {
        let (a, b, c) = (
            mesh.positions[t[0] as usize],
            mesh.positions[t[1] as usize],
            mesh.positions[t[2] as usize],
        );
        let ab = [b.x - a.x, b.y - a.y, b.z - a.z];
        let ac = [c.x - a.x, c.y - a.y, c.z - a.z];
        let cross = [
            ab[1] * ac[2] - ab[2] * ac[1],
            ab[2] * ac[0] - ab[0] * ac[2],
            ab[0] * ac[1] - ab[1] * ac[0],
        ];
        let area = 0.5 * (cross[0].powi(2) + cross[1].powi(2) + cross[2].powi(2)).sqrt();
        let sq =
            |p: Point3, q: Point3| (p.x - q.x).powi(2) + (p.y - q.y).powi(2) + (p.z - q.z).powi(2);
        let sum = sq(a, b) + sq(b, c) + sq(c, a);
        if sum > 0.0 {
            let q = 4.0 * (3.0 as Scalar).sqrt() * area / sum;
            if q < worst {
                worst = q;
            }
        }
    }
    worst
}

/// Four-way split, but cutting each edge at `t` rather than its midpoint.
///
/// The three cut points lie exactly ON the original edges, so the surface is
/// unchanged; only the triangle shapes are. At `t = 0.02` the corner
/// triangles are extreme slivers while the mesh stays closed and valid --
/// which is the point: skinny BUT VALID, so a refusal would be a real
/// finding rather than the fixture being malformed.
///
/// The cut is taken from the lower-indexed endpoint so that both triangles
/// sharing an edge place the point identically. Interpolating from whichever
/// endpoint happened to come first would put two different points on one
/// edge and tear the surface open.
fn skinny_subdivide(mesh: &TriMesh, t: Scalar) -> TriMesh {
    let lerp = |p: Point3, q: Point3, f: Scalar| {
        Point3::new(
            p.x + (q.x - p.x) * f,
            p.y + (q.y - p.y) * f,
            p.z + (q.z - p.z) * f,
        )
    };
    let mut out = TriMesh::default();
    for tri in mesh.indices.chunks(3) {
        let (ia, ib, ic) = (tri[0], tri[1], tri[2]);
        let (a, b, c) = (
            mesh.positions[ia as usize],
            mesh.positions[ib as usize],
            mesh.positions[ic as usize],
        );
        // Canonical orientation per edge, independent of this triangle.
        let cut = |i: u32, p: Point3, j: u32, q: Point3| {
            // Always interpolate FROM the lower-indexed endpoint by the
            // same t, so both triangles sharing this edge compute the
            // identical expression and land on bitwise-equal points.
            if i < j {
                lerp(p, q, t)
            } else {
                lerp(q, p, t)
            }
        };
        let ab = cut(ia, a, ib, b);
        let bc = cut(ib, b, ic, c);
        let ca = cut(ic, c, ia, a);
        let base = out.positions.len() as u32;
        out.positions.extend([a, b, c, ab, bc, ca]);
        let (a, b, c, ab, bc, ca) = (base, base + 1, base + 2, base + 3, base + 4, base + 5);
        out.indices
            .extend([a, ab, ca, ab, b, bc, ca, bc, c, ab, bc, ca]);
    }
    weld(&out)
}

/// Refine only triangles near `focus`, leaving the rest coarse.
///
/// # Why this is not just "split the selected triangles"
///
/// Splitting a triangle but not its neighbour leaves a T-junction: the
/// neighbour keeps one long edge while this side now has two short ones.
/// The surface is then no longer edge-matched, `audit_mesh` reports boundary
/// edges, and the boolean is being handed an OPEN shell -- so any
/// disagreement would be the fixture being broken, not the kernel.
///
/// This uses red-green refinement instead. Every edge of a selected triangle
/// is marked; then EVERY triangle is retriangulated according to how many of
/// its own edges were marked (3 = full four-way split, 2 or 1 = a
/// compatible partial split, 0 = untouched). Marked edges are therefore
/// split from both sides, and the result stays closed and edge-matched
/// while the triangle density still varies sharply across the mesh.
fn uneven_refine(mesh: &TriMesh, focus: Point3, radius: Scalar) -> TriMesh {
    let key = |i: u32, j: u32| if i < j { (i, j) } else { (j, i) };
    let mid =
        |p: Point3, q: Point3| Point3::new(0.5 * (p.x + q.x), 0.5 * (p.y + q.y), 0.5 * (p.z + q.z));
    let near = |p: Point3| {
        let d = (p.x - focus.x).powi(2) + (p.y - focus.y).powi(2) + (p.z - focus.z).powi(2);
        d.sqrt() <= radius
    };

    // Pass 1: mark the edges of every triangle that touches the focus region.
    let mut marked = std::collections::BTreeSet::new();
    for t in mesh.indices.chunks(3) {
        let touches = t.iter().any(|&i| near(mesh.positions[i as usize]));
        if touches {
            marked.insert(key(t[0], t[1]));
            marked.insert(key(t[1], t[2]));
            marked.insert(key(t[2], t[0]));
        }
    }
    // Pass 2: retriangulate EVERY triangle by how many of its edges were
    // marked. This is what keeps the surface closed.
    let mut out = TriMesh::default();
    for t in mesh.indices.chunks(3) {
        let (ia, ib, ic) = (t[0], t[1], t[2]);
        let (a, b, c) = (
            mesh.positions[ia as usize],
            mesh.positions[ib as usize],
            mesh.positions[ic as usize],
        );
        let (m0, m1, m2) = (
            marked.contains(&key(ia, ib)),
            marked.contains(&key(ib, ic)),
            marked.contains(&key(ic, ia)),
        );
        let base = out.positions.len() as u32;
        let mut push = |pts: &[Point3], tris: &[u32]| {
            out.positions.extend_from_slice(pts);
            out.indices.extend(tris.iter().map(|i| base + i));
        };
        match (m0, m1, m2) {
            (false, false, false) => push(&[a, b, c], &[0, 1, 2]),
            // One edge split: fan from the opposite corner.
            (true, false, false) => push(&[a, b, c, mid(a, b)], &[0, 3, 2, 3, 1, 2]),
            (false, true, false) => push(&[a, b, c, mid(b, c)], &[1, 3, 0, 3, 2, 0]),
            (false, false, true) => push(&[a, b, c, mid(c, a)], &[2, 3, 1, 3, 0, 1]),
            // Two edges split: cut the corner between them, then split the quad.
            (true, true, false) => push(
                &[a, b, c, mid(a, b), mid(b, c)],
                &[3, 1, 4, 0, 3, 4, 0, 4, 2],
            ),
            (false, true, true) => push(
                &[a, b, c, mid(b, c), mid(c, a)],
                &[3, 2, 4, 0, 1, 3, 0, 3, 4],
            ),
            (true, false, true) => push(
                &[a, b, c, mid(a, b), mid(c, a)],
                &[0, 3, 4, 3, 1, 2, 3, 2, 4],
            ),
            // All three: the ordinary four-way split.
            (true, true, true) => push(
                &[a, b, c, mid(a, b), mid(b, c), mid(c, a)],
                &[0, 3, 5, 3, 1, 4, 5, 4, 2, 3, 4, 5],
            ),
        }
    }
    weld(&out)
}

/// Weld bit-identical positions into shared vertices.
///
/// The split helpers below emit six vertices per triangle, so a shared edge
/// gets one copy per side and every edge looks like a boundary. That is not
/// a legitimate seam duplication -- it is an unwelded mesh, and the kernel
/// correctly refuses it with `NotManifold`.
///
/// Welding is exact, on the bit pattern, with no tolerance: the split points
/// are computed from the same endpoints in the same canonical order on both
/// sides, so the two copies are bitwise equal by construction. A tolerance
/// here would risk merging genuinely distinct points and silently changing
/// the surface.
fn weld(mesh: &TriMesh) -> TriMesh {
    let mut map = std::collections::BTreeMap::new();
    let mut out = TriMesh::default();
    for &i in &mesh.indices {
        let p = mesh.positions[i as usize];
        let k = (p.x.to_bits(), p.y.to_bits(), p.z.to_bits());
        let slot = *map.entry(k).or_insert_with(|| {
            out.positions.push(p);
            out.positions.len() as u32 - 1
        });
        out.indices.push(slot);
    }
    out
}

/// Ratio of the largest triangle area to the smallest.
///
/// A uniform mesh is near 1; uneven refinement drives it up. Reported so
/// "uneven" is a measured property of the fixture rather than a claim.
fn area_spread(mesh: &TriMesh) -> Scalar {
    let (mut lo, mut hi) = (Scalar::INFINITY, 0.0 as Scalar);
    for t in mesh.indices.chunks(3) {
        let (a, b, c) = (
            mesh.positions[t[0] as usize],
            mesh.positions[t[1] as usize],
            mesh.positions[t[2] as usize],
        );
        let ab = [b.x - a.x, b.y - a.y, b.z - a.z];
        let ac = [c.x - a.x, c.y - a.y, c.z - a.z];
        let cr = [
            ab[1] * ac[2] - ab[2] * ac[1],
            ab[2] * ac[0] - ab[0] * ac[2],
            ab[0] * ac[1] - ab[1] * ac[0],
        ];
        let area = 0.5 * (cr[0].powi(2) + cr[1].powi(2) + cr[2].powi(2)).sqrt();
        if area > 0.0 {
            lo = lo.min(area);
            hi = hi.max(area);
        }
    }
    if lo.is_finite() && lo > 0.0 {
        hi / lo
    } else {
        1.0
    }
}
