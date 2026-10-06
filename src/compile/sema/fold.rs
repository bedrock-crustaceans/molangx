//! The optimiser's post-order step: structural checks, arity bounds and constant folding.

use super::lift_child;
use super::numerical::{numerical_children, numerical_operands};
use super::sum::{NewTerms, flatten_add, new_terms, optimize_add};
use crate::catalog::{MAX_MATH_ARGS, MathImpl};
use crate::compile::{
    Cx, Failed, Pass,
    ast::{Node, Payload},
    program::{CmpOp, EqOp},
};
use crate::diag::LanguageMessage as Msg;
use crate::numeric::{self, PostOp, arith};
use crate::ops::ExpressionOp as Op;
#[cfg(feature = "stdlib")]
use crate::{
    compile::program::{Fn1, Fn2, Fn3},
    stdlib::math,
};

/// The post-order step of a node whose children are optimised.
pub(super) fn after_children(cx: &mut Cx<'_>, node: &mut Node) -> Pass {
    let op = node.op;
    if op == Op::Add {
        // The terms are counted after flattening.
        let new = new_terms(node);
        flatten_add(node);
        child_count_in_bounds(cx, node, op)?;
        return fold_sum(cx, node, new);
    }
    child_count_in_bounds(cx, node, op)?;
    if let Some(cmp) = CmpOp::of(op) {
        return fold_comparison(cx, node, cmp);
    }
    if let Some(test) = EqOp::of(op) {
        fold_equality(node, test);
        return Ok(());
    }
    match op {
        Op::RightBrace
        | Op::RightBracket
        | Op::RightParenthesis
        | Op::ConditionalElse
        | Op::ExpressionArray => unsupported(cx, node),
        Op::LeftBracket | Op::LeftParenthesis => lift_section(cx, node),
        Op::LeftBrace => lift_brace(cx, node),
        Op::ForEach => for_each_variable(cx, node),
        Op::Conditional => {
            fold_conditional(node);
            Ok(())
        }
        // `Array` has exactly one child, so only `=` has a second operand to move.
        Op::Assignment => {
            if only_second_is_float(node) {
                absorb_float_child(node, 1);
            }
            Ok(())
        }
        Op::DieRoll | Op::DieRollInt | Op::Random | Op::RandomInt | Op::HostMathVolatile => {
            numerical_children(cx, node)
        }
        Op::HostMath => fold_host_math(cx, node),
        Op::LogicalNot => fold_not(cx, node),
        Op::LogicalAnd | Op::LogicalOr => fold_logic(cx, node, op),
        Op::Mul => fold_product(cx, node),
        Op::Div => fold_quotient(cx, node),
        Op::Negate => fold_negation(cx, node),
        // The dice and the random functions matched above.
        #[cfg(feature = "stdlib")]
        _ if op.is_math_function() => fold_math(cx, node, op),
        _ => Ok(()),
    }
}

/// The children's values when there are `N` children and all are floats.
fn constants<const N: usize>(node: &Node) -> Option<[f32; N]> {
    let children: &[Node; N] = node.children.as_slice().try_into().ok()?;
    children
        .iter()
        .all(|c| c.is(Op::Float))
        .then(|| children.each_ref().map(Node::float))
}

/// Whether the node has two children and only the second is a float.
fn only_second_is_float(node: &Node) -> bool {
    matches!(
        node.children.as_slice(),
        [first, second] if !first.is(Op::Float) && second.is(Op::Float)
    )
}

/// Moves the float child at `index` into the node's value, its post-op applied.
fn absorb_float_child(node: &mut Node, index: usize) {
    let child = node.children.remove(index);
    node.value = Payload::Float(arith::mul_add(
        child.post.scale,
        child.float(),
        child.post.offset,
    ));
}

/// Absorbs the first float child of a two-child node; without one, the first string child moves
/// its hash into the value.
fn absorb_constant_child(node: &mut Node) {
    if node.children.len() != 2 {
        return;
    }
    if let Some(index) = node.children.iter().position(|c| c.is(Op::Float)) {
        absorb_float_child(node, index);
    } else if let Some(index) = node.children.iter().position(|c| c.is(Op::StringLiteral)) {
        let child = node.children.remove(index);
        node.value = child.value.clone();
    }
}

fn unsupported(cx: &mut Cx<'_>, node: &Node) -> Pass {
    cx.language(
        Msg::UnsupportedInOptimization,
        node.span,
        &[&cx.friendly(node)],
    );
    Err(Failed)
}

#[inline]
fn child_count_in_bounds(cx: &mut Cx<'_>, node: &Node, op: Op) -> Pass {
    let count = node.children.len();
    let (min, max) = (
        usize::from(op.min_children()),
        op.max_children().map(usize::from),
    );
    if count < min || max.is_some_and(|max| count > max) {
        let max = max.map_or(-1, |max| max as i64);
        cx.language(
            Msg::Malformed,
            node.full_span(),
            &[&cx.friendly(node), &count, &min, &max],
        );
        return Err(Failed);
    }
    Ok(())
}

/// `(…)` and `[…]` are replaced by their content; from version 4 a second child is an error.
fn lift_section(cx: &mut Cx<'_>, node: &mut Node) -> Pass {
    let count = node.children.len();
    if cx.version().reports_unexpected_operators() && count >= 2 {
        cx.language(
            Msg::SectionChildCount,
            node.full_span(),
            &[&cx.friendly(node), &count],
        );
        return Err(Failed);
    }
    lift_child(node, 0);
    Ok(())
}

fn lift_brace(cx: &mut Cx<'_>, node: &mut Node) -> Pass {
    if !node.children[0].is(Op::Semicolon) {
        cx.language(Msg::BraceWithoutStatements, node.full_span(), &[]);
        return Err(Failed);
    }
    lift_child(node, 0);
    Ok(())
}

fn for_each_variable(cx: &mut Cx<'_>, node: &Node) -> Pass {
    if !matches!(
        node.children[0].value,
        Payload::Entity(_) | Payload::Temp(_)
    ) {
        cx.language(Msg::ForEachVariable, node.children[0].full_span(), &[]);
        return Err(Failed);
    }
    Ok(())
}

/// `?:` of constants: the branch the condition selects, 0 for a missing `else`.
fn fold_conditional(node: &mut Node) {
    if !node.children.iter().all(|c| c.is(Op::Float)) {
        return;
    }
    let result = match node.children.as_slice() {
        [condition, then, ..] if numeric::truthy(condition.float()) => then.float(),
        [_, _, otherwise] => otherwise.float(),
        _ => 0.0,
    };
    node.set_float(result);
}

/// `+`, flattened: its new terms are checked (the others passed when the sum they extend was
/// optimised), then the terms merge.
fn fold_sum(cx: &mut Cx<'_>, node: &mut Node, new: NewTerms) -> Pass {
    let first_new = match new {
        NewTerms::All => 0,
        NewTerms::Last => node.children.len().saturating_sub(1),
    };
    numerical_operands(cx, node, &node.children[first_new..])?;
    optimize_add(node, new);
    Ok(())
}

/// `<`, `<=`, `>=` and `>`: folded when constant; else a constant second operand moves into the
/// node.
fn fold_comparison(cx: &mut Cx<'_>, node: &mut Node, cmp: CmpOp) -> Pass {
    numerical_children(cx, node)?;
    if let Some([a, b]) = constants(node) {
        node.set_float(f32::from(u8::from(cmp.holds(a, b))));
    } else if only_second_is_float(node) {
        absorb_float_child(node, 1);
    }
    Ok(())
}

/// `==` and `!=`: folded when constant; else a constant operand on either side moves into the node.
fn fold_equality(node: &mut Node, test: EqOp) {
    if let Some([a, b]) = constants(node) {
        node.set_float(f32::from(u8::from(test.holds(numeric::eq(a, b)))));
    } else {
        absorb_constant_child(node);
    }
}

fn fold_not(cx: &mut Cx<'_>, node: &mut Node) -> Pass {
    numerical_children(cx, node)?;
    if let Some([x]) = constants(node) {
        node.set_float(numeric::not(x, PostOp::IDENTITY));
    }
    Ok(())
}

/// `&&` and `||`: folded when constant; else nested nodes of the same operator are merged.
fn fold_logic(cx: &mut Cx<'_>, node: &mut Node, op: Op) -> Pass {
    numerical_children(cx, node)?;
    if let Some([a, b]) = constants(node) {
        // The result is a boolean, never an operand's value: `1 && -0` and `0 || -0` fold to +0.
        let second = if numeric::truthy(b) { 1.0 } else { 0.0 };
        let result = match (op, numeric::truthy(a)) {
            (Op::LogicalAnd, false) => 0.0,
            (Op::LogicalOr, true) => 1.0,
            _ => second,
        };
        node.set_float(result);
    } else if node.children.iter().any(|c| c.is(op)) {
        // `a && (b && c)` is one n-ary node. A nested node's own post-op is dropped.
        let old = std::mem::take(&mut node.children);
        for mut child in old {
            if child.is(op) {
                node.children.append(&mut child.children);
            } else {
                node.children.push(child);
            }
        }
    }
    Ok(())
}

/// `*`: folded when constant; else a constant factor folds into the other operand's post-op.
fn fold_product(cx: &mut Cx<'_>, node: &mut Node) -> Pass {
    numerical_children(cx, node)?;
    if let Some([a, b]) = constants(node) {
        // The left operand's NaN wins on x86-64.
        node.set_float(arith::mul(a, b));
    } else if let Some(index) = node.children.iter().position(|c| c.is(Op::Float)) {
        let constant = &node.children[index];
        let factor = arith::mul_add(constant.post.scale, constant.float(), constant.post.offset);
        let own = node.post;
        let other = lift_child(node, 1 - index);
        other.post = own.fold_scaled(factor, other.post);
    }
    Ok(())
}

/// `/`: folded when constant; else a literal divisor becomes a multiplication by its reciprocal
/// (0 below epsilon).
fn fold_quotient(cx: &mut Cx<'_>, node: &mut Node) -> Pass {
    numerical_children(cx, node)?;
    if let Some([a, b]) = constants(node) {
        node.set_float(numeric::fold_const_div(a, b));
    } else if let [_, divisor] = node.children.as_mut_slice()
        && divisor.is(Op::Float)
    {
        divisor.value = Payload::Float(numeric::fold_const_divisor(divisor.float()));
        node.op = Op::Mul;
    }
    Ok(())
}

/// Unary `-`: folded when constant; else it folds into the operand's post-op.
fn fold_negation(cx: &mut Cx<'_>, node: &mut Node) -> Pass {
    numerical_children(cx, node)?;
    if let Some([x]) = constants(node) {
        node.set_float(-x);
    } else {
        let own = node.post;
        let other = lift_child(node, 0);
        other.post = own.fold_negated(other.post);
    }
    Ok(())
}

/// The standard math functions but the dice and the random ones: folded when constant. Of the
/// two-argument ones, `min` and `max` move a constant operand on either side into the node, `pow`
/// and `mod` only a second one, and `atan2` and `copy_sign` keep both.
#[cfg(feature = "stdlib")]
fn fold_math(cx: &mut Cx<'_>, node: &mut Node, op: Op) -> Pass {
    numerical_children(cx, node)?;
    let id = PostOp::IDENTITY;
    if op == Op::Pi {
        node.set_float(math::PI);
    } else if op == Op::Mod {
        if let Some([a, b]) = constants(node) {
            node.set_float(math::fold::modulo(a, b));
        } else if only_second_is_float(node) {
            absorb_float_child(node, 1);
        }
    } else if let Some(f) = Fn2::of(op) {
        if let Some([a, b]) = constants(node) {
            node.set_float(f.apply(a, b, id));
        } else if matches!(f, Fn2::Min | Fn2::Max) {
            absorb_constant_child(node);
        } else if f == Fn2::Pow && only_second_is_float(node) {
            absorb_float_child(node, 1);
        }
    } else if let Some(f) = Fn3::of(op) {
        if let Some([a, b, c]) = constants(node) {
            node.set_float(f.apply(a, b, c, id));
        }
    } else if let Some(f) = Fn1::of(op)
        && let Some([x]) = constants(node)
    {
        node.set_float(f.apply(x, id));
    }
    Ok(())
}

/// A pure host math function: folded when every argument is constant.
fn fold_host_math(cx: &mut Cx<'_>, node: &mut Node) -> Pass {
    numerical_children(cx, node)?;
    if !node.children.iter().all(|c| c.is(Op::Float)) {
        return Ok(());
    }
    let (Payload::HostMath(function), Some(math)) = (&node.value, cx.opts.math) else {
        return Ok(());
    };
    if let MathImpl::Pure(call) = math.decl(*function).implementation() {
        let mut args = [0.0; MAX_MATH_ARGS as usize];
        for (arg, child) in args.iter_mut().zip(&node.children) {
            *arg = child.float();
        }
        let result = call(&args[..node.children.len()]);
        node.set_float(result);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compile::ast::Span;
    use crate::compile::{CompileOptions, Deviations, sema::test_support::*};
    use crate::diag::Severity;
    use crate::hash::HashedStr;
    use crate::numeric::{
        arch::{arm64, x86_64},
        test_support::per_arch,
    };
    use crate::stdlib::math;

    fn run_after(node: &mut Node) -> (Pass, Vec<Logged>) {
        let opts = client(13);
        let mut cx = Cx::for_test("abcdefghijklmnopqrstuvwxyz", &opts);
        let result = after_children(&mut cx, node);
        (result, logged(&cx))
    }

    fn host_tree(src: &str) -> Node {
        let out = optimise_with(src, &crate::compile::test_support::math_opts());
        assert!(out.ok && out.log.is_empty(), "{src}: {:?}", out.texts);
        out.root
    }

    #[test]
    fn a_pure_host_function_folds() {
        assert_float_bits(&host_tree("math.tag(1)"), 1.25);
        assert_float_bits(&host_tree("math.sum(1, 2 * 3, -0.5)"), 6.5);
        // Folded arguments fold the call; the call's post-op folds into the parent.
        assert_float_bits(&host_tree("math.twice(math.sum(1, 2)) * 2 + 1"), 13.0);
    }

    #[test]
    fn a_host_function_with_a_run_time_argument_or_a_volatile_one_stays_a_call() {
        let pure = host_tree("math.twice(v.x) * 2 + 1");
        assert!(pure.is(Op::HostMath), "{}", pure.tree_notation(9));
        assert_post_bits(&pure, 2.0, 1.0);
        assert!(host_tree("math.noise(1)").is(Op::HostMathVolatile));
        assert!(host_tree("math.sum(1, v.x)").is(Op::HostMath));
    }

    #[test]
    fn host_function_arguments_are_numerical() {
        let out = optimise_with(
            "math.twice('a')",
            &crate::compile::test_support::math_opts(),
        );
        assert!(!out.ok);
        assert_eq!(
            out.texts,
            [
                "'Host Math Function 'math.twice'' expression cannot take a 'String '''' argument. It only supports numerical arguments."
            ]
        );
        let out = optimise_with(
            "math.noise(q.is_baby)",
            &crate::compile::test_support::math_opts(),
        );
        assert!(out.ok, "{:?}", out.texts);
    }

    #[test]
    fn a_member_accessor_keeps_its_variable() {
        assert_eq!(shown("v.x.y"), "(MemberAccessor v.x)");
        assert_eq!(shown("v.x.y.z"), "(MemberAccessor (MemberAccessor v.x))");
        assert_eq!(shown("t.a.b"), "(MemberAccessor t.a)");
        assert_eq!(shown("c.a.b"), "(MemberAccessor c.a)");
    }

    #[test]
    fn a_member_accessor_after_a_pointer_or_another_member_is_accepted() {
        let pointer = parent(Op::Pointer, vec![entity("a"), entity("b")]);
        assert_eq!(
            folded(member("y", pointer)).tree_notation(9),
            "(MemberAccessor (Pointer v.a v.b))"
        );
        assert_eq!(
            folded(member("z", member("y", entity("a")))).tree_notation(9),
            "(MemberAccessor (MemberAccessor v.a))"
        );
    }

    #[test]
    fn a_comma_that_reaches_the_optimiser_is_rejected_at_the_comma() {
        let mut node = at(parent(Op::Comma, vec![float(1.0), float(2.0)]), 7, 8);
        let (ok, log, _) = optimise_node(&mut node, &client(13));
        assert!(!ok);
        assert_eq!(log, [("E26", Severity::Error, (7, 8))]);
    }

    #[test]
    fn closers_are_unsupported_in_the_optimiser() {
        for (op, friendly) in [
            (Op::RightBrace, "Right Brace '}'"),
            (Op::RightBracket, "Right Bracket ']'"),
            (Op::RightParenthesis, "Right Parenthesis ')'"),
        ] {
            let mut node = at(parent(op, vec![float(1.0)]), 6, 7);
            let (ok, log, texts) = optimise_node(&mut node, &client(13));
            assert!(!ok, "{op:?}");
            assert_eq!(log, [("E25", Severity::Error, (6, 7))], "{op:?}");
            assert_eq!(
                texts,
                [format!(
                    "Unsupported {friendly} operator in expression optimization"
                )],
                "{op:?}"
            );
        }
    }

    #[test]
    fn a_child_count_outside_the_op_bounds_is_malformed() {
        let mut node = at(parent(Op::Abs, vec![float(1.0), float(2.0)]), 1, 2);
        let (ok, log, texts) = optimise_node(&mut node, &client(13));
        assert!(!ok);
        assert_eq!(log, [("E24", Severity::Error, (0, 2))]);
        assert_eq!(
            texts,
            [
                "Malformed Absolute Value 'math.abs' expression. It has 2 children but should have between 1 and 1"
            ]
        );
    }

    #[test]
    fn an_unbounded_maximum_prints_as_minus_one() {
        let mut node = parent(Op::Add, vec![entity("x")]);
        let (ok, _, texts) = optimise_node(&mut node, &client(13));
        assert!(!ok);
        assert_eq!(
            texts,
            ["Malformed Add '+' expression. It has 1 children but should have between 2 and -1"]
        );
    }

    #[test]
    fn a_bare_leaf_of_each_op_is_checked_against_its_bounds() {
        // A childless node: the ops that need children are malformed, the calls need their `(`.
        let mut ids = std::collections::BTreeMap::new();
        for &op in Op::all() {
            let mut node = Node::token(op, Payload::None, Span::new(0, 1));
            let (ok, log, _) = optimise_node(&mut node, &client(13));
            ids.insert(format!("{op:?}"), (ok, log.first().map(|m| m.0)));
        }
        for leaf in [
            "Float",
            "Pi",
            "Break",
            "Continue",
            "This",
            "EntityVariable",
            "TempVariable",
            "ContextVariable",
            "ArrayVariable",
            "StringLiteral",
            "Geometry",
            "QueryFunction",
        ] {
            assert_eq!(ids[leaf], (true, None), "{leaf}");
        }
        for call in [
            "Min",
            "Max",
            "Atan2",
            "Clamp",
            "Lerp",
            "Random",
            "Loop",
            "ForEach",
            "EaseInQuad",
        ] {
            assert_eq!(ids[call], (false, Some("E35")), "{call}");
        }
        for malformed in [
            "Abs",
            "Add",
            "Mul",
            "Div",
            "LessThan",
            "Assignment",
            "Semicolon",
            "Return",
            "Conditional",
            "LeftBrace",
            "Array",
        ] {
            assert_eq!(ids[malformed], (false, Some("E24")), "{malformed}");
        }
        assert_eq!(ids["Comma"], (false, Some("E26")));
    }

    #[test]
    fn a_return_with_two_children_is_malformed() {
        // `Return` takes one child by the op table, so two children are #24.
        let mut node = parent(Op::Return, vec![float(1.0), float(2.0)]);
        let (ok, log, _) = optimise_node(&mut node, &client(13));
        assert!(!ok);
        assert_eq!(log[0].0, "E24");
    }

    #[test]
    fn a_parenthesis_with_two_children_depends_on_the_version() {
        // From version 4 `(a b)` is an error (E32); below it the first child is lifted and the rest
        // dropped.
        for raw in [4, 13] {
            let mut node = at(
                parent(Op::LeftParenthesis, vec![float(1.0), float(2.0)]),
                0,
                1,
            );
            let (ok, log, texts) = optimise_node(&mut node, &client(raw));
            assert!(!ok, "version {raw}");
            assert_eq!(log, [("E32", Severity::Error, (0, 1))], "version {raw}");
            assert_eq!(
                texts,
                [
                    "Error: Left Parenthesis '(' optimization expected only one child operation but found 2"
                ]
            );
        }
        for raw in [0, 3] {
            let mut node = parent(Op::LeftParenthesis, vec![float(1.0), float(2.0)]);
            let (ok, log, _) = optimise_node(&mut node, &client(raw));
            assert!(ok && log.is_empty(), "version {raw}");
            assert_float_bits(&node, 1.0);
        }
    }

    #[test]
    fn a_bracket_section_is_lifted_like_a_parenthesis() {
        let root = folded(parent(Op::LeftBracket, vec![entity("x")]));
        assert!(root.is(Op::EntityVariable));
        let mut two = parent(Op::LeftBracket, vec![float(1.0), float(2.0)]);
        let (ok, log, _) = optimise_node(&mut two, &client(4));
        assert!(!ok);
        assert_eq!(log[0].0, "E32");
    }

    #[test]
    fn a_brace_section_lifts_its_statement_list() {
        let root = folded(parent(
            Op::Semicolon,
            vec![group(vec![block(vec![entity("x")])])],
        ));
        assert_eq!(root.tree_notation(9), "(Semicolon (Semicolon v.x))");
    }

    #[test]
    fn the_new_term_of_a_long_sum_is_checked_with_the_message_of_the_whole_sum() {
        // Every `+` but the first extends an optimised sum; the term it adds is checked as the
        // first `+` checks both.
        let out = optimise_with("v.a + v.b + v.c + 'x'", &client(13));
        assert!(!out.ok);
        assert_eq!(out.log, [("E36", Severity::Error, (18, 21))]);
        assert_eq!(
            out.texts,
            [
                "'Add '+'' expression cannot take a 'String '''' argument. It only supports numerical arguments."
            ]
        );
        let first = optimise_with("v.a + 'x'", &client(13));
        assert_eq!(first.log, [("E36", Severity::Error, (6, 9))]);
        assert!(
            optimise_with("v.a + v.b + v.c + 'x'", &client(2)).ok,
            "below version 3 a string is a number"
        );
    }

    #[test]
    fn the_query_lint_fires_from_the_optimiser_when_the_deviation_is_on() {
        // `query.is_name_any` is registered with at least one argument;
        // `query.ride_body_x_rotation` with none.
        let out = optimise_with("q.is_name_any", &client(13));
        assert!(out.ok);
        assert_eq!(out.log, [("-", Severity::Warning, (0, 13))]);
        assert_eq!(
            out.texts,
            [
                "query.is_name_any is registered with at least 1 argument, 0 given (this crate's check)"
            ]
        );
        let out = optimise_with("q.ride_body_x_rotation(1)", &client(13));
        assert!(out.ok);
        assert_eq!(out.log, [("-", Severity::Warning, (0, 24))]);
        assert_eq!(
            out.texts,
            [
                "query.ride_body_x_rotation is registered with 0 arguments, 1 given (this crate's check)"
            ]
        );
        for src in ["q.is_name_any", "q.ride_body_x_rotation(1)"] {
            let no_deviations = optimise_with(
                src,
                &CompileOptions {
                    deviations: Deviations::NONE,
                    ..client(13)
                },
            );
            assert!(no_deviations.ok && no_deviations.log.is_empty(), "{src}");
        }
    }

    #[test]
    fn a_query_within_its_declared_arity_logs_nothing() {
        assert!(
            optimise_with("q.is_name_any('a')", &client(13))
                .log
                .is_empty()
        );
        assert!(
            optimise_with("q.ride_body_x_rotation", &client(13))
                .log
                .is_empty()
        );
        assert!(
            optimise_with("q.is_baby(1,2,3)", &client(13))
                .log
                .is_empty()
        );
    }

    #[test]
    fn after_children_folds_a_node_whose_children_are_done() {
        let mut node = parent(Op::Add, vec![float(1.0), float(2.0)]);
        assert!(run_after(&mut node).0.is_ok());
        assert_float_bits(&node, 3.0);
        let mut product = parent(Op::Mul, vec![entity("x"), float(4.0)]);
        assert!(run_after(&mut product).0.is_ok());
        assert_eq!(product.tree_notation(9), "[v.x*4+0]");
    }

    #[test]
    fn after_children_flattens_nested_sums_before_counting_the_children() {
        // A one-child sum is malformed on its own, but `Add[Add[x, y]]` flattens to two terms
        // first.
        let mut node = parent(
            Op::Add,
            vec![parent(Op::Add, vec![entity("x"), entity("y")])],
        );
        assert!(run_after(&mut node).0.is_ok());
        assert_eq!(node.tree_notation(9), "(Add v.x v.y)");
    }

    #[test]
    fn after_children_replaces_a_section_by_its_content() {
        let mut node = parent(Op::LeftParenthesis, vec![entity("x")]);
        assert!(run_after(&mut node).0.is_ok());
        assert!(node.is(Op::EntityVariable));
        let mut brace = parent(Op::LeftBrace, vec![entity("x")]);
        let (result, log) = run_after(&mut brace);
        assert!(result.is_err());
        assert_eq!(log[0].0, "E34");
    }

    #[test]
    fn after_children_rejects_a_return_with_two_children_by_the_bounds() {
        let mut node = parent(Op::Return, vec![float(1.0)]);
        assert!(run_after(&mut node).0.is_ok());
        let mut two = parent(Op::Return, vec![float(1.0), float(2.0)]);
        let (result, log) = run_after(&mut two);
        assert!(result.is_err());
        assert_eq!(log.iter().map(|m| m.0).collect::<Vec<_>>(), ["E24"]);
    }

    #[test]
    fn a_math_call_without_children_is_malformed_before_anything_else() {
        // Every math call but `math.pi` needs a child (the op table's bounds).
        for op in [Op::Abs, Op::Max, Op::Clamp, Op::Random] {
            let mut node = parent(op, vec![]);
            let (result, log) = run_after(&mut node);
            assert!(result.is_err(), "{op:?}");
            assert_eq!(
                log.iter().map(|m| m.0).collect::<Vec<_>>(),
                ["E24"],
                "{op:?}"
            );
        }
        let mut pi = parent(Op::Pi, vec![]);
        assert!(run_after(&mut pi).0.is_ok());
        assert_float_bits(&pi, std::f32::consts::PI);
    }

    #[test]
    fn after_children_folds_the_conditional_by_its_condition_only_when_all_children_are_constants()
    {
        let mut constant = parent(Op::Conditional, vec![float(0.0), float(1.0), float(2.0)]);
        assert!(run_after(&mut constant).0.is_ok());
        assert_float_bits(&constant, 2.0);
        let mut variable = parent(Op::Conditional, vec![entity("c"), float(1.0), float(2.0)]);
        assert!(run_after(&mut variable).0.is_ok());
        assert_eq!(variable.children.len(), 3);
        let mut branch_variable =
            parent(Op::Conditional, vec![float(1.0), entity("a"), float(2.0)]);
        assert!(run_after(&mut branch_variable).0.is_ok());
        assert_eq!(
            branch_variable.children.len(),
            3,
            "a constant condition with a variable branch is not folded"
        );
    }

    #[test]
    fn literal_arithmetic_folds_to_one_float() {
        for (src, want) in [
            ("1+2", "3"),
            ("2*3", "6"),
            ("7/2", "3.5"),
            ("10-4", "6"),
            ("1+2*3", "7"),
            ("2*(3+4)", "14"),
            ("-3", "-3"),
            ("-(2+3)", "-5"),
            ("2*-3", "-6"),
            ("0.5+0.25", "0.75"),
        ] {
            assert_eq!(shown(src), want, "{src}");
        }
    }

    /// An all-constant `a + b` or `a * b` with two NaNs keeps the left operand's on x86-64, as at
    /// run time.
    #[test]
    fn literal_arithmetic_with_two_nans_keeps_the_left_one() {
        let (neg, pos) = (f32::from_bits(0xffc0_0000), f32::from_bits(0x7fc0_0000));
        for op in [Op::Add, Op::Mul] {
            assert_float_bits(&folded(parent(op, vec![float(neg), float(pos)])), neg);
            assert_float_bits(&folded(parent(op, vec![float(pos), float(neg)])), pos);
            assert_float_bits(&folded(parent(op, vec![float(2.0), float(pos)])), pos);
        }
        assert_float_bits(
            &folded(parent(Op::Add, vec![float(1.5), float(2.25)])),
            3.75,
        );
        assert_float_bits(
            &folded(parent(Op::Mul, vec![float(1.5), float(-2.0)])),
            -3.0,
        );
    }

    #[test]
    fn a_folded_constant_is_a_childless_float_with_the_identity_post_op() {
        let root = tree("1+2*3");
        assert_float_bits(&root, 7.0);
        assert!(root.children.is_empty());
        assert_post_bits(&root, 1.0, 0.0);
    }

    #[test]
    fn pi_becomes_the_float_constant() {
        let root = tree("math.pi");
        assert_float_bits(&root, math::PI);
        assert_eq!(shown("math.pi"), "3.14159274");
        assert_eq!(shown("math.pi*2"), "6.28318548");
    }

    #[test]
    fn unary_math_functions_fold_with_the_numeric_function_of_the_architecture() {
        let id = PostOp::IDENTITY;
        for (op, x) in [
            (Op::Abs, -3.5),
            (Op::Acos, 0.5),
            (Op::Asin, 0.5),
            (Op::Atan, 1.0),
            (Op::Ceil, 1.2),
            (Op::Cos, 60.0),
            (Op::Exp, 1.0),
            (Op::Floor, -1.2),
            (Op::HermiteBlend, 0.3),
            (Op::Ln, 2.0),
            (Op::MinAngle, 270.0),
            (Op::Round, 2.5),
            (Op::Sin, 30.0),
            (Op::Sign, -2.0),
            (Op::Sqrt, 2.0),
            (Op::Trunc, -1.7),
            (Op::LogicalNot, 3.0),
        ] {
            let want = match op {
                Op::Abs => math::abs(x, id),
                Op::Acos => math::acos(x, id),
                Op::Asin => math::asin(x, id),
                Op::Atan => math::atan(x, id),
                Op::Ceil => math::ceil(x, id),
                Op::Cos => math::cos(x, id),
                Op::Exp => math::exp(x, id),
                Op::Floor => math::floor(x, id),
                Op::HermiteBlend => math::hermite_blend(x, id),
                Op::Ln => math::ln(x, id),
                Op::MinAngle => math::min_angle(x, id),
                Op::Round => math::round(x, id),
                Op::Sin => math::sin(x, id),
                Op::Sign => math::sign(x, id),
                Op::Sqrt => math::sqrt(x, id),
                Op::Trunc => math::trunc(x, id),
                _ => numeric::not(x, id),
            };
            let root = folded(parent(op, vec![float(x)]));
            assert_float_bits(&root, want);
        }
    }

    #[test]
    fn unary_math_functions_fold_special_operands_with_the_numeric_function_of_the_architecture() {
        // The architectures differ on a NaN operand (`asin`, `acos`) and on the infinities; each op
        // must use the architecture's.
        let id = PostOp::IDENTITY;
        for x in [
            f32::NAN,
            f32::INFINITY,
            f32::NEG_INFINITY,
            -0.0,
            0.0,
            1.5,
            -1.5,
        ] {
            for (op, want) in [
                (Op::Abs, math::abs(x, id)),
                (Op::Acos, math::acos(x, id)),
                (Op::Asin, math::asin(x, id)),
                (Op::Atan, math::atan(x, id)),
                (Op::Ceil, math::ceil(x, id)),
                (Op::Cos, math::cos(x, id)),
                (Op::Exp, math::exp(x, id)),
                (Op::Floor, math::floor(x, id)),
                (Op::HermiteBlend, math::hermite_blend(x, id)),
                (Op::Ln, math::ln(x, id)),
                (Op::MinAngle, math::min_angle(x, id)),
                (Op::Round, math::round(x, id)),
                (Op::Sin, math::sin(x, id)),
                (Op::Sign, math::sign(x, id)),
                (Op::Sqrt, math::sqrt(x, id)),
                (Op::Trunc, math::trunc(x, id)),
            ] {
                let root = folded(parent(op, vec![float(x)]));
                assert!(root.is(Op::Float), "{op:?}({x}) at ");
                assert_eq!(root.float().to_bits(), want.to_bits(), "{op:?}({x}) at ");
            }
        }
    }

    #[test]
    fn unary_math_functions_fold_to_their_known_values() {
        for (src, want) in [
            ("math.abs(-3)", "3"),
            ("math.sqrt(16)", "4"),
            ("math.floor(-1.2)", "-2"),
            ("math.ceil(1.2)", "2"),
            ("math.trunc(-1.7)", "-1"),
            ("math.sign(-2)", "-1"),
            ("math.sin(90)", "1"),
            ("math.cos(180)", "-1"),
            ("math.min_angle(270)", "-90"),
            ("!0", "1"),
            ("!5", "0"),
            ("!-0", "1"),
        ] {
            assert_eq!(shown(src), want, "{src}");
        }
    }

    #[test]
    fn a_folded_call_keeps_the_post_op_of_the_node_it_replaces() {
        // `Float` v = |-3| with the call node's own (2, 1): the instruction later loads `v` alone.
        let root = folded(with_post(parent(Op::Abs, vec![float(-3.0)]), 2.0, 1.0));
        assert_float_bits(&root, 3.0);
        assert_post_bits(&root, 2.0, 1.0);
    }

    #[test]
    fn binary_and_ternary_math_functions_fold_to_their_known_values() {
        for (src, want) in [
            ("math.atan2(1,1)", "45"),
            ("math.copy_sign(3,-1)", "-3"),
            ("math.min(1,2)", "1"),
            ("math.max(1,2)", "2"),
            ("math.pow(2,10)", "1024"),
            ("math.mod(7,3)", "1"),
            ("math.clamp(5,0,3)", "3"),
            ("math.clamp(-1,0,3)", "0"),
            ("math.lerp(0,10,0.5)", "5"),
            ("math.inverse_lerp(0,10,5)", "0.5"),
            ("math.ease_in_quad(0,10,0.5)", "2.5"),
        ] {
            assert_eq!(shown(src), want, "{src}");
        }
    }

    #[test]
    fn binary_math_functions_fold_with_the_numeric_function_of_the_architecture() {
        let id = PostOp::IDENTITY;
        let opts = client(13);
        for (src, want) in [
            ("math.atan2(0.3,2.5)", math::atan2(0.3, 2.5, id)),
            ("math.copy_sign(2.5,-0.5)", math::copy_sign(2.5, -0.5, id)),
            ("math.min(2.5,-0.5)", math::min(2.5, -0.5, id)),
            ("math.max(2.5,-0.5)", math::max(2.5, -0.5, id)),
            ("math.pow(2.5,3.5)", math::pow(2.5, 3.5, id)),
            ("math.clamp(2.5,0,1)", math::clamp(2.5, 0.0, 1.0, id)),
        ] {
            let out = optimise_with(src, &opts);
            assert!(out.ok && out.log.is_empty(), "{src}");
            assert_float_bits(&out.root, want);
        }
    }

    #[test]
    fn min_and_max_with_a_nan_operand_follow_the_architecture() {
        // x86-64: `(a > b) ? a : b`, so `max(1, NaN)` is NaN; arm64 ignores the NaN.
        let folded = folded(call(Op::Max, vec![float(1.0), float(f32::NAN)]));
        assert!(folded.is(Op::Float));
        assert_eq!(folded.float().is_nan(), per_arch(true, false));
    }

    #[test]
    fn an_all_constant_mod_is_the_plain_remainder_without_the_post_op() {
        // `math.mod(-4, 2)` folds to -0 (the truncated remainder); the run-time post-op would give
        // +0.
        assert_float_bits(&tree("math.mod(-4,2)"), -0.0);
        assert_eq!(shown("math.mod(-4,2)"), "-0");
        assert_float_bits(&tree("math.mod(7,3)"), 1.0);
        assert_float_bits(&tree("math.mod(-7,3)"), -1.0);
    }

    #[test]
    fn comparisons_of_constants_fold_to_one_or_zero() {
        for (src, want) in [
            ("3<5", "1"),
            ("5<3", "0"),
            ("3<3", "0"),
            ("3<=3", "1"),
            ("4<=3", "0"),
            ("3>=4", "0"),
            ("4>=4", "1"),
            ("4>3", "1"),
            ("3>3", "0"),
            ("3==3", "1"),
            ("3==4", "0"),
            ("3!=3", "0"),
            ("3!=4", "1"),
            ("0==-0", "1"),
        ] {
            assert_eq!(shown(src), want, "{src}");
        }
    }

    #[test]
    fn comparisons_with_a_nan_operand_follow_the_architecture() {
        // x86-64: every comparison with a NaN is false. arm64: `<` and `<=` are true when an
        // operand is NaN while `>` and `>=` are not.
        for (op, x86, arm) in [
            (Op::LessThan, 0.0, 1.0),
            (Op::LessEqual, 0.0, 1.0),
            (Op::GreaterThan, 0.0, 0.0),
            (Op::GreaterEqual, 0.0, 0.0),
            (Op::LogicalEqual, 0.0, 0.0),
            (Op::LogicalNotEqual, 1.0, 1.0),
        ] {
            for (a, b) in [(f32::NAN, 1.0), (1.0, f32::NAN)] {
                assert_float_bits(
                    &folded(parent(op, vec![float(a), float(b)])),
                    per_arch(x86, arm),
                );
            }
        }
    }

    #[test]
    fn equality_of_constants_is_exact() {
        assert_float_bits(
            &folded(parent(Op::LogicalEqual, vec![float(0.0), float(-0.0)])),
            1.0,
        );
        assert_float_bits(
            &folded(parent(
                Op::LogicalEqual,
                vec![float(0.1), float(0.100_000_01)],
            )),
            0.0,
        );
        assert_float_bits(
            &folded(parent(
                Op::LogicalNotEqual,
                vec![float(0.1), float(0.100_000_01)],
            )),
            1.0,
        );
    }

    #[test]
    fn logic_of_constants_folds_to_a_boolean() {
        for (src, want) in [
            ("0&&5", 0.0),
            ("3&&5", 1.0),
            ("3&&0", 0.0),
            ("0||5", 1.0),
            ("0||0", 0.0),
            ("5||0", 1.0),
            ("2||0", 1.0),
            ("0&&0", 0.0),
        ] {
            assert_float_bits(&tree(src), want);
        }
    }

    #[test]
    fn logic_of_constants_never_gives_negative_zero() {
        // The result is a boolean, never an operand's value: `1 && -0` is +0.
        for src in ["1&&-0", "0||-0", "-0&&1", "-0||-0", "1&&(0*-1)"] {
            assert_float_bits(&tree(src), 0.0);
        }
    }

    #[test]
    fn a_nan_constant_is_true_in_logic() {
        assert_float_bits(
            &folded(parent(Op::LogicalAnd, vec![float(f32::NAN), float(1.0)])),
            1.0,
        );
        assert_float_bits(
            &folded(parent(Op::LogicalOr, vec![float(f32::NAN), float(0.0)])),
            1.0,
        );
        assert_float_bits(&folded(parent(Op::LogicalNot, vec![float(f32::NAN)])), 0.0);
    }

    #[test]
    fn nested_logic_of_the_same_operator_flattens_into_one_node() {
        assert_eq!(shown("v.a&&(v.b&&v.c)"), "(LogicalAnd v.a v.b v.c)");
        assert_eq!(shown("(v.a&&v.b)&&v.c"), "(LogicalAnd v.a v.b v.c)");
        assert_eq!(shown("v.a||(v.b||v.c)"), "(LogicalOr v.a v.b v.c)");
        assert_eq!(shown("(v.a||v.b)||v.c||v.d"), "(LogicalOr v.a v.b v.c v.d)");
    }

    #[test]
    fn logic_of_different_operators_is_not_flattened() {
        assert_eq!(
            shown("v.a&&(v.b||v.c)"),
            "(LogicalAnd v.a (LogicalOr v.b v.c))"
        );
        assert_eq!(
            shown("v.a||(v.b&&v.c)"),
            "(LogicalOr v.a (LogicalAnd v.b v.c))"
        );
    }

    #[test]
    fn flattening_logic_drops_the_post_op_of_the_nested_node() {
        // `(a && b) * 2` carries (2, 0); merged into the outer `&&` it is lost.
        assert_eq!(shown("(v.a&&v.b)*2&&v.c"), "(LogicalAnd v.a v.b v.c)");
        let inner = with_post(
            parent(Op::LogicalAnd, vec![entity("a"), entity("b")]),
            2.0,
            1.0,
        );
        let root = folded(parent(Op::LogicalAnd, vec![inner, entity("c")]));
        assert_eq!(root.tree_notation(9), "(LogicalAnd v.a v.b v.c)");
        assert_post_bits(&root, 1.0, 0.0);
    }

    #[test]
    fn flattening_logic_keeps_the_post_op_of_the_outer_node() {
        let inner = parent(Op::LogicalAnd, vec![entity("a"), entity("b")]);
        let root = folded(with_post(
            parent(Op::LogicalAnd, vec![inner, entity("c")]),
            2.0,
            1.0,
        ));
        assert_eq!(root.tree_notation(9), "[(LogicalAnd v.a v.b v.c)*2+1]");
    }

    #[test]
    fn a_constant_condition_selects_a_branch() {
        for (src, want) in [
            ("1?5:7", 5.0),
            ("0?5:7", 7.0),
            ("-0?5:7", 7.0),
            ("2?5:7", 5.0),
            ("0?5", 0.0),
            ("1?5", 5.0),
        ] {
            assert_float_bits(&tree(src), want);
        }
    }

    #[test]
    fn a_nan_condition_is_true() {
        let root = folded(parent(
            Op::Conditional,
            vec![float(f32::NAN), float(5.0), float(7.0)],
        ));
        assert_float_bits(&root, 5.0);
    }

    #[test]
    fn a_conditional_with_a_variable_condition_keeps_its_branches() {
        assert_eq!(shown("v.x?1:2"), "(Conditional v.x 1 2)");
        assert_eq!(shown("v.x?1"), "(Conditional v.x 1)");
        assert_eq!(
            shown("v.x?(v.y+v.z):(v.y-v.z)"),
            "(Conditional v.x (Add v.y v.z) (Add v.y [v.z*-1+0]))"
        );
        assert_eq!(
            shown("v.x?(v.y?1:2):3"),
            "(Conditional v.x (Conditional v.y 1 2) 3)"
        );
    }

    #[test]
    fn the_else_branch_is_lifted_out_of_the_colon_node() {
        let root = folded(parent(
            Op::Conditional,
            vec![
                entity("c"),
                parent(Op::ConditionalElse, vec![entity("a"), entity("b")]),
            ],
        ));
        assert_eq!(root.tree_notation(9), "(Conditional v.c v.a v.b)");
    }

    #[test]
    fn a_colon_without_its_branches_is_unsupported() {
        let mut root = parent(
            Op::Conditional,
            vec![
                entity("c"),
                at(parent(Op::ConditionalElse, vec![entity("a")]), 3, 4),
            ],
        );
        let (ok, log, texts) = optimise_node(&mut root, &client(13));
        assert!(!ok);
        assert_eq!(log, [("E25", Severity::Error, (3, 4))]);
        assert_eq!(
            texts,
            ["Unsupported Conditional Else ':' operator in expression optimization"]
        );
    }

    #[test]
    fn a_conditional_that_starts_with_a_colon_has_no_condition() {
        let mut root = at(
            parent(
                Op::Conditional,
                vec![
                    parent(Op::ConditionalElse, vec![entity("a"), entity("b")]),
                    entity("c"),
                ],
            ),
            2,
            3,
        );
        let (ok, log, texts) = optimise_node(&mut root, &client(13));
        assert!(!ok);
        assert_eq!(log, [("E29", Severity::Error, (2, 3))]);
        assert_eq!(
            texts,
            ["Error: '?' operator couldn't find a valid preceding 'if' expression"]
        );
    }

    #[test]
    fn a_float_with_a_post_op_is_worth_its_bare_value_as_a_branch() {
        // `(v.x + v.y + 1) + (-v.x - v.y)` cancels to the Float 1 carrying (1, +1).
        let cancelled = "((v.x+v.y+1)+(-v.x-v.y))";
        let sum = tree(cancelled);
        assert_float_bits(&sum, 1.0);
        assert_post_bits(&sum, 1.0, 1.0);
        // As a branch that stays a child it keeps its post-op in the tree …
        assert_eq!(
            shown(&format!("v.a?{cancelled}:7")),
            "(Conditional v.a [1*1+1] 7)"
        );
        assert_eq!(
            shown(&format!("v.a?7:{cancelled}")),
            "(Conditional v.a 7 [1*1+1])"
        );
        // … and when the condition is constant the branch is read as its bare value, not S·v + O.
        let selected = tree(&format!("1?{cancelled}:7"));
        assert_float_bits(&selected, 1.0);
        assert_post_bits(&selected, 1.0, 0.0);
        // The condition is read as its raw value, whatever its post-op.
        assert_float_bits(&tree(&format!("{cancelled}?5:7")), 5.0);
    }

    #[test]
    fn the_post_op_of_a_constant_condition_does_not_change_its_truth() {
        let condition = with_post(float(0.0), 2.0, 3.0);
        let root = folded(parent(
            Op::Conditional,
            vec![condition, float(5.0), float(7.0)],
        ));
        assert_float_bits(&root, 7.0);
    }

    #[test]
    fn dividing_constants_folds_and_a_tiny_divisor_gives_zero() {
        assert_float_bits(&tree("7/2"), 3.5);
        assert_float_bits(&tree("1/0"), 0.0);
        assert_float_bits(&tree("0/0"), 0.0);
        assert_float_bits(&tree("1/1e-10"), 0.0);
        assert_float_bits(&tree("-1/1e-10"), 0.0);
        assert_float_bits(&tree("-7/2"), -3.5);
        assert_float_bits(&tree("7/-2"), -3.5);
    }

    #[test]
    fn a_literal_divisor_becomes_a_multiplication_by_its_reciprocal() {
        let root = tree("v.x/2");
        assert!(root.is(Op::Mul));
        assert_eq!(root.children.len(), 2);
        assert!(root.children[0].is(Op::EntityVariable));
        assert_float_bits(&root.children[1], 0.5);
        assert_eq!(shown("v.x/4"), "(Mul v.x 0.25)");
        assert_eq!(shown("v.x/0.5/4"), "(Mul (Mul v.x 2) 0.25)");
    }

    #[test]
    fn a_literal_divisor_keeps_its_sign() {
        assert_float_bits(&tree("v.x/-2").children[1], -0.5);
        assert_float_bits(&tree("v.x/-4").children[1], -0.25);
    }

    #[test]
    fn a_literal_divisor_below_epsilon_multiplies_by_positive_zero() {
        for src in ["v.x/0", "v.x/1e-10", "v.x/-1e-10", "v.x/-0"] {
            let root = tree(src);
            assert!(root.is(Op::Mul), "{src}");
            assert_float_bits(&root.children[1], 0.0);
        }
    }

    #[test]
    fn the_epsilon_rule_is_at_f32_epsilon_exactly() {
        let below = f32::from_bits(f32::EPSILON.to_bits() - 1);
        let at_epsilon = folded(parent(Op::Div, vec![entity("x"), float(f32::EPSILON)]));
        assert_float_bits(&at_epsilon.children[1], 1.0 / f32::EPSILON);
        let under = folded(parent(Op::Div, vec![entity("x"), float(below)]));
        assert_float_bits(&under.children[1], 0.0);
        let negative = folded(parent(Op::Div, vec![entity("x"), float(-f32::EPSILON)]));
        assert_float_bits(&negative.children[1], -1.0 / f32::EPSILON);
        let negative_under = folded(parent(Op::Div, vec![entity("x"), float(-below)]));
        assert_float_bits(&negative_under.children[1], 0.0);
        // The all-constant division has the same threshold.
        assert_float_bits(
            &folded(parent(Op::Div, vec![float(1.0), float(f32::EPSILON)])),
            1.0 / f32::EPSILON,
        );
        assert_float_bits(
            &folded(parent(Op::Div, vec![float(1.0), float(below)])),
            0.0,
        );
    }

    #[test]
    fn a_nan_divisor_is_zero() {
        let multiplied = folded(parent(Op::Div, vec![entity("x"), float(f32::NAN)]));
        assert!(multiplied.is(Op::Mul));
        assert_float_bits(&multiplied.children[1], 0.0);
        assert_float_bits(
            &folded(parent(Op::Div, vec![float(1.0), float(f32::NAN)])),
            0.0,
        );
    }

    #[test]
    fn a_divisor_that_is_not_a_literal_is_left_alone() {
        assert_eq!(shown("v.x/v.y"), "(Div v.x v.y)");
        assert_eq!(shown("2/v.y"), "(Div 2 v.y)");
        assert_eq!(shown("(v.x+1)/(v.y+1)"), "(Div [v.x*1+1] [v.y*1+1])");
    }

    #[test]
    fn a_constant_factor_folds_into_the_post_op_of_the_other_operand() {
        for (src, want) in [
            ("v.x*2", "[v.x*2+0]"),
            ("2*v.x", "[v.x*2+0]"),
            ("v.x*2+1", "[v.x*2+1]"),
            ("v.x*-2", "[v.x*-2+0]"),
            ("v.x*0", "[v.x*0+0]"),
            ("v.x*2*3", "[v.x*6+0]"),
            ("2*3*v.x", "[v.x*6+0]"),
            ("(v.x+1)*2", "[v.x*2+2]"),
            ("2*(v.x+1)", "[v.x*2+2]"),
            ("(v.x*2+1)*3", "[v.x*6+3]"),
        ] {
            assert_eq!(shown(src), want, "{src}");
        }
    }

    #[test]
    fn multiplying_two_variables_keeps_the_node() {
        assert_eq!(shown("v.x*v.y"), "(Mul v.x v.y)");
        assert_eq!(shown("(v.x*2)*(v.y*3)"), "(Mul [v.x*2+0] [v.y*3+0])");
    }

    #[test]
    fn an_identity_fold_leaves_the_bare_operand() {
        assert_eq!(shown("v.x*1"), "v.x");
        assert_eq!(shown("v.x+0"), "v.x");
        assert_eq!(shown("0+v.x"), "v.x");
        assert_eq!(shown("v.x-0"), "v.x");
    }

    #[test]
    fn negation_folds_into_the_post_op() {
        let root = tree("-v.x");
        assert_eq!(root.tree_notation(9), "[v.x*-1+0]");
        assert_post_bits(&root, -1.0, 0.0);
        assert_eq!(shown("-(v.x*2+1)"), "[v.x*-2+-1]");
        assert_eq!(shown("-v.x*2"), "[v.x*-2+0]");
        assert_eq!(shown("-(-v.x)"), "v.x");
        assert_eq!(shown("-(v.x+v.y)"), "[(Add v.x v.y)*-1+0]");
        assert_eq!(shown("-(v.x+v.y+3)"), "[(Add v.x v.y)*-1+-3]");
    }

    #[test]
    fn subtracting_a_constant_adds_its_negation_to_the_offset() {
        let root = tree("v.x-3");
        assert_eq!(root.tree_notation(9), "[v.x*1+-3]");
        assert_post_bits(&root, 1.0, -3.0);
    }

    #[test]
    fn the_negation_of_a_constant_is_a_constant() {
        assert_float_bits(&folded(parent(Op::Negate, vec![float(0.0)])), -0.0);
        assert_float_bits(&folded(parent(Op::Negate, vec![float(2.0)])), -2.0);
        assert_float_bits(
            &folded(parent(Op::Negate, vec![with_post(float(2.0), 3.0, 1.0)])),
            -2.0,
        );
    }

    #[test]
    fn negation_folds_through_an_existing_post_op() {
        // -(S, O) -> (-S, -O) for the identity negation node.
        let root = folded(parent(Op::Negate, vec![with_post(entity("x"), 2.0, 3.0)]));
        assert_post_bits(&root, -2.0, -3.0);
        // The own post-op of the negation node is applied around it: (-(S·cs), O − co·S).
        let own = folded(with_post(
            parent(Op::Negate, vec![with_post(entity("x"), 2.0, 3.0)]),
            5.0,
            7.0,
        ));
        assert_post_bits(&own, -10.0, 7.0 - 3.0 * 5.0);
    }

    #[test]
    fn the_post_op_of_a_constant_factor_counts_in_the_fold() {
        // factor = S·v + O = 3·2 + 1 = 7, folded into the other operand's (2, 3): (14, 21).
        let factor = with_post(float(2.0), 3.0, 1.0);
        let root = folded(parent(
            Op::Mul,
            vec![factor, with_post(entity("x"), 2.0, 3.0)],
        ));
        assert_post_bits(&root, 14.0, 21.0);
        assert!(root.is(Op::EntityVariable));
    }

    #[test]
    fn the_own_post_op_of_a_multiplication_is_folded_in() {
        // own (2, 1), constant 4 on a term with (3, 5): scale (2·4)·3 = 24, offset (4·5)·2 + 1
        // = 41.
        let own = with_post(
            parent(Op::Mul, vec![float(4.0), with_post(entity("x"), 3.0, 5.0)]),
            2.0,
            1.0,
        );
        let root = folded(own);
        assert_post_bits(&root, 24.0, 41.0);
    }

    #[test]
    fn the_multiplication_offset_rounds_as_the_architecture_does() {
        // c·o + O: two roundings on x86-64, one on arm64. 0.3·3.3 + 0.3 differs by one ulp.
        let (c, o, offset) = (0.3f32, 3.3f32, 0.3f32);
        assert_ne!(
            x86_64::mul_add(c, o, offset).to_bits(),
            arm64::mul_add(c, o, offset).to_bits()
        );
        let node = with_post(
            parent(Op::Mul, vec![float(c), with_post(entity("x"), 1.0, o)]),
            1.0,
            offset,
        );
        let root = folded(node);
        assert_post_bits(&root, c, arith::mul_add(c, o, offset));
    }

    #[test]
    fn the_constant_factor_of_a_multiplication_rounds_as_the_architecture_does() {
        // The constant's own post-op (scale 0.3, offset 0.3) on the value 3.3: two roundings on
        // x86-64, one on arm64.
        let (scale, value, offset) = (0.3f32, 3.3f32, 0.3f32);
        assert_ne!(
            x86_64::mul_add(scale, value, offset).to_bits(),
            arm64::mul_add(scale, value, offset).to_bits()
        );
        let node = parent(
            Op::Mul,
            vec![with_post(float(value), scale, offset), entity("x")],
        );
        let root = folded(node);
        assert_post_bits(&root, arith::mul_add(scale, value, offset), 0.0);
    }

    #[test]
    fn all_constant_multiplication_ignores_the_operands_post_ops() {
        // Folding reads the raw values of constant children (their post-ops are not applied).
        let root = folded(parent(
            Op::Mul,
            vec![with_post(float(2.0), 3.0, 1.0), float(5.0)],
        ));
        assert_float_bits(&root, 10.0);
        let sum = folded(parent(
            Op::Add,
            vec![with_post(float(2.0), 3.0, 1.0), float(5.0)],
        ));
        assert_float_bits(&sum, 7.0);
    }

    /// The folded constant of `src`, a `Float` with the identity post-op.
    fn constant(src: &str) -> f32 {
        let root = tree(src);
        assert_post_bits(&root, 1.0, 0.0);
        assert!(root.is(Op::Float), "{}", root.tree_notation(9));
        root.float()
    }

    #[test]
    fn a_constant_product_folds_to_its_value_before_a_term_or_a_negation() {
        let product = arith::mul(numeric::fold_const_div(1.0, 3.0), 3.0);
        assert_eq!(constant("(1/3)*3").to_bits(), product.to_bits());
        let less_one = arith::add(product, -1.0).to_bits();
        assert_eq!(constant("(1/3)*3-1").to_bits(), less_one);
        assert_eq!(constant("-1+(1/3)*3").to_bits(), less_one);
        assert_eq!(constant("-((1/3)*3)").to_bits(), (-product).to_bits());
        assert_eq!(constant("-((0)*(1))").to_bits(), (-0.0f32).to_bits());
        assert_eq!(constant("((-1)*(0))+0").to_bits(), 0.0f32.to_bits());
    }

    #[test]
    fn sign_of_a_constant_folds_to_its_value_before_a_factor_or_a_term() {
        assert_eq!(constant("math.sign(-1)"), -1.0);
        assert_eq!(constant("math.sign(-1)*2+1"), -1.0);
        assert_eq!(constant("1-math.sign(-1)*2"), 3.0);
        assert_eq!(constant("-math.sign(-1)"), 1.0);
    }

    #[test]
    fn a_folded_constant_operand_moves_into_a_run_time_parent() {
        assert_eq!(shown("v.x+(1/3)*3"), "[v.x*1+1]");
        assert_eq!(shown("v.x*((-1)*2)"), "[v.x*-2+0]");
        assert_eq!(shown("v.x<(-1)*2+1"), "(LessThan v.x)");
        assert_eq!(shown("v.x*math.sign(-1)"), "[v.x*-1+0]");
        assert_eq!(shown("v.x<math.sign(-1)*2"), "(LessThan v.x)");
    }

    #[test]
    fn absorb_float_child_removes_the_indexed_child_and_applies_its_post_op() {
        let mut node = parent(Op::Min, vec![with_post(float(5.0), 2.0, 1.0), entity("x")]);
        absorb_float_child(&mut node, 0);
        assert_eq!(node.children.len(), 1);
        assert!(node.children[0].is(Op::EntityVariable));
        assert_eq!(node.value, Payload::Float(11.0));
    }

    #[test]
    fn absorb_constant_child_prefers_the_float() {
        let mut node = parent(Op::LogicalEqual, vec![string("a"), float(3.0)]);
        absorb_constant_child(&mut node);
        assert_eq!(node.value, Payload::Float(3.0));
        assert_eq!(node.children.len(), 1);
        assert!(node.children[0].is(Op::StringLiteral));
    }

    #[test]
    fn absorb_constant_child_moves_the_first_string_when_there_is_no_float() {
        let mut node = parent(Op::LogicalEqual, vec![entity("x"), string("a")]);
        absorb_constant_child(&mut node);
        assert_eq!(node.value, Payload::Hash(HashedStr::new("a").as_u64()));
        assert_eq!(node.children.len(), 1);
        assert!(node.children[0].is(Op::EntityVariable));
    }

    #[test]
    fn absorb_constant_child_leaves_other_shapes_alone() {
        let mut no_constant = parent(Op::LogicalEqual, vec![entity("x"), entity("y")]);
        absorb_constant_child(&mut no_constant);
        assert_eq!(no_constant.children.len(), 2);
        assert_eq!(no_constant.value, Payload::None);

        let mut three = parent(
            Op::LogicalEqual,
            vec![string("a"), entity("y"), entity("z")],
        );
        absorb_constant_child(&mut three);
        assert_eq!(three.children.len(), 3);
        assert_eq!(three.value, Payload::None);
    }

    fn moved_value(node: &Node) -> f32 {
        assert_eq!(node.children.len(), 1, "{}", node.tree_notation(9));
        node.float()
    }

    #[test]
    fn the_second_operand_of_a_comparison_moves_into_the_value() {
        for (src, op) in [
            ("v.x<5", Op::LessThan),
            ("v.x<=5", Op::LessEqual),
            ("v.x>=5", Op::GreaterEqual),
            ("v.x>5", Op::GreaterThan),
        ] {
            let root = tree(src);
            assert!(root.is(op), "{src}");
            assert_eq!(root.value, Payload::Float(5.0), "{src}");
            assert_eq!(moved_value(&root), 5.0);
            assert!(root.children[0].is(Op::EntityVariable));
        }
    }

    #[test]
    fn a_constant_first_operand_of_an_ordered_comparison_does_not_move() {
        for src in ["5<v.x", "5<=v.x", "5>=v.x", "5>v.x"] {
            let root = tree(src);
            assert_eq!(root.children.len(), 2, "{src}");
            assert_eq!(root.value, Payload::None, "{src}");
        }
        assert_eq!(shown("5<v.x"), "(LessThan 5 v.x)");
    }

    #[test]
    fn a_comparison_of_two_variables_keeps_both_children() {
        assert_eq!(shown("v.x<v.y"), "(LessThan v.x v.y)");
    }

    #[test]
    fn power_and_modulus_move_only_a_second_operand() {
        for (src, op) in [("math.pow(v.x,2)", Op::Pow), ("math.mod(v.x,3)", Op::Mod)] {
            let root = tree(src);
            assert!(root.is(op), "{src}");
            assert_eq!(moved_value(&root), if op == Op::Pow { 2.0 } else { 3.0 });
        }
        assert_eq!(shown("math.pow(2,v.x)"), "(Pow 2 v.x)");
        assert_eq!(shown("math.mod(3,v.x)"), "(Mod 3 v.x)");
    }

    #[test]
    fn equality_min_and_max_move_a_constant_from_either_side() {
        for (src, op, want) in [
            ("v.x==5", Op::LogicalEqual, 5.0),
            ("5==v.x", Op::LogicalEqual, 5.0),
            ("v.x!=5", Op::LogicalNotEqual, 5.0),
            ("5!=v.x", Op::LogicalNotEqual, 5.0),
            ("math.min(v.x,3)", Op::Min, 3.0),
            ("math.min(3,v.x)", Op::Min, 3.0),
            ("math.max(v.x,3)", Op::Max, 3.0),
            ("math.max(3,v.x)", Op::Max, 3.0),
        ] {
            let root = tree(src);
            assert!(root.is(op), "{src}");
            assert_eq!(moved_value(&root), want, "{src}");
            assert!(root.children[0].is(Op::EntityVariable), "{src}");
        }
    }

    #[test]
    fn equality_with_a_string_moves_its_hash() {
        let b = HashedStr::new("b").as_u64();
        let root = tree("v.x=='b'");
        assert_eq!(root.value, Payload::Hash(b));
        assert_eq!(root.children.len(), 1);
        let root = tree("'b'!=v.x");
        assert!(root.is(Op::LogicalNotEqual));
        assert_eq!(root.value, Payload::Hash(b));
    }

    #[test]
    fn equality_of_two_strings_moves_the_first_and_keeps_the_second() {
        let (a, b) = (HashedStr::new("a").as_u64(), HashedStr::new("b").as_u64());
        let root = tree("'a'=='b'");
        assert!(root.is(Op::LogicalEqual));
        assert_eq!(root.value, Payload::Hash(a));
        assert_eq!(root.children.len(), 1);
        assert_eq!(root.children[0].value, Payload::Hash(b));
    }

    #[test]
    fn equality_of_two_variables_keeps_both_children() {
        assert_eq!(shown("v.x==v.y"), "(LogicalEqual v.x v.y)");
    }

    #[test]
    fn an_assigned_constant_moves_into_the_assignment() {
        let root = tree("v.x=4;");
        assert!(root.is(Op::Semicolon));
        let assignment = &root.children[0];
        assert!(assignment.is(Op::Assignment));
        assert_eq!(assignment.value, Payload::Float(4.0));
        assert_eq!(assignment.children.len(), 1);
        assert_eq!(shown("v.x=v.y;"), "(Semicolon (Assignment v.x v.y))");
    }

    #[test]
    fn a_moved_constant_is_the_value_the_post_op_gives() {
        // S·v + O, rounded as the architecture does: the post-op of the moved node is gone from the
        // tree.
        let (scale, value, offset) = (0.1f32, 0.3f32, 0.9f32);
        assert_ne!(
            x86_64::mul_add(scale, value, offset).to_bits(),
            arm64::mul_add(scale, value, offset).to_bits()
        );
        let node = parent(
            Op::LessThan,
            vec![entity("x"), with_post(float(value), scale, offset)],
        );
        let root = folded(node);
        assert_eq!(
            moved_value(&root).to_bits(),
            arith::mul_add(scale, value, offset).to_bits()
        );
    }

    mod tree_shapes {
        use crate::compile::test_support::pipeline::{tree, tree_at};

        #[test]
        fn division() {
            assert_eq!(tree("v.x/0"), "(Mul v.x 0)");
            assert_eq!(tree("v.x / 2"), "(Mul v.x 0.5)");
            assert_eq!(tree("v.x / 3"), "(Mul v.x 0.333333343)");
            assert_eq!(tree("v.x / 0.00000012"), "(Mul v.x 8333333.5)");
            assert_eq!(
                tree_at("v.x / -2", 6),
                "(Mul v.x -0.5)",
                "a literal divisor keeps its sign below version 7 too"
            );
            assert_eq!(
                tree("2 / v.x"),
                "(Div 2 v.x)",
                "a non-literal divisor stays a division"
            );
            assert_eq!(tree("v.x * 2 / 2"), "v.x");
            assert_eq!(tree("v.x / 2 * 2"), "[(Mul v.x 0.5)*2+0]");
        }

        #[test]
        fn post_op_folding() {
            assert_eq!(tree("v.x + 1"), "[v.x*1+1]");
            assert_eq!(tree("1 + v.x"), "[v.x*1+1]");
            assert_eq!(tree("v.x - 1"), "[v.x*1+-1]");
            assert_eq!(tree("1 - v.x"), "[v.x*-1+1]");
            assert_eq!(tree("-v.x"), "[v.x*-1+0]");
            assert_eq!(tree("-(-v.x)"), "v.x");
            assert_eq!(tree("v.x * 2 + 1"), "[v.x*2+1]");
            assert_eq!(tree("(v.x + 1) * 2 + 3"), "[v.x*2+5]");
            assert_eq!(tree("3 - 2 * v.x"), "[v.x*-2+3]");
            assert_eq!(tree("v.x * 1"), "v.x");
            assert_eq!(tree("v.x + 0"), "v.x");
            assert_eq!(
                tree("v.x * 0"),
                "[v.x*0+0]",
                "a multiplication by zero is not removed"
            );
            assert_eq!(tree("math.abs(v.x) * 2 + 1"), "[(Abs v.x)*2+1]");
            assert_eq!(tree("(v.x < 1) * 2 + 1"), "[(LessThan v.x)*2+1]");
            assert_eq!(tree("(v.x ? v.y : 2) * 3"), "[(Conditional v.x v.y 2)*3+0]");
            assert_eq!(tree("this * 2"), "[This*2+0]");
            assert_eq!(tree("v.x * v.y * 2"), "[(Mul v.x v.y)*2+0]");
            assert_eq!(tree("2 * v.x * v.y"), "(Mul [v.x*2+0] v.y)");
            assert_eq!(tree("v.x + 1 + v.y + 2"), "[(Add [v.x*1+1] v.y)*1+2]");
            assert_eq!(
                tree("(v.x + v.y) * 2 + 1"),
                "[(Add [v.x*2+0] [v.y*2+0])*1+1]",
                "a scaled sum distributes over its terms"
            );
            assert_eq!(tree("v.z - (v.x + 1)"), "(Add v.z [v.x*-1+-1])");
        }

        #[test]
        fn interpolation_functions_fold() {
            assert_eq!(tree("math.min_angle(query.life_time)"), "(MinAngle 40)");
            assert_eq!(
                tree("math.ease_in_quad(0, 10, query.life_time)"),
                "(EaseInQuad 0 10 40)"
            );
        }

        #[test]
        fn constants_move_into_the_node() {
            assert_eq!(tree("v.x < 1"), "(LessThan v.x)");
            assert_eq!(
                tree("1 < v.x"),
                "(LessThan 1 v.x)",
                "only the right side of a relational comparison moves"
            );
            assert_eq!(tree("v.x == 1"), "(LogicalEqual v.x)");
            assert_eq!(
                tree("1 == v.x"),
                "(LogicalEqual v.x)",
                "either side of an equality moves"
            );
            assert_eq!(tree("v.x == 'abc'"), "(LogicalEqual v.x)");
            assert_eq!(tree("'abc' != v.s"), "(LogicalNotEqual v.s)");
            assert_eq!(tree("math.max(2, v.x)"), "(Max v.x)");
            assert_eq!(tree("math.mod(v.x, 2)"), "(Mod v.x)");
            assert_eq!(tree("math.mod(2, v.x)"), "(Mod 2 v.x)");
            assert_eq!(tree("math.pow(v.x, 2)"), "(Pow v.x)");
            assert_eq!(
                tree("math.atan2(v.x, 2)"),
                "(Atan2 v.x 2)",
                "atan2 keeps both operands"
            );
            assert_eq!(tree("v.x = 1;"), "(Semicolon (Assignment v.x))");
            assert_eq!(
                tree("v.x = 'abc';"),
                "(Semicolon (Assignment v.x 15626587013303479755))",
                "a string stays a child of `=`"
            );
            assert_eq!(tree("array.foo[0]"), "(Array 0)");
        }
    }
}
