//! Which expressions the tree walker models exactly.

use molangx::compile::Expr;
use molangx::internals::{Node, tree};
use molangx::ops::ExpressionOp as Op;

/// Whether [`eval`](super::eval) models `expr` exactly. It does not when
/// - a `break` / `continue` can leave a `??` left side inside the loop it belongs to: the VM then
///   resumes the abandoned right side at a later missing read. Such an expression compiles only
///   with the `??` logged as unsupported;
/// - a `break` / `continue` of a `loop` can run inside an operand (anywhere but statement
///   position, a `?:` branch or condition, a block, an assignment's value, a `return`, an `&&` /
///   `||` operand or a `??` side): the loop then ends early and an enclosing loop runs past its
///   count, through the VM's operand stack, which the walker lacks;
/// - it has a `for_each`: over a number or a string the statement after it does not take effect,
///   which the VM gets by skipping one instruction and a tree has no instructions for.
pub fn models(expr: &Expr) -> bool {
    tree(expr).is_none_or(|tree| {
        !has_for_each(tree)
            && !jumps_out_of_handler(tree, false, false)
            && !jumps_with_pending(tree, false, false)
    })
}

/// Whether `node` or a node under it is a `for_each`.
fn has_for_each(node: &Node) -> bool {
    let mut pending = vec![node];
    while let Some(next) = pending.pop() {
        if next.is(Op::ForEach) {
            return true;
        }
        pending.extend(next.children().iter());
    }
    false
}

/// Whether a `break` / `continue` under `node` that belongs to a `loop` can run with an operand
/// pending. `pending`: an enclosing node of the loop body may have pushed an operand before
/// evaluating `node` (any node but the ones that evaluate their children without pushing);
/// `in_loop`: `node` is inside a `loop` body of the current (sub-)program, as the innermost loop.
fn jumps_with_pending(node: &Node, pending: bool, in_loop: bool) -> bool {
    match node.op() {
        Op::Break | Op::Continue => pending && in_loop,
        // The count is in the enclosing context; the body starts with nothing pending.
        Op::Loop => node.children().split_last().is_some_and(|(body, rest)| {
            rest.iter().any(|c| jumps_with_pending(c, pending, in_loop))
                || jumps_with_pending(body, false, true)
        }),
        // A `for_each` keeps its state off the stack and drops what its body left: a jump in its
        // body is the walker's.
        Op::ForEach => node.children().split_last().is_some_and(|(body, rest)| {
            rest.iter().any(|c| jumps_with_pending(c, pending, in_loop))
                || jumps_with_pending(body, false, false)
        }),
        // A query argument is a separate expression.
        Op::QueryFunction => node
            .children()
            .iter()
            .any(|c| jumps_with_pending(c, false, false)),
        // Nodes that evaluate their children without pushing an operand.
        Op::Semicolon
        | Op::LeftBrace
        | Op::Conditional
        | Op::ConditionalElse
        | Op::Assignment
        | Op::Return
        | Op::LogicalAnd
        | Op::LogicalOr
        | Op::NullCoalescing => node
            .children()
            .iter()
            .any(|c| jumps_with_pending(c, pending, in_loop)),
        _ => node
            .children()
            .iter()
            .any(|c| jumps_with_pending(c, true, in_loop)),
    }
}

/// Whether a `break` / `continue` under `node` leaves a `??` left side for a loop outside it.
/// `in_left`: `node` is inside the left side of a `??` within the innermost loop body of the
/// current (sub-)program; `in_loop`: there is such a loop body (without one a jump ends the
/// (sub-)program, which the walker models).
fn jumps_out_of_handler(node: &Node, in_left: bool, in_loop: bool) -> bool {
    match node.op() {
        Op::Break | Op::Continue => in_left && in_loop,
        // A loop body starts afresh: a jump inside it stays inside it; the count / array are
        // still in the enclosing context.
        Op::Loop | Op::ForEach => node.children().split_last().is_some_and(|(body, rest)| {
            rest.iter()
                .any(|c| jumps_out_of_handler(c, in_left, in_loop))
                || jumps_out_of_handler(body, false, true)
        }),
        Op::NullCoalescing => node
            .children()
            .iter()
            .enumerate()
            .any(|(i, c)| jumps_out_of_handler(c, in_left || i == 0, in_loop)),
        // A query argument is a separate expression: no `??` or loop of the caller reaches in.
        Op::QueryFunction => node
            .children()
            .iter()
            .any(|c| jumps_out_of_handler(c, false, false)),
        _ => node
            .children()
            .iter()
            .any(|c| jumps_out_of_handler(c, in_left, in_loop)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tree_walker::test_support::*;

    fn brk() -> Node {
        node(Op::Break, vec![])
    }

    fn cont() -> Node {
        node(Op::Continue, vec![])
    }

    /// What the jump analysis says of `src`, with the `for_each` rule left out: the shapes below
    /// put jumps inside `for_each` bodies to test the analysis, which `models` declines anyway.
    fn modelled(src: &str) -> bool {
        let expr = build(src);
        tree(&expr).is_none_or(|tree| {
            !jumps_out_of_handler(tree, false, false) && !jumps_with_pending(tree, false, false)
        })
    }

    #[test]
    fn models_declines_every_for_each() {
        for src in [
            "for_each(t.e, c.arr, { v.x; });",
            "v.a = 1; for_each(t.e, v.n, 1); v.b = 2;",
            "loop(2, { for_each(v.e, c.arr, { v.x = 1; }); });",
        ] {
            assert!(!models(&build(src)), "{src}");
            assert!(modelled(src), "{src}");
        }
        assert!(models(&build("loop(2, { v.x = 1; });")));
    }

    #[test]
    fn models_accepts_ordinary_expressions() {
        for src in [
            "v.a + 1",
            "v.x ? 1 : 2",
            "v.a = v.x ?? 2;",
            "loop(2, { v.a = v.c ?? 2; break; }); return 1;",
            "v.a = (v.c ?? { break; }) ?? 2; return v.never;",
            "loop(2, { v.x ? { break; }; });",
            "loop(2, { v.x && { continue; }; });",
            "for_each(t.e, c.arr, { v.x ? { break; }; });",
            "(v.c ?? 1) + 2",
        ] {
            assert!(modelled(src), "{src}");
        }
    }

    #[test]
    fn models_declines_a_jump_out_of_a_coalescing_left_side_inside_a_loop() {
        for src in [
            "loop(2, { v.a = (v.c ?? { break; }) ?? 2; }); return v.never;",
            "loop(2, { (v.c ?? { continue; }) ?? 2; }); return v.never;",
            "loop(2, { v.a = (v.c ?? { v.x ? { break; }; 1; }) ?? 2; }); return v.never;",
            "for_each(t.e, c.arr, { v.a = (v.c ?? { break; }) ?? 2; }); return v.never;",
        ] {
            assert!(!modelled(src), "{src}");
        }
    }

    #[test]
    fn models_declines_exactly_what_leaves_a_handler_for_a_loop_outside_it() {
        // Near misses that stay modelled: the jump is in the right side, in a loop of its own
        // inside the left side, in a query argument, or there is no loop around it.
        for src in [
            "loop(2, { v.a = v.c ?? { break; }; }); return v.never;",
            "loop(2, { v.a = (v.c ?? 1) + 1; break; }); return v.never;",
            "loop(2, { v.a = (loop(2, { break; }) ?? 1); }); return v.never;",
            "v.a = (v.c ?? { break; }) ?? 2; return v.never;",
            "loop(2, { v.a = (q.sum_test({ break; }) ?? 1); }); return v.never;",
        ] {
            assert!(modelled(src), "{src}");
        }
    }

    #[test]
    fn models_declines_jumps_with_an_operand_pending() {
        for src in [
            "loop(3, { v.t = v.k * (v.c ? {continue;} : 0); });",
            "loop(3, { v.t = v.k + (v.c ? {break;} : 0); });",
            "loop(3, { v.t = v.k - (v.c ? {break;} : 0); });",
            "loop(3, { v.t = math.max(v.k, (v.c ? {continue;} : 0)); });",
            "loop(3, { v.t = v.k < (v.c ? {break;} : 0); });",
            "loop(3, { v.t = v.k == (v.c ? {break;} : 0); });",
            "loop(2, { loop(3, { v.t = v.z * (v.c ? {continue;} : 0); }); });",
            "for_each(t.x, v.a, { loop(3, { v.t = v.k * (v.c ? {continue;} : 0); }); });",
        ] {
            assert!(!modelled(src), "{src}");
        }
    }

    #[test]
    fn models_accepts_jumps_where_nothing_is_pending() {
        for src in [
            // statement position, branches, conditions, assignment values, `&&` / `||` operands
            "loop(3, { v.c ? {continue;} : 0; });",
            "loop(3, { v.t = (v.c ? {break;} : 0); });",
            "loop(3, { v.c && {break;}; });",
            "loop(3, { v.c || {continue;}; });",
            "loop(2, { break; });",
            "loop(2, { v.x ? { break; } : { continue; }; });",
            "loop(2, { { { break; }; }; });",
            "loop(2, { return { break; }; });",
            "loop(3, { (v.c ? {break;} : 0) ? 1 : 2; });",
            // a `for_each` body is the walker's, whatever it has pending
            "for_each(t.x, v.a, { v.t = v.k * (v.c ? {continue;} : 0); });",
            "loop(3, { for_each(t.x, v.a, { v.t = v.k * (v.c ? {continue;} : 0); }); });",
            // a negation is a post-op: nothing is pushed
            "loop(3, { v.t = -(v.c ? {break;} : 0); });",
            // no loop: the jump ends the program
            "v.t = v.k * (v.c ? {break;} : 0);",
        ] {
            assert!(modelled(src), "{src}");
        }
    }

    #[test]
    fn models_looks_at_the_loop_body_not_at_what_the_loop_sits_in() {
        // Nothing is pending in a loop body even when the loop is an operand's neighbour: the count
        // of an inner loop is where an operand can be pending.
        assert!(modelled("loop(3, { loop(2, { v.t = v.k; break; }); });"));
        assert!(!modelled(
            "loop(3, { loop(2, { v.t = v.k * (v.c ? {break;} : 0); }); });"
        ));
    }

    #[test]
    fn jumps_with_pending_on_hand_built_nodes() {
        // A bare jump: pending only matters inside a loop.
        assert!(jumps_with_pending(&brk(), true, true));
        assert!(jumps_with_pending(&cont(), true, true));
        assert!(!jumps_with_pending(&brk(), false, true));
        assert!(!jumps_with_pending(&brk(), true, false));
        assert!(!jumps_with_pending(&number(1.0), true, true));
        // An op that pushes (`Add`) makes its children pending.
        let add = node(Op::Add, vec![number(1.0), brk()]);
        assert!(jumps_with_pending(&add, false, true));
        assert!(!jumps_with_pending(&add, false, false));
        // Ops that evaluate their children without pushing pass the flag through.
        for op in [
            Op::Semicolon,
            Op::LeftBrace,
            Op::Conditional,
            Op::ConditionalElse,
            Op::Assignment,
            Op::Return,
            Op::LogicalAnd,
            Op::LogicalOr,
            Op::NullCoalescing,
        ] {
            let wrapper = node(op, vec![number(1.0), brk()]);
            assert!(
                !jumps_with_pending(&wrapper, false, true),
                "{op:?} with nothing pending"
            );
            assert!(
                jumps_with_pending(&wrapper, true, true),
                "{op:?} with an operand pending"
            );
        }
        // A node with no children has nothing to find.
        assert!(!jumps_with_pending(&node(Op::Add, vec![]), true, true));
    }

    #[test]
    fn jumps_with_pending_resets_at_loops_for_each_and_queries() {
        // A loop body starts with nothing pending whatever the caller pushed; its count inherits.
        let in_body = node(Op::Loop, vec![number(2.0), brk()]);
        assert!(!jumps_with_pending(&in_body, true, false));
        assert!(!jumps_with_pending(&in_body, true, true));
        let in_count = node(Op::Loop, vec![brk(), number(1.0)]);
        assert!(jumps_with_pending(&in_count, true, true));
        assert!(!jumps_with_pending(&in_count, false, true));
        // The body of a loop is a loop body for a nested pending jump.
        let pending_in_body = node(
            Op::Loop,
            vec![number(2.0), node(Op::Add, vec![number(1.0), brk()])],
        );
        assert!(jumps_with_pending(&pending_in_body, false, false));
        // A `for_each` body is not a loop body of this analysis: even a pending jump in it is the
        // walker's.
        let each_body = node(
            Op::ForEach,
            vec![
                number(0.0),
                number(1.0),
                node(Op::Add, vec![number(1.0), brk()]),
            ],
        );
        assert!(!jumps_with_pending(&each_body, false, true));
        let each_count = node(
            Op::ForEach,
            vec![
                number(0.0),
                node(Op::Add, vec![number(1.0), brk()]),
                number(1.0),
            ],
        );
        assert!(jumps_with_pending(&each_count, false, true));
        // A query argument is a separate expression: neither pending nor in a loop.
        let query_arg = node(
            Op::QueryFunction,
            vec![node(Op::Add, vec![number(1.0), brk()]), brk()],
        );
        assert!(!jumps_with_pending(&query_arg, true, true));
        // A loop without children has nothing to find.
        assert!(!jumps_with_pending(&node(Op::Loop, vec![]), true, true));
    }

    #[test]
    fn jumps_out_of_handler_on_hand_built_nodes() {
        assert!(jumps_out_of_handler(&brk(), true, true));
        assert!(jumps_out_of_handler(&cont(), true, true));
        assert!(!jumps_out_of_handler(&brk(), false, true));
        assert!(!jumps_out_of_handler(&brk(), true, false));
        // Only the left side of a `??` is a handler.
        let left = node(Op::NullCoalescing, vec![brk(), number(1.0)]);
        let right = node(Op::NullCoalescing, vec![number(1.0), brk()]);
        assert!(jumps_out_of_handler(&left, false, true));
        assert!(!jumps_out_of_handler(&right, false, true));
        assert!(
            !jumps_out_of_handler(&left, false, false),
            "without a loop the jump ends the program"
        );
        // Any other node passes both flags on.
        for op in [
            Op::Add,
            Op::Semicolon,
            Op::Conditional,
            Op::LogicalAnd,
            Op::Assignment,
        ] {
            let wrapper = node(op, vec![number(1.0), brk()]);
            assert!(jumps_out_of_handler(&wrapper, true, true), "{op:?}");
            assert!(!jumps_out_of_handler(&wrapper, false, true), "{op:?}");
        }
        // The left side nests: a `??` inside the right side of another is not itself in a left
        // side.
        let nested = node(
            Op::NullCoalescing,
            vec![
                number(0.0),
                node(Op::NullCoalescing, vec![number(1.0), brk()]),
            ],
        );
        assert!(!jumps_out_of_handler(&nested, false, true));
        let nested_left = node(
            Op::NullCoalescing,
            vec![
                node(Op::NullCoalescing, vec![number(1.0), brk()]),
                number(0.0),
            ],
        );
        assert!(jumps_out_of_handler(&nested_left, false, true));
    }

    #[test]
    fn jumps_out_of_handler_starts_afresh_in_loop_bodies_and_query_arguments() {
        for op in [Op::Loop, Op::ForEach] {
            // The body is a new loop: a jump in it stays in it, even from inside a left side.
            let body = node(op, vec![number(1.0), number(1.0), brk()]);
            assert!(!jumps_out_of_handler(&body, true, true), "{op:?} body");
            // The count / array is still in the enclosing context.
            let count = node(op, vec![brk(), number(1.0), number(1.0)]);
            assert!(jumps_out_of_handler(&count, true, true), "{op:?} count");
            assert!(
                !jumps_out_of_handler(&count, false, true),
                "{op:?} count outside a left side"
            );
            // A handler inside the body that a jump leaves, for the body's own loop, is found.
            let left_in_body = node(
                op,
                vec![
                    number(1.0),
                    number(1.0),
                    node(Op::NullCoalescing, vec![brk(), number(0.0)]),
                ],
            );
            assert!(
                jumps_out_of_handler(&left_in_body, false, false),
                "{op:?} handler in the body"
            );
        }
        let argument = node(
            Op::QueryFunction,
            vec![node(Op::NullCoalescing, vec![brk(), number(0.0)])],
        );
        assert!(
            !jumps_out_of_handler(&argument, false, true),
            "no loop reaches into an argument"
        );
        let outer = node(Op::QueryFunction, vec![brk()]);
        assert!(!jumps_out_of_handler(&outer, true, true));
    }
}
