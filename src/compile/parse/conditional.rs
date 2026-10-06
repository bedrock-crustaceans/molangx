//! The conditional pass.

use super::driver::{Level, class, run};
use super::{Failed, Pass, with_children};
use crate::compile::{Cx, ast::Node};
use crate::diag::LanguageMessage as Msg;
use crate::ops::ExpressionOp as Op;
use std::mem::take;

/// The conditional pass (version ≥ 5): right-associative `c ? a : b` and `c ? a`, nested lists
/// first.
///
/// The rule is a scan from the right that folds `then : else` under the `:` when a `?` stands two
/// tokens to its left, folds `cond ? rhs` under the `?`, and restarts from the end after every `?`.
/// A restart can only newly fold a `:` directly after the folded `?` when another `?` now stands
/// two to its left (`a ? b ? c : d : e`), so one leftward scan with that check after each `?` fold
/// gives the same tree.
struct Ternary {
    list: Vec<Node>,
    /// The next node whose children are grouped.
    index: usize,
}

impl Level for Ternary {
    fn step(&mut self, _: &mut Cx<'_>) -> Result<Option<Vec<Node>>, Failed> {
        while let Some(node) = self.list.get_mut(self.index) {
            if !node.is_leaf() {
                return Ok(Some(take(&mut node.children)));
            }
            self.index += 1;
        }
        Ok(None)
    }

    fn resume(&mut self, children: Vec<Node>, _: bool) {
        if let Some(node) = self.list.get_mut(self.index) {
            node.children = children;
        }
        self.index += 1;
    }

    fn finish(self, cx: &mut Cx<'_>) -> Result<(Vec<Node>, bool), Failed> {
        let list = fold_conditionals(self.list);
        if let Some(dangling) = list
            .iter()
            .find(|n| is_token(n, Op::Conditional) || is_token(n, Op::ConditionalElse))
        {
            cx.language(
                Msg::TernaryWithoutOperands,
                dangling.span,
                &[&cx.friendly(dangling)],
            );
            return Err(Failed);
        }
        Ok((list, false))
    }
}

/// Whether `node` is an `op` token nothing is grouped under yet.
fn is_token(node: &Node, op: Op) -> bool {
    node.is_leaf() && node.is(op)
}

/// The scan of [`Ternary`]: `list` is consumed from the right onto `right`, nearest on top.
fn fold_conditionals(mut list: Vec<Node>) -> Vec<Node> {
    if list.len() < 3 {
        return list;
    }
    let mut right: Vec<Node> = Vec::with_capacity(list.len());
    right.extend(list.pop());
    while list.len() >= 2 {
        match list.as_slice() {
            [.., question, _, colon]
                if is_token(colon, Op::ConditionalElse) && is_token(question, Op::Conditional) =>
            {
                if let (Some(colon), Some(then), Some(otherwise)) =
                    (list.pop(), list.pop(), right.pop())
                {
                    right.push(with_children(colon, [then, otherwise]));
                }
            }
            [.., question] if is_token(question, Op::Conditional) => {
                if let (Some(question), Some(condition), Some(then)) =
                    (list.pop(), list.pop(), right.pop())
                {
                    right.push(with_children(question, [condition, then]));
                }
                // Instead of a restart: `? <folded> : x` is now an else-pair for the `?` on the
                // left.
                if list.last().is_some_and(|n| is_token(n, Op::Conditional))
                    && let [.., _, colon, _] = right.as_slice()
                    && is_token(colon, Op::ConditionalElse)
                    && let (Some(then), Some(colon), Some(otherwise)) =
                        (right.pop(), right.pop(), right.pop())
                {
                    right.push(with_children(colon, [then, otherwise]));
                }
            }
            _ => right.extend(list.pop()),
        }
    }
    right.extend(list.pop());
    right.reverse();
    right
}

/// The conditional pass.
pub(super) fn ternary(cx: &mut Cx<'_>, list: &mut Vec<Node>) -> Pass {
    run(
        cx,
        list,
        |list| Ternary { list, index: 0 },
        class(Op::Conditional),
    )
}

#[cfg(test)]
mod tests {
    use crate::compile::parse::test_support::*;

    #[test]
    fn a_conditional_takes_a_condition_and_an_else_pair() {
        assert_eq!(
            passes("ternary", "v.a ? v.b : v.c"),
            "(Conditional variable.a (ConditionalElse variable.b variable.c))"
        );
    }

    #[test]
    fn a_conditional_without_else_takes_two_operands() {
        assert_eq!(
            passes("ternary", "v.a ? v.b"),
            "(Conditional variable.a variable.b)"
        );
    }

    #[test]
    fn conditionals_group_to_the_right() {
        assert_eq!(
            passes("ternary", "v.a ? v.b : v.c ? v.d : v.e"),
            "(Conditional variable.a (ConditionalElse variable.b (Conditional variable.c (ConditionalElse variable.d variable.e))))"
        );
        assert_eq!(
            passes("ternary", "v.a ? v.b : v.c ? v.d"),
            "(Conditional variable.a (ConditionalElse variable.b (Conditional variable.c variable.d)))"
        );
    }

    #[test]
    fn a_nested_then_branch_closes_with_the_right_colon() {
        assert_eq!(
            passes("ternary", "v.a ? v.b ? v.c : v.d : v.e"),
            "(Conditional variable.a (ConditionalElse (Conditional variable.b (ConditionalElse variable.c variable.d)) variable.e))"
        );
        assert_eq!(
            passes("ternary", "v.a ? v.b ? v.c ? v.d : v.e : v.f : v.g"),
            "(Conditional variable.a (ConditionalElse (Conditional variable.b (ConditionalElse (Conditional variable.c (ConditionalElse variable.d variable.e)) variable.f)) variable.g))"
        );
    }

    #[test]
    fn a_conditional_inside_a_section_is_grouped_first() {
        assert_eq!(
            passes("sections+ternary", "(v.a ? v.b : v.c)"),
            "(LeftParenthesis (Conditional variable.a (ConditionalElse variable.b variable.c)))"
        );
    }

    #[test]
    fn a_dangling_question_mark_or_colon_is_an_error() {
        assert_eq!(
            pass_error("ternary", "v.a ?"),
            [(
                "E19",
                (4, 5),
                "Error: could not find sub-expressions for Conditional '?' operator\n".to_owned()
            )]
        );
        assert_eq!(
            pass_error("ternary", "v.a : v.b"),
            [(
                "E19",
                (4, 5),
                "Error: could not find sub-expressions for Conditional Else ':' operator\n"
                    .to_owned()
            )]
        );
        assert_eq!(pass_error("ternary", "?")[0].1, (0, 1));
        assert_eq!(pass_error("ternary", "v.a ? v.b :")[0].1, (10, 11));
    }

    #[test]
    fn a_folded_question_mark_pairs_a_colon_only_with_a_question_mark_on_its_left() {
        let log = pass_error("ternary", "v.a v.b ? v.c : v.d : v.e");
        assert_eq!(ids(&log), ["E19"]);
        assert_eq!(log[0].1, (20, 21));
    }

    #[test]
    fn a_colon_just_right_of_a_folded_conditional_closes_the_outer_question_mark() {
        // The `:` before `v.e` closes the outer `?`; the last colon is the stray one.
        let log = pass_error("ternary", "v.a ? v.b ? v.c : v.d : v.e : v.f");
        assert_eq!(ids(&log), ["E19"]);
        assert_eq!(log[0].1, (28, 29));
    }

    #[test]
    fn only_the_leafness_of_that_colon_counts_not_the_middle_operand() {
        let log = pass_error("sections+ternary", "v.a ? v.b ? v.c : v.d : (v.e) : v.f");
        assert_eq!(ids(&log), ["E19"]);
        assert_eq!(log[0].1, (30, 31));
    }

    #[test]
    fn a_colon_straight_after_the_question_mark_is_left_for_later() {
        assert_eq!(
            passes("ternary", "v.a ? : v.b"),
            "(Conditional variable.a ConditionalElse) | variable.b"
        );
    }

    mod tree_shapes {
        use crate::compile::test_support::pipeline::{messages_at, tree, tree_at};

        #[test]
        fn conditional_bands() {
            for version in [-1, 0, 4] {
                assert_eq!(
                    tree_at("v.a?v.b:v.c?v.d:v.e", version),
                    "(Conditional (Conditional v.a v.b v.c) v.d v.e)",
                    "version {version}"
                );
                assert_eq!(
                    messages_at("v.a?v.b?v.c:v.d:v.e", version),
                    ["Unsupported Conditional Else ':' operator in expression optimization"],
                    "version {version}"
                );
                assert_eq!(
                    messages_at("1 ?", version),
                    ["Error: binary Conditional '?' operator at end of expression"],
                    "version {version}"
                );
                assert_eq!(
                    messages_at(": 1", version),
                    ["Error: binary Conditional Else ':' operator at end of expression"],
                    "version {version}"
                );
            }
            for version in [5, 13] {
                assert_eq!(
                    tree_at("v.a?v.b:v.c?v.d:v.e", version),
                    "(Conditional v.a v.b (Conditional v.c v.d v.e))",
                    "version {version}"
                );
                assert_eq!(
                    tree_at("v.a?v.b?v.c:v.d:v.e", version),
                    "(Conditional v.a (Conditional v.b v.c v.d) v.e)",
                    "version {version}"
                );
                assert_eq!(
                    messages_at("1 ?", version),
                    ["Error: could not find sub-expressions for Conditional '?' operator"],
                    "version {version}"
                );
                assert_eq!(
                    messages_at(": 1", version),
                    ["Error: could not find sub-expressions for Conditional Else ':' operator"],
                    "version {version}"
                );
            }
            assert_eq!(
                tree("v.a ? v.b"),
                "(Conditional v.a v.b)",
                "the else branch is optional"
            );
            assert_eq!(
                tree("v.a||v.b?v.c:v.d"),
                "(Conditional (LogicalOr v.a v.b) v.c v.d)"
            );
            assert_eq!(
                tree("v.a ? v.b ? v.c ? v.d : v.e : v.f : v.g"),
                "(Conditional v.a (Conditional v.b (Conditional v.c v.d v.e) v.f) v.g)"
            );
            assert_eq!(
                tree("v.a ? v.b : v.c ? v.d ? v.e : v.f : v.g"),
                "(Conditional v.a v.b (Conditional v.c (Conditional v.d v.e v.f) v.g))"
            );
        }
    }
}
