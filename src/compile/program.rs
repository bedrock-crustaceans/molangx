//! [`Program`]: the bytecode a compiled expression runs.
//!
//! A linear program for a machine with an **accumulator** (the result of the last value-producing
//! instruction) and an **operand stack**. The shape of the program decides the order of side
//! effects and random draws, and some run-time results: a `for_each` over a number or a string
//! skips the instruction after it, and a `break` in some operand positions ends a `loop` early.
//!
//! - every instruction is 8 bytes ([`Instr`]): an opcode and at most three small operands that
//!   index the program's tables;
//! - every value-producing instruction carries the node's post-op as an index into the post-op
//!   pool, where 0 is the identity;
//! - jump targets are absolute instruction indices;
//! - the arguments of a query call are sub-programs in the same code array, each ending in
//!   [`Instr::End`]: queries receive their arguments **unevaluated** and run them on demand;
//! - `break` / `continue` are plain jumps resolved at build time.
//!
//! A program is immutable and `Send + Sync`; [`crate::compile::Expr`] shares it behind an `Arc`.
use crate::catalog::{MathCatalog, MathRef, QueryCatalog, QueryIndex};
use crate::hash::HashedStr;
use crate::numeric::{self, PostOp};
use crate::ops::ExpressionOp as Op;
#[cfg(feature = "stdlib")]
use crate::stdlib::math;
use crate::version::MolangVersion;
#[cfg(feature = "vm")]
use crate::vm::{ContextName, TempName, VariableName};
use std::fmt::{self, Write as _};

/// Index into [`Program::posts`]; [`PostIdx::PLAIN`] (0) is the identity.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) struct PostIdx(pub(crate) u16);

impl PostIdx {
    /// The identity post-op: the plain instruction form.
    pub(crate) const PLAIN: Self = Self(0);
}

/// Index into [`Program::consts`].
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) struct ConstIdx(pub(crate) u32);

/// Index into [`Program::hashes`].
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) struct HashIdx(pub(crate) u32);

/// Index into [`Program::names`].
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) struct NameIdx(pub(crate) u16);

/// A temp slot: an index into [`Program::temps`].
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) struct TempIdx(pub(crate) u16);

/// Index into [`Program::members`].
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) struct MemberIdx(pub(crate) u16);

/// Index into [`Program::stores`].
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) struct StoreIdx(pub(crate) u16);

/// Index into [`Program::calls`].
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) struct CallIdx(pub(crate) u16);

/// A one-argument math function (`Math1`).
#[cfg(feature = "stdlib")]
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) enum Fn1 {
    Abs,
    Acos,
    Asin,
    Atan,
    Ceil,
    Cos,
    Exp,
    Floor,
    HermiteBlend,
    Ln,
    MinAngle,
    Round,
    Sin,
    Sign,
    Sqrt,
    Trunc,
}

/// A two-argument math function (`Math2`, `Math2Const`).
#[cfg(feature = "stdlib")]
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) enum Fn2 {
    Atan2,
    CopySign,
    Max,
    Min,
    Pow,
}

/// A three-argument math function (`Math3`): `clamp`, the interpolations and the 30 easings.
#[cfg(feature = "stdlib")]
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) enum Fn3 {
    Clamp,
    Lerp,
    LerpRotate,
    InverseLerp,
    EaseInQuad,
    EaseOutQuad,
    EaseInOutQuad,
    EaseInCubic,
    EaseOutCubic,
    EaseInOutCubic,
    EaseInQuart,
    EaseOutQuart,
    EaseInOutQuart,
    EaseInQuint,
    EaseOutQuint,
    EaseInOutQuint,
    EaseInSine,
    EaseOutSine,
    EaseInOutSine,
    EaseInExpo,
    EaseOutExpo,
    EaseInOutExpo,
    EaseInCirc,
    EaseOutCirc,
    EaseInOutCirc,
    EaseInBounce,
    EaseOutBounce,
    EaseInOutBounce,
    EaseInBack,
    EaseOutBack,
    EaseInOutBack,
    EaseInElastic,
    EaseOutElastic,
    EaseInOutElastic,
}

/// An ordered comparison.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) enum CmpOp {
    Lt,
    Le,
    Ge,
    Gt,
}

#[cfg(feature = "stdlib")]
impl Fn1 {
    /// The function of the `math.*` op, if it is one of these.
    pub(crate) const fn of(op: Op) -> Option<Self> {
        Some(match op {
            Op::Abs => Self::Abs,
            Op::Acos => Self::Acos,
            Op::Asin => Self::Asin,
            Op::Atan => Self::Atan,
            Op::Ceil => Self::Ceil,
            Op::Cos => Self::Cos,
            Op::Exp => Self::Exp,
            Op::Floor => Self::Floor,
            Op::HermiteBlend => Self::HermiteBlend,
            Op::Ln => Self::Ln,
            Op::MinAngle => Self::MinAngle,
            Op::Round => Self::Round,
            Op::Sin => Self::Sin,
            Op::Sign => Self::Sign,
            Op::Sqrt => Self::Sqrt,
            Op::Trunc => Self::Trunc,
            _ => return None,
        })
    }

    /// `f(x)` with the post-op.
    #[inline]
    pub(crate) fn apply(self, x: f32, post: PostOp) -> f32 {
        match self {
            Self::Abs => math::abs(x, post),
            Self::Acos => math::acos(x, post),
            Self::Asin => math::asin(x, post),
            Self::Atan => math::atan(x, post),
            Self::Ceil => math::ceil(x, post),
            Self::Cos => math::cos(x, post),
            Self::Exp => math::exp(x, post),
            Self::Floor => math::floor(x, post),
            Self::HermiteBlend => math::hermite_blend(x, post),
            Self::Ln => math::ln(x, post),
            Self::MinAngle => math::min_angle(x, post),
            Self::Round => math::round(x, post),
            Self::Sin => math::sin(x, post),
            Self::Sign => math::sign(x, post),
            Self::Sqrt => math::sqrt(x, post),
            Self::Trunc => math::trunc(x, post),
        }
    }
}

#[cfg(feature = "stdlib")]
impl Fn2 {
    /// The function of the `math.*` op, if it is one of these.
    pub(crate) const fn of(op: Op) -> Option<Self> {
        Some(match op {
            Op::Atan2 => Self::Atan2,
            Op::CopySign => Self::CopySign,
            Op::Max => Self::Max,
            Op::Min => Self::Min,
            Op::Pow => Self::Pow,
            _ => return None,
        })
    }

    /// `f(a, b)` with the post-op.
    #[inline]
    pub(crate) fn apply(self, a: f32, b: f32, post: PostOp) -> f32 {
        match self {
            Self::Atan2 => math::atan2(a, b, post),
            Self::CopySign => math::copy_sign(a, b, post),
            Self::Max => math::max(a, b, post),
            Self::Min => math::min(a, b, post),
            Self::Pow => math::pow(a, b, post),
        }
    }
}

#[cfg(feature = "stdlib")]
impl Fn3 {
    /// The function of the `math.*` op, if it is one of these.
    pub(crate) const fn of(op: Op) -> Option<Self> {
        Some(match op {
            Op::Clamp => Self::Clamp,
            Op::Lerp => Self::Lerp,
            Op::LerpRotate => Self::LerpRotate,
            Op::InverseLerp => Self::InverseLerp,
            Op::EaseInQuad => Self::EaseInQuad,
            Op::EaseOutQuad => Self::EaseOutQuad,
            Op::EaseInOutQuad => Self::EaseInOutQuad,
            Op::EaseInCubic => Self::EaseInCubic,
            Op::EaseOutCubic => Self::EaseOutCubic,
            Op::EaseInOutCubic => Self::EaseInOutCubic,
            Op::EaseInQuart => Self::EaseInQuart,
            Op::EaseOutQuart => Self::EaseOutQuart,
            Op::EaseInOutQuart => Self::EaseInOutQuart,
            Op::EaseInQuint => Self::EaseInQuint,
            Op::EaseOutQuint => Self::EaseOutQuint,
            Op::EaseInOutQuint => Self::EaseInOutQuint,
            Op::EaseInSine => Self::EaseInSine,
            Op::EaseOutSine => Self::EaseOutSine,
            Op::EaseInOutSine => Self::EaseInOutSine,
            Op::EaseInExpo => Self::EaseInExpo,
            Op::EaseOutExpo => Self::EaseOutExpo,
            Op::EaseInOutExpo => Self::EaseInOutExpo,
            Op::EaseInCirc => Self::EaseInCirc,
            Op::EaseOutCirc => Self::EaseOutCirc,
            Op::EaseInOutCirc => Self::EaseInOutCirc,
            Op::EaseInBounce => Self::EaseInBounce,
            Op::EaseOutBounce => Self::EaseOutBounce,
            Op::EaseInOutBounce => Self::EaseInOutBounce,
            Op::EaseInBack => Self::EaseInBack,
            Op::EaseOutBack => Self::EaseOutBack,
            Op::EaseInOutBack => Self::EaseInOutBack,
            Op::EaseInElastic => Self::EaseInElastic,
            Op::EaseOutElastic => Self::EaseOutElastic,
            Op::EaseInOutElastic => Self::EaseInOutElastic,
            _ => return None,
        })
    }

    /// `f(a, b, c)` with the post-op.
    #[inline]
    pub(crate) fn apply(self, a: f32, b: f32, c: f32, post: PostOp) -> f32 {
        match self {
            Self::Clamp => math::clamp(a, b, c, post),
            Self::Lerp => math::lerp(a, b, c, post),
            Self::LerpRotate => math::lerprotate(a, b, c, post),
            Self::InverseLerp => math::inverse_lerp(a, b, c, post),
            Self::EaseInQuad => math::ease_in_quad(a, b, c, post),
            Self::EaseOutQuad => math::ease_out_quad(a, b, c, post),
            Self::EaseInOutQuad => math::ease_in_out_quad(a, b, c, post),
            Self::EaseInCubic => math::ease_in_cubic(a, b, c, post),
            Self::EaseOutCubic => math::ease_out_cubic(a, b, c, post),
            Self::EaseInOutCubic => math::ease_in_out_cubic(a, b, c, post),
            Self::EaseInQuart => math::ease_in_quart(a, b, c, post),
            Self::EaseOutQuart => math::ease_out_quart(a, b, c, post),
            Self::EaseInOutQuart => math::ease_in_out_quart(a, b, c, post),
            Self::EaseInQuint => math::ease_in_quint(a, b, c, post),
            Self::EaseOutQuint => math::ease_out_quint(a, b, c, post),
            Self::EaseInOutQuint => math::ease_in_out_quint(a, b, c, post),
            Self::EaseInSine => math::ease_in_sine(a, b, c, post),
            Self::EaseOutSine => math::ease_out_sine(a, b, c, post),
            Self::EaseInOutSine => math::ease_in_out_sine(a, b, c, post),
            Self::EaseInExpo => math::ease_in_expo(a, b, c, post),
            Self::EaseOutExpo => math::ease_out_expo(a, b, c, post),
            Self::EaseInOutExpo => math::ease_in_out_expo(a, b, c, post),
            Self::EaseInCirc => math::ease_in_circ(a, b, c, post),
            Self::EaseOutCirc => math::ease_out_circ(a, b, c, post),
            Self::EaseInOutCirc => math::ease_in_out_circ(a, b, c, post),
            Self::EaseInBounce => math::ease_in_bounce(a, b, c, post),
            Self::EaseOutBounce => math::ease_out_bounce(a, b, c, post),
            Self::EaseInOutBounce => math::ease_in_out_bounce(a, b, c, post),
            Self::EaseInBack => math::ease_in_back(a, b, c, post),
            Self::EaseOutBack => math::ease_out_back(a, b, c, post),
            Self::EaseInOutBack => math::ease_in_out_back(a, b, c, post),
            Self::EaseInElastic => math::ease_in_elastic(a, b, c, post),
            Self::EaseOutElastic => math::ease_out_elastic(a, b, c, post),
            Self::EaseInOutElastic => math::ease_in_out_elastic(a, b, c, post),
        }
    }
}

impl CmpOp {
    /// The comparison of `<`, `<=`, `>=` or `>`.
    pub(crate) const fn of(op: Op) -> Option<Self> {
        Some(match op {
            Op::LessThan => Self::Lt,
            Op::LessEqual => Self::Le,
            Op::GreaterEqual => Self::Ge,
            Op::GreaterThan => Self::Gt,
            _ => return None,
        })
    }

    /// Whether `a <op> b` holds by this build's comparisons.
    #[inline]
    pub(crate) fn holds(self, a: f32, b: f32) -> bool {
        match self {
            Self::Lt => numeric::lt(a, b),
            Self::Le => numeric::le(a, b),
            Self::Ge => numeric::ge(a, b),
            Self::Gt => numeric::gt(a, b),
        }
    }
}

/// `==` or `!=`.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) enum EqOp {
    Eq,
    Ne,
}

impl EqOp {
    /// The test of `==` or `!=`.
    pub(crate) const fn of(op: Op) -> Option<Self> {
        match op {
            Op::LogicalEqual => Some(Self::Eq),
            Op::LogicalNotEqual => Some(Self::Ne),
            _ => None,
        }
    }

    /// The outcome of the test for operands that are `equal` or not.
    #[inline]
    pub(crate) fn holds(self, equal: bool) -> bool {
        match self {
            Self::Eq => equal,
            Self::Ne => !equal,
        }
    }
}

/// What the division guard pushes for a divisor `d` it lets through.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub(crate) enum Divisor {
    /// `d` (version ≥ 7).
    Signed,
    /// `|d|`.
    Absolute,
}

/// One instruction: 8 bytes.
///
/// `acc` is the accumulator, `top` the most recently pushed operand. `p` is the node's post-op;
/// `c`, `h`, `n`, `t`, `m`, `s` and `q` index the program's constants, hashes, names, temp slots,
/// members, member stores and query calls. Jump operands are absolute instruction indices.
#[derive(Copy, Clone, Debug, PartialEq)]
pub(crate) enum Instr {
    /// `acc = consts[c]` (a `Float` node, or a constant with its post-op applied at build time).
    Const { c: ConstIdx },
    /// `acc = hashes[h]`, a string (no post-op).
    Hash { h: HashIdx },
    /// `acc = resource hashes[h]` (`geometry.` / `material.` / `texture.` after resolution).
    Resource { h: HashIdx },
    /// `acc = post(this)`.
    This { p: PostIdx },
    /// `acc = post(variable.<n>)`; missing → the missing-variable path; inside `->` the public
    /// snapshot, missing reading 0.
    LoadVar { n: NameIdx, p: PostIdx },
    /// `acc = post(temp slot t)`.
    LoadTemp { t: TempIdx, p: PostIdx },
    /// `acc = post(context.<n>)`.
    LoadCtx { n: NameIdx, p: PostIdx },
    /// `acc = post(acc.<member m>)`.
    Member { m: MemberIdx, p: PostIdx },
    /// `variable.<n> = acc`; then `acc = post(acc)`.
    StoreVar { n: NameIdx, p: PostIdx },
    /// `temp slot t = acc`; then `acc = post(acc)`.
    StoreTemp { t: TempIdx, p: PostIdx },
    /// `stores[s] = acc` (a member path), creating intermediate structs.
    StoreMember { s: StoreIdx, p: PostIdx },
    /// Push `acc`.
    Push,
    /// `Push` then `Const { c }` in one dispatch. It costs the pair's two steps, the second
    /// charged between the halves, so a budget ends the evaluation where the pair would.
    PushConst { c: ConstIdx },
    /// `acc = O − acc·S`.
    Negate { p: PostIdx },
    /// `acc = acc ? F : T`.
    Not { p: PostIdx },
    /// `top = top + acc`.
    AddAcc,
    /// `acc = post(acc + pop)`.
    AddLast { p: PostIdx },
    /// `acc = mul(acc, pop)`.
    Mul { p: PostIdx },
    /// The division guard on the divisor in `acc`: below `f32::EPSILON` (2^-23)
    /// `acc = 0` and jump to `end`; otherwise push the `divisor`.
    DivGuard { end: u32, divisor: Divisor },
    /// `acc = div(acc, pop)`.
    Div { p: PostIdx },
    /// `acc = math.mod(pop, acc)` with a run-time divisor.
    #[cfg(feature = "stdlib")]
    Mod { p: PostIdx },
    /// `acc = math.mod(acc, consts[c])`, literal divisor.
    #[cfg(feature = "stdlib")]
    ModConst { c: ConstIdx, p: PostIdx },
    /// `acc = f(acc)`.
    #[cfg(feature = "stdlib")]
    Math1 { f: Fn1, p: PostIdx },
    /// `acc = f(pop, acc)`.
    #[cfg(feature = "stdlib")]
    Math2 { f: Fn2, p: PostIdx },
    /// `acc = f(acc, consts[c])`: the constant operand moved into the node.
    #[cfg(feature = "stdlib")]
    Math2Const { f: Fn2, c: ConstIdx, p: PostIdx },
    /// `acc = f(pop₂, pop₁, acc)` (first argument pushed first).
    #[cfg(feature = "stdlib")]
    Math3 { f: Fn3, p: PostIdx },
    /// `acc = math.random(pop, acc)`, one draw before the bounds are read.
    #[cfg(feature = "stdlib")]
    Random { p: PostIdx },
    /// `acc = r·S + O` for the bounds folded into the post-op `p`.
    #[cfg(feature = "stdlib")]
    RandomConst { p: PostIdx },
    /// `acc = math.random_integer(pop, acc)`.
    #[cfg(feature = "stdlib")]
    RandomInt { p: PostIdx },
    /// `acc = math.random_integer(consts[c], consts[c + 1])` with literal bounds.
    #[cfg(feature = "stdlib")]
    RandomIntConst { c: ConstIdx, p: PostIdx },
    /// `acc = math.die_roll(pop₂, pop₁, acc)`.
    #[cfg(feature = "stdlib")]
    DieRoll { p: PostIdx },
    /// `acc = math.die_roll_integer(pop₂, pop₁, acc)`.
    #[cfg(feature = "stdlib")]
    DieRollInt { p: PostIdx },
    /// `acc = f(pop…, acc)`, the host math function `f` of [`Program::math`] on `argc` arguments
    /// (first argument pushed first).
    HostMath { f: MathRef, argc: u8, p: PostIdx },
    /// `acc = pop <op> acc ? T : F`.
    Cmp { op: CmpOp, p: PostIdx },
    /// `acc = acc <op> consts[c] ? T : F`.
    CmpConst { op: CmpOp, c: ConstIdx, p: PostIdx },
    /// `acc = pop <op> acc ? T : F`.
    Eq { op: EqOp, p: PostIdx },
    /// `acc = acc <op> consts[c] ? T : F`.
    EqConst { op: EqOp, c: ConstIdx, p: PostIdx },
    /// `acc = acc <op> hashes[h] ? T : F`.
    EqHash { op: EqOp, h: HashIdx, p: PostIdx },
    /// `&&` operand: falsy → `acc = F`, jump `to`.
    AndStep { to: u32, p: PostIdx },
    /// `||` operand: truthy → `acc = T`, jump `to`.
    OrStep { to: u32, p: PostIdx },
    /// Last `&&` operand: `acc = acc ? T : F`.
    AndLast { p: PostIdx },
    /// Last `||` operand: `acc = acc ? T : F`.
    OrLast { p: PostIdx },
    /// `pc = to`.
    Jump { to: u32 },
    /// Falsy `acc` → `pc = to`.
    JumpIfFalsy { to: u32 },
    /// `acc = post(acc)` (the post-op of a `?:` or `??` node).
    Post { p: PostIdx },
    /// Register a `??` handler at `to`.
    HandlerPush { to: u32 },
    /// Drop the innermost handler and jump `to`.
    HandlerPop { to: u32 },
    /// `->` with the target in `acc`: enter, or on failure `acc = post(0)` and jump `to`.
    PointerEnter { to: u32, p: PostIdx },
    /// Leave `->`: restore the caller's subjects, `acc = post(acc)`.
    PointerLeave { p: PostIdx },
    /// Call `calls[q]`; `acc = post(result)`.
    Call { q: CallIdx, p: PostIdx },
    /// `loop` entry: count = `acc`; `count ≤ 0` → jump `exit`, else (NaN too) push a loop
    /// frame with `count − 1`.
    LoopBegin { exit: u32 },
    /// `loop` back edge: `top ≤ 0` → fall through; else (NaN too) `top −= 1`, jump `body`.
    LoopCheck { body: u32 },
    /// Pop the innermost loop frame.
    LoopEnd,
    /// `for_each` entry: a non-empty actor array in `acc` pushes a frame; anything else gives the
    /// `for_each`'s value (the constant at `exit`) and skips the instruction after it.
    EachBegin { exit: u32 },
    /// `for_each` step: the next entry that resolves to an actor is written to `variable.<n>`;
    /// none left → jump `exit`.
    EachNextVar { n: NameIdx, exit: u32 },
    /// [`Self::EachNextVar`] for temp slot `t`.
    EachNextTemp { t: TempIdx, exit: u32 },
    /// `return`: `acc = post(acc)` and end the (sub-)program.
    Return { p: PostIdx },
    /// End the (sub-)program with result 0 (`break` / `continue` with no loop).
    Halt,
    /// End of the (sub-)program: the result is `acc`.
    End,
}

impl Instr {
    /// The jump target an instruction carries, for relocation.
    pub(super) fn target_mut(&mut self) -> Option<&mut u32> {
        match self {
            Self::DivGuard { end: to, .. }
            | Self::AndStep { to, .. }
            | Self::OrStep { to, .. }
            | Self::Jump { to }
            | Self::JumpIfFalsy { to }
            | Self::HandlerPush { to }
            | Self::HandlerPop { to }
            | Self::PointerEnter { to, .. }
            | Self::LoopBegin { exit: to }
            | Self::LoopCheck { body: to }
            | Self::EachBegin { exit: to }
            | Self::EachNextVar { exit: to, .. }
            | Self::EachNextTemp { exit: to, .. } => Some(to),
            _ => None,
        }
    }
}

/// A `variable.` / `temp.` / `context.` name: its key and its canonical text (for messages).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct NameEntry {
    /// FNV-1 of the full canonical name (the key of [`Name`](crate::vm::Name) in its namespace).
    pub(crate) hash: HashedStr,
    /// The full canonical name, `variable.x`.
    pub(crate) text: Box<str>,
}

/// A struct member read: the member's hash and the text printed when it is missing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct MemberEntry {
    /// FNV-1 of the member name without its dot.
    pub(crate) hash: HashedStr,
    /// The member as the disassembly prints it, with the leading dot (`.b`).
    pub(crate) text: Box<str>,
    /// What the missing-member messages name: the **last** member of the read path, with its dot
    /// (`v.s.b.b.c` missing its second `.b` logs `.c`).
    pub(crate) report: Box<str>,
}

/// The variable a member assignment (`v.a.b.c = …`) writes into.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub(crate) enum StoreRoot {
    Var(NameIdx),
    Temp(TempIdx),
    /// Any other base (`context.`, a query, `->`): the write is a no-op.
    Other,
}

/// A member assignment target: the root variable and the member path below it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct MemberStore {
    pub(crate) root: StoreRoot,
    pub(crate) path: Box<[HashedStr]>,
}

/// A query call site: the query's position in the program's catalogue, the implementation the
/// version selected, and its argument sub-programs (start indices into the code).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct QueryCall {
    pub(crate) index: QueryIndex,
    pub(crate) impl_idx: u8,
    pub(crate) args: Box<[u32]>,
}

bitflags::bitflags! {
    /// Properties of a compiled expression's program that the embedding (and the evaluator) can
    /// test cheaply ([`Expr::flags`](crate::compile::Expr::flags)).
    #[derive(Copy, Clone, Default, PartialEq, Eq, Hash)]
    pub struct ProgramFlags: u16 {
        /// Every value the program builds is a float: no string, struct, query, `->` or `for_each`.
        /// `eval_f32` runs such a program on the float-only loop without allocating.
        const FLOAT_ONLY = 1 << 0;
        /// The program assigns a variable.
        const HAS_ASSIGNMENT = 1 << 1;
        /// The program draws random numbers (`math.random`, `math.random_integer`,
        /// `math.die_roll*`, volatile host math functions).
        const USES_RANDOM = 1 << 2;
        /// The program uses `->`.
        const USES_ARROW = 1 << 3;
        /// The program reads `variable.*` (of the subject, or of another actor through `->`).
        const READS_ACTOR_VARS = 1 << 4;
        /// The program calls queries.
        const USES_QUERIES = 1 << 5;
        /// The program reads or writes `temp.*`.
        const USES_TEMPS = 1 << 6;
        /// The program reads `context.*`.
        const READS_CONTEXT = 1 << 7;
        /// The program has a `loop` or `for_each`.
        const HAS_LOOPS = 1 << 8;
        /// The program uses `math.random`, `math.random_integer` or a volatile host math function,
        /// the operations
        /// [`OpSet::without_assignments_or_random`](crate::ops::OpSet::without_assignments_or_random)
        /// forbids besides `=` (`math.die_roll*` are not among them).
        const USES_RANDOM_OP = 1 << 9;
        /// The program reads struct members or assigns through a member path.
        const USES_MEMBERS = 1 << 10;
        /// The expression folded to a constant (no instruction runs).
        const CONSTANT = 1 << 11;
    }
}

impl fmt::Debug for ProgramFlags {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "ProgramFlags({})", self.bits())
    }
}

/// How deep the program's dynamic structures get, for sizing the evaluator's inline buffers.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Depths {
    /// Operand stack entries, query arguments included.
    pub(crate) stack: u16,
    /// Nested `loop` / `for_each` frames.
    pub(crate) loops: u16,
    /// Nested `??` handlers.
    pub(crate) handlers: u16,
}

impl Depths {
    /// The deepest operand stack, loop nesting and handler nesting the evaluator holds in its
    /// inline buffers, which the float-only loop must not outgrow.
    pub(crate) const FLOAT_LOOP: Self = Self {
        stack: 16,
        loops: 4,
        handlers: 4,
    };
}

/// The most temps the evaluator holds in its inline buffer, which the float-only loop must not
/// outgrow.
pub(crate) const FLOAT_LOOP_TEMPS: usize = 8;

/// A linked expression.
#[cfg_attr(not(feature = "vm"), allow(dead_code))] // the evaluator reads what the builder writes
pub(crate) struct Program {
    pub(crate) code: Box<[Instr]>,
    pub(crate) consts: Box<[f32]>,
    pub(crate) posts: Box<[PostOp]>,
    pub(crate) hashes: Box<[u64]>,
    pub(crate) names: Box<[NameEntry]>,
    /// The name of each temp slot.
    pub(crate) temps: Box<[NameIdx]>,
    pub(crate) members: Box<[MemberEntry]>,
    pub(crate) stores: Box<[MemberStore]>,
    pub(crate) calls: Box<[QueryCall]>,
    /// The catalogue `calls` index into: the one the program was compiled against.
    pub(crate) catalog: QueryCatalog,
    /// The catalogue host math instructions index into: the one the program was compiled against.
    pub(crate) math: Option<MathCatalog>,
    pub(crate) depths: Depths,
    pub(crate) flags: ProgramFlags,
    /// The evaluator starts on its float-only loop: the program is
    /// [`FLOAT_ONLY`](ProgramFlags::FLOAT_ONLY) but for calls of queries without arguments, which
    /// the float loop makes itself and whose non-float results it hands over.
    pub(crate) float_loop: bool,
    pub(crate) version: MolangVersion,
}

impl Program {
    #[inline]
    pub(crate) fn post(&self, p: PostIdx) -> PostOp {
        self.posts[usize::from(p.0)]
    }

    #[inline]
    pub(crate) fn konst(&self, c: ConstIdx) -> f32 {
        self.consts[c.0 as usize]
    }

    /// Constants `c` and `c + 1`.
    #[cfg(feature = "stdlib")]
    #[inline]
    pub(crate) fn konst_pair(&self, c: ConstIdx) -> [f32; 2] {
        let at = c.0 as usize;
        [self.consts[at], self.consts[at + 1]]
    }

    #[inline]
    pub(crate) fn hash(&self, h: HashIdx) -> u64 {
        self.hashes[h.0 as usize]
    }

    fn name(&self, n: NameIdx) -> &NameEntry {
        &self.names[usize::from(n.0)]
    }

    /// The canonical text of name `n`, for messages.
    pub(crate) fn name_text(&self, n: NameIdx) -> &str {
        &self.name(n).text
    }

    /// The canonical text of the name of temp slot `t`, for messages.
    pub(crate) fn temp_text(&self, t: TempIdx) -> &str {
        self.name_text(self.temps[usize::from(t.0)])
    }

    #[cfg(feature = "vm")]
    #[inline]
    pub(crate) fn variable_name(&self, n: NameIdx) -> VariableName {
        VariableName::from_raw_hash(self.name(n).hash)
    }

    #[cfg(feature = "vm")]
    #[inline]
    pub(crate) fn context_name(&self, n: NameIdx) -> ContextName {
        ContextName::from_raw_hash(self.name(n).hash)
    }

    #[cfg(feature = "vm")]
    #[inline]
    pub(crate) fn temp_name(&self, t: TempIdx) -> TempName {
        TempName::from_raw_hash(self.name(self.temps[usize::from(t.0)]).hash)
    }

    #[inline]
    pub(crate) fn member(&self, m: MemberIdx) -> &MemberEntry {
        &self.members[usize::from(m.0)]
    }

    #[cfg(feature = "vm")]
    #[inline]
    pub(crate) fn store(&self, s: StoreIdx) -> &MemberStore {
        &self.stores[usize::from(s.0)]
    }

    #[inline]
    pub(crate) fn call(&self, q: CallIdx) -> &QueryCall {
        &self.calls[usize::from(q.0)]
    }

    /// The program as text, one instruction per line, for tests and debugging.
    pub(super) fn disassemble(&self) -> String {
        let mut out = String::new();
        for (pc, &instr) in self.code.iter().enumerate() {
            let _ = write!(out, "{pc:4} ");
            let _ = self.write_instr(&mut out, instr);
            out.push('\n');
        }
        out
    }

    /// The disassembly of one instruction.
    #[allow(clippy::too_many_lines, reason = "one arm per instruction")]
    fn write_instr(&self, out: &mut String, instr: Instr) -> fmt::Result {
        let post = |p| self.post_suffix(p);
        let eq = |op| match op {
            EqOp::Eq => "eq",
            EqOp::Ne => "ne",
        };
        match instr {
            Instr::Const { c } => write!(out, "const {}", self.konst(c)),
            Instr::Hash { h } => write!(out, "hash {:#018x}", self.hash(h)),
            Instr::Resource { h } => write!(out, "resource {:#018x}", self.hash(h)),
            Instr::This { p } => write!(out, "this{}", post(p)),
            Instr::LoadVar { n, p } | Instr::LoadCtx { n, p } => {
                write!(out, "load {}{}", self.name_text(n), post(p))
            }
            Instr::LoadTemp { t, p } => write!(out, "load {}{}", self.temp_text(t), post(p)),
            Instr::Member { m, p } => write!(out, "member {}{}", self.member(m).text, post(p)),
            Instr::StoreVar { n, p } => write!(out, "store {}{}", self.name_text(n), post(p)),
            Instr::StoreTemp { t, p } => write!(out, "store {}{}", self.temp_text(t), post(p)),
            Instr::StoreMember { s, p } => write!(out, "store-member #{}{}", s.0, post(p)),
            Instr::Push => write!(out, "push"),
            Instr::PushConst { c } => write!(out, "push; const {}", self.konst(c)),
            Instr::Negate { p } => write!(out, "negate{}", post(p)),
            Instr::Not { p } => write!(out, "not{}", post(p)),
            Instr::AddAcc => write!(out, "add-acc"),
            Instr::AddLast { p } => write!(out, "add{}", post(p)),
            Instr::Mul { p } => write!(out, "mul{}", post(p)),
            Instr::DivGuard { end, divisor } => {
                let form = match divisor {
                    Divisor::Signed => "",
                    Divisor::Absolute => "-abs",
                };
                write!(out, "div-guard{form} -> {end}")
            }
            Instr::Div { p } => write!(out, "div{}", post(p)),
            #[cfg(feature = "stdlib")]
            Instr::Mod { p } => write!(out, "mod{}", post(p)),
            #[cfg(feature = "stdlib")]
            Instr::ModConst { c, p } => write!(out, "mod-const {}{}", self.konst(c), post(p)),
            #[cfg(feature = "stdlib")]
            Instr::Math1 { f, p } => write!(out, "{f:?}{}", post(p)),
            #[cfg(feature = "stdlib")]
            Instr::Math2 { f, p } => write!(out, "{f:?}{}", post(p)),
            #[cfg(feature = "stdlib")]
            Instr::Math2Const { f, c, p } => {
                write!(out, "{f:?}-const {}{}", self.konst(c), post(p))
            }
            #[cfg(feature = "stdlib")]
            Instr::Math3 { f, p } => write!(out, "{f:?}{}", post(p)),
            #[cfg(feature = "stdlib")]
            Instr::Random { p } => write!(out, "random{}", post(p)),
            #[cfg(feature = "stdlib")]
            Instr::RandomConst { p } => write!(out, "random-const{}", post(p)),
            #[cfg(feature = "stdlib")]
            Instr::RandomInt { p } => write!(out, "random-integer{}", post(p)),
            #[cfg(feature = "stdlib")]
            Instr::RandomIntConst { c, p } => {
                let [lo, hi] = self.konst_pair(c);
                write!(out, "random-integer-const {lo} {hi}{}", post(p))
            }
            #[cfg(feature = "stdlib")]
            Instr::DieRoll { p } => write!(out, "die-roll{}", post(p)),
            #[cfg(feature = "stdlib")]
            Instr::DieRollInt { p } => write!(out, "die-roll-integer{}", post(p)),
            Instr::HostMath { f, argc, p } => {
                let name = self.math.as_ref().map_or("?", |math| math.decl(f).name());
                write!(out, "host-math {name} args {argc}{}", post(p))
            }
            Instr::Cmp { op, p } => write!(out, "{op:?}{}", post(p)),
            Instr::CmpConst { op, c, p } => {
                write!(out, "{op:?}-const {}{}", self.konst(c), post(p))
            }
            Instr::Eq { op, p } => write!(out, "{}{}", eq(op), post(p)),
            Instr::EqConst { op, c, p } => {
                write!(out, "{}-const {}{}", eq(op), self.konst(c), post(p))
            }
            Instr::EqHash { op, h, p } => {
                write!(out, "{}-hash {:#018x}{}", eq(op), self.hash(h), post(p))
            }
            Instr::AndStep { to, p } => write!(out, "and-step -> {to}{}", post(p)),
            Instr::OrStep { to, p } => write!(out, "or-step -> {to}{}", post(p)),
            Instr::AndLast { p } => write!(out, "and-last{}", post(p)),
            Instr::OrLast { p } => write!(out, "or-last{}", post(p)),
            Instr::Jump { to } => write!(out, "jump -> {to}"),
            Instr::JumpIfFalsy { to } => write!(out, "jump-if-falsy -> {to}"),
            Instr::Post { p } => write!(out, "post{}", post(p)),
            Instr::HandlerPush { to } => write!(out, "handler-push -> {to}"),
            Instr::HandlerPop { to } => write!(out, "handler-pop -> {to}"),
            Instr::PointerEnter { to, p } => write!(out, "pointer-enter -> {to}{}", post(p)),
            Instr::PointerLeave { p } => write!(out, "pointer-leave{}", post(p)),
            Instr::Call { q, p } => {
                let call = self.call(q);
                write!(
                    out,
                    "call {} args {:?}{}",
                    self.catalog.decl(call.index).name(),
                    call.args,
                    post(p)
                )
            }
            Instr::LoopBegin { exit } => write!(out, "loop-begin -> {exit}"),
            Instr::LoopCheck { body } => write!(out, "loop-check -> {body}"),
            Instr::LoopEnd => write!(out, "loop-end"),
            Instr::EachBegin { exit } => write!(out, "each-begin -> {exit}"),
            Instr::EachNextVar { n, exit } => {
                write!(out, "each-next {} -> {exit}", self.name_text(n))
            }
            Instr::EachNextTemp { t, exit } => {
                write!(out, "each-next {} -> {exit}", self.temp_text(t))
            }
            Instr::Return { p } => write!(out, "return{}", post(p)),
            Instr::Halt => write!(out, "halt"),
            Instr::End => write!(out, "end"),
        }
    }

    /// The post-op suffix of the disassembly: nothing for the plain form.
    fn post_suffix(&self, p: PostIdx) -> PostSuffix {
        PostSuffix((p != PostIdx::PLAIN).then(|| self.post(p)))
    }
}

/// See [`Program::post_suffix`].
struct PostSuffix(Option<PostOp>);

impl fmt::Display for PostSuffix {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.0 {
            Some(post) => write!(f, " *{}+{}", post.scale, post.offset),
            None => Ok(()),
        }
    }
}

impl fmt::Debug for Program {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Program")
            .field("flags", &self.flags)
            .field("version", &self.version)
            .field("depths", &self.depths)
            .field("code", &format_args!("\n{}", self.disassemble()))
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::Side;
    use crate::stdlib::query;
    use std::collections::HashSet;

    const ALL_FLAGS: [(ProgramFlags, u16); 12] = [
        (ProgramFlags::FLOAT_ONLY, 1),
        (ProgramFlags::HAS_ASSIGNMENT, 2),
        (ProgramFlags::USES_RANDOM, 4),
        (ProgramFlags::USES_ARROW, 8),
        (ProgramFlags::READS_ACTOR_VARS, 16),
        (ProgramFlags::USES_QUERIES, 32),
        (ProgramFlags::USES_TEMPS, 64),
        (ProgramFlags::READS_CONTEXT, 128),
        (ProgramFlags::HAS_LOOPS, 256),
        (ProgramFlags::USES_RANDOM_OP, 512),
        (ProgramFlags::USES_MEMBERS, 1024),
        (ProgramFlags::CONSTANT, 2048),
    ];

    /// A program with small pools every disassembler lookup in these tests can hit.
    fn program(code: Vec<Instr>) -> Program {
        Program {
            code: code.into_boxed_slice(),
            consts: vec![1.5, 2.0, 3.0].into_boxed_slice(),
            posts: vec![PostOp::IDENTITY, PostOp::new(2.0, 3.0)].into_boxed_slice(),
            hashes: vec![1, 0xdead_beef].into_boxed_slice(),
            names: vec![
                NameEntry {
                    hash: HashedStr::new("variable.x"),
                    text: "variable.x".into(),
                },
                NameEntry {
                    hash: HashedStr::new("temp.t"),
                    text: "temp.t".into(),
                },
                NameEntry {
                    hash: HashedStr::new("context.c"),
                    text: "context.c".into(),
                },
            ]
            .into_boxed_slice(),
            temps: vec![NameIdx(1)].into_boxed_slice(),
            members: vec![MemberEntry {
                hash: HashedStr::new("b"),
                text: ".b".into(),
                report: ".b".into(),
            }]
            .into_boxed_slice(),
            stores: Box::new([]),
            calls: vec![QueryCall {
                index: crate::catalog::stdlib_index(query::IS_BABY),
                impl_idx: 0,
                args: vec![7, 9].into_boxed_slice(),
            }]
            .into_boxed_slice(),
            catalog: crate::stdlib::queries(Side::Client).clone(),
            math: Some(crate::compile::test_support::host_math().clone()),
            depths: Depths::default(),
            flags: ProgramFlags::empty(),
            float_loop: false,
            version: MolangVersion::LATEST,
        }
    }

    /// The text of the single instruction `instr` (without its pc column).
    fn one(instr: Instr) -> String {
        let text = program(vec![instr]).disassemble();
        text.strip_prefix("   0 ")
            .and_then(|t| t.strip_suffix('\n'))
            .unwrap()
            .to_owned()
    }

    #[test]
    fn every_math_function_op_maps_to_the_function_of_its_name() {
        let mut mapped = 0;
        for &op in Op::all() {
            let name = format!("{op:?}");
            let functions = [
                Fn1::of(op).map(|f| format!("{f:?}")),
                Fn2::of(op).map(|f| format!("{f:?}")),
                Fn3::of(op).map(|f| format!("{f:?}")),
            ];
            for function in functions.into_iter().flatten() {
                assert_eq!(function, name);
                mapped += 1;
            }
        }
        assert_eq!(mapped, 16 + 5 + 34);
        for op in [
            Op::Mod,
            Op::Pi,
            Op::DieRoll,
            Op::Random,
            Op::HostMath,
            Op::Add,
        ] {
            assert_eq!(Fn1::of(op), None, "{op:?}");
            assert_eq!(Fn2::of(op), None, "{op:?}");
            assert_eq!(Fn3::of(op), None, "{op:?}");
        }
    }

    #[test]
    fn the_three_argument_functions_are_clamp_the_lerps_and_the_thirty_easings() {
        let mut count = 0;
        for &op in Op::all() {
            let expected = matches!(op, Op::Clamp | Op::Lerp | Op::LerpRotate | Op::InverseLerp)
                || format!("{op:?}").starts_with("Ease");
            assert_eq!(Fn3::of(op).is_some(), expected, "{op:?}");
            count += usize::from(expected);
        }
        assert_eq!(count, 34);
        for op in [Op::DieRoll, Op::DieRollInt] {
            assert_eq!(Fn3::of(op), None, "{op:?}");
        }
        assert_eq!(Fn3::Clamp.apply(3.0, 2.0, 1.0, PostOp::IDENTITY), 1.0);
    }

    type F1 = fn(f32, PostOp) -> f32;

    type F2 = fn(f32, f32, PostOp) -> f32;

    type F3 = fn(f32, f32, f32, PostOp) -> f32;

    const POSTS: [PostOp; 2] = [PostOp::IDENTITY, PostOp::new(2.0, 1.0)];

    fn same(a: f32, b: f32) -> bool {
        a.to_bits() == b.to_bits()
    }

    #[test]
    fn fn1_applies_the_function_of_its_name() {
        let table: [(Fn1, F1); 16] = [
            (Fn1::Abs, math::abs),
            (Fn1::Acos, math::acos),
            (Fn1::Asin, math::asin),
            (Fn1::Atan, math::atan),
            (Fn1::Ceil, math::ceil),
            (Fn1::Cos, math::cos),
            (Fn1::Exp, math::exp),
            (Fn1::Floor, math::floor),
            (Fn1::HermiteBlend, math::hermite_blend),
            (Fn1::Ln, math::ln),
            (Fn1::MinAngle, math::min_angle),
            (Fn1::Round, math::round),
            (Fn1::Sin, math::sin),
            (Fn1::Sign, math::sign),
            (Fn1::Sqrt, math::sqrt),
            (Fn1::Trunc, math::trunc),
        ];
        for (f, direct) in table {
            for x in [0.5, -1.5, 30.0, 2.0, 0.0] {
                for post in POSTS {
                    assert!(same(f.apply(x, post), direct(x, post)), "{f:?}({x})");
                }
            }
        }
    }

    #[test]
    fn fn1_pinned_values_tell_neighbouring_functions_apart() {
        let id = PostOp::IDENTITY;
        assert_eq!(Fn1::Floor.apply(-1.5, id), -2.0);
        assert_eq!(Fn1::Ceil.apply(-1.5, id), -1.0);
        assert_eq!(Fn1::Trunc.apply(-1.5, id), -1.0);
        assert_eq!(Fn1::Round.apply(2.5, id), 3.0);
        assert_eq!(Fn1::Abs.apply(-1.5, id), 1.5);
        assert_eq!(Fn1::Sign.apply(-3.0, id), -1.0);
        assert_eq!(Fn1::Sqrt.apply(9.0, id), 3.0);
        assert_eq!(Fn1::Abs.apply(-1.5, PostOp::new(2.0, 1.0)), 4.0);
    }

    #[test]
    fn fn2_applies_the_function_of_its_name() {
        let table: [(Fn2, F2); 5] = [
            (Fn2::Atan2, math::atan2),
            (Fn2::CopySign, math::copy_sign),
            (Fn2::Max, math::max),
            (Fn2::Min, math::min),
            (Fn2::Pow, math::pow),
        ];
        for (f, direct) in table {
            for (a, b) in [(3.0, 2.0), (-1.0, 4.0), (0.5, 2.0), (1.0, 1.0)] {
                for post in POSTS {
                    assert!(
                        same(f.apply(a, b, post), direct(a, b, post)),
                        "{f:?}({a}, {b})"
                    );
                }
            }
        }
        let id = PostOp::IDENTITY;
        assert_eq!(Fn2::Max.apply(3.0, 2.0, id), 3.0);
        assert_eq!(Fn2::Min.apply(3.0, 2.0, id), 2.0);
        assert_eq!(Fn2::Pow.apply(2.0, 3.0, id), 8.0);
        assert_eq!(Fn2::CopySign.apply(3.0, -1.0, id), -3.0);
    }

    macro_rules! fn3_table {
        ($($variant:ident => $direct:ident),* $(,)?) => {
            [$((Fn3::$variant, math::$direct as F3)),*]
        };
    }

    #[test]
    fn fn3_applies_the_function_of_its_name() {
        let table: [(Fn3, F3); 34] = fn3_table![
            Clamp => clamp,
            Lerp => lerp,
            LerpRotate => lerprotate,
            InverseLerp => inverse_lerp,
            EaseInQuad => ease_in_quad,
            EaseOutQuad => ease_out_quad,
            EaseInOutQuad => ease_in_out_quad,
            EaseInCubic => ease_in_cubic,
            EaseOutCubic => ease_out_cubic,
            EaseInOutCubic => ease_in_out_cubic,
            EaseInQuart => ease_in_quart,
            EaseOutQuart => ease_out_quart,
            EaseInOutQuart => ease_in_out_quart,
            EaseInQuint => ease_in_quint,
            EaseOutQuint => ease_out_quint,
            EaseInOutQuint => ease_in_out_quint,
            EaseInSine => ease_in_sine,
            EaseOutSine => ease_out_sine,
            EaseInOutSine => ease_in_out_sine,
            EaseInExpo => ease_in_expo,
            EaseOutExpo => ease_out_expo,
            EaseInOutExpo => ease_in_out_expo,
            EaseInCirc => ease_in_circ,
            EaseOutCirc => ease_out_circ,
            EaseInOutCirc => ease_in_out_circ,
            EaseInBounce => ease_in_bounce,
            EaseOutBounce => ease_out_bounce,
            EaseInOutBounce => ease_in_out_bounce,
            EaseInBack => ease_in_back,
            EaseOutBack => ease_out_back,
            EaseInOutBack => ease_in_out_back,
            EaseInElastic => ease_in_elastic,
            EaseOutElastic => ease_out_elastic,
            EaseInOutElastic => ease_in_out_elastic,
        ];
        for (f, direct) in table {
            for (a, b, c) in [
                (0.25, 1.0, 0.5),
                (0.0, 10.0, 0.3),
                (5.0, 0.0, 1.0),
                (-1.0, 2.0, 0.75),
            ] {
                for post in POSTS {
                    assert!(
                        same(f.apply(a, b, c, post), direct(a, b, c, post)),
                        "{f:?}({a}, {b}, {c})"
                    );
                }
            }
        }
        let id = PostOp::IDENTITY;
        assert_eq!(Fn3::Clamp.apply(5.0, 0.0, 1.0, id), 1.0);
        assert_eq!(Fn3::Clamp.apply(-5.0, 0.0, 1.0, id), 0.0);
        assert_eq!(Fn3::Lerp.apply(0.0, 10.0, 0.5, id), 5.0);
        assert_eq!(Fn3::InverseLerp.apply(0.0, 10.0, 5.0, id), 0.5);
    }

    #[test]
    fn every_easing_gives_its_own_value() {
        // The 30 easings are pairwise different at a generic point, so a swapped pair shows.
        let eases = [
            Fn3::EaseInQuad,
            Fn3::EaseOutQuad,
            Fn3::EaseInOutQuad,
            Fn3::EaseInCubic,
            Fn3::EaseOutCubic,
            Fn3::EaseInOutCubic,
            Fn3::EaseInQuart,
            Fn3::EaseOutQuart,
            Fn3::EaseInOutQuart,
            Fn3::EaseInQuint,
            Fn3::EaseOutQuint,
            Fn3::EaseInOutQuint,
            Fn3::EaseInSine,
            Fn3::EaseOutSine,
            Fn3::EaseInOutSine,
            Fn3::EaseInExpo,
            Fn3::EaseOutExpo,
            Fn3::EaseInOutExpo,
            Fn3::EaseInCirc,
            Fn3::EaseOutCirc,
            Fn3::EaseInOutCirc,
            Fn3::EaseInBounce,
            Fn3::EaseOutBounce,
            Fn3::EaseInOutBounce,
            Fn3::EaseInBack,
            Fn3::EaseOutBack,
            Fn3::EaseInOutBack,
            Fn3::EaseInElastic,
            Fn3::EaseOutElastic,
            Fn3::EaseInOutElastic,
        ];
        let bits: std::collections::HashSet<u32> = eases
            .iter()
            .map(|&f| f.apply(1.0, 4.0, 0.37, PostOp::IDENTITY).to_bits())
            .collect();
        assert_eq!(bits.len(), 30);
    }

    #[cfg(feature = "vm")]
    #[test]
    fn cmp_op_orders_numbers() {
        assert!(CmpOp::Lt.holds(1.0, 2.0));
        assert!(!CmpOp::Lt.holds(2.0, 2.0));
        assert!(CmpOp::Le.holds(2.0, 2.0));
        assert!(!CmpOp::Le.holds(3.0, 2.0));
        assert!(CmpOp::Ge.holds(2.0, 2.0));
        assert!(!CmpOp::Ge.holds(1.0, 2.0));
        assert!(CmpOp::Gt.holds(3.0, 2.0));
        assert!(!CmpOp::Gt.holds(2.0, 2.0));
    }

    #[cfg(feature = "vm")]
    #[test]
    fn cmp_op_with_nan_follows_the_architecture() {
        use crate::numeric::test_support::per_arch;
        // x86-64: every comparison with a NaN is false. arm64: `<` and `<=` are true with a NaN.
        for (op, x86, arm) in [
            (CmpOp::Lt, false, true),
            (CmpOp::Le, false, true),
            (CmpOp::Ge, false, false),
            (CmpOp::Gt, false, false),
        ] {
            for (a, b) in [(f32::NAN, 1.0), (1.0, f32::NAN), (f32::NAN, f32::NAN)] {
                assert_eq!(op.holds(a, b), per_arch(x86, arm), "{op:?} {a} {b}");
            }
        }
    }

    #[test]
    fn instructions_are_eight_bytes() {
        assert_eq!(std::mem::size_of::<Instr>(), 8);
    }

    #[test]
    fn program_is_send_and_sync() {
        fn check<T: Send + Sync>() {}
        check::<Program>();
    }

    #[test]
    fn every_flag_is_a_distinct_single_bit_with_a_pinned_value() {
        for (flag, bits) in ALL_FLAGS {
            assert_eq!(flag.bits(), bits);
            assert_eq!(flag.bits().count_ones(), 1);
        }
        assert_eq!(ProgramFlags::empty().bits(), 0);
        let all = ALL_FLAGS
            .iter()
            .fold(ProgramFlags::empty(), |a, &(f, _)| a.union(f));
        assert_eq!(all.bits(), 0x0fff);
        let distinct: HashSet<u16> = ALL_FLAGS.iter().map(|&(f, _)| f.bits()).collect();
        assert_eq!(distinct.len(), 12);
    }

    #[test]
    fn contains_is_a_subset_test() {
        let both = ProgramFlags::FLOAT_ONLY.union(ProgramFlags::HAS_LOOPS);
        assert!(both.contains(ProgramFlags::FLOAT_ONLY));
        assert!(both.contains(ProgramFlags::HAS_LOOPS));
        assert!(both.contains(both));
        assert!(both.contains(ProgramFlags::empty()));
        assert!(!both.contains(ProgramFlags::USES_ARROW));
        assert!(!both.contains(both.union(ProgramFlags::USES_ARROW)));
        assert!(!ProgramFlags::FLOAT_ONLY.contains(both));
        assert!(ProgramFlags::empty().contains(ProgramFlags::empty()));
        assert!(!ProgramFlags::empty().contains(ProgramFlags::CONSTANT));
    }

    #[test]
    fn union_and_difference_are_set_operations() {
        let a = ProgramFlags::FLOAT_ONLY.union(ProgramFlags::HAS_LOOPS);
        let b = ProgramFlags::HAS_LOOPS.union(ProgramFlags::USES_TEMPS);
        assert_eq!(a.union(b), b.union(a));
        assert_eq!(a.union(b).bits(), 1 | 256 | 64);
        assert_eq!(a.union(a), a);
        assert_eq!(
            a.difference(ProgramFlags::FLOAT_ONLY),
            ProgramFlags::HAS_LOOPS
        );
        assert_eq!(a.difference(a), ProgramFlags::empty());
        assert_eq!(a.difference(ProgramFlags::USES_ARROW), a);
        assert_eq!(a.difference(b), ProgramFlags::FLOAT_ONLY);
        assert_eq!(a.difference(ProgramFlags::empty()), a);
    }

    #[test]
    fn insert_and_remove_are_idempotent() {
        let mut flags = ProgramFlags::empty();
        flags |= ProgramFlags::USES_TEMPS;
        flags |= ProgramFlags::USES_TEMPS;
        assert_eq!(flags.bits(), 64);
        flags |= ProgramFlags::CONSTANT;
        assert_eq!(flags.bits(), 64 | 2048);
        flags.remove(ProgramFlags::USES_ARROW);
        assert_eq!(flags.bits(), 64 | 2048);
        flags.remove(ProgramFlags::USES_TEMPS);
        flags.remove(ProgramFlags::USES_TEMPS);
        assert_eq!(flags, ProgramFlags::CONSTANT);
        flags.remove(ProgramFlags::CONSTANT);
        assert_eq!(flags, ProgramFlags::empty());
    }

    #[test]
    fn flags_default_is_empty_and_debug_prints_the_bits() {
        assert_eq!(ProgramFlags::default(), ProgramFlags::empty());
        assert_eq!(format!("{:?}", ProgramFlags::empty()), "ProgramFlags(0)");
        assert_eq!(
            format!(
                "{:?}",
                ProgramFlags::FLOAT_ONLY.union(ProgramFlags::HAS_ASSIGNMENT)
            ),
            "ProgramFlags(3)"
        );
        assert_eq!(
            format!("{:?}", ProgramFlags::CONSTANT),
            "ProgramFlags(2048)"
        );
    }

    #[test]
    fn flags_hash_and_compare_by_bits() {
        let set: HashSet<ProgramFlags> = [
            ProgramFlags::FLOAT_ONLY,
            ProgramFlags::FLOAT_ONLY,
            ProgramFlags::CONSTANT,
        ]
        .into_iter()
        .collect();
        assert_eq!(set.len(), 2);
        assert_eq!(
            ProgramFlags::FLOAT_ONLY.union(ProgramFlags::CONSTANT),
            ProgramFlags::CONSTANT.union(ProgramFlags::FLOAT_ONLY)
        );
    }

    #[test]
    fn the_plain_post_idx_is_zero() {
        assert_eq!(PostIdx::PLAIN, PostIdx(0));
        assert_ne!(PostIdx::PLAIN, PostIdx(1));
    }

    #[test]
    fn depths_default_to_zero() {
        assert_eq!(
            Depths::default(),
            Depths {
                stack: 0,
                loops: 0,
                handlers: 0
            }
        );
    }

    #[test]
    fn pool_accessors_index_their_tables() {
        let p = program(vec![Instr::End]);
        assert_eq!(p.post(PostIdx::PLAIN), PostOp::IDENTITY);
        assert_eq!(p.post(PostIdx(1)), PostOp::new(2.0, 3.0));
        assert_eq!(p.konst(ConstIdx(0)), 1.5);
        assert_eq!(p.konst(ConstIdx(1)), 2.0);
        assert_eq!(p.konst(ConstIdx(2)), 3.0);
        assert_eq!(p.hash(HashIdx(1)), 0xdead_beef);
        assert_eq!(p.name_text(NameIdx(2)), "context.c");
        assert_eq!(p.member(MemberIdx(0)).text.as_ref(), ".b");
        assert_eq!(p.call(CallIdx(0)).args.as_ref(), [7, 9]);
    }

    #[test]
    fn a_temp_slot_resolves_through_the_slot_table_to_its_name() {
        let mut p = program(vec![Instr::End]);
        p.names = ["variable.x", "temp.t", "temp.u"]
            .map(|text| NameEntry {
                hash: HashedStr::new(text),
                text: text.into(),
            })
            .into();
        p.temps = vec![NameIdx(2), NameIdx(1)].into_boxed_slice();
        assert_eq!(p.temp_text(TempIdx(0)), "temp.u");
        assert_eq!(p.temp_text(TempIdx(1)), "temp.t");
        #[cfg(feature = "vm")]
        {
            use crate::vm::{TempName, VariableName};
            assert_eq!(p.temp_name(TempIdx(0)), TempName::new("u"));
            assert_eq!(p.temp_name(TempIdx(1)), TempName::new("t"));
            assert_eq!(p.variable_name(NameIdx(0)), VariableName::new("x"));
        }
    }

    #[test]
    fn target_mut_exposes_exactly_the_jump_operands() {
        let jumps = [
            Instr::DivGuard {
                end: 1,
                divisor: Divisor::Signed,
            },
            Instr::AndStep {
                to: 1,
                p: PostIdx::PLAIN,
            },
            Instr::OrStep {
                to: 1,
                p: PostIdx::PLAIN,
            },
            Instr::Jump { to: 1 },
            Instr::JumpIfFalsy { to: 1 },
            Instr::HandlerPush { to: 1 },
            Instr::HandlerPop { to: 1 },
            Instr::PointerEnter {
                to: 1,
                p: PostIdx::PLAIN,
            },
            Instr::LoopBegin { exit: 1 },
            Instr::LoopCheck { body: 1 },
            Instr::EachBegin { exit: 1 },
            Instr::EachNextVar {
                n: NameIdx(0),
                exit: 1,
            },
        ];
        for mut instr in jumps {
            let before = instr;
            *instr
                .target_mut()
                .unwrap_or_else(|| panic!("{before:?} has a target")) = 99;
            assert_ne!(instr, before, "{before:?}");
            // The write went to the one target field: writing the old value back restores it.
            *instr.target_mut().unwrap() = 1;
            assert_eq!(instr, before);
        }
        let others = [
            Instr::Const { c: ConstIdx(0) },
            Instr::Hash { h: HashIdx(0) },
            Instr::Resource { h: HashIdx(0) },
            Instr::This { p: PostIdx::PLAIN },
            Instr::LoadVar {
                n: NameIdx(0),
                p: PostIdx::PLAIN,
            },
            Instr::Push,
            Instr::PushConst { c: ConstIdx(0) },
            Instr::AddAcc,
            Instr::Call {
                q: CallIdx(0),
                p: PostIdx::PLAIN,
            },
            Instr::LoopEnd,
            Instr::Return { p: PostIdx::PLAIN },
            Instr::Post { p: PostIdx::PLAIN },
            Instr::Halt,
            Instr::End,
        ];
        for mut instr in others {
            assert!(instr.target_mut().is_none(), "{instr:?}");
        }
    }

    #[test]
    fn target_mut_leaves_the_other_operands_alone() {
        let mut i = Instr::DivGuard {
            end: 1,
            divisor: Divisor::Absolute,
        };
        *i.target_mut().unwrap() = 7;
        assert_eq!(
            i,
            Instr::DivGuard {
                end: 7,
                divisor: Divisor::Absolute
            }
        );
        let mut i = Instr::EachNextTemp {
            t: TempIdx(3),
            exit: 1,
        };
        *i.target_mut().unwrap() = 8;
        assert_eq!(
            i,
            Instr::EachNextTemp {
                t: TempIdx(3),
                exit: 8
            }
        );
        let mut i = Instr::LoopCheck { body: 1 };
        *i.target_mut().unwrap() = 9;
        assert_eq!(i, Instr::LoopCheck { body: 9 });
        let mut i = Instr::PointerEnter {
            to: 1,
            p: PostIdx(1),
        };
        *i.target_mut().unwrap() = 4;
        assert_eq!(
            i,
            Instr::PointerEnter {
                to: 4,
                p: PostIdx(1)
            }
        );
    }

    #[test]
    fn the_listing_numbers_every_instruction_right_aligned() {
        let text = program(vec![Instr::Push, Instr::Halt, Instr::End]).disassemble();
        assert_eq!(text, "   0 push\n   1 halt\n   2 end\n");
        assert_eq!(program(vec![]).disassemble(), "");
    }

    #[test]
    fn values_disassemble() {
        assert_eq!(one(Instr::Const { c: ConstIdx(0) }), "const 1.5");
        assert_eq!(one(Instr::Const { c: ConstIdx(1) }), "const 2");
        assert_eq!(
            one(Instr::Hash { h: HashIdx(0) }),
            "hash 0x0000000000000001"
        );
        assert_eq!(
            one(Instr::Hash { h: HashIdx(1) }),
            "hash 0x00000000deadbeef"
        );
        assert_eq!(
            one(Instr::Resource { h: HashIdx(1) }),
            "resource 0x00000000deadbeef"
        );
        assert_eq!(one(Instr::This { p: PostIdx::PLAIN }), "this");
        assert_eq!(one(Instr::This { p: PostIdx(1) }), "this *2+3");
        assert_eq!(one(Instr::PushConst { c: ConstIdx(2) }), "push; const 3");
        assert_eq!(one(Instr::Push), "push");
    }

    #[test]
    fn loads_and_stores_resolve_names() {
        assert_eq!(
            one(Instr::LoadVar {
                n: NameIdx(0),
                p: PostIdx::PLAIN
            }),
            "load variable.x"
        );
        assert_eq!(
            one(Instr::LoadVar {
                n: NameIdx(0),
                p: PostIdx(1)
            }),
            "load variable.x *2+3"
        );
        assert_eq!(
            one(Instr::LoadCtx {
                n: NameIdx(2),
                p: PostIdx::PLAIN
            }),
            "load context.c"
        );
        // A temp slot resolves through `temps` to its name.
        assert_eq!(
            one(Instr::LoadTemp {
                t: TempIdx(0),
                p: PostIdx::PLAIN
            }),
            "load temp.t"
        );
        assert_eq!(
            one(Instr::StoreVar {
                n: NameIdx(0),
                p: PostIdx::PLAIN
            }),
            "store variable.x"
        );
        assert_eq!(
            one(Instr::StoreTemp {
                t: TempIdx(0),
                p: PostIdx(1)
            }),
            "store temp.t *2+3"
        );
        assert_eq!(
            one(Instr::StoreMember {
                s: StoreIdx(4),
                p: PostIdx::PLAIN
            }),
            "store-member #4"
        );
        assert_eq!(
            one(Instr::Member {
                m: MemberIdx(0),
                p: PostIdx::PLAIN
            }),
            "member .b"
        );
        assert_eq!(
            one(Instr::Member {
                m: MemberIdx(0),
                p: PostIdx(1)
            }),
            "member .b *2+3"
        );
    }

    #[test]
    fn arithmetic_disassembles() {
        assert_eq!(one(Instr::Negate { p: PostIdx::PLAIN }), "negate");
        assert_eq!(one(Instr::Not { p: PostIdx(1) }), "not *2+3");
        assert_eq!(one(Instr::AddAcc), "add-acc");
        assert_eq!(one(Instr::AddLast { p: PostIdx::PLAIN }), "add");
        assert_eq!(one(Instr::Mul { p: PostIdx(1) }), "mul *2+3");
        assert_eq!(
            one(Instr::DivGuard {
                end: 5,
                divisor: Divisor::Signed
            }),
            "div-guard -> 5"
        );
        assert_eq!(
            one(Instr::DivGuard {
                end: 5,
                divisor: Divisor::Absolute
            }),
            "div-guard-abs -> 5"
        );
        assert_eq!(one(Instr::Div { p: PostIdx::PLAIN }), "div");
        assert_eq!(one(Instr::Mod { p: PostIdx::PLAIN }), "mod");
        assert_eq!(
            one(Instr::ModConst {
                c: ConstIdx(1),
                p: PostIdx(1)
            }),
            "mod-const 2 *2+3"
        );
    }

    #[test]
    fn math_disassembles_with_the_function_name() {
        assert_eq!(
            one(Instr::Math1 {
                f: Fn1::Abs,
                p: PostIdx::PLAIN
            }),
            "Abs"
        );
        assert_eq!(
            one(Instr::Math1 {
                f: Fn1::HermiteBlend,
                p: PostIdx(1)
            }),
            "HermiteBlend *2+3"
        );
        assert_eq!(
            one(Instr::Math2 {
                f: Fn2::Pow,
                p: PostIdx::PLAIN
            }),
            "Pow"
        );
        assert_eq!(
            one(Instr::Math2Const {
                f: Fn2::Max,
                c: ConstIdx(1),
                p: PostIdx::PLAIN
            }),
            "Max-const 2"
        );
        assert_eq!(
            one(Instr::Math3 {
                f: Fn3::Clamp,
                p: PostIdx::PLAIN
            }),
            "Clamp"
        );
        assert_eq!(
            one(Instr::Math3 {
                f: Fn3::EaseInOutElastic,
                p: PostIdx(1)
            }),
            "EaseInOutElastic *2+3"
        );
    }

    #[test]
    fn a_host_math_call_prints_the_function_name_and_the_argument_count() {
        let sum = crate::compile::test_support::host_math()
            .find("math.sum")
            .unwrap();
        assert_eq!(
            one(Instr::HostMath {
                f: sum,
                argc: 3,
                p: PostIdx::PLAIN
            }),
            "host-math math.sum args 3"
        );
        assert_eq!(
            one(Instr::HostMath {
                f: sum,
                argc: 1,
                p: PostIdx(1)
            }),
            "host-math math.sum args 1 *2+3"
        );
        let mut without = program(vec![Instr::HostMath {
            f: sum,
            argc: 2,
            p: PostIdx::PLAIN,
        }]);
        without.math = None;
        assert_eq!(without.disassemble(), "   0 host-math ? args 2\n");
    }

    #[test]
    fn random_disassembles() {
        assert_eq!(one(Instr::Random { p: PostIdx::PLAIN }), "random");
        assert_eq!(
            one(Instr::RandomConst { p: PostIdx(1) }),
            "random-const *2+3"
        );
        assert_eq!(
            one(Instr::RandomInt { p: PostIdx::PLAIN }),
            "random-integer"
        );
        // The two bounds are consecutive constants.
        assert_eq!(
            one(Instr::RandomIntConst {
                c: ConstIdx(0),
                p: PostIdx::PLAIN
            }),
            "random-integer-const 1.5 2"
        );
        assert_eq!(one(Instr::DieRoll { p: PostIdx::PLAIN }), "die-roll");
        assert_eq!(
            one(Instr::DieRollInt { p: PostIdx(1) }),
            "die-roll-integer *2+3"
        );
    }

    #[test]
    fn comparisons_disassemble() {
        for (op, name) in [
            (CmpOp::Lt, "Lt"),
            (CmpOp::Le, "Le"),
            (CmpOp::Ge, "Ge"),
            (CmpOp::Gt, "Gt"),
        ] {
            assert_eq!(
                one(Instr::Cmp {
                    op,
                    p: PostIdx::PLAIN
                }),
                name
            );
            assert_eq!(
                one(Instr::CmpConst {
                    op,
                    c: ConstIdx(0),
                    p: PostIdx(1)
                }),
                format!("{name}-const 1.5 *2+3")
            );
        }
        assert_eq!(
            one(Instr::Eq {
                op: EqOp::Eq,
                p: PostIdx::PLAIN
            }),
            "eq"
        );
        assert_eq!(
            one(Instr::Eq {
                op: EqOp::Ne,
                p: PostIdx::PLAIN
            }),
            "ne"
        );
        assert_eq!(
            one(Instr::EqConst {
                op: EqOp::Eq,
                c: ConstIdx(0),
                p: PostIdx::PLAIN
            }),
            "eq-const 1.5"
        );
        assert_eq!(
            one(Instr::EqConst {
                op: EqOp::Ne,
                c: ConstIdx(1),
                p: PostIdx(1)
            }),
            "ne-const 2 *2+3"
        );
        assert_eq!(
            one(Instr::EqHash {
                op: EqOp::Eq,
                h: HashIdx(1),
                p: PostIdx::PLAIN
            }),
            "eq-hash 0x00000000deadbeef"
        );
        assert_eq!(
            one(Instr::EqHash {
                op: EqOp::Ne,
                h: HashIdx(0),
                p: PostIdx(1)
            }),
            "ne-hash 0x0000000000000001 *2+3"
        );
    }

    #[test]
    fn logic_and_control_disassemble() {
        assert_eq!(
            one(Instr::AndStep {
                to: 8,
                p: PostIdx::PLAIN
            }),
            "and-step -> 8"
        );
        assert_eq!(
            one(Instr::OrStep {
                to: 8,
                p: PostIdx(1)
            }),
            "or-step -> 8 *2+3"
        );
        assert_eq!(one(Instr::AndLast { p: PostIdx::PLAIN }), "and-last");
        assert_eq!(one(Instr::OrLast { p: PostIdx(1) }), "or-last *2+3");
        assert_eq!(one(Instr::Jump { to: 3 }), "jump -> 3");
        assert_eq!(one(Instr::JumpIfFalsy { to: 4 }), "jump-if-falsy -> 4");
        assert_eq!(one(Instr::Post { p: PostIdx(1) }), "post *2+3");
        assert_eq!(one(Instr::HandlerPush { to: 6 }), "handler-push -> 6");
        assert_eq!(one(Instr::HandlerPop { to: 7 }), "handler-pop -> 7");
        assert_eq!(
            one(Instr::PointerEnter {
                to: 9,
                p: PostIdx::PLAIN
            }),
            "pointer-enter -> 9"
        );
        assert_eq!(
            one(Instr::PointerLeave { p: PostIdx(1) }),
            "pointer-leave *2+3"
        );
        assert_eq!(one(Instr::Return { p: PostIdx::PLAIN }), "return");
        assert_eq!(one(Instr::Return { p: PostIdx(1) }), "return *2+3");
        assert_eq!(one(Instr::Halt), "halt");
        assert_eq!(one(Instr::End), "end");
    }

    #[test]
    fn loops_disassemble() {
        assert_eq!(one(Instr::LoopBegin { exit: 9 }), "loop-begin -> 9");
        assert_eq!(one(Instr::LoopCheck { body: 2 }), "loop-check -> 2");
        assert_eq!(one(Instr::LoopEnd), "loop-end");
        assert_eq!(one(Instr::EachBegin { exit: 11 }), "each-begin -> 11");
        // A variable loop name indexes the names; a temp one goes through the slot table.
        assert_eq!(
            one(Instr::EachNextVar {
                n: NameIdx(0),
                exit: 10
            }),
            "each-next variable.x -> 10"
        );
        assert_eq!(
            one(Instr::EachNextTemp {
                t: TempIdx(0),
                exit: 10
            }),
            "each-next temp.t -> 10"
        );
    }

    #[test]
    fn a_call_prints_the_query_name_and_the_argument_starts() {
        assert_eq!(
            one(Instr::Call {
                q: CallIdx(0),
                p: PostIdx::PLAIN
            }),
            "call query.is_baby args [7, 9]"
        );
        assert_eq!(
            one(Instr::Call {
                q: CallIdx(0),
                p: PostIdx(1)
            }),
            "call query.is_baby args [7, 9] *2+3"
        );
    }

    #[test]
    fn the_post_op_suffix_prints_scale_then_offset() {
        let mut p = program(vec![Instr::Negate { p: PostIdx(1) }]);
        p.posts = vec![PostOp::IDENTITY, PostOp::new(-1.0, 0.25)].into_boxed_slice();
        assert_eq!(p.disassemble(), "   0 negate *-1+0.25\n");
    }

    #[test]
    #[allow(clippy::too_many_lines, reason = "one line per instruction variant")]
    fn disassembly_covers_every_variant_in_order() {
        let p0 = PostIdx::PLAIN;
        let code = vec![
            Instr::Const { c: ConstIdx(0) },
            Instr::Hash { h: HashIdx(0) },
            Instr::Resource { h: HashIdx(0) },
            Instr::This { p: p0 },
            Instr::LoadVar {
                n: NameIdx(0),
                p: p0,
            },
            Instr::LoadTemp {
                t: TempIdx(0),
                p: p0,
            },
            Instr::LoadCtx {
                n: NameIdx(2),
                p: p0,
            },
            Instr::Member {
                m: MemberIdx(0),
                p: p0,
            },
            Instr::StoreVar {
                n: NameIdx(0),
                p: p0,
            },
            Instr::StoreTemp {
                t: TempIdx(0),
                p: p0,
            },
            Instr::StoreMember {
                s: StoreIdx(0),
                p: p0,
            },
            Instr::Push,
            Instr::PushConst { c: ConstIdx(0) },
            Instr::Negate { p: p0 },
            Instr::Not { p: p0 },
            Instr::AddAcc,
            Instr::AddLast { p: p0 },
            Instr::Mul { p: p0 },
            Instr::DivGuard {
                end: 0,
                divisor: Divisor::Signed,
            },
            Instr::Div { p: p0 },
            Instr::Mod { p: p0 },
            Instr::ModConst {
                c: ConstIdx(0),
                p: p0,
            },
            Instr::Math1 { f: Fn1::Abs, p: p0 },
            Instr::Math2 { f: Fn2::Min, p: p0 },
            Instr::Math2Const {
                f: Fn2::Min,
                c: ConstIdx(0),
                p: p0,
            },
            Instr::Math3 {
                f: Fn3::Lerp,
                p: p0,
            },
            Instr::Random { p: p0 },
            Instr::RandomConst { p: p0 },
            Instr::RandomInt { p: p0 },
            Instr::RandomIntConst {
                c: ConstIdx(0),
                p: p0,
            },
            Instr::DieRoll { p: p0 },
            Instr::DieRollInt { p: p0 },
            Instr::HostMath {
                f: crate::compile::test_support::host_math()
                    .find("math.twice")
                    .unwrap(),
                argc: 1,
                p: p0,
            },
            Instr::Cmp {
                op: CmpOp::Lt,
                p: p0,
            },
            Instr::CmpConst {
                op: CmpOp::Lt,
                c: ConstIdx(0),
                p: p0,
            },
            Instr::Eq {
                op: EqOp::Eq,
                p: p0,
            },
            Instr::EqConst {
                op: EqOp::Eq,
                c: ConstIdx(0),
                p: p0,
            },
            Instr::EqHash {
                op: EqOp::Eq,
                h: HashIdx(0),
                p: p0,
            },
            Instr::AndStep { to: 0, p: p0 },
            Instr::OrStep { to: 0, p: p0 },
            Instr::AndLast { p: p0 },
            Instr::OrLast { p: p0 },
            Instr::Jump { to: 0 },
            Instr::JumpIfFalsy { to: 0 },
            Instr::Post { p: p0 },
            Instr::HandlerPush { to: 0 },
            Instr::HandlerPop { to: 0 },
            Instr::PointerEnter { to: 0, p: p0 },
            Instr::PointerLeave { p: p0 },
            Instr::Call {
                q: CallIdx(0),
                p: p0,
            },
            Instr::LoopBegin { exit: 0 },
            Instr::LoopCheck { body: 0 },
            Instr::LoopEnd,
            Instr::EachBegin { exit: 0 },
            Instr::EachNextVar {
                n: NameIdx(0),
                exit: 0,
            },
            Instr::EachNextTemp {
                t: TempIdx(0),
                exit: 0,
            },
            Instr::Return { p: p0 },
            Instr::Halt,
            Instr::End,
        ];
        let n = code.len();
        assert_eq!(n, 59);
        let text = program(code).disassemble();
        assert_eq!(text.lines().count(), n);
        for (pc, line) in text.lines().enumerate() {
            assert_eq!(line[..4].trim().parse::<usize>().unwrap(), pc);
            assert_eq!(&line[4..5], " ");
        }
    }

    #[test]
    fn program_debug_shows_the_header_fields_and_the_code() {
        let mut p = program(vec![Instr::Const { c: ConstIdx(0) }, Instr::End]);
        p.flags = ProgramFlags::FLOAT_ONLY;
        p.depths = Depths {
            stack: 2,
            loops: 1,
            handlers: 0,
        };
        let text = format!("{p:?}");
        assert!(text.starts_with("Program {"), "{text}");
        assert!(text.contains("flags: ProgramFlags(1)"), "{text}");
        assert!(text.contains("version: V13"), "{text}");
        assert!(
            text.contains("depths: Depths { stack: 2, loops: 1, handlers: 0 }"),
            "{text}"
        );
        assert!(text.contains("   0 const 1.5\n   1 end\n"), "{text}");
        assert!(text.trim_end().ends_with(".. }"), "{text}");
    }

    #[test]
    fn pool_entries_compare_by_value() {
        let a = NameEntry {
            hash: HashedStr::new("variable.x"),
            text: "variable.x".into(),
        };
        assert_eq!(a.clone(), a);
        assert_ne!(
            a,
            NameEntry {
                hash: HashedStr::new("variable.y"),
                text: "variable.y".into()
            }
        );
        assert_eq!(StoreRoot::Var(NameIdx(1)), StoreRoot::Var(NameIdx(1)));
        assert_ne!(StoreRoot::Var(NameIdx(1)), StoreRoot::Temp(TempIdx(1)));
        assert_ne!(StoreRoot::Other, StoreRoot::Temp(TempIdx(0)));
        let store = MemberStore {
            root: StoreRoot::Var(NameIdx(0)),
            path: vec![HashedStr::new("a")].into_boxed_slice(),
        };
        assert_eq!(store.clone(), store);
    }
}
