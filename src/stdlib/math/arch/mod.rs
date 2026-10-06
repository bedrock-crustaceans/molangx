//! The standard functions' bodies per architecture: `x86_64` and `arm64` hold free functions with
//! the same names and signatures, and this module re-exports those of the build's architecture. The
//! unit tests compile both.

#[cfg(any(target_arch = "aarch64", test))]
#[cfg_attr(not(target_arch = "aarch64"), allow(dead_code))]
pub(crate) mod arm64;
#[cfg(any(not(target_arch = "aarch64"), test))]
#[cfg_attr(target_arch = "aarch64", allow(dead_code))]
pub(crate) mod x86_64;

#[cfg(target_arch = "aarch64")]
pub(crate) use arm64::*;
#[cfg(not(target_arch = "aarch64"))]
pub(crate) use x86_64::*;

/// 65536/2π.
const SIN_INDEX_SCALE: f32 = f32::from_bits(0x4622_f983);

/// 2π/65536.
#[cfg(any(target_arch = "aarch64", test))]
const SIN_INDEX_STEP: f32 = f32::from_bits(0x38c9_0fdb);

const HALF_PI: f32 = f32::from_bits(0x3fc9_0fdb);

/// The bounce curve's `n1`.
const BOUNCE_N1: f32 = 7.5625;
/// The bounce thresholds `1/2.75`, `2/2.75`, `2.5/2.75`.
const BOUNCE_T1: f32 = f32::from_bits(0x3eba_2e8c);
const BOUNCE_T2: f32 = f32::from_bits(0x3f3a_2e8c);
const BOUNCE_T3: f32 = f32::from_bits(0x3f68_ba2f);
/// The bounce centres `1.5/2.75`, `2.25/2.75`, `2.625/2.75`.
const BOUNCE_C2: f32 = f32::from_bits(0x3f0b_a2e9);
const BOUNCE_C3: f32 = f32::from_bits(0x3f51_745d);
const BOUNCE_C4: f32 = f32::from_bits(0x3f74_5d17);

/// The back constants `c1 = 1.70158`, `c3 = c1 + 1`, `c2 = c1·1.525` and `c2 + 1`.
const BACK_C1: f32 = f32::from_bits(0x3fd9_cd60);
const BACK_C3: f32 = f32::from_bits(0x402c_e6b0);
const BACK_C2: f32 = f32::from_bits(0x4026_12ff);
const BACK_C2_PLUS_1: f32 = f32::from_bits(0x4066_12ff);

/// How x86-64 sorts random bounds, which decides where a NaN bound ends up; arm64 drops a NaN
/// bound.
#[derive(Copy, Clone)]
pub(crate) enum Sort {
    /// A NaN in either position survives in one of the two bounds.
    RunTime,
    /// Two literal bounds: a NaN in either position gives the first operand for both.
    Literal,
}

#[cfg(test)]
pub(crate) mod test_support {
    use crate::numeric::PostOp;

    /// An easing body of an architecture module.
    pub(crate) type Body = fn(f32, f32, f32) -> f32;
    /// A public `math::ease_*` function.
    pub(crate) type Public = fn(f32, f32, f32, PostOp) -> f32;

    /// The 30 easing bodies of the architecture module `$arch` in op order, each with its name and
    /// its public function.
    macro_rules! easing_bodies {
        ($arch:ident) => {{
            use crate::stdlib::math::arch::test_support::{Body, Public};
            use crate::stdlib::math::{self, arch::$arch as m};
            let bodies: [(&str, Body, Public); 30] = [
                ("in_quad", m::in_quad, math::ease_in_quad),
                ("out_quad", m::out_quad, math::ease_out_quad),
                ("in_out_quad", m::in_out_quad, math::ease_in_out_quad),
                ("in_cubic", m::in_cubic, math::ease_in_cubic),
                ("out_cubic", m::out_cubic, math::ease_out_cubic),
                ("in_out_cubic", m::in_out_cubic, math::ease_in_out_cubic),
                ("in_quart", m::in_quart, math::ease_in_quart),
                ("out_quart", m::out_quart, math::ease_out_quart),
                ("in_out_quart", m::in_out_quart, math::ease_in_out_quart),
                ("in_quint", m::in_quint, math::ease_in_quint),
                ("out_quint", m::out_quint, math::ease_out_quint),
                ("in_out_quint", m::in_out_quint, math::ease_in_out_quint),
                ("in_sine", m::in_sine, math::ease_in_sine),
                ("out_sine", m::out_sine, math::ease_out_sine),
                ("in_out_sine", m::in_out_sine, math::ease_in_out_sine),
                ("in_expo", m::in_expo, math::ease_in_expo),
                ("out_expo", m::out_expo, math::ease_out_expo),
                ("in_out_expo", m::in_out_expo, math::ease_in_out_expo),
                ("in_circ", m::in_circ, math::ease_in_circ),
                ("out_circ", m::out_circ, math::ease_out_circ),
                ("in_out_circ", m::in_out_circ, math::ease_in_out_circ),
                ("in_bounce", m::in_bounce, math::ease_in_bounce),
                ("out_bounce", m::out_bounce, math::ease_out_bounce),
                ("in_out_bounce", m::in_out_bounce, math::ease_in_out_bounce),
                ("in_back", m::in_back, math::ease_in_back),
                ("out_back", m::out_back, math::ease_out_back),
                ("in_out_back", m::in_out_back, math::ease_in_out_back),
                ("in_elastic", m::in_elastic, math::ease_in_elastic),
                ("out_elastic", m::out_elastic, math::ease_out_elastic),
                (
                    "in_out_elastic",
                    m::in_out_elastic,
                    math::ease_in_out_elastic,
                ),
            ];
            bodies
        }};
    }

    pub(crate) use easing_bodies;
}

#[cfg(test)]
mod tests {
    use super::test_support::easing_bodies;
    use super::*;
    use crate::numeric::{ARCH, Arch, PostOp, arch::both, test_support::*};
    use crate::stdlib::math::{self, PI, transcendental};

    #[test]
    fn each_architecture_chooses_the_nan_of_each_invalid_operation() {
        let mut got = Vec::new();
        both!(m => {
            got.push([m::sin(INF), m::cos(-INF), m::ln(-1.0), m::ln(-INF), m::pow(-8.0, 1.0 / 3.0), m::sqrt(-1.0)].map(f32::to_bits));
        });
        let x86_64 = [
            0xffc0_0000,
            0xffc0_0000,
            0x7fc0_0000,
            0x7fc0_0000,
            0xffc0_0000,
            0xffc0_0000,
        ];
        let arm64 = [
            0xffc0_0000,
            0xffc0_0000,
            0xffc0_0000,
            0xffc0_0000,
            0xffc0_0000,
            0x7fc0_0000,
        ];
        assert_eq!(got, [x86_64, arm64]);
        let public = [
            math::sin(INF, ID),
            math::cos(-INF, ID),
            math::ln(-1.0, ID),
            math::ln(-INF, ID),
            math::pow(-8.0, 1.0 / 3.0, ID),
            math::sqrt(-1.0, ID),
        ];
        assert_eq!(public.map(f32::to_bits), per_arch(x86_64, arm64));
    }

    #[test]
    fn a_nan_argument_comes_back_quiet_on_both_architectures() {
        let negative_signalling = f32::from_bits(0xff80_0003);
        both!(m => {
            assert_eq!(m::ln(negative_signalling).to_bits(), 0xffc0_0003);
            assert_eq!(m::sqrt(negative_signalling).to_bits(), 0xffc0_0003);
            assert_eq!(m::sin(negative_signalling).to_bits(), 0xffc0_0003);
            assert_eq!(m::cos(negative_signalling).to_bits(), 0xffc0_0003);
            assert_eq!(m::pow(negative_signalling, 2.0).to_bits(), 0xffc0_0003);
            assert_eq!(m::pow(negative_signalling, 3.0).to_bits(), 0x7fc0_0003);
            assert_eq!(m::pow(2.0, negative_signalling).to_bits(), 0xffc0_0003);
            assert_eq!(m::out_circ(0.0, 1.0, negative_signalling).to_bits() & 0x7fff_ffff, 0x7fc0_0003);
        });
    }

    #[test]
    fn the_architectures_choose_different_nans() {
        let (quiet, negative_quiet) = (f32::from_bits(0x7fc0_0001), f32::from_bits(0xffc0_0002));
        let mut got = Vec::new();
        both!(m => {
            got.push([m::lerp(quiet, negative_quiet, 0.5), m::in_circ(1.0, 5.0, quiet)].map(f32::to_bits));
        });
        assert_eq!(
            got,
            [[0xffc0_0002, 0x7fc0_0001], [0x7fc0_0001, 0xffc0_0001]]
        );
    }

    #[test]
    fn ordinary_bounds_sort_and_interpolate_alike() {
        both!(m => {
            for sort in [Sort::RunTime, Sort::Literal] {
                assert_eq!(m::sorted_bounds(sort, 5.0, 1.0), (1.0, 5.0));
                assert_eq!(m::sorted_bounds(sort, -1.0, 2.0), (-1.0, 2.0));
            }
            assert_bits(m::interpolate(1.0, 5.0, 0.0), 1.0);
            assert_bits(m::interpolate(1.0, 5.0, 0.5), 3.0);
            assert_bits(m::interpolate(1.0, 5.0, 1.0), 5.0);
            assert_bits(m::integer_draw(1.0, 6.0, m::integer_reach(1.0, 6.0), 0.0), 1.0);
            assert_bits(m::integer_draw(1.0, 6.0, m::integer_reach(1.0, 6.0), 1.0), 6.0);
            assert_bits(m::integer_draw(1.0, 6.0, m::integer_reach_const(1.0, 6.0), 0.5), 3.0);
            assert_bits(m::roll(2.0, 1.0, 5.0, 0.5), 5.0);
        });
    }

    #[test]
    fn the_formulas_agree_on_exact_operands() {
        both!(m => {
            assert_bits(m::inverse_trig_argument(1.0004), 1.0);
            assert_bits(m::inverse_trig_argument(-1.0004), -1.0);
            assert_bits(m::inverse_trig_argument(1.001), 1.001);
            assert_bits(m::degrees(0.0, PostOp::IDENTITY), 0.0);
            assert_bits(m::degrees(0.0, AFFINE), 1.0);
            assert_bits(m::hermite_blend(0.5), 0.5);
            assert_bits(m::hermite_blend(1.0), 1.0);
            assert_bits(m::lerp(1.0, 5.0, 0.5), 3.0);
            assert_bits(m::lerprotate(350.0, 10.0, 0.5), 360.0);
            assert_bits(m::ln(1.0), 0.0);
            assert_eq!(m::ln(0.0), -INF);
            assert_bits(m::sin_table_entry(0), 0.0);
            assert_bits(m::table_sin(0.0), 0.0);
            close(m::table_cos(0.0), 1.0, 1e-6);
        });
    }

    #[test]
    fn every_easing_starts_at_the_start_and_ends_near_the_end() {
        for bodies in [easing_bodies!(x86_64), easing_bodies!(arm64)] {
            for (_, body, _) in bodies {
                close(body(1.0, 5.0, 0.0), 1.0, 5e-3);
                close(body(1.0, 5.0, 1.0), 5.0, 5e-3);
            }
        }
    }

    #[test]
    fn the_public_easings_call_the_bodies_of_the_build() {
        for (name, body, public) in per_arch(easing_bodies!(x86_64), easing_bodies!(arm64)) {
            for (start, end) in [
                (1.0_f32, 5.0),
                (-2.5, 7.25),
                (0.0, 0.0),
                (-0.0, 1.0e-45),
                (3.0, -4.0),
            ] {
                for t in [
                    0.0_f32, -0.0, 0.1, 0.25, 0.3, 0.5, 0.75, 0.999, 1.0, 1.5, -0.5, 10.0, NAN, INF,
                ] {
                    let (via_public, direct) = (public(start, end, t, ID), body(start, end, t));
                    assert_eq!(
                        via_public.to_bits(),
                        direct.to_bits(),
                        "ease_{name}({start}, {end}, {t})"
                    );
                }
            }
        }
    }

    #[test]
    fn table_constants_are_the_documented_bit_patterns() {
        assert_eq!(SIN_INDEX_SCALE.to_bits(), 0x4622_f983);
        assert_eq!(SIN_INDEX_STEP.to_bits(), 0x38c9_0fdb);
        assert_eq!(HALF_PI.to_bits(), 0x3fc9_0fdb);
        assert_eq!(PI.to_bits(), 0x4049_0fdb);
        close(SIN_INDEX_SCALE, 10_430.378, 1e-3);
        close(SIN_INDEX_STEP, 9.587_38e-5, 1e-9);
        close(SIN_INDEX_SCALE * SIN_INDEX_STEP, 1.0, 1e-6);
    }

    #[test]
    fn sin_table_entry_is_sin_of_the_index_times_the_step() {
        for index in [0_i32, 1, 100, 4096, 16_384, 32_768, 49_152, 65_535] {
            let i = index as f32;
            assert_bits(
                sin_table_entry(index),
                per_arch(
                    transcendental::sin(i / SIN_INDEX_SCALE, NAN),
                    transcendental::sin(i * SIN_INDEX_STEP, NAN),
                ),
            );
        }
    }

    #[test]
    fn sin_table_entry_at_the_quadrant_boundaries() {
        assert_bits(sin_table_entry(0), 0.0);
        close(sin_table_entry(16_384), 1.0, 1e-6);
        close(sin_table_entry(32_768), 0.0, 2e-5);
        close(sin_table_entry(49_152), -1.0, 1e-6);
        close(
            sin_table_entry(8_192),
            std::f32::consts::FRAC_1_SQRT_2,
            1e-5,
        );
        close(sin_table_entry(65_535), 0.0, 2e-4);
    }

    #[test]
    fn sin_table_entry_masks_the_index_to_sixteen_bits() {
        for index in [0_i32, 1, 5, 1000, 40_000, 65_535] {
            assert_bits(sin_table_entry(index + 65_536), sin_table_entry(index));
            assert_bits(sin_table_entry(index + 3 * 65_536), sin_table_entry(index));
            assert_bits(sin_table_entry(index - 65_536), sin_table_entry(index));
        }
        assert_bits(sin_table_entry(-1), sin_table_entry(65_535));
        assert_bits(sin_table_entry(-16_384), sin_table_entry(49_152));
        assert_bits(sin_table_entry(i32::MIN), sin_table_entry(0));
        assert_bits(sin_table_entry(i32::MAX), sin_table_entry(65_535));
    }

    #[test]
    fn the_two_architectures_compute_table_entries_by_different_formulas() {
        let differing = (0..65_536)
            .filter(|&i| {
                x86_64::sin_table_entry(i).to_bits() != arm64::sin_table_entry(i).to_bits()
            })
            .count();
        assert!(
            differing > 0,
            "the multiplied and the divided table should differ somewhere"
        );
        for i in (0..65_536).step_by(97) {
            close(x86_64::sin_table_entry(i), arm64::sin_table_entry(i), 2e-6);
        }
    }

    #[test]
    fn table_sin_truncates_the_scaled_angle() {
        assert_bits(table_sin(0.0), 0.0);
        let angle = 1.5 / SIN_INDEX_SCALE;
        assert_bits(table_sin(angle), sin_table_entry(1));
        close(table_sin(HALF_PI), 1.0, 1e-4);
        close(table_sin(PI), 0.0, 2e-4);
        let small = 3.5 / SIN_INDEX_SCALE;
        assert_bits(table_sin(-small), sin_table_entry(-3));
        close(table_sin(-HALF_PI), -1.0, 1e-4);
    }

    #[test]
    fn table_sin_beyond_the_int_range_differs_between_the_architectures() {
        if ARCH == Arch::X86_64 {
            assert_bits(table_sin(1.0e6), sin_table_entry(0));
            assert_bits(table_sin(INF), sin_table_entry(0));
        }
        if ARCH == Arch::Arm64 {
            assert_bits(table_sin(1.0e6), sin_table_entry(65_535));
            assert_bits(table_sin(INF), sin_table_entry(65_535));
        }
        assert_bits(
            table_sin(-INF),
            per_arch(x86_64::sin_table_entry(0), arm64::sin_table_entry(0)),
        );
        assert_bits(table_sin(NAN), 0.0);
    }

    #[test]
    fn table_cos_is_the_table_shifted_by_a_quarter_turn() {
        assert_bits(table_cos(0.0), sin_table_entry(16_384));
        close(table_cos(0.0), 1.0, 1e-6);
        close(table_cos(PI), -1.0, 1e-4);
        close(table_cos(HALF_PI), 0.0, 2e-4);
        close(table_cos(-PI), -1.0, 1e-4);
        let angle = 1.5 / SIN_INDEX_SCALE;
        assert_bits(table_cos(angle), sin_table_entry(16_385));
        assert_bits(table_cos(-5.0 * HALF_PI), table_cos(-HALF_PI));
    }

    #[test]
    fn table_cos_of_non_finite_angles() {
        assert_bits(table_cos(NAN), 0.0);
        assert_bits(
            table_cos(INF),
            per_arch(x86_64::sin_table_entry(0), arm64::sin_table_entry(65_535)),
        );
        if ARCH == Arch::Arm64 {
            assert_bits(table_cos(-INF), sin_table_entry(0));
        }
    }
}
