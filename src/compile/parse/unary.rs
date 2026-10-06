//! The prefix operators: unary minus and not, and `return`.

use super::driver::{Compact, Found, Level, class, enter, run};
use super::{Failed, Pass, with_children};
use crate::compile::{
    Cx,
    ast::{Node, Payload},
};
use crate::diag::LanguageMessage as Msg;
use crate::ops::ExpressionOp as Op;

/// What the token before a `-` makes of it.
enum BeforeMinus {
    /// An operand: the `-` is binary.
    Operand,
    /// An operator: the `-` is unary.
    Operator,
    /// Another `-`: the pair becomes `+`.
    Minus,
    /// `continue` or an expression array: the `-` is an unknown operation.
    Unknown,
}

fn before_minus(op: Op) -> BeforeMinus {
    match op {
        Op::Negate => BeforeMinus::Minus,
        Op::Continue | Op::ExpressionArray => BeforeMinus::Unknown,
        Op::LeftBrace
        | Op::RightBrace
        | Op::LogicalNot
        | Op::Add
        | Op::Div
        | Op::Mul
        | Op::LessThan
        | Op::LessEqual
        | Op::GreaterEqual
        | Op::GreaterThan
        | Op::LogicalEqual
        | Op::LogicalNotEqual
        | Op::LogicalOr
        | Op::LogicalAnd
        | Op::NullCoalescing
        | Op::Conditional
        | Op::ConditionalElse
        | Op::Loop
        | Op::ForEach
        | Op::Break
        | Op::Assignment
        | Op::Semicolon
        | Op::Return
        | Op::Comma => BeforeMinus::Operator,
        _ => BeforeMinus::Operand,
    }
}

/// The unary pass, right to left. `!` takes the next token as its child; `-` is unary after an
/// operator or at the start of a list, and after an operand becomes `Add` followed by
/// `Negate(next)`; `- -` collapses into `+`.
struct Unary {
    /// The unprocessed left part of the list.
    left: Vec<Node>,
    /// The processed part, nearest element on top.
    right: Vec<Node>,
    hole: Option<Node>,
}

impl Level for Unary {
    const RIGHT_TO_LEFT: bool = true;

    fn step(&mut self, cx: &mut Cx<'_>) -> Result<Option<Vec<Node>>, Failed> {
        while let Some(mut node) = self.left.pop() {
            if !node.is_leaf() {
                return Ok(Some(enter(&mut self.hole, node)));
            }
            if node.is(Op::Negate) {
                let Some(next) = self.right.pop() else {
                    cx.language(Msg::NegateWithoutOperand, node.span, &[]);
                    return Err(Failed);
                };
                let before = self
                    .left
                    .last()
                    .map_or(BeforeMinus::Operator, |previous| before_minus(previous.op));
                match before {
                    BeforeMinus::Operator => self.right.push(with_children(node, [next])),
                    BeforeMinus::Operand => {
                        let negate = Node::token(Op::Negate, Payload::None, node.span);
                        self.right.push(with_children(negate, [next]));
                        node.op = Op::Add;
                        self.right.push(node);
                    }
                    BeforeMinus::Minus => {
                        if let Some(previous) = self.left.last_mut() {
                            previous.op = Op::Add;
                        }
                        self.right.push(next);
                    }
                    BeforeMinus::Unknown => {
                        // Only a token before the `-` makes it unknown.
                        if let Some(previous) = self.left.last() {
                            let name = cx.friendly(previous);
                            cx.language(Msg::UnknownOperation, node.span, &[&name]);
                        }
                        return Err(Failed);
                    }
                }
            } else if node.is(Op::LogicalNot) {
                let Some(next) = self.right.pop() else {
                    cx.language(Msg::NotWithoutOperand, node.span, &[]);
                    return Err(Failed);
                };
                self.right.push(with_children(node, [next]));
            } else {
                self.right.push(node);
            }
        }
        Ok(None)
    }

    fn resume(&mut self, children: Vec<Node>, _: bool) {
        if let Some(mut node) = self.hole.take() {
            node.children = children;
            self.right.push(node);
        }
    }

    fn finish(mut self, _: &mut Cx<'_>) -> Result<(Vec<Node>, bool), Failed> {
        self.right.reverse();
        Ok((self.right, false))
    }
}

/// The pass of unary `-` and `!`.
pub(super) fn unary_minus_and_not(cx: &mut Cx<'_>, list: &mut Vec<Node>) -> Pass {
    run(
        cx,
        list,
        |left| Unary {
            right: Vec::with_capacity(left.len()),
            left,
            hole: None,
        },
        class(Op::LogicalNot),
    )
}

/// The prefix pass: a leaf `op` token (`return`) takes the next token, as it is, as its
/// child. The list is rebuilt in place ([`Compact`]).
struct Prefix {
    op: Op,
    list: Compact,
}

impl Level for Prefix {
    fn step(&mut self, cx: &mut Cx<'_>) -> Result<Option<Vec<Node>>, Failed> {
        let op = self.op;
        while let Some(found) = self.list.next_token(|node| node.is(op)) {
            let node = match found {
                Found::Children(children) => return Ok(Some(children)),
                Found::Token(node) => node,
            };
            let Some(operand) = self.list.next() else {
                cx.language(Msg::UnaryWithoutOperand, node.span, &[&cx.friendly(&node)]);
                return Err(Failed);
            };
            self.list.push(with_children(node, [operand]));
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

/// The prefix pass of `op`.
pub(super) fn prefix(cx: &mut Cx<'_>, list: &mut Vec<Node>, op: Op) -> Pass {
    run(
        cx,
        list,
        |list| Prefix {
            op,
            list: Compact::new(list),
        },
        class(op),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compile::ast::Span;

    use crate::compile::parse::test_support::*;

    #[test]
    fn the_negation_of_a_binary_minus_keeps_the_span_of_the_minus() {
        let list = grouped("unary", "v.a - v.b");
        assert_eq!(show_list(&list), "variable.a | Add | (Negate variable.b)");
        assert_eq!(list[1].span, Span::new(4, 5));
        assert_eq!(list[2].span, Span::new(4, 5));
        assert_eq!(list[2].children[0].span, Span::new(6, 9));
    }

    #[test]
    fn the_token_before_a_minus_decides_what_the_minus_is() {
        const OPERATORS: [Op; 24] = [
            Op::LeftBrace,
            Op::RightBrace,
            Op::LogicalNot,
            Op::Add,
            Op::Div,
            Op::Mul,
            Op::LessThan,
            Op::LessEqual,
            Op::GreaterEqual,
            Op::GreaterThan,
            Op::LogicalEqual,
            Op::LogicalNotEqual,
            Op::LogicalOr,
            Op::LogicalAnd,
            Op::NullCoalescing,
            Op::Conditional,
            Op::ConditionalElse,
            Op::Loop,
            Op::ForEach,
            Op::Break,
            Op::Assignment,
            Op::Semicolon,
            Op::Return,
            Op::Comma,
        ];
        for &op in Op::all() {
            let class = before_minus(op);
            if op == Op::Negate {
                assert!(matches!(class, BeforeMinus::Minus), "{op:?}");
            } else if matches!(op, Op::Continue | Op::ExpressionArray) {
                assert!(matches!(class, BeforeMinus::Unknown), "{op:?}");
            } else if OPERATORS.contains(&op) {
                assert!(matches!(class, BeforeMinus::Operator), "{op:?}");
            } else {
                assert!(matches!(class, BeforeMinus::Operand), "{op:?}");
            }
        }
    }

    #[test]
    fn a_minus_at_the_start_is_unary() {
        assert_eq!(passes("unary", "-1"), "(Negate 1)");
        assert_eq!(passes("unary", "-v.x"), "(Negate variable.x)");
    }

    #[test]
    fn a_minus_after_an_operand_is_a_binary_minus() {
        assert_eq!(passes("unary", "1 - 2"), "1 | Add | (Negate 2)");
        assert_eq!(
            passes("unary", "v.a - v.b"),
            "variable.a | Add | (Negate variable.b)"
        );
        assert_eq!(
            passes("sections+unary", "(1) - 2"),
            "(LeftParenthesis 1) | Add | (Negate 2)"
        );
        assert_eq!(
            passes("sections+unary", "array.a[1] - 2"),
            "array.a | (LeftBracket 1) | Add | (Negate 2)"
        );
    }

    #[test]
    fn a_minus_after_an_operator_is_unary() {
        assert_eq!(passes("unary", "1 * - 2"), "1 | Mul | (Negate 2)");
        assert_eq!(
            passes("sections+unary", "v.x == - 1"),
            "variable.x | LogicalEqual | (Negate 1)"
        );
        assert_eq!(
            passes("sections+unary", "1 > - 2"),
            "1 | GreaterThan | (Negate 2)"
        );
        assert_eq!(
            passes("sections+unary", "1 , - 2"),
            "1 | Comma | (Negate 2)"
        );
        assert_eq!(
            passes("sections+unary", "v.x ? - 1 : - 2"),
            "variable.x | Conditional | (Negate 1) | ConditionalElse | (Negate 2)"
        );
        assert_eq!(
            passes("sections+unary", "return - 1"),
            "Return | (Negate 1)"
        );
        assert_eq!(passes("sections+unary", "break - 1"), "Break | (Negate 1)");
        assert_eq!(
            passes("unary", "! - v.x"),
            "(LogicalNot (Negate variable.x))"
        );
    }

    #[test]
    fn a_minus_at_the_start_of_a_section_is_unary() {
        assert_eq!(
            passes("sections+unary", "{ - 1 }"),
            "(LeftBrace (Negate 1))"
        );
        assert_eq!(
            passes("sections+unary", "( - 1 )"),
            "(LeftParenthesis (Negate 1))"
        );
    }

    #[test]
    fn a_minus_after_another_minus_makes_that_one_an_add() {
        assert_eq!(passes("unary", "- -1"), "Add | 1");
        assert_eq!(passes("unary", "1 - -2"), "1 | Add | 2");
        assert_eq!(
            passes("unary", "v.a - - v.b"),
            "variable.a | Add | variable.b"
        );
    }

    #[test]
    fn three_minus_signs_pair_up_from_the_right() {
        assert_eq!(passes("unary", "1 - - - 2"), "1 | Add | (Negate Add) | 2");
    }

    #[test]
    fn a_minus_after_continue_is_an_unknown_operation() {
        assert_eq!(
            pass_error("unary", "continue - 1"),
            [(
                "E18",
                (9, 10),
                "Error: unknown Continue 'continue' operation in expression\n".to_owned()
            )]
        );
    }

    #[test]
    fn a_minus_before_continue_is_unary() {
        assert_eq!(passes("unary", "- continue"), "(Negate Continue)");
    }

    #[test]
    fn a_minus_without_an_operand_is_an_error() {
        assert_eq!(
            pass_error("unary", "1 -"),
            [(
                "E18",
                (2, 3),
                "Error: '-' not followed by expression\n".to_owned()
            )]
        );
        assert_eq!(pass_error("unary", "-")[0].1, (0, 1));
    }

    #[test]
    fn a_not_takes_the_next_token() {
        assert_eq!(passes("unary", "!v.x"), "(LogicalNot variable.x)");
        assert_eq!(
            passes("unary", "!!v.x"),
            "(LogicalNot (LogicalNot variable.x))"
        );
        assert_eq!(passes("unary", "-!v.x"), "(Negate (LogicalNot variable.x))");
        assert_eq!(passes("unary", "!-v.x"), "(LogicalNot (Negate variable.x))");
        assert_eq!(
            passes("unary", "!v.x == 1"),
            "(LogicalNot variable.x) | LogicalEqual | 1"
        );
    }

    #[test]
    fn a_not_without_an_operand_is_an_error() {
        assert_eq!(
            pass_error("unary", "!"),
            [(
                "E18",
                (0, 1),
                "Error: logical-not ('!') must be followed by expression\n".to_owned()
            )]
        );
        assert_eq!(pass_error("unary", "1 + !")[0].1, (4, 5));
    }

    #[test]
    fn the_unary_pass_enters_sections() {
        assert_eq!(
            passes("sections+unary", "(-1) + (!v.x)"),
            "(LeftParenthesis (Negate 1)) | Add | (LeftParenthesis (LogicalNot variable.x))"
        );
    }

    #[test]
    fn return_takes_the_next_token_as_it_is() {
        assert_eq!(passes("return", "return 1"), "(Return 1)");
        assert_eq!(
            passes("sections+return", "{return 1}"),
            "(LeftBrace (Return 1))"
        );
    }

    #[test]
    fn return_takes_only_one_token_before_the_later_passes() {
        assert_eq!(passes("return", "return 1 + 2"), "(Return 1) | Add | 2");
        assert_eq!(passes("return", "return return 1"), "(Return Return) | 1");
    }

    #[test]
    fn return_without_an_operand_is_an_error() {
        assert_eq!(
            pass_error("return", "return"),
            [(
                "E20",
                (0, 6),
                "Error: unary Return 'return' operator not followed by expression\n".to_owned()
            )]
        );
        assert_eq!(pass_error("return", "1 return")[0].1, (2, 8));
        assert_eq!(pass_error("sections+return", "(return)")[0].1, (1, 7));
    }

    mod tree_shapes {
        use crate::compile::test_support::pipeline::{tree, tree_at};

        #[test]
        fn double_minus() {
            assert_eq!(tree("v.a - -v.b"), "(Add v.a v.b)");
            assert_eq!(tree("1--1"), "2");
        }

        #[test]
        fn unary_operators() {
            assert_eq!(tree("-!!!0"), "-1");
            assert_eq!(tree("!-!!!0"), "0");
            assert_eq!(tree("1+!-!!!0"), "1");
            assert_eq!(tree("!v.a+v.b"), "(Add (LogicalNot v.a) v.b)");
            assert_eq!(tree("!v.a==v.b"), "(LogicalEqual (LogicalNot v.a) v.b)");
            assert_eq!(tree("-v.a*v.b"), "(Mul [v.a*-1+0] v.b)");
            assert_eq!(tree("!!v.a"), "(LogicalNot (LogicalNot v.a))");
        }

        #[test]
        fn minus_after_an_operand_is_binary() {
            for (source, expected) in [
                ("(v.a)-v.b", "(Add v.a [v.b*-1+0])"),
                ("math.abs(v.a)-v.b", "(Add (Abs v.a) [v.b*-1+0])"),
                ("q.count(1)-v.b", "(Add (QueryFunction 1) [v.b*-1+0])"),
                ("v.a-v.b", "(Add v.a [v.b*-1+0])"),
                ("v.a.c-v.b", "(Add (MemberAccessor v.a) [v.b*-1+0])"),
                ("2-v.b", "[v.b*-1+2]"),
                ("math.pi-v.b", "[v.b*-1+3.14159]"),
                ("array.a[0]-v.b", "(Add (Array 0) [v.b*-1+0])"),
                ("c.o->v.a-v.b", "(Add (Pointer c.o v.a) [v.b*-1+0])"),
                ("this-v.b", "(Add This [v.b*-1+0])"),
                (
                    "math.inverse_lerp(v.a, 1, 2)-v.b",
                    "(Add (InverseLerp v.a 1 2) [v.b*-1+0])",
                ),
                (
                    "math.ease_in_quad(v.a, 1, 2)-v.b",
                    "(Add (EaseInQuad v.a 1 2) [v.b*-1+0])",
                ),
            ] {
                assert_eq!(tree(source), expected, "{source}");
            }
            // Version 2: below 3 a string and a resource variable may be added.
            assert_eq!(
                tree_at("'a'-v.b", 2),
                "(Add 12638153115695167422 [v.b*-1+0])"
            );
            assert_eq!(tree_at("geometry.a-v.b", 2), "(Add geometry.a [v.b*-1+0])");
        }

        #[test]
        fn minus_after_an_operator_is_unary() {
            for (source, expected) in [
                ("-v.a", "[v.a*-1+0]"),
                ("!-v.a", "(LogicalNot [v.a*-1+0])"),
                ("v.b+-v.a", "(Add v.b [v.a*-1+0])"),
                ("v.b/-v.a", "(Div v.b [v.a*-1+0])"),
                ("v.b*-v.a", "(Mul v.b [v.a*-1+0])"),
                ("v.b<-v.a", "(LessThan v.b [v.a*-1+0])"),
                ("v.b==-v.a", "(LogicalEqual v.b [v.a*-1+0])"),
                ("v.b&&-v.a", "(LogicalAnd v.b [v.a*-1+0])"),
                ("v.b||-v.a", "(LogicalOr v.b [v.a*-1+0])"),
                ("v.b??-v.a", "(NullCoalescing v.b [v.a*-1+0])"),
                ("v.b?-v.a:-v.c", "(Conditional v.b [v.a*-1+0] [v.c*-1+0])"),
                ("v.b=-v.a;", "(Semicolon (Assignment v.b [v.a*-1+0]))"),
                ("v.b=1;-v.a;", "(Semicolon (Assignment v.b) [v.a*-1+0])"),
                ("return -v.a;", "(Semicolon (Return [v.a*-1+0]))"),
                ("math.max(-v.a,-v.b)", "(Max [v.a*-1+0] [v.b*-1+0])"),
                ("{-v.a;};", "(Semicolon (Semicolon [v.a*-1+0]))"),
            ] {
                assert_eq!(tree(source), expected, "{source}");
            }
        }

        #[test]
        fn pitfall_unary_binding() {
            assert_eq!(tree("-!!!0"), "-1");
            assert_eq!(tree("!v.a==v.b"), "(LogicalEqual (LogicalNot v.a) v.b)");
            assert_eq!(tree("-v.a*v.b"), "(Mul [v.a*-1+0] v.b)");
        }
    }
}
