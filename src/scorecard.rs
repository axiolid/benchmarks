//! Per-scenario scorecard: one table per geometry scenario.
//!
//! Every row is either a GATE or a RECORD, and the distinction is
//! structural rather than naming: Gate rows feed the process exit
//! code, Record rows never do. A row that cannot fail is not a
//! gate, and saying so in the type keeps the table honest.

/// Whether a row can fail the run.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// Contributes to the exit code when it fails.
    Gate,
    /// Measured and printed, never fails the run.
    Record,
}

/// One measured quantity within a scenario.
pub struct Row {
    pub metric: &'static str,
    pub kind: Kind,
    /// Printed value.
    pub value: String,
    /// None when the metric is unavailable for this scenario.
    pub ok: Option<bool>,
    /// Why it failed, or what the record means.
    pub note: String,
}

impl Row {
    /// A gate row that passed or failed.
    pub fn gate(metric: &'static str, ok: bool, value: impl Into<String>) -> Self {
        Row {
            metric,
            kind: Kind::Gate,
            value: value.into(),
            ok: Some(ok),
            note: String::new(),
        }
    }

    /// A recorded row: measured, never fatal.
    pub fn record(metric: &'static str, value: impl Into<String>) -> Self {
        Row {
            metric,
            kind: Kind::Record,
            value: value.into(),
            ok: None,
            note: String::new(),
        }
    }

    /// A metric this scenario cannot supply.
    pub fn absent(metric: &'static str, why: impl Into<String>) -> Self {
        Row {
            metric,
            kind: Kind::Record,
            value: "n/a".into(),
            ok: None,
            note: why.into(),
        }
    }

    pub fn with_note(mut self, note: impl Into<String>) -> Self {
        self.note = note.into();
        self
    }
}

/// Print one scenario table and return its gate-fault count.
///
/// Only Gate rows with ok == Some(false) count. Record rows are
/// printed with a blank verdict so the table never implies a
/// measurement was checked when it was not.
pub fn emit(scenario: &str, rows: &[Row]) -> usize {
    println!("\n  scorecard: {scenario}");
    println!("  {}", "-".repeat(74));
    println!(
        "  {:<28} {:<6} {:>16}  {}",
        "metric", "kind", "value", "verdict"
    );

    let mut faults = 0usize;
    for r in rows {
        let kind = if r.kind == Kind::Gate { "gate" } else { "rec" };
        let verdict = match (r.kind, r.ok) {
            (Kind::Gate, Some(true)) => "ok".to_owned(),
            (Kind::Gate, Some(false)) => {
                faults += 1;
                format!("FAIL {}", r.note)
            }
            _ => r.note.clone(),
        };
        println!(
            "  {:<28} {:<6} {:>16}  {}",
            r.metric, kind, r.value, verdict
        );
    }
    faults
}
