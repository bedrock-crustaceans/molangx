//! `math.random`, `math.random_integer`, `math.die_roll` and `math.die_roll_integer`.
//!
//! Each operator is a pure function of its bounds and of the raw samples it draws, each clamped by
//! [`clamp_sample`]; the number and order of draws are part of the contract.

use super::arch::{
    self, Sort, integer_draw, integer_reach, integer_reach_const, interpolate, sorted_bounds,
};
use crate::numeric::{PostOp, arith};

mod dice;

pub use dice::{DieRoll, die_roll, die_roll_count, die_roll_integer};

/// Clamps a raw sample to `[0, 1]`; NaN and `−0` become `+0`.
#[inline]
pub fn clamp_sample(raw: f32) -> f32 {
    if raw > 1.0 {
        1.0
    } else if raw > 0.0 {
        raw
    } else {
        0.0
    }
}

/// `math.random(a, b)` with run-time bounds, over the closed interval of the sorted bounds.
///
/// `X86_64` computes `hi·r + (1 − r)·lo`, NaN for a NaN bound; `Arm64` computes `lo + r·(hi − lo)`
/// rounded once, ignoring a NaN bound.
#[inline]
pub fn random(a: f32, b: f32, sample: f32, post: PostOp) -> f32 {
    let r = clamp_sample(sample);
    let (lo, hi) = sorted_bounds(Sort::RunTime, a, b);
    post.apply(interpolate(lo, hi, r))
}

/// The post-op `math.random(a, b)` with two literal bounds folds into: `S' = (hi − lo)·S`,
/// `O' = lo·S + O` (rounded once on `Arm64`), evaluated by [`random_folded`].
#[inline]
pub fn random_const_bounds(a: f32, b: f32, post: PostOp) -> PostOp {
    let (lo, hi) = sorted_bounds(Sort::Literal, a, b);
    PostOp {
        scale: arch::mul(arch::sub(hi, lo), post.scale),
        offset: arith::mul_add(lo, post.scale, post.offset),
    }
}

/// `math.random` with literal bounds: `r·S' + O'` for a [`random_const_bounds`] post-op.
///
/// Unlike [`PostOp::apply`] this always multiplies and adds, even for `(1, 0)`.
#[inline]
pub fn random_folded(sample: f32, folded: PostOp) -> f32 {
    arith::mul_add(clamp_sample(sample), folded.scale, folded.offset)
}

/// `math.random_integer(a, b)` with run-time bounds.
///
/// `X86_64`: `floor((1 − r)·lo + r·top)` with `top = (hi·(−ε) + 1) + hi`; `Arm64`:
/// `floor(lo + r·span)` with `span = (hi + 1 − lo) − hi·ε`; then clamped to `[lo, hi]`. The bounds
/// are not rounded, so `random_integer(0.1, 1000001)` at `r = 0` is 0.1.
#[inline]
pub fn random_integer(a: f32, b: f32, sample: f32, post: PostOp) -> f32 {
    let r = clamp_sample(sample);
    let (lo, hi) = sorted_bounds(Sort::RunTime, a, b);
    let reach = integer_reach(lo, hi);
    post.apply(integer_draw(lo, hi, reach, r))
}

/// `math.random_integer(a, b)` with two literal bounds.
///
/// On `Arm64` the span is `(hi·(1 − ε) + 1) − lo`, which rounds differently from the run-time form
/// and makes a draw above 0 with an infinite bound +∞ (`random_integer(0, 1e39)`; 0 with run-time
/// bounds); on `X86_64` only the sorting of a NaN bound differs.
#[inline]
pub fn random_integer_const_bounds(a: f32, b: f32, sample: f32, post: PostOp) -> f32 {
    let r = clamp_sample(sample);
    let (lo, hi) = sorted_bounds(Sort::Literal, a, b);
    let reach = integer_reach_const(lo, hi);
    post.apply(integer_draw(lo, hi, reach, r))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::numeric::{ARCH, Arch, test_support::*};
    use crate::rng::FixedRng;
    use crate::stdlib::math::arch::{arm64::ONE_MINUS_EPSILON, clamp_draw};
    use proptest::prelude::*;

    #[test]
    fn clamp_sample_keeps_values_inside_the_unit_interval() {
        for raw in [
            0.25_f32,
            0.5,
            0.999_999_9,
            1.0e-30,
            1.0e-45,
            f32::MIN_POSITIVE,
        ] {
            assert_bits(clamp_sample(raw), raw);
        }
        assert_bits(clamp_sample(0.0), 0.0);
        assert_bits(clamp_sample(1.0), 1.0);
    }

    #[test]
    fn clamp_sample_clamps_everything_else() {
        for raw in [1.000_000_1, 2.0, 1.0e30, f32::MAX, INF] {
            assert_bits(clamp_sample(raw), 1.0);
        }
        for raw in [-1.0e-45, -0.5, -1.0, -1.0e30, f32::MIN, -INF] {
            assert_bits(clamp_sample(raw), 0.0);
        }
        assert_bits(clamp_sample(-0.0), 0.0);
        assert_bits(clamp_sample(NAN), 0.0);
        assert_bits(clamp_sample(-NAN), 0.0);
    }

    proptest! {
        #[test]
        fn clamp_sample_is_always_inside_the_closed_unit_interval(raw in proptest::num::f32::ANY) {
            let clamped = clamp_sample(raw);
            prop_assert!((0.0..=1.0).contains(&clamped));
            prop_assert!(clamped.is_sign_positive());
            if (0.0..=1.0).contains(&raw) && raw != 0.0 {
                prop_assert_eq!(clamped.to_bits(), raw.to_bits());
            }
        }
    }

    #[test]
    fn sorted_bounds_orders_ordinary_bounds_on_both_architectures() {
        for sort in [Sort::RunTime, Sort::Literal] {
            assert_eq!(sorted_bounds(sort, 1.0, 3.0), (1.0, 3.0));
            assert_eq!(sorted_bounds(sort, 3.0, 1.0), (1.0, 3.0));
            assert_eq!(sorted_bounds(sort, 2.0, 2.0), (2.0, 2.0));
            assert_eq!(sorted_bounds(sort, -INF, INF), (-INF, INF));
            assert_eq!(sorted_bounds(sort, INF, -INF), (-INF, INF));
            assert_eq!(sorted_bounds(sort, -5.0, -2.0), (-5.0, -2.0));
        }
    }

    #[test]
    fn sorted_bounds_with_a_nan_bound_depends_on_the_architecture_and_the_sort() {
        let nan_pair = |(lo, hi): (f32, f32)| (lo.is_nan(), hi.is_nan());
        if ARCH == Arch::X86_64 {
            assert_eq!(
                nan_pair(sorted_bounds(Sort::RunTime, NAN, 4.0)),
                (true, false)
            );
            assert_eq!(sorted_bounds(Sort::RunTime, NAN, 4.0).1, 4.0);
            assert_eq!(
                nan_pair(sorted_bounds(Sort::RunTime, 4.0, NAN)),
                (false, true)
            );
            assert_eq!(sorted_bounds(Sort::RunTime, 4.0, NAN).0, 4.0);
            assert_eq!(
                nan_pair(sorted_bounds(Sort::Literal, NAN, 4.0)),
                (true, true)
            );
            assert_eq!(sorted_bounds(Sort::Literal, 4.0, NAN), (4.0, 4.0));
        }
        for sort in [Sort::RunTime, Sort::Literal] {
            if ARCH == Arch::Arm64 {
                assert_eq!(sorted_bounds(sort, NAN, 4.0), (4.0, 4.0));
                assert_eq!(sorted_bounds(sort, 4.0, NAN), (4.0, 4.0));
            }
            assert_eq!(nan_pair(sorted_bounds(sort, NAN, NAN)), (true, true));
        }
    }

    #[test]
    fn sorted_bounds_of_equal_zeros_of_opposite_sign() {
        let (lo, hi) = crate::stdlib::math::arch::x86_64::sorted_bounds(Sort::RunTime, 0.0, -0.0);
        assert_bits(lo, 0.0);
        assert_bits(hi, -0.0);
        let (lo, hi) = crate::stdlib::math::arch::x86_64::sorted_bounds(Sort::Literal, 0.0, -0.0);
        assert_bits(lo, 0.0);
        assert_bits(hi, 0.0);
        for (a, b) in [(0.0, -0.0), (-0.0, 0.0)] {
            let (lo, hi) = crate::stdlib::math::arch::arm64::sorted_bounds(Sort::RunTime, a, b);
            assert_bits(lo, -0.0);
            assert_bits(hi, 0.0);
        }
    }

    #[test]
    fn clamp_draw_tests_the_upper_bound_first() {
        assert_bits(clamp_draw(5.0, 1.0, 3.0), 3.0);
        assert_bits(clamp_draw(0.0, 1.0, 3.0), 1.0);
        assert_bits(clamp_draw(2.0, 1.0, 3.0), 2.0);
        assert_bits(clamp_draw(3.0, 1.0, 3.0), 3.0);
        assert_bits(clamp_draw(2.0, 3.0, 1.0), 1.0);
        assert_bits(clamp_draw(0.5, 3.0, 1.0), 3.0);
        assert_bits(clamp_draw(NAN, 1.0, 3.0), 1.0);
    }

    #[test]
    fn interpolate_hits_the_bounds_at_samples_zero_and_one() {
        assert_bits(interpolate(2.0, 7.0, 0.0), 2.0);
        assert_bits(interpolate(2.0, 7.0, 1.0), 7.0);
        assert_bits(interpolate(2.0, 7.0, 0.5), 4.5);
        assert_bits(interpolate(-3.0, 3.0, 0.5), 0.0);
        assert_bits(interpolate(5.0, 5.0, 0.37), 5.0);
    }

    #[test]
    fn interpolate_uses_the_formula_of_each_architecture() {
        let (lo, hi, r) = (
            third(),
            f32::from_bits(0x3f91_bc0d),
            f32::from_bits(0x3ecf_a978),
        );
        assert_bits(
            interpolate(lo, hi, r),
            per_arch(hi * r + (1.0 - r) * lo, (hi - lo).mul_add(r, lo)),
        );
    }

    #[test]
    fn interpolate_differs_by_one_ulp_between_the_architectures_on_some_operands() {
        let (lo, hi, r) = (1.0_f32 / 7.0, 1.0_f32 / 7.0 + 1.0 / 3.0, 3.0_f32 / 19.0);
        assert_bits(
            interpolate(lo, hi, r),
            per_arch(hi * r + (1.0 - r) * lo, (hi - lo).mul_add(r, lo)),
        );
        assert_eq!(
            interpolate(lo, hi, r).to_bits(),
            per_arch(0x3e48_2e32, 0x3e48_2e33)
        );
    }

    #[test]
    fn interpolate_with_infinite_bounds_differs_between_the_architectures() {
        // x86-64 never forms hi − lo.
        if ARCH == Arch::X86_64 {
            assert_eq!(
                interpolate(f32::NEG_INFINITY, -180.0, 0.0),
                f32::NEG_INFINITY
            );
        }
        if ARCH == Arch::Arm64 {
            assert!(interpolate(f32::NEG_INFINITY, -180.0, 0.0).is_nan());
        }
        if ARCH == Arch::X86_64 {
            assert!(interpolate(-INF, INF, 0.5).is_nan());
        }
        if ARCH == Arch::Arm64 {
            assert!(interpolate(-INF, INF, 0.5).is_nan());
        }
        if ARCH == Arch::X86_64 {
            assert!(interpolate(1.0, INF, 0.0).is_nan());
        }
        if ARCH == Arch::Arm64 {
            assert!(interpolate(1.0, INF, 0.0).is_nan());
            assert_eq!(interpolate(1.0, INF, 0.5), INF);
        }
    }

    #[test]
    fn integer_reach_follows_the_formula_of_each_architecture() {
        let (lo, hi) = (2.0_f32, 7.0_f32);
        assert_bits(
            integer_reach(lo, hi),
            per_arch(
                (-f32::EPSILON * hi + 1.0) + hi,
                hi.mul_add(-f32::EPSILON, (hi + 1.0) - lo),
            ),
        );
        let x86 = crate::stdlib::math::arch::x86_64::integer_reach(lo, hi);
        assert!(x86 < 8.0 && x86 > 7.999_998, "{x86}");
        let arm = crate::stdlib::math::arch::arm64::integer_reach(lo, hi);
        assert!(arm < 6.0 && arm > 5.999_998, "{arm}");
        close_reach(
            crate::stdlib::math::arch::x86_64::integer_reach(0.5, 2.5),
            3.5,
        );
        close_reach(
            crate::stdlib::math::arch::arm64::integer_reach(0.5, 2.5),
            3.0,
        );
    }

    #[track_caller]
    fn close_reach(actual: f32, expected: f32) {
        assert!(
            (actual - expected).abs() < 1e-5,
            "{actual} is not near {expected}"
        );
    }

    #[test]
    fn integer_draw_floors_and_clamps_to_the_bounds() {
        let (lo, hi) = (2.0_f32, 7.0_f32);
        let reach = integer_reach(lo, hi);
        assert_bits(integer_draw(lo, hi, reach, 0.0), 2.0);
        assert_bits(integer_draw(lo, hi, reach, 1.0), 7.0);
        assert_bits(integer_draw(lo, hi, reach, 0.5), 4.0);
        assert_bits(integer_draw(lo, hi, reach, 0.999), 7.0);
        assert_bits(integer_draw(lo, hi, reach, 0.1), 2.0);
        assert_bits(integer_draw(lo, hi, reach, 0.17), 3.0);
        assert_bits(integer_draw(lo, hi, 100.0, 1.0), 7.0);
        assert_bits(integer_draw(lo, hi, -100.0, 1.0), 2.0);
    }

    #[test]
    fn random_at_samples_zero_half_and_one() {
        assert_bits(random(2.0, 7.0, 0.0, ID), 2.0);
        assert_bits(random(2.0, 7.0, 0.5, ID), 4.5);
        assert_bits(random(2.0, 7.0, 1.0, ID), 7.0);
        assert_bits(random(-4.0, 4.0, 0.5, ID), 0.0);
        assert_bits(random(-4.0, -2.0, 0.5, ID), -3.0);
        for sample in [0.0, 0.3, 1.0] {
            assert_bits(random(3.0, 3.0, sample, ID), 3.0);
        }
    }

    #[test]
    fn random_with_inverted_bounds_equals_the_sorted_call() {
        for sample in [0.0, 0.1, 0.5, 0.9, 1.0] {
            assert_bits(random(7.0, 2.0, sample, ID), random(2.0, 7.0, sample, ID));
            assert_bits(
                random(7.0, 2.0, sample, AFFINE),
                random(2.0, 7.0, sample, AFFINE),
            );
        }
    }

    #[test]
    fn random_clamps_the_sample() {
        assert_bits(random(2.0, 7.0, 3.0, ID), 7.0);
        assert_bits(random(2.0, 7.0, INF, ID), 7.0);
        assert_bits(random(2.0, 7.0, -3.0, ID), 2.0);
        assert_bits(random(2.0, 7.0, -INF, ID), 2.0);
        assert_bits(random(2.0, 7.0, NAN, ID), 2.0);
        assert_bits(random(2.0, 7.0, -0.0, ID), 2.0);
        for (a, b, sample, run_time, literal) in [
            (
                16_777_216.0,
                -1_274.555_4,
                1_695.562_9,
                per_arch(0x4b7f_ffff, 0x4b7f_fffd),
                0x4b7f_ffff,
            ),
            (3.866_985e-26, -INF, 417.559_8, 0xff80_0000, 0xff80_0000),
            (
                712.738_95,
                4.589_961_6e8,
                16_777_216.0,
                0x4dda_ddcc,
                0x4dda_ddcc,
            ),
        ] {
            assert_eq!(
                random_integer(a, b, sample, ID).to_bits(),
                run_time,
                "{a} {b} {sample}"
            );
            assert_eq!(
                random_integer_const_bounds(a, b, sample, ID).to_bits(),
                literal,
                "{a} {b} {sample}"
            );
        }
    }

    /// `floor` can land past either bound; both forms clamp it back.
    #[test]
    fn an_integer_draw_is_clamped_to_the_bounds_on_both_architectures() {
        use crate::stdlib::math::arch::{arm64, x86_64};
        assert_bits(
            arm64::integer_draw(0.0, 2.5, arm64::integer_reach(0.0, 2.5), 1.0),
            2.5,
        );
        assert_bits(
            x86_64::integer_draw(0.0, 2.5, x86_64::integer_reach(0.0, 2.5), 1.0),
            2.5,
        );
        assert_bits(
            arm64::integer_draw(0.5, 3.0, arm64::integer_reach(0.5, 3.0), 0.0),
            0.5,
        );
        assert_bits(
            x86_64::integer_draw(0.5, 3.0, x86_64::integer_reach(0.5, 3.0), 0.0),
            0.5,
        );
    }

    #[test]
    fn random_applies_the_post_op() {
        assert_bits(random(2.0, 7.0, 0.5, AFFINE), 10.0);
        assert_bits(random(2.0, 7.0, 0.0, PostOp::new(-1.0, 0.0)), -2.0);
        assert_bits(random(2.0, 7.0, 1.0, PostOp::new(0.0, 5.0)), 5.0);
        assert_bits(random(0.0, 0.0, 0.5, PostOp::new(-1.0, 0.0)), 0.0);
    }

    #[test]
    fn random_bounds_with_nan() {
        if ARCH == Arch::X86_64 {
            assert!(random(NAN, 4.0, 0.5, ID).is_nan());
        }
        for sample in [0.0, 0.5, 1.0] {
            if ARCH == Arch::X86_64 {
                assert!(random(NAN, 4.0, sample, ID).is_nan());
                assert!(random(4.0, NAN, sample, ID).is_nan());
                assert!(random_integer(NAN, 4.0, sample, ID).is_nan());
                assert_eq!(random_integer(4.0, NAN, sample, ID), 4.0);
            }
        }
        if ARCH == Arch::X86_64 {
            assert!(random_folded(0.5, random_const_bounds(NAN, 4.0, ID)).is_nan());
            assert_eq!(random_folded(0.5, random_const_bounds(4.0, NAN, ID)), 4.0);
        }
        if ARCH == Arch::Arm64 {
            assert_eq!(random(NAN, 5.0, 0.5, ID), 5.0);
            assert_eq!(random(5.0, NAN, 0.5, ID), 5.0);
        }
    }

    #[test]
    fn random_with_two_nan_bounds_is_nan_on_both_architectures() {
        for sample in [0.0, 0.5, 1.0] {
            assert!(random(NAN, NAN, sample, ID).is_nan());
            assert!(random_integer(NAN, NAN, sample, ID).is_nan());
        }
    }

    #[test]
    fn random_with_a_nan_bound_on_arm64_is_the_other_bound_at_every_sample() {
        for sample in [0.0, 0.25, 0.5, 1.0] {
            if ARCH == Arch::Arm64 {
                assert_bits(random(NAN, 5.0, sample, ID), 5.0);
                assert_bits(random(5.0, NAN, sample, ID), 5.0);
                assert_bits(random(NAN, 5.0, sample, AFFINE), 11.0);
            }
        }
    }

    #[test]
    fn random_with_infinite_bounds() {
        if ARCH == Arch::X86_64 {
            assert_eq!(random(-INF, 5.0, 0.0, ID), -INF);
            assert_eq!(random(5.0, INF, 1.0, ID), INF);
            assert!(random(-INF, 5.0, 1.0, ID).is_nan());
        }
        if ARCH == Arch::Arm64 {
            assert!(random(-INF, 5.0, 0.0, ID).is_nan());
            assert_eq!(random(5.0, INF, 1.0, ID), INF);
        }
        assert!(random(-INF, INF, 0.5, ID).is_nan());
        assert_eq!(random(5.0, INF, 0.5, ID), INF);
    }

    #[test]
    fn random_of_negative_zero_bounds_differs_between_the_architectures() {
        // arm64 forms (−0 − (−0))·0.5 + (−0) = +0.
        assert_bits(random(-0.0, -0.0, 0.5, ID), per_arch(-0.0, 0.0));
    }

    #[test]
    fn random_interpolation_differs_between_the_architectures() {
        let (lo, hi, r) = (
            third(),
            f32::from_bits(0x3f91_bc0d),
            f32::from_bits(0x3ecf_a978),
        );
        assert_eq!(
            random(lo, hi, r, ID),
            per_arch(hi * r + (1.0 - r) * lo, (hi - lo).mul_add(r, lo))
        );
        if ARCH == Arch::X86_64 {
            assert_eq!(random(lo, hi, r, ID).to_bits(), 0x3f28_f09f);
        }
        // arm64's shape rounded step by step is one ulp lower.
        assert_eq!(((hi - lo) * r + lo).to_bits(), 0x3f28_f09e);
        if ARCH == Arch::X86_64 {
            assert_eq!(
                random(-180.0, f32::NEG_INFINITY, 0.0, ID),
                f32::NEG_INFINITY
            );
        }
        if ARCH == Arch::Arm64 {
            assert!(random(f32::NEG_INFINITY, f32::INFINITY, 0.5, ID).is_nan());
        }
        let (lo, hi, r) = (-3.0_f32, 2.0_f32, 0.5_f32);
        let top = (-f32::EPSILON * hi + 1.0) + hi;
        if ARCH == Arch::X86_64 {
            assert_eq!(
                random_integer(lo, hi, r, ID),
                ((1.0 - r) * lo + top * r).floor()
            );
            assert_eq!(
                random_integer_const_bounds(lo, hi, r, ID),
                ((1.0 - r) * lo + top * r).floor()
            );
        }
        let span = hi.mul_add(-f32::EPSILON, (hi + 1.0) - lo);
        if ARCH == Arch::Arm64 {
            assert_eq!(random_integer(lo, hi, r, ID), span.mul_add(r, lo).floor());
        }
        // arm64's span 6 − 2ε rounds to 6, giving 0; x86-64's 0.5·(−3) + 0.5·2.9999998 is just
        // below 0.
        assert_eq!(random_integer(lo, hi, r, ID), per_arch(-1.0, 0.0));
        let mut half = FixedRng::HALF;
        assert_eq!(
            die_roll_integer(1.0, lo, hi, 1, &mut half, ID),
            per_arch(Some(-1.0), Some(0.0))
        );
        let mut rng = FixedRng::from_sample(third()).expect("a word's sample");
        assert_eq!(
            die_roll(1.0, 1.1, 7.3, 1, &mut rng, ID),
            per_arch(
                Some(0.0 + (third() * 7.0 + (1.0 - third()) * 1.0)),
                Some(third().mul_add(6.0, 0.0 + 1.0))
            )
        );
    }

    #[test]
    fn random_const_bounds_folds_the_bounds_into_the_post_op() {
        assert_eq!(random_const_bounds(2.0, 7.0, ID), PostOp::new(5.0, 2.0));
        assert_eq!(random_const_bounds(7.0, 2.0, ID), PostOp::new(5.0, 2.0));
        assert_eq!(
            random_const_bounds(7.0, 2.0, AFFINE),
            PostOp::new(10.0, 5.0)
        );
        assert_eq!(random_const_bounds(3.0, 3.0, AFFINE), PostOp::new(0.0, 7.0));
        assert_eq!(
            random_const_bounds(-4.0, 4.0, PostOp::new(-1.0, 0.5)),
            PostOp::new(-8.0, 4.5)
        );
        assert_eq!(random_const_bounds(-INF, 1.0, ID).scale, INF);
    }

    #[test]
    fn random_const_bounds_rounds_the_offset_with_the_architecture() {
        let (lo, post) = (1.1_f32, PostOp::new(3.3, 0.7));
        assert_bits(
            random_const_bounds(lo, 9.0, post).offset,
            per_arch(lo * 3.3 + 0.7, lo.mul_add(3.3, 0.7)),
        );
        assert_bits(
            random_const_bounds(lo, 9.0, post).scale,
            (9.0_f32 - lo) * 3.3,
        );
    }

    #[test]
    fn random_const_bounds_offset_differs_between_the_architectures_for_an_inexact_product() {
        let (lo, post) = (0.1_f32, PostOp::new(3.1, 1.1));
        assert_ne!(
            (lo * 3.1 + 1.1).to_bits(),
            lo.mul_add(3.1, 1.1).to_bits(),
            "the operands must tell fused from unfused"
        );
        assert_eq!(
            random_const_bounds(lo, 0.5, post).offset.to_bits(),
            per_arch(0x3fb4_7ae2, 0x3fb4_7ae1)
        );
        let folded = random_const_bounds(lo, 0.5, post);
        assert_bits(
            random_folded(0.3, folded),
            per_arch(
                0.3 * folded.scale + folded.offset,
                0.3_f32.mul_add(folded.scale, folded.offset),
            ),
        );
    }

    #[test]
    fn random_const_bounds_with_a_nan_bound() {
        let folded = random_const_bounds(NAN, 4.0, ID);
        assert_eq!(
            folded.scale.is_nan() && folded.offset.is_nan(),
            per_arch(true, false)
        );
        if ARCH == Arch::Arm64 {
            assert_eq!(folded, PostOp::new(0.0, 4.0));
        }
        assert_eq!(random_const_bounds(4.0, NAN, ID), PostOp::new(0.0, 4.0));
    }

    #[test]
    fn random_const_bounds_chooses_its_nans() {
        let (q1, q2) = (f32::from_bits(0x7fc0_0001), f32::from_bits(0xffc0_0002));
        if ARCH == Arch::X86_64 {
            assert_eq!(
                random_const_bounds(q1, 4.0, PostOp::new(q2, 0.0))
                    .scale
                    .to_bits(),
                0x7fc0_0001
            );
        }
        if ARCH == Arch::Arm64 {
            assert_eq!(
                random_const_bounds(q1, q1, PostOp::new(q2, 0.0))
                    .scale
                    .to_bits(),
                0x7fc0_0001
            );
        }
        assert_eq!(
            random_const_bounds(INF, INF, ID).scale.to_bits(),
            per_arch(0xffc0_0000, 0x7fc0_0000)
        );
    }

    #[test]
    fn random_folded_always_multiplies_and_adds() {
        assert_bits(random_folded(0.0, PostOp::new(5.0, 2.0)), 2.0);
        assert_bits(random_folded(0.5, PostOp::new(5.0, 2.0)), 4.5);
        assert_bits(random_folded(1.0, PostOp::new(5.0, 2.0)), 7.0);
        assert_bits(random_folded(9.0, PostOp::new(5.0, 2.0)), 7.0);
        assert_bits(random_folded(-9.0, PostOp::new(5.0, 2.0)), 2.0);
        assert_bits(random_folded(NAN, PostOp::new(5.0, 2.0)), 2.0);
        // (1, −0) is still multiplied and added, so the −0 offset is normalised.
        assert_bits(random_folded(0.0, PostOp::new(1.0, -0.0)), 0.0);
        assert_bits(random_folded(0.25, PostOp::IDENTITY), 0.25);
        assert_bits(random_folded(1.0, PostOp::new(0.0, 3.0)), 3.0);
        let folded = PostOp::new(3.0, -1.0);
        if ARCH == Arch::X86_64 {
            assert_bits(random_folded(third(), folded), 0.0);
        }
        if ARCH == Arch::Arm64 {
            assert_eq!(random_folded(third(), folded), 2.980_232_2e-8);
        }
    }

    #[test]
    fn folded_form_agrees_with_the_run_time_form_for_exact_bounds() {
        for (a, b) in [(2.0_f32, 7.0), (7.0, 2.0), (-4.0, 4.0), (0.0, 1.0)] {
            for sample in [0.0, 0.25, 0.5, 1.0] {
                for post in [ID, AFFINE, PostOp::new(0.5, -3.0)] {
                    let folded = random_const_bounds(a, b, post);
                    assert_bits(random_folded(sample, folded), random(a, b, sample, post));
                }
            }
        }
    }

    #[test]
    fn random_integer_at_samples_zero_half_and_one() {
        assert_bits(random_integer(2.0, 7.0, 0.0, ID), 2.0);
        assert_bits(random_integer(2.0, 7.0, 0.5, ID), 4.0);
        assert_bits(random_integer(2.0, 7.0, 1.0, ID), 7.0);
        assert_bits(random_integer(0.0, 1.0, 0.0, ID), 0.0);
        assert_bits(random_integer(0.0, 1.0, 0.5, ID), 0.0);
        assert_bits(random_integer(0.0, 1.0, 1.0, ID), 1.0);
        assert_bits(random_integer(-3.0, 2.0, 0.0, ID), -3.0);
        assert_bits(random_integer(-3.0, 2.0, 1.0, ID), 2.0);
        for sample in [0.0, 0.5, 1.0] {
            assert_bits(random_integer(3.0, 3.0, sample, ID), 3.0);
        }
    }

    #[test]
    fn random_integer_with_inverted_bounds_equals_the_sorted_call() {
        for sample in [0.0, 0.1, 0.5, 0.9, 1.0] {
            assert_bits(
                random_integer(7.0, 2.0, sample, ID),
                random_integer(2.0, 7.0, sample, ID),
            );
            assert_bits(
                random_integer_const_bounds(7.0, 2.0, sample, ID),
                random_integer_const_bounds(2.0, 7.0, sample, ID),
            );
        }
    }

    #[test]
    fn random_integer_covers_every_integer_between_the_bounds() {
        let mut seen = [false; 6];
        for k in 0..=1000 {
            let value = random_integer(2.0, 7.0, k as f32 / 1000.0, ID);
            assert_eq!(value, value.floor());
            assert!((2.0..=7.0).contains(&value));
            seen[value as usize - 2] = true;
        }
        assert_eq!(seen, [true; 6]);
    }

    #[test]
    fn random_integer_does_not_round_its_bounds() {
        assert_bits(random_integer(0.5, 2.5, 0.0, ID), 0.5);
        assert_bits(random_integer(0.5, 2.5, 1.0, ID), 2.5);
        assert_bits(random_integer_const_bounds(0.5, 2.5, 0.0, ID), 0.5);
        assert_bits(random_integer_const_bounds(0.5, 2.5, 1.0, ID), 2.5);
        assert_eq!(random_integer(0.1, 1_000_001.0, 0.0, ID), 0.1);
    }

    #[test]
    fn random_integer_clamps_the_sample_and_applies_the_post_op() {
        assert_bits(random_integer(2.0, 7.0, 9.0, ID), 7.0);
        assert_bits(random_integer(2.0, 7.0, -9.0, ID), 2.0);
        assert_bits(random_integer(2.0, 7.0, NAN, ID), 2.0);
        assert_bits(random_integer(0.0, 3.0, 1.0, AFFINE), 7.0);
        assert_bits(random_integer(0.0, 3.0, 0.0, AFFINE), 1.0);
        assert_bits(random_integer_const_bounds(0.0, 3.0, 1.0, AFFINE), 7.0);
        assert_bits(random_integer(0.0, 0.0, 0.5, PostOp::new(-1.0, 0.0)), 0.0);
    }

    #[test]
    fn random_integer_with_nan_bounds() {
        for sample in [0.0, 0.5, 1.0] {
            if ARCH == Arch::X86_64 {
                assert!(random_integer(NAN, 4.0, sample, ID).is_nan());
                assert_bits(random_integer(4.0, NAN, sample, ID), 4.0);
            }
            if ARCH == Arch::Arm64 {
                assert_bits(random_integer(NAN, 4.0, sample, ID), 4.0);
                assert_bits(random_integer(4.0, NAN, sample, ID), 4.0);
            }
        }
    }

    #[test]
    fn random_integer_const_bounds_sorts_nan_with_the_literal_rule_on_x86_64() {
        for sample in [0.0, 0.5, 1.0] {
            if ARCH == Arch::X86_64 {
                assert!(random_integer_const_bounds(NAN, 4.0, sample, ID).is_nan());
                assert_bits(random_integer_const_bounds(4.0, NAN, sample, ID), 4.0);
            }
            if ARCH == Arch::Arm64 {
                assert_bits(random_integer_const_bounds(NAN, 4.0, sample, ID), 4.0);
                assert_bits(random_integer_const_bounds(4.0, NAN, sample, ID), 4.0);
            }
        }
    }

    #[test]
    fn random_integer_const_bounds_matches_the_run_time_form_on_x86_64() {
        for lo in -8..8 {
            for hi in lo..16 {
                for k in 0..=20 {
                    let (lo, hi, r) = (lo as f32, hi as f32, k as f32 / 20.0);
                    if ARCH == Arch::X86_64 {
                        assert_bits(
                            random_integer_const_bounds(lo, hi, r, ID),
                            random_integer(lo, hi, r, ID),
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn random_integer_const_bounds_rounds_the_span_differently_on_arm64() {
        // (−8, 5) at 0.5: the run-time span 13.999999 floors to −2; the literal span rounds up
        // to 14.
        if ARCH == Arch::Arm64 {
            assert_bits(random_integer(-8.0, 5.0, 0.5, ID), -2.0);
            assert_bits(random_integer_const_bounds(-8.0, 5.0, 0.5, ID), -1.0);
            assert_bits(random_integer(-8.0, 9.0, 0.5, ID), 0.0);
            assert_bits(random_integer_const_bounds(-8.0, 9.0, 0.5, ID), 1.0);
        }
        if ARCH == Arch::X86_64 {
            assert_bits(
                random_integer(-8.0, 5.0, 0.5, ID),
                random_integer_const_bounds(-8.0, 5.0, 0.5, ID),
            );
        }
    }

    #[test]
    fn the_arm64_literal_span_uses_one_minus_epsilon() {
        assert_eq!(ONE_MINUS_EPSILON.to_bits(), 0x3f7f_fffe);
        assert_eq!(ONE_MINUS_EPSILON, 1.0 - f32::EPSILON);
        let (lo, hi, r) = (-8.0_f32, 5.0_f32, 0.5_f32);
        let span = hi.mul_add(ONE_MINUS_EPSILON, 1.0) - lo;
        if ARCH == Arch::Arm64 {
            assert_bits(
                random_integer_const_bounds(lo, hi, r, ID),
                span.mul_add(r, lo).floor(),
            );
        }
    }

    proptest! {
        #[test]
        fn random_lies_between_the_sorted_bounds_for_finite_bounds(a in -1000.0_f32..1000.0, b in -1000.0_f32..1000.0, sample in 0.0_f32..=1.0) {
            let (lo, hi) = (a.min(b), a.max(b));
            let slack = 1e-3;
            let value = random(a, b, sample, ID);
            prop_assert!(value >= lo - slack && value <= hi + slack, "{} not in [{}, {}]", value, lo, hi);
            let integer = random_integer(a, b, sample, ID);
            prop_assert!(integer >= lo && integer <= hi);
        }
    }

    #[test]
    fn random_samples_are_clamped() {
        assert_eq!(clamp_sample(0.25), 0.25);
        assert_eq!(clamp_sample(2.0), 1.0);
        assert_eq!(clamp_sample(-1.0), 0.0);
        assert_eq!(clamp_sample(-0.0).to_bits(), 0.0_f32.to_bits());
        assert_eq!(clamp_sample(NAN), 0.0);
        assert_eq!(clamp_sample(1.0), 1.0);
        assert_eq!(random(2.0, 7.0, 9.0, ID), 7.0);
        assert_eq!(random(2.0, 7.0, -9.0, ID), 2.0);
    }

    #[test]
    fn random_and_random_integer_on_key_samples_and_bounds() {
        for (sample, real, integer) in [(0.0, 2.0, 2.0), (0.5, 4.5, 4.0), (1.0, 7.0, 7.0)] {
            assert_eq!(random(2.0, 7.0, sample, ID), real);
            assert_eq!(random(7.0, 2.0, sample, ID), real, "inverted bounds");
            assert_eq!(random_integer(2.0, 7.0, sample, ID), integer);
            assert_eq!(
                random_integer(7.0, 2.0, sample, ID),
                integer,
                "inverted bounds"
            );
            assert_eq!(random_integer_const_bounds(2.0, 7.0, sample, ID), integer);
            assert_eq!(
                random(2.0, 7.0, sample, PostOp::new(2.0, 1.0)),
                real * 2.0 + 1.0
            );
            let folded = random_const_bounds(7.0, 2.0, PostOp::new(2.0, 1.0));
            assert_eq!(folded, PostOp::new(10.0, 5.0));
            assert_eq!(random_folded(sample, folded), real * 2.0 + 1.0);
            assert_eq!(
                random_folded(sample, random_const_bounds(2.0, 7.0, ID)),
                real
            );
        }
        assert_eq!(random_integer(0.1, 1_000_001.0, 0.0, ID), 0.1);
        assert_eq!(random_integer_const_bounds(0.1, 1_000_001.0, 0.0, ID), 0.1);
        assert_eq!(random_integer(0.0, 3.0, 1.0, PostOp::new(2.0, 1.0)), 7.0);
    }
}
