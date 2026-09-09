//! Sphere-grid union: many-component operands at scale.
//!
//! Every other workload in this harness unions or subtracts a handful of
//! solids. This one builds grids of 8 to 1000+ spheres and unions them,
//! which is the standard large-scale stress case in Manifold's own
//! performance testing.
//!
//! # Why this workload and not another sphere ladder
//!
//! `sphere.rs` scales DENSITY: two operands, more triangles each. This
//! scales COUNT: many operands, each cheap. The two stress different
//! things. Density stresses the intersection-curve tracer; count stresses
//! the broad phase, the component bookkeeping, and any per-operand fixed
//! cost -- and it produces multi-component operands, which is where this
//! harness has already documented a real `boolmesh` nondeterminism (see
//! the determinism probe in `main.rs`: a FUSED multi-component tool drifts
//! where a single-component one does not).
//!
//! That makes the deterministic-hash column the point of this file, not a
//! nice-to-have beside the timings.
//!
//! # Two arrangements, deliberately
//!
//! * `Disjoint` -- spheres separated by a gap. The union is topologically
//!   trivial (the answer IS the concatenation) but the operands are
//!   maximally multi-component. This isolates per-component overhead and
//!   is the arrangement that triggers the known drift.
//! * `Overlapping` -- spheres closer than 2r, fusing into one connected
//!   blob. Every neighbour pair generates a real intersection curve, so
//!   this is the genuine geometric work case.
//!
//! Reporting both matters: a kernel can be fast on one and pathological on
//! the other, and a single "sphere grid" number would hide that.

use crate::sphere::{icosphere, mesh_volume};
use axiolid_contracts::ExecutionOptions;
use axiolid_core::{BooleanOperator, Tolerance};
use axiolid_mesh::{component_count, TriMesh};
use axiolid_mesh_boolean_boolmesh::BoolmeshBoolean;
use axiolid_mesh_boolean_contract::MeshBoolean;
use std::collections::BTreeSet;

/// How the grid is spaced relative to sphere radius.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Arrangement {
    /// Centres far enough apart that no two spheres touch.
    Disjoint,
    /// Centres closer than 2r, so neighbours fuse into one solid.
    Overlapping,
}

impl Arrangement {
    /// Centre-to-centre spacing, in units of radius.
    ///
    /// Disjoint uses 3.0r (a full radius of clear air between surfaces) so
    /// that no tolerance can make neighbours touch. Overlapping uses 1.5r,
    /// matching `sphere.rs`'s two-sphere case, so the per-pair intersection
    /// is the same shape already characterised there.
    fn spacing(self) -> f64 {
        match self {
            Self::Disjoint => 3.0,
            Self::Overlapping => 1.5,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Disjoint => "disjoint",
            Self::Overlapping => "overlap",
        }
    }
}

/// A `k` x `k` x `k` grid of icospheres.
///
/// Radius is fixed at 1.0 and spacing derived from the arrangement, so the
/// only free variables are count and tessellation density.
pub fn sphere_grid(k: usize, arrangement: Arrangement, subdivisions: u32) -> Vec<TriMesh> {
    let spacing = arrangement.spacing();
    let mut out = Vec::with_capacity(k * k * k);
    for i in 0..k {
        for j in 0..k {
            for l in 0..k {
                let c = [i as f64 * spacing, j as f64 * spacing, l as f64 * spacing];
                out.push(icosphere(c, 1.0, subdivisions));
            }
        }
    }
    out
}

/// Peak resident set size of this process, in kilobytes.
///
/// `VmHWM` is the kernel's own high-water mark, not a sample: it cannot
/// miss a spike between polls the way a sampling thread can.
pub fn peak_rss_kb() -> u64 {
    let status = std::fs::read_to_string("/proc/self/status").unwrap_or_default();
    for line in status.lines() {
        if let Some(rest) = line.strip_prefix("VmHWM:") {
            if let Some(kb) = rest.split_whitespace().next() {
                return kb.parse().unwrap_or(0);
            }
        }
    }
    0
}

/// Reset `VmHWM` to the current RSS so the next measurement is per-case.
///
/// Without this every case after the first would inherit the largest peak
/// any earlier case reached, and the column would be monotonic by
/// construction rather than by measurement. Returns whether the reset was
/// accepted -- on a kernel that refuses it the caller must say so rather
/// than print inherited numbers as if they were fresh.
pub fn reset_peak_rss() -> bool {
    use std::io::Write;
    std::fs::OpenOptions::new()
        .write(true)
        .open("/proc/self/clear_refs")
        .and_then(|mut f| f.write_all(b"5"))
        .is_ok()
}

/// Concatenate meshes without any boolean, rebasing indices.
///
/// For the disjoint arrangement this IS the exact union, so it doubles as
/// a ground-truth oracle: a boolean union of disjoint solids must have the
/// same volume as their concatenation, and any deviation is kernel error
/// rather than tessellation error.
pub fn concat(meshes: &[TriMesh]) -> TriMesh {
    let mut positions = Vec::new();
    let mut indices = Vec::new();
    for m in meshes {
        let base = positions.len() as u32;
        positions.extend_from_slice(&m.positions);
        indices.extend(m.indices.iter().map(|i| i + base));
    }
    TriMesh::new(positions, indices)
}

/// How a list of solids is reduced to a single union.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Strategy {
    /// Fold left: `((a u b) u c) u d`. The accumulator grows every step, so
    /// step `i` pays for a subject of roughly `i` spheres. Quadratic in
    /// total work even when each individual union is cheap.
    Sequential,
    /// Balanced pairwise reduction: union adjacent pairs, then pairs of
    /// those, until one remains. Same number of boolean calls, but the
    /// operands stay small until the final levels, so total work is
    /// `O(n log n)` rather than `O(n^2)`.
    ///
    /// This is the column that answers "grouping behavior": if the tree
    /// beats the fold, the provider's reduction ORDER is worth owning
    /// rather than leaving to the caller.
    Tree,
}

impl Strategy {
    fn label(self) -> &'static str {
        match self {
            Self::Sequential => "seq",
            Self::Tree => "tree",
        }
    }
}

/// Union every mesh in `parts` using `strategy`.
///
/// Returns `None` if any single boolean fails, rather than a partial union
/// that would look like a fast success.
pub fn union_all(
    provider: &BoolmeshBoolean,
    options: &ExecutionOptions,
    parts: &[TriMesh],
    strategy: Strategy,
) -> Option<TriMesh> {
    if parts.is_empty() {
        return None;
    }
    let op = BooleanOperator::Union;
    match strategy {
        Strategy::Sequential => {
            let mut acc = parts[0].clone();
            for part in &parts[1..] {
                acc = provider.boolean(&acc, part, op, options).ok()?.mesh;
            }
            Some(acc)
        }
        Strategy::Tree => {
            let mut level: Vec<TriMesh> = parts.to_vec();
            while level.len() > 1 {
                let mut next = Vec::with_capacity(level.len().div_ceil(2));
                let mut it = level.chunks_exact(2);
                for pair in &mut it {
                    next.push(provider.boolean(&pair[0], &pair[1], op, options).ok()?.mesh);
                }
                // An odd element rides to the next level uncombined rather
                // than being unioned against an already-merged neighbour,
                // which would unbalance the tree it exists to keep balanced.
                if let Some(last) = it.remainder().first() {
                    next.push(last.clone());
                }
                level = next;
            }
            level.into_iter().next()
        }
    }
}

/// Mesh fingerprint over positions AND indices.
///
/// Deliberately the same shape as `main.rs`'s `Fp`. The harness has already
/// been burned once by a position-only hash that hid permuted triangle
/// order and reported a false STABLE, inverting the probe's conclusion.
/// Any fingerprint added here must cover both or it is worse than none.
fn fingerprint(m: &TriMesh) -> (usize, usize, u64) {
    let pos = m
        .positions
        .iter()
        .flat_map(|p| [p.x, p.y, p.z])
        .map(f64::to_bits);
    let idx = m.indices.iter().map(|&i| u64::from(i));
    let hash = pos
        .chain(idx)
        .fold(0u64, |h, b| h.wrapping_mul(0x100_0000_01b3) ^ b);
    (m.positions.len(), m.indices.len() / 3, hash)
}

/// Which layer is responsible when a sphere-grid union drifts?
///
/// The ladder shows disjoint grids STABLE and overlapping grids not, using
/// one code path for both -- which already rules out the reduction driver.
/// This narrows it further to the smallest possible operand: ONE boolean
/// between TWO spheres, no accumulation, no grouping, no harness reduction.
///
/// If a single overlapping pair drifts, the fault is inside the boolean on
/// intersecting geometry and has nothing to do with many components. If it
/// is stable, the drift needs more than one intersection to appear, and
/// the multi-component hypothesis survives.
pub fn blame_probe(reps: usize) {
    println!("\n\nSphere-grid blame probe -- {reps} identical runs each");
    println!("{}", "-".repeat(104));
    println!("Narrowing from 'a grid of spheres drifts' to the smallest operand that drifts.");
    println!();

    let provider = BoolmeshBoolean::default();
    let options = ExecutionOptions::new(Tolerance::METRE);
    let op = BooleanOperator::Union;

    let row = |label: &str, seen: BTreeSet<(usize, usize, u64)>| {
        let Some(&(v, t, _)) = seen.iter().next() else {
            println!("  {label:<44} no result");
            return;
        };
        match seen.len() {
            1 => println!("  {label:<44} verts={v:<6} tris={t:<6} STABLE"),
            d => {
                let counts: BTreeSet<_> = seen.iter().map(|&(v, t, _)| (v, t)).collect();
                let kind = if counts.len() == 1 {
                    "ordering/value"
                } else {
                    "TOPOLOGY"
                };
                println!(
                    "  {label:<44} verts={v:<6} tris={t:<6} !! NONDETERMINISTIC ({d} distinct, {kind})"
                );
            }
        }
    };

    // Two spheres, overlapping: the smallest operand with a real
    // intersection curve.
    let a = icosphere([0.0, 0.0, 0.0], 1.0, 2);
    let b = icosphere([1.5, 0.0, 0.0], 1.0, 2);
    row(
        "single boolean, 2 overlapping spheres",
        (0..reps)
            .filter_map(|_| {
                provider
                    .boolean(&a, &b, op, &options)
                    .ok()
                    .map(|o| fingerprint(&o.mesh))
            })
            .collect(),
    );

    // Two spheres, far apart: same call, no intersection curve at all.
    let far = icosphere([3.0, 0.0, 0.0], 1.0, 2);
    row(
        "single boolean, 2 disjoint spheres",
        (0..reps)
            .filter_map(|_| {
                provider
                    .boolean(&a, &far, op, &options)
                    .ok()
                    .map(|o| fingerprint(&o.mesh))
            })
            .collect(),
    );

    // Whole 8-sphere grids through the same reduction, for reference.
    for arrangement in [Arrangement::Disjoint, Arrangement::Overlapping] {
        let parts = sphere_grid(2, arrangement, 2);
        row(
            &format!("8-sphere grid, {}, tree", arrangement.label()),
            (0..reps)
                .filter_map(|_| {
                    union_all(&provider, &options, &parts, Strategy::Tree).map(|m| fingerprint(&m))
                })
                .collect(),
        );
    }
}

/// Grid sizes to run, as `k` in a `k^3` grid.
///
/// 2->8, 3->27, 4->64, 5->125, 8->512, 10->1000. Exactly the ladder in the
/// request; `k` is cubed so the counts land on those values naturally.
const LADDER: [usize; 6] = [2, 3, 4, 5, 8, 10];

/// Run the sphere-grid union ladder.
///
/// `reps` identical runs per case feed the determinism column. Density is
/// fixed at 2 subdivisions (320 triangles per sphere): high enough that
/// every intersection is a generic triangle-triangle crossing, low enough
/// that 1000 spheres stays tractable.
pub fn report(reps: usize, max_spheres: usize) {
    println!("\n\nSphere-grid union -- k^3 grids, {reps} identical runs per case");
    println!("{}", "-".repeat(104));
    println!("Disjoint grids: union volume must equal the concatenation's, exactly.");
    println!("Overlapping grids: neighbours fuse, so components collapse toward 1.");
    println!("hash covers positions AND indices -- a position-only hash hides permuted order.");

    let can_reset = reset_peak_rss();
    if !can_reset {
        println!("NOTE: /proc/self/clear_refs refused; peak RSS is cumulative, not per-case.");
    }
    println!();
    println!(
        "{:>7}{:>10}{:>7}{:>6}{:>11}{:>10}{:>9}{:>8}{:>7}  {}",
        "arrange",
        "spheres",
        "strat",
        "comp",
        "union ms",
        "out tris",
        "peak MB",
        "vol err",
        "runs",
        "determinism"
    );

    let provider = BoolmeshBoolean::default();
    let options = ExecutionOptions::new(Tolerance::METRE);

    for arrangement in [Arrangement::Disjoint, Arrangement::Overlapping] {
        for k in LADDER {
            let n = k * k * k;
            if n > max_spheres {
                continue;
            }
            let parts = sphere_grid(k, arrangement, 2);

            // Ground truth for the disjoint case only: a union of solids
            // that do not touch has exactly the volume of their
            // concatenation, and exactly as many components as spheres.
            // `concat` performs no boolean at all, so it is an independent
            // oracle rather than the same code path asked twice. For the
            // overlapping case there is no cheap closed form, so both are
            // reported as "-" rather than against a guessed number.
            let oracle = (arrangement == Arrangement::Disjoint).then(|| {
                let c = concat(&parts);
                (mesh_volume(&c), component_count(&c))
            });
            let want = oracle.map(|(v, _)| v);

            for strategy in [Strategy::Sequential, Strategy::Tree] {
                if can_reset {
                    reset_peak_rss();
                }

                let start = std::time::Instant::now();
                let first = union_all(&provider, &options, &parts, strategy);
                let ms = start.elapsed().as_secs_f64() * 1e3;

                // Absolute peak, not a delta over a pre-run baseline.
                // `clear_refs` resets VmHWM down to the CURRENT RSS, and
                // the allocator reuses an arena already grown by earlier
                // cases, so a union can run entirely inside resident pages
                // and produce a delta of zero -- which is what a delta
                // column actually printed here. The absolute high-water
                // mark after the reset is a real measurement; a delta is
                // an artefact of allocator reuse.
                let peak_mb = peak_rss_kb() as f64 / 1024.0;

                let Some(mesh) = first else {
                    println!(
                        "{:>7}{n:>10}{:>7}{:>6}{:>11}{:>10}{:>9}{:>8}{:>7}  {}",
                        arrangement.label(),
                        strategy.label(),
                        "-",
                        "FAILED",
                        "-",
                        "-",
                        "-",
                        "-",
                        "union returned no mesh"
                    );
                    continue;
                };

                let components = component_count(&mesh);
                let out_tris = mesh.indices.len() / 3;
                let vol_err = want
                    .map(|w| ((mesh_volume(&mesh) - w) / w).abs())
                    .map(|e| format!("{e:.1e}"))
                    .unwrap_or_else(|| "-".to_owned());

                // A disjoint grid must come back with exactly as many
                // components as it went in with. Volume alone cannot catch
                // a union that fused solids which never touched, or one
                // that dropped a sphere and gained one elsewhere.
                let comp_note = match oracle {
                    Some((_, want_c)) if want_c != components => {
                        format!("  !! COMPONENTS want {want_c}")
                    }
                    _ => String::new(),
                };

                // Determinism: repeat the identical call and collect
                // distinct fingerprints. One distinct value means stable.
                let mut seen: BTreeSet<(usize, usize, u64)> = BTreeSet::new();
                seen.insert(fingerprint(&mesh));
                for _ in 1..reps {
                    if let Some(m) = union_all(&provider, &options, &parts, strategy) {
                        seen.insert(fingerprint(&m));
                    }
                }
                let verdict = match seen.len() {
                    1 => "STABLE".to_owned(),
                    d => {
                        let counts: BTreeSet<_> = seen.iter().map(|&(v, t, _)| (v, t)).collect();
                        let kind = if counts.len() == 1 {
                            "ordering/value"
                        } else {
                            "TOPOLOGY"
                        };
                        format!("!! NONDETERMINISTIC ({d} distinct, {kind})")
                    }
                };

                println!(
                    "{:>7}{n:>10}{:>7}{components:>6}{ms:>11.1}{out_tris:>10}{peak_mb:>9.1}{vol_err:>8}{reps:>7}  {verdict}{comp_note}",
                    arrangement.label(),
                    strategy.label(),
                );
            }
        }
    }
}
