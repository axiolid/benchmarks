//! Adversarial grid: O(n) input triangles, O(n^3) intersections.
//!
//! Three families of n axis-aligned slabs, one per axis. Their triple
//! intersection is n^3 disjoint boxes, so the oracle is analytic:
//! volume = n^3 * tx*ty*tz, components = n^3, chi = 2*n^3.
//!
//! Input grows O(n) while output grows O(n^3), which is the crash
//! test: a kernel can pass every small fixture and still fall over
//! here. Generated parametrically, so it carries no third-party
//! licence, unlike a downloaded corpus model.

use crate::ops::Metrics;
use crate::{axiolid_obb, Box3, Obb};
use axiolid_contracts::ExecutionOptions;
use axiolid_core::{BooleanOperator, Tolerance};
use axiolid_mesh::TriMesh;
use axiolid_mesh_boolean_boolmesh::BoolmeshBoolean;
use axiolid_mesh_boolean_contract::MeshBoolean;

/// Slab thickness and the gap between slabs.
const T: f64 = 0.4;
const GAP: f64 = 1.0;

/// One family of n slabs, unioned into a single comb along `axis`.
///
/// The slabs are disjoint, so the union is n components and the
/// comb itself needs no intersection handling: all the pressure
/// comes from intersecting the three combs with each other.
fn comb(n: usize, axis: usize, span: f64) -> Option<TriMesh> {
    let provider = BoolmeshBoolean::new();
    let options = ExecutionOptions::new(Tolerance::MILLIMETRE);
    let mut acc: Option<TriMesh> = None;
    for i in 0..n {
        let c = GAP * (i as f64) + T / 2.0;
        let mut centre = [span / 2.0; 3];
        let mut half = [span; 3];
        centre[axis] = c;
        half[axis] = T;
        let slab = axiolid_obb(Obb::aabb(Box3::new(
            centre[0], centre[1], centre[2], half[0], half[1], half[2],
        )));
        acc = Some(match acc {
            None => slab,
            Some(prev) => {
                provider
                    .boolean(&prev, &slab, BooleanOperator::Union, &options)
                    .ok()?
                    .mesh
            }
        });
    }
    acc
}

/// Intersect the three combs: the O(n^3) explosion.
fn grid(n: usize) -> Option<TriMesh> {
    let provider = BoolmeshBoolean::new();
    let options = ExecutionOptions::new(Tolerance::MILLIMETRE);
    let span = GAP * (n as f64) + T;
    let x = comb(n, 0, span)?;
    let y = comb(n, 1, span)?;
    let z = comb(n, 2, span)?;
    let xy = provider
        .boolean(&x, &y, BooleanOperator::Intersection, &options)
        .ok()?
        .mesh;
    let xyz = provider
        .boolean(&xy, &z, BooleanOperator::Intersection, &options)
        .ok()?
        .mesh;
    Some(xyz)
}

/// Score the grid against its analytic oracle.
///
/// Returns the fault count so the caller can wire it into the
/// process exit code, like every other invariant fixture.
pub fn report() -> usize {
    println!();
    println!();
    println!("Adversarial grid -- O(n) input triangles, O(n^3) cells");
    println!("{}", "-".repeat(80));
    println!("n slabs per axis, intersected three ways. Oracle: n^3 cells, chi = 2n^3.");
    println!();
    println!(
        "{:>4}  {:>9}  {:>9}  {:>11}  {:>9}  {:>8}",
        "n", "cells", "comps", "vol err", "chi", "verdict"
    );

    let mut faults = 0usize;
    for n in [2usize, 3, 4, 5, 6] {
        let cells = n * n * n;
        let want_vol = (cells as f64) * T * T * T;
        let want_chi = 2 * cells as i64;
        let Some(mesh) = grid(n) else {
            println!(
                "{n:>4}  {cells:>9}  {:>9}  {:>11}  {:>9}  {:>8}",
                "-", "-", "-", "REFUSED"
            );
            faults += 1;
            continue;
        };
        let m: Metrics = crate::exactness::axiolid_metrics(&mesh);
        let comps = m.components.unwrap_or(0);
        let chi = m.euler.unwrap_or(0);
        let vol_err = (m.volume - want_vol).abs() / want_vol;
        let bad = comps != cells || chi != want_chi || vol_err > 1e-9;
        faults += usize::from(bad);
        let mark = if bad { "!!" } else { "ok" };
        println!("{n:>4}  {cells:>9}  {comps:>9}  {vol_err:>11.2e}  {chi:>9}  {mark:>8}");
    }

    println!();
    if faults == 0 {
        println!("  grid matches its analytic oracle at every n.");
    } else {
        println!("  {faults} grid fault(s): the O(n^3) case diverged from ground truth.");
    }
    faults
}
