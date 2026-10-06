//! [`OP_META`]: one hand-maintained row of metadata per operator.

use core::fmt;

use super::op::{COUNT, ExpressionOp};

/// One row of [`OP_META`](super::OP_META).
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct OpMeta {
    /// The op.
    pub op: ExpressionOp,
    /// The variant name (`"LeftBrace"`).
    pub name: &'static str,
    /// The source token (`"math.abs"`, `"+"`, `"query."`); `None` for ops no source text spells.
    pub token: Option<&'static str>,
    /// Every accepted spelling, the token included (`["query.", "q."]`); empty when the token is
    /// the only one.
    pub aliases: &'static [&'static str],
    /// The name parser messages insert (`"Assignment '='"`).
    pub friendly_name: &'static str,
    /// Minimum number of children.
    pub min_children: u8,
    /// Maximum number of children; `None` is unbounded.
    pub max_children: Option<u8>,
    /// Classification flags.
    pub flags: OpFlags,
}

bitflags::bitflags! {
    /// Classification of an op.
    #[derive(Copy, Clone, Default, PartialEq, Eq, Hash)]
    pub struct OpFlags: u8 {
        /// A `math.*` function or `math.pi`.
        const MATH_FUNCTION = 1 << 0;
        /// A resource variable prefix: `geometry.`, `material.`, `texture.`.
        const RESOURCE_REFERENCE = 1 << 1;
        /// `=`, `math.random`, `math.random_integer` and a volatile host math function: the ops
        /// [`OpSet::without_assignments_or_random`] clears.
        const SIDE_EFFECT = 1 << 2;
    }
}

impl fmt::Debug for OpFlags {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        crate::bitmask::fmt_bit_names(
            f,
            self.bits().into(),
            &["MathFunction", "ResourceReference", "SideEffect"],
        )
    }
}

/// The op table, indexed by discriminant.
pub static OP_META: [OpMeta; COUNT] = [
    OpMeta {
        op: ExpressionOp::LeftBrace,
        name: "LeftBrace",
        token: Some("{"),
        aliases: &[],
        friendly_name: "Left Brace '{'",
        min_children: 1,
        max_children: None,
        flags: OpFlags::empty(),
    },
    OpMeta {
        op: ExpressionOp::RightBrace,
        name: "RightBrace",
        token: Some("}"),
        aliases: &[],
        friendly_name: "Right Brace '}'",
        min_children: 1,
        max_children: None,
        flags: OpFlags::empty(),
    },
    OpMeta {
        op: ExpressionOp::LeftBracket,
        name: "LeftBracket",
        token: Some("["),
        aliases: &[],
        friendly_name: "Left Bracket '['",
        min_children: 1,
        max_children: None,
        flags: OpFlags::empty(),
    },
    OpMeta {
        op: ExpressionOp::RightBracket,
        name: "RightBracket",
        token: Some("]"),
        aliases: &[],
        friendly_name: "Right Bracket ']'",
        min_children: 1,
        max_children: None,
        flags: OpFlags::empty(),
    },
    OpMeta {
        op: ExpressionOp::LeftParenthesis,
        name: "LeftParenthesis",
        token: Some("("),
        aliases: &[],
        friendly_name: "Left Parenthesis '('",
        min_children: 1,
        max_children: None,
        flags: OpFlags::empty(),
    },
    OpMeta {
        op: ExpressionOp::RightParenthesis,
        name: "RightParenthesis",
        token: Some(")"),
        aliases: &[],
        friendly_name: "Right Parenthesis ')'",
        min_children: 1,
        max_children: None,
        flags: OpFlags::empty(),
    },
    OpMeta {
        op: ExpressionOp::Negate,
        name: "Negate",
        token: Some("-"),
        aliases: &[],
        friendly_name: "Negate '-'",
        min_children: 1,
        max_children: Some(1),
        flags: OpFlags::empty(),
    },
    OpMeta {
        op: ExpressionOp::LogicalNot,
        name: "LogicalNot",
        token: Some("!"),
        aliases: &[],
        friendly_name: "Logical Not '!'",
        min_children: 1,
        max_children: Some(1),
        flags: OpFlags::empty(),
    },
    OpMeta {
        op: ExpressionOp::Abs,
        name: "Abs",
        token: Some("math.abs"),
        aliases: &[],
        friendly_name: "Absolute Value 'math.abs'",
        min_children: 1,
        max_children: Some(1),
        flags: OpFlags::MATH_FUNCTION,
    },
    OpMeta {
        op: ExpressionOp::Add,
        name: "Add",
        token: Some("+"),
        aliases: &[],
        friendly_name: "Add '+'",
        min_children: 2,
        max_children: None,
        flags: OpFlags::empty(),
    },
    OpMeta {
        op: ExpressionOp::Acos,
        name: "Acos",
        token: Some("math.acos"),
        aliases: &[],
        friendly_name: "Arc Cosine 'math.acos'",
        min_children: 1,
        max_children: Some(1),
        flags: OpFlags::MATH_FUNCTION,
    },
    OpMeta {
        op: ExpressionOp::Asin,
        name: "Asin",
        token: Some("math.asin"),
        aliases: &[],
        friendly_name: "Arc Sine 'math.asin'",
        min_children: 1,
        max_children: Some(1),
        flags: OpFlags::MATH_FUNCTION,
    },
    OpMeta {
        op: ExpressionOp::Atan,
        name: "Atan",
        token: Some("math.atan"),
        aliases: &[],
        friendly_name: "Arc Tangent 'math.atan'",
        min_children: 1,
        max_children: Some(1),
        flags: OpFlags::MATH_FUNCTION,
    },
    OpMeta {
        op: ExpressionOp::Atan2,
        name: "Atan2",
        token: Some("math.atan2"),
        aliases: &[],
        friendly_name: "atan2 'math.atan2'",
        min_children: 2,
        max_children: Some(2),
        flags: OpFlags::MATH_FUNCTION,
    },
    OpMeta {
        op: ExpressionOp::Ceil,
        name: "Ceil",
        token: Some("math.ceil"),
        aliases: &[],
        friendly_name: "Ceiling 'math.ceil'",
        min_children: 1,
        max_children: Some(1),
        flags: OpFlags::MATH_FUNCTION,
    },
    OpMeta {
        op: ExpressionOp::Clamp,
        name: "Clamp",
        token: Some("math.clamp"),
        aliases: &[],
        friendly_name: "Clamp 'math.clamp'",
        min_children: 3,
        max_children: Some(3),
        flags: OpFlags::MATH_FUNCTION,
    },
    OpMeta {
        op: ExpressionOp::CopySign,
        name: "CopySign",
        token: Some("math.copy_sign"),
        aliases: &[],
        friendly_name: "Copy Sign 'math.copy_sign'",
        min_children: 2,
        max_children: Some(2),
        flags: OpFlags::MATH_FUNCTION,
    },
    OpMeta {
        op: ExpressionOp::Cos,
        name: "Cos",
        token: Some("math.cos"),
        aliases: &[],
        friendly_name: "Cosine 'math.cos'",
        min_children: 1,
        max_children: Some(1),
        flags: OpFlags::MATH_FUNCTION,
    },
    OpMeta {
        op: ExpressionOp::DieRoll,
        name: "DieRoll",
        token: Some("math.die_roll"),
        aliases: &[],
        friendly_name: "Die Roll 'math.die_roll'",
        min_children: 3,
        max_children: Some(3),
        flags: OpFlags::MATH_FUNCTION,
    },
    OpMeta {
        op: ExpressionOp::DieRollInt,
        name: "DieRollInt",
        token: Some("math.die_roll_integer"),
        aliases: &[],
        friendly_name: "Die Roll Integer 'math.die_roll_integer'",
        min_children: 3,
        max_children: Some(3),
        flags: OpFlags::MATH_FUNCTION,
    },
    OpMeta {
        op: ExpressionOp::Div,
        name: "Div",
        token: Some("/"),
        aliases: &[],
        friendly_name: "Divide '/'",
        min_children: 2,
        max_children: Some(2),
        flags: OpFlags::empty(),
    },
    OpMeta {
        op: ExpressionOp::Exp,
        name: "Exp",
        token: Some("math.exp"),
        aliases: &[],
        friendly_name: "Base-e Exponent 'math.exp'",
        min_children: 1,
        max_children: Some(1),
        flags: OpFlags::MATH_FUNCTION,
    },
    OpMeta {
        op: ExpressionOp::Floor,
        name: "Floor",
        token: Some("math.floor"),
        aliases: &[],
        friendly_name: "Floor 'math.floor'",
        min_children: 1,
        max_children: Some(1),
        flags: OpFlags::MATH_FUNCTION,
    },
    OpMeta {
        op: ExpressionOp::HermiteBlend,
        name: "HermiteBlend",
        token: Some("math.hermite_blend"),
        aliases: &[],
        friendly_name: "Hermite Blend 'math.hermite_blend'",
        min_children: 1,
        max_children: Some(1),
        flags: OpFlags::MATH_FUNCTION,
    },
    OpMeta {
        op: ExpressionOp::Lerp,
        name: "Lerp",
        token: Some("math.lerp"),
        aliases: &[],
        friendly_name: "Lerp 'math.lerp'",
        min_children: 3,
        max_children: Some(3),
        flags: OpFlags::MATH_FUNCTION,
    },
    OpMeta {
        op: ExpressionOp::LerpRotate,
        name: "LerpRotate",
        token: Some("math.lerprotate"),
        aliases: &[],
        friendly_name: "Lerp Rotate 'math.lerprotate'",
        min_children: 3,
        max_children: Some(3),
        flags: OpFlags::MATH_FUNCTION,
    },
    OpMeta {
        op: ExpressionOp::Ln,
        name: "Ln",
        token: Some("math.ln"),
        aliases: &[],
        friendly_name: "Natural Log 'math.ln'",
        min_children: 1,
        max_children: Some(1),
        flags: OpFlags::MATH_FUNCTION,
    },
    OpMeta {
        op: ExpressionOp::Max,
        name: "Max",
        token: Some("math.max"),
        aliases: &[],
        friendly_name: "Max 'math.max'",
        min_children: 2,
        max_children: Some(2),
        flags: OpFlags::MATH_FUNCTION,
    },
    OpMeta {
        op: ExpressionOp::Min,
        name: "Min",
        token: Some("math.min"),
        aliases: &[],
        friendly_name: "Min 'math.min'",
        min_children: 2,
        max_children: Some(2),
        flags: OpFlags::MATH_FUNCTION,
    },
    OpMeta {
        op: ExpressionOp::MinAngle,
        name: "MinAngle",
        token: Some("math.min_angle"),
        aliases: &[],
        friendly_name: "Min Angle 'math.min_angle'",
        min_children: 1,
        max_children: Some(1),
        flags: OpFlags::MATH_FUNCTION,
    },
    OpMeta {
        op: ExpressionOp::Mod,
        name: "Mod",
        token: Some("math.mod"),
        aliases: &[],
        friendly_name: "Mod 'math.mod'",
        min_children: 2,
        max_children: Some(2),
        flags: OpFlags::MATH_FUNCTION,
    },
    OpMeta {
        op: ExpressionOp::Mul,
        name: "Mul",
        token: Some("*"),
        aliases: &[],
        friendly_name: "Multiply '*'",
        min_children: 2,
        max_children: Some(2),
        flags: OpFlags::empty(),
    },
    OpMeta {
        op: ExpressionOp::Pow,
        name: "Pow",
        token: Some("math.pow"),
        aliases: &[],
        friendly_name: "Power 'math.pow'",
        min_children: 2,
        max_children: Some(2),
        flags: OpFlags::MATH_FUNCTION,
    },
    OpMeta {
        op: ExpressionOp::Random,
        name: "Random",
        token: Some("math.random"),
        aliases: &[],
        friendly_name: "Random 'math.random'",
        min_children: 2,
        max_children: Some(2),
        flags: OpFlags::MATH_FUNCTION.union(OpFlags::SIDE_EFFECT),
    },
    OpMeta {
        op: ExpressionOp::RandomInt,
        name: "RandomInt",
        token: Some("math.random_integer"),
        aliases: &[],
        friendly_name: "Random Integer 'math.random_integer'",
        min_children: 2,
        max_children: Some(2),
        flags: OpFlags::MATH_FUNCTION.union(OpFlags::SIDE_EFFECT),
    },
    OpMeta {
        op: ExpressionOp::Round,
        name: "Round",
        token: Some("math.round"),
        aliases: &[],
        friendly_name: "Round 'math.round'",
        min_children: 1,
        max_children: Some(1),
        flags: OpFlags::MATH_FUNCTION,
    },
    OpMeta {
        op: ExpressionOp::Sin,
        name: "Sin",
        token: Some("math.sin"),
        aliases: &[],
        friendly_name: "Sine 'math.sin'",
        min_children: 1,
        max_children: Some(1),
        flags: OpFlags::MATH_FUNCTION,
    },
    OpMeta {
        op: ExpressionOp::Sign,
        name: "Sign",
        token: Some("math.sign"),
        aliases: &[],
        friendly_name: "Sign 'math.sign'",
        min_children: 1,
        max_children: Some(1),
        flags: OpFlags::MATH_FUNCTION,
    },
    OpMeta {
        op: ExpressionOp::Sqrt,
        name: "Sqrt",
        token: Some("math.sqrt"),
        aliases: &[],
        friendly_name: "Square Root 'math.sqrt'",
        min_children: 1,
        max_children: Some(1),
        flags: OpFlags::MATH_FUNCTION,
    },
    OpMeta {
        op: ExpressionOp::Trunc,
        name: "Trunc",
        token: Some("math.trunc"),
        aliases: &[],
        friendly_name: "Truncate 'math.trunc'",
        min_children: 1,
        max_children: Some(1),
        flags: OpFlags::MATH_FUNCTION,
    },
    OpMeta {
        op: ExpressionOp::QueryFunction,
        name: "QueryFunction",
        token: Some("query."),
        aliases: &["query.", "q."],
        friendly_name: "Query Function 'query.' or 'q.'",
        min_children: 0,
        max_children: None,
        flags: OpFlags::empty(),
    },
    OpMeta {
        op: ExpressionOp::ArrayVariable,
        name: "ArrayVariable",
        token: Some("array."),
        aliases: &[],
        friendly_name: "Array Variable 'array.'",
        min_children: 0,
        max_children: Some(0),
        flags: OpFlags::empty(),
    },
    OpMeta {
        op: ExpressionOp::ContextVariable,
        name: "ContextVariable",
        token: Some("context."),
        aliases: &["context.", "c."],
        friendly_name: "Context Variable 'context.' or 'c.'",
        min_children: 0,
        max_children: Some(0),
        flags: OpFlags::empty(),
    },
    OpMeta {
        op: ExpressionOp::EntityVariable,
        name: "EntityVariable",
        token: Some("variable."),
        aliases: &["variable.", "v."],
        friendly_name: "Entity Variable 'variable.' or 'v.'",
        min_children: 0,
        max_children: Some(0),
        flags: OpFlags::empty(),
    },
    OpMeta {
        op: ExpressionOp::TempVariable,
        name: "TempVariable",
        token: Some("temp."),
        aliases: &["temp.", "t."],
        friendly_name: "Temp Variable 'temp.' or 't.'",
        min_children: 0,
        max_children: Some(0),
        flags: OpFlags::empty(),
    },
    OpMeta {
        op: ExpressionOp::MemberAccessor,
        name: "MemberAccessor",
        token: Some("."),
        aliases: &[],
        friendly_name: "Member Accessor '.'",
        min_children: 1,
        max_children: Some(1),
        flags: OpFlags::empty(),
    },
    OpMeta {
        op: ExpressionOp::StringLiteral,
        name: "StringLiteral",
        token: Some("'"),
        aliases: &[],
        friendly_name: "String '''",
        min_children: 0,
        max_children: Some(0),
        flags: OpFlags::empty(),
    },
    OpMeta {
        op: ExpressionOp::GeometryVariable,
        name: "GeometryVariable",
        token: Some("geometry."),
        aliases: &[],
        friendly_name: "Geometry Variable 'geometry.'",
        min_children: 0,
        max_children: Some(0),
        flags: OpFlags::RESOURCE_REFERENCE,
    },
    OpMeta {
        op: ExpressionOp::MaterialVariable,
        name: "MaterialVariable",
        token: Some("material."),
        aliases: &[],
        friendly_name: "Material Variable 'material.'",
        min_children: 0,
        max_children: Some(0),
        flags: OpFlags::RESOURCE_REFERENCE,
    },
    OpMeta {
        op: ExpressionOp::TextureVariable,
        name: "TextureVariable",
        token: Some("texture."),
        aliases: &[],
        friendly_name: "Texture Variable 'texture.'",
        min_children: 0,
        max_children: Some(0),
        flags: OpFlags::RESOURCE_REFERENCE,
    },
    OpMeta {
        op: ExpressionOp::LessThan,
        name: "LessThan",
        token: Some("<"),
        aliases: &[],
        friendly_name: "Less Than '<'",
        min_children: 2,
        max_children: Some(2),
        flags: OpFlags::empty(),
    },
    OpMeta {
        op: ExpressionOp::LessEqual,
        name: "LessEqual",
        token: Some("<="),
        aliases: &[],
        friendly_name: "Less Than Or Equal '<='",
        min_children: 2,
        max_children: Some(2),
        flags: OpFlags::empty(),
    },
    OpMeta {
        op: ExpressionOp::GreaterEqual,
        name: "GreaterEqual",
        token: Some(">="),
        aliases: &[],
        friendly_name: "Greater Than Or Equal '>='",
        min_children: 2,
        max_children: Some(2),
        flags: OpFlags::empty(),
    },
    OpMeta {
        op: ExpressionOp::GreaterThan,
        name: "GreaterThan",
        token: Some(">"),
        aliases: &[],
        friendly_name: "Greater Than '>'",
        min_children: 2,
        max_children: Some(2),
        flags: OpFlags::empty(),
    },
    OpMeta {
        op: ExpressionOp::LogicalEqual,
        name: "LogicalEqual",
        token: Some("=="),
        aliases: &[],
        friendly_name: "Logical Equal '=='",
        min_children: 2,
        max_children: Some(2),
        flags: OpFlags::empty(),
    },
    OpMeta {
        op: ExpressionOp::LogicalNotEqual,
        name: "LogicalNotEqual",
        token: Some("!="),
        aliases: &[],
        friendly_name: "Logical Not Equal '!='",
        min_children: 2,
        max_children: Some(2),
        flags: OpFlags::empty(),
    },
    OpMeta {
        op: ExpressionOp::LogicalOr,
        name: "LogicalOr",
        token: Some("||"),
        aliases: &[],
        friendly_name: "Logical Or '||'",
        min_children: 2,
        max_children: Some(2),
        flags: OpFlags::empty(),
    },
    OpMeta {
        op: ExpressionOp::LogicalAnd,
        name: "LogicalAnd",
        token: Some("&&"),
        aliases: &[],
        friendly_name: "Logical And '&&'",
        min_children: 2,
        max_children: Some(2),
        flags: OpFlags::empty(),
    },
    OpMeta {
        op: ExpressionOp::NullCoalescing,
        name: "NullCoalescing",
        token: Some("??"),
        aliases: &[],
        friendly_name: "Null Coalescing '??'",
        min_children: 2,
        max_children: Some(2),
        flags: OpFlags::empty(),
    },
    OpMeta {
        op: ExpressionOp::Conditional,
        name: "Conditional",
        token: Some("?"),
        aliases: &[],
        friendly_name: "Conditional '?'",
        min_children: 2,
        max_children: Some(3),
        flags: OpFlags::empty(),
    },
    OpMeta {
        op: ExpressionOp::ConditionalElse,
        name: "ConditionalElse",
        token: Some(":"),
        aliases: &[],
        friendly_name: "Conditional Else ':'",
        min_children: 2,
        max_children: Some(2),
        flags: OpFlags::empty(),
    },
    OpMeta {
        op: ExpressionOp::Float,
        name: "Float",
        token: None,
        aliases: &[],
        friendly_name: "Float",
        min_children: 0,
        max_children: Some(0),
        flags: OpFlags::empty(),
    },
    OpMeta {
        op: ExpressionOp::Pi,
        name: "Pi",
        token: Some("math.pi"),
        aliases: &[],
        friendly_name: "Pi",
        min_children: 0,
        max_children: Some(0),
        flags: OpFlags::MATH_FUNCTION,
    },
    OpMeta {
        op: ExpressionOp::Array,
        name: "Array",
        token: Some("[]"),
        aliases: &[],
        friendly_name: "Array '[]'",
        min_children: 1,
        max_children: Some(1),
        flags: OpFlags::empty(),
    },
    OpMeta {
        op: ExpressionOp::Geometry,
        name: "Geometry",
        token: None,
        aliases: &[],
        friendly_name: "Geometry reference",
        min_children: 0,
        max_children: Some(0),
        flags: OpFlags::empty(),
    },
    OpMeta {
        op: ExpressionOp::Material,
        name: "Material",
        token: None,
        aliases: &[],
        friendly_name: "Material reference",
        min_children: 0,
        max_children: Some(0),
        flags: OpFlags::empty(),
    },
    OpMeta {
        op: ExpressionOp::Texture,
        name: "Texture",
        token: None,
        aliases: &[],
        friendly_name: "Texture reference",
        min_children: 0,
        max_children: Some(0),
        flags: OpFlags::empty(),
    },
    OpMeta {
        op: ExpressionOp::Loop,
        name: "Loop",
        token: Some("loop"),
        aliases: &[],
        friendly_name: "Loop 'loop'",
        min_children: 2,
        max_children: Some(2),
        flags: OpFlags::empty(),
    },
    OpMeta {
        op: ExpressionOp::ForEach,
        name: "ForEach",
        token: Some("for_each"),
        aliases: &[],
        friendly_name: "For Each 'for_each'",
        min_children: 3,
        max_children: Some(3),
        flags: OpFlags::empty(),
    },
    OpMeta {
        op: ExpressionOp::Break,
        name: "Break",
        token: Some("break"),
        aliases: &[],
        friendly_name: "Break 'break'",
        min_children: 0,
        max_children: Some(0),
        flags: OpFlags::empty(),
    },
    OpMeta {
        op: ExpressionOp::Continue,
        name: "Continue",
        token: Some("continue"),
        aliases: &[],
        friendly_name: "Continue 'continue'",
        min_children: 0,
        max_children: Some(0),
        flags: OpFlags::empty(),
    },
    OpMeta {
        op: ExpressionOp::Assignment,
        name: "Assignment",
        token: Some("="),
        aliases: &[],
        friendly_name: "Assignment '='",
        min_children: 2,
        max_children: Some(2),
        flags: OpFlags::SIDE_EFFECT,
    },
    OpMeta {
        op: ExpressionOp::Pointer,
        name: "Pointer",
        token: Some("->"),
        aliases: &[],
        friendly_name: "Pointer '->'",
        min_children: 2,
        max_children: Some(2),
        flags: OpFlags::empty(),
    },
    OpMeta {
        op: ExpressionOp::Semicolon,
        name: "Semicolon",
        token: Some(";"),
        aliases: &[],
        friendly_name: "Semicolon ';'",
        min_children: 1,
        max_children: None,
        flags: OpFlags::empty(),
    },
    OpMeta {
        op: ExpressionOp::Return,
        name: "Return",
        token: Some("return"),
        aliases: &[],
        friendly_name: "Return 'return'",
        min_children: 1,
        max_children: Some(1),
        flags: OpFlags::empty(),
    },
    OpMeta {
        op: ExpressionOp::Comma,
        name: "Comma",
        token: Some(","),
        aliases: &[],
        friendly_name: "Comma ','",
        min_children: 0,
        max_children: Some(0),
        flags: OpFlags::empty(),
    },
    OpMeta {
        op: ExpressionOp::This,
        name: "This",
        token: Some("this"),
        aliases: &[],
        friendly_name: "This 'this'",
        min_children: 0,
        max_children: Some(0),
        flags: OpFlags::empty(),
    },
    OpMeta {
        op: ExpressionOp::ExpressionArray,
        name: "ExpressionArray",
        token: None,
        aliases: &[],
        friendly_name: "Expression array",
        min_children: 1,
        max_children: None,
        flags: OpFlags::empty(),
    },
    OpMeta {
        op: ExpressionOp::InverseLerp,
        name: "InverseLerp",
        token: Some("math.inverse_lerp"),
        aliases: &[],
        friendly_name: "Inverse Lerp 'math.inverse_lerp'",
        min_children: 3,
        max_children: Some(3),
        flags: OpFlags::MATH_FUNCTION,
    },
    OpMeta {
        op: ExpressionOp::EaseInQuad,
        name: "EaseInQuad",
        token: Some("math.ease_in_quad"),
        aliases: &[],
        friendly_name: "Ease In Quad 'math.ease_in_quad'",
        min_children: 3,
        max_children: Some(3),
        flags: OpFlags::MATH_FUNCTION,
    },
    OpMeta {
        op: ExpressionOp::EaseOutQuad,
        name: "EaseOutQuad",
        token: Some("math.ease_out_quad"),
        aliases: &[],
        friendly_name: "Ease Out Quad 'math.ease_out_quad'",
        min_children: 3,
        max_children: Some(3),
        flags: OpFlags::MATH_FUNCTION,
    },
    OpMeta {
        op: ExpressionOp::EaseInOutQuad,
        name: "EaseInOutQuad",
        token: Some("math.ease_in_out_quad"),
        aliases: &[],
        friendly_name: "Ease In Out Quad 'math.ease_in_out_quad'",
        min_children: 3,
        max_children: Some(3),
        flags: OpFlags::MATH_FUNCTION,
    },
    OpMeta {
        op: ExpressionOp::EaseInCubic,
        name: "EaseInCubic",
        token: Some("math.ease_in_cubic"),
        aliases: &[],
        friendly_name: "Ease In Cubic 'math.ease_in_cubic'",
        min_children: 3,
        max_children: Some(3),
        flags: OpFlags::MATH_FUNCTION,
    },
    OpMeta {
        op: ExpressionOp::EaseOutCubic,
        name: "EaseOutCubic",
        token: Some("math.ease_out_cubic"),
        aliases: &[],
        friendly_name: "Ease Out Cubic 'math.ease_out_cubic'",
        min_children: 3,
        max_children: Some(3),
        flags: OpFlags::MATH_FUNCTION,
    },
    OpMeta {
        op: ExpressionOp::EaseInOutCubic,
        name: "EaseInOutCubic",
        token: Some("math.ease_in_out_cubic"),
        aliases: &[],
        friendly_name: "Ease In Out Cubic 'math.ease_in_out_cubic'",
        min_children: 3,
        max_children: Some(3),
        flags: OpFlags::MATH_FUNCTION,
    },
    OpMeta {
        op: ExpressionOp::EaseInQuart,
        name: "EaseInQuart",
        token: Some("math.ease_in_quart"),
        aliases: &[],
        friendly_name: "Ease In Quart 'math.ease_in_quart'",
        min_children: 3,
        max_children: Some(3),
        flags: OpFlags::MATH_FUNCTION,
    },
    OpMeta {
        op: ExpressionOp::EaseOutQuart,
        name: "EaseOutQuart",
        token: Some("math.ease_out_quart"),
        aliases: &[],
        friendly_name: "Ease Out Quart 'math.ease_out_quart'",
        min_children: 3,
        max_children: Some(3),
        flags: OpFlags::MATH_FUNCTION,
    },
    OpMeta {
        op: ExpressionOp::EaseInOutQuart,
        name: "EaseInOutQuart",
        token: Some("math.ease_in_out_quart"),
        aliases: &[],
        friendly_name: "Ease In Out Quart 'math.ease_in_out_quart'",
        min_children: 3,
        max_children: Some(3),
        flags: OpFlags::MATH_FUNCTION,
    },
    OpMeta {
        op: ExpressionOp::EaseInQuint,
        name: "EaseInQuint",
        token: Some("math.ease_in_quint"),
        aliases: &[],
        friendly_name: "Ease In Quint 'math.ease_in_quint'",
        min_children: 3,
        max_children: Some(3),
        flags: OpFlags::MATH_FUNCTION,
    },
    OpMeta {
        op: ExpressionOp::EaseOutQuint,
        name: "EaseOutQuint",
        token: Some("math.ease_out_quint"),
        aliases: &[],
        friendly_name: "Ease Out Quint 'math.ease_out_quint'",
        min_children: 3,
        max_children: Some(3),
        flags: OpFlags::MATH_FUNCTION,
    },
    OpMeta {
        op: ExpressionOp::EaseInOutQuint,
        name: "EaseInOutQuint",
        token: Some("math.ease_in_out_quint"),
        aliases: &[],
        friendly_name: "Ease In Out Quint 'math.ease_in_out_quint'",
        min_children: 3,
        max_children: Some(3),
        flags: OpFlags::MATH_FUNCTION,
    },
    OpMeta {
        op: ExpressionOp::EaseInSine,
        name: "EaseInSine",
        token: Some("math.ease_in_sine"),
        aliases: &[],
        friendly_name: "Ease In Sine 'math.ease_in_sine'",
        min_children: 3,
        max_children: Some(3),
        flags: OpFlags::MATH_FUNCTION,
    },
    OpMeta {
        op: ExpressionOp::EaseOutSine,
        name: "EaseOutSine",
        token: Some("math.ease_out_sine"),
        aliases: &[],
        friendly_name: "Ease Out Sine 'math.ease_out_sine'",
        min_children: 3,
        max_children: Some(3),
        flags: OpFlags::MATH_FUNCTION,
    },
    OpMeta {
        op: ExpressionOp::EaseInOutSine,
        name: "EaseInOutSine",
        token: Some("math.ease_in_out_sine"),
        aliases: &[],
        friendly_name: "Ease In Out Sine 'math.ease_in_out_sine'",
        min_children: 3,
        max_children: Some(3),
        flags: OpFlags::MATH_FUNCTION,
    },
    OpMeta {
        op: ExpressionOp::EaseInExpo,
        name: "EaseInExpo",
        token: Some("math.ease_in_expo"),
        aliases: &[],
        friendly_name: "Ease In Expo 'math.ease_in_expo'",
        min_children: 3,
        max_children: Some(3),
        flags: OpFlags::MATH_FUNCTION,
    },
    OpMeta {
        op: ExpressionOp::EaseOutExpo,
        name: "EaseOutExpo",
        token: Some("math.ease_out_expo"),
        aliases: &[],
        friendly_name: "Ease Out Expo 'math.ease_out_expo'",
        min_children: 3,
        max_children: Some(3),
        flags: OpFlags::MATH_FUNCTION,
    },
    OpMeta {
        op: ExpressionOp::EaseInOutExpo,
        name: "EaseInOutExpo",
        token: Some("math.ease_in_out_expo"),
        aliases: &[],
        friendly_name: "Ease In Out Expo 'math.ease_in_out_expo'",
        min_children: 3,
        max_children: Some(3),
        flags: OpFlags::MATH_FUNCTION,
    },
    OpMeta {
        op: ExpressionOp::EaseInCirc,
        name: "EaseInCirc",
        token: Some("math.ease_in_circ"),
        aliases: &[],
        friendly_name: "Ease In Circ 'math.ease_in_circ'",
        min_children: 3,
        max_children: Some(3),
        flags: OpFlags::MATH_FUNCTION,
    },
    OpMeta {
        op: ExpressionOp::EaseOutCirc,
        name: "EaseOutCirc",
        token: Some("math.ease_out_circ"),
        aliases: &[],
        friendly_name: "Ease Out Circ 'math.ease_out_circ'",
        min_children: 3,
        max_children: Some(3),
        flags: OpFlags::MATH_FUNCTION,
    },
    OpMeta {
        op: ExpressionOp::EaseInOutCirc,
        name: "EaseInOutCirc",
        token: Some("math.ease_in_out_circ"),
        aliases: &[],
        friendly_name: "Ease In Out Circ 'math.ease_in_out_circ'",
        min_children: 3,
        max_children: Some(3),
        flags: OpFlags::MATH_FUNCTION,
    },
    OpMeta {
        op: ExpressionOp::EaseInBounce,
        name: "EaseInBounce",
        token: Some("math.ease_in_bounce"),
        aliases: &[],
        friendly_name: "Ease In Bounce 'math.ease_in_bounce'",
        min_children: 3,
        max_children: Some(3),
        flags: OpFlags::MATH_FUNCTION,
    },
    OpMeta {
        op: ExpressionOp::EaseOutBounce,
        name: "EaseOutBounce",
        token: Some("math.ease_out_bounce"),
        aliases: &[],
        friendly_name: "Ease Out Bounce 'math.ease_out_bounce'",
        min_children: 3,
        max_children: Some(3),
        flags: OpFlags::MATH_FUNCTION,
    },
    OpMeta {
        op: ExpressionOp::EaseInOutBounce,
        name: "EaseInOutBounce",
        token: Some("math.ease_in_out_bounce"),
        aliases: &[],
        friendly_name: "Ease In Out Bounce 'math.ease_in_out_bounce'",
        min_children: 3,
        max_children: Some(3),
        flags: OpFlags::MATH_FUNCTION,
    },
    OpMeta {
        op: ExpressionOp::EaseInBack,
        name: "EaseInBack",
        token: Some("math.ease_in_back"),
        aliases: &[],
        friendly_name: "Ease In Back 'math.ease_in_back'",
        min_children: 3,
        max_children: Some(3),
        flags: OpFlags::MATH_FUNCTION,
    },
    OpMeta {
        op: ExpressionOp::EaseOutBack,
        name: "EaseOutBack",
        token: Some("math.ease_out_back"),
        aliases: &[],
        friendly_name: "Ease Out Back 'math.ease_out_back'",
        min_children: 3,
        max_children: Some(3),
        flags: OpFlags::MATH_FUNCTION,
    },
    OpMeta {
        op: ExpressionOp::EaseInOutBack,
        name: "EaseInOutBack",
        token: Some("math.ease_in_out_back"),
        aliases: &[],
        friendly_name: "Ease In Out Back 'math.ease_in_out_back'",
        min_children: 3,
        max_children: Some(3),
        flags: OpFlags::MATH_FUNCTION,
    },
    OpMeta {
        op: ExpressionOp::EaseInElastic,
        name: "EaseInElastic",
        token: Some("math.ease_in_elastic"),
        aliases: &[],
        friendly_name: "Ease In Elastic 'math.ease_in_elastic'",
        min_children: 3,
        max_children: Some(3),
        flags: OpFlags::MATH_FUNCTION,
    },
    OpMeta {
        op: ExpressionOp::EaseOutElastic,
        name: "EaseOutElastic",
        token: Some("math.ease_out_elastic"),
        aliases: &[],
        friendly_name: "Ease Out Elastic 'math.ease_out_elastic'",
        min_children: 3,
        max_children: Some(3),
        flags: OpFlags::MATH_FUNCTION,
    },
    OpMeta {
        op: ExpressionOp::EaseInOutElastic,
        name: "EaseInOutElastic",
        token: Some("math.ease_in_out_elastic"),
        aliases: &[],
        friendly_name: "Ease In Out Elastic 'math.ease_in_out_elastic'",
        min_children: 3,
        max_children: Some(3),
        flags: OpFlags::MATH_FUNCTION,
    },
    OpMeta {
        op: ExpressionOp::HostMath,
        name: "HostMath",
        token: None,
        aliases: &[],
        friendly_name: "Host Math Function",
        min_children: 1,
        max_children: Some(8),
        flags: OpFlags::empty(),
    },
    OpMeta {
        op: ExpressionOp::HostMathVolatile,
        name: "HostMathVolatile",
        token: None,
        aliases: &[],
        friendly_name: "Volatile Host Math Function",
        min_children: 1,
        max_children: Some(8),
        flags: OpFlags::SIDE_EFFECT,
    },
];

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, BTreeSet};

    use super::*;

    #[test]
    #[allow(clippy::too_many_lines, reason = "one row per checked op")]
    fn op_table_spot_checks() {
        use ExpressionOp as Op;
        let rows = [
            (Op::LeftBrace, 0, Some("{"), "Left Brace '{'", 1, None),
            (Op::Negate, 6, Some("-"), "Negate '-'", 1, Some(1)),
            (Op::Add, 9, Some("+"), "Add '+'", 2, None),
            (
                Op::Atan2,
                13,
                Some("math.atan2"),
                "atan2 'math.atan2'",
                2,
                Some(2),
            ),
            (Op::Div, 20, Some("/"), "Divide '/'", 2, Some(2)),
            (
                Op::Random,
                33,
                Some("math.random"),
                "Random 'math.random'",
                2,
                Some(2),
            ),
            (
                Op::RandomInt,
                34,
                Some("math.random_integer"),
                "Random Integer 'math.random_integer'",
                2,
                Some(2),
            ),
            (
                Op::QueryFunction,
                40,
                Some("query."),
                "Query Function 'query.' or 'q.'",
                0,
                None,
            ),
            (Op::StringLiteral, 46, Some("'"), "String '''", 0, Some(0)),
            (Op::LogicalOr, 56, Some("||"), "Logical Or '||'", 2, Some(2)),
            (
                Op::Conditional,
                59,
                Some("?"),
                "Conditional '?'",
                2,
                Some(3),
            ),
            (Op::Float, 61, None, "Float", 0, Some(0)),
            (Op::Pi, 62, Some("math.pi"), "Pi", 0, Some(0)),
            (Op::Array, 63, Some("[]"), "Array '[]'", 1, Some(1)),
            (Op::Geometry, 64, None, "Geometry reference", 0, Some(0)),
            (Op::Assignment, 71, Some("="), "Assignment '='", 2, Some(2)),
            (Op::Pointer, 72, Some("->"), "Pointer '->'", 2, Some(2)),
            (Op::Semicolon, 73, Some(";"), "Semicolon ';'", 1, None),
            (Op::ExpressionArray, 77, None, "Expression array", 1, None),
            (
                Op::InverseLerp,
                78,
                Some("math.inverse_lerp"),
                "Inverse Lerp 'math.inverse_lerp'",
                3,
                Some(3),
            ),
            (
                Op::EaseInOutElastic,
                108,
                Some("math.ease_in_out_elastic"),
                "Ease In Out Elastic 'math.ease_in_out_elastic'",
                3,
                Some(3),
            ),
            (Op::HostMath, 109, None, "Host Math Function", 1, Some(8)),
            (
                Op::HostMathVolatile,
                110,
                None,
                "Volatile Host Math Function",
                1,
                Some(8),
            ),
        ];
        for (op, ordinal, token, friendly, min, max) in rows {
            assert_eq!(
                (
                    op.ordinal(),
                    op.token(),
                    op.friendly_name(),
                    op.min_children(),
                    op.max_children()
                ),
                (ordinal, token, friendly, min, max),
                "{op:?}"
            );
        }
        assert_eq!(Op::QueryFunction.meta().aliases, ["query.", "q."]);
        assert_eq!(Op::EntityVariable.meta().aliases, ["variable.", "v."]);
        let resources: Vec<Op> = Op::all()
            .iter()
            .copied()
            .filter(|op| op.is_resource_reference())
            .collect();
        assert_eq!(
            resources,
            [
                Op::GeometryVariable,
                Op::MaterialVariable,
                Op::TextureVariable
            ]
        );
    }

    #[test]
    fn ops_table_is_dense_and_consistent() {
        assert_eq!(ExpressionOp::COUNT, 111);
        assert_eq!(ExpressionOp::all().len(), 111);
        assert_eq!(OP_META.len(), 111);
        for (i, &op) in ExpressionOp::all().iter().enumerate() {
            assert_eq!(usize::from(op.ordinal()), i);
            assert_eq!(OP_META[i].op, op);
            assert!(std::ptr::eq(op.meta(), std::ptr::from_ref(&OP_META[i])));
            assert_eq!(
                ExpressionOp::from_ordinal(u8::try_from(i).unwrap()),
                Some(op)
            );
            assert_eq!(format!("{op:?}"), op.meta().name);
        }
    }

    #[test]
    fn op_rows_are_well_formed() {
        let mut names = BTreeSet::new();
        let mut friendly = BTreeSet::new();
        for meta in &OP_META {
            assert!(!meta.name.is_empty());
            assert!(!meta.friendly_name.is_empty(), "{}", meta.name);
            assert!(names.insert(meta.name), "{} repeated", meta.name);
            assert!(
                friendly.insert(meta.friendly_name),
                "{} repeated",
                meta.friendly_name
            );
            assert!(
                meta.min_children <= meta.max_children.unwrap_or(u8::MAX),
                "{}",
                meta.name
            );
            if let Some(token) = meta.token {
                assert!(!token.is_empty(), "{}", meta.name);
                if !meta.aliases.is_empty() {
                    assert!(
                        meta.aliases.contains(&token),
                        "{}: aliases omit the token",
                        meta.name
                    );
                }
            } else {
                assert!(meta.aliases.is_empty(), "{}", meta.name);
            }
        }
    }

    #[test]
    fn tokens_are_unique_among_the_ops_that_have_one() {
        let mut seen = BTreeMap::new();
        for meta in &OP_META {
            if let Some(token) = meta.token
                && let Some(previous) = seen.insert(token, meta.name)
            {
                panic!("token {token:?} is shared by {previous} and {}", meta.name);
            }
        }
        assert_eq!(seen.len(), 111 - 7);
    }

    #[test]
    fn flag_invariants() {
        let side_effects: Vec<u8> = ExpressionOp::all()
            .iter()
            .filter(|op| op.meta().flags.contains(OpFlags::SIDE_EFFECT))
            .map(|op| op.ordinal())
            .collect();
        assert_eq!(side_effects, [33, 34, 71, 110]);
        let resources: Vec<u8> = ExpressionOp::all()
            .iter()
            .filter(|op| op.is_resource_reference())
            .map(|op| op.ordinal())
            .collect();
        assert_eq!(resources, [47, 48, 49]);
        assert_eq!(
            [47, 48, 49].map(|n| ExpressionOp::from_ordinal(n).and_then(ExpressionOp::token)),
            [Some("geometry."), Some("material."), Some("texture.")]
        );
        assert_eq!(
            ExpressionOp::all()
                .iter()
                .filter(|op| op.is_math_function())
                .count(),
            61
        );
        for op in ExpressionOp::all() {
            assert!(
                !(op.is_math_function() && op.is_resource_reference()),
                "{op:?}"
            );
        }
    }

    #[test]
    fn op_flag_bits() {
        assert_eq!(OpFlags::empty().bits(), 0);
        assert_eq!(OpFlags::MATH_FUNCTION.bits(), 1);
        assert_eq!(OpFlags::RESOURCE_REFERENCE.bits(), 2);
        assert_eq!(OpFlags::SIDE_EFFECT.bits(), 4);
        assert_eq!(OpFlags::default(), OpFlags::empty());
    }

    #[test]
    fn op_flag_union_and_contains() {
        let both = OpFlags::MATH_FUNCTION.union(OpFlags::SIDE_EFFECT);
        assert!(both.contains(OpFlags::SIDE_EFFECT));
        assert!(both.contains(OpFlags::MATH_FUNCTION));
        assert!(both.contains(both));
        assert!(!both.contains(OpFlags::RESOURCE_REFERENCE));
        assert!(!OpFlags::MATH_FUNCTION.contains(OpFlags::RESOURCE_REFERENCE));
        assert!(OpFlags::empty().contains(OpFlags::empty()));
        assert!(OpFlags::SIDE_EFFECT.contains(OpFlags::empty()));
        assert_eq!(both.bits(), 5);
        assert_eq!(both.union(both), both);
    }

    #[test]
    fn op_flag_debug_lists_the_names_in_bit_order() {
        assert_eq!(format!("{:?}", OpFlags::empty()), "{}");
        assert_eq!(
            format!("{:?}", OpFlags::MATH_FUNCTION.union(OpFlags::SIDE_EFFECT)),
            "{MathFunction, SideEffect}"
        );
        let all = OpFlags::SIDE_EFFECT
            .union(OpFlags::RESOURCE_REFERENCE)
            .union(OpFlags::MATH_FUNCTION);
        assert_eq!(
            format!("{all:?}"),
            "{MathFunction, ResourceReference, SideEffect}"
        );
        assert_eq!(format!("{:?}", OpFlags::from_bits_retain(0x80)), "{0x80}");
        assert_eq!(
            format!(
                "{:?}",
                OpFlags::SIDE_EFFECT | OpFlags::from_bits_retain(0x18)
            ),
            "{SideEffect, 0x18}"
        );
    }
}
