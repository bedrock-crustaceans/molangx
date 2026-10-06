//! The validator: the checks that run over the optimised tree.

use crate::compile::{
    Cx, Failed, Pass,
    ast::{Node, Payload},
};
use crate::diag::{LanguageMessage as Msg, Severity};
use crate::ops::ExpressionOp as Op;

/// Validates from the root: fails for a finding at the root.
pub(in crate::compile) fn validate_root(cx: &mut Cx<'_>, root: &Node) -> Pass {
    check_tree(cx, root)
}

/// Logs a finding on `node`, an error at the root and a warning below it (unless the
/// `validate_nested` deviation is off). Always `Err`: the node has a finding.
fn finding(
    cx: &mut Cx<'_>,
    is_root: bool,
    message: Msg,
    node: &Node,
    args: &[&dyn std::fmt::Display],
) -> Pass {
    let severity = if is_root || !cx.opts.deviations.validate_nested {
        Severity::Error
    } else {
        Severity::Warning
    };
    cx.language_as(message, severity, node.full_span(), args);
    Err(Failed)
}

/// Where a node stands relative to the target (left side) of an assignment.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Lhs {
    Outside,
    /// The whole target.
    Whole,
    /// Below the target.
    Within,
}

/// Post-order on an explicit stack. A finding below the root does not fail its parent, so the
/// result is the root's.
fn check_tree(cx: &mut Cx<'_>, root: &Node) -> Pass {
    struct Frame<'a> {
        node: &'a Node,
        next: usize,
        inside_loop: bool,
        lhs: Lhs,
    }
    let mut stack = vec![Frame {
        node: root,
        next: 0,
        inside_loop: false,
        lhs: Lhs::Outside,
    }];
    loop {
        let is_root = stack.len() == 1;
        let Some(frame) = stack.last_mut() else {
            return Ok(());
        };
        let node = frame.node;
        let index = frame.next;
        let result = match node.children.get(index) {
            // A statement after `break`, `continue` or `return`, checked on every node's children,
            // is the node's finding: its other children and its own checks are skipped.
            Some(child)
                if index + 1 != node.children.len()
                    && matches!(child.op, Op::Break | Op::Continue | Op::Return) =>
            {
                finding(cx, is_root, Msg::Unreachable, child, &[&cx.friendly(child)])
            }
            Some(child) => {
                frame.next += 1;
                let (inside_loop, lhs) = if node.is(Op::QueryFunction) {
                    (false, Lhs::Outside)
                } else {
                    let inside_loop =
                        frame.inside_loop || matches!(node.op, Op::Loop | Op::ForEach);
                    let lhs = match frame.lhs {
                        Lhs::Outside if index == 0 && node.is(Op::Assignment) => Lhs::Whole,
                        Lhs::Outside => Lhs::Outside,
                        Lhs::Whole | Lhs::Within => Lhs::Within,
                    };
                    (inside_loop, lhs)
                };
                stack.push(Frame {
                    node: child,
                    next: 0,
                    inside_loop,
                    lhs,
                });
                continue;
            }
            None => check(cx, node, frame.inside_loop, frame.lhs, is_root),
        };
        stack.pop();
        if stack.is_empty() {
            return result;
        }
    }
}

/// `Err` when the node has a finding.
fn check(cx: &mut Cx<'_>, node: &Node, inside_loop: bool, lhs: Lhs, is_root: bool) -> Pass {
    let op = node.op;
    // Inside the left side of an assignment only variables, members, queries and `->` may appear,
    // and a temp variable only as the whole left side.
    if lhs != Lhs::Outside {
        match op {
            Op::QueryFunction | Op::EntityVariable | Op::MemberAccessor | Op::Pointer => {}
            Op::TempVariable if lhs == Lhs::Whole => return Ok(()),
            Op::TempVariable => return finding(cx, is_root, Msg::TempLhsNotAlone, node, &[]),
            _ => {
                return finding(cx, is_root, Msg::OperatorOnLhs, node, &[&cx.friendly(node)]);
            }
        }
    }

    match op {
        Op::Pointer => {
            let [target, member] = node.children.as_slice() else {
                return Ok(());
            };
            if target.is(Op::Pointer) {
                return finding(cx, is_root, Msg::NestedPointer, node, &[]);
            }
            if !matches!(member.op, Op::QueryFunction | Op::EntityVariable) {
                return finding(cx, is_root, Msg::PointerRhs, node, &[]);
            }
        }
        Op::Array if node.has_post_op() => {
            return finding(cx, is_root, Msg::MathOnArray, node, &[]);
        }
        Op::Assignment => {
            let Some(target) = node.children.first() else {
                return Ok(());
            };
            match target.op {
                Op::EntityVariable | Op::MemberAccessor => {}
                Op::Pointer => return finding(cx, is_root, Msg::AssignToPointer, node, &[]),
                _ if matches!(target.value, Payload::Temp(_)) => {}
                _ => {
                    return finding(
                        cx,
                        is_root,
                        Msg::AssignmentForm,
                        node,
                        &[&cx.friendly(target)],
                    );
                }
            }
        }
        Op::NullCoalescing
            if node.children.first().is_some_and(|left| {
                !matches!(
                    left.value,
                    Payload::Context(_) | Payload::Entity(_) | Payload::Temp(_)
                )
            }) =>
        {
            return finding(cx, is_root, Msg::CoalesceLhs, node, &[]);
        }
        Op::Break if !inside_loop => return finding(cx, is_root, Msg::BreakOutsideLoop, node, &[]),
        _ => {}
    }
    Ok(())
}

fn contains_node(root: &Node, test: impl Fn(&Node) -> bool) -> bool {
    let mut stack = vec![root];
    while let Some(node) = stack.pop() {
        if test(node) {
            return true;
        }
        stack.extend(node.children.iter());
    }
    false
}

pub(in crate::compile) fn contains_op(root: &Node, test: impl Fn(Op) -> bool) -> bool {
    contains_node(root, |node| test(node.op))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compile::{
        CompileOptions, Deviations,
        ast::{Name, Span},
        sema::test_support::*,
    };

    fn validate_tree(root: &Node, opts: &CompileOptions) -> (bool, Vec<Logged>, Vec<String>) {
        let mut cx = Cx::for_test("abcdefghijklmnopqrstuvwxyz", opts);
        let failed = validate_root(&mut cx, root).is_err();
        (failed, logged(&cx), texts(&cx))
    }

    /// Validates the tree `src` optimises to without a message.
    fn validated_with(src: &str, opts: &CompileOptions) -> (bool, Vec<Logged>) {
        let out = optimise_with(src, opts);
        assert!(
            out.ok && out.log.is_empty(),
            "{src:?} does not optimise cleanly: {:?}",
            out.texts
        );
        let mut cx = Cx::for_test(src, opts);
        let failed = validate_root(&mut cx, &out.root).is_err();
        (failed, logged(&cx))
    }

    fn validated(src: &str) -> (bool, Vec<Logged>) {
        validated_with(src, &client(13))
    }

    fn entities(count: usize) -> Vec<Node> {
        (0..count)
            .map(|index| entity(&format!("v{index}")))
            .collect()
    }

    fn finding_ids(root: &Node) -> Vec<&'static str> {
        validate_tree(root, &client(13))
            .1
            .iter()
            .map(|m| m.0)
            .collect()
    }

    fn bad_pointer() -> Node {
        parent(
            Op::Pointer,
            vec![
                parent(Op::Pointer, vec![entity("a"), entity("b")]),
                entity("c"),
            ],
        )
    }

    #[test]
    fn a_tree_without_findings_is_kept() {
        for src in [
            "1",
            "v.x",
            "v.x*2+1",
            "math.min(v.x,3)",
            "v.x=1;",
            "loop(3,{v.x=1;});",
            "v.a?1:2",
            "q.is_baby",
        ] {
            assert_eq!(validated(src), (false, vec![]), "{src}");
        }
    }

    #[test]
    fn the_accepted_counts_have_no_finding() {
        for (op, counts) in [
            (Op::Clamp, vec![3]),
            (Op::EaseInOutBack, vec![3]),
            (Op::Abs, vec![1]),
            (Op::LogicalNot, vec![1]),
            (Op::Max, vec![1, 2]),
            (Op::LessThan, vec![1, 2]),
            (Op::LogicalEqual, vec![1, 2]),
            (Op::Atan2, vec![2]),
            (Op::Mul, vec![2]),
            (Op::Loop, vec![2]),
            (Op::LogicalAnd, vec![2, 3, 6]),
            (Op::LogicalOr, vec![2, 3, 6]),
            (Op::Conditional, vec![2, 3]),
            (Op::Return, vec![1, 2]),
            (Op::Assignment, vec![1, 2]),
            (Op::NullCoalescing, vec![2]),
            (Op::Pointer, vec![2]),
            (Op::Add, vec![1, 2, 7]),
        ] {
            for count in counts {
                let (failed, log, texts) = validate_tree(&parent(op, entities(count)), &client(13));
                assert!(!failed && log.is_empty(), "{op:?} with {count}: {texts:?}");
            }
        }
    }

    #[test]
    fn a_string_without_a_post_op_has_no_math_on_it() {
        let (failed, log, _) = validate_tree(&string("a"), &client(13));
        assert!(!failed && log.is_empty());
    }

    #[test]
    fn math_on_an_array_element_is_an_error_at_every_version() {
        for raw in [0, 2, 13] {
            let node = with_post(parent(Op::Array, vec![float(0.0)]), 2.0, 0.0);
            let (failed, log, texts) = validate_tree(&node, &client(raw));
            assert!(failed, "version {raw}");
            assert_eq!(log, [("E42", Severity::Error, (0, 1))]);
            assert_eq!(
                texts,
                ["Error: can't currently do math operations on resource array results"]
            );
        }
        let (failed, log, _) = validate_tree(&parent(Op::Array, vec![float(0.0)]), &client(13));
        assert!(!failed && log.is_empty());
    }

    #[test]
    fn math_on_an_array_element_through_the_optimiser() {
        assert_eq!(
            validated("array.a[0]*2"),
            (true, vec![("E42", Severity::Error, (0, 9))])
        );
        assert_eq!(validated("array.a[0]"), (false, vec![]));
    }

    #[test]
    fn an_assignment_target_is_a_variable_a_member_or_a_temp() {
        for target in [entity("a"), member("m", entity("a")), temp("t")] {
            let (failed, log, _) = validate_tree(
                &parent(Op::Assignment, vec![target, float(1.0)]),
                &client(13),
            );
            assert!(!failed && log.is_empty());
        }
    }

    #[test]
    fn assigning_to_a_pointer_result_is_rejected() {
        let target = parent(Op::Pointer, vec![entity("a"), entity("b")]);
        let (failed, log, texts) = validate_tree(
            &at(parent(Op::Assignment, vec![target, float(1.0)]), 0, 9),
            &client(13),
        );
        assert!(failed);
        assert_eq!(log, [("E43", Severity::Error, (0, 9))]);
        assert_eq!(
            texts,
            [
                "Error: Assignment attempted on Pointer result. Writing to another entity's public variables is not supported, only reading them."
            ]
        );
    }

    #[test]
    fn any_other_assignment_target_is_the_wrong_form() {
        // The target is validated first, as part of the left side (E40), then the assignment
        // itself.
        let (failed, log, texts) = validate_tree(
            &parent(Op::Assignment, vec![float(2.0), float(1.0)]),
            &client(13),
        );
        assert!(failed);
        assert_eq!(log.iter().map(|m| m.0).collect::<Vec<_>>(), ["E40", "E47"]);
        assert!(
            texts[1].ends_with("Found an expression where C is a Float"),
            "{}",
            texts[1]
        );
        let context = Node::token(
            Op::ContextVariable,
            Payload::Context(Name::new("context.a")),
            Span::new(0, 1),
        );
        assert_eq!(
            finding_ids(&parent(Op::Assignment, vec![context, float(1.0)])),
            ["E40", "E47"]
        );
    }

    #[test]
    fn a_temp_variable_may_only_be_the_whole_left_side() {
        // `t.x.y = 1`: the temp is the base of a member accessor.
        let node = parent(
            Op::Assignment,
            vec![at(member("y", at(temp("x"), 0, 3)), 3, 5), float(1.0)],
        );
        let (failed, log, texts) = validate_tree(&node, &client(13));
        // The finding is on the temp, below the root: logged as a warning, the expression kept.
        assert!(!failed);
        assert_eq!(log, [("E39", Severity::Warning, (0, 3))]);
        assert_eq!(
            texts,
            [
                "Error: left side of an assignment expression can only use temp variables if they are on their own and not part of a more complicated expression."
            ]
        );
        assert_eq!(
            finding_ids(&parent(Op::Assignment, vec![temp("x"), float(1.0)])),
            Vec::<&str>::new()
        );
    }

    #[test]
    fn only_variables_members_queries_and_pointers_may_stand_on_the_left() {
        // An operator inside the member path of a target is reported on the operator.
        let target = member(
            "m",
            at(parent(Op::Add, vec![entity("a"), entity("b")]), 2, 4),
        );
        let node = parent(Op::Assignment, vec![target, float(1.0)]);
        let (failed, log, texts) = validate_tree(&node, &client(13));
        assert!(!failed);
        assert_eq!(log, [("E40", Severity::Warning, (0, 4))]);
        assert_eq!(
            texts,
            ["Error: cannot use Add '+' operators on the left side of an assignment expression"]
        );
    }

    #[test]
    fn the_left_side_may_hold_queries_and_pointers() {
        let query = parent(Op::QueryFunction, vec![]);
        let pointer = parent(Op::Pointer, vec![member("m", entity("a")), entity("b")]);
        for target in [
            member("m", query),
            member("n", member("m", entity("a"))),
            member("m", pointer),
        ] {
            let (_, log, _) = validate_tree(
                &parent(Op::Assignment, vec![target, float(1.0)]),
                &client(13),
            );
            assert!(log.is_empty(), "{log:?}");
        }
    }

    #[test]
    fn a_query_resets_the_left_side_tracking_for_its_arguments() {
        // `q.f(a + b).m = 1`: the sum is an argument, not part of the path.
        let query = parent(
            Op::QueryFunction,
            vec![parent(Op::Add, vec![entity("a"), entity("b")])],
        );
        let node = parent(Op::Assignment, vec![member("m", query), float(1.0)]);
        let (failed, log, _) = validate_tree(&node, &client(13));
        assert!(!failed && log.is_empty());
        // A temp variable in the arguments is not "part of the left side" either.
        let query = parent(Op::QueryFunction, vec![temp("t")]);
        let node = parent(Op::Assignment, vec![member("m", query), float(1.0)]);
        assert!(validate_tree(&node, &client(13)).1.is_empty());
    }

    #[test]
    fn the_right_side_of_an_assignment_is_not_the_left_side() {
        let node = parent(
            Op::Assignment,
            vec![entity("a"), parent(Op::Add, vec![temp("t"), entity("b")])],
        );
        assert!(validate_tree(&node, &client(13)).1.is_empty());
    }

    #[test]
    fn a_null_coalescing_left_side_must_be_a_direct_variable() {
        for lhs in [
            entity("a"),
            temp("a"),
            Node::token(
                Op::ContextVariable,
                Payload::Context(Name::new("context.a")),
                Span::new(0, 1),
            ),
        ] {
            let (failed, log, _) = validate_tree(
                &parent(Op::NullCoalescing, vec![lhs, float(1.0)]),
                &client(13),
            );
            assert!(!failed && log.is_empty());
        }
        for lhs in [
            member("m", entity("a")),
            float(1.0),
            parent(Op::Add, vec![entity("a"), entity("b")]),
        ] {
            let (failed, log, texts) = validate_tree(
                &at(parent(Op::NullCoalescing, vec![lhs, float(1.0)]), 0, 6),
                &client(13),
            );
            assert!(failed);
            assert_eq!(log.iter().map(|m| m.0).collect::<Vec<_>>(), ["E44"]);
            assert_eq!(
                texts,
                [
                    "Error: found left-hand-side of ?? expression that isn't a direct-variable reference - this is unsupported at this time."
                ]
            );
        }
    }

    #[test]
    fn a_variable_with_a_post_op_is_still_a_direct_variable() {
        // `(v.x + 1) ?? 3`: the term is a variable carrying (1, 1).
        assert_eq!(validated("(v.x+1) ?? 3"), (false, vec![]));
        assert_eq!(validated("t.x ?? 1"), (false, vec![]));
        assert_eq!(validated("c.x ?? 1"), (false, vec![]));
        assert_eq!(
            validated("v.x.y ?? 3"),
            (true, vec![("E44", Severity::Error, (0, 10))])
        );
        assert_eq!(
            validated("1 ?? 3"),
            (true, vec![("E44", Severity::Error, (0, 6))])
        );
    }

    #[test]
    fn pointers_cannot_nest_and_must_end_in_a_variable_or_query() {
        let nested = parent(
            Op::Pointer,
            vec![
                parent(Op::Pointer, vec![entity("a"), entity("b")]),
                entity("c"),
            ],
        );
        let (failed, log, texts) = validate_tree(&at(nested, 0, 13), &client(13));
        assert!(failed);
        assert_eq!(log, [("E41", Severity::Error, (0, 13))]);
        assert!(
            texts[0].starts_with(
                "Error: nested pointer statements (eg: A->B->C) are not yet supported."
            ),
            "{}",
            texts[0]
        );

        for rhs in [float(1.0), temp("t"), member("m", entity("b"))] {
            let (failed, log, texts) =
                validate_tree(&parent(Op::Pointer, vec![entity("a"), rhs]), &client(13));
            assert!(failed);
            assert_eq!(log.iter().map(|m| m.0).collect::<Vec<_>>(), ["E46"]);
            assert_eq!(
                texts,
                [
                    "Error: right-hand-side of pointer expression did not evaluate to an entity variable or query function"
                ]
            );
        }
        for rhs in [entity("b"), parent(Op::QueryFunction, vec![])] {
            assert!(
                validate_tree(&parent(Op::Pointer, vec![entity("a"), rhs]), &client(13))
                    .1
                    .is_empty()
            );
        }
    }

    #[test]
    fn pointers_through_the_optimiser() {
        assert_eq!(
            validated("v.x->v.y->v.z"),
            (true, vec![("E41", Severity::Error, (0, 13))])
        );
        assert_eq!(
            validated("v.a->3"),
            (true, vec![("E46", Severity::Error, (0, 6))])
        );
        assert_eq!(validated("v.x->v.y"), (false, vec![]));
        assert_eq!(validated("t.x->v.y"), (false, vec![]));
        assert_eq!(
            validated("v.x->v.y=1;"),
            (false, vec![("E43", Severity::Warning, (0, 9))])
        );
        assert_eq!(
            validated("q.get_name->v.y=2;"),
            (false, vec![("E43", Severity::Warning, (0, 16))])
        );
    }

    #[test]
    fn break_outside_a_loop_is_logged() {
        let (failed, log, texts) = validate_tree(
            &at(Node::token(Op::Break, Payload::None, Span::new(0, 5)), 0, 5),
            &client(13),
        );
        assert!(failed, "at the root it rejects");
        assert_eq!(log, [("E45", Severity::Error, (0, 5))]);
        assert_eq!(texts, ["Error: break encountered outside of loop"]);
        assert_eq!(
            validated("break;"),
            (false, vec![("E45", Severity::Warning, (0, 5))])
        );
    }

    #[test]
    fn break_inside_a_loop_or_for_each_is_fine() {
        for src in [
            "loop(3,{break;});",
            "for_each(t.a,v.l,{break;});",
            "loop(3,{v.a?break;});",
            "loop(3,{loop(2,{break;});break;});",
            "loop(2,{break;});v.x=1;",
        ] {
            assert_eq!(validated(src), (false, vec![]), "{src}");
        }
        assert_eq!(
            validated("loop(3,{v.a?break:1;});")
                .1
                .iter()
                .map(|m| m.0)
                .collect::<Vec<_>>(),
            ["E38"]
        );
    }

    #[test]
    fn break_after_a_loop_has_ended_is_outside_it() {
        assert_eq!(
            validated("loop(2,{v.x=1;});break;"),
            (false, vec![("E45", Severity::Warning, (17, 22))])
        );
    }

    #[test]
    fn continue_outside_a_loop_is_not_a_finding() {
        assert_eq!(validated("continue;"), (false, vec![]));
        assert_eq!(validated("loop(3,{continue;});"), (false, vec![]));
    }

    #[test]
    fn a_query_argument_is_not_inside_the_loop_around_the_query() {
        let looping = parent(
            Op::Loop,
            vec![
                float(3.0),
                parent(
                    Op::Semicolon,
                    vec![parent(
                        Op::QueryFunction,
                        vec![at(
                            Node::token(Op::Break, Payload::None, Span::new(0, 1)),
                            8,
                            9,
                        )],
                    )],
                ),
            ],
        );
        let (_, log, _) = validate_tree(&looping, &client(13));
        assert_eq!(log, [("E45", Severity::Warning, (8, 9))]);
        // Without the query in between the same `break` is inside the loop.
        let direct = parent(
            Op::Loop,
            vec![
                float(3.0),
                parent(
                    Op::Semicolon,
                    vec![Node::token(Op::Break, Payload::None, Span::new(8, 9))],
                ),
            ],
        );
        assert!(validate_tree(&direct, &client(13)).1.is_empty());
    }

    #[test]
    fn a_statement_after_break_continue_or_return_is_unreachable() {
        for (op, friendly) in [
            (Op::Break, "Break 'break'"),
            (Op::Continue, "Continue 'continue'"),
            (Op::Return, "Return 'return'"),
        ] {
            let jump = if op == Op::Return {
                parent(Op::Return, vec![float(1.0)])
            } else {
                Node::token(op, Payload::None, Span::new(0, 1))
            };
            let body = parent(Op::Semicolon, vec![at(jump, 2, 7), entity("x")]);
            let looping = parent(Op::Loop, vec![float(3.0), body]);
            let (failed, log, texts) = validate_tree(&looping, &client(13));
            assert!(!failed, "{op:?}");
            let span = if op == Op::Return { (0, 7) } else { (2, 7) };
            assert_eq!(log, [("E38", Severity::Warning, span)], "{op:?}");
            assert_eq!(
                texts,
                [format!("Error: unreachable statements after {friendly}.")],
                "{op:?}"
            );
        }
    }

    #[test]
    fn the_last_statement_may_be_a_jump() {
        for src in [
            "return 1;",
            "loop(3,{break;});",
            "loop(3,{continue;});",
            "v.x=1;return 1;",
        ] {
            assert_eq!(validated(src), (false, vec![]), "{src}");
        }
    }

    #[test]
    fn unreachable_statements_at_the_root_reject_the_expression() {
        assert_eq!(
            validated("return 1;v.x=2;"),
            (true, vec![("E38", Severity::Error, (0, 8))])
        );
        assert_eq!(
            validated("loop(2,{return 1;1;});"),
            (false, vec![("E38", Severity::Warning, (8, 16))])
        );
        assert_eq!(
            validated("v.a?break:1;"),
            (false, vec![("E38", Severity::Warning, (4, 9))])
        );
    }

    #[test]
    fn a_jump_before_another_operand_of_any_parent_is_unreachable() {
        // The rule is on every node's children, not just statement lists.
        let node = parent(
            Op::Add,
            vec![
                at(
                    Node::token(Op::Continue, Payload::None, Span::new(0, 1)),
                    0,
                    1,
                ),
                float(1.0),
            ],
        );
        assert_eq!(finding_ids(&node), ["E38"]);
    }

    #[test]
    fn a_finding_at_the_root_rejects() {
        let (failed, log, _) = validate_tree(&bad_pointer(), &client(13));
        assert!(failed);
        assert_eq!(log, [("E41", Severity::Error, (0, 1))]);
    }

    #[test]
    fn the_same_finding_below_the_root_is_logged_and_kept_as_a_warning() {
        let root = parent(
            Op::Conditional,
            vec![entity("c"), bad_pointer(), float(1.0)],
        );
        let (failed, log, _) = validate_tree(&root, &client(13));
        assert!(!failed);
        assert_eq!(log, [("E41", Severity::Warning, (0, 1))]);
    }

    #[test]
    fn without_deviations_a_nested_finding_logs_as_an_error_and_is_still_kept() {
        let root = parent(
            Op::Conditional,
            vec![entity("c"), bad_pointer(), float(1.0)],
        );
        let no_deviations = CompileOptions {
            deviations: Deviations::NONE,
            ..client(13)
        };
        let (failed, log, _) = validate_tree(&root, &no_deviations);
        assert!(!failed);
        assert_eq!(log, [("E41", Severity::Error, (0, 1))]);
        let only = CompileOptions {
            deviations: Deviations {
                validate_nested: false,
                ..Deviations::ALL
            },
            ..client(13)
        };
        assert_eq!(
            validate_tree(&root, &only).1,
            [("E41", Severity::Error, (0, 1))]
        );
    }

    #[test]
    fn a_nested_finding_does_not_reject_but_the_root_s_own_finding_does() {
        // A bad child of a bad root: both are logged, the child first; only the root's rejects.
        let root = parent(Op::Pointer, vec![bad_pointer(), entity("c")]);
        let (failed, log, _) = validate_tree(&root, &client(13));
        assert!(failed);
        assert_eq!(
            log.iter().map(|m| (m.0, m.1)).collect::<Vec<_>>(),
            [("E41", Severity::Warning), ("E41", Severity::Error)]
        );
    }

    #[test]
    fn nested_findings_through_the_optimiser_are_warnings() {
        assert_eq!(
            validated("t.x.y=1;"),
            (false, vec![("E39", Severity::Warning, (0, 3))])
        );
        let no_deviations = CompileOptions {
            deviations: Deviations::NONE,
            ..client(13)
        };
        assert_eq!(
            validated_with("t.x.y=1;", &no_deviations),
            (false, vec![("E39", Severity::Error, (0, 3))])
        );
        assert_eq!(
            validated_with("break;", &no_deviations),
            (false, vec![("E45", Severity::Error, (0, 5))])
        );
        assert_eq!(
            validated_with("v.x->v.y->v.z", &no_deviations),
            (true, vec![("E41", Severity::Error, (0, 13))])
        );
    }

    #[test]
    fn a_finding_has_the_effect_root_only_in_the_message_table() {
        use crate::diag::Effect;
        for message in [
            Msg::Unreachable,
            Msg::TempLhsNotAlone,
            Msg::OperatorOnLhs,
            Msg::NestedPointer,
            Msg::BreakOutsideLoop,
            Msg::CoalesceLhs,
        ] {
            assert_eq!(message.effect(), Effect::RootOnly, "{message:?}");
        }
    }

    #[test]
    fn finding_logs_with_the_full_span_and_fails() {
        let opts = client(13);
        let mut cx = Cx::for_test("abcdefghij", &opts);
        let node = at(parent(Op::Add, vec![at(entity("a"), 5, 8)]), 2, 3);
        assert!(finding(&mut cx, true, Msg::OperatorOnLhs, &node, &[&"x"]).is_err());
        assert_eq!(logged(&cx), [("E40", Severity::Error, (2, 8))]);
        assert_eq!(
            texts(&cx),
            ["Error: cannot use x operators on the left side of an assignment expression"]
        );
        assert!(finding(&mut cx, false, Msg::NestedPointer, &node, &[]).is_err());
        assert_eq!(logged(&cx)[1], ("E41", Severity::Warning, (2, 8)));
    }

    #[test]
    fn validation_visits_every_node_and_logs_in_post_order() {
        let root = parent(
            Op::Semicolon,
            vec![
                bad_pointer(),
                parent(Op::Abs, vec![at(bad_pointer(), 6, 7)]),
            ],
        );
        let (failed, log, _) = validate_tree(&root, &client(13));
        assert!(!failed);
        assert_eq!(
            log,
            [
                ("E41", Severity::Warning, (0, 1)),
                ("E41", Severity::Warning, (0, 7))
            ]
        );
    }

    #[test]
    fn validation_of_a_deep_tree_does_not_recurse() {
        let result = std::thread::Builder::new()
            .stack_size(512 * 1024)
            .spawn(|| {
                let mut deep = float(1.0);
                for _ in 0..100_000 {
                    deep = parent(Op::LogicalNot, vec![deep]);
                }
                let (failed, log, _) = validate_tree(&deep, &client(13));
                assert!(!failed && log.is_empty());
                assert!(!contains_op(&deep, |op| op == Op::Assignment));
                assert!(contains_op(&deep, |op| op == Op::Float));
                assert!(contains_node(&deep, |node| node.is(Op::Float)));
            })
            .expect("a thread spawns")
            .join();
        assert!(result.is_ok());
    }

    #[test]
    fn contains_op_looks_at_the_ops_of_all_nodes() {
        let tree = parent(
            Op::Semicolon,
            vec![parent(
                Op::Conditional,
                vec![
                    entity("a"),
                    parent(Op::Assignment, vec![entity("b"), float(1.0)]),
                ],
            )],
        );
        assert!(contains_op(&tree, |op| op == Op::Assignment));
        assert!(contains_op(&tree, |op| op == Op::Semicolon));
        assert!(contains_op(&tree, |op| op == Op::Float));
        assert!(!contains_op(&tree, |op| op == Op::Loop));
        assert!(!contains_op(&float(1.0), |op| op == Op::Assignment));
    }

    #[test]
    fn validate_root_is_what_compile_calls_after_the_optimiser() {
        let out = optimise_with("return 1;v.x=2;", &client(13));
        assert!(out.ok && out.log.is_empty());
        let opts = client(13);
        let mut cx = Cx::for_test("return 1;v.x=2;", &opts);
        assert!(validate_root(&mut cx, &out.root).is_err());
    }

    mod tree_shapes {
        use crate::compile::test_support::pipeline::{
            assert_parsed, assert_rejected, at, ids, tree,
        };

        /// `->` accepts `v.` and `query.` on its right; other right sides log E46.
        #[test]
        fn pointer_right_side() {
            assert_eq!(tree("c.other->v.x"), "(Pointer c.other v.x)");
            assert_parsed("c.other->q.is_baby", 13, &[]);
            assert_parsed("v.x->v.y", 13, &[]);
            for source in [
                "c.other->t.x",
                "c.owner->1",
                "c.owner->this",
                "c.owner->v.a.b",
                "c.x->v.y.z",
                "c.a->math.pi",
            ] {
                assert_rejected(source, 13, &["E46"]);
            }
        }

        #[test]
        fn parenthesised_pointer_right_side() {
            assert_eq!(tree("c.other->(v.x)"), "(Pointer c.other v.x)");
        }

        #[test]
        fn null_coalescing_ignores_a_folded_post_op() {
            for (source, expected) in [
                ("-v.x ?? 1", "(NullCoalescing [v.x*-1+0] 1)"),
                ("v.x * 2 ?? 1", "(NullCoalescing [v.x*2+0] 1)"),
                ("v.x - 3 ?? 1", "(NullCoalescing [v.x*1+-3] 1)"),
                ("(v.x + 0) ?? 1", "(NullCoalescing v.x 1)"),
            ] {
                assert_parsed(source, 13, &[]);
                assert_eq!(tree(source), expected);
            }
        }

        /// The root list rejects, a nested one keeps; the check also covers the operands of any
        /// node.
        #[test]
        fn unreachable_statements() {
            assert_rejected("return 1; return 2;", 13, &["E38"]);
            assert_rejected("return 0; return 0;", 13, &["E38"]);
            assert_parsed("loop(3, {break; v.x = 1;});", 13, &["E38"]);
            assert_eq!(
                tree("loop(3, {break; v.x = 1;});"),
                "(Semicolon (Loop 3 (Semicolon Break (Assignment v.x))))"
            );
            assert_parsed("v.x ? break : 1;", 13, &["E38"]);
            assert_parsed("loop(2, {v.x ? break : continue;});", 13, &["E38"]);
        }

        #[test]
        fn continue_outside_a_loop() {
            assert_parsed("continue;", 13, &[]);
            assert_eq!(tree("continue;"), "(Semicolon Continue)");
            assert_parsed("v.x = 1; continue;", 13, &[]);
        }

        /// A chained assignment statement is E28; under `return` it parses, logging E40 and E47,
        /// and then fails to link (E48).
        #[test]
        fn chained_assignment() {
            assert_rejected("v.x = v.y = 1;", 13, &["E28"]);
            assert_eq!(
                tree("return v.b = v.a = 1;"),
                "(Semicolon (Return (Assignment (Assignment v.b v.a))))"
            );
            let compiled = at("return v.b = v.a = 1;", 13);
            assert!(compiled.parsed());
            assert_eq!(ids(&compiled), ["E40", "E47", "E48"]);
        }
    }
}
