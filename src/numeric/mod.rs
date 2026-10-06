//! Scalar semantics of the operators, comparisons and post-ops, as pure `f32` functions; the
//! `math.*` functions are in `stdlib::math`.
//!
//! The constant folder and the VM both call these. A constant operand (a literal, a call with
//! constant arguments, a product of constants) folds to its value before its parent applies a
//! negation, a term or a factor to it; a run-time operand takes them into its post-op `S·x + O`.
//! So a folded result can differ from the run-time form of the same expression in:
//!
//! - the sign of a zero: `-((0) * (1))` folds to −0 and `-(v.a * v.b)` runs to +0;
//!   `((-1) * (0)) + 0` folds to +0 and `(v.a * v.b) + 0` runs to −0;
//! - the sign or the choice of a NaN: a folded negation flips its sign, a run-time one leaves it;
//!   with a constant term left of a NaN operand, the fold takes the term's NaN where the run-time
//!   form can take the operand's: `math.ln(-1) + 3*math.sqrt(-1)` and
//!   `math.ln(-1) + v.k*math.sqrt(-1)`;
//! - `math.sign`: with a negative argument the post-op gives `−(S + O)`, the folded value
//!   `−S + O`;
//! - a division the guard stops: at run time it is 0 without its post-op;
//! - a literal divisor: the run-time form multiplies by its reciprocal: `(1/3)/3` folds to
//!   `0x3de38e39`, `v.k/3` runs to `0x3de38e3a`; below the guard's threshold the factor is 0, so a
//!   NaN or infinite dividend gives NaN where the fold gives 0: `math.sqrt(-1)/0` and `v.k/0`;
//! - on `X86_64`, a NaN divisor: the fold gives 0, the run-time form NaN: `1/math.sqrt(-1)` and
//!   `1/v.k`;
//! - `math.mod`: a zero divisor folds to NaN and runs to 0, and a −0 remainder folds to −0 and
//!   runs to +0: `math.mod(3, 0)` and `math.mod(3, v.k)`, `math.mod(-4, 2)` and
//!   `math.mod(v.k, 2)`;
//! - the rounding of sums: the run-time form flattens nested sums, adds the constant terms
//!   together first and multiplies the terms of an inner sum by its factor: `1 * 1 + (1/3) + 2`
//!   folds to `0x40555556`, `v.a * v.b + (1/3) + 2` runs to `0x40555555`;
//! - the rounding of products: the run-time form multiplies chained constant factors together
//!   first: `(0.1*3)*(1/3)` folds to `0x3dccccce`, `(v.k*3)*(1/3)` runs to `0x3dcccccd`; and
//!   `3.4e38*3.4e38*1e-45+1` folds to +∞ where `v.k*3.4e38*1e-45+1` does not overflow;
//! - an infinite factor: a constant factor folds into its operand's post-op, so an infinite one
//!   makes the offset `∞·O`, a NaN for an operand without an offset: `v.x * math.exp(1000)` and
//!   `(v.x < 2) * math.exp(1000)` are NaN where the same infinity read from a variable multiplies;
//! - on `Arm64`, a post-op computed inside its instruction: a multiply-add rounds once, and a
//!   factor scales the dividend, one factor of a product or the degree conversion first (and can
//!   overflow there): `(-1/3) * 3 + 1` folds to 0 and `v.d * 3 + 1` with `v.d` = −1/3 runs to
//!   −2⁻²⁵.
//!
//! The float behaviour is chosen when the crate is built: an `aarch64` build has the `Arm64`
//! behaviour, every other target the `X86_64` behaviour ([`ARCH`]).
//!
//! - `X86_64` rounds every multiply, add and divide separately, `min`/`max` return their second
//!   operand when either is NaN and every comparison with a NaN is false. An arithmetic operation
//!   with NaN operands returns its left NaN operand quietened; an invalid one gives `0xffc00000`.
//! - `Arm64` rounds `x·S + O` and the other multiply-adds once, `min`/`max` ignore a NaN operand,
//!   `<`/`<=` with a NaN are true, a NaN divisor gives 0 and a NaN `asin`/`acos` argument counts as
//!   −1. An arithmetic operation (`+ − · /` and the multiply-adds) with NaN operands returns the
//!   first signalling one quietened, else the first quiet one, the addend of a multiply-add first;
//!   an invalid one gives `0x7fc00000`.
//!
//! Formulas of several operations (the easings, `math.hermite_blend`, the random interpolation, the
//! inverse-trigonometric degree conversion) also differ in shape. An invalid `math.sin`,
//! `math.cos`, `math.pow` or `math.mod` gives `0xffc00000` on both, `math.asin` / `math.acos`
//! beyond the tolerance `0x7fc00000`; `math.ln` of a negative number gives `0x7fc00000` on `X86_64`
//! and `0xffc00000` on `Arm64`, `math.sqrt` the reverse.
//!
//! A target other than x86-64 and aarch64 (wasm32, riscv64, 32-bit arm, 32-bit x86 with SSE2, …)
//! has the `X86_64` behaviour and gives the same bits for every result that is not a NaN; 32-bit
//! x86 without SSE2 does not compile. The arithmetic and the standard functions choose the sign and
//! payload of the NaNs they return, so those are the same too, except where an instruction of the
//! target makes the NaN: `math.floor`, `math.ceil`, `math.round` and `math.trunc` of a NaN, a NaN
//! bound of `math.die_roll` / `math.die_roll_integer` (floored), and the result of a comparison or
//! logical node whose folded post-op holds a NaN. On 32-bit x86 a value returned through the x87
//! register may have a signalling NaN quietened.
//!
//! Every instruction function takes the node's [`PostOp`] as its last argument and returns the
//! final value; pass [`PostOp::IDENTITY`] for the plain form.

pub(crate) mod arch;
pub mod arith;
mod instr;
mod post_op;

pub use arch::{ARCH, Arch};
pub use instr::{
    add, array_index, div, div_guard, eq, fold_const_div, fold_const_divisor, ge, gt, le, lt, mul,
    ne, negate, not, truthy,
};
pub use post_op::PostOp;

#[cfg(test)]
pub(crate) mod test_support {
    use super::*;

    pub(crate) const ID: PostOp = PostOp::IDENTITY;
    pub(crate) const NAN: f32 = f32::NAN;
    pub(crate) const INF: f32 = f32::INFINITY;

    /// `x86_64` on an `X86_64` build, `arm64` on an `Arm64` one.
    pub(crate) fn per_arch<T>(x86_64: T, arm64: T) -> T {
        match ARCH {
            Arch::X86_64 => x86_64,
            Arch::Arm64 => arm64,
        }
    }

    /// A post-op that is not the identity and has exact arithmetic on small integers.
    pub(crate) const AFFINE: PostOp = PostOp::new(2.0, 1.0);

    /// `|actual − expected| <= tolerance`; a non-finite expectation must match exactly.
    pub(crate) fn within(actual: f32, expected: f32, tolerance: f32) -> bool {
        if expected.is_nan() {
            actual.is_nan()
        } else if expected.is_infinite() {
            actual == expected
        } else {
            (actual - expected).abs() <= tolerance
        }
    }

    #[track_caller]
    pub(crate) fn close(actual: f32, expected: f32, tolerance: f32) {
        assert!(
            within(actual, expected, tolerance),
            "{actual:e} is not {expected:e} ± {tolerance:e}"
        );
    }

    /// The literal `1 / 3` as the optimiser folds it.
    pub(crate) fn third() -> f32 {
        fold_const_div(1.0, 3.0)
    }

    /// A grid over and beyond `[0, 1]`, a fixed pseudo-random scatter, and the values around the
    /// easing thresholds, centres and phases, so that a constant one step off or a branch taken at
    /// a threshold shows.
    #[allow(clippy::manual_midpoint)]
    pub(crate) fn sweep_ts() -> Vec<f32> {
        let mut ts: Vec<f32> = (-128..=768).map(|k| k as f32 / 512.0).collect();
        let mut state = 0x9e37_79b9_7f4a_7c15_u64;
        for _ in 0..30_000 {
            state = state
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            ts.push((state >> 40) as f32 / 16_777_216.0 * 2.0 - 0.5);
        }
        // Bounce thresholds and centres, their arm64 mirrored and shifted forms, and the elastic
        // shift.
        let specials = [
            0x3eba_2e8c_u32,
            0x3f3a_2e8c,
            0x3f68_ba2f,
            0x3f0b_a2e9,
            0x3f51_745d,
            0x3f74_5d17,
            0x3ee8_ba2e,
            0x3e3a_2e8c,
            0x3d3a_2e90,
            0x3fc5_d174,
            0x3fe8_ba2e,
            0x3ffa_2e8c,
            0x3f00_0000,
            0x3e80_0000,
            0x3f40_0000,
            0x3d99_999a,
            0x3e99_999a,
            0x3eaa_aaab,
        ];
        for bits in specials {
            for delta in -4_i32..=4 {
                let s = f32::from_bits(bits.wrapping_add_signed(delta));
                ts.extend([
                    s,
                    -s,
                    1.0 - s,
                    s * 2.0,
                    s * 0.5,
                    (1.0 + s) * 0.5,
                    (1.0 - s) * 0.5,
                    2.0 - s,
                    s + 0.5,
                ]);
            }
        }
        ts
    }

    /// FNV-1a over the bits of `f` at every [`sweep_ts`] point, for `(start, end)` pairs whose
    /// range is not a power of two (so fused and separate rounding differ); all NaNs hash alike.
    pub(crate) fn sweep_hash(f: impl Fn(f32, f32, f32) -> f32) -> u64 {
        let mut hash = 0xcbf2_9ce4_8422_2325_u64;
        for (start, end) in [(0.0_f32, 1.0_f32), (1.0, 5.0), (-3.0, 7.5), (0.3, 0.1)] {
            for t in sweep_ts() {
                let value = f(start, end, t);
                let bits = if value.is_nan() {
                    0x7fc0_0000
                } else {
                    value.to_bits()
                };
                for byte in bits.to_le_bytes() {
                    hash = (hash ^ u64::from(byte)).wrapping_mul(0x0000_0100_0000_01b3);
                }
            }
        }
        hash
    }

    #[track_caller]
    pub(crate) fn assert_bits(actual: f32, expected: f32) {
        assert_eq!(
            actual.to_bits(),
            expected.to_bits(),
            "{actual:e} ({:#010x}) != {expected:e} ({:#010x})",
            actual.to_bits(),
            expected.to_bits()
        );
    }
}

#[cfg(test)]
mod tests {
    use super::test_support::*;

    #[test]
    fn within_holds_a_non_finite_expectation_exactly() {
        assert!(within(1.05, 1.0, 0.1) && !within(1.2, 1.0, 0.1));
        assert!(within(NAN, NAN, 0.0) && !within(0.0, NAN, 1.0));
        assert!(within(INF, INF, 0.0) && !within(f32::MAX, INF, f32::MAX));
    }
}
