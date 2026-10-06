//! The 61 `math.*` functions (`math.pi` among them): `ExpressionOp`s, not queries.

use crate::ops::ExpressionOp;

/// One row per function, `Variant = discriminant, token, friendly name, min args, max args;`; the
/// variant also names the `ExpressionOp` the function is.
macro_rules! math_fns {
    ($($(#[doc = $doc:literal])* $function:ident = $index:literal, $token:literal, $friendly:literal, $min:literal, $max:literal;)*) => {
        /// A `math.*` function. Each is one [`ExpressionOp`]; discriminants are dense (`0..61`) in op
        /// order.
        #[repr(u8)]
        #[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
        #[non_exhaustive]
        pub enum MathFn {
            $(
                #[doc = concat!("`", $token, "`.")]
                $(#[doc = $doc])*
                $function = $index,
            )*
        }

        /// One row per math function, indexed by discriminant.
        pub static MATH_META: [MathMeta; MathFn::COUNT] = [$(
            MathMeta {
                function: MathFn::$function,
                op: ExpressionOp::$function,
                token: $token,
                friendly_name: $friendly,
                min_args: $min,
                max_args: $max,
            }
        ),*];

        const ALL: [MathFn; MathFn::COUNT] = [$(MathFn::$function),*];

        impl ExpressionOp {
            /// The math function this op is, if it is one.
            pub const fn math_fn(self) -> Option<MathFn> {
                match self {
                    $(Self::$function => Some(MathFn::$function),)*
                    _ => None,
                }
            }
        }
    };
}

math_fns! {
    Abs = 0, "math.abs", "Absolute Value 'math.abs'", 1, 1;
    Acos = 1, "math.acos", "Arc Cosine 'math.acos'", 1, 1;
    Asin = 2, "math.asin", "Arc Sine 'math.asin'", 1, 1;
    Atan = 3, "math.atan", "Arc Tangent 'math.atan'", 1, 1;
    Atan2 = 4, "math.atan2", "atan2 'math.atan2'", 2, 2;
    Ceil = 5, "math.ceil", "Ceiling 'math.ceil'", 1, 1;
    Clamp = 6, "math.clamp", "Clamp 'math.clamp'", 3, 3;
    CopySign = 7, "math.copy_sign", "Copy Sign 'math.copy_sign'", 2, 2;
    Cos = 8, "math.cos", "Cosine 'math.cos'", 1, 1;
    DieRoll = 9, "math.die_roll", "Die Roll 'math.die_roll'", 3, 3;
    DieRollInt = 10, "math.die_roll_integer", "Die Roll Integer 'math.die_roll_integer'", 3, 3;
    Exp = 11, "math.exp", "Base-e Exponent 'math.exp'", 1, 1;
    Floor = 12, "math.floor", "Floor 'math.floor'", 1, 1;
    HermiteBlend = 13, "math.hermite_blend", "Hermite Blend 'math.hermite_blend'", 1, 1;
    Lerp = 14, "math.lerp", "Lerp 'math.lerp'", 3, 3;
    LerpRotate = 15, "math.lerprotate", "Lerp Rotate 'math.lerprotate'", 3, 3;
    Ln = 16, "math.ln", "Natural Log 'math.ln'", 1, 1;
    Max = 17, "math.max", "Max 'math.max'", 2, 2;
    Min = 18, "math.min", "Min 'math.min'", 2, 2;
    MinAngle = 19, "math.min_angle", "Min Angle 'math.min_angle'", 1, 1;
    Mod = 20, "math.mod", "Mod 'math.mod'", 2, 2;
    Pow = 21, "math.pow", "Power 'math.pow'", 2, 2;
    Random = 22, "math.random", "Random 'math.random'", 2, 2;
    RandomInt = 23, "math.random_integer", "Random Integer 'math.random_integer'", 2, 2;
    Round = 24, "math.round", "Round 'math.round'", 1, 1;
    Sin = 25, "math.sin", "Sine 'math.sin'", 1, 1;
    Sign = 26, "math.sign", "Sign 'math.sign'", 1, 1;
    Sqrt = 27, "math.sqrt", "Square Root 'math.sqrt'", 1, 1;
    Trunc = 28, "math.trunc", "Truncate 'math.trunc'", 1, 1;
    /// Written without parentheses.
    Pi = 29, "math.pi", "Pi", 0, 0;
    InverseLerp = 30, "math.inverse_lerp", "Inverse Lerp 'math.inverse_lerp'", 3, 3;
    EaseInQuad = 31, "math.ease_in_quad", "Ease In Quad 'math.ease_in_quad'", 3, 3;
    EaseOutQuad = 32, "math.ease_out_quad", "Ease Out Quad 'math.ease_out_quad'", 3, 3;
    EaseInOutQuad = 33, "math.ease_in_out_quad", "Ease In Out Quad 'math.ease_in_out_quad'", 3, 3;
    EaseInCubic = 34, "math.ease_in_cubic", "Ease In Cubic 'math.ease_in_cubic'", 3, 3;
    EaseOutCubic = 35, "math.ease_out_cubic", "Ease Out Cubic 'math.ease_out_cubic'", 3, 3;
    EaseInOutCubic = 36, "math.ease_in_out_cubic", "Ease In Out Cubic 'math.ease_in_out_cubic'", 3, 3;
    EaseInQuart = 37, "math.ease_in_quart", "Ease In Quart 'math.ease_in_quart'", 3, 3;
    EaseOutQuart = 38, "math.ease_out_quart", "Ease Out Quart 'math.ease_out_quart'", 3, 3;
    EaseInOutQuart = 39, "math.ease_in_out_quart", "Ease In Out Quart 'math.ease_in_out_quart'", 3, 3;
    EaseInQuint = 40, "math.ease_in_quint", "Ease In Quint 'math.ease_in_quint'", 3, 3;
    EaseOutQuint = 41, "math.ease_out_quint", "Ease Out Quint 'math.ease_out_quint'", 3, 3;
    EaseInOutQuint = 42, "math.ease_in_out_quint", "Ease In Out Quint 'math.ease_in_out_quint'", 3, 3;
    EaseInSine = 43, "math.ease_in_sine", "Ease In Sine 'math.ease_in_sine'", 3, 3;
    EaseOutSine = 44, "math.ease_out_sine", "Ease Out Sine 'math.ease_out_sine'", 3, 3;
    EaseInOutSine = 45, "math.ease_in_out_sine", "Ease In Out Sine 'math.ease_in_out_sine'", 3, 3;
    EaseInExpo = 46, "math.ease_in_expo", "Ease In Expo 'math.ease_in_expo'", 3, 3;
    EaseOutExpo = 47, "math.ease_out_expo", "Ease Out Expo 'math.ease_out_expo'", 3, 3;
    EaseInOutExpo = 48, "math.ease_in_out_expo", "Ease In Out Expo 'math.ease_in_out_expo'", 3, 3;
    EaseInCirc = 49, "math.ease_in_circ", "Ease In Circ 'math.ease_in_circ'", 3, 3;
    EaseOutCirc = 50, "math.ease_out_circ", "Ease Out Circ 'math.ease_out_circ'", 3, 3;
    EaseInOutCirc = 51, "math.ease_in_out_circ", "Ease In Out Circ 'math.ease_in_out_circ'", 3, 3;
    EaseInBounce = 52, "math.ease_in_bounce", "Ease In Bounce 'math.ease_in_bounce'", 3, 3;
    EaseOutBounce = 53, "math.ease_out_bounce", "Ease Out Bounce 'math.ease_out_bounce'", 3, 3;
    EaseInOutBounce = 54, "math.ease_in_out_bounce", "Ease In Out Bounce 'math.ease_in_out_bounce'", 3, 3;
    EaseInBack = 55, "math.ease_in_back", "Ease In Back 'math.ease_in_back'", 3, 3;
    EaseOutBack = 56, "math.ease_out_back", "Ease Out Back 'math.ease_out_back'", 3, 3;
    EaseInOutBack = 57, "math.ease_in_out_back", "Ease In Out Back 'math.ease_in_out_back'", 3, 3;
    EaseInElastic = 58, "math.ease_in_elastic", "Ease In Elastic 'math.ease_in_elastic'", 3, 3;
    EaseOutElastic = 59, "math.ease_out_elastic", "Ease Out Elastic 'math.ease_out_elastic'", 3, 3;
    EaseInOutElastic = 60, "math.ease_in_out_elastic", "Ease In Out Elastic 'math.ease_in_out_elastic'", 3, 3;
}

/// [`ALL`] sorted by token, bytewise, for binary search.
static BY_TOKEN: [MathFn; MathFn::COUNT] = {
    const fn precedes(a: &str, b: &str) -> bool {
        let (a, b) = (a.as_bytes(), b.as_bytes());
        let mut i = 0;
        while i < a.len() && i < b.len() {
            if a[i] != b[i] {
                return a[i] < b[i];
            }
            i += 1;
        }
        a.len() < b.len()
    }
    let mut sorted = ALL;
    let mut i = 1;
    while i < sorted.len() {
        let mut j = i;
        while j > 0 && precedes(sorted[j].token(), sorted[j - 1].token()) {
            sorted.swap(j, j - 1);
            j -= 1;
        }
        i += 1;
    }
    sorted
};

impl MathFn {
    /// Number of math functions (61, `math.pi` included).
    pub const COUNT: usize = 61;

    /// Every math function, in op-index order.
    pub const fn all() -> &'static [Self; Self::COUNT] {
        &ALL
    }

    /// The function's row.
    pub const fn meta(self) -> &'static MathMeta {
        &MATH_META[self as usize]
    }

    /// The `ExpressionOp` this function is.
    pub const fn op(self) -> ExpressionOp {
        self.meta().op
    }

    /// The token (`"math.abs"`).
    pub const fn token(self) -> &'static str {
        self.meta().token
    }

    /// The name parser messages insert (`"Absolute Value 'math.abs'"`).
    pub const fn friendly_name(self) -> &'static str {
        self.meta().friendly_name
    }

    /// The function with this token (`"math.abs"`); case-sensitive.
    pub fn from_token(token: &str) -> Option<Self> {
        BY_TOKEN
            .binary_search_by(|function| function.token().cmp(token))
            .ok()
            .map(|index| BY_TOKEN[index])
    }
}

/// One math function: an `ExpressionOp`, not a query.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct MathMeta {
    /// The function.
    pub function: MathFn,
    /// The op it is.
    pub op: ExpressionOp,
    /// The token (`"math.abs"`).
    pub token: &'static str,
    /// The name parser messages insert.
    pub friendly_name: &'static str,
    /// Minimum number of arguments.
    pub min_args: u8,
    /// Maximum number of arguments.
    pub max_args: u8,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::Side;

    #[test]
    fn math_functions() {
        assert_eq!(MathFn::COUNT, 61);
        assert_eq!(MathFn::all().len(), 61);
        assert_eq!(MATH_META.len(), 61);
        assert_eq!(
            ExpressionOp::all()
                .iter()
                .filter(|op| op.is_math_function())
                .count(),
            61
        );
        for (index, (&function, meta)) in MathFn::all().iter().zip(MATH_META.iter()).enumerate() {
            assert_eq!(function as usize, index);
            assert_eq!(meta.function, function);
            assert_eq!(function.meta(), meta);
            let op = function.op();
            assert!(op.is_math_function());
            assert_eq!(op.math_fn(), Some(function));
            assert_eq!(Some(function.token()), op.token());
            assert_eq!(function.friendly_name(), op.friendly_name());
            assert_eq!(
                (meta.min_args, Some(meta.max_args)),
                (op.min_children(), op.max_children())
            );
            assert!(meta.min_args <= meta.max_args);
            assert_eq!(MathFn::from_token(function.token()), Some(function));
            assert!(!crate::stdlib::queries(Side::Client).contains(function.token()));
        }
        let order: Vec<u8> = MathFn::all().iter().map(|f| f.op().ordinal()).collect();
        assert!(
            order.windows(2).all(|w| w[0] < w[1]),
            "dense in op-index order"
        );
        assert_eq!(MathFn::from_token("math.pi"), Some(MathFn::Pi));
        assert_eq!(
            (MathFn::Pi.meta().min_args, MathFn::Pi.meta().max_args),
            (0, 0)
        );
        assert_eq!(MathFn::from_token("math.clamp").unwrap().meta().max_args, 3);
        assert_eq!(
            MathFn::from_token("math.mod").unwrap().op(),
            ExpressionOp::Mod
        );
        assert_eq!(
            MathFn::from_token("math.random").unwrap().op(),
            ExpressionOp::Random
        );
        assert_eq!(MathFn::from_token("Math.abs"), None);
        assert_eq!(MathFn::from_token("math."), None);
        assert_eq!(ExpressionOp::Add.math_fn(), None);
        let eases = MathFn::all()
            .iter()
            .filter(|f| f.token().starts_with("math.ease_"))
            .count();
        assert_eq!(eases, 30);
    }

    #[test]
    fn math_ops_have_a_math_fn_and_the_others_do_not() {
        for op in ExpressionOp::all() {
            assert_eq!(op.math_fn().is_some(), op.is_math_function(), "{op:?}");
        }
        assert_eq!(ExpressionOp::Abs.math_fn(), Some(MathFn::Abs));
        assert_eq!(ExpressionOp::Assignment.math_fn(), None);
    }

    #[test]
    fn math_tokens_round_trip_and_start_with_math_dot() {
        for &f in MathFn::all() {
            assert!(f.token().starts_with("math."), "{}", f.token());
            assert_eq!(MathFn::from_token(f.token()), Some(f));
        }
        assert!(BY_TOKEN.windows(2).all(|w| w[0].token() < w[1].token()));
    }

    #[test]
    fn from_token_rejects_near_misses() {
        for bad in [
            "",
            "math.",
            "Math.abs",
            "MATH.ABS",
            "math.abs ",
            " math.abs",
            "abs",
            "math.nonexistent",
            "query.is_baby",
            "math.ab",
            "math.absx",
            "m.abs",
        ] {
            assert_eq!(MathFn::from_token(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn pi_takes_no_arguments() {
        assert_eq!(MathFn::Pi.token(), "math.pi");
        assert_eq!(
            (MathFn::Pi.meta().min_args, MathFn::Pi.meta().max_args),
            (0, 0)
        );
    }
}
