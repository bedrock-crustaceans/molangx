//! [`ExpressionOp`]: the node kinds of a parsed expression, in index order.

use super::table::{OP_META, OpFlags, OpMeta};

pub(crate) const COUNT: usize = 111;

/// The node kinds of a parsed expression. The discriminant is the index [`OpSet`](super::OpSet)
/// uses.
#[repr(u8)]
#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[non_exhaustive]
pub enum ExpressionOp {
    /// `{`.
    LeftBrace = 0,
    /// `}`.
    RightBrace = 1,
    /// `[`.
    LeftBracket = 2,
    /// `]`.
    RightBracket = 3,
    /// `(`.
    LeftParenthesis = 4,
    /// `)`.
    RightParenthesis = 5,
    /// `-`.
    Negate = 6,
    /// `!`.
    LogicalNot = 7,
    /// `math.abs`.
    Abs = 8,
    /// `+`.
    Add = 9,
    /// `math.acos`.
    Acos = 10,
    /// `math.asin`.
    Asin = 11,
    /// `math.atan`.
    Atan = 12,
    /// `math.atan2`.
    Atan2 = 13,
    /// `math.ceil`.
    Ceil = 14,
    /// `math.clamp`.
    Clamp = 15,
    /// `math.copy_sign`.
    CopySign = 16,
    /// `math.cos`.
    Cos = 17,
    /// `math.die_roll`.
    DieRoll = 18,
    /// `math.die_roll_integer`.
    DieRollInt = 19,
    /// `/`.
    Div = 20,
    /// `math.exp`.
    Exp = 21,
    /// `math.floor`.
    Floor = 22,
    /// `math.hermite_blend`.
    HermiteBlend = 23,
    /// `math.lerp`.
    Lerp = 24,
    /// `math.lerprotate`.
    LerpRotate = 25,
    /// `math.ln`.
    Ln = 26,
    /// `math.max`.
    Max = 27,
    /// `math.min`.
    Min = 28,
    /// `math.min_angle`.
    MinAngle = 29,
    /// `math.mod`.
    Mod = 30,
    /// `*`.
    Mul = 31,
    /// `math.pow`.
    Pow = 32,
    /// `math.random`.
    Random = 33,
    /// `math.random_integer`.
    RandomInt = 34,
    /// `math.round`.
    Round = 35,
    /// `math.sin`.
    Sin = 36,
    /// `math.sign`.
    Sign = 37,
    /// `math.sqrt`.
    Sqrt = 38,
    /// `math.trunc`.
    Trunc = 39,
    /// `query.` or `q.`.
    QueryFunction = 40,
    /// `array.`.
    ArrayVariable = 41,
    /// `context.` or `c.`.
    ContextVariable = 42,
    /// `variable.` or `v.`.
    EntityVariable = 43,
    /// `temp.` or `t.`.
    TempVariable = 44,
    /// `.` (member access).
    MemberAccessor = 45,
    /// A string literal.
    StringLiteral = 46,
    /// `geometry.`.
    GeometryVariable = 47,
    /// `material.`.
    MaterialVariable = 48,
    /// `texture.`.
    TextureVariable = 49,
    /// `<`.
    LessThan = 50,
    /// `<=`.
    LessEqual = 51,
    /// `>=`.
    GreaterEqual = 52,
    /// `>`.
    GreaterThan = 53,
    /// `==`.
    LogicalEqual = 54,
    /// `!=`.
    LogicalNotEqual = 55,
    /// `||`.
    LogicalOr = 56,
    /// `&&`.
    LogicalAnd = 57,
    /// `??`.
    NullCoalescing = 58,
    /// `?`.
    Conditional = 59,
    /// `:`.
    ConditionalElse = 60,
    /// A number literal.
    Float = 61,
    /// `math.pi`, written without parentheses.
    Pi = 62,
    /// `[]`.
    Array = 63,
    /// A geometry reference.
    Geometry = 64,
    /// A material reference.
    Material = 65,
    /// A texture reference.
    Texture = 66,
    /// `loop`.
    Loop = 67,
    /// `for_each`.
    ForEach = 68,
    /// `break`.
    Break = 69,
    /// `continue`.
    Continue = 70,
    /// `=`.
    Assignment = 71,
    /// `->`.
    Pointer = 72,
    /// `;`.
    Semicolon = 73,
    /// `return`.
    Return = 74,
    /// `,`.
    Comma = 75,
    /// `this`.
    This = 76,
    /// An expression array.
    ExpressionArray = 77,
    /// `math.inverse_lerp`.
    InverseLerp = 78,
    /// `math.ease_in_quad`.
    EaseInQuad = 79,
    /// `math.ease_out_quad`.
    EaseOutQuad = 80,
    /// `math.ease_in_out_quad`.
    EaseInOutQuad = 81,
    /// `math.ease_in_cubic`.
    EaseInCubic = 82,
    /// `math.ease_out_cubic`.
    EaseOutCubic = 83,
    /// `math.ease_in_out_cubic`.
    EaseInOutCubic = 84,
    /// `math.ease_in_quart`.
    EaseInQuart = 85,
    /// `math.ease_out_quart`.
    EaseOutQuart = 86,
    /// `math.ease_in_out_quart`.
    EaseInOutQuart = 87,
    /// `math.ease_in_quint`.
    EaseInQuint = 88,
    /// `math.ease_out_quint`.
    EaseOutQuint = 89,
    /// `math.ease_in_out_quint`.
    EaseInOutQuint = 90,
    /// `math.ease_in_sine`.
    EaseInSine = 91,
    /// `math.ease_out_sine`.
    EaseOutSine = 92,
    /// `math.ease_in_out_sine`.
    EaseInOutSine = 93,
    /// `math.ease_in_expo`.
    EaseInExpo = 94,
    /// `math.ease_out_expo`.
    EaseOutExpo = 95,
    /// `math.ease_in_out_expo`.
    EaseInOutExpo = 96,
    /// `math.ease_in_circ`.
    EaseInCirc = 97,
    /// `math.ease_out_circ`.
    EaseOutCirc = 98,
    /// `math.ease_in_out_circ`.
    EaseInOutCirc = 99,
    /// `math.ease_in_bounce`.
    EaseInBounce = 100,
    /// `math.ease_out_bounce`.
    EaseOutBounce = 101,
    /// `math.ease_in_out_bounce`.
    EaseInOutBounce = 102,
    /// `math.ease_in_back`.
    EaseInBack = 103,
    /// `math.ease_out_back`.
    EaseOutBack = 104,
    /// `math.ease_in_out_back`.
    EaseInOutBack = 105,
    /// `math.ease_in_elastic`.
    EaseInElastic = 106,
    /// `math.ease_out_elastic`.
    EaseOutElastic = 107,
    /// `math.ease_in_out_elastic`.
    EaseInOutElastic = 108,
    /// A pure function of the compile's `MathCatalog`.
    HostMath = 109,
    /// A volatile function of the compile's `MathCatalog`.
    HostMathVolatile = 110,
}

/// The ops of the table rows, in index order.
pub(crate) const ALL: [ExpressionOp; COUNT] = {
    let mut all = [ExpressionOp::LeftBrace; COUNT];
    let mut i = 0;
    while i < COUNT {
        all[i] = OP_META[i].op;
        i += 1;
    }
    all
};

impl ExpressionOp {
    /// Number of ops.
    pub const COUNT: usize = COUNT;

    /// The op with this index (`0..111`).
    pub const fn from_ordinal(ordinal: u8) -> Option<Self> {
        if (ordinal as usize) < Self::COUNT {
            Some(ALL[ordinal as usize])
        } else {
            None
        }
    }

    /// The op's index, its bit in [`OpSet`](super::OpSet).
    pub const fn ordinal(self) -> u8 {
        self as u8
    }

    /// Every op, in index order.
    pub const fn all() -> &'static [Self; COUNT] {
        &ALL
    }

    /// The op's table row.
    pub const fn meta(self) -> &'static OpMeta {
        &OP_META[self as usize]
    }

    /// The source token, if the op has one.
    pub const fn token(self) -> Option<&'static str> {
        self.meta().token
    }

    /// The name parser messages insert (`"Assignment '='"`).
    pub const fn friendly_name(self) -> &'static str {
        self.meta().friendly_name
    }

    /// Minimum number of children.
    pub const fn min_children(self) -> u8 {
        self.meta().min_children
    }

    /// Maximum number of children; `None` is unbounded.
    pub const fn max_children(self) -> Option<u8> {
        self.meta().max_children
    }

    /// Whether the op is a standard `math.*` function or `math.pi`.
    pub const fn is_math_function(self) -> bool {
        self.meta().flags.contains(OpFlags::MATH_FUNCTION)
    }

    /// Whether the op is a resource variable (`geometry.`, `material.`, `texture.`).
    pub const fn is_resource_reference(self) -> bool {
        self.meta().flags.contains(OpFlags::RESOURCE_REFERENCE)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_ordinal_rejects_everything_from_111_up() {
        for ordinal in 111..=u8::MAX {
            assert_eq!(ExpressionOp::from_ordinal(ordinal), None, "{ordinal}");
        }
        assert_eq!(
            ExpressionOp::from_ordinal(110),
            Some(ExpressionOp::HostMathVolatile)
        );
        assert_eq!(
            ExpressionOp::from_ordinal(108),
            Some(ExpressionOp::EaseInOutElastic)
        );
        assert_eq!(ExpressionOp::from_ordinal(0), Some(ExpressionOp::LeftBrace));
    }

    #[test]
    fn tokenless_ops_are_the_seven_without_source_text() {
        use ExpressionOp as Op;
        let tokenless: Vec<Op> = Op::all()
            .iter()
            .copied()
            .filter(|op| op.token().is_none())
            .collect();
        assert_eq!(
            tokenless,
            [
                Op::Float,
                Op::Geometry,
                Op::Material,
                Op::Texture,
                Op::ExpressionArray,
                Op::HostMath,
                Op::HostMathVolatile
            ]
        );
    }

    #[test]
    fn accessors_read_the_row() {
        use ExpressionOp as Op;
        assert_eq!(Op::Assignment.friendly_name(), "Assignment '='");
        assert_eq!(Op::Abs.friendly_name(), "Absolute Value 'math.abs'");
        assert_eq!(Op::LeftBrace.token(), Some("{"));
        assert_eq!(Op::Float.token(), None);
        assert_eq!(
            (
                Op::Conditional.min_children(),
                Op::Conditional.max_children()
            ),
            (2, Some(3))
        );
        assert_eq!((Op::Add.min_children(), Op::Add.max_children()), (2, None));
        assert!(Op::Abs.is_math_function() && !Op::Add.is_math_function());
        assert!(
            Op::TextureVariable.is_resource_reference() && !Op::Texture.is_resource_reference()
        );
    }
}
