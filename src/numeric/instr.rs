//! Truthiness, comparisons, `add`, `negate`, `mul`, `div` and `array_index`.

use super::PostOp;
use super::arch::native;

/// `x != 0.0`: `−0.0` is false and NaN is **true**.
#[inline]
pub fn truthy(x: f32) -> bool {
    x != 0.0
}

/// `!x`: the node's "false" constant when `x` is truthy (NaN included), else its "true" constant.
#[inline]
pub fn not(x: f32, post: PostOp) -> f32 {
    post.select(!truthy(x))
}

/// `a < b`. False with a NaN on `X86_64`; true with a NaN on `Arm64`.
#[inline]
pub fn lt(a: f32, b: f32) -> bool {
    native::lt(a, b)
}

/// `a <= b`. False with a NaN on `X86_64`; true with a NaN on `Arm64`.
#[inline]
pub fn le(a: f32, b: f32) -> bool {
    native::le(a, b)
}

/// `a > b`: false with a NaN on both architectures.
#[inline]
pub fn gt(a: f32, b: f32) -> bool {
    a > b
}

/// `a >= b`: false with a NaN on both architectures.
#[inline]
pub fn ge(a: f32, b: f32) -> bool {
    a >= b
}

/// `a == b`: exact, no epsilon; false with a NaN.
#[inline]
pub fn eq(a: f32, b: f32) -> bool {
    a == b
}

/// `a != b`: exact, no epsilon; true with a NaN.
#[inline]
pub fn ne(a: f32, b: f32) -> bool {
    a != b
}

/// The last step of an n-ary `+`: `(top + acc)·S + O`, with `top` the sum of the earlier terms.
///
/// On `X86_64` the NaN of `top` wins over that of `acc`, so in `a + b` the left operand's.
#[inline]
pub fn add(acc: f32, top: f32, post: PostOp) -> f32 {
    post.apply(native::add(top, acc))
}

/// Unary minus: `O − x·S` (rounded once on `Arm64`), or the sign flip `−x` in the plain form.
///
/// `negate(+0)` is −0, unlike `0 − x`. The optimiser folds every negation into a post-op, so the
/// evaluator never runs this.
#[inline]
pub fn negate(x: f32, post: PostOp) -> f32 {
    if post.is_identity() {
        -x
    } else {
        native::mul_sub(x, post.scale, post.offset)
    }
}

/// `acc * top` with the node's post-op.
///
/// `X86_64`: `(top·acc)·S + O`, each step rounded, the NaN of `top` (the left operand of `a * b`)
/// winning. `Arm64`: `acc·(top·S) + O`, with `top·S` rounded and the rest rounded once.
#[inline]
pub fn mul(acc: f32, top: f32, post: PostOp) -> f32 {
    native::mul_post(acc, top, post)
}

/// The division guard, run on the divisor before the numerator is evaluated.
///
/// `None` when `|d| < f32::EPSILON` (or `d` is NaN, on `Arm64` only): the division is `0.0`, with
/// no post-op, and the numerator is **not** evaluated. Otherwise the divisor: `d` when `signed`
/// (`MolangVersion` ≥ 7), `|d|` otherwise.
#[inline]
pub fn div_guard(signed: bool, d: f32) -> Option<f32> {
    native::div_guard(signed, d)
}

/// `a / b` for a divisor that passed [`div_guard`], with the node's post-op.
///
/// `(a/b)·S + O` on `X86_64`, `(a·S)/b + O` on `Arm64`; nothing is fused.
#[inline]
pub fn div(a: f32, b: f32, post: PostOp) -> f32 {
    native::div_post(a, b, post)
}

/// The multiplier a literal divisor folds into: `x / c` becomes `x · (1/c)`, or `x · 0` when
/// `|c| < f32::EPSILON` or `c` is NaN. The sign is kept in every version.
#[inline]
pub fn fold_const_divisor(c: f32) -> f32 {
    if c.abs() >= f32::EPSILON {
        1.0 / c
    } else {
        0.0
    }
}

/// An all-constant `a / b`: `0` when `|b| < f32::EPSILON` or `b` is NaN.
#[inline]
pub fn fold_const_div(a: f32, b: f32) -> f32 {
    if b.abs() >= f32::EPSILON {
        native::div(a, b)
    } else {
        0.0
    }
}

/// The element index of `array.x[index]`: `max(0, trunc(index)) mod len`, or `None` for an empty
/// array.
///
/// NaN gives element 0; past the `i32` range the index is 0 on `X86_64` and saturates on `Arm64`
/// ([`arith::to_int`](super::arith::to_int)).
#[inline]
pub fn array_index(index: f32, len: usize) -> Option<usize> {
    (native::to_int(index).max(0) as usize).checked_rem(len)
}

#[cfg(test)]
mod tests {
    // Expected values keep nine significant digits; arm64 comparisons are asserted in their negated
    // form.
    #![allow(clippy::excessive_precision, clippy::neg_cmp_op_on_partial_ord)]

    use super::*;
    use crate::numeric::{
        ARCH, Arch,
        arch::{arm64, x86_64},
        test_support::*,
    };
    use proptest::prelude::*;

    #[test]
    fn truthiness_is_a_float_compare_with_zero() {
        assert!(truthy(NAN));
        assert!(!truthy(0.0));
        assert!(!truthy(-0.0));
        assert!(truthy(-0.5));
        assert!(truthy(0.000_000_1));
        assert_eq!(not(NAN, ID), 0.0);
        assert_eq!(not(-0.0, ID), 1.0);
        assert_eq!(not(3.0, ID), 0.0);
        assert_eq!(not(0.0, PostOp::new(2.0, 1.0)), 3.0);
        assert_eq!(not(5.0, PostOp::new(2.0, 1.0)), 1.0);
    }

    #[test]
    fn truthy_at_the_extremes() {
        for v in [
            1.0_f32,
            -1.0,
            1.0e-45,
            -1.0e-45,
            f32::MIN_POSITIVE,
            f32::MAX,
            f32::MIN,
            INF,
            -INF,
            NAN,
        ] {
            assert!(truthy(v), "{v:e}");
        }
        assert!(!truthy(0.0));
        assert!(!truthy(-0.0));
    }

    proptest! {
        #[test]
        fn truthy_is_not_equal_to_zero(x in proptest::num::f32::ANY) {
            prop_assert_eq!(truthy(x), x != 0.0);
        }

        #[test]
        fn not_selects_the_opposite_constant(x in proptest::num::f32::ANY, scale in -8.0_f32..8.0, offset in -8.0_f32..8.0) {
            let post = PostOp::new(scale, offset);
            let expected = if truthy(x) { post.falsy_value() } else { post.truthy_value() };
            prop_assert_eq!(not(x, post).to_bits(), expected.to_bits());
        }
    }

    #[test]
    fn not_returns_the_post_op_constants() {
        let post = PostOp::new(-2.0, 0.5);
        assert_eq!(not(0.0, post), -1.5);
        assert_eq!(not(-0.0, post), -1.5);
        assert_eq!(not(7.0, post), 0.5);
        assert_eq!(not(NAN, post), 0.5);
        assert_eq!(not(INF, post), 0.5);
        assert_eq!(not(-INF, ID), 0.0);
        assert_eq!(not(1.0e-45, ID), 0.0);
    }

    #[test]
    fn nan_comparisons_are_ordered_on_x86_64() {
        for (a, b) in [(NAN, 4.0), (4.0, NAN), (NAN, NAN)] {
            assert!(!x86_64::lt(a, b));
            assert!(!x86_64::le(a, b));
            assert!(!gt(a, b));
            assert!(!ge(a, b));
            assert!(!eq(a, b));
            assert!(ne(a, b));
        }
    }

    #[test]
    fn nan_makes_less_than_and_less_equal_true_on_arm64() {
        for (a, b) in [(NAN, 1.0), (1.0, NAN), (NAN, NAN)] {
            assert!(arm64::lt(a, b));
            assert!(arm64::le(a, b));
            assert!(!gt(a, b));
            assert!(!ge(a, b));
        }
    }

    #[test]
    fn equality_and_inequality_do_not_depend_on_nan_handling_of_the_architecture() {
        for (a, b) in [(NAN, 1.0), (1.0, NAN), (NAN, NAN), (INF, NAN)] {
            assert!(!eq(a, b));
            assert!(ne(a, b));
        }
    }

    #[test]
    fn ordinary_comparisons_agree_on_both_architectures() {
        let samples = [
            -2.0_f32,
            -0.0,
            0.0,
            1.0,
            1.000_000_1,
            f32::INFINITY,
            f32::NEG_INFINITY,
        ];
        for a in samples {
            for b in samples {
                assert_eq!(lt(a, b), a < b);
                assert_eq!(le(a, b), a <= b);
                assert_eq!(gt(a, b), a > b);
                assert_eq!(ge(a, b), a >= b);
            }
        }
    }

    #[test]
    fn comparisons_at_equal_and_adjacent_values() {
        let next = f32::from_bits(1.0_f32.to_bits() + 1);
        assert!(!lt(1.0, 1.0));
        assert!(le(1.0, 1.0));
        assert!(!gt(1.0, 1.0));
        assert!(ge(1.0, 1.0));
        assert!(lt(1.0, next));
        assert!(!lt(next, 1.0));
        assert!(le(1.0, next));
        assert!(!le(next, 1.0));
        assert!(gt(next, 1.0));
        assert!(ge(next, 1.0));
        assert!(!ge(1.0, next));
        assert!(eq(1.0, 1.0));
        assert!(!eq(1.0, next));
        assert!(ne(1.0, next));
        assert!(!ne(1.0, 1.0));
    }

    #[test]
    fn comparisons_treat_signed_zeros_and_infinities_as_ordered_values() {
        assert!(!lt(-0.0, 0.0));
        assert!(le(-0.0, 0.0));
        assert!(!gt(0.0, -0.0));
        assert!(ge(0.0, -0.0));
        assert!(lt(-INF, INF));
        assert!(gt(INF, -INF));
        assert!(le(INF, INF));
        assert!(ge(-INF, -INF));
        assert!(lt(f32::MAX, INF));
        assert!(eq(-0.0, 0.0));
        assert!(!ne(-0.0, 0.0));
        assert!(eq(INF, INF));
        assert!(ne(INF, -INF));
    }

    #[test]
    fn gt_and_ge_ignore_the_architecture() {
        for (a, b) in [
            (NAN, 1.0),
            (1.0, NAN),
            (NAN, NAN),
            (1.0, 2.0),
            (2.0, 1.0),
            (2.0, 2.0),
            (-0.0, 0.0),
        ] {
            assert_eq!(gt(a, b), x86_64::lt(b, a));
            assert_eq!(ge(a, b), x86_64::le(b, a));
        }
    }

    proptest! {
        #[test]
        fn comparisons_agree_between_architectures_for_non_nan_inputs(a in proptest::num::f32::NORMAL | proptest::num::f32::SUBNORMAL | proptest::num::f32::ZERO | proptest::num::f32::INFINITE, b in proptest::num::f32::NORMAL | proptest::num::f32::SUBNORMAL | proptest::num::f32::ZERO | proptest::num::f32::INFINITE) {
            prop_assert_eq!(x86_64::lt(a, b), arm64::lt(a, b));
            prop_assert_eq!(x86_64::le(a, b), arm64::le(a, b));
            prop_assert_eq!(gt(a, b), arm64::lt(b, a));
            prop_assert_eq!(ge(a, b), arm64::le(b, a));
        }

        #[test]
        fn arm64_lt_and_le_are_true_exactly_when_the_ordered_opposite_is_false(a in proptest::num::f32::ANY, b in proptest::num::f32::ANY) {
            prop_assert_eq!(arm64::lt(a, b), !(a >= b));
            prop_assert_eq!(arm64::le(a, b), !(a > b));
            prop_assert_eq!(x86_64::lt(a, b), a < b);
            prop_assert_eq!(x86_64::le(a, b), a <= b);
        }
    }

    #[test]
    fn negate_and_add_apply_the_post_op() {
        assert_eq!(negate(2.5, ID), -2.5);
        assert_eq!(negate(0.0, ID).to_bits(), (-0.0_f32).to_bits());
        assert_eq!(negate(2.5, PostOp::new(2.0, 1.0)), -4.0);
        assert_eq!(add(0.1, 0.2, ID), 0.300_000_01);
        assert_eq!(add(1.5, 2.5, PostOp::new(2.0, 1.0)), 9.0);
        let x = third();
        assert_eq!(
            negate(x, PostOp::new(3.0, 1.0)),
            per_arch(1.0 - x * 3.0, (-x).mul_add(3.0, 1.0))
        );
        assert_ne!(1.0 - x * 3.0, (-x).mul_add(3.0, 1.0));
    }

    #[test]
    fn negate_plain_form_is_a_sign_flip() {
        assert_bits(negate(0.0, ID), -0.0);
        assert_bits(negate(-0.0, ID), 0.0);
        assert_bits(negate(1.5, ID), -1.5);
        assert_eq!(negate(INF, ID), -INF);
        assert_eq!(negate(-INF, ID), INF);
        assert!(negate(NAN, ID).is_nan());
        assert_bits(negate(0.0, PostOp::new(1.0, -0.0)), -0.0);
    }

    #[test]
    fn negate_with_a_post_op_is_offset_minus_x_times_scale() {
        assert_eq!(negate(3.0, PostOp::new(1.0, 10.0)), 7.0);
        assert_eq!(negate(3.0, PostOp::new(-2.0, 0.5)), 6.5);
        assert_eq!(negate(0.0, PostOp::new(1.0, 0.5)), 0.5);
        assert!(negate(NAN, PostOp::new(2.0, 1.0)).is_nan());
        assert_eq!(negate(INF, PostOp::new(2.0, 1.0)), -INF);
        assert_eq!(negate(-INF, PostOp::new(2.0, 1.0)), INF);
        assert!(negate(INF, PostOp::new(0.0, 1.0)).is_nan());
    }

    #[test]
    fn add_plain_form_is_a_single_f32_addition() {
        assert_bits(add(-0.0, -0.0, ID), -0.0);
        assert_bits(add(0.0, -0.0, ID), 0.0);
        assert_bits(add(0.1, 0.2, ID), 0.1_f32 + 0.2);
        assert_eq!(add(f32::MAX, f32::MAX, ID), INF);
        assert!(add(INF, -INF, ID).is_nan());
        assert!(add(NAN, 1.0, ID).is_nan());
        assert_eq!(add(INF, 1.0, ID), INF);
    }

    #[test]
    fn add_and_mul_keep_the_earlier_nan_on_x86_64() {
        let (neg, pos) = (f32::from_bits(0xffc0_0000), f32::from_bits(0x7fc0_0000));
        for post in [ID, AFFINE] {
            if ARCH == Arch::X86_64 {
                assert_bits(add(pos, neg, post), neg);
                assert_bits(add(neg, pos, post), pos);
            }
            assert_bits(x86_64::mul_post(pos, neg, post), neg);
            assert_bits(x86_64::mul_post(neg, pos, post), pos);
        }
        if ARCH == Arch::X86_64 {
            assert_bits(add(INF, -INF, ID), neg);
        }
        assert_bits(x86_64::mul_post(0.0, INF, ID), neg);
    }

    #[test]
    fn add_applies_the_post_op_to_the_sum() {
        assert_eq!(add(1.0, 2.0, PostOp::new(3.0, 1.0)), 10.0);
        assert_eq!(add(1.0, 2.0, PostOp::new(1.0, 1.0)), 4.0);
        assert_eq!(add(1.0, -1.0, PostOp::new(5.0, 2.0)), 2.0);
        assert_bits(add(-0.0, -0.0, PostOp::new(2.0, 0.0)), 0.0);
        assert_bits(add(-0.0, -0.0, PostOp::new(2.0, -0.0)), -0.0);
        let post = PostOp::new(3.0, -1.0);
        if ARCH == Arch::X86_64 {
            assert_bits(add(0.0, third(), post), 0.0);
        }
        if ARCH == Arch::Arm64 {
            assert_eq!(add(0.0, third(), post), 2.980_232_2e-8);
        }
    }

    #[test]
    fn mul_plain_form_is_a_single_f32_multiplication() {
        assert_bits(mul(3.0, 0.1, ID), 3.0_f32 * 0.1);
        assert_bits(mul(-0.0, 5.0, ID), -0.0);
        assert_bits(mul(0.0, -5.0, ID), -0.0);
        assert!(mul(INF, 0.0, ID).is_nan());
        assert_eq!(mul(INF, -2.0, ID), -INF);
        assert!(mul(NAN, 1.0, ID).is_nan());
        assert_eq!(mul(f32::MAX, 2.0, ID), INF);
        assert_bits(mul(-0.0, 5.0, PostOp::new(1.0, -0.0)), -0.0);
    }

    #[test]
    fn multiplication_scales_the_product_on_x86_64_and_the_operand_on_arm64() {
        // v.k * v.j * 7
        let (k, j) = (1.4_f32, 0.75_f32);
        let product_then_scale = (k * j) * 7.0;
        assert_ne!(product_then_scale, k * (j * 7.0));
        assert_ne!(product_then_scale, j * (k * 7.0));
        assert_eq!(
            x86_64::mul_post(k, j, PostOp::new(7.0, 0.0)),
            product_then_scale
        );
        assert_eq!(
            x86_64::mul_post(3.0, 0.1, PostOp::new(7.0, 0.0)),
            (3.0_f32 * 0.1) * 7.0
        );
        assert_eq!(arm64::mul_post(k, j, PostOp::new(7.0, 0.0)), k * (j * 7.0));
        assert_eq!(
            arm64::mul_post(k, j, PostOp::new(7.0, 0.25)),
            k.mul_add(j * 7.0, 0.25)
        );
        assert_eq!(
            x86_64::mul_post(k, j, PostOp::new(7.0, 0.25)),
            product_then_scale + 0.25
        );
        assert_eq!(mul(k, j, ID), k * j);
    }

    #[test]
    fn mul_with_an_exact_post_op_agrees_across_architectures() {
        assert_eq!(mul(3.0, 4.0, PostOp::new(2.0, 1.0)), 25.0);
        assert_eq!(mul(-3.0, 4.0, PostOp::new(0.5, 0.25)), -5.75);
        assert_eq!(mul(3.0, 0.0, PostOp::new(2.0, 1.5)), 1.5);
    }

    #[test]
    fn mul_scales_the_operand_before_multiplying_on_arm64_only() {
        let post = PostOp::new(1.0e-30, 0.0);
        assert_eq!(x86_64::mul_post(1.0e30, 1.0e30, post), INF);
        assert_bits(
            arm64::mul_post(1.0e30, 1.0e30, post),
            1.0e30_f32 * (1.0e30_f32 * 1.0e-30),
        );
        assert!((arm64::mul_post(1.0e30, 1.0e30, post) - 1.0e30).abs() < 1.0e24);
    }

    #[test]
    fn mul_normalises_negative_zero_only_through_a_real_post_op() {
        assert_bits(mul(-0.0, 1.0, PostOp::new(2.0, 0.0)), 0.0);
        assert_bits(mul(-0.0, 1.0, PostOp::new(2.0, -0.0)), -0.0);
    }

    #[test]
    fn div_guard_and_its_version_switch() {
        assert_eq!(f32::EPSILON, 1.192_092_9e-7);
        assert_eq!(div_guard(true, 0.000_000_1), None);
        assert_eq!(div_guard(false, 0.000_000_1), None);
        assert_eq!(div_guard(true, -0.000_000_1), None);
        assert_eq!(div_guard(true, 0.0), None);
        assert_eq!(div_guard(true, f32::EPSILON), Some(f32::EPSILON));
        assert_eq!(
            div_guard(true, f32::from_bits(f32::EPSILON.to_bits() - 1)),
            None
        );
        let signed = div_guard(true, -1.0).unwrap();
        assert_eq!(div(5.0, signed, ID), -5.0);
        let unsigned = div_guard(false, -1.0).unwrap();
        assert_eq!(div(5.0, unsigned, ID), 5.0);
        assert_eq!(div(-10.0, div_guard(false, -2.0).unwrap(), ID), -5.0);
        assert_eq!(div(-10.0, div_guard(true, -2.0).unwrap(), ID), 5.0);
        assert_eq!(arm64::div_guard(true, NAN), None);
        assert_eq!(arm64::div_guard(false, NAN), None);
        for signed in [true, false] {
            let divisor = x86_64::div_guard(signed, NAN).unwrap();
            assert!(divisor.is_nan());
            assert!(x86_64::div_post(1.0, divisor, ID).is_nan());
            assert!(x86_64::div_post(1.0, divisor, PostOp::new(2.0, 1.0)).is_nan());
        }
    }

    #[test]
    fn div_guard_negative_threshold_and_signed_zero() {
        for signed in [true, false] {
            assert_eq!(div_guard(signed, -0.0), None);
            assert_eq!(div_guard(signed, -f32::EPSILON * 0.5), None);
            assert_eq!(div_guard(signed, 1.0e-45), None);
            assert_eq!(div_guard(signed, -1.0e-45), None);
            assert_eq!(div_guard(signed, f32::MIN_POSITIVE), None);
        }
        assert_eq!(div_guard(true, -f32::EPSILON), Some(-f32::EPSILON));
        assert_eq!(div_guard(false, -f32::EPSILON), Some(f32::EPSILON));
        assert_eq!(
            div_guard(true, f32::from_bits(f32::EPSILON.to_bits() + 1)),
            Some(f32::from_bits(f32::EPSILON.to_bits() + 1))
        );
    }

    #[test]
    fn div_guard_passes_infinities_and_ordinary_divisors() {
        assert_eq!(div_guard(true, INF), Some(INF));
        assert_eq!(div_guard(true, -INF), Some(-INF));
        assert_eq!(div_guard(false, INF), Some(INF));
        assert_eq!(div_guard(false, -INF), Some(INF));
        assert_eq!(div_guard(true, 4.0), Some(4.0));
        assert_eq!(div_guard(false, 4.0), Some(4.0));
        assert_eq!(div_guard(true, -4.0), Some(-4.0));
        assert_eq!(div_guard(false, -4.0), Some(4.0));
    }

    proptest! {
        #[test]
        fn div_guard_fires_exactly_below_epsilon_for_non_nan_divisors(d in proptest::num::f32::NORMAL | proptest::num::f32::SUBNORMAL | proptest::num::f32::ZERO | proptest::num::f32::INFINITE, signed in any::<bool>()) {
            let guarded = div_guard(signed, d);
            prop_assert_eq!(guarded.is_none(), d.abs() < f32::EPSILON);
            if let Some(divisor) = guarded {
                prop_assert_eq!(divisor.to_bits(), if signed { d } else { d.abs() }.to_bits());
            }
        }
    }

    #[test]
    fn division_scales_after_dividing_on_x86_64_and_before_on_arm64() {
        let (a, b) = (1000.0_f32, 13.0_f32);
        let divide_first = (a / b) * 3.0;
        let scale_first = (a * 3.0) / b;
        assert_ne!(divide_first, scale_first);
        assert_eq!(x86_64::div_post(a, b, PostOp::new(3.0, 0.0)), divide_first);
        assert_eq!(
            x86_64::div_post(a, b, PostOp::new(3.0, 0.5)),
            divide_first + 0.5
        );
        assert_eq!(arm64::div_post(a, b, PostOp::new(3.0, 0.0)), scale_first);
        assert_eq!(
            arm64::div_post(a, b, PostOp::new(3.0, 0.5)),
            scale_first + 0.5
        );
        assert_eq!(div(a, b, ID), a / b);
    }

    #[test]
    fn div_plain_form_is_a_single_division() {
        assert_bits(div(-0.0, 5.0, ID), -0.0);
        assert_bits(div(0.0, -5.0, ID), -0.0);
        assert_eq!(div(1.0, 0.0, ID), INF);
        assert_eq!(div(-1.0, 0.0, ID), -INF);
        assert!(div(0.0, 0.0, ID).is_nan());
        assert_bits(div(1.0, INF, ID), 0.0);
        assert!(div(INF, INF, ID).is_nan());
        assert_eq!(div(6.0, 4.0, ID), 1.5);
    }

    #[test]
    fn div_with_an_exact_post_op_agrees_across_architectures() {
        assert_eq!(div(8.0, 2.0, PostOp::new(3.0, 1.0)), 13.0);
        assert_eq!(div(-8.0, 4.0, PostOp::new(0.5, 0.25)), -0.75);
        assert_bits(div(-0.0, 4.0, PostOp::new(2.0, 0.0)), 0.0);
    }

    #[test]
    fn div_scale_placement_shows_in_overflow() {
        let post = PostOp::new(1.0e10, 0.0);
        assert_bits(
            x86_64::div_post(1.0e30, 1.0e10, post),
            (1.0e30_f32 / 1.0e10) * 1.0e10,
        );
        assert_eq!(arm64::div_post(1.0e30, 1.0e10, post), INF);
    }

    #[test]
    fn div_does_not_fuse_the_offset() {
        let (a, b, post) = (0.7_f32, 0.3_f32, PostOp::new(1.9, 0.11));
        assert_bits(
            arm64::div_post(a, b, post),
            (a * post.scale) / b + post.offset,
        );
        assert_bits(
            x86_64::div_post(a, b, post),
            (a / b) * post.scale + post.offset,
        );
    }

    #[test]
    fn literal_divisors_fold_to_a_reciprocal() {
        assert_eq!(fold_const_divisor(4.0), 0.25);
        assert_eq!(fold_const_divisor(-1.0), -1.0);
        assert_eq!(fold_const_divisor(0.0), 0.0);
        assert_eq!(fold_const_divisor(1.0e-8), 0.0);
        assert_eq!(fold_const_divisor(NAN), 0.0);
        // The numerator is still evaluated, so a NaN one stays NaN.
        assert_eq!(1.0 * fold_const_divisor(0.0), 0.0);
        assert!((NAN * fold_const_divisor(0.0)).is_nan());
        assert_eq!(fold_const_div(1.0, 0.0), 0.0);
        assert_eq!(fold_const_div(0.0, 0.0), 0.0);
        assert_eq!(fold_const_div(1.0, NAN), 0.0);
        assert_eq!(fold_const_div(-10.0, -2.0), 5.0);
        // `7 * 3 / 9` groups the division first.
        let grouped = 7.0 * fold_const_div(3.0, 9.0);
        assert_eq!(grouped, 7.0 * 0.333_333_34_f32);
        assert_eq!(grouped, 2.333_333_5);
        assert_eq!(
            PostOp::new(fold_const_div(3.0, 9.0), 0.0).apply(1.0),
            0.333_333_34
        );
    }

    #[test]
    fn fold_const_divisor_threshold_and_infinities() {
        assert_bits(fold_const_divisor(f32::EPSILON), 1.0 / f32::EPSILON);
        assert_bits(fold_const_divisor(-f32::EPSILON), -(1.0 / f32::EPSILON));
        assert_bits(
            fold_const_divisor(f32::from_bits(f32::EPSILON.to_bits() - 1)),
            0.0,
        );
        assert_bits(
            fold_const_divisor(-f32::from_bits(f32::EPSILON.to_bits() - 1)),
            0.0,
        );
        assert_bits(fold_const_divisor(-0.0), 0.0);
        assert_bits(fold_const_divisor(1.0e-45), 0.0);
        assert_bits(fold_const_divisor(INF), 0.0);
        assert_bits(fold_const_divisor(-INF), -0.0);
        assert_bits(fold_const_divisor(2.0), 0.5);
        assert_bits(fold_const_divisor(3.0), 1.0 / 3.0);
    }

    #[test]
    fn fold_const_div_threshold_and_non_finite_operands() {
        assert_bits(fold_const_div(1.0, f32::EPSILON), 1.0 / f32::EPSILON);
        assert_bits(fold_const_div(1.0, -f32::EPSILON), -(1.0 / f32::EPSILON));
        assert_bits(
            fold_const_div(1.0, f32::from_bits(f32::EPSILON.to_bits() - 1)),
            0.0,
        );
        assert_bits(fold_const_div(-1.0, 0.0), 0.0);
        assert_bits(fold_const_div(INF, 0.0), 0.0);
        assert_bits(fold_const_div(NAN, 0.0), 0.0);
        assert!(fold_const_div(NAN, 2.0).is_nan());
        assert_eq!(fold_const_div(INF, 2.0), INF);
        assert_bits(fold_const_div(1.0, INF), 0.0);
        assert_bits(fold_const_div(-1.0, INF), -0.0);
        assert_eq!(
            fold_const_div(INF, INF).to_bits(),
            per_arch(0xffc0_0000, 0x7fc0_0000)
        );
        assert_bits(fold_const_div(0.0, -4.0), -0.0);
        assert_bits(fold_const_div(1.0, NAN), 0.0);
        assert_bits(fold_const_div(NAN, NAN), 0.0);
    }

    proptest! {
        #[test]
        fn fold_const_div_matches_the_division_wherever_the_guard_does_not_fire(a in proptest::num::f32::NORMAL, b in proptest::num::f32::NORMAL | proptest::num::f32::ZERO | proptest::num::f32::SUBNORMAL) {
            let folded = fold_const_div(a, b);
            if b.abs() >= f32::EPSILON {
                prop_assert_eq!(folded.to_bits(), (a / b).to_bits());
            } else {
                prop_assert_eq!(folded.to_bits(), 0.0_f32.to_bits());
            }
        }
    }

    #[test]
    fn array_index_wraps_and_never_goes_out_of_range() {
        assert_eq!(array_index(0.0, 3), Some(0));
        assert_eq!(array_index(2.9, 3), Some(2));
        assert_eq!(array_index(3.0, 3), Some(0));
        assert_eq!(array_index(7.5, 3), Some(1));
        assert_eq!(array_index(-1.0, 3), Some(0));
        assert_eq!(array_index(NAN, 3), Some(0));
        assert_eq!(array_index(f32::NEG_INFINITY, 3), Some(0));
        for index in [
            0.0,
            2.0,
            -1.0,
            NAN,
            f32::INFINITY,
            f32::NEG_INFINITY,
            1.0e30,
        ] {
            assert_eq!(array_index(index, 0), None);
        }
        assert_eq!(array_index(1.0e30, 1), Some(0));
        assert_eq!(
            array_index(f32::INFINITY, 3),
            per_arch(Some(0), Some((i32::MAX as usize) % 3))
        );
    }

    #[test]
    fn array_index_truncates_negative_fractions_to_the_first_element() {
        for index in [-0.0, -0.5, -0.999, -1.0, -2.5, -1.0e9, -1.0e30] {
            assert_eq!(array_index(index, 5), Some(0), "{index}");
        }
    }

    #[test]
    fn array_index_of_a_single_element_array_is_always_zero() {
        for index in [0.0, 0.5, 1.0, 7.9, -3.0, NAN, INF, -INF, 1.0e10] {
            assert_eq!(array_index(index, 1), Some(0), "{index}");
        }
    }

    #[test]
    fn array_index_wraps_with_the_remainder_by_the_length() {
        for (index, len, expected) in [
            (4.0_f32, 4, 0),
            (5.0, 4, 1),
            (11.99, 4, 3),
            (12.0, 4, 0),
            (100.0, 7, 2),
            (6.0, 7, 6),
            (7.0, 7, 0),
        ] {
            assert_eq!(array_index(index, len), Some(expected), "{index} % {len}");
        }
    }

    #[test]
    fn array_index_saturates_on_arm64_and_falls_to_zero_on_x86_64_beyond_the_i32_range() {
        assert_eq!(
            array_index(3.0e9, 7),
            per_arch(Some(0), Some((i32::MAX as usize) % 7))
        );
        assert_eq!(
            array_index(f32::MAX, 10),
            per_arch(Some(0), Some((i32::MAX as usize) % 10))
        );
        assert_eq!(
            array_index(2_147_483_520.0, 1000),
            Some(2_147_483_520 % 1000)
        );
    }

    #[test]
    fn array_index_with_a_huge_length_returns_the_truncated_index() {
        assert_eq!(array_index(5.7, usize::MAX), Some(5));
        assert_eq!(array_index(-5.7, usize::MAX), Some(0));
    }

    proptest! {
        #[test]
        fn array_index_is_always_in_range_for_a_non_empty_array(index in proptest::num::f32::ANY, len in 1_usize..1000) {
            let i = array_index(index, len).unwrap();
            prop_assert!(i < len);
        }
    }
}
