/// Boolean operation codes, mirroring the `BENCH_OP_*` defines in `cpp/shim.cpp`.
///
/// A shared integer rather than three entry points per kernel: adding an
/// operation then touches one enum and one `match` per kernel, not nine
/// `extern` declarations.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Op {
    Difference = 0,
    Union = 1,
    Intersection = 2,
}

/// What a kernel is asked to prove, independent of speed.
///
/// Each identity is a law of set algebra that any correct boolean must satisfy
/// for ANY operands. Measuring the residual turns "is this kernel exact?" into
/// a number rather than an opinion, and it needs no ground truth: the identity
/// is its own oracle, so it works on inputs whose true volume nobody knows.
#[derive(Clone, Copy)]
pub struct Identity {
    pub name: &'static str,
    pub law: &'static str,
}

pub const IDENTITIES: [Identity; 4] = [
    Identity {
        name: "partition",
        law: "vol(A-B) + vol(A^B) = vol(A)",
    },
    Identity {
        name: "inclusion-exclusion",
        law: "vol(AuB) + vol(A^B) = vol(A) + vol(B)",
    },
    Identity {
        name: "idempotence",
        law: "vol(AuA) = vol(A)",
    },
    Identity {
        name: "commutativity",
        law: "vol(AuB) = vol(BuA)",
    },
];

/// Which operand pair an identity wants evaluated.
///
/// Explicit rather than a bare `swap: bool`: idempotence needs `A op A`, which
/// no combination of "swapped or not" over `(A, B)` can express. Encoding that
/// as a swap silently measured `B u A` instead and produced a plausible-looking
/// 1.5e-1 residual that was pure harness error, not a kernel defect.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Pair {
    /// subject = A, tool = B
    Ab,
    /// subject = B, tool = A
    Ba,
    /// subject = A, tool = A
    Aa,
}

/// What a kernel reports about one boolean result.
///
/// Volume alone cannot distinguish a correct solid from a wrong one
/// with the same volume: a unit cube and the same cube translated 100
/// units away both measure 1. Bounds and the topological counts are
/// what make an identity discriminating rather than merely plausible.
///
/// Every field except `volume` is optional because the C ABI kernels
/// return a bare double: they can be scored on volume and nothing
/// else. `None` means NOT REPORTABLE by this kernel, which must render
/// as an absence rather than as a passing zero.
#[derive(Clone, Copy, Default)]
pub struct Metrics {
    /// Enclosed volume. Every kernel can supply this.
    pub volume: f64,
    /// Total surface area.
    pub area: Option<f64>,
    /// Axis-aligned bounds as [minx, miny, minz, maxx, maxy, maxz].
    ///
    /// The only POSITIONAL metric here. Area, Euler and component count
    /// are all translation-invariant, so without bounds a result that is
    /// the right shape in the wrong place still scores clean.
    pub bounds: Option<[f64; 6]>,
    /// Euler characteristic V - E + F.
    pub euler: Option<i64>,
    /// Number of connected components.
    pub components: Option<usize>,
    /// Whether the result passed a closed-manifold audit.
    pub manifold: Option<bool>,
}

/// One scored metric within an identity.
#[derive(Clone, Copy)]
pub struct Score {
    /// Which quantity was compared.
    pub metric: &'static str,
    /// Relative residual, or absolute for integer-valued metrics.
    pub residual: f64,
}

/// The result of scoring one identity against one kernel.
///
/// A list rather than a single number: reporting only the worst metric
/// would hide WHICH invariant broke, and that is the diagnostic value.
/// An empty list means the kernel could not be scored at all.
#[derive(Clone, Default)]
pub struct Verdict {
    /// One entry per metric that both sides could report.
    pub scores: Vec<Score>,
}

impl Verdict {
    /// Worst residual across every scored metric.
    #[must_use]
    pub fn worst(&self) -> Option<f64> {
        self.scores
            .iter()
            .map(|s| s.residual)
            .fold(None, |acc: Option<f64>, r| {
                Some(acc.map_or(r, |a: f64| a.max(r)))
            })
    }

    /// Name of the metric with the worst residual.
    #[must_use]
    pub fn worst_metric(&self) -> Option<&'static str> {
        self.scores
            .iter()
            .fold(None, |acc: Option<&Score>, s| match acc {
                Some(b) if b.residual >= s.residual => Some(b),
                _ => Some(s),
            })
            .map(|s| s.metric)
    }
}

/// Compare two results that must denote the SAME solid.
///
/// Only metrics BOTH sides report are scored. A metric one side cannot
/// supply is skipped rather than treated as zero, so a kernel is never
/// credited for a check it did not perform.
fn compare(left: &Metrics, right: &Metrics, scale: f64) -> Verdict {
    let mut scores = Vec::new();
    scores.push(Score {
        metric: "volume",
        residual: (left.volume - right.volume).abs() / scale,
    });
    if let (Some(la), Some(ra)) = (left.area, right.area) {
        // Normalised by area, not volume: the two have different units and
        // dividing an area error by a volume would not be dimensionless.
        let denom = (la.abs() + ra.abs()).max(1e-12);
        scores.push(Score {
            metric: "area",
            residual: (la - ra).abs() / denom,
        });
    }
    if let (Some(lb), Some(rb)) = (left.bounds, right.bounds) {
        // The positional check. Worst corner deviation, normalised by the
        // diagonal so it is comparable across operand sizes.
        let diag = ((rb[3] - rb[0]).powi(2) + (rb[4] - rb[1]).powi(2) + (rb[5] - rb[2]).powi(2))
            .sqrt()
            .max(1e-12);
        let worst = (0..6)
            .map(|i| (lb[i] - rb[i]).abs())
            .fold(0.0_f64, f64::max);
        scores.push(Score {
            metric: "bounds",
            residual: worst / diag,
        });
    }
    if let (Some(le), Some(re)) = (left.euler, right.euler) {
        // Integer-valued: any difference at all is a topological defect, so
        // this is an absolute count, not a relative error.
        scores.push(Score {
            metric: "euler",
            residual: (le - re).abs() as f64,
        });
    }
    if let (Some(lc), Some(rc)) = (left.components, right.components) {
        scores.push(Score {
            metric: "components",
            residual: lc.abs_diff(rc) as f64,
        });
    }
    if let (Some(lm), Some(rm)) = (left.manifold, right.manifold) {
        // A result that stopped being a closed manifold is broken even if
        // every measurement still agrees.
        scores.push(Score {
            metric: "manifold",
            residual: if lm == rm { 0.0 } else { 1.0 },
        });
    }
    Verdict { scores }
}

/// Score one identity, given a kernel evaluator that reports full metrics.
///
/// Returns `None` when the kernel could not produce every operand the
/// identity needs, which the caller must render as an absence: a kernel
/// that refuses everything would otherwise look flawless.
///
/// Additive laws are scored on volume alone because area and the
/// topological counts are not additive across a cut. Equivalence laws
/// are scored on every metric both sides report.
pub fn score<F>(identity: &Identity, a: &Metrics, b: &Metrics, mut op: F) -> Option<Verdict>
where
    F: FnMut(Op, Pair) -> Option<Metrics>,
{
    let scale = (a.volume.abs() + b.volume.abs()).max(1e-12);
    match identity.name {
        "partition" => {
            let d = op(Op::Difference, Pair::Ab)?;
            let i = op(Op::Intersection, Pair::Ab)?;
            Some(Verdict {
                scores: vec![Score {
                    metric: "volume",
                    residual: (d.volume + i.volume - a.volume).abs() / scale,
                }],
            })
        }
        "inclusion-exclusion" => {
            let u = op(Op::Union, Pair::Ab)?;
            let i = op(Op::Intersection, Pair::Ab)?;
            Some(Verdict {
                scores: vec![Score {
                    metric: "volume",
                    residual: (u.volume + i.volume - (a.volume + b.volume)).abs() / scale,
                }],
            })
        }
        // A u A is A: an EQUIVALENCE, so the result must match A in every
        // reported metric, not merely in volume.
        "idempotence" => {
            let u = op(Op::Union, Pair::Aa)?;
            Some(compare(&u, a, scale))
        }
        // A u B and B u A denote the same solid.
        "commutativity" => {
            let ab = op(Op::Union, Pair::Ab)?;
            let ba = op(Op::Union, Pair::Ba)?;
            Some(compare(&ab, &ba, scale))
        }
        _ => None,
    }
}
