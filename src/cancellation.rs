//! Cancellation and budget behaviour under stress.
//!
//! The provider DECLARES CancellationGranularity::BetweenOperations,
//! meaning a batch can be interrupted between groups but a single
//! boolean cannot be interrupted once started. That declaration is
//! a promise callers rely on to decide whether a cancel button can
//! work, so it is tested rather than trusted: a provider that
//! silently ignores a cancelled token, or one that claims less than
//! it delivers, both mislead the caller.

use crate::scorecard::{emit, Row};
use crate::{axiolid_obb, Box3, Obb};
use axiolid_contracts::{CancellationToken, ExecutionOptions};
use axiolid_core::{BooleanOperator, Tolerance};
use axiolid_mesh::TriMesh;
use axiolid_mesh_boolean_boolmesh::BoolmeshBoolean;
use axiolid_mesh_boolean_contract::MeshBoolean;

/// A subject and enough tools that a batch has groups to poll
/// between.
fn workload() -> (TriMesh, Vec<TriMesh>) {
    let subject = axiolid_obb(Obb::aabb(Box3::new(0.0, 0.0, 0.0, 20.0, 4.0, 4.0)));
    let tools = (0..24)
        .map(|i| {
            let x = -9.0 + f64::from(i) * 0.75;
            axiolid_obb(Obb::aabb(Box3::new(x, 0.0, 0.0, 0.4, 6.0, 0.4)))
        })
        .collect();
    (subject, tools)
}

pub fn report() -> usize {
    println!("\n\nCancellation and budget behaviour");
    println!("{}", "-".repeat(80));

    let provider = BoolmeshBoolean::new();
    let (subject, tools) = workload();
    let mut rows = Vec::new();

    // Pre-cancelled token: a batch must refuse rather than run to
    // completion and hand back a result nobody is waiting for.
    let token = CancellationToken::new();
    token.cancel();
    let opts = ExecutionOptions::new(Tolerance::MILLIMETRE).with_cancellation(token.clone());
    let cancelled = provider.subtract_many(&subject, &tools, &opts);
    rows.push(Row::gate(
        "batch honours cancel",
        cancelled.is_err(),
        if cancelled.is_err() {
            "refused"
        } else {
            "ran anyway"
        },
    ));

    // A live token must NOT refuse: a gate that only proves refusal
    // would pass on a provider that refuses unconditionally.
    let live =
        ExecutionOptions::new(Tolerance::MILLIMETRE).with_cancellation(CancellationToken::new());
    let ran = provider.subtract_many(&subject, &tools, &live);
    rows.push(Row::gate(
        "live token still runs",
        ran.is_ok(),
        if ran.is_ok() {
            "completed"
        } else {
            "refused wrongly"
        },
    ));

    // The declared granularity is BetweenOperations, so a SINGLE
    // boolean is expected to run to completion even when cancelled.
    // Recorded rather than gated: the contract permits either, and
    // gating would freeze one permitted behaviour into a rule.
    let t2 = CancellationToken::new();
    t2.cancel();
    let single_opts = ExecutionOptions::new(Tolerance::MILLIMETRE).with_cancellation(t2);
    let single = provider.boolean(
        &subject,
        &tools[0],
        BooleanOperator::Difference,
        &single_opts,
    );
    // Measured, and it contradicts the declaration: the provider
    // declares BetweenOperations, which says a single boolean cannot be
    // interrupted, yet a pre-cancelled token refuses one. The refusal
    // comes from a poll before the work starts, so the declaration is
    // conservative rather than wrong -- callers are promised less than
    // they get. Recorded, not gated, because the contract permits it
    // and gating would freeze today's behaviour into a rule.
    //
    // Note this partly overlaps the mesh-boolean conformance suite,
    // which already checks declared_cancellation_is_honoured. Kept
    // because it runs here against a batch workload with real groups,
    // which the conformance case does not exercise.
    rows.push(Row::record(
        "single op under cancel",
        if single.is_err() {
            "refused"
        } else {
            "ran to completion"
        },
    ));

    // A REAL gap, not a curiosity: ScratchRequirement::fits_budget
    // exists and is unit-tested, but a grep across crates/ finds no
    // production caller. Nothing on the boolean path consults
    // memory_budget_bytes, so a caller setting a budget gets no
    // enforcement and no error -- the option is silently inert.
    let tiny = ExecutionOptions::new(Tolerance::MILLIMETRE).with_memory_budget(1024);
    let budgeted = provider.subtract_many(&subject, &tools, &tiny);
    rows.push(
        Row::record(
            "1 KB memory budget",
            if budgeted.is_err() {
                "declined"
            } else {
                "ignored"
            },
        )
        .with_note("fits_budget has no production caller (kernel#104)"),
    );

    emit("cancellation and budget", &rows)
}
