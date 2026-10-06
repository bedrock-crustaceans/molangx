//! The bracket structure: member accessors and sections.

use super::driver::{TokenClasses, classes_at};
use super::{Failed, Pass, with_children};
use crate::compile::{Cx, ast::Node};
use crate::diag::LanguageMessage as Msg;
use crate::ops::ExpressionOp as Op;
use std::mem::take;

/// The member-accessor pass: `X .name` becomes `MemberAccessor(X)` when `X` is a variable, a
/// query or another accessor. Runs first, on the flat list, and notes [`Node::below`].
pub(super) fn member_accessors(cx: &mut Cx<'_>, list: &mut Vec<Node>) -> Pass {
    let old = take(list);
    list.reserve(old.len());
    for mut node in old {
        if node.is(Op::MemberAccessor) {
            if list.is_empty() {
                cx.language(Msg::LeadingMemberAccessor, node.span, &[]);
                return Err(Failed);
            }
            if let Some(base) = list.pop_if(|previous| {
                matches!(
                    previous.op,
                    Op::QueryFunction
                        | Op::ContextVariable
                        | Op::EntityVariable
                        | Op::TempVariable
                        | Op::MemberAccessor
                )
            }) {
                node.below = classes_at(&base);
                node = with_children(node, [base]);
            }
        }
        list.push(node);
    }
    Ok(())
}

const fn closing_of(op: Op) -> Option<Op> {
    match op {
        Op::LeftBrace => Some(Op::RightBrace),
        Op::LeftBracket => Some(Op::RightBracket),
        Op::LeftParenthesis => Some(Op::RightParenthesis),
        _ => None,
    }
}

/// A section whose closer has not been seen yet.
struct Open {
    node: Node,
    closing: Op,
    /// The opener's index plus one, the position the messages report.
    start: usize,
    /// The classes of the tokens in the section so far.
    below: TokenClasses,
}

impl Open {
    fn log_not_closed(&self, cx: &mut Cx<'_>) {
        cx.language(
            Msg::SectionNotClosed,
            self.node.span,
            &[&self.closing.friendly_name(), &cx.friendly(&self.node)],
        );
    }
}

/// Appends `node` to the innermost open section, or to `list` outside any.
fn place(open: &mut [Open], list: &mut Vec<Node>, node: Node) {
    match open.last_mut() {
        Some(parent) => {
            parent.below |= classes_at(&node);
            parent.node.children.push(node);
        }
        None => list.push(node),
    }
}

/// The section pass: each opener takes everything up to its matching closer as its children. A
/// stray closer outside any section stays in the list.
///
/// Message indices are positions in the list as it is when a top-level section is looked for:
/// earlier top-level sections collapsed, the rest flat. Each section notes [`Node::below`].
pub(super) fn sections(cx: &mut Cx<'_>, list: &mut Vec<Node>) -> Pass {
    let old = take(list);
    let mut open: Vec<Open> = Vec::new();
    let mut index = 0;
    for node in old {
        index = if open.is_empty() {
            list.len()
        } else {
            index + 1
        };
        if let Some(closing) = closing_of(node.op) {
            open.push(Open {
                node,
                closing,
                start: index + 1,
                below: TokenClasses::NONE,
            });
        } else if let op @ (Op::RightBrace | Op::RightBracket | Op::RightParenthesis) = node.op
            && let Some(section) = open.pop()
        {
            if section.closing != op {
                cx.language(
                    Msg::ClosingMismatch,
                    section.node.span.to(node.span),
                    &[
                        &section.start,
                        &cx.friendly(&section.node),
                        &section.closing.friendly_name(),
                        &op.friendly_name(),
                        &index,
                    ],
                );
                open.first().unwrap_or(&section).log_not_closed(cx);
                return Err(Failed);
            }
            let mut closed = section.node;
            closed.below = section.below;
            place(&mut open, list, closed);
        } else {
            place(&mut open, list, node);
        }
    }
    if let (Some(outer), Some(inner)) = (open.first(), open.last()) {
        cx.language(
            Msg::NoClosingSymbol,
            inner.node.span,
            &[
                &inner.start,
                &cx.friendly(&inner.node),
                &inner.closing.friendly_name(),
            ],
        );
        outer.log_not_closed(cx);
        return Err(Failed);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compile::ast::Span;

    use crate::compile::parse::test_support::*;

    #[test]
    fn only_the_three_openers_have_a_closer() {
        assert_eq!(closing_of(Op::LeftBrace), Some(Op::RightBrace));
        assert_eq!(closing_of(Op::LeftBracket), Some(Op::RightBracket));
        assert_eq!(closing_of(Op::LeftParenthesis), Some(Op::RightParenthesis));
        for &op in Op::all() {
            if !matches!(op, Op::LeftBrace | Op::LeftBracket | Op::LeftParenthesis) {
                assert_eq!(closing_of(op), None, "{op:?}");
            }
        }
    }

    #[test]
    fn an_accessor_takes_the_variable_before_it() {
        assert_eq!(passes("member", "v.x.y"), "(.y variable.x)");
        assert_eq!(passes("member", "t.a.b"), "(.b temp.a)");
        assert_eq!(passes("member", "c.a.b"), "(.b context.a)");
    }

    #[test]
    fn an_accessor_takes_a_query_before_it() {
        assert_eq!(passes("member", "q.is_baby.x"), "(.x query.is_baby)");
    }

    #[test]
    fn accessors_chain_into_nested_nodes() {
        assert_eq!(passes("member", "c.a.b.c"), "(.c (.b context.a))");
        assert_eq!(passes("member", "v.a.b.c.d"), "(.d (.c (.b variable.a)))");
    }

    #[test]
    fn an_accessor_after_anything_else_stays_a_flat_leaf() {
        assert_eq!(passes("member", "1 .x"), "1 | .x");
        assert_eq!(
            passes("member", "(v.x).y"),
            "LeftParenthesis | variable.x | RightParenthesis | .y"
        );
        assert_eq!(passes("member", "array.a.x"), "array.a | .x");
        assert_eq!(passes("member", "geometry.a .x"), "geometry.a | .x");
    }

    #[test]
    fn a_leading_accessor_is_an_error() {
        assert_eq!(
            pass_error("member", ".x"),
            [(
                "E09",
                (0, 2),
                "Error: cannot start an expression with a member accessor; member accessors require a variable of which to access a member.".to_owned()
            )]
        );
    }

    #[test]
    fn an_accessor_keeps_its_own_span_and_the_base_becomes_its_child() {
        let list = grouped("member", "v.x.yz");
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].span, Span::new(3, 6));
        assert_eq!(list[0].children[0].span, Span::new(0, 3));
        assert_eq!(list[0].full_span(), Span::new(0, 6));
    }

    #[test]
    fn a_section_takes_everything_up_to_its_closer() {
        assert_eq!(passes("sections", "(1)"), "(LeftParenthesis 1)");
        assert_eq!(passes("sections", "{1}"), "(LeftBrace 1)");
        assert_eq!(passes("sections", "[1]"), "(LeftBracket 1)");
        assert_eq!(passes("sections", "(1 + 2)"), "(LeftParenthesis 1 Add 2)");
    }

    #[test]
    fn sections_nest() {
        assert_eq!(
            passes("sections", "((1))"),
            "(LeftParenthesis (LeftParenthesis 1))"
        );
        assert_eq!(
            passes("sections", "( [ { 1 } ] )"),
            "(LeftParenthesis (LeftBracket (LeftBrace 1)))"
        );
        assert_eq!(
            passes("sections", "(1 (2) 3)"),
            "(LeftParenthesis 1 (LeftParenthesis 2) 3)"
        );
    }

    #[test]
    fn sibling_sections_stay_side_by_side() {
        assert_eq!(
            passes("sections", "(1) (2) [3]"),
            "(LeftParenthesis 1) | (LeftParenthesis 2) | (LeftBracket 3)"
        );
        assert_eq!(passes("sections", "1 (2) 3"), "1 | (LeftParenthesis 2) | 3");
    }

    #[test]
    fn an_empty_section_is_a_leaf_opener() {
        assert_eq!(passes("sections", "()"), "LeftParenthesis");
        assert_eq!(passes("sections", "{ }"), "LeftBrace");
    }

    #[test]
    fn a_stray_closer_stays_in_the_list() {
        assert_eq!(passes("sections", "1)"), "1 | RightParenthesis");
        assert_eq!(
            passes("sections", "(1) ]"),
            "(LeftParenthesis 1) | RightBracket"
        );
        assert_eq!(passes("sections", "}"), "RightBrace");
    }

    #[test]
    fn an_opener_without_a_closer_reports_two_messages() {
        let log = pass_error("sections", "(1");
        assert_eq!(
            log,
            [
                (
                    "E10",
                    (0, 1),
                    "Unable to find matching closing section symbol for symbol at 1(Left Parenthesis '(') -- looking for Right Parenthesis ')'".to_owned()
                ),
                (
                    "E12",
                    (0, 1),
                    "Error: Could not find Right Parenthesis ')' to close section started with Left Parenthesis '('\n".to_owned()
                ),
            ]
        );
    }

    #[test]
    fn the_unclosed_message_names_the_innermost_opener_and_the_section_the_outermost() {
        let log = pass_error("sections", "{ ( 1");
        assert_eq!(log[0].0, "E10");
        assert_eq!(log[0].1, (2, 3));
        assert!(
            log[0].2.contains("at 2(Left Parenthesis '(')"),
            "{}",
            log[0].2
        );
        assert_eq!(log[1].0, "E12");
        assert_eq!(log[1].1, (0, 1));
        assert!(log[1].2.contains("Right Brace '}'"), "{}", log[1].2);
    }

    #[test]
    fn a_wrong_closer_reports_the_mismatch_and_the_unclosed_section() {
        let log = pass_error("sections", "(1]");
        assert_eq!(
            log,
            [
                (
                    "E11",
                    (0, 3),
                    "Unable to match closing section symbol at 1(Left Parenthesis '(') - looking for Right Parenthesis ')', found Right Bracket ']' at 2".to_owned()
                ),
                (
                    "E12",
                    (0, 1),
                    "Error: Could not find Right Parenthesis ')' to close section started with Left Parenthesis '('\n".to_owned()
                ),
            ]
        );
    }

    #[test]
    fn a_mismatch_inside_nested_sections_names_the_outermost_unclosed_one() {
        // `[` is closed by `)`: the sections still open below it are `{` and `(`; the note is about
        // `{`.
        let log = pass_error("sections", "{ ( [ )");
        assert_eq!(
            log,
            [
                (
                    "E11",
                    (4, 7),
                    "Unable to match closing section symbol at 3(Left Bracket '[') - looking for Right Bracket ']', found Right Parenthesis ')' at 3".to_owned()
                ),
                ("E12", (0, 1), "Error: Could not find Right Brace '}' to close section started with Left Brace '{'\n".to_owned()),
            ]
        );
    }

    #[test]
    fn mismatch_indices_count_positions_in_the_list_as_it_is_when_the_scan_runs() {
        // The first section has collapsed to one node, so the second opener is at 2 and its closer
        // at 3.
        let log = pass_error("sections", "(1) (2]");
        assert_eq!(log[0].0, "E11");
        assert_eq!(log[0].1, (4, 7));
        assert!(
            log[0].2.contains("at 2(Left Parenthesis '(')")
                && log[0].2.ends_with("found Right Bracket ']' at 3"),
            "{}",
            log[0].2
        );
    }

    #[test]
    fn a_nested_mismatch_names_the_inner_opener_and_the_outer_section() {
        let log = pass_error("sections", "{ ( ] }");
        assert_eq!(log[0].0, "E11");
        assert_eq!(log[0].1, (2, 5));
        assert!(log[0].2.contains("Left Parenthesis '('"), "{}", log[0].2);
        assert_eq!(log[1].0, "E12");
        assert_eq!(log[1].1, (0, 1));
        assert!(
            log[1].2.contains("Right Brace '}'") && log[1].2.contains("Left Brace '{'"),
            "{}",
            log[1].2
        );
    }

    #[test]
    fn a_closer_of_the_wrong_kind_inside_two_levels_reports_once() {
        assert_eq!(ids(&pass_error("sections", "( [ ) ]")), ["E11", "E12"]);
        assert_eq!(ids(&pass_error("sections", "((")), ["E10", "E12"]);
    }

    mod tree_shapes {
        use crate::compile::test_support::pipeline::{tree, tree_at};

        #[test]
        fn structure_passes() {
            assert_eq!(tree("v.x.y+1"), "[(MemberAccessor v.x)*1+1]");
            assert_eq!(tree("[1]"), "1");
            assert_eq!(tree("((1))"), "1");
            assert_eq!(tree("array.foo[v.x]"), "(Array v.x)");
            assert_eq!(tree("array.foo"), "41");
        }

        /// Below version 4 a `( )` or `[ ]` section with several children is its first child.
        #[test]
        fn sections_with_several_children() {
            for version in [0, 2, 3] {
                assert_eq!(tree_at("(1 2)", version), "1", "version {version}");
                assert_eq!(tree_at("[1 2]", version), "1", "version {version}");
            }
        }
    }
}
