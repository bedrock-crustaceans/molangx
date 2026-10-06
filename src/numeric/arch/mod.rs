//! The engine's float behaviour per architecture: `x86_64` and `arm64` hold free functions with the
//! same names and signatures, and `native` is the one of this build. The unit tests compile both.

#[cfg(all(target_arch = "x86", not(target_feature = "sse2")))]
compile_error!("molangx needs SSE2 on 32-bit x86: x87 rounds f32 operations in extended precision");

#[cfg(any(target_arch = "aarch64", test))]
#[cfg_attr(not(target_arch = "aarch64"), allow(dead_code))]
pub(crate) mod arm64;
#[cfg(any(not(target_arch = "aarch64"), test))]
#[cfg_attr(target_arch = "aarch64", allow(dead_code))]
pub(crate) mod x86_64;

#[cfg(target_arch = "aarch64")]
pub(crate) use arm64 as native;
#[cfg(not(target_arch = "aarch64"))]
pub(crate) use x86_64 as native;

/// A float behaviour; see the [module documentation](super) for what differs.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum Arch {
    /// Every operation rounded separately in formula order, every comparison with a NaN false,
    /// `min`/`max` returning the second operand when either is NaN.
    X86_64,
    /// Multiply-adds rounded once, `min`/`max` ignoring a NaN operand, and `<` and `<=` true with a
    /// NaN operand.
    Arm64,
}

/// The float behaviour of this build: [`Arch::Arm64`] on `aarch64`, [`Arch::X86_64`] on every other
/// target.
pub const ARCH: Arch = if cfg!(target_arch = "aarch64") {
    Arch::Arm64
} else {
    Arch::X86_64
};

/// Runs a test body once with `$m` naming each architecture module of the arch `mod.rs` whose
/// `tests` module invokes it.
#[cfg(test)]
macro_rules! both {
    ($m:ident => $body:block) => {{
        {
            use super::x86_64 as $m;
            $body
        }
        {
            use super::arm64 as $m;
            $body
        }
    }};
}

#[cfg(test)]
pub(crate) use both;

#[cfg(test)]
mod tests {
    use super::{arm64, x86_64};
    use crate::numeric::{PostOp, test_support::*};
    use crate::stdlib::math;
    use proptest::prelude::*;

    #[test]
    fn the_fused_forms_follow_their_formulas() {
        both!(m => {
            assert_bits(m::mul_add(-2.0, 4.0, 3.0), -5.0);
            assert_bits(m::mul_sub(-2.0, 4.0, 3.0), 11.0);
            assert_bits(m::neg_mul_add(-2.0, 4.0, 3.0), 5.0);
            assert_bits(m::neg_mul_sub(-2.0, 4.0, 3.0), -11.0);
            assert_bits(m::neg_mul_add(2.0, 3.0, -6.0), 0.0);
        });
    }

    #[test]
    fn ordinary_values_compare_and_round_alike() {
        let samples = [-2.0_f32, -0.0, 0.0, 1.0, 1.000_000_1, INF, -INF];
        both!(m => {
            for a in samples {
                for b in samples {
                    assert_eq!(m::lt(a, b), a < b, "{a} < {b}");
                    assert_eq!(m::le(a, b), a <= b, "{a} <= {b}");
                    if a != b || a.to_bits() == b.to_bits() {
                        assert_bits(m::max(a, b), if a > b { a } else { b });
                        assert_bits(m::min(a, b), if a < b { a } else { b });
                    }
                    if !(a + b).is_nan() {
                        assert_bits(m::add(a, b), a + b);
                    }
                }
            }
            assert_eq!(m::to_int(-2.9), -2);
            assert_eq!(m::to_int(2_147_483_520.0), 2_147_483_520);
        });
    }

    #[test]
    fn the_architectures_choose_different_nans() {
        let (quiet, negative_quiet) = (f32::from_bits(0x7fc0_0001), f32::from_bits(0xffc0_0002));
        let mut got = Vec::new();
        both!(m => {
            got.push([
                m::add(INF, -INF),
                m::mul(0.0, INF),
                m::mul_add(INF, 0.0, quiet),
                m::mul_add(quiet, 1.0, negative_quiet),
                m::mul_sub(quiet, 1.0, 1.0),
                m::neg_mul_add(1.0, 1.0, quiet),
                m::neg_mul_sub(quiet, 1.0, negative_quiet),
            ].map(f32::to_bits));
        });
        let x86_64 = [
            0xffc0_0000,
            0xffc0_0000,
            0xffc0_0000,
            0x7fc0_0001,
            0x7fc0_0001,
            0x7fc0_0001,
            0x7fc0_0001,
        ];
        let arm64 = [
            0x7fc0_0000,
            0x7fc0_0000,
            0x7fc0_0000,
            0xffc0_0002,
            0xffc0_0001,
            0xffc0_0001,
            0x7fc0_0002,
        ];
        assert_eq!(got, [x86_64, arm64]);
    }

    #[test]
    fn the_post_op_forms_agree_on_exact_operands() {
        both!(m => {
            assert_bits(m::apply(PostOp::IDENTITY, -0.0), -0.0);
            assert_bits(m::apply(AFFINE, 3.0), 7.0);
            assert_bits(m::mul_post(3.0, 2.0, AFFINE), 13.0);
            assert_bits(m::div_post(6.0, 3.0, AFFINE), 5.0);
            assert_eq!(m::div_guard(true, -2.0), Some(-2.0));
            assert_eq!(m::div_guard(false, -2.0), Some(2.0));
            assert_eq!(m::div_guard(true, 1.0e-8), None);
            assert_eq!(m::fold_scaled(PostOp::new(5.0, 7.0), 3.0, PostOp::new(2.0, 1.0)), PostOp::new(30.0, 22.0));
        });
    }

    #[test]
    fn the_primitives_of_each_architecture() {
        let x = third();
        assert_eq!(x86_64::mul_add(x, 3.0, -1.0), 0.0);
        assert_eq!(arm64::mul_add(x, 3.0, -1.0), 2.980_232_2e-8);
        assert_eq!(x86_64::mul_sub(x, 3.0, 1.0), 0.0);
        assert_eq!(arm64::mul_sub(x, 3.0, 1.0), -2.980_232_2e-8);
        assert_eq!(arm64::neg_mul_add(x, 3.0, -1.0), -2.980_232_2e-8);
        assert_eq!(arm64::neg_mul_sub(x, 3.0, 1.0), 2.980_232_2e-8);
        both!(m => {
            assert_eq!(m::neg_mul_add(2.0, 3.0, -6.0).to_bits(), 0.0_f32.to_bits());
            assert_eq!(
                m::neg_mul_add(-2.0, 0.0, -0.0).to_bits(),
                0.0_f32.to_bits()
            );
        });
        // The arm64 in-out quad's second half is a negated multiply-add.
        assert_eq!(
            math::ease_in_out_quad(-0.0, 0.0, 16.5, ID).to_bits(),
            per_arch(0x8000_0000, 0)
        );
        both!(m => {
            assert_eq!(m::mul_add(2.0, 3.0, 1.0), 7.0);
            assert_eq!(m::mul_sub(2.0, 3.0, 1.0), -5.0);
            assert_eq!(m::neg_mul_add(2.0, 3.0, 1.0), -7.0);
            assert_eq!(m::neg_mul_sub(2.0, 3.0, 1.0), 5.0);
            assert_eq!(m::max(1.0, 2.0), 2.0);
            assert_eq!(m::min(1.0, 2.0), 1.0);
        });
    }

    #[test]
    fn mul_add_is_one_rounding_on_arm64_and_two_on_x86_64() {
        // (1 + 2^-12)² = 1 + 2^-11 + 2^-24; the product rounds to 1 + 2^-11, so subtracting
        // 1 + 2^-11 leaves 0 when rounded twice and the 2^-24 residue when fused.
        let a = 1.0 + 2.0_f32.powi(-12);
        let c = -(1.0 + 2.0_f32.powi(-11));
        assert_bits(x86_64::mul_add(a, a, c), 0.0);
        assert_bits(arm64::mul_add(a, a, c), 2.0_f32.powi(-24));
        let c = 1.0 + 2.0_f32.powi(-11);
        assert_bits(x86_64::mul_sub(a, a, c), 0.0);
        assert_bits(arm64::mul_sub(a, a, c), -(2.0_f32.powi(-24)));
        assert_bits(x86_64::neg_mul_add(a, a, c), -(c + c));
        assert_bits(x86_64::neg_mul_add(a, a, -c), 0.0);
        assert_bits(arm64::neg_mul_add(a, a, -c), -(2.0_f32.powi(-24)));
        assert_bits(x86_64::neg_mul_sub(a, a, c), 0.0);
        assert_bits(arm64::neg_mul_sub(a, a, c), 2.0_f32.powi(-24));
    }

    #[test]
    fn the_four_fused_forms_on_exact_operands_agree_across_architectures() {
        for (a, b, c) in [
            (2.0, 3.0, 1.0),
            (-2.0, 3.0, 1.0),
            (0.5, 0.5, 0.25),
            (1024.0, 2.0, -2048.0),
            (0.0, 5.0, 7.0),
        ] {
            assert_bits(x86_64::mul_add(a, b, c), arm64::mul_add(a, b, c));
            assert_bits(x86_64::mul_sub(a, b, c), arm64::mul_sub(a, b, c));
            assert_bits(x86_64::neg_mul_add(a, b, c), arm64::neg_mul_add(a, b, c));
            assert_bits(x86_64::neg_mul_sub(a, b, c), arm64::neg_mul_sub(a, b, c));
        }
    }

    #[test]
    fn neg_mul_sub_is_mul_add_with_a_negated_addend() {
        both!(m => {
            for (a, b, c) in [
                (1.5_f32, 2.5, 0.1),
                (-1.5, 2.5, 0.1),
                (0.0, 1.0, 0.0),
                (1.0, 0.0, -0.0),
                (INF, 2.0, 1.0),
            ] {
                assert_bits(m::neg_mul_sub(a, b, c), m::mul_add(a, b, -c));
            }
        });
    }

    #[test]
    fn neg_mul_add_is_not_the_negation_of_mul_add_at_an_exact_zero() {
        both!(m => {
            assert_bits(m::neg_mul_add(-2.0, 3.0, 6.0), 0.0);
            assert_bits(-m::mul_add(-2.0, 3.0, 6.0), -0.0);
            assert_bits(m::neg_mul_add(2.0, 3.0, -6.0), 0.0);
        });
    }

    #[test]
    fn the_fused_forms_propagate_nan_and_infinity() {
        both!(m => {
            for (a, b, c) in [
                (NAN, 1.0, 1.0),
                (1.0, NAN, 1.0),
                (1.0, 1.0, NAN),
                (INF, 0.0, 1.0),
            ] {
                assert!(m::mul_add(a, b, c).is_nan(), "mul_add({a}, {b}, {c})");
                assert!(m::mul_sub(a, b, c).is_nan(), "mul_sub({a}, {b}, {c})");
                assert!(
                    m::neg_mul_add(a, b, c).is_nan(),
                    "neg_mul_add({a}, {b}, {c})"
                );
                assert!(
                    m::neg_mul_sub(a, b, c).is_nan(),
                    "neg_mul_sub({a}, {b}, {c})"
                );
            }
            assert!(m::mul_add(INF, 1.0, -INF).is_nan());
            assert_eq!(m::mul_sub(INF, 1.0, -INF), -INF);
            assert!(m::neg_mul_add(INF, 1.0, -INF).is_nan());
            assert_eq!(m::neg_mul_sub(INF, 1.0, -INF), INF);
            assert_eq!(m::mul_add(INF, 2.0, 1.0), INF);
            assert_eq!(m::mul_add(-INF, 2.0, 1.0), -INF);
            assert_eq!(m::mul_sub(INF, 2.0, 1.0), -INF);
            assert_eq!(m::neg_mul_add(INF, 2.0, 1.0), -INF);
            assert_eq!(m::neg_mul_sub(INF, 2.0, 1.0), INF);
            assert_eq!(m::mul_add(2.0, 3.0, INF), INF);
            assert_eq!(m::mul_sub(2.0, 3.0, INF), INF);
        });
    }

    #[test]
    fn the_fused_forms_keep_the_sign_of_a_zero_result() {
        both!(m => {
            assert_bits(m::mul_add(0.0, 1.0, 0.0), 0.0);
            assert_bits(m::mul_add(-0.0, 1.0, -0.0), -0.0);
            assert_bits(m::mul_add(-0.0, 1.0, 0.0), 0.0);
            assert_bits(m::mul_sub(0.0, 1.0, -0.0), -0.0);
            assert_bits(m::mul_sub(0.0, 1.0, 0.0), 0.0);
            assert_bits(m::neg_mul_add(0.0, 1.0, 0.0), -0.0);
            assert_bits(m::neg_mul_add(-0.0, 1.0, 0.0), 0.0);
            assert_bits(m::neg_mul_sub(0.0, 1.0, 0.0), 0.0);
            assert_bits(m::neg_mul_sub(0.0, 1.0, -0.0), 0.0);
            assert_bits(m::neg_mul_sub(-0.0, 1.0, 0.0), -0.0);
        });
    }

    #[test]
    fn max_and_min_on_ordinary_values_agree() {
        both!(m => {
            for (a, b) in [
                (1.0_f32, 2.0),
                (2.0, 1.0),
                (-1.0, -2.0),
                (-3.0, 3.0),
                (INF, 1.0),
                (-INF, 1.0),
                (INF, -INF),
                (5.0, 5.0),
            ] {
                assert_bits(m::max(a, b), if a > b { a } else { b });
                assert_bits(m::min(a, b), if a < b { a } else { b });
            }
            assert_eq!(m::max(1.0, 2.0), 2.0);
            assert_eq!(m::min(1.0, 2.0), 1.0);
            assert_eq!(m::max(INF, 1.0), INF);
            assert_eq!(m::min(-INF, 1.0), -INF);
        });
    }

    #[test]
    fn max_and_min_with_nan_differ_between_the_architectures() {
        assert_bits(x86_64::max(NAN, 4.0), 4.0);
        assert!(x86_64::max(4.0, NAN).is_nan());
        assert!(x86_64::max(NAN, NAN).is_nan());
        assert_bits(x86_64::min(NAN, 4.0), 4.0);
        assert!(x86_64::min(4.0, NAN).is_nan());
        assert!(x86_64::min(NAN, NAN).is_nan());
        assert_bits(arm64::max(NAN, 4.0), 4.0);
        assert_bits(arm64::max(4.0, NAN), 4.0);
        assert_eq!(
            arm64::max(f32::from_bits(0x7fc0_0001), f32::from_bits(0xffc0_0002)).to_bits(),
            0xffc0_0002
        );
        assert_bits(arm64::min(NAN, 4.0), 4.0);
        assert_bits(arm64::min(4.0, NAN), 4.0);
        assert_eq!(
            arm64::min(f32::from_bits(0x7fc0_0001), f32::from_bits(0xff80_0002)).to_bits(),
            0xff80_0002
        );
        assert_bits(arm64::max(-INF, NAN), -INF);
        assert!(x86_64::max(-INF, NAN).is_nan());
    }

    #[test]
    fn max_and_min_of_signed_zeros() {
        // x86-64 returns the second of two equal zeros.
        assert_bits(x86_64::max(0.0, -0.0), -0.0);
        assert_bits(x86_64::max(-0.0, 0.0), 0.0);
        assert_bits(x86_64::min(0.0, -0.0), -0.0);
        assert_bits(x86_64::min(-0.0, 0.0), 0.0);
        assert_bits(arm64::max(0.0, -0.0), 0.0);
        assert_bits(arm64::max(-0.0, 0.0), 0.0);
        assert_bits(arm64::min(0.0, -0.0), -0.0);
        assert_bits(arm64::min(-0.0, 0.0), -0.0);
    }

    #[test]
    fn arm64_max_and_min_ignore_a_nan_against_a_number() {
        assert_bits(arm64::max(1.0, 2.0), 2.0);
        assert_bits(arm64::max(2.0, 1.0), 2.0);
        assert_bits(arm64::max(NAN, 1.0), 1.0);
        assert_bits(arm64::max(1.0, NAN), 1.0);
        assert_eq!(
            arm64::max(f32::from_bits(0x7fc0_0001), f32::from_bits(0xffc0_0002)).to_bits(),
            0xffc0_0002
        );
        assert_bits(arm64::max(-0.0, 0.0), 0.0);
        assert_bits(arm64::max(0.0, -0.0), 0.0);
        assert_bits(arm64::max(-0.0, -0.0), -0.0);
        assert_bits(arm64::max(0.0, 0.0), 0.0);
        assert_bits(arm64::min(1.0, 2.0), 1.0);
        assert_bits(arm64::min(2.0, 1.0), 1.0);
        assert_bits(arm64::min(NAN, 1.0), 1.0);
        assert_bits(arm64::min(1.0, NAN), 1.0);
        assert_eq!(
            arm64::min(f32::from_bits(0x7fc0_0001), f32::from_bits(0xff80_0002)).to_bits(),
            0xff80_0002
        );
        assert_bits(arm64::min(-0.0, 0.0), -0.0);
        assert_bits(arm64::min(0.0, -0.0), -0.0);
        assert_bits(arm64::min(-0.0, -0.0), -0.0);
        assert_bits(arm64::min(0.0, 0.0), 0.0);
    }

    proptest! {
        #[test]
        fn max_and_min_agree_between_the_architectures_without_nan_or_zero_ties(a in proptest::num::f32::NORMAL | proptest::num::f32::INFINITE, b in proptest::num::f32::NORMAL | proptest::num::f32::INFINITE) {
            prop_assert_eq!(x86_64::max(a, b).to_bits(), arm64::max(a, b).to_bits());
            prop_assert_eq!(x86_64::min(a, b).to_bits(), arm64::min(a, b).to_bits());
        }
    }

    #[test]
    fn float_to_int_conversion() {
        both!(m => {
            assert_eq!(m::to_int(2.9), 2);
            assert_eq!(m::to_int(-2.9), -2);
            assert_eq!(m::to_int(2_147_483_520.0), 2_147_483_520);
            assert_eq!(m::to_int(-2_147_483_648.0), i32::MIN);
        });
        assert_eq!(arm64::to_int(NAN), 0);
        assert_eq!(arm64::to_int(3.0e9), i32::MAX);
        assert_eq!(arm64::to_int(-3.0e9), i32::MIN);
        assert_eq!(x86_64::to_int(NAN), i32::MIN);
        assert_eq!(x86_64::to_int(3.0e9), i32::MIN);
        assert_eq!(x86_64::to_int(-3.0e9), i32::MIN);
    }

    #[test]
    fn to_int_truncates_toward_zero_and_maps_zeros_to_zero() {
        both!(m => {
            for (v, expected) in [
                (0.0_f32, 0),
                (-0.0, 0),
                (0.9, 0),
                (-0.9, 0),
                (1.0, 1),
                (-1.0, -1),
                (1.999_999_9, 1),
                (-1.999_999_9, -1),
                (16_777_216.0, 16_777_216),
                (-16_777_216.0, -16_777_216),
            ] {
                assert_eq!(m::to_int(v), expected, "to_int({v})");
            }
        });
    }

    #[test]
    fn to_int_at_the_edges_of_the_i32_range() {
        let just_below_2_31 = f32::from_bits(0x4eff_ffff); // 2147483520
        let two_pow_31 = 2_147_483_648.0_f32;
        let just_below_minus_2_31 = f32::from_bits(0xcf00_0001); // −2147483904
        assert_eq!(just_below_2_31, 2_147_483_520.0);
        both!(m => {
            assert_eq!(m::to_int(just_below_2_31), 2_147_483_520);
            assert_eq!(m::to_int(-two_pow_31), i32::MIN);
            assert_eq!(m::to_int(just_below_minus_2_31), i32::MIN);
        });
        assert_eq!(x86_64::to_int(two_pow_31), i32::MIN);
        assert_eq!(arm64::to_int(two_pow_31), i32::MAX);
        assert_eq!(x86_64::to_int(INF), i32::MIN);
        assert_eq!(arm64::to_int(INF), i32::MAX);
        assert_eq!(x86_64::to_int(-INF), i32::MIN);
        assert_eq!(arm64::to_int(-INF), i32::MIN);
        assert_eq!(x86_64::to_int(f32::MAX), i32::MIN);
        assert_eq!(arm64::to_int(f32::MAX), i32::MAX);
    }

    proptest! {
        #[test]
        fn to_int_agrees_between_the_architectures_inside_the_i32_range(v in -2_147_483_000.0_f32..2_147_483_000.0) {
            prop_assert_eq!(x86_64::to_int(v), arm64::to_int(v));
            prop_assert_eq!(x86_64::to_int(v), v as i32);
        }
    }
}
