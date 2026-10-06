//! `ceil`, `floor`, `round` and `trunc`.

use crate::numeric::PostOp;

/// `math.ceil(x)`: round toward +∞ (`ceil(-0.5)` = −0).
#[inline]
pub fn ceil(x: f32, post: PostOp) -> f32 {
    post.apply(x.ceil())
}

/// `math.floor(x)`: round toward −∞.
#[inline]
pub fn floor(x: f32, post: PostOp) -> f32 {
    post.apply(x.floor())
}

/// `math.round(x)`: round half away from zero (`round(-2.5)` = −3).
#[inline]
pub fn round(x: f32, post: PostOp) -> f32 {
    post.apply(x.round())
}

/// `math.trunc(x)`: round toward zero.
#[inline]
pub fn trunc(x: f32, post: PostOp) -> f32 {
    post.apply(x.trunc())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::numeric::{PostOp, test_support::*};

    #[test]
    fn ceil_floor_round_and_trunc_on_ordinary_values() {
        // (x, ceil, floor, round, trunc)
        let table: [(f32, f32, f32, f32, f32); 10] = [
            (1.1, 2.0, 1.0, 1.0, 1.0),
            (-1.1, -1.0, -2.0, -1.0, -1.0),
            (1.5, 2.0, 1.0, 2.0, 1.0),
            (-1.5, -1.0, -2.0, -2.0, -1.0),
            (2.5, 3.0, 2.0, 3.0, 2.0),
            (-2.5, -2.0, -3.0, -3.0, -2.0),
            (0.499_999_97, 1.0, 0.0, 0.0, 0.0),
            (3.0, 3.0, 3.0, 3.0, 3.0),
            (-3.0, -3.0, -3.0, -3.0, -3.0),
            (
                8_388_609.0,
                8_388_609.0,
                8_388_609.0,
                8_388_609.0,
                8_388_609.0,
            ),
        ];
        for (x, c, f, r, t) in table {
            assert_bits(ceil(x, ID), c);
            assert_bits(floor(x, ID), f);
            assert_bits(round(x, ID), r);
            assert_bits(trunc(x, ID), t);
        }
    }

    #[test]
    fn rounding_keeps_zeros_infinities_and_nan() {
        for f in [ceil, floor, round, trunc] {
            assert_bits(f(0.0, ID), 0.0);
            assert_bits(f(-0.0, ID), -0.0);
            assert_eq!(f(INF, ID), INF);
            assert_eq!(f(-INF, ID), -INF);
            assert!(f(NAN, ID).is_nan());
        }
    }

    #[test]
    fn rounding_a_small_negative_value_gives_negative_zero() {
        assert_bits(ceil(-0.5, ID), -0.0);
        assert_bits(round(-0.4, ID), -0.0);
        assert_bits(trunc(-0.9, ID), -0.0);
        assert_bits(floor(0.9, ID), 0.0);
        assert_bits(ceil(-1.0e-45, ID), -0.0);
    }

    #[test]
    fn rounding_with_a_real_post_op_normalises_negative_zero() {
        let post = PostOp::new(2.0, 0.0);
        assert_bits(ceil(-0.5, post), 0.0);
        assert_bits(round(-0.4, post), 0.0);
        assert_bits(trunc(-0.9, post), 0.0);
        assert_bits(floor(0.9, post), 0.0);
    }

    #[test]
    fn rounding_applies_the_post_op_to_the_rounded_value() {
        assert_eq!(ceil(1.1, AFFINE), 5.0);
        assert_eq!(floor(1.9, AFFINE), 3.0);
        assert_eq!(round(1.5, AFFINE), 5.0);
        assert_eq!(round(-1.5, AFFINE), -3.0);
        assert_eq!(trunc(-1.9, AFFINE), -1.0);
        assert_eq!(ceil(INF, AFFINE), INF);
        assert!(ceil(NAN, AFFINE).is_nan());
    }

    #[test]
    fn ceil_floor_round_trunc_on_halves_and_with_a_post_op() {
        assert_eq!(ceil(-0.5, ID).to_bits(), (-0.0_f32).to_bits());
        assert_eq!(ceil(1.1, ID), 2.0);
        assert_eq!(floor(-0.5, ID), -1.0);
        assert_eq!(floor(1.9, ID), 1.0);
        assert_eq!(round(0.5, ID), 1.0);
        assert_eq!(round(-0.5, ID), -1.0);
        assert_eq!(round(-1.5, ID), -2.0);
        assert_eq!(round(-2.5, ID), -3.0);
        assert_eq!(round(2.4, ID), 2.0);
        assert_eq!(trunc(1.7, ID), 1.0);
        assert_eq!(trunc(-1.7, ID), -1.0);
        assert_eq!(ceil(1.1, PostOp::new(-2.0, -1.0)), -5.0);
    }
}
