//! The bytecode builder: from the optimised tree to a [`Program`].
//!
//! The tree is walked in evaluation order, which fixes the order of side effects and random draws:
//! binary operators and functions evaluate their first operand, push it, then the next; a division
//! evaluates the **divisor** first, runs its guard, then the numerator.
//!
//! The first render-controller array or resource reference met in that order ends the build
//! without a program (`geometry.a / array.b[0]` uses arrays). An assignment whose target the
//! validator kept cannot be built: the link fails and [`compile`](crate::compile::compile()) logs
//! `expression '{}' compile failed`, after `Error: You cannot write to a variable on another mob.`
//! for a target behind `->`.
//!
//! The tree is at most 256 levels deep, so the recursion here is bounded. A failed build drops the
//! builder, so nothing is restored on the way out of an error.

use crate::compile::{
    Cx,
    ast::{Node, Payload},
    program::{
        CmpOp, Depths, Divisor, EqOp, FLOAT_LOOP_TEMPS, Instr, MemberEntry, MemberStore, NameEntry,
        NameIdx, Program, ProgramFlags, QueryCall,
    },
};
use crate::numeric::PostOp;
use crate::ops::ExpressionOp as Op;

mod arith;
mod calls;
mod code;
mod control;
mod logic;
mod pools;
mod values;

use code::{Code, Depth};
use control::LoopTargets;
use logic::Junction;
use pools::Pool;

/// Why a tree could not be linked.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub(super) enum LinkError {
    /// The expression uses a render-controller array, which the caller resolves first.
    UsesArrays,
    /// The expression uses a resource variable, which the caller resolves first.
    UsesResources,
    /// The link failed: `expression '{}' compile failed` follows.
    Failed,
}

type Build = Result<(), LinkError>;

/// A finished argument sub-program: argument `index` of `calls[call]`.
struct Argument {
    call: usize,
    index: usize,
    code: Code,
}

/// The builder state shared by the main program and every argument sub-program.
struct Builder<'c, 'o> {
    cx: &'c mut Cx<'o>,
    divisor: Divisor,
    code: Code,
    /// Keyed by their bits.
    consts: Pool<u32, f32>,
    /// Keyed by the bits of scale and offset.
    posts: Pool<(u32, u32), PostOp>,
    hashes: Pool<u64, u64>,
    names: Pool<u64, NameEntry>,
    /// The name of each temp slot, keyed by its index.
    temps: Pool<u16, NameIdx>,
    /// Keyed by the hashes of the member and of the last member of its path.
    members: Pool<(u64, u64), MemberEntry>,
    stores: Vec<MemberStore>,
    calls: Vec<QueryCall>,
    args: Vec<Argument>,
    /// The loops around the code being built, innermost last.
    loops: Vec<LoopTargets>,
    depth: Depth,
    flags: ProgramFlags,
    /// Whether the program can start on the float-only loop: [`ProgramFlags::FLOAT_ONLY`], or
    /// that but for queries called without arguments ([`Program::float_loop`]).
    float_loop: bool,
}

pub(super) fn build(cx: &mut Cx<'_>, root: &Node) -> Result<Program, LinkError> {
    let mut b = Builder::new(cx);
    b.expr(root)?;
    b.code.emit(Instr::End);
    b.finish()
}

impl<'c, 'o> Builder<'c, 'o> {
    fn new(cx: &'c mut Cx<'o>) -> Self {
        Self {
            divisor: if cx.version().signed_division_fix() {
                Divisor::Signed
            } else {
                Divisor::Absolute
            },
            cx,
            code: Code::default(),
            consts: Pool::default(),
            // Index 0 is the identity, the plain form, which is never looked up.
            posts: Pool::starting_with(PostOp::IDENTITY),
            hashes: Pool::default(),
            names: Pool::default(),
            temps: Pool::default(),
            members: Pool::default(),
            stores: Vec::new(),
            calls: Vec::new(),
            args: Vec::new(),
            loops: Vec::new(),
            depth: Depth::default(),
            flags: ProgramFlags::FLOAT_ONLY,
            float_loop: true,
        }
    }

    /// Links the main program and the argument sub-programs after it.
    fn finish(mut self) -> Result<Program, LinkError> {
        let mut code = std::mem::take(&mut self.code).finish(0)?;
        for Argument {
            call,
            index,
            code: sub,
        } in std::mem::take(&mut self.args)
        {
            let start = u32::try_from(code.len()).map_err(|_| LinkError::Failed)?;
            code.extend(sub.finish(start)?);
            self.calls[call].args[index] = start;
        }
        let depths = self.depth.max;
        if depths.stack > Depths::FLOAT_LOOP.stack
            || depths.loops > Depths::FLOAT_LOOP.loops
            || depths.handlers > Depths::FLOAT_LOOP.handlers
            || self.temps.len() > FLOAT_LOOP_TEMPS
        {
            self.not_float_only();
        }
        Ok(Program {
            code: code.into_boxed_slice(),
            consts: self.consts.into_boxed_slice(),
            posts: self.posts.into_boxed_slice(),
            hashes: self.hashes.into_boxed_slice(),
            names: self.names.into_boxed_slice(),
            temps: self.temps.into_boxed_slice(),
            members: self.members.into_boxed_slice(),
            stores: self.stores.into_boxed_slice(),
            calls: self.calls.into_boxed_slice(),
            catalog: self.cx.opts.catalog.clone(),
            math: self.cx.opts.math.cloned(),
            depths,
            flags: self.flags,
            float_loop: self.float_loop,
            version: self.cx.version(),
        })
    }

    /// Emits the code of `node`: afterwards `acc` holds its value.
    ///
    /// The walk recurses through here once per tree level (up to 256), so it only dispatches: each
    /// case with locals is a separate `#[inline(never)]` function. Otherwise an unoptimised build
    /// gives every level a frame with every case's locals (about 4 KiB per level, overflowing a
    /// 512 KiB thread at 250 levels).
    ///
    /// Every op has an arm, so a new op does not compile until it is given one.
    #[allow(clippy::too_many_lines, reason = "one arm per op")]
    fn expr(&mut self, node: &Node) -> Build {
        let op = node.op;
        match op {
            Op::ArrayVariable | Op::Array | Op::ExpressionArray => Err(LinkError::UsesArrays),
            Op::GeometryVariable | Op::MaterialVariable | Op::TextureVariable => {
                Err(LinkError::UsesResources)
            }
            Op::Float => self.float(node),
            Op::StringLiteral => self.string_literal(node),
            Op::Geometry | Op::Material | Op::Texture => self.resource(node),
            Op::This => self.this(node),
            Op::EntityVariable | Op::TempVariable | Op::ContextVariable => self.load(node),
            Op::MemberAccessor => self.member_read(node),
            Op::QueryFunction => self.query(node),

            Op::LessThan => self.compare(node, CmpOp::Lt),
            Op::LessEqual => self.compare(node, CmpOp::Le),
            Op::GreaterEqual => self.compare(node, CmpOp::Ge),
            Op::GreaterThan => self.compare(node, CmpOp::Gt),
            Op::LogicalEqual => self.equal(node, EqOp::Eq),
            Op::LogicalNotEqual => self.equal(node, EqOp::Ne),
            Op::LogicalAnd => self.logic(node, Junction::And),
            Op::LogicalOr => self.logic(node, Junction::Or),

            Op::NullCoalescing => self.coalesce(node),
            Op::Conditional => self.conditional(node),
            Op::Semicolon => self.statements(node),
            Op::Loop => self.counted_loop(node),
            Op::ForEach => self.for_each(node),
            Op::Break => {
                self.jump_out(|targets| targets.cleanup);
                Ok(())
            }
            Op::Continue => {
                self.jump_out(|targets| targets.check);
                Ok(())
            }
            Op::Assignment => self.assignment(node),
            Op::Pointer => self.pointer(node),
            // Sections never survive optimisation; a section builds as its first child.
            Op::LeftBracket | Op::LeftParenthesis => self.expr(first_child(node)?),

            Op::Negate => self.unary(node, |p| Instr::Negate { p }),
            Op::LogicalNot => self.unary(node, |p| Instr::Not { p }),
            Op::Add => self.add(node),
            Op::Mul => self.mul(node),
            Op::Div => self.div(node),
            Op::HostMath | Op::HostMathVolatile => self.host_math(node),

            // Without the stdlib the lexer makes no `math.*` function.
            #[cfg(feature = "stdlib")]
            Op::Mod
            | Op::Random
            | Op::RandomInt
            | Op::DieRoll
            | Op::DieRollInt
            | Op::Abs
            | Op::Acos
            | Op::Asin
            | Op::Atan
            | Op::Ceil
            | Op::Cos
            | Op::Exp
            | Op::Floor
            | Op::HermiteBlend
            | Op::Ln
            | Op::MinAngle
            | Op::Round
            | Op::Sin
            | Op::Sign
            | Op::Sqrt
            | Op::Trunc
            | Op::Atan2
            | Op::CopySign
            | Op::Max
            | Op::Min
            | Op::Pow
            | Op::Clamp
            | Op::Lerp
            | Op::LerpRotate
            | Op::InverseLerp
            | Op::EaseInQuad
            | Op::EaseOutQuad
            | Op::EaseInOutQuad
            | Op::EaseInCubic
            | Op::EaseOutCubic
            | Op::EaseInOutCubic
            | Op::EaseInQuart
            | Op::EaseOutQuart
            | Op::EaseInOutQuart
            | Op::EaseInQuint
            | Op::EaseOutQuint
            | Op::EaseInOutQuint
            | Op::EaseInSine
            | Op::EaseOutSine
            | Op::EaseInOutSine
            | Op::EaseInExpo
            | Op::EaseOutExpo
            | Op::EaseInOutExpo
            | Op::EaseInCirc
            | Op::EaseOutCirc
            | Op::EaseInOutCirc
            | Op::EaseInBounce
            | Op::EaseOutBounce
            | Op::EaseInOutBounce
            | Op::EaseInBack
            | Op::EaseOutBack
            | Op::EaseInOutBack
            | Op::EaseInElastic
            | Op::EaseOutElastic
            | Op::EaseInOutElastic => self.math(node, op),

            // `Return` outside a statement list, `Pi`, `{`, the closing tokens, `:` and `,`: the
            // node's value with its post-op applied at build time.
            Op::Return
            | Op::Pi
            | Op::LeftBrace
            | Op::RightBrace
            | Op::RightBracket
            | Op::RightParenthesis
            | Op::ConditionalElse
            | Op::Comma => self.generic_constant(node),
            #[cfg(not(feature = "stdlib"))]
            _ => Err(LinkError::Failed),
        }
    }
}

fn first_child(node: &Node) -> Result<&Node, LinkError> {
    node.children.first().ok_or(LinkError::Failed)
}

/// Debug builds assert there are no extra children; a release build ignores them rather than
/// failing the link.
fn two(node: &Node) -> Result<[&Node; 2], LinkError> {
    debug_assert!(
        node.children.len() <= 2,
        "{:?} has {} children, the builder reads two",
        node.op,
        node.children.len()
    );
    match node.children.as_slice() {
        [a, b, ..] => Ok([a, b]),
        _ => Err(LinkError::Failed),
    }
}

fn three(node: &Node) -> Result<[&Node; 3], LinkError> {
    debug_assert!(
        node.children.len() <= 3,
        "{:?} has {} children, the builder reads three",
        node.op,
        node.children.len()
    );
    match node.children.as_slice() {
        [a, b, c, ..] => Ok([a, b, c]),
        _ => Err(LinkError::Failed),
    }
}

/// The constant folded into a node's value: a float, or a string hash read as the float whose bits
/// are its low 32 bits.
fn payload_f32(node: &Node) -> f32 {
    match node.value {
        Payload::Float(v) => v,
        Payload::Hash(h) => f32::from_bits(h as u32),
        _ => 0.0,
    }
}

#[cfg(test)]
pub(crate) mod test_support {
    //! Helpers shared by the builder unit tests.

    use super::{LinkError, build};
    use crate::catalog::Side;
    use crate::compile::{
        CompileOptions, Cx, Expr,
        ast::{Name, Node, Payload, Span},
        compile,
        program::Program,
    };
    use crate::hash::HashedStr;
    use crate::numeric::PostOp;
    use crate::ops::ExpressionOp as Op;
    use crate::version::RawVersion;

    pub(super) fn opts(raw: i16) -> CompileOptions {
        CompileOptions::from_raw_version(
            crate::stdlib::queries(Side::Server).clone(),
            RawVersion(raw),
        )
    }

    #[track_caller]
    pub(super) fn compiled(src: &str, raw: i16) -> Expr {
        let c = compile(src, &opts(raw));
        assert_eq!(
            c.failure(),
            None,
            "{src}: {:?}",
            c.diagnostics()
                .iter()
                .map(|d| d.message().into_owned())
                .collect::<Vec<_>>()
        );
        c.expr().cloned().unwrap()
    }

    /// The disassembly of `src` at version 13.
    #[track_caller]
    pub(super) fn listing(src: &str) -> String {
        compiled(src, 13).program().unwrap().disassemble()
    }

    /// The program of `src` at version 13, built again from its compiled tree.
    #[track_caller]
    pub(super) fn program_of(src: &str) -> Program {
        built(compiled(src, 13).tree().unwrap()).unwrap()
    }

    /// The disassembly lines of `src` without their pc column.
    #[track_caller]
    pub(super) fn lines(src: &str) -> Vec<String> {
        listing(src).lines().map(|l| l[5..].to_owned()).collect()
    }

    pub(super) fn numbered<S: AsRef<str>>(lines: &[S]) -> String {
        lines
            .iter()
            .enumerate()
            .map(|(pc, l)| format!("{pc:4} {}\n", l.as_ref()))
            .collect::<Vec<_>>()
            .concat()
    }

    /// Asserts the disassembly of `src` at version 13, one line per instruction.
    #[track_caller]
    pub(super) fn assert_listing<S: AsRef<str>>(src: &str, expected: &[S]) {
        assert_eq!(listing(src), numbered(expected), "{src}");
    }

    /// Asserts the disassembly of a hand-built tree, one line per instruction.
    #[track_caller]
    pub(super) fn assert_built<S: AsRef<str>>(tree: &Node, expected: &[S]) {
        assert_eq!(
            built(tree).unwrap().disassemble(),
            numbered(expected),
            "{:?}",
            tree.op
        );
    }

    pub(super) fn hex(s: &str) -> String {
        format!("{:#018x}", HashedStr::new(s).as_u64())
    }

    pub(super) fn node(op: Op, value: Payload, children: Vec<Node>) -> Node {
        let mut node = Node::token(op, value, Span::new(0, 0));
        node.children = children;
        node
    }

    pub(super) fn op(op: Op, children: Vec<Node>) -> Node {
        node(op, Payload::None, children)
    }

    pub(super) fn float(v: f32) -> Node {
        node(Op::Float, Payload::Float(v), vec![])
    }

    pub(super) fn var(name: &str) -> Node {
        node(
            Op::EntityVariable,
            Payload::Entity(Name::new(format!("variable.{name}"))),
            vec![],
        )
    }

    pub(super) fn temp(name: &str) -> Node {
        node(
            Op::TempVariable,
            Payload::Temp(Name::new(format!("temp.{name}"))),
            vec![],
        )
    }

    pub(super) fn with_post(mut n: Node, scale: f32, offset: f32) -> Node {
        n.post = PostOp::new(scale, offset);
        n
    }

    /// The program of a hand-built tree at version 13.
    pub(super) fn built(tree: &Node) -> Result<Program, LinkError> {
        let opts = opts(13);
        let mut cx = Cx::for_test("", &opts);
        build(&mut cx, tree)
    }

    /// Binds `$b` to a builder at version 13.
    macro_rules! builder {
        ($b:ident) => {
            let options = $crate::compile::codegen::test_support::opts(13);
            let mut cx = $crate::compile::Cx::for_test("", &options);
            let mut $b = $crate::compile::codegen::Builder::new(&mut cx);
        };
    }

    pub(super) use builder;
}

#[cfg(test)]
mod tests {
    use crate::compile::{
        CompileFailure, Cx,
        ast::Payload,
        codegen::{LinkError, build, first_child, payload_f32, test_support::*, three, two},
        compile,
        program::ProgramFlags,
    };
    use crate::numeric::PostOp;
    use crate::ops::ExpressionOp as Op;
    use crate::stdlib::math;
    use crate::version::RawVersion;

    #[test]
    fn payload_f32_reads_floats_and_the_low_bits_of_hashes() {
        assert_eq!(payload_f32(&float(2.5)), 2.5);
        let hash = node(
            Op::StringLiteral,
            Payload::Hash(0xdead_beef_3fc0_0000),
            vec![],
        );
        assert_eq!(payload_f32(&hash), 1.5);
        assert_eq!(payload_f32(&op(Op::Add, vec![])), 0.0);
        assert_eq!(payload_f32(&var("x")), 0.0);
    }

    #[test]
    fn child_accessors_demand_enough_children() {
        let none = op(Op::Add, vec![]);
        let one = op(Op::Add, vec![float(1.0)]);
        let two_kids = op(Op::Add, vec![float(1.0), float(2.0)]);
        let three_kids = op(Op::Clamp, vec![float(1.0), float(2.0), float(3.0)]);
        assert_eq!(first_child(&none).unwrap_err(), LinkError::Failed);
        assert_eq!(first_child(&one).unwrap().float(), 1.0);
        assert_eq!(two(&none).unwrap_err(), LinkError::Failed);
        assert_eq!(two(&one).unwrap_err(), LinkError::Failed);
        let [a, b] = two(&two_kids).unwrap();
        assert_eq!((a.float(), b.float()), (1.0, 2.0));
        assert_eq!(three(&two_kids).unwrap_err(), LinkError::Failed);
        let [a, b, c] = three(&three_kids).unwrap();
        assert_eq!((a.float(), b.float(), c.float()), (1.0, 2.0, 3.0));
    }

    #[cfg(debug_assertions)]
    #[test]
    #[should_panic(expected = "the builder reads two")]
    fn two_asserts_on_a_third_child() {
        let _ = two(&op(Op::Add, vec![float(1.0), float(2.0), float(3.0)]));
    }

    #[cfg(debug_assertions)]
    #[test]
    #[should_panic(expected = "the builder reads three")]
    fn three_asserts_on_a_fourth_child() {
        let _ = three(&op(
            Op::Clamp,
            vec![float(1.0), float(2.0), float(3.0), float(4.0)],
        ));
    }

    #[test]
    fn a_block_section_is_walked_through_its_first_child() {
        let tree = op(Op::LeftBracket, vec![var("x")]);
        assert_built(&tree, &["load variable.x", "end"]);
        let tree = op(Op::LeftParenthesis, vec![op(Op::Abs, vec![var("x")])]);
        assert_built(&tree, &["load variable.x", "Abs", "end"]);
        assert_eq!(
            built(&op(Op::LeftParenthesis, vec![])).unwrap_err(),
            LinkError::Failed
        );
    }

    #[test]
    fn the_builder_never_sets_the_constant_flag() {
        // `Expr::flags` adds CONSTANT for a folded expression; the program itself does not carry
        // it.
        let p = program_of("1 + 2");
        assert_eq!(p.flags, ProgramFlags::FLOAT_ONLY);
        assert!(!p.flags.contains(ProgramFlags::CONSTANT));
        assert_eq!(
            compiled("1 + 2", 13).flags(),
            ProgramFlags::CONSTANT.union(ProgramFlags::FLOAT_ONLY)
        );
    }

    #[test]
    fn program_flags_follow_the_constructs() {
        let f = ProgramFlags::FLOAT_ONLY;
        let table: &[(&str, ProgramFlags)] = &[
            ("v.x * v.y", f.union(ProgramFlags::READS_ACTOR_VARS)),
            ("this", f),
            ("c.foo", f.union(ProgramFlags::READS_CONTEXT)),
            (
                "t.a = 1;",
                f.union(ProgramFlags::HAS_ASSIGNMENT)
                    .union(ProgramFlags::USES_TEMPS),
            ),
            ("v.a = 1;", f.union(ProgramFlags::HAS_ASSIGNMENT)),
            ("'moo'", ProgramFlags::empty()),
            ("v.x == 'abc'", ProgramFlags::READS_ACTOR_VARS),
            ("v.s = 'abc';", ProgramFlags::HAS_ASSIGNMENT),
            (
                "v.s.b",
                ProgramFlags::READS_ACTOR_VARS.union(ProgramFlags::USES_MEMBERS),
            ),
            (
                "v.s.a = 1;",
                ProgramFlags::HAS_ASSIGNMENT.union(ProgramFlags::USES_MEMBERS),
            ),
            ("q.is_baby", ProgramFlags::USES_QUERIES),
            (
                "c.o->v.x",
                ProgramFlags::USES_ARROW
                    .union(ProgramFlags::READS_ACTOR_VARS)
                    .union(ProgramFlags::READS_CONTEXT),
            ),
            (
                "loop(1, { v.x = 1; });",
                f.union(ProgramFlags::HAS_LOOPS)
                    .union(ProgramFlags::HAS_ASSIGNMENT),
            ),
            (
                "for_each(t.e, v.arr, { t.n = 1; });",
                ProgramFlags::HAS_LOOPS
                    .union(ProgramFlags::HAS_ASSIGNMENT)
                    .union(ProgramFlags::READS_ACTOR_VARS)
                    .union(ProgramFlags::USES_TEMPS),
            ),
            ("math.die_roll(1, 1, 6)", f.union(ProgramFlags::USES_RANDOM)),
            (
                "math.random(1, 6)",
                f.union(ProgramFlags::USES_RANDOM)
                    .union(ProgramFlags::USES_RANDOM_OP),
            ),
            ("break;", f),
        ];
        for (src, flags) in table {
            assert_eq!(compiled(src, 13).program().unwrap().flags, *flags, "{src}");
        }
    }

    #[test]
    fn loops_that_only_loop_do_not_read_variables() {
        // `HAS_LOOPS` without `READS_ACTOR_VARS`: the count is a constant, the body writes a temp.
        let p = program_of("loop(2, { t.a = 1; });");
        assert!(
            p.flags.contains(ProgramFlags::HAS_LOOPS) && p.flags.contains(ProgramFlags::USES_TEMPS)
        );
        assert!(!p.flags.contains(ProgramFlags::READS_ACTOR_VARS));
    }

    #[test]
    fn a_program_records_its_version() {
        let p = compiled("v.x", 9).program().unwrap().version;
        assert_eq!(p, RawVersion(9).effective());
    }

    /// `v.a0 * (v.a1 * (… * v.a{n-1}))`: `n - 1` pushed operands.
    fn right_nested_product(n: usize) -> String {
        let mut src = format!("v.a{}", n - 1);
        for i in (0..n - 1).rev() {
            src = format!("v.a{i} * ({src})");
        }
        src
    }

    #[test]
    fn the_operand_stack_limit_of_the_float_loop_is_sixteen() {
        // 17 operands: 16 pushed.
        let at_limit = program_of(&right_nested_product(17));
        assert_eq!(at_limit.depths.stack, 16);
        assert!(at_limit.flags.contains(ProgramFlags::FLOAT_ONLY));
        assert!(at_limit.float_loop);
        // 18 operands: 17 pushed.
        let past = program_of(&right_nested_product(18));
        assert_eq!(past.depths.stack, 17);
        assert!(!past.flags.contains(ProgramFlags::FLOAT_ONLY));
        assert!(!past.float_loop);
        assert!(past.flags.contains(ProgramFlags::READS_ACTOR_VARS));
    }

    #[test]
    fn the_operand_stack_limit_also_decides_the_loop_of_an_argument_less_query() {
        let small = program_of(
            "v.a * (v.b * (v.c * (v.d * (v.e * (v.f * (v.g * (v.h * (v.i * (v.j * (v.k * (v.l * (v.m * (v.n * (v.o * (v.p * q.is_baby)))))))))))))))",
        );
        assert_eq!(small.depths.stack, 16);
        assert!(small.float_loop);
        assert!(!small.flags.contains(ProgramFlags::FLOAT_ONLY));
        let past = program_of(
            "v.a * (v.b * (v.c * (v.d * (v.e * (v.f * (v.g * (v.h * (v.i * (v.j * (v.k * (v.l * (v.m * (v.n * (v.o * (v.p * (v.q * q.is_baby))))))))))))))))",
        );
        assert_eq!(past.depths.stack, 17);
        assert!(!past.float_loop);
    }

    fn nested_loops(n: usize, body: &str) -> String {
        let mut src = body.to_owned();
        for _ in 0..n {
            src = format!("loop(1, {{ {src} }});");
        }
        src
    }

    #[test]
    fn the_loop_nesting_limit_of_the_float_loop_is_four() {
        let at_limit = program_of(&nested_loops(4, "v.x = 1;"));
        assert_eq!((at_limit.depths.loops, at_limit.depths.stack), (4, 4));
        assert!(at_limit.flags.contains(ProgramFlags::FLOAT_ONLY) && at_limit.float_loop);
        let past = program_of(&nested_loops(5, "v.x = 1;"));
        assert_eq!((past.depths.loops, past.depths.stack), (5, 5));
        assert!(!past.flags.contains(ProgramFlags::FLOAT_ONLY) && !past.float_loop);
        assert!(past.flags.contains(ProgramFlags::HAS_LOOPS));
    }

    #[test]
    fn the_loop_limit_also_applies_to_an_argument_less_query_inside() {
        let at_limit = program_of(&nested_loops(4, "v.x = q.is_baby;"));
        assert!(at_limit.float_loop);
        assert_eq!(at_limit.depths.loops, 4);
        let past = program_of(&nested_loops(5, "v.x = q.is_baby;"));
        assert!(!past.float_loop);
    }

    #[test]
    fn the_temp_limit_of_the_float_loop_is_eight() {
        let temps = |n: usize| {
            (0..n)
                .map(|i| format!("t.a{i} = 1;"))
                .collect::<Vec<_>>()
                .concat()
        };
        let at_limit = program_of(&temps(8));
        assert_eq!(at_limit.temps.len(), 8);
        assert!(at_limit.flags.contains(ProgramFlags::FLOAT_ONLY) && at_limit.float_loop);
        let past = program_of(&temps(9));
        assert_eq!(past.temps.len(), 9);
        assert!(!past.flags.contains(ProgramFlags::FLOAT_ONLY) && !past.float_loop);
        assert!(past.flags.contains(ProgramFlags::USES_TEMPS));
        // Writing the same temp again does not count twice.
        let repeated = program_of(&format!("{}t.a0 = 2;", temps(8)));
        assert_eq!(repeated.temps.len(), 8);
        assert!(repeated.float_loop);
    }

    #[test]
    fn the_temp_limit_also_applies_to_an_argument_less_query() {
        let temps = |n: usize| {
            format!(
                "t.q = q.is_baby;{}",
                (1..n)
                    .map(|i| format!("t.a{i} = 1;"))
                    .collect::<Vec<_>>()
                    .concat()
            )
        };
        assert!(program_of(&temps(8)).float_loop);
        assert!(!program_of(&temps(9)).float_loop);
    }

    #[test]
    fn the_handler_limit_of_the_float_loop_is_four() {
        let nested = |levels: usize| {
            let mut tree = var("a");
            for _ in 0..levels {
                tree = op(Op::NullCoalescing, vec![tree, float(1.0)]);
            }
            tree
        };
        let at_limit = built(&nested(4)).unwrap();
        assert_eq!(at_limit.depths.handlers, 4);
        assert!(at_limit.flags.contains(ProgramFlags::FLOAT_ONLY) && at_limit.float_loop);
        let past = built(&nested(5)).unwrap();
        assert_eq!(past.depths.handlers, 5);
        assert!(!past.flags.contains(ProgramFlags::FLOAT_ONLY) && !past.float_loop);
    }

    #[test]
    fn arrays_need_resolution() {
        for src in ["array.a[0]", "array.a"] {
            let c = compile(src, &opts(13));
            assert_eq!(c.failure(), Some(CompileFailure::UsesArrays), "{src}");
            assert!(c.expr().is_none());
        }
        for op_ in [Op::ArrayVariable, Op::Array, Op::ExpressionArray] {
            assert_eq!(
                built(&op(op_, vec![])).unwrap_err(),
                LinkError::UsesArrays,
                "{op_:?}"
            );
        }
    }

    #[test]
    fn resource_variables_need_resolution() {
        for src in ["geometry.default", "material.default", "texture.default"] {
            let c = compile(src, &opts(13));
            assert_eq!(c.failure(), Some(CompileFailure::UsesResources), "{src}");
            assert!(c.expr().is_none());
        }
        for op_ in [
            Op::GeometryVariable,
            Op::MaterialVariable,
            Op::TextureVariable,
        ] {
            assert_eq!(
                built(&op(op_, vec![])).unwrap_err(),
                LinkError::UsesResources,
                "{op_:?}"
            );
        }
    }

    #[test]
    fn the_first_link_failure_in_emission_order_wins() {
        // A division builds its divisor first.
        assert_eq!(
            compile("geometry.a / array.b[0]", &opts(2)).failure(),
            Some(CompileFailure::UsesArrays)
        );
        assert_eq!(
            compile("array.b[0] / geometry.a", &opts(2)).failure(),
            Some(CompileFailure::UsesResources)
        );
        // Every other operator builds its first operand first.
        let array = || op(Op::ArrayVariable, vec![]);
        let geometry = || op(Op::GeometryVariable, vec![]);
        for op_ in [Op::Add, Op::Mul, Op::Max] {
            assert_eq!(
                built(&op(op_, vec![array(), geometry()])).unwrap_err(),
                LinkError::UsesArrays,
                "{op_:?}"
            );
            assert_eq!(
                built(&op(op_, vec![geometry(), array()])).unwrap_err(),
                LinkError::UsesResources,
                "{op_:?}"
            );
        }
        assert_eq!(
            built(&op(Op::Div, vec![array(), geometry()])).unwrap_err(),
            LinkError::UsesResources
        );
        assert_eq!(
            built(&op(Op::Div, vec![geometry(), array()])).unwrap_err(),
            LinkError::UsesArrays
        );
    }

    #[test]
    fn the_link_failure_logs_only_the_compile_failed_message_for_other_causes() {
        // An assignment target the validator should have caught fails without the mob message.
        let bad = op(Op::Assignment, vec![float(1.0), float(2.0)]);
        let options = opts(13);
        let mut cx = Cx::for_test("", &options);
        assert_eq!(build(&mut cx, &bad).unwrap_err(), LinkError::Failed);
        assert!(cx.logged_diagnostics().is_empty());
    }

    #[test]
    fn constants_folded_into_a_random_or_a_loop_use_the_numeric_functions() {
        // The arm64 behaviour rounds the offset of the folded bounds once: the pooled post-op is
        // `random_const_bounds`'s.
        let (lo, hi, post) = (1.1_f32, 9.0, PostOp::new(3.3, 0.7));
        let mut random = op(Op::Random, vec![float(lo), float(hi)]);
        random.post = post;
        let options = opts(13);
        let mut cx = Cx::for_test("", &options);
        let p = build(&mut cx, &random).unwrap();
        assert_eq!(p.posts[1], math::random_const_bounds(lo, hi, post));
        // A loop's exit constant is the loop's post-op applied to 0.
        let body = op(Op::Semicolon, vec![]);
        let mut looped = op(Op::Loop, vec![float(1.0), body]);
        looped.post = post;
        let p = build(&mut cx, &looped).unwrap();
        assert!(p.consts.contains(&post.apply(0.0)));
    }

    #[test]
    fn every_surviving_op_links() {
        let sources = [
            "v.x",
            "t.x = 1; return t.x;",
            "c.x",
            "this",
            "'abc'",
            "v.a.b",
            "q.is_baby",
            "!v.x",
            "v.x + v.y + v.z",
            "v.x * v.y",
            "v.x / v.y",
            "math.mod(v.x, v.y)",
            "math.mod(v.x, 3)",
            "math.abs(v.x) + math.acos(v.x) + math.asin(v.x) + math.atan(v.x) + math.ceil(v.x) + math.cos(v.x)",
            "math.exp(v.x) + math.floor(v.x) + math.hermite_blend(v.x) + math.ln(v.x) + math.min_angle(v.x)",
            "math.round(v.x) + math.sin(v.x) + math.sign(v.x) + math.sqrt(v.x) + math.trunc(v.x)",
            "math.atan2(v.x, v.y) + math.copy_sign(v.x, v.y) + math.max(v.x, v.y) + math.min(v.x, v.y) + math.pow(v.x, v.y)",
            "math.max(v.x, 2) + math.min(2, v.x) + math.pow(v.x, 2)",
            "math.clamp(v.x, 0, 1) + math.lerp(v.a, v.b, v.t) + math.lerprotate(v.a, v.b, v.t) + math.inverse_lerp(v.a, v.b, v.t)",
            "math.ease_in_out_elastic(v.a, v.b, v.t) + math.ease_out_bounce(v.a, v.b, v.t)",
            "math.random(1, 2) + math.random(v.x, 2) + math.random_integer(1, 6) + math.random_integer(v.x, 6)",
            "math.die_roll(2, 1, 6) + math.die_roll_integer(2, 1, 6)",
            "v.x < v.y",
            "v.x < 1",
            "v.x <= 1 && v.x >= 0 || v.x > 5",
            "v.x == v.y",
            "v.x == 1",
            "v.x != 'abc'",
            "v.x ?? 1",
            "v.x ? 1 : 2",
            "v.x ? 1",
            "loop(3, {v.x = v.x + 1; v.x > 1 ? break; v.x < 0 ? continue;});",
            "for_each(t.e, v.arr, {t.n = 1;});",
            "c.other->v.x",
            "c.other->q.is_baby",
            "v.a.b = 1;",
            "t.a.b = 1;",
            "return 1;",
            "break;",
        ];
        for source in sources {
            let compiled = compile(source, &opts(13));
            assert_eq!(
                compiled.failure(),
                None,
                "{source}: {:?}",
                compiled.diagnostics()
            );
            let expr = compiled.expr().cloned().expect("an expression");
            assert!(!expr.disassemble().is_empty(), "{source}");
        }
    }
}
