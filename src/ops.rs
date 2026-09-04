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

/// Relative residual for one identity, given a kernel's `op` evaluator.
///
/// `op(a_is_host, operand, op)` returns the volume of the requested operation,
/// or `None` if the kernel failed or is not built. Returning `None` here means
/// "cannot be scored", which the caller must render as an absence rather than
/// as a perfect zero -- a kernel that refuses every operation would otherwise
/// look flawless.
///
/// The residual is normalised by `vol(A) + vol(B)` so magnitudes are comparable
/// across operand sizes; an absolute residual would make large operands look
/// worse than small ones for the same relative error.
pub fn residual<F>(identity: &Identity, vol_a: f64, vol_b: f64, mut op: F) -> Option<f64>
where
    F: FnMut(Op, Pair) -> Option<f64>,
{
    let scale = (vol_a.abs() + vol_b.abs()).max(1e-12);
    let value = match identity.name {
        // vol(A-B) + vol(A^B) must reconstruct vol(A) exactly: every point of A
        // is either in B or not, with no third case.
        "partition" => {
            let d = op(Op::Difference, Pair::Ab)?;
            let i = op(Op::Intersection, Pair::Ab)?;
            (d + i - vol_a).abs()
        }
        // The union double-counts the overlap; adding it back must recover the
        // sum of the parts.
        "inclusion-exclusion" => {
            let u = op(Op::Union, Pair::Ab)?;
            let i = op(Op::Intersection, Pair::Ab)?;
            (u + i - (vol_a + vol_b)).abs()
        }
        // A u A is A. Self-union is the classic degenerate case: every face of
        // the second operand is coincident with one of the first.
        "idempotence" => {
            let u = op(Op::Union, Pair::Aa)?;
            (u - vol_a).abs()
        }
        // Union is commutative. Any difference is pure operand-order
        // sensitivity, which a correct kernel cannot have.
        "commutativity" => {
            let ab = op(Op::Union, Pair::Ab)?;
            let ba = op(Op::Union, Pair::Ba)?;
            (ab - ba).abs()
        }
        _ => return None,
    };
    Some(value / scale)
}
