//! The x86-64 bodies: every operation rounded on its own, in formula order.

mod ease;

pub(crate) use ease::*;

use super::{SIN_INDEX_SCALE, Sort};
use crate::numeric::PostOp;
pub(crate) use crate::numeric::arch::x86_64::{add, div, mul, sub};
use crate::numeric::arch::x86_64::{apply, max, mul_add, to_int};
use crate::stdlib::math::{
    INVERSE_TRIG_TOLERANCE, RAD_TO_DEG,
    transcendental::{self, POSITIVE_NAN},
};

/// The NaN of an invalid `sin`, `cos`, `pow` or `sqrt`.
const NEGATIVE_NAN: f32 = f32::from_bits(0xffc0_0000);

/// `sin(i / SIN_INDEX_SCALE)`.
#[inline]
pub(crate) fn sin_table_entry(index: i32) -> f32 {
    sin((index & 0xffff) as f32 / SIN_INDEX_SCALE)
}

/// Entry `trunc(x·SIN_INDEX_SCALE) mod 65536`.
#[inline]
pub(crate) fn table_sin(radians: f32) -> f32 {
    sin_table_entry(to_int(radians * SIN_INDEX_SCALE))
}

/// Entry `trunc(x·SIN_INDEX_SCALE + 16384) mod 65536`, rounded twice.
#[inline]
pub(crate) fn table_cos(radians: f32) -> f32 {
    sin_table_entry(to_int(mul_add(radians, SIN_INDEX_SCALE, 16384.0)))
}

#[inline]
pub(crate) fn sorted_bounds(sort: Sort, a: f32, b: f32) -> (f32, f32) {
    let lo = if b < a { b } else { a };
    let hi = match sort {
        Sort::RunTime => {
            if a > b {
                a
            } else {
                b
            }
        }
        Sort::Literal => {
            if b > a {
                b
            } else {
                a
            }
        }
    };
    (lo, hi)
}

/// `hi·r + (1 − r)·lo`.
#[inline]
pub(crate) fn interpolate(lo: f32, hi: f32, r: f32) -> f32 {
    add(mul(hi, r), mul(sub(1.0, r), lo))
}

/// The upper interpolation end `(hi·(−ε) + 1) + hi`.
#[inline]
pub(crate) fn integer_reach(_lo: f32, hi: f32) -> f32 {
    add(add(mul(-f32::EPSILON, hi), 1.0), hi)
}

/// As [`integer_reach`].
#[inline]
pub(crate) fn integer_reach_const(lo: f32, hi: f32) -> f32 {
    integer_reach(lo, hi)
}

/// `v > hi ? hi : max(v, lo)`.
#[inline]
pub(crate) fn clamp_draw(v: f32, lo: f32, hi: f32) -> f32 {
    if v > hi { hi } else { max(v, lo) }
}

/// `floor((1 − r)·lo + r·reach)`, clamped to `[lo, hi]` upper bound first.
#[inline]
pub(crate) fn integer_draw(lo: f32, hi: f32, reach: f32, r: f32) -> f32 {
    let v = add(mul(sub(1.0, r), lo), mul(reach, r)).floor();
    clamp_draw(v, lo, hi)
}

/// `sum + integer_draw(…)`.
#[inline]
pub(crate) fn integer_roll(sum: f32, lo: f32, hi: f32, reach: f32, r: f32) -> f32 {
    add(sum, integer_draw(lo, hi, reach, r))
}

/// `sum + (r·hi + (1 − r)·lo)`.
#[inline]
pub(crate) fn roll(sum: f32, lo: f32, hi: f32, r: f32) -> f32 {
    add(sum, add(mul(r, hi), mul(sub(1.0, r), lo)))
}

/// Clamps to `[-1, 1]` unless `|x| > 1.0005`; a NaN passes through.
#[inline]
pub(crate) fn inverse_trig_argument(x: f32) -> f32 {
    if x.abs() > INVERSE_TRIG_TOLERANCE || x.is_nan() {
        x
    } else if x > 1.0 {
        1.0
    } else {
        max(x, -1.0)
    }
}

/// `((rad·k)·S) + O`.
#[inline]
pub(crate) fn degrees(rad: f32, post: PostOp) -> f32 {
    apply(post, mul(rad, RAD_TO_DEG))
}

/// `(3·t)·t − ((t + t)·t)·t`.
#[inline]
pub(crate) fn hermite_blend(t: f32) -> f32 {
    sub(mul(mul(3.0, t), t), mul(mul(add(t, t), t), t))
}

/// `(b − a)·t + a`; of two NaNs, `b − a` wins over `t` and the product over `a`.
#[inline]
pub(crate) fn lerp(a: f32, b: f32, t: f32) -> f32 {
    add(mul(sub(b, a), t), a)
}

/// `wrap(b − a)·t + a`; of two NaNs, the wrapped difference wins over `t` and the product over `a`.
#[inline]
pub(crate) fn lerprotate(a: f32, b: f32, t: f32) -> f32 {
    add(mul(wrap_angle(sub(b, a)), t), a)
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

/// A negative argument (−∞ included) gives [`POSITIVE_NAN`].
#[inline]
pub(crate) fn ln(x: f32) -> f32 {
    transcendental::ln(x, POSITIVE_NAN)
}

/// An invalid operation gives [`NEGATIVE_NAN`].
#[inline]
pub(crate) fn pow(a: f32, b: f32) -> f32 {
    transcendental::pow(a, b, NEGATIVE_NAN)
}

/// A negative argument (−∞ included) gives [`NEGATIVE_NAN`].
#[inline]
pub(crate) fn sqrt(x: f32) -> f32 {
    transcendental::sqrt(x, NEGATIVE_NAN)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::numeric::test_support::*;

    #[test]
    fn a_nan_bound_survives_the_sort() {
        assert!(sorted_bounds(Sort::RunTime, NAN, 1.0).0.is_nan());
        assert!(sorted_bounds(Sort::RunTime, 1.0, NAN).1.is_nan());
        assert!(
            sorted_bounds(Sort::Literal, NAN, 1.0).0.is_nan()
                && sorted_bounds(Sort::Literal, NAN, 1.0).1.is_nan()
        );
        assert_eq!(sorted_bounds(Sort::Literal, 1.0, NAN), (1.0, 1.0));
    }

    #[test]
    fn degrees_keeps_the_sign_and_payload_of_a_nan() {
        let nan = f32::from_bits(0xffc0_0001);
        assert_eq!(degrees(nan, PostOp::IDENTITY).to_bits(), 0xffc0_0001);
        assert_eq!(degrees(nan, AFFINE).to_bits(), 0xffc0_0001);
        assert_eq!(
            degrees(f32::from_bits(0xff80_0001), PostOp::IDENTITY).to_bits(),
            0xffc0_0001
        );
    }

    #[test]
    fn a_nan_passes_the_inverse_trig_clamp() {
        assert!(inverse_trig_argument(NAN).is_nan());
        assert_bits(inverse_trig_argument(-1.0004), -1.0);
    }

    #[test]
    fn each_invalid_operation_chooses_its_nan() {
        for x in [-1.0, -1.0e-45, -INF] {
            assert_eq!(ln(x).to_bits(), 0x7fc0_0000, "ln({x})");
            assert_eq!(sqrt(x).to_bits(), 0xffc0_0000, "sqrt({x})");
        }
        assert_eq!(ln(f32::from_bits(0xffc0_0000)).to_bits(), 0xffc0_0000);
        for x in [INF, -INF] {
            assert_eq!(sin(x).to_bits(), 0xffc0_0000);
            assert_eq!(cos(x).to_bits(), 0xffc0_0000);
        }
        assert_eq!(pow(-8.0, 1.0 / 3.0).to_bits(), 0xffc0_0000);
        assert_eq!(out_circ(0.0, 1.0, 3.0).to_bits(), 0xffc0_0000);
    }

    #[test]
    fn the_sine_table_divides_by_the_scale() {
        assert_bits(sin_table_entry(1), sin(1.0 / SIN_INDEX_SCALE));
        assert_bits(table_sin(1.0e6), sin_table_entry(0));
    }

    #[test]
    fn each_operation_chooses_its_nan() {
        let (q1, q2, q3) = (
            f32::from_bits(0x7fc0_0001),
            f32::from_bits(0xffc0_0002),
            f32::from_bits(0x7fc0_0003),
        );
        // The left operand of each step first.
        assert_eq!(interpolate(q1, q2, q3).to_bits(), 0xffc0_0002);
        assert_eq!(interpolate(q1, 1.0, q3).to_bits(), 0x7fc0_0003);
        let (lo, hi) = (f32::from_bits(0xffc0_0001), f32::from_bits(0x7fc0_0002));
        assert_eq!(interpolate(lo, hi, 0.5).to_bits(), 0x7fc0_0002);
        assert_eq!(roll(0.0, lo, hi, 0.5).to_bits(), 0x7fc0_0002);
        assert_eq!(integer_reach(0.0, q2).to_bits(), 0xffc0_0002);
        // The clamp returns `lo` for a NaN draw.
        assert_eq!(integer_draw(q1, 6.0, q2, q3).to_bits(), 0x7fc0_0001);
        assert_eq!(integer_roll(q2, q1, 6.0, 6.0, 0.5).to_bits(), 0xffc0_0002);
        assert_eq!(roll(q2, q1, q3, 0.5).to_bits(), 0xffc0_0002);
        assert_eq!(roll(1.0, q1, q3, 0.5).to_bits(), 0x7fc0_0003);
        assert_eq!(hermite_blend(q2).to_bits(), 0xffc0_0002);
        assert_eq!(inverse_lerp(q1, q2, q3).to_bits(), 0x7fc0_0003);
        assert_eq!(inverse_lerp(q1, q2, 1.0).to_bits(), 0x7fc0_0001);
        // Invalid operations on numbers.
        assert_eq!(interpolate(1.0, INF, 0.0).to_bits(), 0xffc0_0000);
        assert_eq!(integer_reach(0.0, -INF).to_bits(), 0xffc0_0000);
        assert_eq!(integer_draw(-INF, 6.0, INF, 0.5), -INF);
        assert_eq!(
            integer_roll(-INF, 1.0, 6.0, 6.0, 0.5).to_bits(),
            0xff80_0000
        );
        assert_eq!(roll(-INF, 1.0, INF, 0.5).to_bits(), 0xffc0_0000);
        assert_eq!(hermite_blend(INF).to_bits(), 0xffc0_0000);
        assert_eq!(inverse_lerp(5.0, 5.0, 5.0).to_bits(), 0xffc0_0000);
    }

    #[test]
    fn the_formulas_round_each_step() {
        let x = 0.3_f32;
        assert_bits(hermite_blend(x), (3.0 * x) * x - ((x + x) * x) * x);
        assert_bits(interpolate(-3.0, 7.5, x), 7.5 * x + (1.0 - x) * -3.0);
        assert_bits(roll(1.0, -3.0, 7.5, x), 1.0 + (x * 7.5 + (1.0 - x) * -3.0));
        assert_bits(
            degrees(x, PostOp::new(3.0, -1.0)),
            (x * RAD_TO_DEG) * 3.0 - 1.0,
        );
    }
}
