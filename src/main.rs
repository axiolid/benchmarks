//! Head-to-head mesh-boolean benchmark: axiolid vs ifc-lite, kernels driven
//! DIRECTLY through their Rust APIs.
//!
//! Why this exists: every previous comparison went through `IfcConvert`, which
//! adds STEP parsing, element mapping, its own `-j N` element threading, and
//! serialisation. Those costs are identical for both kernels but they dominate
//! a sub-200ms run, so they mask the thing being measured. Here the geometry is
//! constructed in-process, handed to each kernel as an equivalent triangle
//! soup, and only the boolean call is timed.
//!
//! Fairness rules:
//!   * Both kernels receive the SAME vertex positions, built once.
//!   * Both are timed on the same wall-clock basis, best-of-N after a warm-up.
//!   * Result volume is reported so a "fast" kernel that cut the wrong amount
//!     is visible rather than silently winning.
//!   * ifc-lite is measured on BOTH its general kernel path and its
//!     `rect_fast` analytic path, because conflating the two is exactly the
//!     mistake that made the earlier IfcConvert comparison misleading.

use std::hint::black_box;
use std::time::Instant;

use axiolid_contracts::ExecutionOptions;
use axiolid_core::{Point3, Tolerance};
use axiolid_mesh::TriMesh;
use axiolid_mesh_boolean_boolmesh::BoolmeshBoolean;
use axiolid_mesh_boolean_contract::MeshBoolean;

use ifc_lite_geometry::mesh::Mesh as LiteMesh;

/// Opt-in analytic fast path (see `cellular.rs`). Never auto-dispatched: the
/// caller chooses fast-vs-exact explicitly, so a run's topology is predictable.
mod cellular;
mod drift;
mod exactness;
mod ops;

// C++ kernels behind a C ABI (`cpp/shim.cpp`). Each takes the same host box
// and flat 8-corner (24 doubles) cutter array every Rust column gets, and returns the
// result VOLUME (never a mesh), so a kernel that fails cannot be mistaken for
// a fast one: failures return a negative volume and are flagged like any other
// wrong answer.
//
// A kernel that is not installed becomes an ABSENT COLUMN, never a fake
// number: each block is cfg-gated on a probe in build.rs.
#[cfg(has_manifold)]
extern "C" {
    fn bench_manifold_subtract(
        host_min: *const f64,
        host_max: *const f64,
        cutters: *const f64,
        n: i32,
    ) -> f64;
    fn bench_manifold_op(
        host_min: *const f64,
        host_max: *const f64,
        operand: *const f64,
        op: i32,
    ) -> f64;
}

#[cfg(has_cgal)]
extern "C" {
    fn bench_cgal_subtract(
        host_min: *const f64,
        host_max: *const f64,
        cutters: *const f64,
        n: i32,
    ) -> f64;
    fn bench_cgal_op(
        host_min: *const f64,
        host_max: *const f64,
        operand: *const f64,
        op: i32,
    ) -> f64;
}

#[cfg(has_occt)]
extern "C" {
    fn bench_occt_subtract(
        host_min: *const f64,
        host_max: *const f64,
        cutters: *const f64,
        n: i32,
    ) -> f64;
    fn bench_occt_op(
        host_min: *const f64,
        host_max: *const f64,
        operand: *const f64,
        op: i32,
    ) -> f64;
}

/// Flatten cutters to the `[min,max]*n` layout the C ABI expects.
fn flat_cutters(openings: &[Obb]) -> Vec<f64> {
    let mut v = Vec::with_capacity(openings.len() * 24);
    for o in openings {
        for c in o.corners() {
            v.extend_from_slice(&c);
        }
    }
    v
}

/// One axis-aligned box, as (center, half-extent-derived) corner bounds.
#[derive(Clone, Copy)]
struct Box3 {
    min: [f64; 3],
    max: [f64; 3],
}

impl Box3 {
    fn new(cx: f64, cy: f64, cz: f64, sx: f64, sy: f64, sz: f64) -> Self {
        Self {
            min: [cx - sx / 2.0, cy - sy / 2.0, cz - sz / 2.0],
            max: [cx + sx / 2.0, cy + sy / 2.0, cz + sz / 2.0],
        }
    }

    /// The 8 corners, in a fixed order shared by both kernels' builders.
    fn corners(&self) -> [[f64; 3]; 8] {
        let (n, x) = (self.min, self.max);
        [
            [n[0], n[1], n[2]],
            [x[0], n[1], n[2]],
            [x[0], x[1], n[2]],
            [n[0], x[1], n[2]],
            [n[0], n[1], x[2]],
            [x[0], n[1], x[2]],
            [x[0], x[1], x[2]],
            [n[0], x[1], x[2]],
        ]
    }
}

/// Outward-wound triangle indices for a box built from `Box3::corners`.
const BOX_TRIS: [u32; 36] = [
    0, 2, 1, 0, 3, 2, 4, 5, 6, 4, 6, 7, 0, 1, 5, 0, 5, 4, 1, 2, 6, 1, 6, 5, 2, 3, 7, 2, 7, 6, 3, 0,
    4, 3, 4, 7,
];

fn axiolid_box(b: Box3) -> TriMesh {
    let positions = b
        .corners()
        .iter()
        .map(|c| Point3::new(c[0], c[1], c[2]))
        .collect();
    TriMesh::new(positions, BOX_TRIS.to_vec())
}

/// Triangle mesh for an oriented box, same index table as [`axiolid_box`].
fn axiolid_obb(o: Obb) -> TriMesh {
    let positions = o
        .corners()
        .iter()
        .map(|c| Point3::new(c[0], c[1], c[2]))
        .collect();
    TriMesh::new(positions, BOX_TRIS.to_vec())
}

/// ifc-lite mesh for an oriented box.
fn lite_obb(o: Obb) -> LiteMesh {
    let mut m = LiteMesh::new();
    for c in o.corners() {
        m.positions.push(c[0] as f32);
        m.positions.push(c[1] as f32);
        m.positions.push(c[2] as f32);
        m.normals.extend_from_slice(&[0.0, 0.0, 1.0]);
    }
    for &i in BOX_TRIS.iter() {
        m.indices.push(i);
    }
    m
}

fn lite_box(b: Box3) -> LiteMesh {
    let mut m = LiteMesh::new();
    for c in b.corners() {
        m.positions.push(c[0] as f32);
        m.positions.push(c[1] as f32);
        m.positions.push(c[2] as f32);
        m.normals.extend_from_slice(&[0.0, 0.0, 1.0]);
    }
    m.indices = BOX_TRIS.to_vec();
    m
}

/// Signed volume of a closed mesh, by the divergence theorem.
///
/// Generic over both kernels' storage: `tris` yields index triples, `vert`
/// resolves an index to a position. One implementation so a difference between
/// columns is a real geometric difference, never a transcription slip.
fn signed_volume(tris: impl Iterator<Item = [u32; 3]>, vert: impl Fn(u32) -> [f64; 3]) -> f64 {
    tris.map(|t| {
        let (a, b, c) = (vert(t[0]), vert(t[1]), vert(t[2]));
        a[0] * (b[1] * c[2] - b[2] * c[1]) - a[1] * (b[0] * c[2] - b[2] * c[0])
            + a[2] * (b[0] * c[1] - b[1] * c[0])
    })
    .sum::<f64>()
        / 6.0
}

/// Index triples from a flat `[i0, i1, i2, ...]` buffer.
fn triples(indices: &[u32]) -> impl Iterator<Item = [u32; 3]> + '_ {
    indices.chunks_exact(3).map(|t| [t[0], t[1], t[2]])
}

fn axiolid_volume(m: &TriMesh) -> f64 {
    signed_volume(triples(&m.indices), |i| {
        let p = m.positions[i as usize];
        [p.x, p.y, p.z]
    })
}

fn lite_volume(m: &LiteMesh) -> f64 {
    signed_volume(triples(&m.indices), |i| {
        let i = i as usize * 3;
        [
            m.positions[i] as f64,
            m.positions[i + 1] as f64,
            m.positions[i + 2] as f64,
        ]
    })
}

/// The same oriented box as a raw `boolmesh::Manifold`.
fn to_manifold_obb(o: Obb) -> boolmesh::prelude::Manifold {
    let mut pos = Vec::with_capacity(24);
    for c in o.corners() {
        pos.extend_from_slice(&c);
    }
    let idx: Vec<usize> = BOX_TRIS.iter().map(|&i| i as usize).collect();
    boolmesh::prelude::Manifold::new(&pos, &idx).expect("raw boolmesh manifold")
}

/// The same box as a raw `boolmesh::Manifold`, bypassing axiolid's provider.
fn to_manifold_box(b: Box3) -> boolmesh::prelude::Manifold {
    let mut pos = Vec::with_capacity(24);
    for c in b.corners() {
        pos.extend_from_slice(&c);
    }
    let idx: Vec<usize> = BOX_TRIS.iter().map(|&i| i as usize).collect();
    boolmesh::prelude::Manifold::new(&pos, &idx).expect("raw boolmesh manifold")
}

/// The IFC-dominant case: one wall, N disjoint rectangular openings.
fn wall_and_openings(n: usize) -> (Box3, Vec<Box3>) {
    let length = n as f64 + 1.0;
    let wall = Box3::new(length / 2.0, 0.1, 1.5, length, 0.2, 3.0);
    let openings = (0..n)
        .map(|i| Box3::new(0.75 + i as f64, 0.1, 1.0, 0.5, 0.5, 1.0))
        .collect();
    (wall, openings)
}

/// A wall whose openings sit FLUSH against its top and bottom faces, so the
/// cut planes are COINCIDENT with host faces.
///
/// Real IFC hits this constantly: a door at floor level, a window flush with a
/// slab. Coincident faces are the dominant source of cross-kernel disagreement,
/// because each kernel has to decide whether a zero-thickness contact is inside
/// or outside. The regular workload deliberately avoids the case, so nothing in
/// the table currently exercises it.
///
/// The opening spans the wall's full height, so the result is the wall split
/// into `n + 1` disjoint pillars -- which also makes this the only workload
/// where a kernel's component handling is visible.
fn wall_with_flush_openings(n: usize) -> (Box3, Vec<Box3>) {
    let length = n as f64 + 1.0;
    let wall = Box3::new(length / 2.0, 0.1, 1.5, length, 0.2, 3.0);
    // Height 3.0 centred at 1.5 == exactly the wall's z-range: the top and
    // bottom cut planes land ON the host's faces rather than inside it.
    let openings = (0..n)
        .map(|i| Box3::new(0.75 + i as f64, 0.1, 1.5, 0.5, 0.5, 3.0))
        .collect();
    (wall, openings)
}

/// Ground truth for [`wall_with_flush_openings`], derived not measured.
///
/// Each opening removes a full-height slot: `0.5 * wall_thickness * 3.0`. The
/// flush contact removes no extra material, so a kernel that reports less has
/// treated a coincident face as an overlap.
fn expected_flush_volume(n: usize) -> f64 {
    let wall = (n as f64 + 1.0) * 0.2 * 3.0;
    wall - n as f64 * (0.5 * 0.2 * 3.0)
}

/// A wall whose openings are ROTATED in plan, so no cut plane is axis-aligned.
///
/// Every analytic fast path here (`rect_fast`, `cellular`) is defined only for
/// axis-aligned operands, so this workload forces them to decline and exposes
/// what the general solver actually costs. That is the point: the headline
/// speedups on the other two workloads are real but conditional, and a chart
/// that never shows the condition failing overstates them.
///
/// The rotation is 30 degrees, well away from a multiple of 90 where the boxes
/// would be accidentally axis-aligned again.
fn wall_with_rotated_openings(n: usize) -> (Box3, Vec<Obb>) {
    let length = n as f64 + 1.0;
    let wall = Box3::new(length / 2.0, 0.1, 1.5, length, 0.2, 3.0);
    let openings = (0..n)
        .map(|i| Obb {
            centre: [0.75 + i as f64, 0.1, 1.0],
            half: [0.25, 0.5, 0.5],
            angle: std::f64::consts::FRAC_PI_6,
        })
        .collect();
    (wall, openings)
}

/// Signed area of a polygon by the shoelace formula.
fn shoelace(poly: &[[f64; 2]]) -> f64 {
    let mut acc = 0.0;
    for i in 0..poly.len() {
        let a = poly[i];
        let b = poly[(i + 1) % poly.len()];
        acc += a[0] * b[1] - b[0] * a[1];
    }
    0.5 * acc
}

/// Clip a convex polygon to the half-plane `keep`, Sutherland-Hodgman.
fn clip_half_plane(poly: &[[f64; 2]], keep: impl Fn([f64; 2]) -> f64) -> Vec<[f64; 2]> {
    let mut out: Vec<[f64; 2]> = Vec::new();
    for i in 0..poly.len() {
        let cur = poly[i];
        let prev = poly[(i + poly.len() - 1) % poly.len()];
        let (dc, dp) = (keep(cur), keep(prev));
        // Crossing the boundary contributes the intersection point; `dp - dc`
        // is non-zero exactly when the sign differs, so this cannot divide by 0.
        if (dc >= 0.0) != (dp >= 0.0) {
            let t = dp / (dp - dc);
            out.push([
                prev[0] + t * (cur[0] - prev[0]),
                prev[1] + t * (cur[1] - prev[1]),
            ]);
        }
        if dc >= 0.0 {
            out.push(cur);
        }
    }
    out
}

/// Ground truth for [`wall_with_rotated_openings`], derived not measured.
///
/// Each opening removes (its footprint clipped to the wall's XY rectangle)
/// times (its Z overlap with the wall). The footprint is a rotated rectangle,
/// so the clipped area is computed exactly rather than assumed: at 30 degrees
/// the opening is wider than the 0.2 wall in Y, so a naive `w * d * h` would
/// be wrong. Openings are spaced 1.0 apart with a 0.5 diagonal half-extent, so
/// they never touch each other and the removed volumes simply sum.
fn expected_rotated_volume(n: usize) -> f64 {
    let (wall, openings) = wall_with_rotated_openings(n);
    let wall_volume =
        (wall.max[0] - wall.min[0]) * (wall.max[1] - wall.min[1]) * (wall.max[2] - wall.min[2]);

    let mut removed = 0.0;
    for o in &openings {
        let mut poly: Vec<[f64; 2]> = o.footprint().to_vec();
        poly = clip_half_plane(&poly, |p| p[0] - wall.min[0]);
        poly = clip_half_plane(&poly, |p| wall.max[0] - p[0]);
        poly = clip_half_plane(&poly, |p| p[1] - wall.min[1]);
        poly = clip_half_plane(&poly, |p| wall.max[1] - p[1]);
        if poly.len() < 3 {
            continue;
        }
        let area = shoelace(&poly).abs();
        let z_lo = (o.centre[2] - o.half[2]).max(wall.min[2]);
        let z_hi = (o.centre[2] + o.half[2]).min(wall.max[2]);
        removed += area * (z_hi - z_lo).max(0.0);
    }
    wall_volume - removed
}

/// A box that may be rotated about the Z axis through its own centre.
///
/// IFC openings are not always axis-aligned: a wall skewed in plan carries
/// openings skewed with it. `angle == 0` is the axis-aligned case and is still
/// exactly representable, so one type covers every workload here.
#[derive(Clone, Copy)]
struct Obb {
    centre: [f64; 3],
    half: [f64; 3],
    /// Radians about +Z through `centre`.
    angle: f64,
}

impl Obb {
    /// Lift an axis-aligned box. Kept exact: no rotation is applied.
    fn aabb(b: Box3) -> Self {
        Self {
            centre: [
                0.5 * (b.min[0] + b.max[0]),
                0.5 * (b.min[1] + b.max[1]),
                0.5 * (b.min[2] + b.max[2]),
            ],
            half: [
                0.5 * (b.max[0] - b.min[0]),
                0.5 * (b.max[1] - b.min[1]),
                0.5 * (b.max[2] - b.min[2]),
            ],
            angle: 0.0,
        }
    }

    /// The axis-aligned box this represents, or `None` when rotated.
    ///
    /// Analytic fast paths are only valid for axis-aligned operands, so they
    /// ask through this and decline rather than cutting the wrong solid.
    fn as_aabb(&self) -> Option<Box3> {
        if self.angle != 0.0 {
            return None;
        }
        Some(Box3 {
            min: [
                self.centre[0] - self.half[0],
                self.centre[1] - self.half[1],
                self.centre[2] - self.half[2],
            ],
            max: [
                self.centre[0] + self.half[0],
                self.centre[1] + self.half[1],
                self.centre[2] + self.half[2],
            ],
        })
    }

    /// The 8 corners, in the SAME order as [`Box3::corners`] so both share one
    /// triangle index table and no kernel sees a different winding.
    fn corners(&self) -> [[f64; 3]; 8] {
        let (sin, cos) = self.angle.sin_cos();
        let (hx, hy, hz) = (self.half[0], self.half[1], self.half[2]);
        let mut out = [[0.0; 3]; 8];
        let mut i = 0;
        for &dz in &[-hz, hz] {
            for &(dx, dy) in &[(-hx, -hy), (hx, -hy), (hx, hy), (-hx, hy)] {
                out[i] = [
                    self.centre[0] + dx * cos - dy * sin,
                    self.centre[1] + dx * sin + dy * cos,
                    self.centre[2] + dz,
                ];
                i += 1;
            }
        }
        out
    }

    /// Footprint in the XY plane, counter-clockwise, for exact area work.
    fn footprint(&self) -> [[f64; 2]; 4] {
        let c = self.corners();
        [
            [c[0][0], c[0][1]],
            [c[1][0], c[1][1]],
            [c[2][0], c[2][1]],
            [c[3][0], c[3][1]],
        ]
    }
}

/// Ground truth for [`wall_and_openings`], derived not measured.
///
/// The openings are disjoint, fully pierce the wall in x/z, and are wider than
/// the wall in y, so each removes exactly `0.5 * wall_thickness * 1.0`. Any
/// kernel disagreeing with this cut the wrong solid -- which is the failure the
/// `lite-kernel` column exhibits at n=64 (returns 0.0 in 0.2ms and would
/// otherwise look like a win).
fn expected_volume(n: usize) -> f64 {
    let wall = (n as f64 + 1.0) * 0.2 * 3.0;
    wall - n as f64 * (0.5 * 0.2 * 1.0)
}

/// Best-of-N wall-clock in milliseconds, after one warm-up.
fn best_of<T, F: FnMut() -> T>(reps: usize, mut f: F) -> (f64, T) {
    let mut out = f(); // warm-up, discarded
    let mut best = f64::MAX;
    for _ in 0..reps {
        let start = Instant::now();
        out = black_box(f());
        best = best.min(start.elapsed().as_secs_f64() * 1e3);
    }
    (best, out)
}

/// Mesh identity: (vertices, triangles, hash over positions AND indices).
///
/// Every path must fingerprint the same data. Hashing positions alone hides a
/// permuted triangle order and reports a false STABLE — that mistake once
/// inverted this probe's conclusion, so there is exactly one shape of
/// fingerprint here and both constructors below feed it identically.
type Fp = (usize, usize, u64);

fn fnv(bits: impl Iterator<Item = u64>) -> u64 {
    bits.fold(0u64, |h, b| h.wrapping_mul(0x100_0000_01b3) ^ b)
}

fn trimesh_fp(m: &TriMesh) -> Fp {
    let pos = m
        .positions
        .iter()
        .flat_map(|p| [p.x, p.y, p.z])
        .map(f64::to_bits);
    let idx = m.indices.iter().map(|&i| u64::from(i));
    (m.positions.len(), m.indices.len() / 3, fnv(pos.chain(idx)))
}

fn manifold_fp(m: &boolmesh::prelude::Manifold) -> Fp {
    let tris = m.get_indices();
    let pos = m.ps.iter().flat_map(|p| [p.x, p.y, p.z]).map(f64::to_bits);
    let idx = tris.iter().flat_map(|t| [t.x, t.y, t.z]).map(|i| i as u64);
    (m.ps.len(), tris.len(), fnv(pos.chain(idx)))
}

/// Distinct results across `reps` identical runs. One = deterministic.
fn distinct(reps: usize, mut f: impl FnMut() -> Fp) -> std::collections::BTreeSet<Fp> {
    (0..reps).map(|_| f()).collect()
}

/// Print one probe row, classifying any instability.
fn verdict(label: &str, seen: &std::collections::BTreeSet<Fp>, blame: &str) {
    let Some(&(v, t, _)) = seen.iter().next() else {
        return;
    };
    match seen.len() {
        1 => println!("  {label:<32} verts={v:<6} tris={t:<6} STABLE"),
        n => {
            // Same counts + different hash means ordering drift; differing
            // counts would mean the topology itself is unstable, which is worse.
            let kind = if seen
                .iter()
                .map(|&(v, t, _)| (v, t))
                .collect::<std::collections::BTreeSet<_>>()
                .len()
                == 1
            {
                "ordering/value"
            } else {
                "TOPOLOGY"
            };
            println!("  {label:<32} verts={v:<6} tris={t:<6} !! NONDETERMINISTIC ({n} distinct, {kind}){blame}");
        }
    }
}

/// Determinism probe: identical input, repeated runs, per-layer blame.
///
/// `IfcConvert --kernel axiolid` yields different vertex counts across
/// identical runs (9664/9665) where `--kernel manifold` is stable. That was
/// observed through IfcOpenShell, so it could not separate an axiolid/boolmesh
/// fault from IfcOpenShell's own triangulation. These four layers call the
/// kernels directly and isolate which one actually drifts.
fn determinism_probe(reps: usize) {
    println!("\n\nDeterminism probe -- same input, {reps} identical runs each");
    println!("{}", "-".repeat(80));

    let provider = BoolmeshBoolean::new();
    let options = ExecutionOptions::new(Tolerance::MILLIMETRE);
    // `OpType` is not `Copy`, so it is constructed per call rather than hoisted.
    let sub = || boolmesh::prelude::OpType::Subtract;

    for &n in &[16usize, 64] {
        let (wall, openings) = wall_and_openings(n);
        let host = axiolid_box(wall);
        let tools: Vec<TriMesh> = openings.iter().map(|o| axiolid_box(*o)).collect();
        let raw_host = to_manifold_box(wall);
        let raw_tools: Vec<_> = openings.iter().map(|o| to_manifold_box(*o)).collect();

        // The multi-component tool `subtract_many` builds internally, assembled
        // here so raw boolmesh can be handed the identical operand.
        let fused = {
            let (mut pos, mut idx) = (Vec::new(), Vec::<usize>::new());
            for o in &openings {
                let base = pos.len() / 3;
                pos.extend(o.corners().iter().flatten());
                idx.extend(BOX_TRIS.iter().map(|&i| i as usize + base));
            }
            boolmesh::prelude::Manifold::new(&pos, &idx).expect("fused manifold")
        };

        println!("\n  n={n}");
        verdict(
            "axiolid subtract_many (grouped)",
            &distinct(reps, || {
                trimesh_fp(
                    &provider
                        .subtract_many(&host, &tools, &options)
                        .expect("many")
                        .mesh,
                )
            }),
            "",
        );
        verdict(
            "axiolid single boolean",
            &distinct(reps, || {
                let op = axiolid_core::BooleanOperator::Difference;
                trimesh_fp(
                    &provider
                        .boolean(&host, &tools[0], op, &options)
                        .expect("one")
                        .mesh,
                )
            }),
            "",
        );
        verdict(
            "raw boolmesh (sequential)",
            &distinct(reps, || {
                let acc = raw_tools.iter().fold(raw_host.clone(), |a, t| {
                    boolmesh::prelude::compute_boolean(&a, t, sub()).expect("raw")
                });
                manifold_fp(&acc)
            }),
            "",
        );
        // Decisive: same disconnected operand, axiolid bypassed entirely.
        verdict(
            "raw boolmesh (FUSED tool)",
            &distinct(reps, || {
                manifold_fp(
                    &boolmesh::prelude::compute_boolean(&raw_host, &fused, sub()).expect("fused"),
                )
            }),
            "  <-- upstream boolmesh",
        );
        // The analytic path's determinism claim, tested rather than asserted:
        // ordered (BTreeMap) vertex identity should be byte-reproducible where
        // the general boolean is not.
        let cboxes: Vec<([f64; 3], [f64; 3])> = openings.iter().map(|o| (o.min, o.max)).collect();
        verdict(
            "cellular (analytic, opt-in)",
            &distinct(reps, || {
                let c =
                    cellular::subtract_boxes((wall.min, wall.max), &cboxes, 1 << 20).expect("cell");
                let pos = c.positions.iter().flatten().copied().map(f64::to_bits);
                let idx = c.indices.iter().map(|&i| u64::from(i));
                (c.positions.len(), c.indices.len() / 3, fnv(pos.chain(idx)))
            }),
            "",
        );
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    // `--json` emits machine-readable rows for the viewer server. The human
    // table and the JSON come from the SAME measurement run, so the UI can
    // never drift from the numbers printed here.
    let json = args.iter().any(|a| a == "--json");
    let reps: usize = args.iter().find_map(|a| a.parse().ok()).unwrap_or(5);
    let mut rows: Vec<String> = Vec::new();

    let provider = BoolmeshBoolean::new();
    let options = ExecutionOptions::new(Tolerance::MILLIMETRE);

    if !json {
        println!("Direct kernel benchmark -- no IfcConvert, no file I/O, no STEP parsing");
        println!("wall with N disjoint rectangular openings, best-of-{reps}\n");
    }
    if !json {
        println!(
            "{:>4}  {:>11}  {:>11}  {:>11}  {:>11}  {:>11}  {:>11}  {:>11}  {:>11}",
            "n",
            "axiolid",
            "raw bmesh",
            "lite-kernel",
            "lite-rectfast",
            "cellular",
            "manifold",
            "cgal",
            "occt"
        );
        println!("{}", "-".repeat(104));
    }
    let mut wrong = 0usize;

    // (label, generator, ground truth). The flush variant forces coincident
    // cut planes, which is where kernels genuinely disagree.
    type Workload = (
        &'static str,
        fn(usize) -> (Box3, Vec<Obb>),
        fn(usize) -> f64,
    );
    fn offset_obb(n: usize) -> (Box3, Vec<Obb>) {
        let (w, o) = wall_and_openings(n);
        (w, o.into_iter().map(Obb::aabb).collect())
    }
    fn flush_obb(n: usize) -> (Box3, Vec<Obb>) {
        let (w, o) = wall_with_flush_openings(n);
        (w, o.into_iter().map(Obb::aabb).collect())
    }
    let workloads: [Workload; 3] = [
        ("offset", offset_obb, expected_volume),
        ("flush", flush_obb, expected_flush_volume),
        (
            "rotated",
            wall_with_rotated_openings,
            expected_rotated_volume,
        ),
    ];

    for (workload, generate, ground_truth) in workloads {
        if !json && workload == "flush" {
            println!("\n-- coincident cut planes (openings flush with the wall's faces) --");
        }
        if !json && workload == "rotated" {
            println!("\n-- rotated openings (30 deg in plan; analytic paths must decline) --");
        }
        for &n in &[1usize, 4, 16, 64] {
            let (wall, openings) = generate(n);

            // --- axiolid: subtract_many through the provider contract ---
            let ax_host = axiolid_box(wall);
            let ax_tools: Vec<TriMesh> = openings.iter().map(|o| axiolid_obb(*o)).collect();
            let (ax_ms, ax_out) = best_of(reps, || {
                provider
                    .subtract_many(&ax_host, &ax_tools, &options)
                    .expect("axiolid subtract_many")
                    .mesh
            });
            let ax_vol = axiolid_volume(&ax_out);

            // --- ifc-lite: general exact mesh-arrangement kernel ---
            let lite_host = lite_box(wall);
            let lite_tools: Vec<LiteMesh> = openings.iter().map(|o| lite_obb(*o)).collect();
            let (lk_ms, lk_vol) = {
                let refs: Vec<&LiteMesh> = lite_tools.iter().collect();
                let (ms, out) = best_of(reps, || {
                    ifc_lite_geometry::kernel::mesh_bridge::subtract_many(&lite_host, &refs)
                });
                match out {
                    Some(m) => (ms, Some(lite_volume(&m))),
                    None => (ms, None),
                }
            };

            // --- ifc-lite: analytic rect_fast cellular path ---
            // `None` as soon as any opening is rotated: both analytic paths are
            // defined only for axis-aligned operands. Declining is the correct
            // answer, and the table renders it as `n/a` rather than a timing.
            let boxes: Option<Vec<([f64; 3], [f64; 3])>> = openings
                .iter()
                .map(|o| o.as_aabb().map(|b| (b.min, b.max)))
                .collect();
            let (rf_ms, rf_vol) = match &boxes {
                None => (f64::NAN, None),
                Some(boxes) => {
                    let (ms, out) = best_of(reps, || {
                        let mut stats = ifc_lite_geometry::RectFastStats::default();
                        ifc_lite_geometry::rect_fast::subtract_rect_openings(
                            &lite_host, boxes, &mut stats,
                        )
                    });
                    match out {
                        Some(m) => (ms, Some(lite_volume(&m))),
                        None => (ms, None),
                    }
                }
            };

            // --- axiolid-side analytic path, opt-in (never auto-dispatched).
            // Same grid decomposition idea as rect_fast, but built on ordered
            // (BTreeMap) vertex identity so its output is byte-reproducible --
            // unlike the general boolean path, which inherits upstream boolmesh's
            // HashMap-seeded instability.
            let (cell_ms, cell_vol) = match &boxes {
                None => (f64::NAN, None),
                Some(boxes) => {
                    let (ms, out) = best_of(reps, || {
                        cellular::subtract_boxes((wall.min, wall.max), boxes, 1 << 20)
                    });
                    match out {
                        Some(c) => (
                            ms,
                            Some(signed_volume(triples(&c.indices), |i| {
                                c.positions[i as usize]
                            })),
                        ),
                        None => (ms, None),
                    }
                }
            };

            // --- C++ kernels through the C ABI. Same boxes, same order.
            // These return a VOLUME, not a mesh: the C++ kernels use their own
            // native representations (Manifold's halfedge mesh, CGAL's Surface_mesh
            // over an exact kernel), and marshalling those back into TriMesh would
            // charge them a conversion cost no other column pays. Volume is the one
            // quantity every kernel computes natively and that the ground-truth
            // check already relies on.
            let flat = flat_cutters(&openings);
            type CppFn = unsafe extern "C" fn(*const f64, *const f64, *const f64, i32) -> f64;
            let cpp_col = |f: CppFn| -> (f64, Option<f64>) {
                let (ms, v) = best_of(reps, || unsafe {
                    f(
                        wall.min.as_ptr(),
                        wall.max.as_ptr(),
                        flat.as_ptr(),
                        openings.len() as i32,
                    )
                });
                // Negative volume is the shims' failure signal; surface it as a
                // wrong answer rather than letting it read as a fast success.
                (ms, Some(v))
            };
            #[cfg(has_manifold)]
            let (mf_ms, mf_vol) = cpp_col(bench_manifold_subtract);
            #[cfg(not(has_manifold))]
            let (mf_ms, mf_vol) = (f64::NAN, None);
            #[cfg(has_cgal)]
            let (cg_ms, cg_vol) = cpp_col(bench_cgal_subtract);
            #[cfg(not(has_cgal))]
            let (cg_ms, cg_vol) = (f64::NAN, None);
            #[cfg(has_occt)]
            let (oc_ms, oc_vol) = cpp_col(bench_occt_subtract);
            #[cfg(not(has_occt))]
            let (oc_ms, oc_vol) = (f64::NAN, None);
            let _ = &cpp_col;

            // --- raw boolmesh: the same backend axiolid wraps, called directly.
            // The delta against `axiolid` above is the provider's own cost:
            // input validation, TriMesh<->boolmesh conversion, result orientation
            // checking, and evidence/connected-component construction.
            let (raw_ms, raw_vol) = {
                let host = to_manifold_box(wall);
                let tools: Vec<_> = openings.iter().map(|o| to_manifold_obb(*o)).collect();
                let (ms, out) = best_of(reps, || {
                    let mut acc = host.clone();
                    for t in &tools {
                        acc = boolmesh::prelude::compute_boolean(
                            &acc,
                            t,
                            boolmesh::prelude::OpType::Subtract,
                        )
                        .expect("raw boolmesh subtract");
                    }
                    acc
                });
                let v = signed_volume(
                    out.get_indices()
                        .iter()
                        .map(|t| [t.x as u32, t.y as u32, t.z as u32]),
                    |i| {
                        let q = out.ps[i as usize];
                        [q.x, q.y, q.z]
                    },
                );
                (ms, v)
            };

            let want = ground_truth(n);
            // Correctness gates the timing. A kernel that returned an empty
            // mesh in 5us would otherwise read as the fastest in the table.
            let agrees = |v: Option<f64>| match v {
                Some(v) => (v - want).abs() <= 1e-4 * want.abs().max(1.0),
                None => false,
            };
            let fmt = |ms: f64, v: Option<f64>| {
                if agrees(v) {
                    format!("{ms:.3} ms")
                } else if v.is_none() {
                    "deferred".to_owned()
                } else {
                    "WRONG".to_owned()
                }
            };
            if !json {
                println!(
                    "{n:>4}  {:>11}  {:>11}  {:>11}  {:>11}  {:>11}  {:>11}  {:>11}  {:>11}",
                    format!("{ax_ms:.3} ms"),
                    format!("{raw_ms:.3} ms"),
                    fmt(lk_ms, lk_vol),
                    fmt(rf_ms, rf_vol),
                    fmt(cell_ms, cell_vol),
                    fmt(mf_ms, mf_vol),
                    fmt(cg_ms, cg_vol),
                    fmt(oc_ms, oc_vol),
                );
            }
            // `null` where a kernel declined or is not compiled in -- the UI must
            // render an absence as an absence, never as a zero.
            let j = |ms: f64, v: Option<f64>| {
                if agrees(v) && ms.is_finite() {
                    format!("{ms:.4}")
                } else {
                    "null".to_owned()
                }
            };
            rows.push(format!(
            "{{\"workload\":\"{workload}\",\"n\":{n},\"axiolid\":{},\"raw_boolmesh\":{},\"lite_kernel\":{},\"lite_rectfast\":{},\"cellular\":{},\"manifold\":{},\"cgal\":{},\"occt\":{}}}",
            j(ax_ms, Some(ax_vol)),
            j(raw_ms, Some(raw_vol)),
            j(lk_ms, lk_vol),
            j(rf_ms, rf_vol),
            j(cell_ms, cell_vol),
            j(mf_ms, mf_vol),
            j(cg_ms, cg_vol),
            j(oc_ms, oc_vol),
        ));

            // Volume agreement against DERIVED ground truth, not against axiolid:
            // anchoring on one kernel would let a shared error pass silently.
            // ifc-lite stores positions as f32, so parity is checked at f32
            // resolution -- a tighter bound flags representation noise as a bug.
            let mut check = |label: &str, v: Option<f64>| match v {
                Some(v) if (v - want).abs() > 1e-4 * want.abs().max(1.0) => {
                    // stderr: keeps `--json` stdout machine-parseable while the
                    // warning still surfaces in logs and the human table.
                    eprintln!("        !! {label} volume {v:.6}, expected {want:.6}");
                    wrong += 1;
                }
                _ => {}
            };
            check("axiolid", Some(ax_vol));
            check("raw boolmesh", Some(raw_vol));
            check("lite-kernel", lk_vol);
            check("lite-rectfast", rf_vol);
            check("cellular", cell_vol);
            check("manifold", mf_vol);
            check("cgal", cg_vol);
            check("occt", oc_vol);
        }
    }

    if json {
        // Which kernels this binary can actually measure. The UI labels absent
        // kernels explicitly rather than silently omitting them, so a missing
        // dependency is visible instead of looking like a kernel that lost.
        let built: Vec<&str> = [
            Some("axiolid"),
            Some("raw_boolmesh"),
            Some("lite_kernel"),
            Some("lite_rectfast"),
            Some("cellular"),
            cfg!(has_manifold).then_some("manifold"),
            cfg!(has_cgal).then_some("cgal"),
            cfg!(has_occt).then_some("occt"),
        ]
        .into_iter()
        .flatten()
        .collect();
        let exact = exactness::report(true);
        println!(
            "{{\"reps\":{reps},\"mismatches\":{wrong},\"built\":[{}],\"rows\":[{}],\"exactness\":[{}]}}",
            built
                .iter()
                .map(|k| format!("\"{k}\""))
                .collect::<Vec<_>>()
                .join(","),
            rows.join(","),
            exact.join(",")
        );
        return;
    }

    exactness::report(false);
    drift::drift_report();

    if wrong > 0 {
        println!("\n{wrong} volume mismatch(es) -- timings above are not comparable.");
        determinism_probe(20);
        std::process::exit(1);
    }
    println!("\nall reported results match derived ground truth.");
    determinism_probe(20);
}
