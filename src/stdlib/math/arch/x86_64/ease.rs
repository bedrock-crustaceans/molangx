//! The `X86_64` easings: the Penner formulas in order, every operation rounded on its own.
//!
//! The first operand of each arithmetic call is the one whose NaN wins;
//! the start value is the second operand of the final addition except in the elastic curves and the
//! second half of `in_out_bounce`.

use super::{sqrt, table_cos, table_sin};
use crate::numeric::arch::x86_64::{add, div, mul, sub};
use crate::stdlib::math::{
    PI,
    arch::{
        BACK_C1, BACK_C2, BACK_C2_PLUS_1, BACK_C3, BOUNCE_C2, BOUNCE_C3, BOUNCE_C4, BOUNCE_N1,
        BOUNCE_T1, BOUNCE_T2, BOUNCE_T3, HALF_PI,
    },
    transcendental,
};

/// 0.075, 2π and 0.3.
const ELASTIC_SHIFT: f32 = f32::from_bits(0x3d99_999a);
const TWO_PI: f32 = f32::from_bits(0x40c9_0fdb);
const ELASTIC_PERIOD: f32 = f32::from_bits(0x3e99_999a);

/// `x + 0`, turning a `−0` product into `+0` where the formulas need it.
#[inline]
fn plus_zero(x: f32) -> f32 {
    add(x, 0.0)
}

/// `in_quad`: `c·t·t + b`.
#[inline]
pub(crate) fn in_quad(start: f32, end: f32, t: f32) -> f32 {
    add(mul(mul(sub(end, start), t), t), start)
}

/// `out_quad`: `−c·t·(t − 2) + b`, computed as `b − (t − 2)·(c·t)`.
#[inline]
pub(crate) fn out_quad(start: f32, end: f32, t: f32) -> f32 {
    let x = mul(sub(end, start), t);
    sub(start, mul(add(t, -2.0), x))
}

/// `in_out_quad`: `1 > u ? c/2·u·u + b : −c/2·((u − 1)·((u − 1) − 2) − 1) + b`.
#[inline]
pub(crate) fn in_out_quad(start: f32, end: f32, t: f32) -> f32 {
    let u = add(t, t);
    let d = sub(end, start);
    if 1.0 > u {
        add(mul(mul(mul(d, 0.5), u), u), start)
    } else {
        let v = add(u, -1.0);
        add(mul(add(mul(add(-2.0, v), v), -1.0), mul(d, -0.5)), start)
    }
}

/// `in_cubic`: `c·t·t·t + b`.
#[inline]
pub(crate) fn in_cubic(start: f32, end: f32, t: f32) -> f32 {
    add(mul(mul(mul(sub(end, start), t), t), t), start)
}

/// `out_cubic`: `c·((t − 1)³ + 1) + b`.
#[inline]
pub(crate) fn out_cubic(start: f32, end: f32, t: f32) -> f32 {
    let v = add(t, -1.0);
    add(mul(add(mul(mul(v, v), v), 1.0), sub(end, start)), start)
}

/// `in_out_cubic`: `1 > u ? c/2·u·u·u + b : c/2·((u − 2)³ + 2) + b`.
#[inline]
pub(crate) fn in_out_cubic(start: f32, end: f32, t: f32) -> f32 {
    let u = add(t, t);
    let d = sub(end, start);
    if 1.0 > u {
        add(mul(mul(mul(mul(d, 0.5), u), u), u), start)
    } else {
        let w = add(u, -2.0);
        add(mul(add(mul(mul(w, w), w), 2.0), mul(d, 0.5)), start)
    }
}

/// `in_quart`: `c·t·t·t·t + b`.
#[inline]
pub(crate) fn in_quart(start: f32, end: f32, t: f32) -> f32 {
    add(mul(mul(mul(mul(sub(end, start), t), t), t), t), start)
}

/// `out_quart`: `−c·((t − 1)⁴ − 1) + b`.
#[inline]
pub(crate) fn out_quart(start: f32, end: f32, t: f32) -> f32 {
    let v = add(t, -1.0);
    sub(
        start,
        mul(add(mul(mul(mul(v, v), v), v), -1.0), sub(end, start)),
    )
}

/// `in_out_quart`: `1 > u ? c/2·u⁴ + b : −c/2·((u − 2)⁴ − 2) + b`.
#[inline]
pub(crate) fn in_out_quart(start: f32, end: f32, t: f32) -> f32 {
    let u = add(t, t);
    let d = sub(end, start);
    if 1.0 > u {
        add(mul(mul(mul(mul(mul(d, 0.5), u), u), u), u), start)
    } else {
        let w = add(u, -2.0);
        add(
            mul(add(mul(mul(mul(w, w), w), w), -2.0), mul(d, -0.5)),
            start,
        )
    }
}

/// `in_quint`: `c·t·t·t·t·t + b`.
#[inline]
pub(crate) fn in_quint(start: f32, end: f32, t: f32) -> f32 {
    add(
        mul(mul(mul(mul(mul(sub(end, start), t), t), t), t), t),
        start,
    )
}

/// `out_quint`: `c·((t − 1)⁵ + 1) + b`.
#[inline]
pub(crate) fn out_quint(start: f32, end: f32, t: f32) -> f32 {
    let v = add(t, -1.0);
    add(
        mul(add(mul(mul(mul(mul(v, v), v), v), v), 1.0), sub(end, start)),
        start,
    )
}

/// `in_out_quint`: `1 > u ? c/2·u⁵ + b : c/2·((u − 2)⁵ + 2) + b`.
#[inline]
pub(crate) fn in_out_quint(start: f32, end: f32, t: f32) -> f32 {
    let u = add(t, t);
    let d = sub(end, start);
    if 1.0 > u {
        add(mul(mul(mul(mul(mul(mul(d, 0.5), u), u), u), u), u), start)
    } else {
        let w = add(u, -2.0);
        add(
            mul(add(mul(mul(mul(mul(w, w), w), w), w), 2.0), mul(d, 0.5)),
            start,
        )
    }
}

/// `in_sine`: `c − c·cos(t·π/2) + b`.
#[inline]
pub(crate) fn in_sine(start: f32, end: f32, t: f32) -> f32 {
    let c = table_cos(mul(t, HALF_PI));
    let d = sub(end, start);
    add(sub(d, mul(c, d)), start)
}

/// `out_sine`: `c·sin(t·π/2) + b`.
#[inline]
pub(crate) fn out_sine(start: f32, end: f32, t: f32) -> f32 {
    add(mul(sub(end, start), table_sin(mul(t, HALF_PI))), start)
}

/// `in_out_sine`: `−c/2·(cos(π·t) − 1) + b`.
#[inline]
pub(crate) fn in_out_sine(start: f32, end: f32, t: f32) -> f32 {
    let h = mul(sub(end, start), -0.5);
    let c = table_cos(mul(t, PI));
    add(mul(add(c, -1.0), h), start)
}

/// `in_expo`: `c·2^(10·(t − 1)) + b`.
#[inline]
pub(crate) fn in_expo(start: f32, end: f32, t: f32) -> f32 {
    let d = sub(end, start);
    add(mul(transcendental::exp2(mul(add(t, -1.0), 10.0)), d), start)
}

/// `out_expo`: `c·(1 − 2^(−10t)) + b`.
#[inline]
pub(crate) fn out_expo(start: f32, end: f32, t: f32) -> f32 {
    let d = sub(end, start);
    add(mul(sub(1.0, transcendental::exp2(mul(t, -10.0))), d), start)
}

/// `in_out_expo`: `1 > u ? c/2·2^(10·(u − 1)) + b : c/2·(2 − 2^(−10·(u − 1))) + b`.
#[inline]
pub(crate) fn in_out_expo(start: f32, end: f32, t: f32) -> f32 {
    let u = add(t, t);
    let h = mul(sub(end, start), 0.5);
    let v = add(u, -1.0);
    if 1.0 > u {
        add(mul(transcendental::exp2(mul(v, 10.0)), h), start)
    } else {
        add(mul(sub(2.0, transcendental::exp2(mul(v, -10.0))), h), start)
    }
}

/// `in_circ`: `−c·(sqrt(1 − t·t) − 1) + b`; negating `c` flips a NaN's sign too.
#[inline]
pub(crate) fn in_circ(start: f32, end: f32, t: f32) -> f32 {
    let neg_d = -sub(end, start);
    let q = sqrt(sub(1.0, mul(t, t)));
    add(mul(add(q, -1.0), neg_d), start)
}

/// `out_circ`: `c·sqrt(1 − (t − 1)²) + b`.
#[inline]
pub(crate) fn out_circ(start: f32, end: f32, t: f32) -> f32 {
    let v = add(t, -1.0);
    add(mul(sub(end, start), sqrt(sub(1.0, mul(v, v)))), start)
}

/// `in_out_circ`: `1 > u ? −c/2·(sqrt(1 − u²) − 1) + b : c/2·(sqrt(1 − (u − 2)²) + 1) + b`, with
/// the halved range as the first factor.
#[inline]
pub(crate) fn in_out_circ(start: f32, end: f32, t: f32) -> f32 {
    let u = add(t, t);
    let d = sub(end, start);
    let (x, k, factor) = if 1.0 > u {
        (u, -1.0, mul(d, -0.5))
    } else {
        (add(u, -2.0), 1.0, mul(d, 0.5))
    };
    add(mul(factor, add(k, sqrt(sub(1.0, mul(x, x))))), start)
}

/// `n1·y·y + k` with `y = x − centre`; a NaN takes the last piece.
#[inline]
fn bounce(x: f32) -> f32 {
    if BOUNCE_T1 > x {
        mul(mul(BOUNCE_N1, x), x)
    } else if BOUNCE_T2 > x {
        let y = add(x, -BOUNCE_C2);
        add(mul(mul(BOUNCE_N1, y), y), 0.75)
    } else if BOUNCE_T3 > x {
        let y = add(x, -BOUNCE_C3);
        add(mul(mul(BOUNCE_N1, y), y), 0.9375)
    } else {
        let y = add(x, -BOUNCE_C4);
        add(mul(mul(BOUNCE_N1, y), y), 0.984_375)
    }
}

/// `in_bounce`: `c − out_bounce(1 − t)·c + b`.
#[inline]
pub(crate) fn in_bounce(start: f32, end: f32, t: f32) -> f32 {
    let d = sub(end, start);
    let x = plus_zero(mul(bounce(sub(1.0, t)), d));
    add(sub(d, x), start)
}

/// `out_bounce`: `c·out_bounce(t) + b`.
#[inline]
pub(crate) fn out_bounce(start: f32, end: f32, t: f32) -> f32 {
    add(mul(sub(end, start), bounce(t)), start)
}

/// `in_out_bounce`: `0.5 > t ? in_bounce(2t)·0.5 + b : out_bounce(2t − 1)·0.5 + c·0.5 + b`,
/// each half over `(0, c)`.
#[inline]
pub(crate) fn in_out_bounce(start: f32, end: f32, t: f32) -> f32 {
    let d = sub(end, start);
    if 0.5 > t {
        let x = plus_zero(mul(bounce(sub(1.0, add(t, t))), d));
        add(mul(plus_zero(sub(d, x)), 0.5), start)
    } else {
        let x = plus_zero(mul(bounce(add(add(t, t), -1.0)), d));
        add(start, add(mul(x, 0.5), mul(d, 0.5)))
    }
}

/// `in_back`: `c·t·t·(c3·t − c1) + b`.
#[inline]
pub(crate) fn in_back(start: f32, end: f32, t: f32) -> f32 {
    let x = mul(mul(sub(end, start), t), t);
    add(mul(add(mul(t, BACK_C3), -BACK_C1), x), start)
}

/// `out_back`: `c·((t − 1)²·(c3·(t − 1) + c1) + 1) + b`.
#[inline]
pub(crate) fn out_back(start: f32, end: f32, t: f32) -> f32 {
    let v = add(t, -1.0);
    let y = add(mul(v, BACK_C3), BACK_C1);
    add(mul(add(mul(mul(v, v), y), 1.0), sub(end, start)), start)
}

/// `in_out_back`: `1 > u ? c/2·(u²·((c2 + 1)·u − c2)) + b :
/// c/2·((u − 2)²·((c2 + 1)·(u − 2) + c2) + 2) + b`.
#[inline]
pub(crate) fn in_out_back(start: f32, end: f32, t: f32) -> f32 {
    let h = mul(sub(end, start), 0.5);
    let u = add(t, t);
    if 1.0 > u {
        add(
            mul(mul(add(mul(u, BACK_C2_PLUS_1), -BACK_C2), mul(u, u)), h),
            start,
        )
    } else {
        let w = add(u, -2.0);
        add(
            mul(
                add(mul(add(mul(w, BACK_C2_PLUS_1), BACK_C2), mul(w, w)), 2.0),
                h,
            ),
            start,
        )
    }
}

/// `((x − 0.075)·2π)/0.3`, rounded three times.
#[inline]
fn elastic_angle(x: f32) -> f32 {
    div(mul(add(x, -ELASTIC_SHIFT), TWO_PI), ELASTIC_PERIOD)
}

/// `in_elastic`: `t == 0` → `b`, `t == 1` → `b + c`, otherwise
/// `b − c·2^(10·(t − 1))·sin(((t − 1) − 0.075)·2π/0.3)`.
#[inline]
pub(crate) fn in_elastic(start: f32, end: f32, t: f32) -> f32 {
    if t == 0.0 {
        return start;
    }
    let d = sub(end, start);
    if t == 1.0 {
        return add(start, d);
    }
    let v = add(t, -1.0);
    let p = transcendental::exp2(mul(10.0, v));
    sub(start, mul(mul(d, p), table_sin(elastic_angle(v))))
}

/// `out_elastic`: `t == 0` → `b`, `t == 1` → `b + c`, otherwise
/// `c·2^(−10t)·sin((t − 0.075)·2π/0.3) + c + b`.
#[inline]
pub(crate) fn out_elastic(start: f32, end: f32, t: f32) -> f32 {
    if t == 0.0 {
        return start;
    }
    let d = sub(end, start);
    if t == 1.0 {
        return add(start, d);
    }
    let x = mul(transcendental::exp2(mul(-10.0, t)), d);
    add(start, add(mul(x, table_sin(elastic_angle(t))), d))
}

/// `in_out_elastic`: `t == 0` → `b`, `2t == 2` → `b + c`; with `v = 2t − 1` and
/// `s = sin((v − 0.075)·2π/0.3)`, `1 > 2t ? b + −0.5·(c·2^(10v)·s) : b + (c·2^(−10v)·s·0.5 + c)`.
#[inline]
pub(crate) fn in_out_elastic(start: f32, end: f32, t: f32) -> f32 {
    if t == 0.0 {
        return start;
    }
    let d = sub(end, start);
    let u = add(t, t);
    if u == 2.0 {
        return add(start, d);
    }
    let v = add(u, -1.0);
    let s = table_sin(elastic_angle(v));
    if 1.0 > u {
        add(
            start,
            mul(mul(mul(transcendental::exp2(mul(v, 10.0)), d), s), -0.5),
        )
    } else {
        add(
            start,
            add(
                mul(mul(mul(transcendental::exp2(mul(v, -10.0)), d), s), 0.5),
                d,
            ),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::numeric::test_support::*;
    use crate::stdlib::math::{
        self,
        arch::test_support::{Body, Public, easing_bodies},
    };

    const BODIES: [(&str, Body, Public); 30] = easing_bodies!(x86_64);

    /// The sign of the NaN each body returns (`+` clear, `-` set) with two NaNs of opposite sign in
    /// `(start, end)`, `(end, start)`, `(end, t)`, `(t, end)`, `(start, t)`, `(t, start)`, the
    /// first of each pair negative and the third argument 0.5.
    #[test]
    fn every_body_returns_its_nan_of_two() {
        let (n, p) = (f32::from_bits(0xffc0_0000), f32::from_bits(0x7fc0_0000));
        #[rustfmt::skip]
        let nans: [(&str, &str); 30] = [
            ("in_quad", "+--+-+"), ("out_quad", "-++--+"), ("in_out_quad", "+-+-+-"),
            ("in_cubic", "+--+-+"), ("out_cubic", "+-+-+-"), ("in_out_cubic", "+-+-+-"),
            ("in_quart", "+--+-+"), ("out_quart", "-++--+"), ("in_out_quart", "+-+-+-"),
            ("in_quint", "+--+-+"), ("out_quint", "+-+-+-"), ("in_out_quint", "+-+-+-"),
            ("in_sine", "+--+-+"), ("out_sine", "+--+-+"), ("in_out_sine", "+--+-+"),
            ("in_expo", "+-+-+-"), ("out_expo", "+-+-+-"), ("in_out_expo", "+-+-+-"),
            ("in_circ", "-++-+-"), ("out_circ", "+--+-+"), ("in_out_circ", "+--+-+"),
            ("in_bounce", "+--+-+"), ("out_bounce", "+--+-+"), ("in_out_bounce", "-++--+"),
            ("in_back", "+-+-+-"), ("out_back", "+-+-+-"), ("in_out_back", "+-+-+-"),
            ("in_elastic", "-+-+-+"), ("out_elastic", "-++--+"), ("in_out_elastic", "-++--+"),
        ];
        for ((name, body, _), (signs_name, signs)) in BODIES.into_iter().zip(nans) {
            assert_eq!(name, signs_name);
            let rows = [
                (n, p, 0.5),
                (p, n, 0.5),
                (0.5, n, p),
                (0.5, p, n),
                (n, 0.5, p),
                (p, 0.5, n),
            ];
            for ((start, end, t), sign) in rows.into_iter().zip(signs.chars()) {
                let got = body(start, end, t);
                assert!(got.is_nan(), "{name}({start}, {end}, {t})");
                let want = if sign == '+' { p } else { n };
                assert_eq!(got.to_bits(), want.to_bits(), "{name}({start}, {end}, {t})");
            }
        }
    }

    #[test]
    fn plus_zero_turns_negative_zero_into_positive_zero_and_keeps_everything_else() {
        assert_bits(plus_zero(-0.0), 0.0);
        assert_bits(plus_zero(0.0), 0.0);
        assert_bits(plus_zero(-1.5), -1.5);
        assert_bits(plus_zero(1.0e-45), 1.0e-45);
        assert_bits(plus_zero(-1.0e-45), -1.0e-45);
        assert_eq!(plus_zero(INF), INF);
        assert!(plus_zero(NAN).is_nan());
    }

    /// With `start = −0` and `d·0.5` underflowing to −0, the `+0` from `plus_zero` decides the
    /// sign.
    #[test]
    fn in_out_bounce_second_half_normalises_the_negative_zero_product() {
        let tiny = f32::from_bits(1);
        assert_bits(in_out_bounce(-0.0, -tiny, 0.5), 0.0);
        assert_bits(
            math::ease_in_out_bounce(-0.0, -tiny, 0.5, ID),
            per_arch(0.0, -0.0),
        );
        assert_bits(
            crate::stdlib::math::arch::arm64::in_out_bounce(-0.0, -tiny, 0.5),
            -0.0,
        );
    }

    #[test]
    fn in_out_quad_at_the_midpoint_is_start_plus_half_the_range() {
        assert_bits(in_out_quad(1.0, 5.0, 0.5), 3.0);
        assert_bits(in_out_quad(-1.0, 0.0, 0.5), -0.5);
        assert_bits(in_out_quad(2.0, 2.0, 0.5), 2.0);
    }

    /// At `u = 1` the branches round differently: for `c = 2^-149` the first gives `fl(c/2) = 0`
    /// and the second `−0 + c = c`.
    #[test]
    fn in_out_elastic_at_the_midpoint_takes_the_second_branch() {
        let tiny = f32::from_bits(1);
        assert_bits(table_sin(elastic_angle(0.0)), -1.0);
        assert_bits(in_out_elastic(0.0, tiny, 0.5), tiny);
        assert_bits(in_out_elastic(0.0, -tiny, 0.5), -tiny);
        let three = f32::from_bits(3);
        assert_bits(in_out_elastic(0.0, three, 0.5), tiny);
        assert_bits(in_out_elastic(1.0, 1.0 + 4.0, 0.5), 3.0);
        assert_bits(math::ease_in_out_elastic(0.0, tiny, 0.5, ID), tiny);
    }

    /// At these `t` a regrouped angle or a period one step off moves the table index.
    #[test]
    fn the_elastic_curves_read_the_sine_table_at_the_index_of_the_three_step_angle() {
        let curves: [(Body, f32, u32); 6] = [
            (in_elastic, 0.2235, 0xbb80_080a),
            (in_elastic, 0.4485, 0x3c3c_cdfe),
            (out_elastic, 0.0375, 0x3ee8_d482),
            (out_elastic, 0.1875, 0x3f98_ad80),
            (in_out_elastic, 0.0125, 0x2e5f_42b2),
            (in_out_elastic, 0.0875, 0xb429_1280),
        ];
        for (ease, t, bits) in curves {
            assert_eq!(ease(0.0, 1.0, t).to_bits(), bits, "t = {t}");
        }
    }

    /// [`sweep_hash`] of each body.
    #[rustfmt::skip]
    const SWEEP_HASHES: [(&str, u64); 30] = [
        ("in_quad", 0xa740_e060_0249_41dc),
        ("out_quad", 0x6e0c_6154_a7e9_c7a7),
        ("in_out_quad", 0x909d_b438_fa55_cc6e),
        ("in_cubic", 0x2675_f5d3_048b_14b3),
        ("out_cubic", 0xe72e_5c82_74e7_6e21),
        ("in_out_cubic", 0x6efe_e071_ff47_524f),
        ("in_quart", 0x5263_6d29_b0e0_43ea),
        ("out_quart", 0x0d5e_0527_cbcf_b19c),
        ("in_out_quart", 0x5daa_5531_b8de_0d2a),
        ("in_quint", 0xde79_6eb2_6dc0_3eee),
        ("out_quint", 0x0ee9_154b_f207_c53f),
        ("in_out_quint", 0x5850_494c_7ea1_be3a),
        ("in_sine", 0x29f9_cdc7_1100_00d5),
        ("out_sine", 0x41a7_84c6_6dac_2635),
        ("in_out_sine", 0x837e_f434_6ff7_7055),
        ("in_expo", 0xd0ae_d93f_7426_7609),
        ("out_expo", 0xf1b2_7628_cfa0_2f92),
        ("in_out_expo", 0x0921_0b7b_a41f_2b8a),
        ("in_circ", 0xc1a7_1232_d230_380f),
        ("out_circ", 0xc0d4_b980_a419_ea7c),
        ("in_out_circ", 0x99f6_d001_0c7e_a0cb),
        ("in_bounce", 0x516e_7ceb_aeba_2db8),
        ("out_bounce", 0x1647_68d2_6ac7_8d43),
        ("in_out_bounce", 0x8950_180f_031f_c6dc),
        ("in_back", 0x6352_1748_d047_74d5),
        ("out_back", 0x4f45_06bc_aced_9fb8),
        ("in_out_back", 0x5529_758c_e25a_e093),
        ("in_elastic", 0x4766_1d3b_cf39_170c),
        ("out_elastic", 0xa8e2_087c_0c08_285d),
        ("in_out_elastic", 0xb7bc_19af_bdc4_b912),
    ];

    #[test]
    fn every_easing_of_this_architecture_keeps_the_bits_of_its_sweep() {
        for ((name, body, _), (hashed, expected)) in BODIES.into_iter().zip(SWEEP_HASHES) {
            assert_eq!(name, hashed);
            assert_eq!(sweep_hash(body), expected, "{name}");
        }
    }
}
