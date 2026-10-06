//! The transcendental functions, each evaluated in `f64` and rounded to `f32`, and `sqrt` and the
//! remainder, where `f32` gives the same value; every NaN they return is chosen here.
//!
//! A NaN argument comes back with its quiet bit set (of two, `atan2`'s `y`, `pow`'s base and the
//! dividend win). An invalid operation on other arguments gives the `invalid` NaN the caller
//! passes, or `0xffc00000` for the remainder.

/// The quiet NaN with the sign bit clear.
pub(crate) const POSITIVE_NAN: f32 = f32::from_bits(0x7fc0_0000);

const QUIET_BIT: u32 = 0x0040_0000;

/// The quiet NaN with the sign bit set.
const NEGATIVE_NAN: f32 = f32::from_bits(0xffc0_0000);

#[inline]
fn quiet(x: f32) -> f32 {
    f32::from_bits(x.to_bits() | QUIET_BIT)
}

/// The NaN for a NaN result of a function of `x`.
#[cold]
fn nan_of(x: f32, invalid: f32) -> f32 {
    if x.is_nan() { quiet(x) } else { invalid }
}

/// `r` rounded, or [`nan_of`] when `r` is NaN.
#[inline]
fn rounded(r: f64, x: f32, invalid: f32) -> f32 {
    if r.is_nan() {
        nan_of(x, invalid)
    } else {
        r as f32
    }
}

/// `r` rounded, for a function that gives NaN only for a NaN `x`.
#[inline]
fn rounded_total(r: f64, x: f32) -> f32 {
    rounded(r, x, POSITIVE_NAN)
}

pub(crate) fn sin(x: f32, invalid: f32) -> f32 {
    rounded(libm::sin(f64::from(x)), x, invalid)
}

pub(crate) fn cos(x: f32, invalid: f32) -> f32 {
    rounded(libm::cos(f64::from(x)), x, invalid)
}

/// `|x| > 1` gives [`POSITIVE_NAN`].
pub(crate) fn asin(x: f32) -> f32 {
    rounded(libm::asin(f64::from(x)), x, POSITIVE_NAN)
}

/// `|x| > 1` gives [`POSITIVE_NAN`].
pub(crate) fn acos(x: f32) -> f32 {
    rounded(libm::acos(f64::from(x)), x, POSITIVE_NAN)
}

pub(crate) fn atan(x: f32) -> f32 {
    rounded_total(libm::atan(f64::from(x)), x)
}

pub(crate) fn atan2(y: f32, x: f32) -> f32 {
    let r = libm::atan2(f64::from(y), f64::from(x));
    if r.is_nan() {
        nan_of(y, quiet(x))
    } else {
        r as f32
    }
}

pub(crate) fn exp(x: f32) -> f32 {
    rounded_total(libm::exp(f64::from(x)), x)
}

pub(crate) fn exp2(x: f32) -> f32 {
    rounded_total(libm::exp2(f64::from(x)), x)
}

pub(crate) fn ln(x: f32, invalid: f32) -> f32 {
    rounded(libm::log(f64::from(x)), x, invalid)
}

/// `pow(NaN, 0)` = `pow(1, NaN)` = 1; a NaN `a` with an odd integer `b` gives that NaN quietened,
/// sign bit cleared.
pub(crate) fn pow(a: f32, b: f32, invalid: f32) -> f32 {
    let r = libm::pow(f64::from(a), f64::from(b));
    if r.is_nan() {
        pow_nan(a, b, invalid)
    } else {
        r as f32
    }
}

#[cold]
fn pow_nan(a: f32, b: f32, invalid: f32) -> f32 {
    if a.is_nan() && (b % 2.0).abs() == 1.0 {
        quiet(a.abs())
    } else {
        nan_of(a, nan_of(b, invalid))
    }
}

pub(crate) fn sqrt(x: f32, invalid: f32) -> f32 {
    let r = x.sqrt();
    if r.is_nan() { nan_of(x, invalid) } else { r }
}

/// `a % b`, the remainder truncated toward zero; a zero `b` or an infinite `a` gives `0xffc00000`.
pub(crate) fn rem(a: f32, b: f32) -> f32 {
    let r = a % b;
    if r.is_nan() {
        nan_of(a, nan_of(b, NEGATIVE_NAN))
    } else {
        r
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::numeric::test_support::*;

    /// Differs from every NaN the tests pass.
    const INVALID: f32 = f32::from_bits(0xff80_1234);
    const NEGATIVE_NAN: f32 = f32::from_bits(0xffc0_0000);
    const SIGNALLING: f32 = f32::from_bits(0x7f80_0001);
    const NEGATIVE_SIGNALLING: f32 = f32::from_bits(0xff80_0002);

    fn bits(x: f32) -> u32 {
        x.to_bits()
    }

    #[test]
    fn a_nan_argument_comes_back_quiet_with_its_sign_and_payload() {
        for (x, expected) in [
            (NAN, 0x7fc0_0000),
            (NEGATIVE_NAN, 0xffc0_0000),
            (SIGNALLING, 0x7fc0_0001),
            (NEGATIVE_SIGNALLING, 0xffc0_0002),
        ] {
            let unary: [(&str, f32); 10] = [
                ("sin", sin(x, INVALID)),
                ("cos", cos(x, INVALID)),
                ("asin", asin(x)),
                ("acos", acos(x)),
                ("atan", atan(x)),
                ("exp", exp(x)),
                ("exp2", exp2(x)),
                ("ln", ln(x, INVALID)),
                ("sqrt", sqrt(x, INVALID)),
                ("pow", pow(x, 2.0, INVALID)),
            ];
            for (name, got) in unary {
                assert_eq!(bits(got), expected, "{name}({:#x})", bits(x));
            }
            assert_eq!(bits(pow(3.0, x, INVALID)), expected);
            assert_eq!(bits(atan2(x, 1.0)), expected);
            assert_eq!(bits(atan2(1.0, x)), expected);
        }
    }

    #[test]
    fn of_two_nans_the_first_argument_wins() {
        for (first, second) in [
            (NAN, NEGATIVE_NAN),
            (NEGATIVE_NAN, NAN),
            (SIGNALLING, NEGATIVE_NAN),
            (NEGATIVE_NAN, SIGNALLING),
        ] {
            assert_eq!(bits(atan2(first, second)), bits(first) | QUIET_BIT);
            assert_eq!(bits(pow(first, second, INVALID)), bits(first) | QUIET_BIT);
        }
    }

    #[test]
    fn an_invalid_operation_gives_the_callers_nan() {
        for x in [INF, -INF] {
            assert_eq!(bits(sin(x, INVALID)), bits(INVALID));
            assert_eq!(bits(cos(x, INVALID)), bits(INVALID));
        }
        for x in [-1.0, -1.0e-45, -INF] {
            assert_eq!(bits(ln(x, INVALID)), bits(INVALID));
            assert_eq!(bits(sqrt(x, INVALID)), bits(INVALID));
        }
        assert_eq!(bits(pow(-8.0, 1.0 / 3.0, INVALID)), bits(INVALID));
        assert_eq!(bits(pow(-INF, 0.5, INVALID)), 0x7f80_0000);
    }

    #[test]
    fn the_remainder_chooses_its_nans() {
        for (a, b) in [
            (5.0, 0.0),
            (1.0, -0.0),
            (INF, 3.0),
            (-INF, INF),
            (-INF, 0.0),
        ] {
            assert_eq!(bits(rem(a, b)), 0xffc0_0000, "{a} % {b}");
        }
        for (first, second) in [
            (NAN, NEGATIVE_NAN),
            (NEGATIVE_SIGNALLING, NAN),
            (SIGNALLING, 0.0),
            (INF, NEGATIVE_SIGNALLING),
        ] {
            let expected = if first.is_nan() {
                bits(first)
            } else {
                bits(second)
            } | QUIET_BIT;
            assert_eq!(bits(rem(first, second)), expected);
        }
        assert_bits(rem(-4.0, 2.0), -0.0);
        assert_bits(rem(7.5, -2.0), 1.5);
        assert_bits(rem(-3.0, INF), -3.0);
    }

    #[test]
    fn asin_and_acos_beyond_one_give_the_positive_nan() {
        for x in [1.000_001, -1.000_001, 2.0, INF, -INF] {
            assert_eq!(bits(asin(x)), 0x7fc0_0000, "asin({x})");
            assert_eq!(bits(acos(x)), 0x7fc0_0000, "acos({x})");
        }
    }

    #[test]
    fn pow_keeps_its_special_cases() {
        for nan in [NAN, NEGATIVE_NAN, SIGNALLING] {
            assert_bits(pow(nan, 0.0, INVALID), 1.0);
            assert_bits(pow(1.0, nan, INVALID), 1.0);
        }
        for b in [1.0, -1.0, 3.0, -5.0] {
            assert_eq!(
                bits(pow(NEGATIVE_NAN, b, INVALID)),
                0x7fc0_0000,
                "pow(-NaN, {b})"
            );
            assert_eq!(
                bits(pow(NEGATIVE_SIGNALLING, b, INVALID)),
                0x7fc0_0002,
                "pow(-sNaN, {b})"
            );
            assert_eq!(
                bits(pow(SIGNALLING, b, INVALID)),
                0x7fc0_0001,
                "pow(sNaN, {b})"
            );
        }
        for b in [2.0, 0.5, -2.0, 16_777_217.0] {
            assert_eq!(
                bits(pow(NEGATIVE_NAN, b, INVALID)),
                0xffc0_0000,
                "pow(-NaN, {b})"
            );
        }
    }

    #[test]
    fn ordinary_arguments_are_not_nan() {
        assert_bits(sqrt(-0.0, INVALID), -0.0);
        assert_eq!(ln(0.0, INVALID), -INF);
        assert_eq!(ln(-0.0, INVALID), -INF);
        assert_bits(atan2(0.0, 0.0), 0.0);
        assert_bits(asin(1.0), std::f32::consts::FRAC_PI_2);
    }

    /// Arguments where the `f32` functions round differently.
    #[test]
    fn each_function_is_the_f64_one_rounded() {
        let at = f32::from_bits;
        assert_eq!(bits(sin(at(0x400c_dd30), INVALID)), 0x3f4e_d304);
        assert_eq!(bits(cos(at(0x3e99_1688), INVALID)), 0x3f74_a444);
        assert_eq!(bits(asin(at(0x3bf8_b588))), 0x3bf8_b625);
        assert_eq!(bits(exp2(at(0xb59b_cfa6))), 0x3f7f_fff3);
        assert_eq!(bits(pow(at(0x3f8e_147b), -10.0, INVALID)), 0x3eb4_5185);
    }

    #[test]
    fn exp2_at_exponents_no_easing_reaches() {
        for (x, expected) in [
            (0xbf80_0000, 0x3f00_0000),
            (0xc080_0000, 0x3d80_0000),
            (0xbfd6_ab74, 0x3ea0_1b65),
            (0xbf13_7db9, 0x3f2b_b6c1),
            (0xc08c_be3f, 0x3d42_4033),
        ] {
            assert_eq!(bits(exp2(f32::from_bits(x))), expected, "exp2({x:#x})");
        }
    }
}
