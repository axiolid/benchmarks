//! Isolated axiolid-only sphere boolean, for perf profiling.
//!
//! Self-contained: main.rs is a binary crate with no lib target, so this
//! duplicates the tiny icosphere generator rather than
//! restructuring the crate for a scratch profiling tool.
use axiolid_contracts::ExecutionOptions;
use axiolid_core::{BooleanOperator, Point3, Tolerance};
use axiolid_mesh::TriMesh;
use axiolid_mesh_boolean_boolmesh::BoolmeshBoolean;
use axiolid_mesh_boolean_contract::MeshBoolean;
use std::collections::HashMap;

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

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let subdivisions: u32 = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(6);
    let iterations: u32 = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(20);
    let op = match args.get(3).map(String::as_str) {
        Some("union") => BooleanOperator::Union,
        Some("intersection") => BooleanOperator::Intersection,
        _ => BooleanOperator::Difference,
    };

    let radius = 1.0;
    let a = icosphere([0.0, 0.0, 0.0], radius, subdivisions);
    let b = icosphere([radius * 0.5, 0.0, 0.0], radius, subdivisions);

    eprintln!(
        "profile_sphere: sub={subdivisions} tris_a={} tris_b={} iters={iterations} op={op:?}",
        a.indices.len() / 3,
        b.indices.len() / 3
    );

    let provider = BoolmeshBoolean::default();
    let options = ExecutionOptions::new(Tolerance::METRE);

    let mut checksum = 0.0f64;
    for i in 0..iterations {
        match provider.boolean(&a, &b, op, &options) {
            Ok(r) => checksum += r.mesh.positions.len() as f64,
            Err(e) => {
                eprintln!("iteration {i} failed: {e}");
                std::process::exit(1);
            }
        }
    }
    println!("checksum={checksum}");
}
