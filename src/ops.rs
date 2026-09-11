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
#[derive(Clone)]
pub struct Identity {
    pub name: &'static str,
    pub law: &'static str,
    /// What the kernel must satisfy, as an expression tree.
    pub claim: Claim,
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

/// A boolean expression over the three operands.
///
/// `Pair` named two operands, so it could not express a law whose
/// operand is itself a result -- associativity needs `(AuB) u C`.
/// Encoding that as another enum variant would need one variant per
/// shape; a tree needs none.
#[derive(Clone, PartialEq, Eq)]
pub enum Expr {
    /// The first operand.
    A,
    /// The second operand.
    B,
    /// The third operand, used only by the associativity laws.
    C,
    /// One boolean applied to two sub-expressions.
    Apply(Op, Box<Expr>, Box<Expr>),
}

impl Expr {
    /// `a op b`, boxed.
    fn of(op: Op, a: Expr, b: Expr) -> Expr {
        Expr::Apply(op, Box::new(a), Box::new(b))
    }
}

/// Shorthand builders keep the identity table readable.
fn u(a: Expr, b: Expr) -> Expr {
    Expr::of(Op::Union, a, b)
}
fn i(a: Expr, b: Expr) -> Expr {
    Expr::of(Op::Intersection, a, b)
}
fn d(a: Expr, b: Expr) -> Expr {
    Expr::of(Op::Difference, a, b)
}

/// What the two sides of an identity must satisfy.
#[derive(Clone, PartialEq, Eq)]
pub enum Claim {
    /// Both sides denote the same solid: compare every shared metric.
    Same(Expr, Expr),
    /// The expression must be empty. Scored as volume against zero:
    /// an empty result has no bounds, genus or component count to
    /// compare, and a kernel legitimately returns no triangles.
    Empty(Expr),
    /// Summed volumes on the left must equal summed volumes on the
    /// right. Volume only -- area and the topological counts are not
    /// additive across a cut, so scoring them here would penalise a
    /// correct kernel.
    Sum(Vec<Expr>, Vec<Expr>),
}

/// The identity table: name, readable law, and the claim to score.
///
/// Built at runtime rather than as a const because `Expr` owns boxed
/// sub-expressions.
pub fn identities() -> Vec<Identity> {
    use Expr::{A, B, C};
    vec![
        Identity {
            name: "partition",
            law: "vol(A-B) + vol(A^B) = vol(A)",
            claim: Claim::Sum(vec![d(A, B), i(A, B)], vec![A]),
        },
        Identity {
            name: "inclusion-exclusion",
            law: "vol(AuB) + vol(A^B) = vol(A) + vol(B)",
            claim: Claim::Sum(vec![u(A, B), i(A, B)], vec![A, B]),
        },
        Identity {
            name: "idempotence-u",
            law: "A u A = A",
            claim: Claim::Same(u(A, A), A),
        },
        Identity {
            name: "idempotence-i",
            law: "A ^ A = A",
            claim: Claim::Same(i(A, A), A),
        },
        Identity {
            name: "commutativity-u",
            law: "A u B = B u A",
            claim: Claim::Same(u(A, B), u(B, A)),
        },
        Identity {
            name: "commutativity-i",
            law: "A ^ B = B ^ A",
            claim: Claim::Same(i(A, B), i(B, A)),
        },
        Identity {
            name: "self-difference",
            law: "A - A = 0",
            claim: Claim::Empty(d(A, A)),
        },
        Identity {
            name: "difference-disjoint",
            law: "(A-B) ^ B = 0",
            claim: Claim::Empty(i(d(A, B), B)),
        },
        Identity {
            name: "reconstruction",
            law: "(A-B) u (A^B) = A",
            claim: Claim::Same(u(d(A, B), i(A, B)), A),
        },
        Identity {
            name: "associativity-u",
            law: "(AuB)uC = Au(BuC)",
            claim: Claim::Same(u(u(A, B), C), u(A, u(B, C))),
        },
        Identity {
            name: "associativity-i",
            law: "(A^B)^C = A^(B^C)",
            claim: Claim::Same(i(i(A, B), C), i(A, i(B, C))),
        },
        Identity {
            name: "absorption-u",
            law: "A u (A^B) = A",
            claim: Claim::Same(u(A, i(A, B)), A),
        },
        Identity {
            name: "absorption-i",
            law: "A ^ (AuB) = A",
            claim: Claim::Same(i(A, u(A, B)), A),
        },
    ]
}

/// A kernel that can evaluate an expression tree.
///
/// `Solid` is the kernel-native representation, so an intermediate
/// result feeds the next operation directly. Returning `Metrics` from
/// `apply` instead would be wrong: a measurement cannot be an operand,
/// and round-tripping through one would discard the geometry that the
/// next step needs.
pub trait Kernel {
    /// The kernel native solid type.
    type Solid;

    /// One of the three input operands, by index.
    fn operand(&mut self, index: usize) -> Option<Self::Solid>;

    /// Apply one boolean. `None` means refused or unsupported.
    fn apply(&mut self, op: Op, a: &Self::Solid, b: &Self::Solid) -> Option<Self::Solid>;

    /// Measure a solid. Fields the kernel cannot supply stay `None`.
    fn measure(&mut self, solid: &Self::Solid) -> Metrics;
}

/// Evaluate an expression tree, returning the kernel native solid.
///
/// `None` propagates: a partially-evaluated identity must not be
/// scored, or a kernel that refuses one step would look perfect.
fn eval<K: Kernel>(expr: &Expr, k: &mut K) -> Option<K::Solid> {
    match expr {
        Expr::A => k.operand(0),
        Expr::B => k.operand(1),
        Expr::C => k.operand(2),
        Expr::Apply(op, l, r) => {
            let lv = eval(l, k)?;
            let rv = eval(r, k)?;
            k.apply(*op, &lv, &rv)
        }
    }
}

/// Score one identity against a kernel.
///
/// Returns `None` when the kernel could not evaluate every side, which
/// the caller must render as an absence rather than a passing zero.
pub fn score<K: Kernel>(identity: &Identity, k: &mut K) -> Option<Verdict> {
    match &identity.claim {
        Claim::Same(left, right) => {
            let l = eval(left, k)?;
            let r = eval(right, k)?;
            let (lm, rm) = (k.measure(&l), k.measure(&r));
            let scale = (lm.volume.abs() + rm.volume.abs()).max(1e-12);
            Some(compare(&lm, &rm, scale))
        }
        // Normalised by the INPUT scale, never by the result itself:
        // dividing a near-zero volume by its own magnitude yields ~1
        // for any leftover, or 0/0 when the kernel returns nothing.
        Claim::Empty(expr) => {
            let v = eval(expr, k)?;
            let m = k.measure(&v);
            let a = k.operand(0).map(|s| k.measure(&s))?;
            let b = k.operand(1).map(|s| k.measure(&s))?;
            let scale = (a.volume.abs() + b.volume.abs()).max(1e-12);
            Some(Verdict {
                scores: vec![Score {
                    metric: "volume",
                    residual: m.volume.abs() / scale,
                }],
            })
        }
        Claim::Sum(left, right) => {
            let mut total = 0.0;
            for e in left {
                let s = eval(e, k)?;
                total += k.measure(&s).volume;
            }
            let mut want = 0.0;
            for e in right {
                let s = eval(e, k)?;
                want += k.measure(&s).volume;
            }
            let scale = want.abs().max(1e-12);
            Some(Verdict {
                scores: vec![Score {
                    metric: "volume",
                    residual: (total - want).abs() / scale,
                }],
            })
        }
    }
}
