//! Dumps the two icosphere operands used by `sphere.rs`'s union benchmark to
//! raw binary files, so a standalone C++ harness can run Manifold's own
//! boolean under `perf record` without the multi-kernel process this
//! benchmark's main binary interleaves three codebases inside.
use axiolid_core::Point3;
use axiolid_mesh::TriMesh;
use std::collections::HashMap;

/// Copied from `sphere.rs::icosphere` rather than shared via a module: that
/// file pulls `crate::best_of` from the main binary's root, which a separate
/// bin target does not have. This is pure geometry with no other
/// dependency, so duplicating it here is safer than restructuring the
/// module boundary for a throwaway dump tool.
fn icosphere(center: [f64; 3], radius: f64, subdivisions: u32) -> TriMesh {
    let t = (1.0 + 5.0f64.sqrt()) / 2.0;
    let mut verts: Vec<[f64; 3]> = vec![
        [-1.0, t, 0.0],
        [1.0, t, 0.0],
        [-1.0, -t, 0.0],
        [1.0, -t, 0.0],
        [0.0, -1.0, t],
        [0.0, 1.0, t],
        [0.0, -1.0, -t],
        [0.0, 1.0, -t],
        [t, 0.0, -1.0],
        [t, 0.0, 1.0],
        [-t, 0.0, -1.0],
        [-t, 0.0, 1.0],
    ];
    let mut faces: Vec<[u32; 3]> = vec![
        [0, 11, 5],
        [0, 5, 1],
        [0, 1, 7],
        [0, 7, 10],
        [0, 10, 11],
        [1, 5, 9],
        [5, 11, 4],
        [11, 10, 2],
        [10, 7, 6],
        [7, 1, 8],
        [3, 9, 4],
        [3, 4, 2],
        [3, 2, 6],
        [3, 6, 8],
        [3, 8, 9],
        [4, 9, 5],
        [2, 4, 11],
        [6, 2, 10],
        [8, 6, 7],
        [9, 8, 1],
    ];

    for _ in 0..subdivisions {
        let mut midpoint: HashMap<(u32, u32), u32> = HashMap::new();
        let mut next: Vec<[u32; 3]> = Vec::with_capacity(faces.len() * 4);
        for f in &faces {
            let mut mid = [0u32; 3];
            for e in 0..3 {
                let (a, b) = (f[e], f[(e + 1) % 3]);
                let key = (a.min(b), a.max(b));
                mid[e] = *midpoint.entry(key).or_insert_with(|| {
                    let (pa, pb) = (verts[a as usize], verts[b as usize]);
                    verts.push([
                        (pa[0] + pb[0]) * 0.5,
                        (pa[1] + pb[1]) * 0.5,
                        (pa[2] + pb[2]) * 0.5,
                    ]);
                    (verts.len() - 1) as u32
                });
            }
            next.push([f[0], mid[0], mid[2]]);
            next.push([f[1], mid[1], mid[0]]);
            next.push([f[2], mid[2], mid[1]]);
            next.push([mid[0], mid[1], mid[2]]);
        }
        faces = next;
    }

    let positions: Vec<Point3> = verts
        .iter()
        .map(|v| {
            let len = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
            let k = radius / len;
            Point3::new(
                center[0] + v[0] * k,
                center[1] + v[1] * k,
                center[2] + v[2] * k,
            )
        })
        .collect();
    let indices: Vec<u32> = faces.iter().flat_map(|f| [f[0], f[1], f[2]]).collect();
    TriMesh::new(positions, indices)
}

fn write_mesh(path: &str, mesh: &TriMesh) {
    use std::io::Write;
    let mut f = std::fs::File::create(path).unwrap();
    f.write_all(&(mesh.positions.len() as u32).to_le_bytes())
        .unwrap();
    f.write_all(&((mesh.indices.len() / 3) as u32).to_le_bytes())
        .unwrap();
    for v in &mesh.positions {
        f.write_all(&v.x.to_le_bytes()).unwrap();
        f.write_all(&v.y.to_le_bytes()).unwrap();
        f.write_all(&v.z.to_le_bytes()).unwrap();
    }
    for i in &mesh.indices {
        f.write_all(&(*i).to_le_bytes()).unwrap();
    }
}

fn main() {
    let sub: u32 = std::env::args()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(6);
    let radius = 1.0;
    let a = icosphere([0.0, 0.0, 0.0], radius, sub);
    let b = icosphere([radius * 0.5, 0.0, 0.0], radius, sub);
    write_mesh("/tmp/sphere_a.bin", &a);
    write_mesh("/tmp/sphere_b.bin", &b);
    eprintln!(
        "wrote a={} tris, b={} tris",
        a.indices.len() / 3,
        b.indices.len() / 3
    );
}
