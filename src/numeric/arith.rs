//! The engine's primitive float operations, as this build performs them (see
//! [`ARCH`](super::ARCH)).

use super::arch::native;

/// `a·b + c`: rounded once on `Arm64`; rounded twice on `X86_64`, a NaN `a` winning over a NaN `b`
/// and the product over `c`.
#[inline]
pub fn mul_add(a: f32, b: f32, c: f32) -> f32 {
    native::mul_add(a, b, c)
}

/// `c − a·b`, rounded once on `Arm64`.
#[inline]
pub fn mul_sub(a: f32, b: f32, c: f32) -> f32 {
    native::mul_sub(a, b, c)
}

/// `a + b`; on `X86_64` a NaN `a` wins over a NaN `b`.
#[inline]
pub fn add(a: f32, b: f32) -> f32 {
    native::add(a, b)
}

/// `a·b`; on `X86_64` a NaN `a` wins over a NaN `b`.
#[inline]
pub fn mul(a: f32, b: f32) -> f32 {
    native::mul(a, b)
}

/// `−(a·b) − c`, rounded once on `Arm64`.
///
/// Not `−(a·b + c)`: an exact cancellation gives `+0`.
#[inline]
pub fn neg_mul_add(a: f32, b: f32, c: f32) -> f32 {
    native::neg_mul_add(a, b, c)
}

/// `a·b − c`, rounded once on `Arm64`.
#[inline]
pub fn neg_mul_sub(a: f32, b: f32, c: f32) -> f32 {
    native::neg_mul_sub(a, b, c)
}

/// `(a > b) ? a : b` on `X86_64`; on `Arm64` the larger operand, a NaN against a number ignored and
/// of two NaNs the second, as it is.
#[inline]
pub fn max(a: f32, b: f32) -> f32 {
    native::max(a, b)
}

/// `(a < b) ? a : b` on `X86_64`; on `Arm64` the smaller operand, a NaN against a number ignored
/// and of two NaNs the second, as it is.
#[inline]
pub fn min(a: f32, b: f32) -> f32 {
    native::min(a, b)
}

/// Truncating float-to-int: `Arm64` saturates and maps NaN to 0; `X86_64` gives `i32::MIN` for NaN
/// and out-of-range values.
#[inline]
pub fn to_int(v: f32) -> i32 {
    native::to_int(v)
}

#[cfg(test)]
mod tests {
    use crate::numeric::test_support::*;

    #[test]
    fn the_wrappers_are_the_native_primitives() {
        let x = third();
        assert_bits(super::mul_add(x, 3.0, -1.0), per_arch(0.0, 2.980_232_2e-8));
        assert_bits(super::mul_sub(x, 3.0, 1.0), per_arch(0.0, -2.980_232_2e-8));
        assert_bits(
            super::neg_mul_add(x, 3.0, -1.0),
            per_arch(0.0, -2.980_232_2e-8),
        );
        assert_bits(
            super::neg_mul_sub(x, 3.0, 1.0),
            per_arch(0.0, 2.980_232_2e-8),
        );
        assert_bits(super::add(0.1, 0.2), 0.1 + 0.2);
        assert_bits(super::mul(0.1, 0.2), 0.1 * 0.2);
        assert_bits(super::max(NAN, 4.0), 4.0);
        assert_eq!(super::max(4.0, NAN).is_nan(), per_arch(true, false));
        assert_eq!(super::min(4.0, NAN).is_nan(), per_arch(true, false));
        assert_eq!(super::to_int(NAN), per_arch(i32::MIN, 0));
    }
}
