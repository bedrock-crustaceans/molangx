//! The optimiser's pre-order step: the shape of arguments, statements and assignments.

use super::{is_three_argument_math, lift_child};
use crate::compile::{
    Cx, Failed, Pass,
    ast::{Node, Payload},
};
use crate::diag::{DiagCode, LanguageMessage as Msg, Severity};
use crate::ops::ExpressionOp as Op;

/// Whether the node is a leaf: the walk skips both steps for it and does not descend.
pub(super) fn is_leaf(node: &Node) -> bool {
    matches!(
        node.op,
        Op::ArrayVariable
            | Op::ContextVariable
            | Op::EntityVariable
            | Op::TempVariable
            | Op::StringLiteral
            | Op::GeometryVariable
            | Op::MaterialVariable
            | Op::TextureVariable
            | Op::Float
            | Op::Geometry
            | Op::Material
            | Op::Texture
            | Op::Break
            | Op::Continue
            | Op::This
    )
}

/// The pre-order step of a node that is not a leaf.
pub(super) fn before_children(cx: &mut Cx<'_>, node: &mut Node) -> Pass {
    match node.op {
        op if is_three_argument_math(op) => call_with_arguments(cx, node, 3, Msg::ParameterCount3),
        Op::Atan2
        | Op::CopySign
        | Op::Max
        | Op::Min
        | Op::Mod
        | Op::Pow
        | Op::Random
        | Op::RandomInt => call_with_arguments(cx, node, 2, Msg::ParameterCount2),
        Op::QueryFunction => query_arguments(cx, node),
        Op::HostMath | Op::HostMathVolatile => host_math_arguments(cx, node),
        Op::Comma => {
            cx.language(Msg::UnexpectedComma, node.span, &[&cx.friendly(node)]);
            Err(Failed)
        }
        Op::ForEach => for_each_parameters(cx, node),
        Op::Semicolon => statements(cx, node),
        Op::Conditional => conditional_branches(cx, node),
        Op::Loop => loop_parameters(cx, node),
        _ => Ok(()),
    }
}

fn call_with_arguments(cx: &mut Cx<'_>, node: &mut Node, expected: usize, message: Msg) -> Pass {
    call_parameters(cx, node)?;
    if node.children.len() != expected {
        cx.language(
            message,
            node.full_span(),
            &[&cx.friendly(node), &node.children.len()],
        );
        return Err(Failed);
    }
    Ok(())
}

fn query_arguments(cx: &mut Cx<'_>, node: &mut Node) -> Pass {
    if !node.is_leaf() {
        call_parameters(cx, node)?;
    }
    if let Payload::Query(query) = &node.value {
        cx.lint_query_arity(query.index, node);
    }
    Ok(())
}

/// A host math function takes the argument counts it was declared with.
fn host_math_arguments(cx: &mut Cx<'_>, node: &mut Node) -> Pass {
    call_parameters(cx, node)?;
    let (Payload::HostMath(function), Some(math)) = (&node.value, cx.opts.math) else {
        return Ok(());
    };
    let decl = math.decl(*function);
    let given = node.children.len();
    if decl.args().contains(given) {
        return Ok(());
    }
    let message = format!(
        "{} takes {}, {given} given",
        decl.name(),
        decl.args().arguments()
    );
    cx.lint(
        DiagCode::StatementForm,
        Severity::Error,
        node.full_span(),
        message,
    );
    Err(Failed)
}

/// `for_each` takes three parameters, the first a variable unless the third is a block.
fn for_each_parameters(cx: &mut Cx<'_>, node: &mut Node) -> Pass {
    call_parameters(cx, node)?;
    let ok = matches!(
        node.children.as_slice(),
        [variable, _, body]
            if matches!(variable.op, Op::EntityVariable | Op::TempVariable)
                || body.is(Op::LeftBrace)
    );
    if !ok {
        cx.language(Msg::ForEachParameters, node.full_span(), &[]);
        return Err(Failed);
    }
    Ok(())
}

/// A conditional cannot start with `:`; its `:` child gives it the two branches.
fn conditional_branches(cx: &mut Cx<'_>, node: &mut Node) -> Pass {
    if node
        .children
        .first()
        .is_some_and(|c| c.is(Op::ConditionalElse))
    {
        cx.language(Msg::ConditionalWithoutIf, node.span, &[]);
        return Err(Failed);
    }
    if node
        .children
        .get(1)
        .is_some_and(|c| c.is(Op::ConditionalElse))
    {
        // `cond ? (then : else)` becomes `[cond, then, else]`.
        let mut otherwise = node.children.remove(1);
        if otherwise.children.len() < 2 {
            // A `:` without both branches is reported as an unsupported token.
            cx.language(
                Msg::UnsupportedInOptimization,
                otherwise.span,
                &[&cx.friendly(&otherwise)],
            );
            return Err(Failed);
        }
        node.children
            .extend(std::mem::take(&mut otherwise.children).into_iter().take(2));
    }
    Ok(())
}

fn loop_parameters(cx: &mut Cx<'_>, node: &mut Node) -> Pass {
    call_parameters(cx, node)?;
    if !matches!(node.children.as_slice(), [_, body] if body.is(Op::LeftBrace)) {
        cx.language(Msg::LoopParameters, node.full_span(), &[]);
        return Err(Failed);
    }
    Ok(())
}

/// Each statement's `Semicolon` node must hold one node, which replaces it; an assignment statement must target
/// a variable, a member, or a variable / member behind `->`.
fn statements(cx: &mut Cx<'_>, node: &mut Node) -> Pass {
    for group in &mut node.children {
        if group.children.len() != 1 {
            cx.language(Msg::StatementNotSingle, group.full_span(), &[]);
            return Err(Failed);
        }
        let statement = lift_child(group, 0);
        if statement.is(Op::Assignment)
            && let Some(target) = statement.children.first()
            && !is_assignable(target)
        {
            cx.language(
                Msg::AssignToNonVariable,
                target.full_span(),
                &[&cx.friendly(target)],
            );
            return Err(Failed);
        }
    }
    Ok(())
}

fn is_assignable(target: &Node) -> bool {
    match target.op {
        Op::EntityVariable | Op::TempVariable | Op::MemberAccessor => true,
        Op::Pointer => matches!(
            target.children.get(1).map(|member| member.op),
            Some(Op::EntityVariable | Op::MemberAccessor)
        ),
        _ => false,
    }
}

/// Replaces a call's single `(` child by its arguments, flattening the left-nested `,` chain.
fn call_parameters(cx: &mut Cx<'_>, node: &mut Node) -> Pass {
    if node.children.len() != 1 {
        cx.language(Msg::ParametersNotOneChild, node.span, &[&cx.friendly(node)]);
        return Err(Failed);
    }
    if node.children[0].is_leaf() {
        cx.language(Msg::ParametersEmpty, node.span, &[&cx.friendly(node)]);
        return Err(Failed);
    }
    // Only the first child of the section is looked at.
    let first = node.children.swap_remove(0).children.swap_remove(0);

    // The left-nested `Comma(Comma(a, b), c)` chain gives the arguments in source order; a `,`
    // without operands anywhere in it is an error.
    let mut pending = vec![first];
    while let Some(mut child) = pending.pop() {
        if !child.is(Op::Comma) {
            node.children.push(child);
            continue;
        }
        if child.is_leaf() {
            cx.language(
                Msg::ParametersDanglingComma,
                child.span,
                &[&cx.friendly(node)],
            );
            return Err(Failed);
        }
        // A `,` has two operands; the left one is visited first.
        pending.extend(
            std::mem::take(&mut child.children)
                .into_iter()
                .take(2)
                .rev(),
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compile::{
        ast::{Name, Span},
        sema::test_support::*,
    };

    fn params_cx<R>(f: impl FnOnce(&mut Cx<'_>) -> R) -> (R, Vec<Logged>, Vec<String>) {
        let opts = client(13);
        let mut cx = Cx::for_test("abcdefghijklmnopqrstuvwxyz", &opts);
        let result = f(&mut cx);
        (result, logged(&cx), texts(&cx))
    }

    /// The pre-order step of `node`, which must not be a leaf.
    fn run_before(node: &mut Node) -> (Pass, Vec<Logged>) {
        assert!(!is_leaf(node), "{:?} is a leaf", node.op);
        let (result, log, _) = params_cx(|cx| before_children(cx, node));
        (result, log)
    }

    fn host_rejected(src: &str) -> (Vec<Logged>, Vec<String>) {
        let out = optimise_with(src, &crate::compile::test_support::math_opts());
        assert!(!out.ok, "{src:?} was accepted");
        (out.log, out.texts)
    }

    #[test]
    fn a_host_math_function_takes_its_declared_argument_counts() {
        for (src, text, span) in [
            (
                "math.twice(1, 2)",
                "math.twice takes 1 argument, 2 given",
                (0, 15),
            ),
            (
                "math.sum(1, 2, 3, 4)",
                "math.sum takes 1 to 3 arguments, 4 given",
                (0, 19),
            ),
        ] {
            let (log, texts) = host_rejected(src);
            assert_eq!(log, [("-", Severity::Error, span)], "{src}");
            assert_eq!(texts, [text], "{src}");
        }
        for src in [
            "math.sum(1)",
            "math.sum(1, 2, 3)",
            "math.twice(v.x)",
            "math.noise(1)",
        ] {
            let out = optimise_with(src, &crate::compile::test_support::math_opts());
            assert!(out.ok && out.log.is_empty(), "{src}: {:?}", out.texts);
        }
        // No argument at all is the standard functions' empty-parameter error.
        assert_eq!(
            host_rejected("math.twice()")
                .0
                .iter()
                .map(|m| m.0)
                .collect::<Vec<_>>(),
            ["E35"]
        );
    }

    #[test]
    fn a_two_argument_math_function_checks_its_argument_count() {
        for (src, span, found) in [
            ("math.min(1)", (0, 10), 1),
            ("math.min(1,2,3)", (0, 14), 3),
            ("math.pow(1)", (0, 10), 1),
            ("math.atan2(1,2,3,4)", (0, 18), 4),
        ] {
            assert_eq!(rejected(src), ("E23", span), "{src}");
            assert!(
                rejected_text(src).ends_with(&format!("function - expected 2, found {found}.\n")),
                "{src}"
            );
        }
        assert_eq!(
            rejected_text("math.min(1)"),
            "Unexpected number of parameters to Min 'math.min' function - expected 2, found 1.\n"
        );
    }

    #[test]
    fn a_three_argument_math_function_checks_its_argument_count() {
        for (src, span, found) in [
            ("math.clamp(1,2)", (0, 14), 2),
            ("math.clamp(1,2,3,4)", (0, 18), 4),
            ("math.lerp(1,2)", (0, 13), 2),
            ("math.ease_in_quad(1)", (0, 19), 1),
        ] {
            assert_eq!(rejected(src), ("E23", span), "{src}");
            assert!(
                rejected_text(src).ends_with(&format!("function - expected 3, found {found}.\n")),
                "{src}"
            );
        }
        assert_eq!(
            rejected_text("math.clamp(1,2)"),
            "Unexpected number of parameters to Clamp 'math.clamp' function - expected 3, found 2.\n"
        );
    }

    #[test]
    fn a_comma_in_an_ordinary_group_is_unexpected() {
        assert_eq!(rejected("math.abs(1,2)"), ("E26", (10, 11)));
        assert_eq!(rejected("(1,2)"), ("E26", (2, 3)));
        assert_eq!(rejected("v.x=(1,2);"), ("E26", (6, 7)));
        assert_eq!(rejected("1+(2,3)"), ("E26", (4, 5)));
        assert_eq!(
            rejected_text("(1,2)"),
            "Error: Unexpected Comma ',' operator not inside an arguments list for a query, loop, or math function"
        );
    }

    #[test]
    fn an_empty_section_is_malformed() {
        assert_eq!(rejected("math.abs()"), ("E24", (8, 9)));
        assert_eq!(
            rejected_text("math.abs()"),
            "Malformed Left Parenthesis '(' expression. It has 0 children but should have between 1 and -1"
        );
        assert_eq!(rejected("{}"), ("E24", (0, 1)));
        assert_eq!(
            rejected_text("{}"),
            "Malformed Left Brace '{' expression. It has 0 children but should have between 1 and -1"
        );
        assert_eq!(rejected("loop(3,{});"), ("E24", (7, 8)));
    }

    #[test]
    fn a_dangling_comma_in_an_argument_list_is_an_error() {
        assert_eq!(rejected("math.min(1,,2)"), ("E35", (11, 12)));
        assert_eq!(
            rejected_text("math.min(1,,2)"),
            "Error while optimizing parameters for Min 'math.min' operation: comma found without a following expression."
        );
    }

    #[test]
    fn loop_needs_a_count_and_a_block() {
        for (src, span) in [
            ("loop(3);", (0, 6)),
            ("loop(3,1);", (0, 8)),
            ("loop(1,2,{v.x=1;});", (0, 16)),
            ("loop(3,v.x);", (0, 10)),
        ] {
            assert_eq!(rejected(src), ("E30", span), "{src}");
        }
        assert_eq!(
            rejected_text("loop(3,1);"),
            "Error: loop requires two parameters - an expression resulting in a number of times to loop, and a {}-delimited expression to loop."
        );
        assert_eq!(
            shown("loop(3,{v.x=1;});"),
            "(Semicolon (Loop 3 (Semicolon (Assignment v.x))))"
        );
    }

    #[test]
    fn for_each_needs_three_parameters() {
        for (src, span) in [
            ("for_each(t.a,v.l);", (0, 16)),
            ("for_each(t.a,v.l,{v.x=1;},4);", (0, 27)),
            ("for_each(1,v.l,1);", (0, 16)),
        ] {
            assert_eq!(rejected(src), ("E27", span), "{src}");
        }
    }

    #[test]
    fn for_each_takes_an_entity_or_temp_variable_or_a_block() {
        // With a block as the third parameter, a non-variable first one is E33 instead of E27.
        assert_eq!(
            shown("for_each(t.a,v.l,{v.x=1;});"),
            "(Semicolon (ForEach t.a v.l (Semicolon (Assignment v.x))))"
        );
        assert_eq!(
            shown("for_each(v.a,v.l,{v.x=1;});"),
            "(Semicolon (ForEach v.a v.l (Semicolon (Assignment v.x))))"
        );
        assert_eq!(rejected("for_each(1,v.l,{v.x=1;});"), ("E33", (9, 10)));
        assert_eq!(
            rejected_text("for_each(1,v.l,{v.x=1;});"),
            "Error: for_each expressions require either a temp or entity variable as the iteration variable (the first parameter)"
        );
    }

    #[test]
    fn for_each_accepts_a_body_that_is_not_a_block() {
        assert_eq!(
            shown("for_each(t.a,v.l,1);"),
            "(Semicolon (ForEach t.a v.l 1))"
        );
        assert_eq!(
            shown("for_each(v.l,v.l,1);"),
            "(Semicolon (ForEach v.l v.l 1))"
        );
    }

    #[test]
    fn a_brace_section_needs_semicolon_delimited_statements() {
        assert_eq!(rejected("{1}"), ("E34", (0, 2)));
        assert_eq!(
            rejected_text("{1}"),
            "Brace sections must only contain semicolon-delimited expressions, even if only one expression is contained.\n"
        );
        assert_eq!(
            shown("{v.x=1;};"),
            "(Semicolon (Semicolon (Assignment v.x)))"
        );
    }

    #[test]
    fn an_assignment_target_must_be_a_variable_or_a_member() {
        assert_eq!(shown("v.x=1;"), "(Semicolon (Assignment v.x))");
        assert_eq!(shown("t.x=1;"), "(Semicolon (Assignment t.x))");
        assert_eq!(
            shown("v.x.y=2;"),
            "(Semicolon (Assignment (MemberAccessor v.x)))"
        );
        assert_eq!(
            shown("v.x->v.y=1;"),
            "(Semicolon (Assignment (Pointer v.x v.y)))"
        );
        assert!(shown("q.get_name->v.y=2;").starts_with("(Semicolon (Assignment (Pointer"));
    }

    #[test]
    fn assigning_to_a_non_variable_names_the_target() {
        for (src, span, target) in [
            ("1=2;", (0, 1), "Float"),
            ("math.abs(v.x)=2;", (0, 12), "Absolute Value 'math.abs'"),
            ("(v.x+1)=2;", (0, 6), "Left Parenthesis '('"),
            ("c.x=2;", (0, 3), "Context Variable 'context.' or 'c.'"),
            ("q.is_baby=2;", (0, 9), "Query Function 'query.' or 'q.'"),
            ("v.x->t.y=2;", (0, 8), "Pointer '->'"),
        ] {
            assert_eq!(rejected(src), ("E28", span), "{src}");
            assert_eq!(
                rejected_text(src),
                format!(
                    "Error: assignment to non-variable not allowed. Expression is trying to assign to a: {target}"
                ),
                "{src}"
            );
        }
    }

    #[test]
    fn a_pointer_target_must_end_in_a_variable_or_member() {
        let pointer = at(parent(Op::Pointer, vec![entity("a"), float(1.0)]), 4, 5);
        let mut root = statements_root(vec![parent(Op::Assignment, vec![pointer, float(1.0)])]);
        let (ok, log, texts) = optimise_node(&mut root, &client(13));
        assert!(!ok);
        assert_eq!(log, [("E28", Severity::Error, (0, 5))]);
        assert_eq!(
            texts,
            [
                "Error: assignment to non-variable not allowed. Expression is trying to assign to a: Pointer '->'"
            ]
        );
        let ok_pointer = parent(Op::Pointer, vec![entity("a"), member("m", entity("b"))]);
        let mut fine = statements_root(vec![parent(Op::Assignment, vec![ok_pointer, entity("v")])]);
        let (ok, log, _) = optimise_node(&mut fine, &client(13));
        assert!(ok && log.is_empty());
    }

    #[test]
    fn a_statement_group_must_reduce_to_one_node() {
        let mut root = parent(
            Op::Semicolon,
            vec![at(group(vec![float(1.0), float(2.0)]), 5, 6)],
        );
        let (ok, log, texts) = optimise_node(&mut root, &client(13));
        assert!(!ok);
        assert_eq!(log, [("E31", Severity::Error, (0, 6))]);
        assert_eq!(
            texts,
            [
                "Error: Could not reduce sub-expression before a semicolon to a single operation to evaluate"
            ]
        );
        let mut empty = parent(Op::Semicolon, vec![group(vec![])]);
        let (ok, log, _) = optimise_node(&mut empty, &client(13));
        assert!(!ok);
        assert_eq!(log[0].0, "E31");
    }

    #[test]
    fn statements_replaces_each_group_by_its_node() {
        let mut node = parent(
            Op::Semicolon,
            vec![group(vec![entity("a")]), group(vec![float(2.0)])],
        );
        let (result, log, _) = params_cx(|cx| statements(cx, &mut node));
        assert!(result.is_ok() && log.is_empty());
        assert_eq!(node.tree_notation(9), "(Semicolon v.a 2)");
    }

    #[test]
    fn call_parameters_flattens_a_left_nested_comma_chain_in_source_order() {
        let mut node = call(
            Op::Clamp,
            vec![entity("a"), entity("b"), entity("c"), entity("d")],
        );
        let (result, log, _) = params_cx(|cx| call_parameters(cx, &mut node));
        assert!(result.is_ok() && log.is_empty());
        assert_eq!(node.tree_notation(9), "(Clamp v.a v.b v.c v.d)");
    }

    #[test]
    fn call_parameters_flattens_a_right_nested_chain_in_source_order_too() {
        let chain = parent(
            Op::Comma,
            vec![
                entity("a"),
                parent(Op::Comma, vec![entity("b"), entity("c")]),
            ],
        );
        let mut node = parent(Op::Max, vec![parent(Op::LeftParenthesis, vec![chain])]);
        let (result, _, _) = params_cx(|cx| call_parameters(cx, &mut node));
        assert!(result.is_ok());
        assert_eq!(node.tree_notation(9), "(Max v.a v.b v.c)");
    }

    #[test]
    fn call_parameters_with_a_single_argument_keeps_it() {
        let mut node = call(Op::Abs, vec![entity("a")]);
        let (result, _, _) = params_cx(|cx| call_parameters(cx, &mut node));
        assert!(result.is_ok());
        assert_eq!(node.tree_notation(9), "(Abs v.a)");
    }

    #[test]
    fn call_parameters_looks_only_at_the_first_child_of_the_section() {
        let mut node = parent(
            Op::Abs,
            vec![parent(Op::LeftParenthesis, vec![entity("a"), entity("b")])],
        );
        let (result, _, _) = params_cx(|cx| call_parameters(cx, &mut node));
        assert!(result.is_ok());
        assert_eq!(node.tree_notation(9), "(Abs v.a)");
    }

    #[test]
    fn call_parameters_rejects_a_comma_without_operands() {
        let chain = parent(
            Op::Comma,
            vec![at(parent(Op::Comma, vec![]), 8, 9), entity("b")],
        );
        let mut node = parent(Op::Min, vec![parent(Op::LeftParenthesis, vec![chain])]);
        let (result, log, texts) = params_cx(|cx| call_parameters(cx, &mut node));
        assert!(result.is_err());
        assert_eq!(log, [("E35", Severity::Error, (8, 9))]);
        assert_eq!(
            texts,
            [
                "Error while optimizing parameters for Min 'math.min' operation: comma found without a following expression."
            ]
        );
    }

    #[test]
    fn call_parameters_needs_exactly_one_child() {
        for children in [
            vec![],
            vec![
                parent(Op::LeftParenthesis, vec![float(1.0)]),
                parent(Op::LeftParenthesis, vec![float(1.0)]),
            ],
        ] {
            let mut node = parent(Op::Loop, children);
            let (result, log, texts) = params_cx(|cx| call_parameters(cx, &mut node));
            assert!(result.is_err());
            assert_eq!(log, [("E35", Severity::Error, (0, 1))]);
            assert_eq!(
                texts,
                [
                    "Loop 'loop' operator should have exactly one child (a left-parenthesis expression with the params as children of it) prior to optimization."
                ]
            );
        }
    }

    #[test]
    fn call_parameters_rejects_an_empty_parenthesis() {
        let mut node = parent(Op::Min, vec![parent(Op::LeftParenthesis, vec![])]);
        let (result, log, texts) = params_cx(|cx| call_parameters(cx, &mut node));
        assert!(result.is_err());
        assert_eq!(log[0].0, "E35");
        assert_eq!(
            texts,
            [
                "Min 'math.min' operator (math, query, loop, etc) with empty parameter list should have failed to parse"
            ]
        );
    }

    #[test]
    fn a_query_without_parentheses_skips_the_parameter_pass() {
        assert!(tree("q.is_baby").is(Op::QueryFunction));
        assert!(tree("q.is_baby").children.is_empty());
    }

    #[test]
    fn the_walk_does_not_descend_into_leaves() {
        for leaf in [
            float(1.0),
            entity("x"),
            temp("t"),
            string("a"),
            Node::token(Op::Break, Payload::None, Span::new(0, 1)),
            Node::token(Op::Continue, Payload::None, Span::new(0, 1)),
            Node::token(Op::This, Payload::None, Span::new(0, 1)),
            Node::token(
                Op::Geometry,
                Payload::Geometry(Name::new("geometry.a")),
                Span::new(0, 1),
            ),
        ] {
            assert!(is_leaf(&leaf), "{:?}", leaf.op);
        }
    }

    #[test]
    fn before_children_descends_into_ordinary_operators() {
        for op in [
            Op::Add,
            Op::Mul,
            Op::Abs,
            Op::LogicalAnd,
            Op::Return,
            Op::Assignment,
            Op::Pointer,
            Op::MemberAccessor,
        ] {
            let mut node = parent(op, vec![float(1.0)]);
            assert!(run_before(&mut node).0.is_ok(), "{op:?}");
        }
    }

    #[test]
    fn before_children_replaces_a_call_parenthesis_by_the_arguments() {
        let mut node = call(Op::Max, vec![entity("a"), entity("b")]);
        let (result, log) = run_before(&mut node);
        assert!(result.is_ok());
        assert!(log.is_empty());
        assert_eq!(node.tree_notation(9), "(Max v.a v.b)");
        let mut three = call(Op::Clamp, vec![entity("a"), entity("b"), entity("c")]);
        assert!(run_before(&mut three).0.is_ok());
        assert_eq!(three.children.len(), 3);
    }

    #[test]
    fn before_children_checks_the_argument_count_of_a_call() {
        let mut node = at(call(Op::Min, vec![entity("a")]), 2, 5);
        let (result, log) = run_before(&mut node);
        assert!(result.is_err());
        assert_eq!(log, [("E23", Severity::Error, (0, 5))]);
        let mut node = call(
            Op::Lerp,
            vec![entity("a"), entity("b"), entity("c"), entity("d")],
        );
        let (result, log) = run_before(&mut node);
        assert!(result.is_err());
        assert_eq!(log[0].0, "E23");
    }

    #[test]
    fn before_children_lets_a_query_without_arguments_through() {
        let mut node = Node::token(Op::QueryFunction, Payload::None, Span::new(0, 1));
        assert!(run_before(&mut node).0.is_ok());
        let mut with_args = call(Op::QueryFunction, vec![float(1.0), float(2.0)]);
        assert!(run_before(&mut with_args).0.is_ok());
        assert_eq!(with_args.children.len(), 2);
    }

    #[test]
    fn before_children_rejects_a_comma_and_checks_loop_and_for_each() {
        let mut comma = parent(Op::Comma, vec![float(1.0), float(2.0)]);
        assert!(run_before(&mut comma).0.is_err());
        let mut looping = call(Op::Loop, vec![float(2.0), block(vec![float(1.0)])]);
        assert!(run_before(&mut looping).0.is_ok());
        let mut bad_loop = call(Op::Loop, vec![float(2.0), float(1.0)]);
        let (result, log) = run_before(&mut bad_loop);
        assert!(result.is_err());
        assert_eq!(log[0].0, "E30");
        let mut each = call(Op::ForEach, vec![temp("a"), entity("l"), float(1.0)]);
        assert!(run_before(&mut each).0.is_ok());
    }

    #[test]
    fn before_children_reshapes_a_colon_node_under_a_conditional() {
        let mut node = parent(
            Op::Conditional,
            vec![
                entity("c"),
                parent(Op::ConditionalElse, vec![entity("a"), entity("b")]),
            ],
        );
        assert!(run_before(&mut node).0.is_ok());
        assert_eq!(node.tree_notation(9), "(Conditional v.c v.a v.b)");
        let mut plain = parent(Op::Conditional, vec![entity("c"), entity("a"), entity("b")]);
        assert!(run_before(&mut plain).0.is_ok());
        assert_eq!(plain.children.len(), 3);
    }

    mod tree_shapes {
        use crate::compile::test_support::pipeline::{assert_parsed, assert_rejected, tree};

        #[test]
        fn member_chains_are_assignable() {
            assert_eq!(
                tree("v.x.y.z = 1;"),
                "(Semicolon (Assignment (MemberAccessor (MemberAccessor v.x))))"
            );
            assert_parsed("v.x.y.z = 1;", 13, &[]);
            assert_parsed("t.x = 1; return t.x;", 13, &[]);
        }

        #[test]
        fn this_is_read_only() {
            assert_eq!(tree("this + 1"), "[This*1+1]");
            assert_eq!(tree("return this;"), "(Semicolon (Return This))");
            assert_rejected("this = 1;", 13, &["E28"]);
        }

        /// Any count expression parses, even a string, and a space before `(` is allowed.
        #[test]
        fn loop_parameters() {
            assert_rejected("loop(3, v.x = v.x + 1);", 13, &["E30"]);
            assert_rejected("loop(3);", 13, &["E30"]);
            assert_rejected("loop(3, {v.x = 1;}, 2);", 13, &["E30"]);
            assert_parsed("loop (3, {v.x = 1;});", 13, &[]);
            assert_parsed("loop('a', {v.y = 1;});", 13, &[]);
            assert_parsed("loop(v.n, {v.c = v.c + 1;});", 13, &[]);
            assert_eq!(
                tree("loop(1+1, {v.c = 1;});"),
                "(Semicolon (Loop 2 (Semicolon (Assignment v.c))))"
            );
        }

        /// A third parameter that is not a block is accepted, unless it is an assignment.
        #[test]
        fn for_each_parameters() {
            assert_rejected("for_each(v.x, v.arr);", 13, &["E27"]);
            assert_rejected("for_each(v.x, v.arr, v.y = 1);", 13, &["E27"]);
            assert_parsed("for_each(v.x, v.a, 1);", 13, &[]);
            assert_eq!(
                tree("for_each(v.e, v.arr, {v.x = 1;});"),
                "(Semicolon (ForEach v.e v.arr (Semicolon (Assignment v.x))))"
            );
        }

        #[test]
        fn for_each_array_is_not_checked() {
            assert_eq!(
                tree("for_each(v.x, 1, {v.y = 1;});"),
                "(Semicolon (ForEach v.x 1 (Semicolon (Assignment v.y))))"
            );
            assert_parsed("for_each(v.x, 1, {v.y = 1;});", 13, &[]);
        }
    }
}
