//! Behavioural harness over the real-world corpus.
//!
//! The corpus has no ground truth, so this asserts BEHAVIOUR, not
//! answers: every model must produce a definite outcome -- a result or a
//! typed refusal -- and never a panic, a hang, or a silent wrong-shaped
//! success. Correctness gates live in the synthetic suite.
//!
//! Input is produced by `scripts/fetch-corpus.py` then
//! `scripts/pack-corpus.py`, both of which keep every downloaded byte
//! outside the repository. This module reads only the packed cache and
//! is skipped entirely when that cache is absent, so a clone without the
//! corpus still builds and runs.

use axiolid_contracts::ExecutionOptions;
use axiolid_core::{BooleanOperator, Point3, Tolerance};
use axiolid_mesh::{audit_mesh, TriMesh};
use axiolid_mesh_boolean_boolmesh::BoolmeshBoolean;
use axiolid_mesh_boolean_contract::MeshBoolean;
use std::path::{Path, PathBuf};

/// One packed model plus the class it was filed under.
struct Model {
    class: String,
    file_id: String,
    mesh: TriMesh,
}

/// Read the little-endian pack written by `pack-corpus.py`.
fn read_pack(path: &Path) -> Option<TriMesh> {
    let bytes = std::fs::read(path).ok()?;
    if bytes.len() < 8 {
        return None;
    }
    let nv = u32::from_le_bytes(bytes[0..4].try_into().ok()?) as usize;
    let nf = u32::from_le_bytes(bytes[4..8].try_into().ok()?) as usize;
    let want = 8 + nv * 24 + nf * 12;
    if bytes.len() != want {
        return None;
    }
    let mut positions = Vec::with_capacity(nv);
    for i in 0..nv {
        let o = 8 + i * 24;
        let x = f64::from_le_bytes(bytes[o..o + 8].try_into().ok()?);
        let y = f64::from_le_bytes(bytes[o + 8..o + 16].try_into().ok()?);
        let z = f64::from_le_bytes(bytes[o + 16..o + 24].try_into().ok()?);
        positions.push(Point3::new(x, y, z));
    }
    let base = 8 + nv * 24;
    let mut indices = Vec::with_capacity(nf * 3);
    for i in 0..nf * 3 {
        let o = base + i * 4;
        indices.push(u32::from_le_bytes(bytes[o..o + 4].try_into().ok()?));
    }
    Some(TriMesh::new(positions, indices))
}

/// Root of the packed cache, overridable for a different mount.
fn pack_dir() -> PathBuf {
    std::env::var("AXIOLID_CORPUS_PACK")
        .unwrap_or_else(|_| "/mnt/archive/corpus/pack".to_owned())
        .into()
}

/// Load the packed sample. Returns an empty vec when the cache is absent,
/// which is the normal state for a fresh clone.
fn load() -> Vec<Model> {
    let dir = pack_dir();
    let index = dir.join("index.tsv");
    let Ok(text) = std::fs::read_to_string(&index) else {
        return Vec::new();
    };
    let mut models = Vec::new();
    for line in text.lines() {
        let cols: Vec<&str> = line.split('\t').collect();
        if cols.len() != 4 {
            continue;
        }
        if let Some(mesh) = read_pack(&dir.join(cols[3])) {
            models.push(Model {
                class: cols[0].to_owned(),
                file_id: cols[1].to_owned(),
                mesh,
            });
        }
    }
    models
}

/// What the kernel did with one model.
///
/// A refusal is a SUCCESS for damaged input: the contract is that the
/// kernel either produces a result or declines in a typed way. The only
/// failures are a wrong-shaped success or a crash.
enum Outcome {
    Produced,
    Refused,
    WrongShape(&'static str),
    /// The provider crashed instead of returning. Caught rather than
    /// allowed to kill the run: one bad model must not hide the other
    /// 239, and a crash is the most severe fault class there is.
    Panicked,
}

/// Exercise one model and decide whether the kernel behaved.
///
/// The operation is a self-union, chosen because it is well defined for
/// every input class: a valid solid must survive it, and damaged input
/// must be refused rather than silently mangled. The audit runs first so
/// a refusal can be checked against what the mesh actually is.
fn exercise(model: &Model) -> Outcome {
    trace(model);
    let tol = Tolerance::MILLIMETRE;
    let health = audit_mesh(&model.mesh, tol);

    // A model filed as damaged must actually be damaged. If the audit
    // disagrees with the manifest, the harness is testing the wrong
    // thing and should say so rather than quietly pass.
    // Structural damage only. `degenerate_triangles` is deliberately NOT
    // included: it counts triangles below an area threshold, which is a
    // tolerance judgement rather than a structural defect. An
    // independent edge-multiplicity check confirmed models flagged this
    // way (37743, 75653, 91024) are closed with no boundary or
    // non-manifold edges, so counting thin triangles as damage was a
    // harness bug, not a corpus mislabel.
    let damaged = health.non_manifold_edges > 0
        || health.boundary_edges > 0
        || health.inconsistent_winding_edges > 0
        || health.invalid_indices > 0
        || health.non_finite_positions > 0;

    match model.class.as_str() {
        "open" if health.boundary_edges == 0 => {
            return Outcome::WrongShape("filed open but audit found no boundary edge");
        }
        "non_manifold"
            if health.non_manifold_edges == 0 && health.inconsistent_winding_edges == 0 =>
        {
            return Outcome::WrongShape("filed non-manifold but audit found none");
        }
        // A sliver triangle is dropped from `usable_triangles`, which
        // orphans its edges and reports them as boundary. Model 37743:
        // 1 degenerate triangle -> exactly 3 boundary edges, while an
        // independent edge-multiplicity count over ALL triangles finds
        // zero. So boundary edges attributable to dropped slivers are
        // not damage; anything beyond that is.
        "clean" if damaged && health.boundary_edges > 3 * health.degenerate_triangles => {
            return Outcome::WrongShape("filed clean but audit found structural damage");
        }
        _ => {}
    }

    let provider = BoolmeshBoolean::new();
    let options = ExecutionOptions::new(tol);
    // The provider is allowed to return Err. It is NOT allowed to panic,
    // but #101 proves it can, so the call is isolated: a crash becomes a
    // counted fault instead of ending the run.
    let caught = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        provider.boolean(&model.mesh, &model.mesh, BooleanOperator::Union, &options)
    }));
    let Ok(result) = caught else {
        return Outcome::Panicked;
    };

    match result {
        Err(_) => Outcome::Refused,
        Ok(out) => {
            let mesh = out.mesh;
            if mesh.triangles().len() == 0 {
                // An empty success is the silent failure this harness
                // exists to catch: no error, no geometry.
                return Outcome::WrongShape("empty mesh returned without an error");
            }
            let after = audit_mesh(&mesh, tol);
            if after.non_finite_positions > 0 {
                return Outcome::WrongShape("non-finite positions in output");
            }
            if after.invalid_indices > 0 {
                return Outcome::WrongShape("invalid indices in output");
            }
            // A clean closed solid must stay closed through a self-union.
            if model.class == "clean" && after.boundary_edges > health.boundary_edges {
                return Outcome::WrongShape("clean input gained boundary edges");
            }
            Outcome::Produced
        }
    }
}

/// Run the corpus and report per class.
///
/// Returns the fault count so the caller can feed it into the process
/// exit code. Only WrongShape counts as a fault: a refusal is correct
/// behaviour for damaged input, and is reported separately so a change
/// in the refusal rate is still visible.
pub fn report() -> usize {
    let models = load();
    println!();
    println!("Real-world corpus -- behavioural, not correctness");
    println!("{}", "-".repeat(80));
    if models.is_empty() {
        println!("  no packed corpus at {}", pack_dir().display());
        println!("  run scripts/fetch-corpus.py then scripts/pack-corpus.py");
        println!("  skipped, not failed: the corpus is optional by design.");
        return 0;
    }
    println!("Self-union of every model. A typed refusal is correct for damaged");
    println!("input; only a wrong-shaped success or a crash is a fault.");
    println!();
    println!(
        "  {:<18} {:>7} {:>9} {:>8} {:>7} {:>7}",
        "class", "models", "produced", "refused", "panics", "faults"
    );

    // The provider panics on some real input (#101). Its default hook
    // would print a backtrace per model and bury the table, so it is
    // silenced for the duration and restored afterwards.
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(|_| {}));

    let mut classes: Vec<String> = models.iter().map(|m| m.class.clone()).collect();
    classes.sort();
    classes.dedup();

    let mut total_faults = 0usize;
    let mut examples: Vec<String> = Vec::new();

    for class in &classes {
        let mut produced = 0usize;
        let mut refused = 0usize;
        let mut panics = 0usize;
        let mut faults = 0usize;
        for model in models.iter().filter(|m| &m.class == class) {
            match exercise(model) {
                Outcome::Produced => produced += 1,
                Outcome::Refused => refused += 1,
                Outcome::Panicked => {
                    panics += 1;
                    faults += 1;
                    if examples.len() < 8 {
                        examples.push(format!(
                            "    {} {}: PANIC inside the provider",
                            class, model.file_id
                        ));
                    }
                }
                Outcome::WrongShape(why) => {
                    faults += 1;
                    if examples.len() < 8 {
                        examples.push(format!("    {} {}: {}", class, model.file_id, why));
                    }
                }
            }
        }
        total_faults += faults;
        let flag = if faults > 0 { " !!" } else { "" };
        println!(
            "  {:<18} {:>7} {:>9} {:>8} {:>7} {:>7}{}",
            class,
            produced + refused + faults,
            produced,
            refused,
            panics,
            faults,
            flag
        );
    }

    std::panic::set_hook(previous);

    if !examples.is_empty() {
        println!();
        for line in &examples {
            println!("{line}");
        }
    }

    println!();
    if total_faults == 0 {
        println!("  every model produced a result or a typed refusal.");
    } else {
        println!("  {total_faults} behavioural fault(s).");
    }
    total_faults
}

/// Print the model being exercised before touching it, so a panic
/// inside the provider names its input instead of dying anonymously.
/// Enabled with AXIOLID_CORPUS_TRACE=1.
fn trace(model: &Model) {
    if std::env::var("AXIOLID_CORPUS_TRACE").is_ok() {
        eprintln!("TRACE {} {}", model.class, model.file_id);
    }
}
