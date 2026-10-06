//! `abs`, `copy_sign`, `exp`, `ln`, `max`, `min`, `mod`, `pow`, `sign` and `sqrt`.

use super::{arch, transcendental};
use crate::numeric::{PostOp, arith, lt};

/// `math.abs(x)`: `x` with its sign bit cleared.
#[inline]
pub fn abs(x: f32, post: PostOp) -> f32 {
    post.apply(x.abs())
}

/// `math.copy_sign(a, b)`: the magnitude of `a` with the sign bit of `b`.
#[inline]
pub fn copy_sign(a: f32, b: f32, post: PostOp) -> f32 {
    post.apply(a.copysign(b))
}

/// `math.exp(x)`: e^x.
#[inline]
pub fn exp(x: f32, post: PostOp) -> f32 {
    post.apply(transcendental::exp(x))
}

/// `math.ln(x)`: the natural logarithm (`ln(0)` = −∞).
///
/// A negative argument (−∞ included) gives the NaN `0x7fc00000` on `X86_64` and `0xffc00000` on
/// `Arm64`; a NaN argument comes back quietened.
#[inline]
pub fn ln(x: f32, post: PostOp) -> f32 {
    post.apply(arch::ln(x))
}

/// `math.max(a, b)`: see [`arith::max`].
#[inline]
pub fn max(a: f32, b: f32, post: PostOp) -> f32 {
    post.apply(arith::max(a, b))
}

/// `math.min(a, b)`: see [`arith::min`].
#[inline]
pub fn min(a: f32, b: f32, post: PostOp) -> f32 {
    post.apply(arith::min(a, b))
}

/// `math.mod(a, b)` with a non-literal divisor: the truncated remainder, or the node's offset when
/// `b == 0`.
///
/// The post-op is applied even when it is `(1, 0)`, so `math.mod(-3, 3)` is `+0`.
#[inline]
pub fn mod_runtime(a: f32, b: f32, post: PostOp) -> f32 {
    if b == 0.0 {
        post.offset
    } else {
        mod_const(a, b, post)
    }
}

/// `math.mod(a, b)` with a literal divisor: the truncated remainder with no zero test, so a literal
/// 0 gives the NaN `0xffc00000`. Always applies the post-op.
#[inline]
pub fn mod_const(a: f32, b: f32, post: PostOp) -> f32 {
    arith::mul_add(transcendental::rem(a, b), post.scale, post.offset)
}

/// `math.pow(a, b)`: `a` to the power `b`. A NaN `a` with an odd integer `b` gives that NaN with
/// its sign bit cleared and its quiet bit set; an invalid operation gives the NaN `0xffc00000`.
#[inline]
pub fn pow(a: f32, b: f32, post: PostOp) -> f32 {
    post.apply(arch::pow(a, b))
}

/// `math.sign(x)`: `−(S + O)` when `x < 0`, else `S + O`; `sign(0)` = `sign(−0)` = 1.
///
/// The post-op is not `raw·S + O`: `v.x = -1; math.sign(v.x) + 1` is −2. `sign(NaN)` is 1 on
/// `X86_64` and −1 on `Arm64`.
#[inline]
pub fn sign(x: f32, post: PostOp) -> f32 {
    let k = post.truthy_value();
    if lt(x, 0.0) { -k } else { k }
}

/// `math.sqrt(x)`: IEEE square root; a negative argument gives the NaN `0xffc00000` on `X86_64` and
/// `0x7fc00000` on `Arm64`.
#[inline]
pub fn sqrt(x: f32, post: PostOp) -> f32 {
    post.apply(arch::sqrt(x))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::numeric::{ARCH, Arch, PostOp, test_support::*};
    use crate::stdlib::math::{clamp, hermite_blend};

    #[test]
    fn ln_of_a_negative_number_is_the_nan_of_the_architecture() {
        let nan = per_arch(0x7fc0_0000, 0xffc0_0000);
        for x in [
            -1.0_f32,
            -0.5,
            -1.0e-30,
            -1.0e-45,
            -f32::MAX,
            f32::NEG_INFINITY,
        ] {
            assert_eq!(ln(x, ID).to_bits(), nan, "{x}");
            assert_eq!(ln(x, AFFINE).to_bits(), nan, "{x} with a post-op");
        }
        assert_eq!(ln(f32::from_bits(0xffc0_0000), ID).to_bits(), 0xffc0_0000);
        assert_eq!(ln(0.0, ID), f32::NEG_INFINITY);
        assert_eq!(ln(-0.0, ID), f32::NEG_INFINITY);
        assert_eq!(ln(1.0, ID).to_bits(), 0);
    }

    #[test]
    fn abs_clears_the_sign_of_every_value() {
        for (x, expected) in [
            (-2.0_f32, 2.0),
            (2.0, 2.0),
            (-0.0, 0.0),
            (0.0, 0.0),
            (-INF, INF),
            (INF, INF),
            (-1.0e-45, 1.0e-45),
            (f32::MIN, f32::MAX),
        ] {
            assert_bits(abs(x, ID), expected);
        }
        assert!(abs(NAN, ID).is_nan());
        assert!(abs(-NAN, ID).is_nan());
        assert!(abs(-NAN, ID).is_sign_positive());
    }

    #[test]
    fn abs_applies_the_post_op_after_the_absolute_value() {
        assert_eq!(abs(-3.0, AFFINE), 7.0);
        assert_eq!(abs(3.0, AFFINE), 7.0);
        assert_eq!(abs(-3.0, PostOp::new(-1.0, 0.0)), -3.0);
        assert_bits(abs(-0.0, PostOp::new(-1.0, 0.0)), 0.0);
        assert_eq!(abs(-INF, AFFINE), INF);
    }

    #[test]
    fn copy_sign_takes_the_sign_bit_of_the_second_argument() {
        assert_bits(copy_sign(2.0, 3.0, ID), 2.0);
        assert_bits(copy_sign(2.0, -3.0, ID), -2.0);
        assert_bits(copy_sign(-2.0, 3.0, ID), 2.0);
        assert_bits(copy_sign(-2.0, -3.0, ID), -2.0);
        assert_bits(copy_sign(2.0, 0.0, ID), 2.0);
        assert_bits(copy_sign(2.0, -0.0, ID), -2.0);
        assert_bits(copy_sign(0.0, -1.0, ID), -0.0);
        assert_bits(copy_sign(-0.0, 1.0, ID), 0.0);
        assert_bits(copy_sign(INF, -1.0, ID), -INF);
        assert_bits(copy_sign(-INF, 1.0, ID), INF);
        assert_bits(copy_sign(2.0, -INF, ID), -2.0);
    }

    #[test]
    fn copy_sign_with_nan_operands() {
        let negative = copy_sign(NAN, -1.0, ID);
        assert!(negative.is_nan() && negative.is_sign_negative());
        let positive = copy_sign(NAN, 1.0, ID);
        assert!(positive.is_nan() && positive.is_sign_positive());
        assert_bits(copy_sign(2.0, -NAN, ID), -2.0);
        assert_bits(copy_sign(2.0, NAN, ID), 2.0);
    }

    #[test]
    fn copy_sign_applies_the_post_op() {
        assert_eq!(copy_sign(3.0, -1.0, AFFINE), -5.0);
        assert_eq!(copy_sign(-3.0, 1.0, AFFINE), 7.0);
        assert_bits(copy_sign(0.0, -1.0, PostOp::new(2.0, 0.0)), 0.0);
    }

    #[test]
    fn exp_and_ln_on_key_values() {
        assert_bits(exp(0.0, ID), 1.0);
        assert_bits(exp(-0.0, ID), 1.0);
        close(exp(1.0, ID), std::f32::consts::E, 1e-6);
        close(exp(-1.0, ID), 1.0 / std::f32::consts::E, 1e-7);
        assert_eq!(exp(INF, ID), INF);
        assert_bits(exp(-INF, ID), 0.0);
        assert_eq!(exp(100.0, ID), INF);
        assert_bits(exp(-200.0, ID), 0.0);
        assert!(exp(NAN, ID).is_nan());
        assert_bits(ln(1.0, ID), 0.0);
        close(ln(std::f32::consts::E, ID), 1.0, 1e-6);
        close(ln(10.0, ID), std::f32::consts::LN_10, 1e-6);
        assert_eq!(ln(0.0, ID), -INF);
        assert_eq!(ln(-0.0, ID), -INF);
        assert_eq!(ln(INF, ID), INF);
        assert!(ln(-1.0, ID).is_nan());
        assert!(ln(-INF, ID).is_nan());
        assert!(ln(NAN, ID).is_nan());
    }

    #[test]
    fn exp_and_ln_apply_the_post_op() {
        assert_eq!(exp(0.0, AFFINE), 3.0);
        assert_eq!(ln(1.0, AFFINE), 1.0);
        assert_eq!(exp(INF, AFFINE), INF);
        assert_eq!(ln(0.0, AFFINE), -INF);
        assert_eq!(exp(-INF, PostOp::new(5.0, 7.0)), 7.0);
    }

    #[test]
    fn pow_on_key_values() {
        assert_eq!(pow(2.0, 10.0, ID), 1024.0);
        assert_eq!(pow(2.0, -1.0, ID), 0.5);
        assert_eq!(pow(-2.0, 3.0, ID), -8.0);
        assert_eq!(pow(-2.0, 2.0, ID), 4.0);
        assert_eq!(pow(9.0, 0.5, ID), 3.0);
        assert_bits(pow(7.0, 0.0, ID), 1.0);
        assert_bits(pow(NAN, 0.0, ID), 1.0);
        assert_bits(pow(1.0, NAN, ID), 1.0);
        assert_bits(pow(0.0, 0.0, ID), 1.0);
        assert!(pow(-8.0, 1.0 / 3.0, ID).is_nan());
        assert!(pow(NAN, 1.0, ID).is_nan());
        assert!(pow(2.0, NAN, ID).is_nan());
        assert_eq!(pow(0.0, -1.0, ID), INF);
        assert_eq!(pow(-0.0, -1.0, ID), -INF);
        assert_eq!(pow(-0.0, -2.0, ID), INF);
        assert_bits(pow(0.0, 3.0, ID), 0.0);
        assert_bits(pow(-0.0, 3.0, ID), -0.0);
        assert_eq!(pow(INF, 2.0, ID), INF);
        assert_bits(pow(INF, -2.0, ID), 0.0);
        assert_eq!(pow(2.0, INF, ID), INF);
        assert_bits(pow(0.5, INF, ID), 0.0);
        assert_eq!(pow(10.0, 100.0, ID), INF);
        assert_bits(pow(2.0, 10.0, ID), 1024.0);
    }

    #[test]
    fn pow_of_a_nan_to_an_odd_integer_clears_the_sign_bit() {
        let negative_nan = f32::from_bits(0xffc0_0000);
        for b in [1.0, -1.0, 3.0, -5.0] {
            assert_eq!(
                pow(negative_nan, b, ID).to_bits(),
                0x7fc0_0000,
                "pow(-NaN, {b})"
            );
        }
        for b in [2.0, 0.5, -2.0, 16_777_217.0] {
            assert_eq!(
                pow(negative_nan, b, ID).to_bits(),
                0xffc0_0000,
                "pow(-NaN, {b})"
            );
        }
        assert_bits(pow(negative_nan, 0.0, ID), 1.0);
    }

    #[test]
    fn pow_never_returns_a_signalling_nan() {
        assert_eq!(
            pow(f32::from_bits(0x7f80_0001), 1.0, ID).to_bits(),
            0x7fc0_0001
        );
        assert_eq!(
            pow(f32::from_bits(0xff80_0001), -1.0, ID).to_bits(),
            0x7fc0_0001
        );
        assert_eq!(
            pow(f32::from_bits(0xff80_0001), 2.0, ID).to_bits(),
            0xffc0_0001
        );
        assert_eq!(
            pow(2.0, f32::from_bits(0x7f80_0001), ID).to_bits(),
            0x7fc0_0001
        );
    }

    #[test]
    fn pow_applies_the_post_op() {
        assert_eq!(pow(2.0, 3.0, AFFINE), 17.0);
        assert_eq!(pow(2.0, 3.0, PostOp::new(-1.0, 0.0)), -8.0);
        assert_eq!(pow(0.0, -1.0, AFFINE), INF);
    }

    #[test]
    fn sqrt_on_key_values() {
        assert_eq!(sqrt(16.0, ID), 4.0);
        assert_eq!(sqrt(2.25, ID), 1.5);
        assert_bits(sqrt(0.0, ID), 0.0);
        assert_bits(sqrt(-0.0, ID), -0.0);
        assert_eq!(sqrt(INF, ID), INF);
        assert!(sqrt(-1.0, ID).is_nan());
        assert!(sqrt(-INF, ID).is_nan());
        assert!(sqrt(NAN, ID).is_nan());
        assert_bits(sqrt(2.0, ID), 2.0_f32.sqrt());
        assert_bits(sqrt(1.0e-45, ID), 1.0e-45_f32.sqrt());
        assert_eq!(sqrt(16.0, AFFINE), 9.0);
        assert_bits(sqrt(-0.0, PostOp::new(2.0, 0.0)), 0.0);
    }

    #[test]
    fn max_and_min_on_ordinary_values() {
        assert_eq!(max(0.0, 1.0, ID), 1.0);
        assert_eq!(max(1.0, 0.0, ID), 1.0);
        assert_eq!(min(0.0, 1.0, ID), 0.0);
        assert_eq!(min(1.0, 0.0, ID), 0.0);
        assert_eq!(max(-INF, INF, ID), INF);
        assert_eq!(min(-INF, INF, ID), -INF);
        assert_eq!(max(2.0, 2.0, ID), 2.0);
        assert_eq!(min(2.0, 2.0, ID), 2.0);
        assert_eq!(max(1.0, 2.0, PostOp::new(3.0, 1.0)), 7.0);
        assert_eq!(min(1.0, 2.0, PostOp::new(3.0, 1.0)), 4.0);
    }

    #[test]
    fn max_and_min_with_a_real_post_op_normalise_negative_zero() {
        assert_bits(max(-0.0, -0.0, ID), -0.0);
        assert_bits(max(-0.0, -0.0, PostOp::new(2.0, 0.0)), 0.0);
        assert_bits(min(-0.0, -0.0, PostOp::new(2.0, 0.0)), 0.0);
    }

    #[test]
    fn mod_runtime_and_mod_const_agree_for_a_non_zero_divisor() {
        for (a, b) in [
            (7.5_f32, 2.0),
            (-7.5, 2.0),
            (7.5, -2.0),
            (-7.5, -2.0),
            (0.5, 3.0),
            (3.0, 7.0),
            (INF, 1.0 + 1.0),
            (5.0, INF),
            (5.0, -INF),
        ] {
            for post in [ID, AFFINE, PostOp::new(0.5, -3.0)] {
                assert_bits(mod_runtime(a, b, post), mod_const(a, b, post));
            }
        }
    }

    #[test]
    fn mod_follows_the_sign_of_the_dividend() {
        assert_eq!(mod_runtime(7.5, 2.0, ID), 1.5);
        assert_eq!(mod_runtime(-7.5, 2.0, ID), -1.5);
        assert_eq!(mod_runtime(7.5, -2.0, ID), 1.5);
        assert_eq!(mod_runtime(-7.5, -2.0, ID), -1.5);
        assert_eq!(mod_runtime(5.0, INF, ID), 5.0);
        assert_eq!(mod_runtime(-5.0, -INF, ID), -5.0);
        assert!(mod_runtime(INF, 2.0, ID).is_nan());
        assert!(mod_runtime(-INF, 2.0, ID).is_nan());
        assert!(mod_runtime(NAN, 2.0, ID).is_nan());
        assert!(mod_runtime(2.0, NAN, ID).is_nan());
        assert_eq!(mod_runtime(1.0, 3.0, ID), 1.0);
        assert_eq!(mod_runtime(3.0, 3.0, ID), 0.0);
    }

    #[test]
    fn mod_always_applies_the_post_op_even_for_the_identity() {
        assert_bits(mod_runtime(-0.0, 3.0, ID), 0.0);
        assert_bits(mod_const(-0.0, 3.0, ID), 0.0);
        assert_bits(mod_runtime(-6.0, 3.0, ID), 0.0);
        assert_bits(mod_const(-6.0, 3.0, ID), 0.0);
        assert_bits(mod_const(-6.0, 3.0, PostOp::new(1.0, -0.0)), -0.0);
    }

    #[test]
    fn mod_runtime_returns_the_offset_for_a_zero_divisor() {
        assert_eq!(mod_runtime(5.0, 0.0, ID), 0.0);
        assert_bits(mod_runtime(5.0, 0.0, ID), 0.0);
        assert_bits(mod_runtime(5.0, -0.0, ID), 0.0);
        assert_eq!(mod_runtime(5.0, 0.0, PostOp::new(2.0, 0.125)), 0.125);
        assert_eq!(mod_runtime(NAN, 0.0, PostOp::new(2.0, 0.125)), 0.125);
        assert_eq!(mod_runtime(INF, 0.0, PostOp::new(2.0, 0.125)), 0.125);
        assert_bits(mod_runtime(5.0, 0.0, PostOp::new(2.0, -0.0)), -0.0);
    }

    #[test]
    fn mod_const_by_zero_is_nan() {
        assert!(mod_const(5.0, 0.0, ID).is_nan());
        assert!(mod_const(5.0, -0.0, ID).is_nan());
        assert!(mod_const(0.0, 0.0, ID).is_nan());
        assert!(mod_const(5.0, 0.0, AFFINE).is_nan());
        assert!(mod_const(INF, 0.0, ID).is_nan());
    }

    #[test]
    fn mod_applies_the_post_op_as_one_fused_operation_on_arm64() {
        let post = PostOp::new(0.7, 0.3);
        let r = 5.3_f32 % 1.1;
        assert_bits(
            mod_runtime(5.3, 1.1, post),
            per_arch(r * 0.7 + 0.3, r.mul_add(0.7, 0.3)),
        );
        assert_bits(
            mod_const(5.3, 1.1, post),
            per_arch(r * 0.7 + 0.3, r.mul_add(0.7, 0.3)),
        );
    }

    #[test]
    fn sign_on_ordinary_values_and_zeros() {
        assert_bits(sign(3.0, ID), 1.0);
        assert_bits(sign(-3.0, ID), -1.0);
        assert_bits(sign(1.0e-45, ID), 1.0);
        assert_bits(sign(-1.0e-45, ID), -1.0);
        assert_bits(sign(INF, ID), 1.0);
        assert_bits(sign(-INF, ID), -1.0);
        assert_bits(sign(0.0, ID), 1.0);
        assert_bits(sign(-0.0, ID), 1.0);
    }

    #[test]
    fn sign_of_nan_depends_on_the_architecture() {
        assert_bits(sign(NAN, ID), per_arch(1.0, -1.0));
        assert_bits(sign(NAN, PostOp::new(2.0, 3.0)), per_arch(5.0, -5.0));
    }

    #[test]
    fn sign_returns_the_precomputed_constant_not_raw_times_scale_plus_offset() {
        assert_eq!(sign(4.0, PostOp::new(2.0, 3.0)), 5.0);
        assert_eq!(sign(-4.0, PostOp::new(2.0, 3.0)), -5.0);
        assert_eq!(sign(0.0, PostOp::new(2.0, 3.0)), 5.0);
        assert_bits(sign(4.0, PostOp::new(0.0, 0.0)), 0.0);
        assert_bits(sign(-4.0, PostOp::new(0.0, 0.0)), -0.0);
        assert_bits(sign(1.0, PostOp::new(0.1, 0.2)), 0.1_f32 + 0.2);
        assert_bits(sign(-1.0, PostOp::new(0.1, 0.2)), -(0.1_f32 + 0.2));
    }

    #[test]
    fn sign_post_op_quirk() {
        assert_eq!(sign(-1.0, PostOp::new(1.0, 1.0)), -2.0);
        assert_eq!(sign(-1.0, PostOp::new(-2.0, 1.0)), 1.0);
        assert_eq!(sign(1.0, PostOp::new(1.0, 1.0)), 2.0);
        assert_eq!(sign(1.0, PostOp::new(-2.0, 1.0)), -1.0);
    }

    #[test]
    fn abs_clamp_copy_sign_exp_ln_sqrt_pow_hermite_max_min_on_key_values() {
        assert_eq!(abs(-2.0, ID), 2.0);
        assert_eq!(abs(-0.0, ID).to_bits(), 0.0_f32.to_bits());
        assert_eq!(clamp(3.0, 2.0, 1.0, ID), 1.0);
        assert_eq!(clamp(-1.0, -2.0, -3.0, ID), -3.0);
        assert_eq!(clamp(1.0, 2.0, 3.0, ID), 2.0);
        assert_eq!(
            clamp(2.1, 0.0, 1.1, PostOp::new(2.0, 1.0)),
            arith::mul_add(1.1, 2.0, 1.0)
        );
        assert_eq!(copy_sign(-1.1, 3.1, ID), 1.1);
        assert_eq!(copy_sign(2.0, -0.0, ID), -2.0);
        assert_eq!(copy_sign(0.0, -2.0, ID).to_bits(), (-0.0_f32).to_bits());
        assert_eq!(exp(0.0, ID), 1.0);
        assert_eq!(ln(1.0, ID), 0.0);
        assert_eq!(ln(0.0, ID), f32::NEG_INFINITY);
        assert!(sqrt(-1.0, ID).is_nan());
        assert_eq!(sqrt(16.0, ID), 4.0);
        assert_eq!(pow(2.0, 10.0, ID), 1024.0);
        assert_eq!(hermite_blend(0.5, ID), 0.5);
        assert_eq!(hermite_blend(2.0, ID), -4.0);
        assert_eq!(hermite_blend(0.0, ID), 0.0);
        assert_eq!(hermite_blend(1.0, ID), 1.0);
        assert_eq!(max(0.0, 1.0, ID), 1.0);
        assert_eq!(min(0.0, 1.0, ID), 0.0);
        assert_eq!(max(1.0, 2.0, PostOp::new(3.0, 1.0)), 7.0);
    }

    #[test]
    fn min_max_ignore_nan_on_arm64() {
        if ARCH == Arch::Arm64 {
            assert_eq!(max(NAN, 4.0, ID), 4.0);
            assert_eq!(max(4.0, NAN, ID), 4.0);
            assert_eq!(min(NAN, 4.0, ID), 4.0);
            assert_eq!(min(4.0, NAN, ID), 4.0);
            assert!(max(NAN, NAN, ID).is_nan());
            assert_eq!(max(-0.0, 0.0, ID).to_bits(), 0.0_f32.to_bits());
            assert_eq!(max(0.0, -0.0, ID).to_bits(), 0.0_f32.to_bits());
            assert_eq!(min(0.0, -0.0, ID).to_bits(), (-0.0_f32).to_bits());
            assert_eq!(min(-0.0, 0.0, ID).to_bits(), (-0.0_f32).to_bits());
        }
    }

    #[test]
    fn min_max_return_the_second_operand_on_x86_64() {
        if ARCH == Arch::X86_64 {
            assert_eq!(max(NAN, 4.0, ID), 4.0);
            assert!(max(4.0, NAN, ID).is_nan());
            assert!(max(NAN, NAN, ID).is_nan());
            assert_eq!(min(NAN, 4.0, ID), 4.0);
            assert!(min(4.0, NAN, ID).is_nan());
        }
    }

    #[test]
    fn sign_of_nan_and_of_zero() {
        assert_eq!(sign(NAN, ID), per_arch(1.0, -1.0));
        assert_eq!(sign(0.0, ID), 1.0);
        assert_eq!(sign(-0.0, ID), 1.0);
        assert_eq!(sign(-3.0, ID), -1.0);
        assert_eq!(sign(3.0, ID), 1.0);
    }

    #[test]
    fn mod_zero_rules() {
        assert!(mod_const(1.0, 0.0, ID).is_nan());
        assert!(mod_const(5.0, 0.0, ID).is_nan());
        assert_eq!(mod_runtime(1.0, 0.0, ID), 0.0);
        assert_eq!(mod_runtime(1.0, -0.0, ID), 0.0);
        assert_eq!(mod_runtime(1.0, 0.0, PostOp::new(2.0, 0.125)), 0.125);
        assert_eq!(mod_runtime(-5.1, 3.0, ID), -5.1_f32 % 3.0);
        assert!((mod_runtime(-5.1, 3.0, ID) - -2.1).abs() <= 1e-6);
        assert_eq!(mod_const(7.5, 2.0, PostOp::new(2.0, 1.0)), 4.0);
        assert!(mod_runtime(1.0, NAN, ID).is_nan());
        assert_eq!(-3.0_f32 % 3.0, 0.0);
        assert!((-3.0_f32 % 3.0).is_sign_negative());
        assert_eq!(mod_const(-3.0, 3.0, ID).to_bits(), 0.0_f32.to_bits());
        assert_eq!(mod_runtime(-3.0, 3.0, ID).to_bits(), 0.0_f32.to_bits());
    }
}
