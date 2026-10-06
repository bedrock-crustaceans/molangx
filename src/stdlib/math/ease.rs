//! The 30 `math.ease_*(start, end, t)` functions: `start + (end − start)·f(t)`, `t` not clamped.
//!
//! Each architecture has its own formulas. `Arm64` fuses products into multiply-adds, folds the
//! bounce centres into the mirrored subtractions and computes the elastic angle as `t·20.943951 −
//! 22.514748` rounded once. `X86_64` rounds every operation in order (`c·t·t·t + b` chains, `(1 −
//! t) − centre`, the elastic angle `((x − 0.075)·2π)/0.3`) and returns `start + (end − start)`, not
//! `end`, at the elastic end point.
//!
//! The sine and elastic easings read a 65,536-entry sine table with a truncated index, so their
//! angle is quantised to 1/65536 of a turn. A NaN `t` gives NaN, except in the sine easings, which
//! read entry 0.

use super::arch;
use crate::numeric::PostOp;

macro_rules! easings {
    ($($(#[$doc:meta])* $name:ident => $raw:ident;)*) => {
        $(
            $(#[$doc])*
            #[inline]
            pub fn $name(start: f32, end: f32, t: f32, post: PostOp) -> f32 {
                post.apply(arch::$raw(start, end, t))
            }
        )*
    };
}

easings! {
    /// `math.ease_in_quad`: `f(t) = t²`.
    ease_in_quad => in_quad;
    /// `math.ease_out_quad`: `f(t) = 1 − (1 − t)²`.
    ease_out_quad => out_quad;
    /// `math.ease_in_out_quad`: with `u = 2t`, `u < 1 ? u²/2 : 1 − (2 − u)²/2`.
    ease_in_out_quad => in_out_quad;
    /// `math.ease_in_cubic`: `f(t) = t³`.
    ease_in_cubic => in_cubic;
    /// `math.ease_out_cubic`: `f(t) = (t − 1)³ + 1`.
    ease_out_cubic => out_cubic;
    /// `math.ease_in_out_cubic`: with `u = 2t`, `u < 1 ? u³/2 : ((u − 2)³ + 2)/2`.
    ease_in_out_cubic => in_out_cubic;
    /// `math.ease_in_quart`: `f(t) = t⁴`.
    ease_in_quart => in_quart;
    /// `math.ease_out_quart`: `f(t) = 1 − (t − 1)⁴`.
    ease_out_quart => out_quart;
    /// `math.ease_in_out_quart`: with `u = 2t`, `u < 1 ? u⁴/2 : −((u − 2)⁴ − 2)/2`.
    ease_in_out_quart => in_out_quart;
    /// `math.ease_in_quint`: `f(t) = t⁵`.
    ease_in_quint => in_quint;
    /// `math.ease_out_quint`: `f(t) = (t − 1)⁵ + 1`.
    ease_out_quint => out_quint;
    /// `math.ease_in_out_quint`: with `u = 2t`, `u < 1 ? u⁵/2 : ((u − 2)⁵ + 2)/2`.
    ease_in_out_quint => in_out_quint;
    /// `math.ease_in_sine`: `f(t) = 1 − cos(t·π/2)` with the cosine from the sine table.
    ease_in_sine => in_sine;
    /// `math.ease_out_sine`: `f(t) = sin(t·π/2)` from the sine table.
    ease_out_sine => out_sine;
    /// `math.ease_in_out_sine`: `f(t) = −(cos(π·t) − 1)/2` with the cosine from the sine table.
    ease_in_out_sine => in_out_sine;
    /// `math.ease_in_expo`: `f(t) = 2^(10t − 10)` with no `t == 0` special case.
    ease_in_expo => in_expo;
    /// `math.ease_out_expo`: `f(t) = 1 − 2^(−10t)` with no `t == 1` special case.
    ease_out_expo => out_expo;
    /// `math.ease_in_out_expo`: `2t < 1 ? 2^(20t − 10)/2 : (2 − 2^(10 − 20t))/2`, with no endpoint
    /// special cases.
    ease_in_out_expo => in_out_expo;
    /// `math.ease_in_circ`: `f(t) = 1 − sqrt(1 − t²)`.
    ease_in_circ => in_circ;
    /// `math.ease_out_circ`: `f(t) = sqrt(1 − (t − 1)²)`.
    ease_out_circ => out_circ;
    /// `math.ease_in_out_circ`: with `u = 2t`, `u < 1 ? −(sqrt(1 − u²) − 1)/2 : (sqrt(1 − (u −
    /// 2)²) + 1)/2`.
    ease_in_out_circ => in_out_circ;
    /// `math.ease_in_bounce`: `1 − out_bounce(1 − t)`.
    ease_in_bounce => in_bounce;
    /// `math.ease_out_bounce`: piecewise with `n1 = 7.5625`, `d1 = 2.75` and thresholds `1/d1`,
    /// `2/d1`, `2.5/d1`.
    ease_out_bounce => out_bounce;
    /// `math.ease_in_out_bounce`: `t < 0.5 ? (1 − out_bounce(1 − 2t))/2 : (1 + out_bounce(2t −
    /// 1))/2`.
    ease_in_out_bounce => in_out_bounce;
    /// `math.ease_in_back`: `f(t) = t²·(c3·t − c1)` with `c1 = 1.70158`, `c3 = 2.70158`.
    ease_in_back => in_back;
    /// `math.ease_out_back`: `f(t) = 1 + (t − 1)²·(c3·(t − 1) + c1)`.
    ease_out_back => out_back;
    /// `math.ease_in_out_back`: with `u = 2t` and `c2 = 2.5949094`, `u < 1 ? u²·((c2 + 1)·u − c2)/2
    /// : ((u − 2)²·((c2 + 1)(u − 2) + c2) + 2)/2`.
    ease_in_out_back => in_out_back;
    /// `math.ease_in_elastic`: `t == 0` → start, `t == 1` → end (on `X86_64`, `start + (end −
    /// start)`), otherwise `−2^(10t − 10)·sin((10t − 10.75)·2π/3)` with the sine from the table.
    ease_in_elastic => in_elastic;
    /// `math.ease_out_elastic`: `t == 0` → start, `t == 1` → end (on `X86_64`, `start + (end −
    /// start)`), otherwise `2^(−10t)·sin((10t − 0.75)·2π/3) + 1` with the sine from the table.
    ease_out_elastic => out_elastic;
    /// `math.ease_in_out_elastic`: `t == 0` → start, `2t == 2` → end (on `X86_64`, `start + (end −
    /// start)`); with `s = sin((20t − 10.75)·2π/3)` from the table, `2t < 1 ? −2^(20t − 10)·s/2 :
    /// 2^(10 − 20t)·s/2 + 1`.
    ease_in_out_elastic => in_out_elastic;
}

#[cfg(test)]
mod tests {
    // The curve table holds nine significant digits, computed in double precision.
    #![allow(
        clippy::excessive_precision,
        clippy::unreadable_literal,
        clippy::approx_constant
    )]
    use super::*;
    use crate::numeric::{ARCH, Arch, test_support::*};
    use crate::stdlib::math::transcendental;

    const T: [f32; 5] = [0.0, 0.25, 0.5, 0.75, 1.0];

    type Ease = fn(f32, f32, f32, PostOp) -> f32;

    /// All 30 easings with their curve `f` at [`T`].
    #[rustfmt::skip]
    const CURVES: [(&str, Ease, [f32; 5]); 30] = [
        ("in_quad", ease_in_quad, [0.0, 0.0625, 0.25, 0.5625, 1.0]),
        ("out_quad", ease_out_quad, [0.0, 0.4375, 0.75, 0.9375, 1.0]),
        ("in_out_quad", ease_in_out_quad, [0.0, 0.125, 0.5, 0.875, 1.0]),
        ("in_cubic", ease_in_cubic, [0.0, 0.015625, 0.125, 0.421875, 1.0]),
        ("out_cubic", ease_out_cubic, [0.0, 0.578125, 0.875, 0.984375, 1.0]),
        ("in_out_cubic", ease_in_out_cubic, [0.0, 0.0625, 0.5, 0.9375, 1.0]),
        ("in_quart", ease_in_quart, [0.0, 0.00390625, 0.0625, 0.31640625, 1.0]),
        ("out_quart", ease_out_quart, [0.0, 0.68359375, 0.9375, 0.99609375, 1.0]),
        ("in_out_quart", ease_in_out_quart, [0.0, 0.03125, 0.5, 0.96875, 1.0]),
        ("in_quint", ease_in_quint, [0.0, 0.0009765625, 0.03125, 0.2373046875, 1.0]),
        ("out_quint", ease_out_quint, [0.0, 0.7626953125, 0.96875, 0.9990234375, 1.0]),
        ("in_out_quint", ease_in_out_quint, [0.0, 0.015625, 0.5, 0.984375, 1.0]),
        ("in_sine", ease_in_sine, [0.0, 0.076120467, 0.292893219, 0.617316568, 1.0]),
        ("out_sine", ease_out_sine, [0.0, 0.382683432, 0.707106781, 0.923879533, 1.0]),
        ("in_out_sine", ease_in_out_sine, [0.0, 0.146446609, 0.5, 0.853553391, 1.0]),
        ("in_expo", ease_in_expo, [0.000976562, 0.005524272, 0.03125, 0.176776695, 1.0]),
        ("out_expo", ease_out_expo, [0.0, 0.823223305, 0.96875, 0.994475728, 0.999023438]),
        ("in_out_expo", ease_in_out_expo, [0.000488281, 0.015625, 0.5, 0.984375, 0.999511719]),
        ("in_circ", ease_in_circ, [0.0, 0.031754163, 0.133974596, 0.338562172, 1.0]),
        ("out_circ", ease_out_circ, [0.0, 0.661437828, 0.866025404, 0.968245837, 1.0]),
        ("in_out_circ", ease_in_out_circ, [0.0, 0.066987298, 0.5, 0.933012702, 1.0]),
        ("in_bounce", ease_in_bounce, [0.0, 0.02734375, 0.234375, 0.52734375, 1.0]),
        ("out_bounce", ease_out_bounce, [0.0, 0.47265625, 0.765625, 0.97265625, 1.0]),
        ("in_out_bounce", ease_in_out_bounce, [0.0, 0.1171875, 0.5, 0.8828125, 1.0]),
        ("in_back", ease_in_back, [0.0, -0.064136563, -0.0876975, 0.182590312, 1.0]),
        ("out_back", ease_out_back, [0.0, 0.817409688, 1.0876975, 1.064136563, 1.0]),
        ("in_out_back", ease_in_out_back, [0.0, -0.099681838, 0.5, 1.099681837, 1.0]),
        ("in_elastic", ease_in_elastic, [0.0, -0.005524272, -0.015625, 0.088388348, 1.0]),
        ("out_elastic", ease_out_elastic, [0.0, 0.911611652, 1.015625, 1.005524272, 1.0]),
        ("in_out_elastic", ease_in_out_elastic, [0.0, -0.0078125, 0.5, 1.0078125, 1.0]),
    ];

    /// The first 12 curves are exact in `f32` at [`T`].
    const EXACT_CURVES: usize = 12;

    #[test]
    fn every_easing_follows_its_curve_at_the_key_points() {
        for (index, (name, ease, curve)) in CURVES.iter().enumerate() {
            let table_based = name.contains("sine") || name.contains("elastic");
            for (t, f) in T.iter().zip(curve) {
                let actual = ease(1.0, 5.0, *t, ID);
                let expected = 1.0 + 4.0 * f;
                if index < EXACT_CURVES {
                    assert_eq!(actual, expected, "ease_{name}(1, 5, {t})");
                } else {
                    let tolerance = if table_based { 1e-3 } else { 2e-5 };
                    assert!(
                        within(actual, expected, tolerance),
                        "ease_{name}(1, 5, {t}) = {actual} (expected {expected})"
                    );
                }
            }
        }
    }

    #[test]
    fn every_easing_with_unit_range_returns_the_curve() {
        for (index, (name, ease, curve)) in CURVES.iter().enumerate() {
            let table_based = name.contains("sine") || name.contains("elastic");
            for (t, f) in T.iter().zip(curve) {
                let actual = ease(0.0, 1.0, *t, ID);
                if index < EXACT_CURVES {
                    assert_eq!(actual, *f, "ease_{name}(0, 1, {t})");
                } else {
                    let tolerance = if table_based { 3e-4 } else { 5e-6 };
                    assert!(
                        within(actual, *f, tolerance),
                        "ease_{name}(0, 1, {t}) = {actual} (expected {f})"
                    );
                }
            }
        }
    }

    #[test]
    fn every_easing_scales_by_the_range_and_shifts_by_the_start() {
        for (index, (name, ease, curve)) in CURVES.iter().enumerate() {
            let table_based = name.contains("sine") || name.contains("elastic");
            for (t, f) in T.iter().zip(curve) {
                let actual = ease(10.0, -6.0, *t, ID);
                let expected = 10.0 - 16.0 * f;
                let tolerance = if table_based {
                    2e-3
                } else if index < EXACT_CURVES {
                    0.0
                } else {
                    1e-4
                };
                assert!(
                    within(actual, expected, tolerance),
                    "ease_{name}(10, -6, {t}) = {actual} (expected {expected})"
                );
            }
        }
    }

    #[test]
    fn an_empty_range_returns_the_start_for_every_easing_and_t() {
        for (_, ease, _) in CURVES {
            for t in [0.0, 0.1, 0.25, 0.5, 0.9, 1.0] {
                assert_bits(ease(2.5, 2.5, t, ID), 2.5);
            }
        }
    }

    #[test]
    fn every_easing_applies_the_post_op_to_the_eased_value() {
        for (_, ease, _) in CURVES {
            for t in T {
                let raw = ease(1.0, 5.0, t, ID);
                assert_bits(
                    ease(1.0, 5.0, t, AFFINE),
                    crate::numeric::arith::mul_add(raw, 2.0, 1.0),
                );
                assert_bits(
                    ease(1.0, 5.0, t, PostOp::new(0.5, -3.0)),
                    crate::numeric::arith::mul_add(raw, 0.5, -3.0),
                );
            }
        }
    }

    #[test]
    fn a_post_op_of_scale_zero_gives_the_offset_for_every_easing() {
        for (_, ease, _) in CURVES {
            assert_bits(ease(1.0, 5.0, 0.4, PostOp::new(0.0, 7.0)), 7.0);
        }
    }

    #[test]
    fn a_nan_t_gives_nan_except_for_the_sine_easings_which_read_table_entry_zero() {
        for (name, ease, _) in CURVES {
            let result = ease(1.0, 5.0, NAN, ID);
            if name.ends_with("sine") {
                assert!(!result.is_nan(), "ease_{name}(NaN)");
            } else {
                assert!(result.is_nan(), "ease_{name}(NaN)");
            }
        }
    }

    #[test]
    fn the_sine_easings_of_nan_read_table_entry_zero() {
        // A NaN index truncates to entry 0, which is 0.
        assert_eq!(ease_out_sine(1.0, 5.0, NAN, ID), 1.0);
        assert_eq!(ease_in_sine(1.0, 5.0, NAN, ID), 5.0);
        assert_eq!(ease_in_out_sine(1.0, 5.0, NAN, ID), 3.0);
    }

    #[test]
    fn easings_do_not_clamp_t() {
        assert_eq!(ease_in_quad(0.0, 1.0, 2.0, ID), 4.0);
        assert_eq!(ease_in_quad(0.0, 1.0, -2.0, ID), 4.0);
        assert_eq!(ease_in_cubic(0.0, 1.0, 2.0, ID), 8.0);
        assert_eq!(ease_in_cubic(0.0, 1.0, -2.0, ID), -8.0);
        assert_eq!(ease_out_cubic(0.0, 1.0, 2.0, ID), 2.0);
        assert_eq!(ease_in_quart(0.0, 1.0, 2.0, ID), 16.0);
        assert_eq!(ease_in_quint(0.0, 1.0, -2.0, ID), -32.0);
        assert_eq!(ease_out_quad(0.0, 1.0, 2.0, ID), 0.0);
        assert_eq!(ease_in_expo(0.0, 1.0, 2.0, ID), 1024.0);
        assert!(ease_in_circ(0.0, 1.0, 2.0, ID).is_nan());
        assert!(ease_out_circ(0.0, 1.0, -1.0, ID).is_nan());
    }

    #[test]
    fn easings_of_infinite_t() {
        assert_eq!(ease_in_quad(0.0, 1.0, INF, ID), INF);
        assert_eq!(ease_in_quad(0.0, 1.0, -INF, ID), INF);
        assert_eq!(ease_in_cubic(0.0, 1.0, INF, ID), INF);
        assert_eq!(ease_in_cubic(0.0, 1.0, -INF, ID), -INF);
        assert_eq!(ease_in_expo(0.0, 1.0, INF, ID), INF);
        assert_bits(ease_in_expo(0.0, 1.0, -INF, ID), 0.0);
        assert_eq!(ease_out_expo(0.0, 1.0, INF, ID), 1.0);
        assert_eq!(ease_out_expo(0.0, 1.0, -INF, ID), -INF);
        assert!(ease_in_circ(0.0, 1.0, INF, ID).is_nan());
    }

    #[test]
    fn zero_and_negative_zero_t_give_the_start() {
        for (name, ease, _) in CURVES {
            for t in [0.0_f32, -0.0] {
                let result = ease(1.0, 5.0, t, ID);
                let tolerance =
                    if name.contains("expo") || name.contains("back") || name.contains("sine") {
                        5e-3
                    } else {
                        1e-6
                    };
                assert!(
                    within(result, 1.0, tolerance),
                    "ease_{name}(1, 5, {t}) = {result}"
                );
            }
        }
    }

    #[test]
    fn t_equal_to_one_gives_the_end_for_every_easing() {
        for (name, ease, _) in CURVES {
            let result = ease(1.0, 5.0, 1.0, ID);
            let tolerance =
                if name.contains("expo") || name.contains("back") || name.contains("sine") {
                    5e-3
                } else {
                    1e-6
                };
            assert!(
                within(result, 5.0, tolerance),
                "ease_{name}(1, 5, 1) = {result}"
            );
        }
    }

    #[test]
    fn elastic_endpoints_return_start_and_end_without_evaluating_the_curve() {
        for ease in [ease_in_elastic, ease_out_elastic, ease_in_out_elastic] {
            assert_bits(ease(1.0, 5.0, 0.0, ID), 1.0);
            assert_bits(ease(1.0, 5.0, -0.0, ID), 1.0);
            assert_bits(ease(1.0, 5.0, 1.0, ID), 5.0);
            assert_bits(ease(-0.0, 5.0, 0.0, ID), -0.0);
            assert_bits(ease(1.0, 5.0, 1.0, AFFINE), 11.0);
            assert_bits(ease(1.0, 5.0, 0.0, AFFINE), 3.0);
        }
    }

    #[test]
    fn elastic_end_point_is_computed_on_x86_64_and_exact_on_arm64() {
        let (start, end) = (0.3_f32, 0.1_f32);
        let computed = start + (end - start);
        assert_ne!(computed, end);
        for ease in [ease_in_elastic, ease_out_elastic, ease_in_out_elastic] {
            assert_bits(ease(start, end, 1.0, ID), per_arch(computed, end));
            assert_bits(ease(start, end, 0.0, ID), start);
        }
    }

    #[test]
    fn expo_easings_have_no_endpoint_special_cases() {
        assert_eq!(ease_in_expo(1.0, 5.0, 0.0, ID), 1.003_906_25);
        assert_eq!(ease_out_expo(1.0, 5.0, 1.0, ID), 4.996_093_75);
        assert_eq!(ease_in_out_expo(1.0, 5.0, 0.0, ID), 1.001_953_12);
        assert_eq!(ease_in_out_expo(1.0, 5.0, 1.0, ID), 4.998_046_9);
        assert_eq!(ease_out_expo(1.0, 5.0, 0.0, ID), 1.0);
        assert_eq!(ease_in_expo(1.0, 5.0, 1.0, ID), 5.0);
    }

    #[test]
    fn every_in_out_curve_passes_through_the_midpoint_exactly_or_closely() {
        for (name, ease, _) in CURVES {
            if name.starts_with("in_out") {
                close(ease(1.0, 5.0, 0.5, ID), 3.0, 1e-3);
            }
        }
    }

    #[test]
    fn in_and_out_variants_mirror_each_other_for_the_polynomial_curves() {
        for t in [0.0_f32, 0.25, 0.5, 0.75, 1.0] {
            assert_eq!(
                ease_out_quad(0.0, 1.0, t, ID),
                1.0 - ease_in_quad(0.0, 1.0, 1.0 - t, ID)
            );
            assert_eq!(
                ease_out_cubic(0.0, 1.0, t, ID),
                1.0 - ease_in_cubic(0.0, 1.0, 1.0 - t, ID)
            );
            assert_eq!(
                ease_out_quart(0.0, 1.0, t, ID),
                1.0 - ease_in_quart(0.0, 1.0, 1.0 - t, ID)
            );
            assert_eq!(
                ease_out_quint(0.0, 1.0, t, ID),
                1.0 - ease_in_quint(0.0, 1.0, 1.0 - t, ID)
            );
        }
    }

    #[test]
    fn bounce_segments_are_continuous_at_their_thresholds() {
        for threshold in [1.0_f32 / 2.75, 2.0 / 2.75, 2.5 / 2.75] {
            let below = ease_out_bounce(0.0, 1.0, f32::from_bits(threshold.to_bits() - 4), ID);
            let above = ease_out_bounce(0.0, 1.0, f32::from_bits(threshold.to_bits() + 4), ID);
            close(below, above, 1e-4);
        }
        close(ease_out_bounce(0.0, 1.0, 1.0, ID), 1.0, 1e-6);
        for t in T {
            close(
                ease_in_bounce(0.0, 1.0, t, ID),
                1.0 - ease_out_bounce(0.0, 1.0, 1.0 - t, ID),
                1e-6,
            );
        }
    }

    #[test]
    fn back_easings_overshoot_their_range() {
        assert!(ease_in_back(0.0, 1.0, 0.25, ID) < 0.0);
        assert!(ease_out_back(0.0, 1.0, 0.75, ID) > 1.0);
        assert!(ease_in_out_back(0.0, 1.0, 0.25, ID) < 0.0);
        assert!(ease_in_out_back(0.0, 1.0, 0.75, ID) > 1.0);
    }

    #[test]
    fn the_in_out_elastic_curve_is_not_the_common_formula() {
        close(ease_in_out_elastic(1.0, 5.0, 0.3, ID), 0.937_503_457, 1e-6);
        assert!((ease_in_out_elastic(1.0, 5.0, 0.3, ID) - 1.095_753_8).abs() > 0.1);
    }

    #[test]
    fn easing_endpoints_and_special_cases() {
        assert_eq!(ease_in_expo(1.0, 5.0, 0.0, ID), 1.003_906_25);
        assert_eq!(ease_out_expo(1.0, 5.0, 1.0, ID), 4.996_093_75);
        assert_eq!(ease_in_out_expo(1.0, 5.0, 0.0, ID), 1.001_953_12);
        for ease in [ease_in_elastic, ease_out_elastic, ease_in_out_elastic] {
            assert_eq!(ease(1.0, 5.0, 0.0, ID), 1.0);
            assert_eq!(ease(1.0, 5.0, 1.0, ID), 5.0);
            assert_eq!(ease(1.0, 5.0, -0.0, ID), 1.0);
        }
        close(ease_in_out_elastic(1.0, 5.0, 0.3, ID), 0.937_503_457, 1e-6);
        assert_eq!(ease_in_quad(0.0, 1.0, 2.0, ID), 4.0);
        assert_eq!(ease_in_cubic(0.0, 1.0, -2.0, ID), -8.0);
        assert_eq!(ease_out_quad(0.0, 1.0, 2.0, ID), 0.0);
        assert_eq!(ease_in_quad(0.0, 1.0, 2.0, PostOp::new(2.0, 1.0)), 9.0);
        assert_eq!(ease_in_elastic(1.0, 5.0, 1.0, PostOp::new(2.0, 1.0)), 11.0);
    }

    #[test]
    fn the_two_architectures_use_different_easing_formulas() {
        // x86-64 rounds c·t·t·t·t in order; arm64 squares t² with the last step rounded once.
        assert_eq!(
            ease_in_quart(0.0, 1.0, 0.1, ID).to_bits(),
            per_arch(0x38d1_b718, 0x38d1_b719)
        );
        if ARCH == Arch::X86_64 {
            assert_eq!(
                ease_in_quart(0.0, 1.0, 0.1, ID),
                (((1.0_f32 * 0.1) * 0.1) * 0.1) * 0.1
            );
        }
        // The two elastic angle formulas can pick neighbouring table entries.
        if ARCH == Arch::X86_64 {
            assert_eq!(ease_in_elastic(1.0, 5.0, 0.9, ID).to_bits(), 0x386a_4000);
        }
        if ARCH == Arch::Arm64 {
            assert_ne!(ease_in_elastic(1.0, 5.0, 0.9, ID).to_bits(), 0x386a_4000);
        }
        let (start, end) = (0.3_f32, 0.1_f32);
        assert_ne!(start + (end - start), end);
        for ease in [ease_in_elastic, ease_out_elastic, ease_in_out_elastic] {
            assert_eq!(
                ease(start, end, 1.0, ID),
                per_arch(start + (end - start), end)
            );
        }
        // Rounding in formula order keeps what the reassociated form loses.
        if ARCH == Arch::X86_64 {
            assert_eq!(ease_in_quad(-0.0, 0.0, -1.0e38, ID), 0.0);
        }
        if ARCH == Arch::Arm64 {
            assert!(ease_in_quad(-0.0, 0.0, -1.0e38, ID).is_nan());
        }
        if ARCH == Arch::X86_64 {
            assert_eq!(
                ease_in_out_quad(1.0, 5.0, f32::INFINITY, ID),
                f32::NEG_INFINITY
            );
        }
    }

    #[test]
    fn every_easing_runs_from_start_to_end() {
        let exact: [(&str, Ease); 21] = [
            ("in_quad", ease_in_quad),
            ("out_quad", ease_out_quad),
            ("in_out_quad", ease_in_out_quad),
            ("in_cubic", ease_in_cubic),
            ("out_cubic", ease_out_cubic),
            ("in_out_cubic", ease_in_out_cubic),
            ("in_quart", ease_in_quart),
            ("out_quart", ease_out_quart),
            ("in_out_quart", ease_in_out_quart),
            ("in_quint", ease_in_quint),
            ("out_quint", ease_out_quint),
            ("in_out_quint", ease_in_out_quint),
            ("in_circ", ease_in_circ),
            ("out_circ", ease_out_circ),
            ("in_out_circ", ease_in_out_circ),
            ("in_bounce", ease_in_bounce),
            ("out_bounce", ease_out_bounce),
            ("in_out_bounce", ease_in_out_bounce),
            ("in_elastic", ease_in_elastic),
            ("out_elastic", ease_out_elastic),
            ("in_out_elastic", ease_in_out_elastic),
        ];
        // Table quantisation, expo's 2^-10 residue and the rounded back constants miss the
        // endpoints slightly.
        let approximate: [(&str, Ease); 9] = [
            ("in_sine", ease_in_sine),
            ("out_sine", ease_out_sine),
            ("in_out_sine", ease_in_out_sine),
            ("in_expo", ease_in_expo),
            ("out_expo", ease_out_expo),
            ("in_out_expo", ease_in_out_expo),
            ("in_back", ease_in_back),
            ("out_back", ease_out_back),
            ("in_out_back", ease_in_out_back),
        ];
        for (name, ease) in exact {
            assert!(within(ease(1.0, 5.0, 0.0, ID), 1.0, 1e-6), "{name} at 0");
            assert!(within(ease(1.0, 5.0, 1.0, ID), 5.0, 1e-6), "{name} at 1");
            assert!(ease(1.0, 5.0, NAN, ID).is_nan(), "{name} of NaN");
        }
        for (name, ease) in approximate {
            assert!(within(ease(1.0, 5.0, 0.0, ID), 1.0, 5e-3), "{name} at 0");
            assert!(within(ease(1.0, 5.0, 1.0, ID), 5.0, 5e-3), "{name} at 1");
        }
        let in_out: [(&str, Ease); 9] = [
            ("quad", ease_in_out_quad),
            ("cubic", ease_in_out_cubic),
            ("quart", ease_in_out_quart),
            ("quint", ease_in_out_quint),
            ("sine", ease_in_out_sine),
            ("expo", ease_in_out_expo),
            ("circ", ease_in_out_circ),
            ("bounce", ease_in_out_bounce),
            ("back", ease_in_out_back),
        ];
        for (name, ease) in in_out {
            assert!(
                within(ease(1.0, 5.0, 0.5, ID), 3.0, 1e-3),
                "in_out_{name} at 0.5"
            );
        }
    }

    #[test]
    fn sine_easings_are_quantised_by_the_sine_table() {
        let scale = f32::from_bits(0x4622_f983);
        let step_angle = f32::from_bits(0x38c9_0fdb);
        let half_pi = f32::from_bits(0x3fc9_0fdb);
        let mut differs_from_plain_sine = false;
        for step in 1..100_u8 {
            let t = f32::from(step) / 100.0;
            let radians = t * half_pi;
            let index = ((radians * scale) as i32 & 0xffff) as f32;
            assert_eq!(
                ease_out_sine(0.0, 1.0, t, ID),
                per_arch(
                    transcendental::sin(index / scale, NAN),
                    transcendental::sin(index * step_angle, NAN)
                )
            );
            let eased = ease_out_sine(0.0, 1.0, t, ID);
            close(eased, transcendental::sin(radians, NAN), 1e-4);
            differs_from_plain_sine |= eased != transcendental::sin(radians, NAN);
        }
        assert!(
            differs_from_plain_sine,
            "the table sine should not equal the plain sine everywhere"
        );
    }
}
