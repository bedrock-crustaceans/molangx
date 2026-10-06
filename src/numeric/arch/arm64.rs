//! The arm64 behaviour: multiply-adds rounded once, `min`/`max` ignoring a NaN against a number,
//! `<` and `<=` true with a NaN operand.
//!
//! The arithmetic operations (`+ − · /` and the multiply-adds) choose their NaN here. Of the NaN
//! operands, in priority order, the first signalling one is returned quietened, else the first
//! quiet one; an invalid operation on numbers gives [`DEFAULT_NAN`]. A two-operand operation checks
//! its left operand first. A multiply-add checks the addend, then the first factor, then the
//! second, after negating the inputs its form negates; `∞·0` with a quiet NaN addend is an invalid
//! operation.

use crate::numeric::PostOp;

/// The NaN of an invalid operation: sign bit clear, quiet, no payload.
pub(crate) const DEFAULT_NAN: f32 = f32::from_bits(0x7fc0_0000);

const QUIET_BIT: u32 = 0x0040_0000;

#[inline]
fn is_signalling(x: f32) -> bool {
    x.is_nan() && x.to_bits() & QUIET_BIT == 0
}

/// The NaN result of an operation on `operands`, given in priority order.
#[cold]
fn nan_of<const N: usize>(operands: [f32; N]) -> f32 {
    let quiet = |x: f32| f32::from_bits(x.to_bits() | QUIET_BIT);
    let signalling = operands.into_iter().find(|&x| is_signalling(x));
    signalling
        .or_else(|| operands.into_iter().find(|x| x.is_nan()))
        .map_or(DEFAULT_NAN, quiet)
}

/// `r`, or [`nan_of`] the operands when `r` is NaN.
#[inline]
fn chosen<const N: usize>(r: f32, operands: [f32; N]) -> f32 {
    if r.is_nan() { nan_of(operands) } else { r }
}

/// `a + b`.
#[inline]
pub(crate) fn add(a: f32, b: f32) -> f32 {
    chosen(a + b, [a, b])
}

/// `a − b`.
#[cfg(any(feature = "stdlib", test))]
#[inline]
pub(crate) fn sub(a: f32, b: f32) -> f32 {
    chosen(a - b, [a, b])
}

/// `a·b`.
#[inline]
pub(crate) fn mul(a: f32, b: f32) -> f32 {
    chosen(a * b, [a, b])
}

/// `a / b`.
#[inline]
pub(crate) fn div(a: f32, b: f32) -> f32 {
    chosen(a / b, [a, b])
}

/// `a·b + c`, rounded once; a NaN result checks `c`, then `a`, then `b`.
#[inline]
pub(crate) fn mul_add(a: f32, b: f32, c: f32) -> f32 {
    let r = a.mul_add(b, c);
    if r.is_nan() { fused_nan(a, b, c) } else { r }
}

#[cold]
fn fused_nan(a: f32, b: f32, c: f32) -> f32 {
    let infinity_times_zero = (a.is_infinite() && b == 0.0) || (a == 0.0 && b.is_infinite());
    if infinity_times_zero && c.is_nan() && !is_signalling(c) {
        DEFAULT_NAN
    } else {
        nan_of([c, a, b])
    }
}

/// `c − a·b`, rounded once; a NaN `a` comes back negated.
#[inline]
pub(crate) fn mul_sub(a: f32, b: f32, c: f32) -> f32 {
    mul_add(-a, b, c)
}

/// `−(a·b) − c`, rounded once; a NaN `a` or `c` comes back negated.
#[inline]
pub(crate) fn neg_mul_add(a: f32, b: f32, c: f32) -> f32 {
    mul_add(-a, b, -c)
}

/// `a·b − c`, rounded once; a NaN `c` comes back negated.
#[inline]
pub(crate) fn neg_mul_sub(a: f32, b: f32, c: f32) -> f32 {
    mul_add(a, b, -c)
}

/// The larger operand, `+0` greater than `−0`. A NaN against a number gives the number; of two
/// NaNs, the second, as it is.
#[inline]
pub(crate) fn max(a: f32, b: f32) -> f32 {
    if a.is_nan() {
        return b;
    }
    let take_a = b.is_nan() || a > b || (a == b && a.is_sign_positive());
    if take_a { a } else { b }
}

/// The smaller operand, `−0` less than `+0`. A NaN against a number gives the number; of two NaNs,
/// the second, as it is.
#[inline]
pub(crate) fn min(a: f32, b: f32) -> f32 {
    if a.is_nan() {
        return b;
    }
    let take_a = b.is_nan() || a < b || (a == b && a.is_sign_negative());
    if take_a { a } else { b }
}

/// Truncating float-to-int, saturating, NaN to 0.
#[inline]
pub(crate) fn to_int(v: f32) -> i32 {
    v as i32
}

/// `a < b`: true with a NaN.
#[allow(
    clippy::neg_cmp_op_on_partial_ord,
    reason = "`!(a >= b)` is the point: true with a NaN"
)]
#[inline]
pub(crate) fn lt(a: f32, b: f32) -> bool {
    !(a >= b)
}

/// `a <= b`: true with a NaN.
#[allow(
    clippy::neg_cmp_op_on_partial_ord,
    reason = "`!(a > b)` is the point: true with a NaN"
)]
#[inline]
pub(crate) fn le(a: f32, b: f32) -> bool {
    !(a > b)
}

/// `raw·S + O` rounded once; the identity returns `raw` untouched.
#[inline]
pub(crate) fn apply(post: PostOp, raw: f32) -> f32 {
    if post.is_identity() {
        raw
    } else {
        mul_add(raw, post.scale, post.offset)
    }
}

/// `(top·S)·acc + O`, with `top·S` rounded and the rest rounded once; `top` is the left operand.
#[inline]
pub(crate) fn mul_post(acc: f32, top: f32, post: PostOp) -> f32 {
    if post.is_identity() {
        mul(top, acc)
    } else {
        mul_add(mul(top, post.scale), acc, post.offset)
    }
}

/// `(a·S)/b + O`; nothing is fused.
#[inline]
pub(crate) fn div_post(a: f32, b: f32, post: PostOp) -> f32 {
    if post.is_identity() {
        div(a, b)
    } else {
        add(div(mul(a, post.scale), b), post.offset)
    }
}

/// `None` when `|d| < f32::EPSILON` or `d` is NaN.
#[inline]
pub(crate) fn div_guard(signed: bool, d: f32) -> Option<f32> {
    if d.abs() < f32::EPSILON || d.is_nan() {
        None
    } else {
        Some(if signed { d } else { d.abs() })
    }
}

/// Folds `Mul(child, c)` into `child`: the offset is `(S·c)·child.O + O` rounded once.
#[inline]
pub(crate) fn fold_scaled(outer: PostOp, c: f32, child: PostOp) -> PostOp {
    let k = mul(outer.scale, c);
    PostOp {
        scale: mul(k, child.scale),
        offset: mul_add(k, child.offset, outer.offset),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::numeric::test_support::*;

    const QUIET: f32 = f32::from_bits(0x7fc0_0001);
    const NEGATIVE_QUIET: f32 = f32::from_bits(0xffc0_0002);
    const SIGNALLING: f32 = f32::from_bits(0x7f80_0003);
    const NEGATIVE_SIGNALLING: f32 = f32::from_bits(0xff80_0004);

    #[track_caller]
    fn assert_nan(actual: f32, expected: u32) {
        assert_eq!(
            actual.to_bits(),
            expected,
            "{:#010x} != {expected:#010x}",
            actual.to_bits()
        );
    }

    #[test]
    fn max_and_min_give_the_number_or_the_second_of_two_nans() {
        let (quiet, signalling) = (f32::from_bits(0xffc0_0001), f32::from_bits(0x7f80_0002));
        for f in [max, min] {
            assert_eq!(f(quiet, signalling).to_bits(), 0x7f80_0002);
            assert_eq!(f(signalling, quiet).to_bits(), 0xffc0_0001);
            assert_eq!(f(signalling, 3.0), 3.0);
            assert_eq!(f(3.0, signalling), 3.0);
            assert_eq!(f(quiet, -3.0), -3.0);
        }
    }

    #[test]
    fn nan_makes_less_than_and_less_equal_true() {
        for (a, b) in [(NAN, 1.0), (1.0, NAN), (NAN, NAN)] {
            assert!(lt(a, b) && le(a, b));
        }
    }

    #[test]
    fn max_and_min_ignore_a_nan_and_order_the_zeros() {
        assert_bits(max(4.0, NAN), 4.0);
        assert_bits(min(4.0, NAN), 4.0);
        assert!(max(NAN, NAN).is_nan());
        assert_bits(max(0.0, -0.0), 0.0);
        assert_bits(min(0.0, -0.0), -0.0);
    }

    #[test]
    fn to_int_saturates_and_maps_nan_to_zero() {
        assert_eq!(to_int(NAN), 0);
        assert_eq!(to_int(INF), i32::MAX);
        assert_eq!(to_int(2_147_483_648.0), i32::MAX);
        assert_eq!(to_int(-INF), i32::MIN);
    }

    #[test]
    fn the_fused_forms_round_once() {
        let a = 1.0 + 2.0_f32.powi(-12);
        let c = 1.0 + 2.0_f32.powi(-11);
        let residue = 2.0_f32.powi(-24);
        assert_bits(mul_add(a, a, -c), residue);
        assert_bits(mul_sub(a, a, c), -residue);
        assert_bits(neg_mul_add(a, a, -c), -residue);
        assert_bits(neg_mul_sub(a, a, c), residue);
        assert_bits(apply(PostOp::new(a, -c), a), residue);
    }

    #[test]
    fn a_nan_divisor_fires_the_guard() {
        assert_eq!(div_guard(true, NAN), None);
        assert_eq!(div_guard(false, -NAN), None);
    }

    #[test]
    fn the_post_ops_of_mul_div_and_fold_fuse_where_they_differ() {
        let x = third();
        assert_bits(
            mul_post(x, 3.0, PostOp::new(1.0, -1.0)),
            x.mul_add(3.0, -1.0),
        );
        assert_bits(div_post(1.0, 3.0, PostOp::new(3.0, -1.0)), 3.0 / 3.0 - 1.0);
        assert_bits(
            fold_scaled(PostOp::new(1.0, -1.0), 3.0, PostOp::new(1.0, x)).offset,
            3.0_f32.mul_add(x, -1.0),
        );
    }

    #[test]
    fn a_nan_operand_comes_back_quietened_with_its_sign_and_payload() {
        for op in [add, sub, mul, div] {
            assert_nan(op(QUIET, 1.0), 0x7fc0_0001);
            assert_nan(op(1.0, NEGATIVE_QUIET), 0xffc0_0002);
            assert_nan(op(SIGNALLING, -INF), 0x7fc0_0003);
            assert_nan(op(0.0, NEGATIVE_SIGNALLING), 0xffc0_0004);
        }
    }

    #[test]
    fn of_two_nans_a_signalling_one_wins_then_the_left_one() {
        for op in [add, sub, mul, div] {
            assert_nan(op(QUIET, NEGATIVE_QUIET), 0x7fc0_0001);
            assert_nan(op(NEGATIVE_QUIET, QUIET), 0xffc0_0002);
            assert_nan(op(QUIET, NEGATIVE_SIGNALLING), 0xffc0_0004);
            assert_nan(op(NEGATIVE_SIGNALLING, SIGNALLING), 0xffc0_0004);
            assert_nan(op(SIGNALLING, NEGATIVE_SIGNALLING), 0x7fc0_0003);
        }
    }

    #[test]
    fn an_invalid_operation_gives_the_default_nan() {
        assert_nan(add(INF, -INF), 0x7fc0_0000);
        assert_nan(sub(-INF, -INF), 0x7fc0_0000);
        assert_nan(mul(-0.0, INF), 0x7fc0_0000);
        assert_nan(div(0.0, -0.0), 0x7fc0_0000);
        assert_nan(div(-INF, INF), 0x7fc0_0000);
        assert_nan(mul_add(INF, 1.0, -INF), 0x7fc0_0000);
        assert_nan(mul_add(0.0, -INF, 1.0), 0x7fc0_0000);
        assert_nan(mul_sub(INF, 1.0, INF), 0x7fc0_0000);
        assert_nan(neg_mul_add(INF, 1.0, -INF), 0x7fc0_0000);
        assert_nan(neg_mul_sub(INF, 1.0, INF), 0x7fc0_0000);
    }

    #[test]
    fn a_multiply_add_checks_the_addend_then_the_first_factor_then_the_second() {
        assert_nan(
            mul_add(QUIET, NEGATIVE_QUIET, f32::from_bits(0x7fc0_0005)),
            0x7fc0_0005,
        );
        assert_nan(mul_add(QUIET, NEGATIVE_QUIET, 1.0), 0x7fc0_0001);
        assert_nan(mul_add(1.0, NEGATIVE_QUIET, 1.0), 0xffc0_0002);
        assert_nan(
            mul_add(QUIET, NEGATIVE_SIGNALLING, NEGATIVE_QUIET),
            0xffc0_0004,
        );
        assert_nan(mul_add(SIGNALLING, NEGATIVE_SIGNALLING, QUIET), 0x7fc0_0003);
        assert_nan(mul_add(QUIET, 1.0, NEGATIVE_SIGNALLING), 0xffc0_0004);
    }

    #[test]
    fn infinity_times_zero_with_a_quiet_nan_addend_is_invalid() {
        for (a, b) in [(INF, 0.0), (-0.0, INF), (-INF, -0.0)] {
            assert_nan(mul_add(a, b, NEGATIVE_QUIET), 0x7fc0_0000);
            assert_nan(mul_sub(a, b, NEGATIVE_QUIET), 0x7fc0_0000);
            assert_nan(neg_mul_add(a, b, QUIET), 0x7fc0_0000);
            assert_nan(neg_mul_sub(a, b, QUIET), 0x7fc0_0000);
            assert_nan(mul_add(a, b, NEGATIVE_SIGNALLING), 0xffc0_0004);
        }
        assert_nan(mul_add(INF, 2.0, NEGATIVE_QUIET), 0xffc0_0002);
    }

    #[test]
    fn each_fused_form_negates_the_nans_of_its_negated_inputs() {
        // mul_sub negates `a`, neg_mul_add negates `a` and `c`, neg_mul_sub negates `c`; `b` never.
        assert_nan(mul_sub(QUIET, 1.0, 1.0), 0xffc0_0001);
        assert_nan(mul_sub(1.0, QUIET, 1.0), 0x7fc0_0001);
        assert_nan(mul_sub(1.0, 1.0, QUIET), 0x7fc0_0001);
        assert_nan(mul_sub(QUIET, QUIET, 1.0), 0xffc0_0001);
        assert_nan(neg_mul_add(QUIET, 1.0, 1.0), 0xffc0_0001);
        assert_nan(neg_mul_add(1.0, QUIET, 1.0), 0x7fc0_0001);
        assert_nan(neg_mul_add(1.0, 1.0, NEGATIVE_QUIET), 0x7fc0_0002);
        assert_nan(neg_mul_add(SIGNALLING, 1.0, QUIET), 0xffc0_0003);
        assert_nan(neg_mul_sub(QUIET, 1.0, 1.0), 0x7fc0_0001);
        assert_nan(neg_mul_sub(1.0, 1.0, QUIET), 0xffc0_0001);
        assert_nan(neg_mul_sub(NEGATIVE_QUIET, 1.0, QUIET), 0xffc0_0001);
        assert_nan(neg_mul_sub(1.0, NEGATIVE_SIGNALLING, QUIET), 0xffc0_0004);
    }

    #[test]
    fn the_post_op_forms_choose_their_nans() {
        assert_nan(apply(AFFINE, NEGATIVE_SIGNALLING), 0xffc0_0004);
        assert_nan(apply(PostOp::new(INF, QUIET), 0.0), 0x7fc0_0000);
        // `a * b` with `a = top`: the left operand first, also through the scaled form.
        assert_nan(mul_post(QUIET, NEGATIVE_QUIET, ID), 0xffc0_0002);
        assert_nan(mul_post(QUIET, NEGATIVE_QUIET, AFFINE), 0xffc0_0002);
        assert_nan(mul_post(0.0, INF, AFFINE), 0x7fc0_0000);
        assert_nan(div_post(NEGATIVE_QUIET, QUIET, ID), 0xffc0_0002);
        assert_nan(div_post(QUIET, NEGATIVE_QUIET, AFFINE), 0x7fc0_0001);
        assert_nan(div_post(INF, INF, AFFINE), 0x7fc0_0000);
        let folded = fold_scaled(
            PostOp::new(QUIET, 0.0),
            NEGATIVE_QUIET,
            PostOp::new(1.0, 1.0),
        );
        assert_nan(folded.scale, 0x7fc0_0001);
        assert_nan(folded.offset, 0x7fc0_0001);
    }

    #[test]
    fn ordinary_results_are_the_plain_operations() {
        let values = [0.0_f32, -0.0, 1.0, -2.5, 0.1, 3.0e38, -1.0e-45, INF, -INF];
        for a in values {
            for b in values {
                for (got, want) in [
                    (add(a, b), a + b),
                    (sub(a, b), a - b),
                    (mul(a, b), a * b),
                    (div(a, b), a / b),
                ] {
                    if want.is_nan() {
                        assert_nan(got, 0x7fc0_0000);
                    } else {
                        assert_bits(got, want);
                    }
                }
                for c in values {
                    for (got, want) in [
                        (mul_add(a, b, c), a.mul_add(b, c)),
                        (neg_mul_add(a, b, c), (-a).mul_add(b, -c)),
                    ] {
                        if want.is_nan() {
                            assert_nan(got, 0x7fc0_0000);
                        } else {
                            assert_bits(got, want);
                        }
                    }
                }
            }
        }
    }
}
