//! One function per `math.*` operator.
//!
//! Each takes the Molang arguments in order, then the node's [`PostOp`], and computes with this
//! build's float behaviour ([`numeric::ARCH`](crate::numeric::ARCH)). Nothing clamps `t` or
//! validates arguments. The random family takes raw samples from the caller and clamps them with
//! [`clamp_sample`].

// Single-letter names follow the formulas.
#![allow(clippy::many_single_char_names)]

use crate::numeric::PostOp;
pub use ease::{
    ease_in_back, ease_in_bounce, ease_in_circ, ease_in_cubic, ease_in_elastic, ease_in_expo,
    ease_in_out_back, ease_in_out_bounce, ease_in_out_circ, ease_in_out_cubic, ease_in_out_elastic,
    ease_in_out_expo, ease_in_out_quad, ease_in_out_quart, ease_in_out_quint, ease_in_out_sine,
    ease_in_quad, ease_in_quart, ease_in_quint, ease_in_sine, ease_out_back, ease_out_bounce,
    ease_out_circ, ease_out_cubic, ease_out_elastic, ease_out_expo, ease_out_quad, ease_out_quart,
    ease_out_quint, ease_out_sine,
};
pub use random::{
    DieRoll, clamp_sample, die_roll, die_roll_count, die_roll_integer, random, random_const_bounds,
    random_folded, random_integer, random_integer_const_bounds,
};

/// `math.pi`: π as an `f32`.
pub const PI: f32 = std::f32::consts::PI;

/// The degrees → radians factor of `math.sin` / `math.cos`.
pub const DEG_TO_RAD: f32 = f32::from_bits(0x3c8e_fa35);

/// The radians → degrees factor of `math.asin` / `acos` / `atan` / `atan2`.
pub const RAD_TO_DEG: f32 = f32::from_bits(0x4265_2ee0);

/// The largest `|x|` that `math.asin` / `math.acos` clamp to `[-1, 1]` (1.0005); beyond it they
/// give NaN.
pub const INVERSE_TRIG_TOLERANCE: f32 = f32::from_bits(0x3f80_1062);

/// `math.pi` with the node's post-op.
#[inline]
pub fn pi(post: PostOp) -> f32 {
    post.apply(PI)
}

mod arch;
mod arithmetic;
mod ease;
pub(crate) mod fold;
mod interpolation;
mod random;
mod rounding;
mod transcendental;
mod trig;

pub use arithmetic::{abs, copy_sign, exp, ln, max, min, mod_const, mod_runtime, pow, sign, sqrt};
pub use interpolation::{clamp, hermite_blend, inverse_lerp, lerp, lerprotate, min_angle};
pub use rounding::{ceil, floor, round, trunc};
pub use trig::{acos, asin, atan, atan2, cos, sin};

#[cfg(test)]
mod tests {
    // Expected values keep nine significant digits.
    #![allow(clippy::excessive_precision)]

    use super::*;
    use crate::numeric::{PostOp, test_support::*};

    #[test]
    fn constants_are_the_documented_f32_values() {
        assert_eq!(PI.to_bits(), 0x4049_0fdb);
        assert_eq!(PI, 3.141_592_74);
        assert_eq!(DEG_TO_RAD.to_bits(), 0x3c8e_fa35);
        assert_eq!(RAD_TO_DEG.to_bits(), 0x4265_2ee0);
        assert_eq!(INVERSE_TRIG_TOLERANCE.to_bits(), 0x3f80_1062);
        close(DEG_TO_RAD * RAD_TO_DEG, 1.0, 1.2e-7);
        const { assert!(INVERSE_TRIG_TOLERANCE > 1.0004 && INVERSE_TRIG_TOLERANCE < 1.0006) };
    }

    #[test]
    fn pi_applies_the_post_op_with_the_architecture_rounding() {
        assert_bits(pi(ID), PI);
        assert_eq!(pi(ID), 3.141_592_74);
        assert_eq!(pi(PostOp::new(2.0, 0.0)), 6.283_185_5);
        assert_bits(pi(AFFINE), 2.0 * PI + 1.0);
        assert_bits(pi(PostOp::new(0.0, 5.0)), 5.0);
        assert_bits(pi(PostOp::new(-1.0, 0.0)), -PI);
        let post = PostOp::new(3.0, -1.0);
        assert_bits(pi(post), per_arch(PI * 3.0 - 1.0, PI.mul_add(3.0, -1.0)));
    }
}
