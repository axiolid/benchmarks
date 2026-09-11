//! Repro for the boolean45 pair_up assertion panic found by the corpus
//! harness on Thingi10K model 48556.
//!
//! The mesh is structurally sound by every check we apply: finite
//! positions, valid indices, no duplicate facets, no degenerate facets.
//! A self-union still panics on a debug assertion rather than returning
//! a typed refusal.
//!
//! Reads the packed cache, so it is skipped when the corpus is absent.

use axiolid_contracts::ExecutionOptions;
use axiolid_core::{BooleanOperator, Point3, Tolerance};
use axiolid_mesh::TriMesh;
use axiolid_mesh_boolean_boolmesh::BoolmeshBoolean;
use axiolid_mesh_boolean_contract::MeshBoolean;

fn main() {
    let path = std::env::var("AXIOLID_CORPUS_PACK")
        .unwrap_or_else(|_| "/mnt/archive/corpus/pack".to_owned())
        + "/multi_component_48556.bin";
    let Ok(bytes) = std::fs::read(&path) else {
        println!("no packed corpus at {path}, skipping");
        return;
    };
    let nv = u32::from_le_bytes(bytes[0..4].try_into().unwrap()) as usize;
    let nf = u32::from_le_bytes(bytes[4..8].try_into().unwrap()) as usize;
    let mut positions = Vec::with_capacity(nv);
    for i in 0..nv {
        let o = 8 + i * 24;
        positions.push(Point3::new(
            f64::from_le_bytes(bytes[o..o + 8].try_into().unwrap()),
            f64::from_le_bytes(bytes[o + 8..o + 16].try_into().unwrap()),
            f64::from_le_bytes(bytes[o + 16..o + 24].try_into().unwrap()),
        ));
    }
    let base = 8 + nv * 24;
    let mut indices = Vec::with_capacity(nf * 3);
    for i in 0..nf * 3 {
        let o = base + i * 4;
        indices.push(u32::from_le_bytes(bytes[o..o + 4].try_into().unwrap()));
    }
    let mesh = TriMesh::new(positions, indices);
    println!("loaded {nv} verts {nf} facets");

    let provider = BoolmeshBoolean::new();
    let options = ExecutionOptions::new(Tolerance::MILLIMETRE);
    println!("calling self-union, expect a panic at boolean45.rs pair_up");
    match provider.boolean(&mesh, &mesh, BooleanOperator::Union, &options) {
        Ok(out) => println!("returned {} triangles", out.mesh.triangles().len()),
        Err(e) => println!("typed refusal: {e:?}"),
    }
}
