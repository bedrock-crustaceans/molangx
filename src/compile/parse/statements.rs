//! Statements: the semicolon pass.

use super::driver::{Level, class, enter, run};
use super::{Failed, Pass};
use crate::compile::{
    Cx,
    ast::{Node, Payload},
};
use crate::diag::LanguageMessage as Msg;
use crate::ops::ExpressionOp as Op;
use std::vec::IntoIter;

/// The statement pass: a list that contains `;` becomes one `Semicolon` node whose children are
/// the statements, each gathered under a `Semicolon` node of its own (with the span of its first
/// token) until the optimiser lifts it out. Empty statements vanish.
struct Statements {
    rest: IntoIter<Node>,
    out: Out,
    hole: Option<Node>,
}

/// Where [`Statements`] puts the nodes of its list.
enum Out {
    /// A list without `;`: every node is entered.
    Plain(Vec<Node>),
    /// A list split at its `;` tokens.
    Split {
        semicolon: Node,
        /// The statement being gathered; `None` after a `;`.
        statement: Option<Node>,
    },
}

impl Statements {
    fn new(list: Vec<Node>) -> Self {
        let out = match list.iter().find(|n| n.is(Op::Semicolon)) {
            None => Out::Plain(Vec::with_capacity(list.len())),
            Some(first) => Out::Split {
                semicolon: Node::token(Op::Semicolon, Payload::None, first.span),
                statement: None,
            },
        };
        Self {
            rest: list.into_iter(),
            out,
            hole: None,
        }
    }
}

impl Out {
    /// Whether a node with children is entered: in a split list `[ ]` and query arguments are not
    /// searched for nested statement lists.
    fn enters(&self, node: &Node) -> bool {
        match self {
            Self::Plain(_) => true,
            Self::Split { .. } => matches!(
                node.op,
                Op::Loop | Op::ForEach | Op::LeftParenthesis | Op::LeftBrace
            ),
        }
    }

    fn push(&mut self, node: Node) {
        match self {
            Self::Plain(out) => out.push(node),
            Self::Split { statement, .. } => {
                if let Some(statement) = statement {
                    statement.children.push(node);
                }
            }
        }
    }
}

impl Level for Statements {
    fn step(&mut self, cx: &mut Cx<'_>) -> Result<Option<Vec<Node>>, Failed> {
        for node in self.rest.by_ref() {
            if let Out::Split {
                semicolon,
                statement,
            } = &mut self.out
            {
                if node.is(Op::Semicolon) {
                    if semicolon.children.is_empty() && statement.is_none() {
                        cx.language(Msg::LeadingSemicolon, node.span, &[]);
                        return Err(Failed);
                    }
                    semicolon.children.extend(statement.take());
                    continue;
                }
                statement
                    .get_or_insert_with(|| Node::token(Op::Semicolon, Payload::None, node.span));
            }
            if !node.is_leaf() && self.out.enters(&node) {
                return Ok(Some(enter(&mut self.hole, node)));
            }
            self.out.push(node);
        }
        Ok(None)
    }

    fn resume(&mut self, children: Vec<Node>, _: bool) {
        if let Some(mut node) = self.hole.take() {
            node.children = children;
            self.out.push(node);
        }
    }

    fn finish(self, _: &mut Cx<'_>) -> Result<(Vec<Node>, bool), Failed> {
        let list = match self.out {
            Out::Plain(out) => out,
            Out::Split {
                mut semicolon,
                statement,
            } => {
                semicolon.children.extend(statement);
                vec![semicolon]
            }
        };
        Ok((list, false))
    }
}

/// The statement pass.
pub(super) fn semicolons(cx: &mut Cx<'_>, list: &mut Vec<Node>) -> Pass {
    run(cx, list, Statements::new, class(Op::Semicolon))
}

#[cfg(test)]
mod tests {
    use crate::compile::ast::Span;
    use crate::compile::parse::test_support::*;

    #[test]
    fn semicolons_split_a_list_into_statement_groups() {
        assert_eq!(
            passes("sections+semi", "v.x = 1; v.y = 2;"),
            "(Semicolon (Semicolon variable.x Assignment 1) (Semicolon variable.y Assignment 2))"
        );
        assert_eq!(
            passes("sections+semi", "1;2;"),
            "(Semicolon (Semicolon 1) (Semicolon 2))"
        );
    }

    #[test]
    fn empty_statements_vanish() {
        assert_eq!(
            passes("sections+semi", "v.x=1;; v.y=2;"),
            "(Semicolon (Semicolon variable.x Assignment 1) (Semicolon variable.y Assignment 2))"
        );
        assert_eq!(
            passes("sections+semi", "1;;;;2;"),
            "(Semicolon (Semicolon 1) (Semicolon 2))"
        );
    }

    #[test]
    fn a_leading_semicolon_is_an_error() {
        assert_eq!(
            pass_error("sections+semi", ";v.x"),
            [(
                "E14",
                (0, 1),
                "Error: expressions can't begin with a semicolon\n".to_owned()
            )]
        );
        assert_eq!(ids(&pass_error("sections+semi", ";")), ["E14"]);
    }

    #[test]
    fn the_semicolon_node_takes_the_span_of_the_first_separator() {
        let list = grouped("sections+semi", "1;2;");
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].span, Span::new(1, 2));
        assert_eq!(
            list[0].children[0].span,
            Span::new(0, 1),
            "a statement group has the span of its first token"
        );
        assert_eq!(list[0].children[1].span, Span::new(2, 3));
    }

    #[test]
    fn a_list_without_semicolons_stays_flat() {
        assert_eq!(
            passes("sections+semi", "v.x = 1 + 2"),
            "variable.x | Assignment | 1 | Add | 2"
        );
    }

    #[test]
    fn nested_statement_lists_are_split_inside_loops_parentheses_and_braces() {
        assert_eq!(
            passes("sections+semi", "(v.x = 1;)"),
            "(LeftParenthesis (Semicolon (Semicolon variable.x Assignment 1)))"
        );
        assert_eq!(
            passes("sections+semi", "loop(1,{v.x=1;}) ;"),
            "(Semicolon (Semicolon Loop (LeftParenthesis 1 Comma (LeftBrace (Semicolon (Semicolon variable.x Assignment 1))))))"
        );
        assert_eq!(
            passes("sections+semi", "v.x = (1;2); v.y = 1;"),
            "(Semicolon (Semicolon variable.x Assignment (LeftParenthesis (Semicolon (Semicolon 1) (Semicolon 2)))) (Semicolon variable.y Assignment 1))"
        );
    }

    #[test]
    fn a_bracket_section_inside_a_split_list_is_not_searched_for_statements() {
        assert_eq!(
            passes("sections+semi", "v.x = [1;2]; v.y = 1;"),
            "(Semicolon (Semicolon variable.x Assignment (LeftBracket 1 Semicolon 2)) (Semicolon variable.y Assignment 1))"
        );
    }

    #[test]
    fn a_plain_list_enters_every_section_for_statements() {
        assert_eq!(
            passes("sections+semi", "[1;2]"),
            "(LeftBracket (Semicolon (Semicolon 1) (Semicolon 2)))"
        );
    }

    mod tree_shapes {
        use crate::compile::test_support::pipeline::{
            assert_parsed, assert_rejected, constant, ids_at, tree,
        };

        #[test]
        fn statements() {
            assert_eq!(
                tree("v.x = 1; v.y = 2;"),
                "(Semicolon (Assignment v.x) (Assignment v.y))"
            );
            assert_eq!(
                tree("v.x = 1;; v.y = 2;"),
                "(Semicolon (Assignment v.x) (Assignment v.y))"
            );
            assert_eq!(tree("return 1"), "(Return 1)");
            assert_eq!(
                tree("return v.a ? v.b;"),
                "(Semicolon (Return (Conditional v.a v.b)))"
            );
            assert_eq!(
                tree("return v.a = 5;"),
                "(Semicolon (Return (Assignment v.a)))"
            );
            assert_eq!(
                tree("v.b = (v.a = 1);"),
                "(Semicolon (Assignment v.b (Assignment v.a)))"
            );
            assert_eq!(
                tree("{v.a=1; v.b=2};"),
                "(Semicolon (Semicolon (Assignment v.a) (Assignment v.b)))"
            );
            assert_eq!(
                tree("(v.x = 1;);"),
                "(Semicolon (Semicolon (Assignment v.x)))"
            );
        }

        #[test]
        fn simple_expressions() {
            assert_eq!(tree("1"), "1");
            assert_eq!(tree("v.x+1"), "[v.x*1+1]");
            assert_parsed("v.x+1", 13, &[]);
            assert_eq!(constant("1"), 1.0);
        }

        /// The last statement of a block may omit its `;`.
        #[test]
        fn last_statement_of_a_block() {
            assert_eq!(
                tree("{v.a=1; v.b=2};"),
                "(Semicolon (Semicolon (Assignment v.a) (Assignment v.b)))"
            );
            assert_parsed("{v.a=1; v.b=2};", 13, &[]);
        }

        #[test]
        fn parenthesised_statement_list() {
            assert_eq!(
                tree("(v.x = 1;);"),
                "(Semicolon (Semicolon (Assignment v.x)))"
            );
            assert_parsed("(v.x = 1;);", 13, &[]);
        }

        #[test]
        fn blocks_as_branches() {
            assert_eq!(
                tree("v.x ? {v.y = 1;} : {v.y = 2;};"),
                "(Semicolon (Conditional v.x (Semicolon (Assignment v.y)) (Semicolon (Assignment v.y))))"
            );
            assert_eq!(
                tree("variable.direction ?? { variable.direction.x = 0.0; };"),
                "(Semicolon (NullCoalescing v.direction (Semicolon (Assignment (MemberAccessor v.direction)))))"
            );
        }

        #[test]
        fn nested_loops() {
            assert_eq!(
                tree("loop(2, {loop(2, {break;}); v.c = 1;});"),
                "(Semicolon (Loop 2 (Semicolon (Loop 2 (Semicolon Break)) (Assignment v.c))))"
            );
            assert_parsed("loop(2, {loop(2, {v.c = v.c + 1;});});", 13, &[]);
        }

        #[test]
        fn return_in_a_loop_body() {
            assert_eq!(
                tree("loop(3, {return 1;});"),
                "(Semicolon (Loop 3 (Semicolon (Return 1))))"
            );
            assert_parsed("loop(3, {return 1;});", 13, &[]);
        }

        #[test]
        fn value_statements() {
            assert_eq!(tree("v.x = 1; v.x;"), "(Semicolon (Assignment v.x) v.x)");
            assert_eq!(tree("1; return 2;"), "(Semicolon 1 (Return 2))");
            assert_eq!(ids_at("v.x = 1; v.x;", 13), Vec::<String>::new());
        }

        #[test]
        fn block_statement_needs_its_own_semicolon() {
            assert_eq!(
                tree("{v.x = 1;};"),
                "(Semicolon (Semicolon (Assignment v.x)))"
            );
            assert_rejected("{v.x = 1;}", 13, &["E07"]);
            assert_eq!(
                tree("v.x = 1; {v.y = 2;};"),
                "(Semicolon (Assignment v.x) (Semicolon (Assignment v.y)))"
            );
        }
    }
}
