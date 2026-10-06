//! The parser: groups the flat token list into one tree.
//!
//! A fixed sequence of grouping passes runs over the list, each folding one kind of construct and
//! running only when one of its operators occurs. The pass order is the precedence (`/` binds
//! tighter than `*`, `??` looser than `?:`) and decides which error a malformed text reports.
//! Below version 5 the conditional is two left-associative binary operators; below 6 the
//! comparisons are six levels and `||` binds tighter than `&&`.
//!
//! Each pass rebuilds a list in one linear sweep and accepts any depth (the optimiser enforces the
//! nesting limit). No pass recurses natively: [`driver::run`] drives each [`driver::Level`] on an
//! explicit stack, so deep nesting cannot overflow the native stack.

use crate::compile::{Cx, Failed, Pass, ast::Node};
use crate::diag::{LanguageMessage as Msg, fixed6};
use crate::ops::{ExpressionOp as Op, OpSet};

mod binary;
mod calls;
mod conditional;
mod driver;
mod sections;
mod statements;
mod unary;

pub(super) use driver::TokenClasses;

use binary::binary;
use calls::{arrays, calls, math_functions};
use conditional::ternary;
use sections::{member_accessors, sections};
use statements::semicolons;
use unary::{prefix, unary_minus_and_not};

/// `node` with `children` appended to its children.
fn with_children<const N: usize>(mut node: Node, children: [Node; N]) -> Node {
    // Exact: a tree holds a child list per operator.
    node.children.reserve_exact(N);
    node.children.extend(children);
    node
}

/// The `math.*` calls the math-function pass groups: every math function except `math.pi`, and
/// the host's.
pub(super) fn is_math_call(op: Op) -> bool {
    (op.is_math_function() && op != Op::Pi) || matches!(op, Op::HostMath | Op::HostMathVolatile)
}

/// Groups the token list into one root, or logs why it cannot.
pub(super) fn group_tokens(cx: &mut Cx<'_>, mut list: Vec<Node>, used: OpSet) -> Option<Node> {
    if group(cx, &mut list, used).is_err() {
        return None;
    }
    if list.len() == 1 {
        return list.pop();
    }
    let mut remaining = String::new();
    for node in &list {
        remaining.push('\t');
        match node.op {
            Op::Float => remaining.push_str(&fixed6(node.float())),
            op => remaining.push_str(op.token().unwrap_or("")),
        }
        remaining.push('\n');
    }
    let span = match (list.first(), list.last()) {
        (Some(head), Some(tail)) => head.full_span().to(tail.full_span()),
        _ => cx.whole(),
    };
    cx.language(Msg::MultipleRoots, span, &[&remaining]);
    None
}

/// Runs the grouping passes in their fixed order.
fn group(cx: &mut Cx<'_>, list: &mut Vec<Node>, used: OpSet) -> Pass {
    let version = cx.version().as_i16();
    ends_with_semicolon(cx, list, used)?;
    if used.contains(Op::MemberAccessor) {
        member_accessors(cx, list)?;
    }
    sections_and_calls(cx, list, used)?;
    pointer_and_arithmetic(cx, list, used)?;
    comparisons(cx, list, used, version)?;
    logic(cx, list, used, version)?;
    statement_operators(cx, list, used)
}

/// A complex expression (one that contains `=` or `;`) must end with `;`.
#[inline]
fn ends_with_semicolon(cx: &mut Cx<'_>, list: &[Node], used: OpSet) -> Pass {
    if (used.contains(Op::Assignment) || used.contains(Op::Semicolon))
        && !list.last().is_some_and(|n| n.is(Op::Semicolon))
    {
        let span = list.last().map_or(cx.whole(), |n| n.span);
        cx.language(Msg::ComplexMustEndWithSemicolon, span, &[]);
        return Err(Failed);
    }
    Ok(())
}

/// Sections, calls, statements, array indexing.
#[inline]
fn sections_and_calls(cx: &mut Cx<'_>, list: &mut Vec<Node>, used: OpSet) -> Pass {
    let u = |op: Op| used.contains(op);
    let sections_used = u(Op::LeftBrace) || u(Op::LeftBracket) || u(Op::LeftParenthesis);
    if sections_used || u(Op::QueryFunction) || u(Op::ArrayVariable) || u(Op::Semicolon) {
        if sections_used {
            sections(cx, list)?;
        }
        if u(Op::QueryFunction) || u(Op::Loop) || u(Op::ForEach) {
            calls(cx, list)?;
        }
        if u(Op::Semicolon) {
            semicolons(cx, list)?;
        }
        if u(Op::ArrayVariable) {
            arrays(cx, list)?;
        }
    }
    Ok(())
}

/// `->`, math functions, unary `-` and `!`, `/`, `*`, `+`.
#[inline]
fn pointer_and_arithmetic(cx: &mut Cx<'_>, list: &mut Vec<Node>, used: OpSet) -> Pass {
    let u = |op: Op| used.contains(op);
    // Unlike every other pass, a failure of `->` does not return: it leaves the list empty, the
    // remaining passes run on it and the root check reports it.
    if u(Op::Pointer) {
        let _ = binary(cx, list, &[Op::Pointer]);
    }
    if used.iter().any(is_math_call) {
        math_functions(cx, list)?;
    }
    if u(Op::Negate) || u(Op::LogicalNot) {
        unary_minus_and_not(cx, list)?;
    }
    if u(Op::Div) {
        binary(cx, list, &[Op::Div])?;
    }
    if u(Op::Mul) {
        binary(cx, list, &[Op::Mul])?;
    }
    if u(Op::Negate) || u(Op::Add) {
        binary(cx, list, &[Op::Add])?;
    }
    Ok(())
}

#[inline]
fn comparisons(cx: &mut Cx<'_>, list: &mut Vec<Node>, used: OpSet, version: i16) -> Pass {
    let u = |op: Op| used.contains(op);
    if version < 6 {
        for op in [
            Op::LessThan,
            Op::LogicalEqual,
            Op::GreaterEqual,
            Op::GreaterThan,
            Op::LessEqual,
            Op::LogicalNotEqual,
        ] {
            if u(op) {
                binary(cx, list, &[op])?;
            }
        }
    } else {
        if u(Op::LessThan) || u(Op::LessEqual) || u(Op::GreaterEqual) || u(Op::GreaterThan) {
            binary(
                cx,
                list,
                &[
                    Op::LessThan,
                    Op::LessEqual,
                    Op::GreaterEqual,
                    Op::GreaterThan,
                ],
            )?;
        }
        if u(Op::LogicalEqual) || u(Op::LogicalNotEqual) {
            binary(cx, list, &[Op::LogicalEqual, Op::LogicalNotEqual])?;
        }
    }
    Ok(())
}

/// Logical operators, the conditional, `??`.
#[inline]
fn logic(cx: &mut Cx<'_>, list: &mut Vec<Node>, used: OpSet, version: i16) -> Pass {
    let u = |op: Op| used.contains(op);
    if !(u(Op::LogicalOr)
        || u(Op::LogicalAnd)
        || u(Op::NullCoalescing)
        || u(Op::Conditional)
        || u(Op::ConditionalElse))
    {
        return Ok(());
    }
    if version <= 5 && u(Op::LogicalOr) {
        binary(cx, list, &[Op::LogicalOr])?;
    }
    if u(Op::LogicalAnd) {
        binary(cx, list, &[Op::LogicalAnd])?;
    }
    if version >= 6 && u(Op::LogicalOr) {
        binary(cx, list, &[Op::LogicalOr])?;
    }
    conditional(cx, list, used, version)?;
    if u(Op::NullCoalescing) {
        binary(cx, list, &[Op::NullCoalescing])?;
    }
    Ok(())
}

/// From version 5 one ternary pass, below it `:` then `?` as two binary operators.
#[inline]
fn conditional(cx: &mut Cx<'_>, list: &mut Vec<Node>, used: OpSet, version: i16) -> Pass {
    let u = |op: Op| used.contains(op);
    if version >= 5 {
        if u(Op::Conditional) || u(Op::ConditionalElse) {
            ternary(cx, list)?;
        }
    } else {
        if u(Op::ConditionalElse) {
            binary(cx, list, &[Op::ConditionalElse])?;
        }
        if u(Op::Conditional) {
            binary(cx, list, &[Op::Conditional])?;
        }
    }
    Ok(())
}

/// `,`, `=`, `return`.
#[inline]
fn statement_operators(cx: &mut Cx<'_>, list: &mut Vec<Node>, used: OpSet) -> Pass {
    if used.contains(Op::Comma) {
        binary(cx, list, &[Op::Comma])?;
    }
    if used.contains(Op::Assignment) {
        binary(cx, list, &[Op::Assignment])?;
    }
    if used.contains(Op::Return) {
        prefix(cx, list, Op::Return)?;
    }
    Ok(())
}

#[cfg(test)]
pub(crate) mod test_support {
    //! Helpers for the unit tests of the grouping passes: lex, run named passes, render the list.
    //!
    //! A rendered node is `(label child ...)`, a leaf its bare label.

    use super::*;
    use crate::catalog::Side;
    use crate::compile::{CompileOptions, ast::Payload};
    use crate::version::RawVersion;

    use crate::compile::lex;

    pub(super) type Msg3 = (&'static str, (u32, u32), String);

    /// The label of a node: its number, name or op name.
    pub(super) fn label(node: &Node) -> String {
        if node.is(Op::Array) {
            return "Array".to_owned();
        }
        match &node.value {
            Payload::Float(v) => crate::compile::ast::format_g(f64::from(*v), 9),
            Payload::Entity(n)
            | Payload::Temp(n)
            | Payload::Context(n)
            | Payload::ArrayVariable(n)
            | Payload::Geometry(n)
            | Payload::Material(n)
            | Payload::Texture(n) => n.as_str().to_owned(),
            Payload::Query(q) => crate::reference_catalog::catalog()
                .decl(q.index)
                .name()
                .to_owned(),
            Payload::HostMath(f) => crate::compile::test_support::host_math()
                .decl(*f)
                .name()
                .to_owned(),
            Payload::Hash(h) => format!("#{h}"),
            Payload::Member(n) => format!(".{}", n.as_str()),
            Payload::None => node.op.meta().name.to_owned(),
        }
    }

    pub(super) fn show(node: &Node) -> String {
        if node.is_leaf() {
            label(node)
        } else {
            let kids: Vec<String> = node.children.iter().map(show).collect();
            format!("({} {})", label(node), kids.join(" "))
        }
    }

    pub(super) fn show_list(list: &[Node]) -> String {
        list.iter().map(show).collect::<Vec<_>>().join(" | ")
    }

    pub(super) fn apply(cx: &mut Cx<'_>, pass: &str, list: &mut Vec<Node>) -> Pass {
        match pass {
            "member" => member_accessors(cx, list),
            "sections" => sections(cx, list),
            "calls" => calls(cx, list),
            "math" => math_functions(cx, list),
            "semi" => semicolons(cx, list),
            "arrays" => arrays(cx, list),
            "unary" => unary_minus_and_not(cx, list),
            "ternary" => ternary(cx, list),
            "return" => prefix(cx, list, Op::Return),
            "add" => binary(cx, list, &[Op::Add]),
            "div" => binary(cx, list, &[Op::Div]),
            "mul" => binary(cx, list, &[Op::Mul]),
            "comma" => binary(cx, list, &[Op::Comma]),
            "assign" => binary(cx, list, &[Op::Assignment]),
            "ptr" => binary(cx, list, &[Op::Pointer]),
            "lt" => binary(cx, list, &[Op::LessThan]),
            other => panic!("no pass {other}"),
        }
    }

    pub(super) fn messages(cx: &Cx<'_>) -> Vec<Msg3> {
        cx.logged_diagnostics()
            .iter()
            .map(|d| {
                (
                    d.language_message().map_or("-", |v| v.id()),
                    (d.span().start, d.span().end),
                    d.message().into_owned(),
                )
            })
            .collect()
    }

    pub(super) fn options(raw: i16) -> CompileOptions {
        CompileOptions::from_raw_version(
            crate::stdlib::queries(Side::Client).clone(),
            RawVersion(raw),
        )
    }

    /// Lexes `src` and runs the passes of `chain` (`+`-separated) in order, stopping at a failure.
    fn chain_nodes(chain: &str, src: &str, raw: i16) -> (Result<(), ()>, Vec<Node>, Vec<Msg3>) {
        let opts = options(raw);
        let mut cx = Cx::for_test(src, &opts);
        let tokens = lex::scan(&mut cx, &lex::lower(src)).expect("the source lexes");
        let mut list = tokens.nodes;
        let result = chain
            .split('+')
            .try_for_each(|pass| apply(&mut cx, pass, &mut list))
            .map_err(|_| ());
        (result, list, messages(&cx))
    }

    /// [`chain_nodes`] with the list rendered.
    pub(super) fn run_chain(
        chain: &str,
        src: &str,
        raw: i16,
    ) -> (Result<(), ()>, String, Vec<Msg3>) {
        let (result, list, log) = chain_nodes(chain, src, raw);
        (result, show_list(&list), log)
    }

    /// The list the passes of `chain` leave, which must succeed without a message.
    pub(super) fn grouped(chain: &str, src: &str) -> Vec<Node> {
        let (result, list, log) = chain_nodes(chain, src, 13);
        assert!(result.is_ok(), "{chain} failed on {src:?}: {log:?}");
        assert!(log.is_empty(), "{chain} logged on {src:?}: {log:?}");
        list
    }

    /// [`grouped`], rendered.
    pub(super) fn passes(chain: &str, src: &str) -> String {
        show_list(&grouped(chain, src))
    }

    /// The messages a failing chain logs.
    pub(super) fn pass_error(chain: &str, src: &str) -> Vec<Msg3> {
        let (result, _, log) = run_chain(chain, src, 13);
        assert!(result.is_err(), "{chain} accepted {src:?}");
        log
    }

    pub(super) fn ids(log: &[Msg3]) -> Vec<&'static str> {
        log.iter().map(|m| m.0).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compile::ast::{Payload, Span};

    use crate::compile::{lex, parse::test_support::*};
    use crate::diag::Severity;

    /// The tree `group_tokens` makes at `raw`, or the messages it logs.
    fn tree_at(src: &str, raw: i16) -> Result<String, Vec<Msg3>> {
        let opts = options(raw);
        let mut cx = Cx::for_test(src, &opts);
        let tokens = lex::scan(&mut cx, &lex::lower(src)).expect("the source lexes");
        match group_tokens(&mut cx, tokens.nodes, tokens.used) {
            Some(root) => {
                assert!(
                    cx.logged_diagnostics().is_empty(),
                    "{src:?}: {:?}",
                    messages(&cx)
                );
                Ok(show(&root))
            }
            None => Err(messages(&cx)),
        }
    }

    fn tree(src: &str) -> String {
        tree_at(src, 13).unwrap_or_else(|log| panic!("{src:?} did not group: {log:?}"))
    }

    fn tree_error(src: &str) -> Vec<Msg3> {
        tree_at(src, 13).expect_err("the source does not group")
    }

    /// Runs `f` on a thread with a 512 KiB stack.
    fn on_small_stack<R: Send + 'static>(f: impl FnOnce() -> R + Send + 'static) -> R {
        std::thread::Builder::new()
            .stack_size(512 * 1024)
            .spawn(f)
            .unwrap()
            .join()
            .unwrap()
    }

    fn depth_of(root: &Node) -> usize {
        let mut deepest = 0;
        let mut stack = vec![(root, 1)];
        while let Some((node, depth)) = stack.pop() {
            deepest = deepest.max(depth);
            stack.extend(node.children.iter().map(|c| (c, depth + 1)));
        }
        deepest
    }

    #[test]
    fn math_calls_are_the_math_functions_except_pi() {
        for &op in Op::all() {
            let host = matches!(op, Op::HostMath | Op::HostMathVolatile);
            assert_eq!(
                is_math_call(op),
                (op.is_math_function() && op != Op::Pi) || host,
                "{op:?}"
            );
        }
        for op in [
            Op::Add,
            Op::Div,
            Op::Mul,
            Op::Pi,
            Op::Float,
            Op::QueryFunction,
            Op::Negate,
        ] {
            assert!(!is_math_call(op), "{op:?}");
        }
        for op in [
            Op::Abs,
            Op::Atan2,
            Op::Clamp,
            Op::Random,
            Op::Trunc,
            Op::InverseLerp,
            Op::EaseInQuad,
            Op::HostMath,
            Op::HostMathVolatile,
        ] {
            assert!(is_math_call(op), "{op:?}");
        }
    }

    #[test]
    fn every_binary_level_is_left_associative() {
        assert_eq!(
            tree("v.a + v.b + v.c"),
            "(Add (Add variable.a variable.b) variable.c)"
        );
        assert_eq!(
            tree("v.a * v.b * v.c"),
            "(Mul (Mul variable.a variable.b) variable.c)"
        );
        assert_eq!(
            tree("v.a / v.b / v.c"),
            "(Div (Div variable.a variable.b) variable.c)"
        );
        assert_eq!(
            tree("v.a < v.b < v.c"),
            "(LessThan (LessThan variable.a variable.b) variable.c)"
        );
        assert_eq!(
            tree("v.a == v.b == v.c"),
            "(LogicalEqual (LogicalEqual variable.a variable.b) variable.c)"
        );
        assert_eq!(
            tree("v.a && v.b && v.c"),
            "(LogicalAnd (LogicalAnd variable.a variable.b) variable.c)"
        );
        assert_eq!(
            tree("v.a || v.b || v.c"),
            "(LogicalOr (LogicalOr variable.a variable.b) variable.c)"
        );
        assert_eq!(
            tree("v.a ?? v.b ?? v.c"),
            "(NullCoalescing (NullCoalescing variable.a variable.b) variable.c)"
        );
        assert_eq!(
            tree("v.a , v.b , v.c"),
            "(Comma (Comma variable.a variable.b) variable.c)"
        );
        assert_eq!(
            tree("v.a -> v.b -> v.c"),
            "(Pointer (Pointer variable.a variable.b) variable.c)"
        );
        assert_eq!(
            tree("v.a = v.b = 1;"),
            "(Semicolon (Semicolon (Assignment (Assignment variable.a variable.b) 1)))"
        );
    }

    #[test]
    fn division_binds_tighter_than_multiplication_which_binds_tighter_than_addition() {
        assert_eq!(
            tree("v.a * v.b / v.c"),
            "(Mul variable.a (Div variable.b variable.c))"
        );
        assert_eq!(
            tree("v.a / v.b * v.c"),
            "(Mul (Div variable.a variable.b) variable.c)"
        );
        assert_eq!(
            tree("v.a + v.b * v.c"),
            "(Add variable.a (Mul variable.b variable.c))"
        );
        assert_eq!(
            tree("v.a * v.b + v.c"),
            "(Add (Mul variable.a variable.b) variable.c)"
        );
        assert_eq!(
            tree("v.a + v.b / v.c"),
            "(Add variable.a (Div variable.b variable.c))"
        );
    }

    #[test]
    fn a_binary_minus_is_an_add_of_a_negation() {
        assert_eq!(tree("v.a - v.b"), "(Add variable.a (Negate variable.b))");
        assert_eq!(
            tree("v.a - v.b - v.c"),
            "(Add (Add variable.a (Negate variable.b)) (Negate variable.c))"
        );
        assert_eq!(
            tree("v.a - v.b * v.c"),
            "(Add variable.a (Mul (Negate variable.b) variable.c))"
        );
    }

    #[test]
    fn a_unary_minus_alone_is_a_negate() {
        assert_eq!(tree("-v.a"), "(Negate variable.a)");
        assert_eq!(
            tree("-math.abs(-1)"),
            "(Negate (Abs (LeftParenthesis (Negate 1))))"
        );
    }

    #[test]
    fn two_minus_signs_make_a_plus() {
        assert_eq!(tree("v.a - - v.b"), "(Add variable.a variable.b)");
        assert_eq!(tree("v.a + - v.b"), "(Add variable.a (Negate variable.b))");
    }

    #[test]
    fn a_leading_double_minus_leaves_a_plus_without_a_left_operand() {
        assert_eq!(
            tree_error("- - v.a"),
            [(
                "E16",
                (0, 1),
                "Error: binary Add '+' operator at end of expression\n".to_owned()
            )]
        );
    }

    #[test]
    fn a_not_binds_tighter_than_the_logical_operators() {
        assert_eq!(
            tree("!v.a && v.b"),
            "(LogicalAnd (LogicalNot variable.a) variable.b)"
        );
        assert_eq!(
            tree("!v.a == 1"),
            "(LogicalEqual (LogicalNot variable.a) 1)"
        );
    }

    #[test]
    fn comparisons_bind_looser_than_arithmetic() {
        assert_eq!(
            tree("v.a + 1 < v.b * 2"),
            "(LessThan (Add variable.a 1) (Mul variable.b 2))"
        );
    }

    #[test]
    fn the_null_coalescing_is_looser_than_the_conditional() {
        assert_eq!(
            tree("v.a ? v.b : v.c ?? v.d"),
            "(NullCoalescing (Conditional variable.a (ConditionalElse variable.b variable.c)) variable.d)"
        );
        assert_eq!(
            tree("v.x ? 1 : 2 ?? 3"),
            "(NullCoalescing (Conditional variable.x (ConditionalElse 1 2)) 3)"
        );
    }

    #[test]
    fn the_conditional_is_looser_than_the_logical_operators() {
        assert_eq!(
            tree("v.a ?? v.b ? v.c : v.d"),
            "(NullCoalescing variable.a (Conditional variable.b (ConditionalElse variable.c variable.d)))"
        );
        assert_eq!(
            tree("v.a || v.b ? 1 : 2"),
            "(Conditional (LogicalOr variable.a variable.b) (ConditionalElse 1 2))"
        );
    }

    #[test]
    fn the_coalescing_operator_cannot_sit_between_a_question_mark_and_its_colon() {
        let log = tree_error("v.a ? v.b ?? v.c : v.d");
        assert_eq!(ids(&log), ["E19"]);
        assert_eq!(log[0].1, (17, 18));
    }

    #[test]
    fn the_comma_and_the_assignment_are_the_loosest_binary_levels() {
        assert_eq!(
            tree("v.x = 1, 2;"),
            "(Semicolon (Semicolon (Assignment variable.x (Comma 1 2))))"
        );
        assert_eq!(
            tree("v.x = 1 ? 2 : 3;"),
            "(Semicolon (Semicolon (Assignment variable.x (Conditional 1 (ConditionalElse 2 3)))))"
        );
    }

    #[test]
    fn return_groups_last() {
        assert_eq!(
            tree("return v.a + 1;"),
            "(Semicolon (Semicolon (Return (Add variable.a 1))))"
        );
        assert_eq!(
            tree("return v.a;"),
            "(Semicolon (Semicolon (Return variable.a)))"
        );
    }

    #[test]
    fn the_pointer_binds_tighter_than_anything_but_the_structure_passes() {
        assert_eq!(
            tree("v.x ->v.y = 1;"),
            "(Semicolon (Semicolon (Assignment (Pointer variable.x variable.y) 1)))"
        );
        assert_eq!(
            tree("v.x -> v.y + 1"),
            "(Add (Pointer variable.x variable.y) 1)"
        );
    }

    #[test]
    fn a_member_accessor_binds_tighter_than_everything() {
        assert_eq!(tree("v.a.b.c + 1"), "(Add (.c (.b variable.a)) 1)");
        assert_eq!(tree("q.is_baby.x * 2"), "(Mul (.x query.is_baby) 2)");
    }

    #[test]
    fn a_realistic_expression_groups_by_the_pass_order() {
        assert_eq!(
            tree("v.x > 1 || v.y < 2 && v.z == 3 ?? 4"),
            "(NullCoalescing (LogicalOr (GreaterThan variable.x 1) (LogicalAnd (LessThan variable.y 2) (LogicalEqual variable.z 3))) 4)"
        );
        assert_eq!(
            tree("math.max(1, 2) + 3"),
            "(Add (Max (LeftParenthesis (Comma 1 2))) 3)"
        );
        assert_eq!(
            tree("loop(2, {v.x = v.x + 1;});"),
            "(Semicolon (Semicolon (Loop (LeftParenthesis (Comma 2 (LeftBrace (Semicolon (Semicolon (Assignment variable.x (Add variable.x 1))))))))))"
        );
    }

    #[test]
    fn sections_and_calls_are_grouped_before_the_operators() {
        assert_eq!(tree("(1 + 2) * 3"), "(Mul (LeftParenthesis (Add 1 2)) 3)");
        assert_eq!(
            tree("q.is_baby(1) + 1"),
            "(Add (query.is_baby (LeftParenthesis 1)) 1)"
        );
        assert_eq!(tree("array.a[v.i] + 1"), "(Add (Array variable.i) 1)");
        assert_eq!(tree("math.abs(1) * 2"), "(Mul (Abs (LeftParenthesis 1)) 2)");
    }

    #[test]
    fn a_constant_and_a_boolean_are_plain_leaves() {
        assert_eq!(tree("this"), "This");
        assert_eq!(tree("t"), "1");
        assert_eq!(tree("math.pi"), "Pi");
        assert_eq!(tree("math.pi + 1"), "(Add Pi 1)");
    }

    #[test]
    fn the_conditional_is_two_binary_operators_below_version_5() {
        for raw in [-1, 0, 3, 4] {
            assert_eq!(
                tree_at("v.a ? v.b : v.c ? v.d : v.e", raw).unwrap(),
                "(Conditional (Conditional variable.a (ConditionalElse variable.b variable.c)) (ConditionalElse variable.d variable.e))",
                "version {raw}"
            );
        }
        for raw in [5, 6, 13] {
            assert_eq!(
                tree_at("v.a ? v.b : v.c ? v.d : v.e", raw).unwrap(),
                "(Conditional variable.a (ConditionalElse variable.b (Conditional variable.c (ConditionalElse variable.d variable.e))))",
                "version {raw}"
            );
        }
    }

    #[test]
    fn a_plain_conditional_groups_the_same_in_every_band() {
        for raw in [-1, 0, 4, 5, 13] {
            assert_eq!(
                tree_at("v.a ? v.b : v.c", raw).unwrap(),
                "(Conditional variable.a (ConditionalElse variable.b variable.c))",
                "version {raw}"
            );
            assert_eq!(
                tree_at("v.a ? v.b", raw).unwrap(),
                "(Conditional variable.a variable.b)",
                "version {raw}"
            );
        }
    }

    #[test]
    fn below_version_6_each_comparison_is_its_own_level() {
        // The order is `<`, `==`, `>=`, `>`, `<=`, `!=`.
        assert_eq!(
            tree_at("v.a > v.b < v.c", 5).unwrap(),
            "(GreaterThan variable.a (LessThan variable.b variable.c))"
        );
        assert_eq!(
            tree_at("v.a == v.b < v.c", 5).unwrap(),
            "(LogicalEqual variable.a (LessThan variable.b variable.c))"
        );
        assert_eq!(
            tree_at("v.a != v.b == v.c", 5).unwrap(),
            "(LogicalNotEqual variable.a (LogicalEqual variable.b variable.c))"
        );
        assert_eq!(
            tree_at("v.a >= v.b <= v.c", 5).unwrap(),
            "(LessEqual (GreaterEqual variable.a variable.b) variable.c)"
        );
        assert_eq!(
            tree_at("v.a > v.b >= v.c", 5).unwrap(),
            "(GreaterThan variable.a (GreaterEqual variable.b variable.c))"
        );
    }

    #[test]
    fn from_version_6_the_relations_share_one_level_and_the_equalities_another() {
        assert_eq!(
            tree_at("v.a > v.b < v.c", 6).unwrap(),
            "(LessThan (GreaterThan variable.a variable.b) variable.c)"
        );
        assert_eq!(
            tree_at("v.a >= v.b <= v.c", 6).unwrap(),
            "(LessEqual (GreaterEqual variable.a variable.b) variable.c)"
        );
        assert_eq!(
            tree_at("v.a == v.b < v.c", 6).unwrap(),
            "(LogicalEqual variable.a (LessThan variable.b variable.c))"
        );
        assert_eq!(
            tree_at("v.a < v.b == v.c", 6).unwrap(),
            "(LogicalEqual (LessThan variable.a variable.b) variable.c)"
        );
        assert_eq!(
            tree_at("v.a != v.b == v.c", 6).unwrap(),
            "(LogicalEqual (LogicalNotEqual variable.a variable.b) variable.c)"
        );
    }

    #[test]
    fn or_binds_tighter_than_and_below_version_6() {
        for raw in [-1, 0, 4, 5] {
            assert_eq!(
                tree_at("v.a || v.b && v.c", raw).unwrap(),
                "(LogicalAnd (LogicalOr variable.a variable.b) variable.c)",
                "version {raw}"
            );
            assert_eq!(
                tree_at("v.a && v.b || v.c", raw).unwrap(),
                "(LogicalAnd variable.a (LogicalOr variable.b variable.c))",
                "version {raw}"
            );
        }
    }

    #[test]
    fn and_binds_tighter_than_or_from_version_6() {
        for raw in [6, 7, 13] {
            assert_eq!(
                tree_at("v.a || v.b && v.c", raw).unwrap(),
                "(LogicalOr variable.a (LogicalAnd variable.b variable.c))",
                "version {raw}"
            );
            assert_eq!(
                tree_at("v.a && v.b || v.c", raw).unwrap(),
                "(LogicalOr (LogicalAnd variable.a variable.b) variable.c)",
                "version {raw}"
            );
        }
    }

    #[test]
    fn the_or_level_runs_once_whichever_side_of_version_6_it_is_on() {
        // The unfolded `||` taken as a right operand would be found, and fail, if the level ran
        // twice.
        for raw in [5, 6, 13] {
            assert_eq!(
                tree_at("1 || ||", raw).unwrap(),
                "(LogicalOr 1 LogicalOr)",
                "version {raw}"
            );
        }
    }

    #[test]
    fn the_invalid_version_takes_the_oldest_bands() {
        assert_eq!(
            tree_at("v.a > v.b < v.c", -1).unwrap(),
            tree_at("v.a > v.b < v.c", 0).unwrap()
        );
        assert_eq!(
            tree_at("v.a || v.b && v.c", -1).unwrap(),
            tree_at("v.a || v.b && v.c", 0).unwrap()
        );
        assert_eq!(
            tree_at("v.a > v.b < v.c", -1).unwrap(),
            "(GreaterThan variable.a (LessThan variable.b variable.c))"
        );
    }

    #[test]
    fn a_raw_version_above_thirteen_groups_like_thirteen() {
        assert_eq!(
            tree_at("v.a || v.b && v.c", 14).unwrap(),
            tree_at("v.a || v.b && v.c", 13).unwrap()
        );
        assert_eq!(
            tree_at("v.a > v.b < v.c", i16::MAX).unwrap(),
            tree_at("v.a > v.b < v.c", 13).unwrap()
        );
    }

    #[test]
    fn the_other_levels_do_not_depend_on_the_version() {
        for raw in [-1, 0, 5, 6, 13] {
            assert_eq!(
                tree_at("v.a + v.b * v.c", raw).unwrap(),
                "(Add variable.a (Mul variable.b variable.c))",
                "version {raw}"
            );
            assert_eq!(
                tree_at("v.a ?? v.b ?? v.c", raw).unwrap(),
                "(NullCoalescing (NullCoalescing variable.a variable.b) variable.c)",
                "version {raw}"
            );
        }
    }

    #[test]
    fn a_complex_expression_must_end_with_a_semicolon() {
        let end = "Error: complex expressions (contains either '=' or ';') must end with a ';'\n"
            .to_owned();
        assert_eq!(tree_error("v.x = 1"), [("E07", (6, 7), end.clone())]);
        assert_eq!(tree_error("1;2"), [("E07", (2, 3), end.clone())]);
        assert_eq!(tree_error("{v.x = 1;}"), [("E07", (9, 10), end.clone())]);
        assert_eq!(tree_error("(1;2)")[0].0, "E07");
        assert_eq!(
            tree("v.x = 1;"),
            "(Semicolon (Semicolon (Assignment variable.x 1)))"
        );
    }

    #[test]
    fn an_expression_without_assignment_or_semicolon_needs_no_final_semicolon() {
        assert_eq!(tree("{ 1 } ;"), "(Semicolon (Semicolon (LeftBrace 1)))");
        assert_eq!(tree("v.x == 1"), "(LogicalEqual variable.x 1)");
    }

    #[test]
    fn trailing_empty_statements_are_dropped_but_a_leading_one_is_an_error() {
        assert_eq!(
            tree("v.x=1;;"),
            "(Semicolon (Semicolon (Assignment variable.x 1)))"
        );
        assert_eq!(
            tree_error(";;"),
            [(
                "E14",
                (0, 1),
                "Error: expressions can't begin with a semicolon\n".to_owned()
            )]
        );
        assert_eq!(ids(&tree_error(";")), ["E14"]);
    }

    #[test]
    fn statements_are_split_before_the_operators_are_grouped() {
        assert_eq!(
            tree("v.x = 1; v.y = 2;"),
            "(Semicolon (Semicolon (Assignment variable.x 1)) (Semicolon (Assignment variable.y 2)))"
        );
        assert_eq!(tree("break;"), "(Semicolon (Semicolon Break))");
        assert_eq!(
            tree("v.a ? break : 1;"),
            "(Semicolon (Semicolon (Conditional variable.a (ConditionalElse Break 1))))"
        );
    }

    #[test]
    fn the_failing_pointer_pass_clears_the_tree_and_the_root_check_reports_it() {
        let log = tree_error("v.x ->");
        assert_eq!(ids(&log), ["E16", "E08"]);
        assert_eq!(log[0].1, (4, 6));
        assert_eq!(
            log[0].2,
            "Error: binary Pointer '->' operator at end of expression\n"
        );
        assert_eq!(
            log[1].1,
            (0, 6),
            "an empty remainder falls back to the whole source"
        );
        assert_eq!(
            log[1].2,
            "found multiple operations without a combining operation between them:\n"
        );
        assert_eq!(tree_error("-> v.x")[0].0, "E16");
        assert_eq!(ids(&tree_error("-> v.x")), ["E16", "E08"]);
    }

    #[test]
    fn a_failing_pointer_pass_leaves_its_list_empty() {
        // `group` relies on this when it goes on after a failed `->` pass.
        for src in ["v.x ->", "-> v.x", "1 + v.x ->"] {
            let (result, list, log) = run_chain("ptr", src, 13);
            assert_eq!(result, Err(()), "{src:?}");
            assert_eq!(list, "", "{src:?}");
            assert_eq!(ids(&log), ["E16"], "{src:?}");
        }
    }

    #[test]
    fn the_other_failing_passes_stop_the_tree_at_once() {
        assert_eq!(ids(&tree_error("1 +")), ["E16"]);
        assert_eq!(ids(&tree_error("(1")), ["E10", "E12"]);
        assert_eq!(ids(&tree_error("q.is_baby()")), ["E13"]);
        assert_eq!(ids(&tree_error("math.abs")), ["E17"]);
        assert_eq!(ids(&tree_error("array.a[ ]")), ["E15"]);
        assert_eq!(ids(&tree_error(".x")), ["E09"]);
        assert_eq!(ids(&tree_error("return")), ["E20"]);
        assert_eq!(ids(&tree_error("v.x = ;")), ["E16"]);
        assert_eq!(ids(&tree_error("v.x ? ;")), ["E19"]);
        assert_eq!(ids(&tree_error("!")), ["E18"]);
    }

    #[test]
    fn every_message_of_the_passes_is_an_error() {
        let opts = options(13);
        let mut cx = Cx::for_test("1 +", &opts);
        let tokens = lex::scan(&mut cx, b"1 +").unwrap();
        assert!(group_tokens(&mut cx, tokens.nodes, tokens.used).is_none());
        assert!(
            cx.logged_diagnostics()
                .iter()
                .all(|d| d.severity() == Severity::Error)
        );
    }

    #[test]
    fn a_single_root_is_returned_as_it_is() {
        assert_eq!(tree("42"), "42");
        assert_eq!(tree("v.x"), "variable.x");
    }

    #[test]
    fn leftover_tokens_are_listed_one_per_line() {
        let log = tree_error("1 2");
        assert_eq!(log.len(), 1);
        assert_eq!(log[0].0, "E08");
        assert_eq!(log[0].1, (0, 3));
        assert_eq!(
            log[0].2,
            "found multiple operations without a combining operation between them:\n\t1.000000\n\t2.000000\n"
        );
    }

    #[test]
    fn leftover_numbers_are_printed_with_six_decimals_and_operators_by_token() {
        let log = tree_error("1.5 2 )");
        assert_eq!(
            log[0].2,
            "found multiple operations without a combining operation between them:\n\t1.500000\n\t2.000000\n\t)\n"
        );
        let log = tree_error("1 + + 2");
        assert_eq!(log[0].1, (0, 7));
        assert_eq!(
            log[0].2,
            "found multiple operations without a combining operation between them:\n\t+\n\t2.000000\n"
        );
    }

    #[test]
    fn a_leftover_group_prints_its_opening_token() {
        let log = tree_error("(1) (2)");
        assert_eq!(
            log[0].1,
            (0, 6),
            "from the first group's start to the end of the last group's contents"
        );
        assert_eq!(
            log[0].2,
            "found multiple operations without a combining operation between them:\n\t(\n\t(\n"
        );
    }

    #[test]
    fn a_leftover_array_name_and_bracket_pair_print_their_tokens() {
        let log = tree_error("array.a[]");
        assert_eq!(
            log[0].2,
            "found multiple operations without a combining operation between them:\n\tarray.\n\t[]\n"
        );
    }

    #[test]
    fn the_root_span_runs_from_the_first_leftover_to_the_end_of_the_last() {
        let opts = options(13);
        let mut cx = Cx::for_test("          ", &opts);
        let mut parent = Node::token(Op::LeftParenthesis, Payload::None, Span::new(3, 4));
        parent
            .children
            .push(Node::token(Op::Float, Payload::Float(1.0), Span::new(1, 2)));
        let list = vec![
            parent,
            Node::token(Op::Float, Payload::Float(2.0), Span::new(6, 9)),
        ];
        assert!(group_tokens(&mut cx, list, OpSet::empty()).is_none());
        assert_eq!(messages(&cx)[0].1, (1, 9));
    }

    #[test]
    fn a_pass_only_runs_when_one_of_its_operators_was_lexed() {
        let opts = options(13);
        let mut cx = Cx::for_test("1 + 2", &opts);
        let tokens = lex::scan(&mut cx, b"1 + 2").unwrap();
        assert!(group_tokens(&mut cx, tokens.nodes, OpSet::empty()).is_none());
        assert_eq!(
            messages(&cx)[0].2,
            "found multiple operations without a combining operation between them:\n\t1.000000\n\t+\n\t2.000000\n"
        );
    }

    #[test]
    fn an_operator_in_the_used_set_that_does_not_occur_is_harmless() {
        let opts = options(13);
        let mut cx = Cx::for_test("1", &opts);
        let tokens = lex::scan(&mut cx, b"1").unwrap();
        let used = tokens
            .used
            .with(Op::Add)
            .with(Op::Mul)
            .with(Op::LeftParenthesis)
            .with(Op::LogicalNot);
        let root = group_tokens(&mut cx, tokens.nodes, used).unwrap();
        assert_eq!(show(&root), "1");
        assert!(cx.logged_diagnostics().is_empty());
    }

    #[test]
    fn thousands_of_nested_parentheses_do_not_overflow_the_stack() {
        const DEPTH: usize = 4_000;
        let depth = on_small_stack(|| {
            let src = format!("{}1{}", "(".repeat(DEPTH), ")".repeat(DEPTH));
            let opts = options(13);
            let mut cx = Cx::for_test(&src, &opts);
            let tokens = lex::scan(&mut cx, &lex::lower(&src)).unwrap();
            let root = group_tokens(&mut cx, tokens.nodes, tokens.used).unwrap();
            assert!(cx.logged_diagnostics().is_empty());
            depth_of(&root)
        });
        assert_eq!(depth, DEPTH + 1);
    }

    #[test]
    fn thousands_of_nested_calls_sections_and_negations_group_on_a_small_stack() {
        let depths = on_small_stack(|| {
            let mut out = Vec::new();
            for src in [
                format!("{}1{}", "-(".repeat(2_000), ")".repeat(2_000)),
                format!("{}v.x", "!".repeat(5_000)),
                format!("{}1{}", "math.abs(".repeat(1_500), ")".repeat(1_500)),
                format!("{}1{}", "q.is_baby(".repeat(1_500), ")".repeat(1_500)),
                format!("{}1{}", "{(".repeat(1_500), ")}".repeat(1_500)),
            ] {
                let opts = options(13);
                let mut cx = Cx::for_test(&src, &opts);
                let tokens = lex::scan(&mut cx, &lex::lower(&src)).unwrap();
                let root = group_tokens(&mut cx, tokens.nodes, tokens.used).expect("groups");
                out.push(depth_of(&root));
            }
            out
        });
        assert_eq!(depths, [4_001, 5_001, 3_001, 3_001, 3_001]);
    }

    #[test]
    fn a_long_flat_chain_groups_into_a_deep_left_leaning_tree() {
        let depth = on_small_stack(|| {
            let src = format!("1{}", " + 1".repeat(5_000));
            let opts = options(13);
            let mut cx = Cx::for_test(&src, &opts);
            let tokens = lex::scan(&mut cx, &lex::lower(&src)).unwrap();
            let root = group_tokens(&mut cx, tokens.nodes, tokens.used).unwrap();
            depth_of(&root)
        });
        assert_eq!(depth, 5_001);
    }

    mod tree_shapes {
        use crate::compile::test_support::pipeline::{tree, tree_at};

        #[test]
        fn pointer() {
            assert_eq!(tree("c.x->v.y+1"), "[(Pointer c.x v.y)*1+1]");
            assert_eq!(tree("-c.x->v.y"), "[(Pointer c.x v.y)*-1+0]");
            assert_eq!(tree("(math.floor(1.5))->v.hp"), "(Pointer 1 v.hp)");
            assert_eq!(tree("1->v.b"), "(Pointer 1 v.b)");
            assert_eq!(tree("'a'->v.hp"), "(Pointer 12638153115695167422 v.hp)");
            assert_eq!(tree("this->v.hp"), "(Pointer This v.hp)");
            assert_eq!(tree("c.owner->(v.x)"), "(Pointer c.owner v.x)");
        }

        /// The statement pass enters only `loop`, `for_each`, `(` and `{`; every other pass enters
        /// every nested list.
        #[test]
        fn passes_recurse_into_nested_lists() {
            assert_eq!(
                tree("((v.a / v.b) * [v.c - (v.d < v.e)])"),
                "(Mul (Div v.a v.b) (Add v.c [(LessThan v.d v.e)*-1+0]))"
            );
            assert_eq!(
                tree("(v.x = 1;);"),
                "(Semicolon (Semicolon (Assignment v.x)))"
            );
        }

        /// The `->` pass runs before the math-call pass, so a call needs parentheses to be a `->`
        /// base.
        #[test]
        fn pointer_bases() {
            assert_eq!(tree("(v.a + 1)->v.hp"), "(Pointer [v.a*1+1] v.hp)");
            assert_eq!(tree("(math.floor(1.5))->v.hp"), "(Pointer 1 v.hp)");
        }

        #[test]
        fn division_binds_tighter_than_multiplication() {
            for version in [-1, 0, 5, 6, 13] {
                assert_eq!(
                    tree_at("v.a*v.b/v.c", version),
                    "(Mul v.a (Div v.b v.c))",
                    "version {version}"
                );
                assert_eq!(
                    tree_at("v.a/v.b*v.c", version),
                    "(Mul (Div v.a v.b) v.c)",
                    "version {version}"
                );
            }
            assert_eq!(
                tree("v.a * v.b / v.c * v.d"),
                "(Mul (Mul v.a (Div v.b v.c)) v.d)"
            );
        }

        #[test]
        fn comparison_bands() {
            for version in [-1, 0, 5] {
                assert_eq!(
                    tree_at("v.a>v.b<v.c", version),
                    "(GreaterThan v.a (LessThan v.b v.c))",
                    "version {version}"
                );
                assert_eq!(
                    tree_at("v.a!=v.b==v.c", version),
                    "(LogicalNotEqual v.a (LogicalEqual v.b v.c))",
                    "version {version}"
                );
                assert_eq!(
                    tree_at("v.a<v.b==v.c", version),
                    "(LogicalEqual (LessThan v.a v.b) v.c)",
                    "version {version}"
                );
                assert_eq!(
                    tree_at("v.a==v.b<v.c", version),
                    "(LogicalEqual v.a (LessThan v.b v.c))",
                    "version {version}"
                );
                assert_eq!(
                    tree_at("v.a<=v.b>=v.c", version),
                    "(LessEqual v.a (GreaterEqual v.b v.c))",
                    "version {version}"
                );
            }
            for version in [6, 13] {
                assert_eq!(
                    tree_at("v.a>v.b<v.c", version),
                    "(LessThan (GreaterThan v.a v.b) v.c)",
                    "version {version}"
                );
                assert_eq!(
                    tree_at("v.a!=v.b==v.c", version),
                    "(LogicalEqual (LogicalNotEqual v.a v.b) v.c)",
                    "version {version}"
                );
                assert_eq!(
                    tree_at("v.a==v.b<v.c", version),
                    "(LogicalEqual v.a (LessThan v.b v.c))",
                    "version {version}"
                );
                assert_eq!(
                    tree_at("v.a < v.b == v.c < v.d", version),
                    "(LogicalEqual (LessThan v.a v.b) (LessThan v.c v.d))",
                    "version {version}"
                );
            }
        }

        #[test]
        fn logical_bands() {
            for version in [-1, 0, 5] {
                assert_eq!(
                    tree_at("v.a||v.b&&v.c", version),
                    "(LogicalAnd (LogicalOr v.a v.b) v.c)",
                    "version {version}"
                );
                assert_eq!(
                    tree_at("v.a&&v.b||v.c", version),
                    "(LogicalAnd v.a (LogicalOr v.b v.c))",
                    "version {version}"
                );
            }
            for version in [6, 13] {
                assert_eq!(
                    tree_at("v.a||v.b&&v.c", version),
                    "(LogicalOr v.a (LogicalAnd v.b v.c))",
                    "version {version}"
                );
                assert_eq!(
                    tree_at("v.a&&v.b||v.c", version),
                    "(LogicalOr (LogicalAnd v.a v.b) v.c)",
                    "version {version}"
                );
            }
            assert_eq!(
                tree("v.a && v.b && v.c"),
                "(LogicalAnd v.a v.b v.c)",
                "nested `&&` flattens into one n-ary node"
            );
            assert_eq!(
                tree("v.a || v.b || v.c || v.d"),
                "(LogicalOr v.a v.b v.c v.d)"
            );
            assert_eq!(tree("v.a && (v.b && v.c)"), "(LogicalAnd v.a v.b v.c)");
        }

        #[test]
        fn pitfall_grouping_per_version() {
            assert_eq!(
                tree_at("v.a||v.b&&v.c", 13),
                "(LogicalOr v.a (LogicalAnd v.b v.c))"
            );
            assert_eq!(
                tree_at("v.a?v.b:v.c?v.d:v.e", 13),
                "(Conditional v.a v.b (Conditional v.c v.d v.e))"
            );
            assert_ne!(tree_at("v.a!=v.b==v.c", 13), tree_at("v.a!=v.b==v.c", 0));
        }

        #[test]
        fn null_coalescing_is_looser_than_the_conditional() {
            assert_eq!(
                tree("v.a??v.b?v.c:v.d"),
                "(NullCoalescing v.a (Conditional v.b v.c v.d))"
            );
            assert_eq!(tree("v.a ?? 0 > 1"), "(NullCoalescing v.a 0)");
        }
    }
}
