//! Trigonometry in degrees: `sin`, `cos`, `asin`, `acos`, `atan` and `atan2`.

use super::DEG_TO_RAD;
use super::arch::{self, degrees, inverse_trig_argument};
use super::transcendental;
use crate::numeric::PostOp;

/// `math.acos(x)` in degrees: `acos(1.0005)` = 0, `acos(1.0006)` = the NaN `0x7fc00000`,
/// `acos(NaN)` = 180 on `Arm64`.
#[inline]
pub fn acos(x: f32, post: PostOp) -> f32 {
    degrees(transcendental::acos(inverse_trig_argument(x)), post)
}

/// `math.asin(x)` in degrees: `asin(1.0004)` = 90, `asin(1.001)` = the NaN `0x7fc00000`,
/// `asin(NaN)` = −90 on `Arm64`.
#[inline]
pub fn asin(x: f32, post: PostOp) -> f32 {
    degrees(transcendental::asin(inverse_trig_argument(x)), post)
}

/// `math.atan(x)` in degrees.
#[inline]
pub fn atan(x: f32, post: PostOp) -> f32 {
    degrees(transcendental::atan(x), post)
}

/// `math.atan2(y, x)` in degrees; `atan2(0, 0)` = 0. A NaN `y` comes back quietened, else a NaN
/// `x`.
#[inline]
pub fn atan2(y: f32, x: f32, post: PostOp) -> f32 {
    degrees(transcendental::atan2(y, x), post)
}

/// `math.cos(x)`, `x` in degrees; ±∞ gives the NaN `0xffc00000`.
#[inline]
pub fn cos(x: f32, post: PostOp) -> f32 {
    post.apply(arch::cos(arch::mul(x, DEG_TO_RAD)))
}

/// `math.sin(x)`, `x` in degrees; ±∞ gives the NaN `0xffc00000`.
#[inline]
pub fn sin(x: f32, post: PostOp) -> f32 {
    post.apply(arch::sin(arch::mul(x, DEG_TO_RAD)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::numeric::{ARCH, Arch, PostOp, test_support::*};
    use crate::stdlib::math::{DEG_TO_RAD, INVERSE_TRIG_TOLERANCE, RAD_TO_DEG, transcendental};

    #[test]
    fn a_nan_angle_comes_back_quietened() {
        for f in [sin, cos] {
            assert_eq!(f(f32::from_bits(0xff80_0005), ID).to_bits(), 0xffc0_0005);
            assert_eq!(f(f32::from_bits(0x7fc0_0006), ID).to_bits(), 0x7fc0_0006);
        }
    }

    #[test]
    fn inverse_trig_argument_follows_the_tolerance_rule() {
        assert_bits(inverse_trig_argument(0.5), 0.5);
        assert_bits(inverse_trig_argument(-0.5), -0.5);
        assert_bits(inverse_trig_argument(1.0004), 1.0);
        assert_bits(inverse_trig_argument(-1.0004), -1.0);
        assert_bits(inverse_trig_argument(INVERSE_TRIG_TOLERANCE), 1.0);
        assert_bits(inverse_trig_argument(-INVERSE_TRIG_TOLERANCE), -1.0);
        let above = f32::from_bits(INVERSE_TRIG_TOLERANCE.to_bits() + 1);
        assert_bits(inverse_trig_argument(above), above);
        assert_bits(inverse_trig_argument(-above), -above);
        assert_bits(inverse_trig_argument(2.0), 2.0);
        assert_bits(inverse_trig_argument(INF), INF);
        assert_bits(inverse_trig_argument(-INF), -INF);
        assert_bits(inverse_trig_argument(0.0), 0.0);
        assert_bits(inverse_trig_argument(-0.0), -0.0);
        if ARCH == Arch::Arm64 {
            assert_bits(inverse_trig_argument(NAN), -1.0);
        }
        if ARCH == Arch::X86_64 {
            assert!(inverse_trig_argument(NAN).is_nan());
        }
    }

    #[test]
    fn degrees_converts_with_the_post_op_merged_into_the_factor_on_arm64() {
        let rad = 0.7_f32;
        assert_bits(degrees(rad, ID), rad * RAD_TO_DEG);
        assert_bits(degrees(-0.0, ID), -0.0);
        assert!(degrees(NAN, AFFINE).is_nan());
        assert_eq!(degrees(INF, ID), INF);
        let post = PostOp::new(1.7, 0.3);
        assert_bits(
            degrees(rad, post),
            per_arch(
                (rad * RAD_TO_DEG) * post.scale + post.offset,
                rad.mul_add(RAD_TO_DEG * post.scale, post.offset),
            ),
        );
    }

    #[test]
    fn asin_and_acos_at_the_key_arguments() {
        close(asin(0.0, ID), 0.0, 1e-6);
        close(asin(0.5, ID), 30.0, 1e-4);
        close(asin(-0.5, ID), -30.0, 1e-4);
        close(asin(1.0, ID), 90.0, 1e-4);
        close(asin(-1.0, ID), -90.0, 1e-4);
        assert_bits(asin(-0.0, ID), -0.0);
        assert_bits(asin(0.0, ID), 0.0);
        close(acos(0.0, ID), 90.0, 1e-4);
        close(acos(0.5, ID), 60.0, 1e-4);
        close(acos(-0.5, ID), 120.0, 1e-4);
        close(acos(1.0, ID), 0.0, 1e-6);
        assert_bits(acos(1.0, ID), 0.0);
        close(acos(-1.0, ID), 180.0, 1e-4);
        assert_bits(acos(1.0005, ID), 0.0);
    }

    #[test]
    fn asin_and_acos_tolerance_window_edges() {
        let above = f32::from_bits(INVERSE_TRIG_TOLERANCE.to_bits() + 1);
        close(asin(INVERSE_TRIG_TOLERANCE, ID), 90.0, 1e-4);
        close(asin(-INVERSE_TRIG_TOLERANCE, ID), -90.0, 1e-4);
        assert_bits(acos(INVERSE_TRIG_TOLERANCE, ID), 0.0);
        close(acos(-INVERSE_TRIG_TOLERANCE, ID), 180.0, 1e-4);
        assert!(asin(above, ID).is_nan());
        assert!(asin(-above, ID).is_nan());
        assert!(acos(above, ID).is_nan());
        assert!(acos(-above, ID).is_nan());
        for x in [2.0, -2.0, 1.0e10, INF, -INF] {
            assert!(asin(x, ID).is_nan(), "asin({x})");
            assert!(acos(x, ID).is_nan(), "acos({x})");
        }
        assert!(asin(above, AFFINE).is_nan());
        assert!(acos(above, AFFINE).is_nan());
    }

    #[test]
    fn asin_and_acos_of_nan_depend_on_the_architecture() {
        if ARCH == Arch::Arm64 {
            assert_eq!(asin(NAN, ID), -90.0);
            assert_eq!(acos(NAN, ID), 180.0);
        }
        if ARCH == Arch::X86_64 {
            assert!(asin(NAN, ID).is_nan());
            assert!(acos(NAN, ID).is_nan());
        }
        if ARCH == Arch::Arm64 {
            close(asin(NAN, AFFINE), -179.0, 1e-3);
            close(acos(NAN, AFFINE), 361.0, 1e-3);
        }
        if ARCH == Arch::X86_64 {
            assert!(asin(NAN, AFFINE).is_nan());
            assert!(acos(NAN, AFFINE).is_nan());
        }
    }

    #[test]
    fn asin_and_acos_post_op_is_merged_into_the_factor_on_arm64() {
        let post = PostOp::new(1.7, 0.3);
        for (f, name) in [(asin as fn(f32, PostOp) -> f32, "asin"), (acos, "acos")] {
            let x = 0.3_f32;
            let rad = if name == "asin" {
                transcendental::asin(x)
            } else {
                transcendental::acos(x)
            };
            assert_bits(
                f(x, post),
                per_arch(
                    (rad * RAD_TO_DEG) * post.scale + post.offset,
                    rad.mul_add(RAD_TO_DEG * post.scale, post.offset),
                ),
            );
            assert_bits(f(x, ID), rad * RAD_TO_DEG);
        }
    }

    #[test]
    fn a_domain_error_is_the_positive_nan_and_a_nan_operand_keeps_its_sign() {
        let negative_nan = f32::from_bits(0xffc0_0000);
        for x in [2.0, -2.0, 1.0006, INF, -INF] {
            assert_eq!(asin(x, ID).to_bits(), 0x7fc0_0000, "asin({x})");
            assert_eq!(acos(x, ID).to_bits(), 0x7fc0_0000, "acos({x})");
        }
        assert_eq!(atan2(negative_nan, NAN, ID).to_bits(), 0xffc0_0000);
        assert_eq!(atan2(NAN, negative_nan, ID).to_bits(), 0x7fc0_0000);
        assert_eq!(atan2(1.0, negative_nan, ID).to_bits(), 0xffc0_0000);
        if ARCH == Arch::X86_64 {
            assert_eq!(asin(negative_nan, ID).to_bits(), 0xffc0_0000);
        }
    }

    #[test]
    fn atan_in_degrees() {
        assert_bits(atan(0.0, ID), 0.0);
        assert_bits(atan(-0.0, ID), -0.0);
        close(atan(1.0, ID), 45.0, 1e-4);
        close(atan(-1.0, ID), -45.0, 1e-4);
        close(atan(INF, ID), 90.0, 1e-4);
        close(atan(-INF, ID), -90.0, 1e-4);
        assert!(atan(NAN, ID).is_nan());
        close(atan(1.0, AFFINE), 91.0, 1e-3);
        close(atan(INF, PostOp::new(-1.0, 10.0)), -80.0, 1e-3);
    }

    #[test]
    fn atan2_covers_every_quadrant_and_axis() {
        close(atan2(1.0, 1.0, ID), 45.0, 1e-4);
        close(atan2(1.0, -1.0, ID), 135.0, 1e-4);
        close(atan2(-1.0, -1.0, ID), -135.0, 1e-4);
        close(atan2(-1.0, 1.0, ID), -45.0, 1e-4);
        close(atan2(1.0, 0.0, ID), 90.0, 1e-4);
        close(atan2(-1.0, 0.0, ID), -90.0, 1e-4);
        close(atan2(0.0, -1.0, ID), 180.0, 1e-4);
        close(atan2(-0.0, -1.0, ID), -180.0, 1e-4);
        assert_bits(atan2(0.0, 1.0, ID), 0.0);
        assert_bits(atan2(-0.0, 1.0, ID), -0.0);
        assert_bits(atan2(0.0, 0.0, ID), 0.0);
        assert_bits(atan2(-0.0, 0.0, ID), -0.0);
        close(atan2(0.0, -0.0, ID), 180.0, 1e-4);
        close(atan2(INF, INF, ID), 45.0, 1e-4);
        close(atan2(INF, -INF, ID), 135.0, 1e-4);
        close(atan2(1.0, INF, ID), 0.0, 1e-6);
        assert!(atan2(NAN, 1.0, ID).is_nan());
        assert!(atan2(1.0, NAN, ID).is_nan());
        close(atan2(1.0, 1.0, AFFINE), 91.0, 1e-3);
    }

    #[test]
    fn atan_and_atan2_post_op_is_merged_into_the_factor_on_arm64() {
        let post = PostOp::new(0.9, -0.4);
        let (y, x) = (0.8_f32, 1.9_f32);
        let (angle, rad) = (transcendental::atan2(y, x), transcendental::atan(x));
        assert_bits(
            atan2(y, x, post),
            per_arch(
                (angle * RAD_TO_DEG) * post.scale + post.offset,
                angle.mul_add(RAD_TO_DEG * post.scale, post.offset),
            ),
        );
        assert_bits(
            atan(x, post),
            per_arch(
                (rad * RAD_TO_DEG) * post.scale + post.offset,
                rad.mul_add(RAD_TO_DEG * post.scale, post.offset),
            ),
        );
    }

    #[test]
    fn sin_and_cos_take_degrees() {
        for (deg, s, c) in [
            (0.0_f32, 0.0, 1.0),
            (30.0, 0.5, 0.866_025_4),
            (90.0, 1.0, 0.0),
            (180.0, 0.0, -1.0),
            (270.0, -1.0, 0.0),
            (360.0, 0.0, 1.0),
            (-90.0, -1.0, 0.0),
            (
                45.0,
                std::f32::consts::FRAC_1_SQRT_2,
                std::f32::consts::FRAC_1_SQRT_2,
            ),
        ] {
            close(sin(deg, ID), s, 1e-5);
            close(cos(deg, ID), c, 1e-5);
        }
    }

    #[test]
    fn sin_and_cos_zeros_and_non_finite_arguments() {
        assert_bits(sin(0.0, ID), 0.0);
        assert_bits(sin(-0.0, ID), -0.0);
        assert_bits(cos(0.0, ID), 1.0);
        assert_bits(cos(-0.0, ID), 1.0);
        for f in [sin, cos] {
            assert!(f(NAN, ID).is_nan());
            assert!(f(INF, ID).is_nan());
            assert!(f(-INF, ID).is_nan());
        }
    }

    #[test]
    fn sin_and_cos_take_the_sine_of_the_f32_product_with_the_conversion_factor() {
        for deg in [1.0_f32, 17.5, 123.456, -300.0, 720.0, 1.0e6] {
            assert_bits(sin(deg, ID), transcendental::sin(deg * DEG_TO_RAD, NAN));
            assert_bits(cos(deg, ID), transcendental::cos(deg * DEG_TO_RAD, NAN));
        }
    }

    #[test]
    fn sin_and_cos_apply_the_post_op() {
        close(sin(90.0, AFFINE), 3.0, 1e-5);
        close(cos(0.0, AFFINE), 3.0, 1e-6);
        close(cos(0.0, PostOp::new(-2.0, 1.1)), -0.9, 1e-6);
        assert_bits(cos(0.0, PostOp::new(0.0, 4.0)), 4.0);
        let post = PostOp::new(3.0, -1.0);
        let raw = transcendental::sin(33.0_f32 * DEG_TO_RAD, NAN);
        assert_bits(
            sin(33.0, post),
            per_arch(raw * 3.0 - 1.0, raw.mul_add(3.0, -1.0)),
        );
    }

    #[test]
    fn inverse_trigonometry_in_degrees_with_the_tolerance_window() {
        close(acos(1.0005, ID), 0.0, 5e-4);
        close(acos(-1.0005, ID), 180.0, 5e-4);
        close(acos(-1.0001, ID), 180.0, 5e-4);
        assert!(acos(1.0006, ID).is_nan());
        assert!(acos(-1.0006, ID).is_nan());
        close(asin(1.0005, ID), 90.0, 5e-4);
        assert_eq!(asin(1.0004, ID), 90.0);
        assert!(asin(1.001, ID).is_nan());
        close(asin(-1.0, ID), -90.0, 5e-4);
        assert!(asin(1.0006, ID).is_nan());
        close(atan(1.0, ID), 45.0, 5e-4);
        close(atan2(-7.0, -7.0, ID), -135.0, 5e-4);
        assert_eq!(atan2(0.0, 0.0, ID), 0.0);
        close(asin(1.0, PostOp::new(2.0, 1.0)), 181.0, 5e-4);
        close(atan2(1.0, 1.0, PostOp::new(-2.0, 0.5)), -89.5, 5e-4);
        if ARCH == Arch::Arm64 {
            assert_eq!(acos(NAN, ID), 180.0);
            assert_eq!(asin(NAN, ID), -90.0);
        }
        if ARCH == Arch::X86_64 {
            assert!(acos(NAN, ID).is_nan());
            assert!(asin(NAN, PostOp::new(3.0, 0.5)).is_nan());
        }
        let (x, post) = (
            f32::from_bits(0xbe85_e4a7),
            PostOp::new(f32::from_bits(0xbfa0_820c), f32::from_bits(0x3ffb_1d93)),
        );
        let rad = transcendental::atan(x);
        assert_eq!(
            atan(x, post),
            per_arch(
                (rad * RAD_TO_DEG) * post.scale + post.offset,
                rad.mul_add(RAD_TO_DEG * post.scale, post.offset)
            )
        );
        if ARCH == Arch::X86_64 {
            assert_eq!(atan(x, post).to_bits(), 0x41a2_b659);
        }
        assert_eq!(
            (rad * (RAD_TO_DEG * post.scale) + post.offset).to_bits(),
            0x41a2_b65a
        );
    }

    #[test]
    fn sin_and_cos_at_key_degrees() {
        close(sin(90.0, ID), 1.0, 1e-6);
        close(cos(180.0, ID), -1.0, 1e-6);
        close(sin(180.0, ID), -8.742_278e-8, 1e-12);
        assert_eq!(sin(0.0, ID), 0.0);
        assert_eq!(cos(0.0, ID), 1.0);
        close(cos(0.0, PostOp::new(-2.0, 1.1)), -0.9, 1e-6);
    }
}
