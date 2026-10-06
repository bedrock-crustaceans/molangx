//! Calls, math functions and array indices: the passes that attach a section to the token before.

use super::driver::{Compact, Found, Level, class, run};
use super::{Failed, Pass, is_math_call, with_children};
use crate::compile::{Cx, ast::Node};
use crate::diag::LanguageMessage as Msg;
use crate::ops::ExpressionOp as Op;
use std::iter::Peekable;
use std::mem::take;
use std::vec::IntoIter;

/// What a call-like token takes as its argument section.
#[derive(Copy, Clone)]
enum CallKind {
    /// A query, `loop` or `for_each`; an empty `()` is an error.
    QueryOrLoop,
    /// A math function; the `(` section is required.
    Math,
}

impl CallKind {
    fn calls(self, node: &Node) -> bool {
        match self {
            Self::QueryOrLoop => {
                matches!(node.op, Op::QueryFunction | Op::Loop | Op::ForEach)
            }
            Self::Math => is_math_call(node.op),
        }
    }
}

/// The call passes: a call token directly followed by a `(` section takes it as its child. The
/// list is rebuilt in place ([`Compact`]).
struct Calls {
    kind: CallKind,
    list: Compact,
}

impl Level for Calls {
    fn step(&mut self, cx: &mut Cx<'_>) -> Result<Option<Vec<Node>>, Failed> {
        let kind = self.kind;
        while let Some(found) = self.list.next_token(|node| kind.calls(node)) {
            let node = match found {
                Found::Children(children) => return Ok(Some(children)),
                Found::Token(node) => node,
            };
            match (kind, self.list.next_if(|next| next.is(Op::LeftParenthesis))) {
                (CallKind::QueryOrLoop, Some(arguments)) if arguments.is_leaf() => {
                    cx.language(
                        Msg::EmptyParameterList,
                        node.span.to(arguments.span),
                        &[&cx.friendly(&node)],
                    );
                    return Err(Failed);
                }
                (_, Some(arguments)) => {
                    let call = with_children(node, [arguments]);
                    return Ok(Some(self.list.push_entered(call)));
                }
                (CallKind::QueryOrLoop, None) => {
                    self.list.push(node);
                }
                (CallKind::Math, None) => {
                    let message = if self.list.peek().is_none() {
                        Msg::MathAtEnd
                    } else {
                        Msg::MathWithoutParenthesis
                    };
                    cx.language(message, node.span, &[&cx.friendly(&node)]);
                    return Err(Failed);
                }
            }
        }
        Ok(None)
    }

    fn resume(&mut self, children: Vec<Node>, _: bool) {
        self.list.resume(children);
    }

    fn finish(self, _: &mut Cx<'_>) -> Result<(Vec<Node>, bool), Failed> {
        Ok((self.list.finish(), false))
    }
}

/// The array pass: `array.name` followed by `[ … ]` sections becomes `Array` with the contents of
/// the sections as its children.
///
/// Only each new section's contents are walked (with the last child so far in front when it is a
/// bare `array.name` they can complete) and appended, so a chain of sections stays linear.
struct Arrays {
    rest: Peekable<IntoIter<Node>>,
    out: Vec<Node>,
    /// An `Array` node that may take further `[ … ]` sections.
    array: Option<Node>,
    entered: Option<Entered>,
}

/// A node of [`Arrays`] whose children are out.
enum Entered {
    /// A node of the list; its children come back whole.
    Node(Node),
    /// An `Array` node; the contents of its next section come back, to be appended.
    Array(Node),
}

impl Arrays {
    fn new(list: Vec<Node>) -> Self {
        Self {
            rest: list.into_iter().peekable(),
            out: Vec::new(),
            array: None,
            entered: None,
        }
    }
}

impl Level for Arrays {
    fn step(&mut self, cx: &mut Cx<'_>) -> Result<Option<Vec<Node>>, Failed> {
        loop {
            if let Some(mut node) = self.array.take() {
                if let Some(mut index) = self.rest.next_if(|next| next.is(Op::LeftBracket)) {
                    if index.is_leaf() {
                        cx.language(Msg::EmptyArrayIndex, node.span.to(index.span), &[]);
                        return Err(Failed);
                    }
                    let mut contents = take(&mut index.children);
                    if node
                        .children
                        .last()
                        .is_some_and(|n| n.is(Op::ArrayVariable) && n.is_leaf())
                        && contents.first().is_some_and(|n| n.is(Op::LeftBracket))
                        && let Some(name) = node.children.pop()
                    {
                        contents.insert(0, name);
                    }
                    self.entered = Some(Entered::Array(node));
                    return Ok(Some(contents));
                }
                self.out.push(node);
                continue;
            }
            let Some(mut node) = self.rest.next() else {
                return Ok(None);
            };
            if !node.is_leaf() {
                let children = take(&mut node.children);
                self.entered = Some(Entered::Node(node));
                return Ok(Some(children));
            }
            if node.is(Op::ArrayVariable)
                && self
                    .rest
                    .peek()
                    .is_some_and(|next| next.is(Op::LeftBracket))
            {
                node.op = Op::Array;
                self.array = Some(node);
                continue;
            }
            self.out.push(node);
        }
    }

    fn resume(&mut self, children: Vec<Node>, _: bool) {
        match self.entered.take() {
            Some(Entered::Node(mut node)) => {
                node.children = children;
                self.out.push(node);
            }
            Some(Entered::Array(mut node)) => {
                if node.children.is_empty() {
                    node.children = children;
                } else {
                    node.children.extend(children);
                }
                self.array = Some(node);
            }
            None => {}
        }
    }

    fn finish(self, _: &mut Cx<'_>) -> Result<(Vec<Node>, bool), Failed> {
        Ok((self.out, false))
    }
}

/// The pass that groups queries, `loop` and `for_each` with their argument sections.
pub(super) fn calls(cx: &mut Cx<'_>, list: &mut Vec<Node>) -> Pass {
    run(
        cx,
        list,
        |list| Calls {
            kind: CallKind::QueryOrLoop,
            list: Compact::new(list),
        },
        class(Op::QueryFunction),
    )
}

/// The pass that groups math functions with their argument sections.
pub(super) fn math_functions(cx: &mut Cx<'_>, list: &mut Vec<Node>) -> Pass {
    run(
        cx,
        list,
        |list| Calls {
            kind: CallKind::Math,
            list: Compact::new(list),
        },
        class(Op::Sin),
    )
}

/// The pass that groups `array.name[…]`.
pub(super) fn arrays(cx: &mut Cx<'_>, list: &mut Vec<Node>) -> Pass {
    run(cx, list, Arrays::new, class(Op::ArrayVariable))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compile::ast::Span;

    use crate::compile::parse::test_support::*;

    #[test]
    fn a_query_takes_the_parenthesis_section_after_it() {
        assert_eq!(
            passes("sections+calls", "q.is_baby(1)"),
            "(query.is_baby (LeftParenthesis 1))"
        );
        assert_eq!(
            passes("sections+calls", "q.is_baby (1)"),
            "(query.is_baby (LeftParenthesis 1))"
        );
    }

    #[test]
    fn a_query_without_parentheses_is_left_alone() {
        assert_eq!(passes("sections+calls", "q.is_baby"), "query.is_baby");
        assert_eq!(
            passes("sections+calls", "q.is_baby + 1"),
            "query.is_baby | Add | 1"
        );
    }

    #[test]
    fn a_query_with_empty_parentheses_is_an_error() {
        assert_eq!(
            pass_error("sections+calls", "q.is_baby()"),
            [(
                "E13",
                (0, 10),
                "Error: Query Function 'query.' or 'q.' operators with no params should not use parentheses\n".to_owned()
            )]
        );
        assert_eq!(pass_error("sections+calls", "loop()")[0].1, (0, 5));
    }

    #[test]
    fn loop_and_for_each_take_their_parenthesis_section() {
        assert_eq!(
            passes("sections+calls", "loop(3, {})"),
            "(Loop (LeftParenthesis 3 Comma LeftBrace))"
        );
        assert_eq!(
            passes("sections+calls", "for_each(t.x, v.a, {})"),
            "(ForEach (LeftParenthesis temp.x Comma variable.a Comma LeftBrace))"
        );
    }

    #[test]
    fn calls_are_found_inside_sections_and_arguments() {
        assert_eq!(
            passes("sections+calls", "(q.is_baby(1))"),
            "(LeftParenthesis (query.is_baby (LeftParenthesis 1)))"
        );
        assert_eq!(
            passes("sections+calls", "q.is_baby(q.is_baby(2))"),
            "(query.is_baby (LeftParenthesis (query.is_baby (LeftParenthesis 2))))"
        );
        assert_eq!(
            passes("sections+calls", "{q.is_baby(1)}"),
            "(LeftBrace (query.is_baby (LeftParenthesis 1)))"
        );
    }

    #[test]
    fn a_math_call_is_not_touched_by_the_call_pass() {
        assert_eq!(
            passes("sections+calls", "math.abs(1)"),
            "Abs | (LeftParenthesis 1)"
        );
    }

    #[test]
    fn a_math_function_takes_the_parenthesis_section_after_it() {
        assert_eq!(
            passes("sections+math", "math.abs(1)"),
            "(Abs (LeftParenthesis 1))"
        );
        assert_eq!(
            passes("sections+math", "math.abs(math.abs(1))"),
            "(Abs (LeftParenthesis (Abs (LeftParenthesis 1))))"
        );
        assert_eq!(
            passes("sections+math", "(math.abs(1))"),
            "(LeftParenthesis (Abs (LeftParenthesis 1)))"
        );
        assert_eq!(
            passes("sections+math", "{math.abs(1)}"),
            "(LeftBrace (Abs (LeftParenthesis 1)))"
        );
    }

    #[test]
    fn math_pi_is_a_constant_not_a_call() {
        assert_eq!(passes("sections+math", "math.pi"), "Pi");
        assert_eq!(passes("sections+math", "math.pi + 1"), "Pi | Add | 1");
    }

    #[test]
    fn a_math_function_with_empty_parentheses_is_accepted_by_the_pass() {
        assert_eq!(
            passes("sections+math", "math.abs()"),
            "(Abs LeftParenthesis)"
        );
    }

    #[test]
    fn a_math_function_at_the_end_has_its_own_message() {
        assert_eq!(
            pass_error("sections+math", "math.abs"),
            [(
                "E17",
                (0, 8),
                "Error: Absolute Value 'math.abs' operator at end of expression without a parenthesis section\n".to_owned()
            )]
        );
    }

    #[test]
    fn a_math_function_not_followed_by_parentheses_has_its_own_message() {
        assert_eq!(
            pass_error("sections+math", "math.abs + 1"),
            [(
                "E17",
                (0, 8),
                "Error: Absolute Value 'math.abs' operator not followed by parenthesis section\n"
                    .to_owned()
            )]
        );
        assert_eq!(ids(&pass_error("sections+math", "math.abs 1")), ["E17"]);
        assert_eq!(
            pass_error("sections+math", "1 + math.max [1]")[0].1,
            (4, 12)
        );
    }

    #[test]
    fn an_array_variable_followed_by_brackets_becomes_an_array_node() {
        assert_eq!(
            passes("sections+arrays", "array.a[v.x]"),
            "(Array variable.x)"
        );
        assert_eq!(
            passes("sections+arrays", "array.a [v.x]"),
            "(Array variable.x)"
        );
        assert_eq!(
            passes("sections+arrays", "array.a[1 + 2]"),
            "(Array 1 Add 2)"
        );
    }

    #[test]
    fn array_sections_in_a_row_append_their_contents() {
        assert_eq!(passes("sections+arrays", "array.a[1][2]"), "(Array 1 2)");
        assert_eq!(
            passes("sections+arrays", "array.a[1] [2] [3]"),
            "(Array 1 2 3)"
        );
    }

    #[test]
    fn array_indices_nest() {
        assert_eq!(
            passes("sections+arrays", "array.a[array.b[1]]"),
            "(Array (Array 1))"
        );
        assert_eq!(
            passes("sections+arrays", "(array.a[1])"),
            "(LeftParenthesis (Array 1))"
        );
    }

    #[test]
    fn an_array_variable_without_brackets_is_left_alone() {
        assert_eq!(passes("sections+arrays", "array.a"), "array.a");
        assert_eq!(
            passes("sections+arrays", "array.a + 1"),
            "array.a | Add | 1"
        );
    }

    #[test]
    fn spaced_empty_brackets_are_an_error() {
        assert_eq!(
            pass_error("sections+arrays", "array.a[ ]"),
            [(
                "E15",
                (0, 8),
                "Error: array expression is empty\n".to_owned()
            )]
        );
    }

    #[test]
    fn the_bracket_pair_without_a_space_is_one_array_token() {
        // `[]` is an operator token of its own.
        assert_eq!(passes("sections+arrays", "array.a[]"), "array.a | Array");
    }

    #[test]
    fn the_array_op_is_rewritten_in_place() {
        let list = grouped("sections+arrays", "array.a[1]");
        assert_eq!(list.len(), 1);
        assert!(list[0].is(Op::Array));
        assert_eq!(list[0].span, Span::new(0, 7));
    }

    mod tree_shapes {
        use crate::compile::test_support::pipeline::tree;

        #[test]
        fn calls_take_their_argument_section() {
            assert_eq!(tree("q.count(1, 2)"), tree("query.count(1, 2)"));
        }
    }
}
