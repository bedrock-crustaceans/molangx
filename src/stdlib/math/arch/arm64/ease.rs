//! The `Arm64` easings: the Penner curves with products fused into multiply-adds.
//!
//! The NaN of each arithmetic operation is chosen by [`arm64`](crate::numeric::arch::arm64),
//! operands in the order written.

use super::{sqrt, table_cos, table_sin};
use crate::numeric::arch::arm64::{add, mul, mul_add, mul_sub, neg_mul_add, neg_mul_sub, sub};
use crate::stdlib::math::{
    PI,
    arch::{
        BACK_C1, BACK_C2, BACK_C2_PLUS_1, BACK_C3, BOUNCE_C2, BOUNCE_C3, BOUNCE_C4, BOUNCE_N1,
        BOUNCE_T1, BOUNCE_T2, BOUNCE_T3, HALF_PI,
    },
    transcendental,
};

#[inline]
pub(crate) fn in_quad(start: f32, end: f32, t: f32) -> f32 {
    let d = sub(end, start);
    mul_add(mul(t, t), d, start)
}

/// `out_quad`: `f(t) = 1 − (1 − t)²`, computed as `−c·t·(t − 2) + start`.
#[inline]
pub(crate) fn out_quad(start: f32, end: f32, t: f32) -> f32 {
    let x = mul(sub(start, end), t);
    mul_add(x, add(t, -2.0), start)
}

/// `in_out_quad`: with `u = 2t`, `u >= 1 ? −c/2·((u − 1)(u − 3) − 1) : c/2·u²`.
#[inline]
pub(crate) fn in_out_quad(start: f32, end: f32, t: f32) -> f32 {
    let u = add(t, t);
    let a = add(u, -3.0);
    let d = sub(end, start);
    let neg_half = mul(d, -0.5);
    let half = mul(d, 0.5);
    let r = if u >= 1.0 {
        // a − u·a = −(u − 1)(u − 3), then −(x·(−c/2)) − (−c/2).
        let x = mul_sub(u, a, a);
        neg_mul_add(x, neg_half, neg_half)
    } else {
        mul(mul(u, u), half)
    };
    add(r, start)
}

#[inline]
pub(crate) fn in_cubic(start: f32, end: f32, t: f32) -> f32 {
    let x = mul(mul(t, t), sub(end, start));
    mul_add(x, t, start)
}

#[inline]
pub(crate) fn out_cubic(start: f32, end: f32, t: f32) -> f32 {
    let d = sub(end, start);
    let u = add(t, -1.0);
    let cube = mul(mul(u, u), u);
    add(mul_add(cube, d, d), start)
}

#[inline]
pub(crate) fn in_out_cubic(start: f32, end: f32, t: f32) -> f32 {
    let u = add(t, t);
    let a = add(u, -2.0);
    let half = mul(sub(end, start), 0.5);
    let r = if u >= 1.0 {
        mul(half, mul_add(mul(a, a), a, 2.0))
    } else {
        mul(mul(mul(u, u), half), u)
    };
    add(r, start)
}

#[inline]
pub(crate) fn in_quart(start: f32, end: f32, t: f32) -> f32 {
    let tt = mul(t, t);
    let d = sub(end, start);
    mul_add(mul(tt, tt), d, start)
}

#[inline]
pub(crate) fn out_quart(start: f32, end: f32, t: f32) -> f32 {
    let neg_d = sub(start, end);
    let u = add(t, -1.0);
    let u2 = mul(u, u);
    add(neg_mul_sub(mul(u2, u2), neg_d, neg_d), start)
}

#[inline]
pub(crate) fn in_out_quart(start: f32, end: f32, t: f32) -> f32 {
    let u = add(t, t);
    let d = sub(end, start);
    let a = add(u, -2.0);
    let aa = mul(a, a);
    let sq = mul(u, u);
    let r = if u >= 1.0 {
        mul(mul(d, -0.5), mul_add(aa, aa, -2.0))
    } else {
        mul(mul(sq, sq), mul(d, 0.5))
    };
    add(r, start)
}

#[inline]
pub(crate) fn in_quint(start: f32, end: f32, t: f32) -> f32 {
    let tt = mul(t, t);
    let x = mul(sub(end, start), t);
    mul_add(mul(tt, tt), x, start)
}

#[inline]
pub(crate) fn out_quint(start: f32, end: f32, t: f32) -> f32 {
    let d = sub(end, start);
    let u = add(t, -1.0);
    let u2 = mul(u, u);
    let u5 = mul(mul(u2, u2), u);
    add(mul_add(u5, d, d), start)
}

#[inline]
pub(crate) fn in_out_quint(start: f32, end: f32, t: f32) -> f32 {
    let u = add(t, t);
    let a = add(u, -2.0);
    let half = mul(sub(end, start), 0.5);
    let sq = mul(u, u);
    let aa = mul(a, a);
    let r = if u >= 1.0 {
        mul(half, mul_add(mul(aa, aa), a, 2.0))
    } else {
        mul(mul(sq, sq), mul(half, u))
    };
    add(r, start)
}

/// `in_sine`: `f(t) = 1 − cos(t·π/2)` with the cosine from the sine table, computed as
/// `end + cos·(start − end)`.
#[inline]
pub(crate) fn in_sine(start: f32, end: f32, t: f32) -> f32 {
    let c = table_cos(t * HALF_PI);
    mul_add(c, sub(start, end), end)
}

#[inline]
pub(crate) fn out_sine(start: f32, end: f32, t: f32) -> f32 {
    let s = table_sin(t * HALF_PI);
    mul_add(s, sub(end, start), start)
}

#[inline]
pub(crate) fn in_out_sine(start: f32, end: f32, t: f32) -> f32 {
    let c = table_cos(t * PI);
    let neg_half = mul(sub(start, end), 0.5);
    add(neg_mul_sub(c, neg_half, neg_half), start)
}

#[inline]
pub(crate) fn in_expo(start: f32, end: f32, t: f32) -> f32 {
    let p = transcendental::exp2(mul_add(t, 10.0, -10.0));
    mul_add(p, sub(end, start), start)
}

#[inline]
pub(crate) fn out_expo(start: f32, end: f32, t: f32) -> f32 {
    let d = sub(end, start);
    let p = transcendental::exp2(mul(t, -10.0));
    add(mul_sub(p, d, d), start)
}

#[inline]
pub(crate) fn in_out_expo(start: f32, end: f32, t: f32) -> f32 {
    let v = if t + t >= 1.0 {
        sub(2.0, transcendental::exp2(mul_add(t, -20.0, 10.0)))
    } else {
        transcendental::exp2(mul_add(t, 20.0, -10.0))
    };
    mul_add(sub(end, start), mul(0.5, v), start)
}

#[inline]
pub(crate) fn in_circ(start: f32, end: f32, t: f32) -> f32 {
    let neg_d = sub(start, end);
    let q = sqrt(mul_sub(t, t, 1.0));
    add(neg_mul_sub(q, neg_d, neg_d), start)
}

#[inline]
pub(crate) fn out_circ(start: f32, end: f32, t: f32) -> f32 {
    let u = add(t, -1.0);
    let q = sqrt(mul_sub(u, u, 1.0));
    mul_add(q, sub(end, start), start)
}

#[inline]
pub(crate) fn in_out_circ(start: f32, end: f32, t: f32) -> f32 {
    let u = add(t, t);
    let a = add(u, -2.0);
    let d = sub(end, start);
    let (square, k, factor) = if u >= 1.0 {
        (mul(a, a), 1.0, mul(d, 0.5))
    } else {
        (mul(u, u), -1.0, mul(d, -0.5))
    };
    let q = add(sqrt(sub(1.0, square)), k);
    mul_add(factor, q, start)
}

/// `1 − centre` of each piece, rounded once, for the mirrored curve.
const BOUNCE_M2: f32 = f32::from_bits(0x3ee8_ba2e);
const BOUNCE_M3: f32 = f32::from_bits(0x3e3a_2e8c);
const BOUNCE_M4: f32 = f32::from_bits(0x3d3a_2e90);
/// `1 + centre` of each piece, rounded once, for the second in-out half.
const BOUNCE_P2: f32 = f32::from_bits(0x3fc5_d174);
const BOUNCE_P3: f32 = f32::from_bits(0x3fe8_ba2e);
const BOUNCE_P4: f32 = f32::from_bits(0x3ffa_2e8c);

/// `n1·x² + k`.
#[inline]
fn bounce_piece(x: f32, k: f32) -> f32 {
    mul_add(mul(x, x), BOUNCE_N1, k)
}

#[inline]
fn bounce_out(t: f32) -> f32 {
    if t >= BOUNCE_T1 {
        if t >= BOUNCE_T2 {
            if t >= BOUNCE_T3 {
                bounce_piece(add(t, -BOUNCE_C4), 0.984_375)
            } else {
                bounce_piece(add(t, -BOUNCE_C3), 0.9375)
            }
        } else {
            bounce_piece(add(t, -BOUNCE_C2), 0.75)
        }
    } else {
        mul(mul(t, t), BOUNCE_N1)
    }
}

/// The out-bounce curve at `u = 1 − x`, each piece computed from `(1 − centre) − x`.
#[inline]
fn bounce_out_mirrored(u: f32, x: f32) -> f32 {
    if u >= BOUNCE_T1 {
        if u >= BOUNCE_T2 {
            if u >= BOUNCE_T3 {
                bounce_piece(sub(BOUNCE_M4, x), 0.984_375)
            } else {
                bounce_piece(sub(BOUNCE_M3, x), 0.9375)
            }
        } else {
            bounce_piece(sub(BOUNCE_M2, x), 0.75)
        }
    } else {
        mul(mul(u, u), BOUNCE_N1)
    }
}

#[inline]
pub(crate) fn out_bounce(start: f32, end: f32, t: f32) -> f32 {
    let v = bounce_out(t);
    mul_add(v, sub(end, start), start)
}

/// `in_bounce`: `1 − out_bounce(1 − t)`, computed as `end + out·(start − end)`.
#[inline]
pub(crate) fn in_bounce(start: f32, end: f32, t: f32) -> f32 {
    let v = bounce_out_mirrored(sub(1.0, t), t);
    mul_add(v, sub(start, end), end)
}

#[inline]
pub(crate) fn in_out_bounce(start: f32, end: f32, t: f32) -> f32 {
    let d = sub(end, start);
    let u2 = add(t, t);
    let x = if t >= 0.5 {
        let u = add(u2, -1.0);
        let v = if u >= BOUNCE_T1 {
            if u >= BOUNCE_T2 {
                if u >= BOUNCE_T3 {
                    bounce_piece(add(u2, -BOUNCE_P4), 0.984_375)
                } else {
                    bounce_piece(add(u2, -BOUNCE_P3), 0.9375)
                }
            } else {
                bounce_piece(add(u2, -BOUNCE_P2), 0.75)
            }
        } else {
            mul(mul(u, u), BOUNCE_N1)
        };
        mul_add(v, d, d)
    } else {
        let v = bounce_out_mirrored(sub(1.0, u2), u2);
        mul_sub(v, d, d)
    };
    mul_add(x, 0.5, start)
}

const BACK_C2_PLUS_1_TWICE: f32 = f32::from_bits(0x40e6_12ff);

#[inline]
pub(crate) fn in_back(start: f32, end: f32, t: f32) -> f32 {
    let x = mul(mul(t, t), sub(end, start));
    let y = mul_add(t, BACK_C3, -BACK_C1);
    mul_add(x, y, start)
}

#[inline]
pub(crate) fn out_back(start: f32, end: f32, t: f32) -> f32 {
    let d = sub(end, start);
    let u = add(t, -1.0);
    let z = mul(mul(u, u), mul_add(u, BACK_C3, BACK_C1));
    add(mul_add(z, d, d), start)
}

#[inline]
pub(crate) fn in_out_back(start: f32, end: f32, t: f32) -> f32 {
    let u = add(t, t);
    let half = mul(sub(end, start), 0.5);
    let a = add(u, -2.0);
    let r = if u >= 1.0 {
        let inner = mul_add(a, BACK_C2_PLUS_1, BACK_C2);
        mul(half, mul_add(mul(a, a), inner, 2.0))
    } else {
        mul(
            mul(mul(u, u), half),
            mul_add(t, BACK_C2_PLUS_1_TWICE, -BACK_C2),
        )
    };
    add(r, start)
}

/// `2π/0.3`.
const ELASTIC_C4: f32 = f32::from_bits(0x41a7_8d36);
/// `10.75·c4/10`, the phase of the in and in-out curves.
const ELASTIC_PHASE_IN: f32 = f32::from_bits(0x41b4_1e34);
/// `0.75·c4/10`, the phase of the out curve.
const ELASTIC_PHASE_OUT: f32 = f32::from_bits(0x3fc9_0fdc);
const ELASTIC_C4_TWICE: f32 = f32::from_bits(0x4227_8d36);

/// `in_elastic`: `t == 0` → start, `t == 1` → end, otherwise
/// `start − (end − start)·2^(10t − 10)·sin(c4·t − 22.514748)` with the sine from the table.
#[inline]
pub(crate) fn in_elastic(start: f32, end: f32, t: f32) -> f32 {
    if t == 0.0 {
        start
    } else if t == 1.0 {
        end
    } else {
        let p = transcendental::exp2(mul_add(t, 10.0, -10.0));
        let s = table_sin(mul_add(t, ELASTIC_C4, -ELASTIC_PHASE_IN));
        mul_add(mul(p, sub(start, end)), s, start)
    }
}

/// `out_elastic`: `t == 0` → start, `t == 1` → end, otherwise
/// `end + (end − start)·2^(−10t)·sin(c4·t − 1.5707965)` with the sine from the table.
#[inline]
pub(crate) fn out_elastic(start: f32, end: f32, t: f32) -> f32 {
    if t == 0.0 {
        start
    } else if t == 1.0 {
        end
    } else {
        let x = mul(transcendental::exp2(mul(t, -10.0)), sub(end, start));
        let s = table_sin(mul_add(t, ELASTIC_C4, -ELASTIC_PHASE_OUT));
        mul_add(x, s, end)
    }
}

/// `in_out_elastic`: `t == 0` → start, `2t == 2` → end; with `h = (end − start)/2` and
/// `s = sin(41.887901·t − 22.514748)` from the table, `2t >= 1 ? end + h·2^(10 − 20t)·s :
/// start − h·2^(20t − 10)·s`.
#[inline]
pub(crate) fn in_out_elastic(start: f32, end: f32, t: f32) -> f32 {
    let u = add(t, t);
    if t == 0.0 {
        start
    } else if u == 2.0 {
        end
    } else {
        let half = mul(sub(end, start), 0.5);
        let angle = mul_add(t, ELASTIC_C4_TWICE, -ELASTIC_PHASE_IN);
        if u >= 1.0 {
            let p = transcendental::exp2(mul_add(t, -20.0, 10.0));
            let s = table_sin(angle);
            mul_add(mul(half, p), s, end)
        } else {
            let p = transcendental::exp2(mul_add(t, 20.0, -10.0));
            let s = table_sin(angle);
            mul_sub(mul(half, p), s, start)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::numeric::test_support::*;
    use crate::stdlib::math::arch::test_support::{Body, Public, easing_bodies};

    const BODIES: [(&str, Body, Public); 30] = easing_bodies!(arm64);

    /// With `start − end = −10.5` the product is inexact, so an unfused multiply and add end one
    /// step away.
    #[test]
    fn in_sine_fuses_the_product_with_the_end() {
        for (t, bits) in [
            (0.0005_f32, 0xc03f_fff3_u32),
            (0.002, 0xc03f_ff31),
            (0.3, 0xbfed_8644),
        ] {
            assert_eq!(in_sine(-3.0, 7.5, t).to_bits(), bits, "t = {t}");
        }
    }

    /// A NaN through a negated input of a multiply-add comes back negated; an invalid operation
    /// gives `0x7fc00000`.
    #[test]
    fn the_nans_of_the_curves() {
        let negated: [(Body, f32); 6] = [
            (in_out_quad, INF),
            (in_circ, NAN),
            (out_circ, NAN),
            (out_expo, NAN),
            (in_out_bounce, NAN),
            (in_out_elastic, NAN),
        ];
        for (body, t) in negated {
            assert_eq!(body(1.0, 5.0, t).to_bits(), 0xffc0_0000, "{t}");
        }
        let invalid: [(Body, f32, u32); 5] = [
            (out_cubic, INF, 0x3f19_999a),
            (in_sine, INF, 0x3f00_0001),
            (out_quart, INF, 0x3eff_ffff),
            (in_bounce, INF, 0x3eff_0d0a),
            (out_circ, 5.0, 0xbf00_0000),
        ];
        for (body, end, t) in invalid {
            assert_eq!(
                body(1.0, end, f32::from_bits(t)).to_bits(),
                0x7fc0_0000,
                "{end} {t:#x}"
            );
        }
        assert_eq!(
            in_quad(1.0, 5.0, f32::from_bits(0xff80_0001)).to_bits(),
            0xffc0_0001
        );
    }

    /// At these `t` a phase one step off moves the table index.
    #[test]
    fn out_elastic_reads_the_sine_table_at_the_index_of_the_fused_angle() {
        for (t, bits_unit, bits_wide) in [
            (0.0278_f32, 0x3e9f_558a_u32, 0x400f_aac5_u32),
            (0.1028, 0x3fa2_8459, 0x40c2_8459),
            (0.1778, 0x3f9f_2c6c, 0x40bf_2c6c),
        ] {
            assert_eq!(out_elastic(0.0, 1.0, t).to_bits(), bits_unit, "t = {t}");
            assert_eq!(out_elastic(1.0, 5.0, t).to_bits(), bits_wide, "t = {t}");
        }
    }

    /// [`sweep_hash`] of each body.
    #[rustfmt::skip]
    const SWEEP_HASHES: [(&str, u64); 30] = [
        ("in_quad", 0x6980_d2e3_c429_d872),
        ("out_quad", 0x6b69_8b06_875b_1533),
        ("in_out_quad", 0x9b13_eb9e_86f7_36e5),
        ("in_cubic", 0x9f44_3002_56e7_b3a8),
        ("out_cubic", 0x1fae_6204_5329_b140),
        ("in_out_cubic", 0xff03_40a5_a22c_0d0e),
        ("in_quart", 0x52cd_7226_4d9a_5ad3),
        ("out_quart", 0x515b_25dc_7737_6733),
        ("in_out_quart", 0xf165_128f_92b3_9c2b),
        ("in_quint", 0x4248_ba50_ac59_f42f),
        ("out_quint", 0xee9d_1df3_4fc8_bf60),
        ("in_out_quint", 0x815c_05e7_9459_8c9e),
        ("in_sine", 0xceee_34dc_d403_15a1),
        ("out_sine", 0x42b2_57c7_3927_b597),
        ("in_out_sine", 0x5f42_8faf_abdd_be64),
        ("in_expo", 0x1f4a_8d1e_1d3d_c67c),
        ("out_expo", 0x2b6e_800f_5065_bcdd),
        ("in_out_expo", 0x74ba_42f4_a5fb_7f32),
        ("in_circ", 0x7670_30c1_0334_a5f8),
        ("out_circ", 0x6f98_c1c2_8cc8_df28),
        ("in_out_circ", 0x70ec_fe61_cb6f_9a28),
        ("in_bounce", 0x3bdb_6fd8_8a13_a3d7),
        ("out_bounce", 0x1e65_f77e_33ba_5983),
        ("in_out_bounce", 0xdacb_d2cb_533c_b3ec),
        ("in_back", 0x635e_2be0_49b8_83bb),
        ("out_back", 0xc6d3_8ef7_4536_7329),
        ("in_out_back", 0x55d5_b9e7_4c63_856f),
        ("in_elastic", 0xa63f_94b4_a6f0_73cc),
        ("out_elastic", 0xa506_6da1_dee1_bd11),
        ("in_out_elastic", 0x793c_ebe5_b30a_87ed),
    ];

    #[test]
    fn every_easing_of_this_architecture_keeps_the_bits_of_its_sweep() {
        for ((name, body, _), (hashed, expected)) in BODIES.into_iter().zip(SWEEP_HASHES) {
            assert_eq!(name, hashed);
            assert_eq!(sweep_hash(body), expected, "{name}");
        }
    }
}
