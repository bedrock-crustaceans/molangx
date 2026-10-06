//! The arm64 bodies: products fused into multiply-adds, a NaN bound or argument ignored where
//! `min`/`max` ignore it. The NaN of each arithmetic operation is chosen by
//! [`arm64`](crate::numeric::arch::arm64), operands in the order written; each invalid standard
//! function names its own NaN.

mod ease;

pub(crate) use ease::*;

use super::{SIN_INDEX_SCALE, SIN_INDEX_STEP, Sort};
pub(crate) use crate::numeric::arch::arm64::{add, div, mul, sub};
use crate::numeric::{
    PostOp,
    arch::arm64::{DEFAULT_NAN, max, min, mul_add, to_int},
};
use crate::stdlib::math::{INVERSE_TRIG_TOLERANCE, RAD_TO_DEG, transcendental};

/// The NaN of an invalid `sin`, `cos`, `ln` or `pow`.
const NEGATIVE_NAN: f32 = f32::from_bits(0xffc0_0000);

/// `1 − f32::EPSILON`: the span factor of `math.random_integer` with literal bounds.
pub(crate) const ONE_MINUS_EPSILON: f32 = f32::from_bits(0x3f7f_fffe);

/// `sin(i · SIN_INDEX_STEP)`.
#[inline]
pub(crate) fn sin_table_entry(index: i32) -> f32 {
    sin((index & 0xffff) as f32 * SIN_INDEX_STEP)
}

/// Entry `trunc(x·SIN_INDEX_SCALE) mod 65536`.
#[inline]
pub(crate) fn table_sin(radians: f32) -> f32 {
    sin_table_entry(to_int(radians * SIN_INDEX_SCALE))
}

/// Entry `trunc(x·SIN_INDEX_SCALE + 16384) mod 65536`, the multiply-add rounded once.
#[inline]
pub(crate) fn table_cos(radians: f32) -> f32 {
    sin_table_entry(to_int(mul_add(radians, SIN_INDEX_SCALE, 16384.0)))
}

/// `(min(a, b), max(a, b))`: a NaN against a number is dropped.
#[inline]
pub(crate) fn sorted_bounds(_sort: Sort, a: f32, b: f32) -> (f32, f32) {
    (min(a, b), max(a, b))
}

/// `lo + r·(hi − lo)`, rounded once.
#[inline]
pub(crate) fn interpolate(lo: f32, hi: f32, r: f32) -> f32 {
    mul_add(sub(hi, lo), r, lo)
}

/// The span `(hi + 1 − lo) − hi·ε`, rounded once.
#[inline]
pub(crate) fn integer_reach(lo: f32, hi: f32) -> f32 {
    mul_add(hi, -f32::EPSILON, sub(add(hi, 1.0), lo))
}

/// The span `(hi·(1 − ε) + 1) − lo`.
#[inline]
pub(crate) fn integer_reach_const(lo: f32, hi: f32) -> f32 {
    sub(mul_add(hi, ONE_MINUS_EPSILON, 1.0), lo)
}

/// `v > hi ? hi : max(v, lo)`.
#[inline]
pub(crate) fn clamp_draw(v: f32, lo: f32, hi: f32) -> f32 {
    if v > hi { hi } else { max(v, lo) }
}

/// `floor(lo + r·reach)` rounded once, clamped to `[lo, hi]` upper bound first.
#[inline]
pub(crate) fn integer_draw(lo: f32, hi: f32, reach: f32, r: f32) -> f32 {
    let v = mul_add(reach, r, lo).floor();
    clamp_draw(v, lo, hi)
}

/// `sum + integer_draw(…)`.
#[inline]
pub(crate) fn integer_roll(sum: f32, lo: f32, hi: f32, reach: f32, r: f32) -> f32 {
    add(sum, integer_draw(lo, hi, reach, r))
}

/// `r·(hi − lo) + (sum + lo)`, rounded once.
#[inline]
pub(crate) fn roll(sum: f32, lo: f32, hi: f32, r: f32) -> f32 {
    mul_add(r, sub(hi, lo), add(sum, lo))
}

/// Clamps to `[-1, 1]` unless `|x| > 1.0005`; a NaN becomes −1.
#[inline]
pub(crate) fn inverse_trig_argument(x: f32) -> f32 {
    if x.abs() > INVERSE_TRIG_TOLERANCE {
        x
    } else if x > 1.0 {
        1.0
    } else {
        max(x, -1.0)
    }
}

/// `rad·(k·S) + O`, with `k·S` rounded and the rest rounded once.
#[inline]
pub(crate) fn degrees(rad: f32, post: PostOp) -> f32 {
    if post.is_identity() {
        mul(rad, RAD_TO_DEG)
    } else {
        mul_add(rad, mul(RAD_TO_DEG, post.scale), post.offset)
    }
}

/// `(3 − 2t)·(t·t)`.
#[inline]
pub(crate) fn hermite_blend(t: f32) -> f32 {
    mul(sub(3.0, add(t, t)), mul(t, t))
}

/// `a + t·(b − a)`, rounded once.
#[inline]
pub(crate) fn lerp(a: f32, b: f32, t: f32) -> f32 {
    mul_add(t, sub(b, a), a)
}

/// `t·wrap(b − a) + a`, rounded twice.
#[inline]
pub(crate) fn lerprotate(a: f32, b: f32, t: f32) -> f32 {
    add(mul(t, wrap_angle(sub(b, a))), a)
}

/// `(v − a) / (b − a)`.
#[inline]
pub(crate) fn inverse_lerp(a: f32, b: f32, v: f32) -> f32 {
    div(sub(v, a), sub(b, a))
}

/// Wraps degrees into `[-180, 180)`.
#[inline]
pub(crate) fn wrap_angle(x: f32) -> f32 {
    let r = transcendental::rem(add(x, 180.0), 360.0);
    sub(if r < 0.0 { add(r, 360.0) } else { r }, 180.0)
}

/// ±∞ gives [`NEGATIVE_NAN`].
#[inline]
pub(crate) fn sin(x: f32) -> f32 {
    transcendental::sin(x, NEGATIVE_NAN)
}

/// ±∞ gives [`NEGATIVE_NAN`].
#[inline]
pub(crate) fn cos(x: f32) -> f32 {
    transcendental::cos(x, NEGATIVE_NAN)
}

/// A negative argument (−∞ included) gives [`NEGATIVE_NAN`].
#[inline]
pub(crate) fn ln(x: f32) -> f32 {
    transcendental::ln(x, NEGATIVE_NAN)
}

/// An invalid operation gives [`NEGATIVE_NAN`].
#[inline]
pub(crate) fn pow(a: f32, b: f32) -> f32 {
    transcendental::pow(a, b, NEGATIVE_NAN)
}

/// A negative argument (−∞ included) gives [`DEFAULT_NAN`].
#[inline]
pub(crate) fn sqrt(x: f32) -> f32 {
    transcendental::sqrt(x, DEFAULT_NAN)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::numeric::test_support::*;

    #[test]
    fn a_nan_bound_is_dropped() {
        for sort in [Sort::RunTime, Sort::Literal] {
            assert_eq!(sorted_bounds(sort, NAN, 1.0), (1.0, 1.0));
            assert_eq!(sorted_bounds(sort, 1.0, NAN), (1.0, 1.0));
        }
    }

    #[test]
    fn a_nan_inverse_trig_argument_counts_as_minus_one() {
        assert_bits(inverse_trig_argument(NAN), -1.0);
        assert_bits(inverse_trig_argument(-1.0004), -1.0);
    }

    #[test]
    fn each_invalid_operation_chooses_its_nan() {
        for x in [-1.0, -1.0e-45, -INF] {
            assert_eq!(ln(x).to_bits(), 0xffc0_0000, "ln({x})");
            assert_eq!(sqrt(x).to_bits(), 0x7fc0_0000, "sqrt({x})");
        }
        for x in [INF, -INF] {
            assert_eq!(sin(x).to_bits(), 0xffc0_0000);
            assert_eq!(cos(x).to_bits(), 0xffc0_0000);
        }
        assert_eq!(pow(-8.0, 1.0 / 3.0).to_bits(), 0xffc0_0000);
        assert_eq!(out_circ(0.0, 1.0, 3.0).to_bits(), 0x7fc0_0000);
    }

    #[test]
    fn the_sine_table_multiplies_by_the_step() {
        assert_bits(sin_table_entry(1), sin(SIN_INDEX_STEP));
        assert_bits(table_sin(1.0e6), sin_table_entry(65_535));
    }

    #[test]
    fn each_operation_chooses_its_nan() {
        let (quiet, negative_quiet) = (f32::from_bits(0x7fc0_0001), f32::from_bits(0xffc0_0002));
        // The addend of a multiply-add first, else the left operand.
        assert_eq!(lerp(quiet, negative_quiet, 0.5).to_bits(), 0x7fc0_0001);
        assert_eq!(
            lerprotate(quiet, negative_quiet, 0.5).to_bits(),
            0xffc0_0002
        );
        assert_eq!(
            interpolate(negative_quiet, quiet, 0.5).to_bits(),
            0xffc0_0002
        );
        assert_eq!(roll(negative_quiet, 1.0, quiet, 0.5).to_bits(), 0xffc0_0002);
        assert_eq!(
            integer_roll(negative_quiet, 1.0, 6.0, integer_reach(1.0, 6.0), 0.5).to_bits(),
            0xffc0_0002
        );
        assert_eq!(
            inverse_lerp(quiet, 1.0, negative_quiet).to_bits(),
            0xffc0_0002
        );
        assert_eq!(hermite_blend(negative_quiet).to_bits(), 0xffc0_0002);
        assert_eq!(
            degrees(negative_quiet, PostOp::new(2.0, quiet)).to_bits(),
            0x7fc0_0001
        );
        // Invalid operations on numbers.
        assert_eq!(inverse_lerp(5.0, 5.0, 5.0).to_bits(), 0x7fc0_0000);
        assert_eq!(interpolate(-INF, -180.0, 0.75).to_bits(), 0x7fc0_0000);
        assert_eq!(lerp(-INF, INF, 0.0).to_bits(), 0x7fc0_0000);
    }

    #[test]
    fn the_formulas_fuse_their_products() {
        let x = 0.3_f32;
        assert_bits(hermite_blend(x), (3.0 - (x + x)) * (x * x));
        assert_bits(interpolate(-3.0, 7.5, x), 10.5_f32.mul_add(x, -3.0));
        assert_bits(roll(1.0, -3.0, 7.5, x), x.mul_add(10.5, -2.0));
        assert_bits(
            integer_reach_const(0.0, 9.0),
            9.0_f32.mul_add(ONE_MINUS_EPSILON, 1.0),
        );
        assert_bits(
            degrees(x, PostOp::new(3.0, -1.0)),
            x.mul_add(RAD_TO_DEG * 3.0, -1.0),
        );
    }

    /// Inputs whose fused and unfused results differ, so each fused form is pinned.
    #[test]
    fn the_fused_forms_differ_from_their_unfused_forms_here() {
        let pins: [(f32, f32, f32); 7] = [
            (
                integer_reach_const(0.0, -0.67),
                (-0.67_f32).mul_add(ONE_MINUS_EPSILON, 1.0),
                -0.67 * ONE_MINUS_EPSILON + 1.0,
            ),
            (
                integer_reach_const(0.0, -2.89),
                (-2.89_f32).mul_add(ONE_MINUS_EPSILON, 1.0),
                -2.89 * ONE_MINUS_EPSILON + 1.0,
            ),
            (
                lerp(-3.0, 7.5, 0.3),
                0.3_f32.mul_add(10.5, -3.0),
                0.3 * 10.5 - 3.0,
            ),
            (
                roll(1.0, -3.0, 7.5, 0.1),
                0.1_f32.mul_add(10.5, -2.0),
                0.1 * 10.5 - 2.0,
            ),
            (
                degrees(0.3, PostOp::new(3.0, 0.1)),
                0.3_f32.mul_add(RAD_TO_DEG * 3.0, 0.1),
                0.3 * (RAD_TO_DEG * 3.0) + 0.1,
            ),
            (
                integer_draw(
                    -3.0,
                    6.0,
                    integer_reach(-3.0, 6.0),
                    f32::from_bits(0x3f33_3334),
                ),
                3.0,
                4.0,
            ),
            // `x·0.5` of an odd subnormal `x` is inexact.
            (
                in_out_bounce(f32::from_bits(1), f32::from_bits(4), 0.5),
                f32::from_bits(2),
                f32::from_bits(3),
            ),
        ];
        for (n, (got, fused, unfused)) in pins.into_iter().enumerate() {
            assert_bits(got, fused);
            assert_ne!(
                fused.to_bits(),
                unfused.to_bits(),
                "pin {n} does not tell fused from unfused"
            );
        }
    }
}
