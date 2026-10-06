//! The x86-64 behaviour: every operation rounded on its own, every comparison with a NaN false,
//! `min`/`max` returning the second operand when either is NaN.
//!
//! The arithmetic operations choose their NaN here: a NaN operand is returned quietened, the left
//! one if both are NaN; an invalid operation on two numbers (`∞ − ∞`, `0·∞`, `0/0`) returns
//! [`DEFAULT_NAN`]. Rust's operators leave both to the compiler and host.

use crate::numeric::PostOp;

/// The NaN of an invalid operation: sign bit set, quiet, no payload.
pub(crate) const DEFAULT_NAN: f32 = f32::from_bits(0xffc0_0000);

/// The result of an operation on `a` and `b` whose host result is `r`.
#[inline]
fn chosen(r: f32, a: f32, b: f32) -> f32 {
    let quiet = |x: f32| f32::from_bits(x.to_bits() | 0x0040_0000);
    if a.is_nan() {
        quiet(a)
    } else if b.is_nan() {
        quiet(b)
    } else if r.is_nan() {
        DEFAULT_NAN
    } else {
        r
    }
}

/// `a·b + c`, rounded twice; a NaN `a` wins over a NaN `b` and the product over `c`.
#[inline]
pub(crate) fn mul_add(a: f32, b: f32, c: f32) -> f32 {
    add(mul(a, b), c)
}

/// `c − a·b`, rounded twice.
#[inline]
pub(crate) fn mul_sub(a: f32, b: f32, c: f32) -> f32 {
    sub(c, mul(a, b))
}

/// `a + b`; a NaN `a` wins over a NaN `b`.
#[inline]
pub(crate) fn add(a: f32, b: f32) -> f32 {
    chosen(a + b, a, b)
}

/// `a − b`; a NaN `a` wins over a NaN `b`.
#[inline]
pub(crate) fn sub(a: f32, b: f32) -> f32 {
    chosen(a - b, a, b)
}

/// `a·b`; a NaN `a` wins over a NaN `b`.
#[inline]
pub(crate) fn mul(a: f32, b: f32) -> f32 {
    chosen(a * b, a, b)
}

/// `a / b`; a NaN `a` wins over a NaN `b`.
#[inline]
pub(crate) fn div(a: f32, b: f32) -> f32 {
    chosen(a / b, a, b)
}

/// `−(a·b) − c`, rounded twice.
#[inline]
pub(crate) fn neg_mul_add(a: f32, b: f32, c: f32) -> f32 {
    sub(-mul(a, b), c)
}

/// `a·b − c`, rounded twice.
#[inline]
pub(crate) fn neg_mul_sub(a: f32, b: f32, c: f32) -> f32 {
    mul_add(a, b, -c)
}

/// `(a > b) ? a : b`.
#[inline]
pub(crate) fn max(a: f32, b: f32) -> f32 {
    if a > b { a } else { b }
}

/// `(a < b) ? a : b`.
#[inline]
pub(crate) fn min(a: f32, b: f32) -> f32 {
    if a < b { a } else { b }
}

/// Truncating float-to-int; `i32::MIN` for NaN and out-of-range values.
#[inline]
pub(crate) fn to_int(v: f32) -> i32 {
    if (-2_147_483_648.0..2_147_483_648.0).contains(&v) {
        v as i32
    } else {
        i32::MIN
    }
}

/// `a < b`: false with a NaN.
#[inline]
pub(crate) fn lt(a: f32, b: f32) -> bool {
    a < b
}

/// `a <= b`: false with a NaN.
#[inline]
pub(crate) fn le(a: f32, b: f32) -> bool {
    a <= b
}

/// `raw·S + O` rounded twice; the identity returns `raw` untouched.
#[inline]
pub(crate) fn apply(post: PostOp, raw: f32) -> f32 {
    if post.is_identity() {
        raw
    } else {
        mul_add(raw, post.scale, post.offset)
    }
}

/// `(top·acc)·S + O`, each step rounded, the NaN of `top` winning.
#[inline]
pub(crate) fn mul_post(acc: f32, top: f32, post: PostOp) -> f32 {
    apply(post, mul(top, acc))
}

/// `(a/b)·S + O`.
#[inline]
pub(crate) fn div_post(a: f32, b: f32, post: PostOp) -> f32 {
    apply(post, div(a, b))
}

/// `None` when `|d| < f32::EPSILON`; a NaN divisor passes.
#[inline]
pub(crate) fn div_guard(signed: bool, d: f32) -> Option<f32> {
    if d.abs() < f32::EPSILON {
        None
    } else {
        Some(if signed { d } else { d.abs() })
    }
}

/// Folds `Mul(child, c)` into `child`: the offset is `((c·child.O)·S) + O`.
#[inline]
pub(crate) fn fold_scaled(outer: PostOp, c: f32, child: PostOp) -> PostOp {
    let k = mul(outer.scale, c);
    PostOp {
        scale: mul(k, child.scale),
        offset: mul_add(mul(c, child.offset), outer.scale, outer.offset),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::numeric::test_support::*;

    const NEG_NAN: f32 = f32::from_bits(0xffc0_0000);
    const POS_NAN: f32 = f32::from_bits(0x7fc0_0000);

    fn bits(x: f32) -> u32 {
        x.to_bits()
    }

    #[test]
    fn nan_comparisons_are_false() {
        for (a, b) in [(NAN, 1.0), (1.0, NAN), (NAN, NAN)] {
            assert!(!lt(a, b) && !le(a, b));
        }
    }

    #[test]
    fn max_and_min_return_the_second_operand_on_nan_and_equal_zeros() {
        assert_bits(max(NAN, 4.0), 4.0);
        assert!(max(4.0, NAN).is_nan());
        assert_bits(min(NAN, 4.0), 4.0);
        assert!(min(4.0, NAN).is_nan());
        assert_bits(max(0.0, -0.0), -0.0);
        assert_bits(min(-0.0, 0.0), 0.0);
    }

    #[test]
    fn to_int_gives_the_minimum_outside_the_range() {
        for v in [NAN, INF, -INF, 2_147_483_648.0, -3.0e9, f32::MAX] {
            assert_eq!(to_int(v), i32::MIN, "{v}");
        }
        assert_eq!(to_int(-2_147_483_648.0), i32::MIN);
    }

    #[test]
    fn the_fused_forms_round_twice() {
        let a = 1.0 + 2.0_f32.powi(-12);
        let c = 1.0 + 2.0_f32.powi(-11);
        assert_bits(mul_add(a, a, -c), 0.0);
        assert_bits(mul_sub(a, a, c), 0.0);
        assert_bits(neg_mul_add(a, a, -c), 0.0);
        assert_bits(neg_mul_sub(a, a, c), 0.0);
        assert_bits(apply(PostOp::new(a, -c), a), 0.0);
    }

    #[test]
    fn a_nan_divisor_passes_the_guard() {
        assert!(div_guard(true, NAN).is_some_and(f32::is_nan));
        assert!(div_guard(false, NAN).is_some_and(f32::is_nan));
    }

    #[test]
    fn neg_mul_add_div_and_fold_choose_their_nans() {
        let (q1, q2, q3) = (
            f32::from_bits(0x7fc0_0001),
            f32::from_bits(0xffc0_0002),
            f32::from_bits(0x7fc0_0003),
        );
        assert_eq!(neg_mul_add(q1, q2, q3).to_bits(), 0xffc0_0001);
        assert_eq!(neg_mul_add(1.0, q2, q3).to_bits(), 0x7fc0_0002);
        assert_eq!(neg_mul_add(INF, 0.0, q3).to_bits(), 0x7fc0_0000);
        assert_eq!(neg_mul_add(INF, 1.0, -INF).to_bits(), 0xffc0_0000);
        assert_eq!(div_post(q2, q1, ID).to_bits(), 0xffc0_0002);
        assert_eq!(
            div_post(q2, q1, PostOp::new(q3, 1.0)).to_bits(),
            0xffc0_0002
        );
        assert_eq!(div_post(0.0, 0.0, ID).to_bits(), 0xffc0_0000);
        assert_eq!(
            div_post(1.0, 2.0, PostOp::new(INF, -INF)).to_bits(),
            0xffc0_0000
        );
        let folded = fold_scaled(PostOp::new(q1, q2), q3, PostOp::new(q2, q3));
        assert_eq!(
            [folded.scale, folded.offset].map(f32::to_bits),
            [0x7fc0_0001, 0x7fc0_0003]
        );
        let folded = fold_scaled(PostOp::new(INF, 1.0), 0.0, PostOp::new(1.0, 1.0));
        assert_eq!(
            [folded.scale, folded.offset].map(f32::to_bits),
            [0xffc0_0000, 0xffc0_0000]
        );
    }

    #[test]
    fn the_post_ops_of_mul_div_and_fold_round_each_step() {
        let x = third();
        assert_bits(
            div_post(1.0, 3.0, PostOp::new(3.0, -1.0)),
            (1.0_f32 / 3.0) * 3.0 - 1.0,
        );
        assert_bits(mul_post(x, 3.0, PostOp::new(1.0, -1.0)), 0.0);
        assert_bits(
            fold_scaled(ID, 1.0, PostOp::new(1.0, x * 3.0 - 1.0)).offset,
            0.0,
        );
    }

    #[test]
    fn the_first_nan_operand_wins() {
        for op in [add, sub, mul, div] {
            assert_eq!(bits(op(NEG_NAN, POS_NAN)), 0xffc0_0000);
            assert_eq!(bits(op(POS_NAN, NEG_NAN)), 0x7fc0_0000);
            assert_eq!(bits(op(1.0, POS_NAN)), 0x7fc0_0000);
            assert_eq!(bits(op(NEG_NAN, 1.0)), 0xffc0_0000);
        }
        assert_bits(add(NEG_NAN, POS_NAN), NEG_NAN);
        assert_bits(mul(POS_NAN, NEG_NAN), POS_NAN);
        assert_bits(mul_add(NEG_NAN, 1.0, POS_NAN), NEG_NAN);
        assert_bits(mul_post(POS_NAN, NEG_NAN, ID), NEG_NAN);
    }

    #[test]
    fn a_nan_operand_keeps_its_payload_and_is_quietened() {
        let signalling = f32::from_bits(0x7f80_0001);
        assert_eq!(bits(add(signalling, 1.0)), 0x7fc0_0001);
        assert_eq!(bits(mul(2.0, f32::from_bits(0xffc0_1234))), 0xffc0_1234);
    }

    #[test]
    fn invalid_operations_give_the_default_nan() {
        let inf = f32::INFINITY;
        assert_eq!(bits(add(inf, -inf)), 0xffc0_0000);
        assert_eq!(bits(sub(inf, inf)), 0xffc0_0000);
        assert_eq!(bits(mul(0.0, inf)), 0xffc0_0000);
        assert_eq!(bits(div(0.0, 0.0)), 0xffc0_0000);
        assert_eq!(bits(div(-inf, inf)), 0xffc0_0000);
    }

    #[test]
    fn numbers_are_the_plain_operations() {
        let values = [
            0.0_f32,
            -0.0,
            1.0,
            -2.5,
            0.1,
            3.0e38,
            -1.0e-45,
            f32::INFINITY,
            f32::NEG_INFINITY,
        ];
        for a in values {
            for b in values {
                for (got, want) in [
                    (add(a, b), a + b),
                    (sub(a, b), a - b),
                    (mul(a, b), a * b),
                    (div(a, b), a / b),
                ] {
                    if want.is_nan() {
                        assert_eq!(bits(got), 0xffc0_0000, "{a} {b}");
                    } else {
                        assert_eq!(bits(got), bits(want), "{a} {b}");
                    }
                }
            }
        }
    }
}
