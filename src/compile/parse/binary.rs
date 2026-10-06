//! The binary levels.

use super::driver::{Compact, Found, Level, TokenClasses, class, run};
use super::{Failed, Pass};
use crate::compile::{Cx, ast::Node};
use crate::diag::LanguageMessage as Msg;
use crate::ops::ExpressionOp as Op;
use std::mem::take;

/// A fold whose left operand is in place: the operator node with its left operand as its only
/// child, and the right operand.
struct Fold {
    node: Node,
    right: Node,
    /// Whether the left operand holds an unfolded operator token.
    left_unfolded: bool,
}

/// Whose children a [`Binary`] level has handed out.
enum Entered {
    /// The node of the output the [`Compact`] list entered.
    Kept,
    /// The left operand of a fold, for its second walk.
    Left(Fold),
    /// The right operand of a fold.
    Right(Fold),
}

/// A binary level: each leaf token of `ops` takes its left and right neighbours as children, left
/// to right, so each level is left-associative. The list is folded in place ([`Compact`]).
///
/// A token consumed as a right operand (`1 + +`) stays unfolded below. A level reports whether its
/// list holds one, and walks the left operand of a fold again exactly when it does, failing on it.
struct Binary<'m> {
    /// The operators of the level.
    ops: &'m [Op],
    list: Compact,
    /// The indices of the output, ascending, whose subtree holds an unfolded operator token.
    unfolded: Vec<usize>,
    entered: Option<Entered>,
    /// A fold whose left operand is done and whose right operand is next.
    fold: Option<Fold>,
}

impl<'m> Binary<'m> {
    fn new(ops: &'m [Op], list: Vec<Node>) -> Self {
        Self {
            ops,
            list: Compact::new(list),
            unfolded: Vec::new(),
            entered: None,
            fold: None,
        }
    }

    /// Appends `node` to the output, noting whether its subtree holds an unfolded token.
    fn push(&mut self, node: Node, unfolded: bool) {
        let at = self.list.push(node);
        self.note(at, unfolded);
    }

    fn note(&mut self, at: usize, unfolded: bool) {
        if unfolded {
            self.unfolded.push(at);
        }
    }

    /// Takes the last node of the output and whether its subtree holds an unfolded token.
    fn pop(&mut self) -> Option<(Node, bool)> {
        let node = self.list.pop()?;
        let at = self.list.len();
        let unfolded = self.unfolded.pop_if(|&mut noted| noted == at).is_some();
        Some((node, unfolded))
    }

    /// Completes `fold` with its right operand, or hands out the right operand's children first.
    fn fold_right(&mut self, mut fold: Fold) -> Option<Vec<Node>> {
        if fold.right.is_leaf() {
            let right_unfolded = is_one_of(self.ops, &fold.right);
            self.complete(fold, right_unfolded);
            None
        } else {
            let children = take(&mut fold.right.children);
            self.entered = Some(Entered::Right(fold));
            Some(children)
        }
    }

    fn complete(&mut self, mut fold: Fold, right_unfolded: bool) {
        fold.node.children.push(fold.right);
        self.push(fold.node, fold.left_unfolded || right_unfolded);
    }
}

/// Whether `node` is a token of one of `ops`.
fn is_one_of(ops: &[Op], node: &Node) -> bool {
    ops.contains(&node.op)
}

impl Level for Binary<'_> {
    fn step(&mut self, cx: &mut Cx<'_>) -> Result<Option<Vec<Node>>, Failed> {
        let ops = self.ops;
        loop {
            if let Some(fold) = self.fold.take()
                && let Some(children) = self.fold_right(fold)
            {
                return Ok(Some(children));
            }
            let mut node = match self.list.next_token(|node| is_one_of(ops, node)) {
                None => return Ok(None),
                Some(Found::Children(children)) => {
                    self.entered = Some(Entered::Kept);
                    return Ok(Some(children));
                }
                Some(Found::Token(node)) => node,
            };
            let (Some((left, left_unfolded)), Some(right)) = (self.pop(), self.list.next()) else {
                cx.language(Msg::BinaryAtEnd, node.span, &[&cx.friendly(&node)]);
                return Err(Failed);
            };
            node.children.reserve_exact(2);
            node.children.push(left);
            let mut fold = Fold {
                node,
                right,
                left_unfolded,
            };
            if left_unfolded {
                // The second walk over the left operand finds the unfolded token and fails on it.
                let children = take(&mut fold.node.children[0].children);
                self.entered = Some(Entered::Left(fold));
                return Ok(Some(children));
            }
            self.fold = Some(fold);
        }
    }

    fn resume(&mut self, children: Vec<Node>, unfolded: bool) {
        match self.entered.take() {
            None => {}
            Some(Entered::Kept) => {
                if let Some(at) = self.list.resume(children) {
                    self.note(at, unfolded);
                }
            }
            Some(Entered::Left(mut fold)) => {
                fold.node.children[0].children = children;
                self.fold = Some(fold);
            }
            Some(Entered::Right(mut fold)) => {
                fold.right.children = children;
                self.complete(fold, unfolded);
            }
        }
    }

    fn finish(self, _: &mut Cx<'_>) -> Result<(Vec<Node>, bool), Failed> {
        Ok((self.list.finish(), !self.unfolded.is_empty()))
    }
}

/// The binary level of the operators `ops`.
pub(super) fn binary(cx: &mut Cx<'_>, list: &mut Vec<Node>, ops: &[Op]) -> Pass {
    let classes = ops
        .iter()
        .fold(TokenClasses::NONE, |classes, &op| classes | class(op));
    run(cx, list, |children| Binary::new(ops, children), classes)
}

#[cfg(test)]
mod tests {
    use crate::compile::parse::test_support::*;

    #[test]
    fn division_folds_left_to_right() {
        assert_eq!(passes("div", "1/2/3"), "(Div (Div 1 2) 3)");
        assert_eq!(passes("div", "1/2/3/4"), "(Div (Div (Div 1 2) 3) 4)");
    }

    #[test]
    fn a_binary_pass_leaves_other_tokens_alone() {
        assert_eq!(
            passes("div", "1 + 2 / 3 * 4"),
            "1 | Add | (Div 2 3) | Mul | 4"
        );
        assert_eq!(
            passes("mul", "1 + 2 / 3 * 4"),
            "1 | Add | 2 | Div | (Mul 3 4)"
        );
    }

    #[test]
    fn division_runs_before_multiplication() {
        assert_eq!(
            passes("div+mul", "v.a*v.b/v.c"),
            "(Mul variable.a (Div variable.b variable.c))"
        );
        assert_eq!(
            passes("div+mul", "v.a/v.b*v.c"),
            "(Mul (Div variable.a variable.b) variable.c)"
        );
    }

    #[test]
    fn a_binary_pass_works_inside_sections() {
        assert_eq!(
            passes("sections+add", "(1 + 2) + 3"),
            "(Add (LeftParenthesis (Add 1 2)) 3)"
        );
        assert_eq!(passes("sections+add", "{1 + 2}"), "(LeftBrace (Add 1 2))");
        assert_eq!(
            passes("sections+calls+comma", "q.is_baby(1, 2)"),
            "(query.is_baby (LeftParenthesis (Comma 1 2)))"
        );
    }

    #[test]
    fn a_binary_operator_without_a_right_operand_is_an_error() {
        assert_eq!(
            pass_error("add", "1 +"),
            [(
                "E16",
                (2, 3),
                "Error: binary Add '+' operator at end of expression\n".to_owned()
            )]
        );
        assert_eq!(
            pass_error("div", "1 /")[0].2,
            "Error: binary Divide '/' operator at end of expression\n"
        );
        assert_eq!(
            pass_error("mul", "v.a *")[0].2,
            "Error: binary Multiply '*' operator at end of expression\n"
        );
    }

    #[test]
    fn a_binary_operator_without_a_left_operand_is_the_same_error() {
        assert_eq!(
            pass_error("add", "+ 1"),
            [(
                "E16",
                (0, 1),
                "Error: binary Add '+' operator at end of expression\n".to_owned()
            )]
        );
        assert_eq!(
            pass_error("assign", "= 1")[0].2,
            "Error: binary Assignment '=' operator at end of expression\n"
        );
    }

    #[test]
    fn an_operator_taken_as_a_right_operand_is_reported_at_the_next_one() {
        assert_eq!(passes("add", "1 + + 2"), "(Add 1 Add) | 2");
        assert_eq!(passes("add", "1 + +"), "(Add 1 Add)");
        assert_eq!(
            pass_error("add", "1 + + + 2"),
            [(
                "E16",
                (4, 5),
                "Error: binary Add '+' operator at end of expression\n".to_owned()
            )]
        );
    }

    #[test]
    fn an_unfolded_operator_is_found_inside_sections_too() {
        assert_eq!(
            passes("sections+add", "(1 + + 2)"),
            "(LeftParenthesis (Add 1 Add) 2)"
        );
        assert_eq!(
            pass_error("sections+add", "(1 + + + 2)"),
            [(
                "E16",
                (5, 6),
                "Error: binary Add '+' operator at end of expression\n".to_owned()
            )]
        );
        assert_eq!(pass_error("sections+add", "1 + (2 +)")[0].1, (7, 8));
        assert_eq!(pass_error("sections+add", "(1 +) + 2")[0].1, (3, 4));
        assert_eq!(
            passes("sections+mul", "(v.a * * 2)"),
            "(LeftParenthesis (Mul variable.a Mul) 2)"
        );
    }

    #[test]
    fn an_unfolded_operator_in_a_right_operand_section_is_found_by_the_enclosing_fold() {
        // The section holding the dangling `+` is the left operand of the outer `+`.
        assert_eq!(
            pass_error("sections+add", "(1 + (2 + +)) + 3"),
            [(
                "E16",
                (10, 11),
                "Error: binary Add '+' operator at end of expression\n".to_owned()
            )]
        );
    }

    #[test]
    fn an_operator_as_the_last_right_operand_deep_in_a_tree_survives() {
        assert_eq!(
            passes("sections+add", "1 + (2 + +)"),
            "(Add 1 (LeftParenthesis (Add 2 Add)))"
        );
    }

    mod tree_shapes {
        use crate::compile::test_support::pipeline::{at, messages, tree};

        #[test]
        fn binary_levels_are_left_associative() {
            assert_eq!(tree("v.a/v.b/v.c"), "(Div (Div v.a v.b) v.c)");
            assert_eq!(tree("v.a<v.b<v.c"), "(LessThan (LessThan v.a v.b) v.c)");
            assert_eq!(
                tree("v.a>=v.b>v.c<=v.d"),
                "(LessEqual (GreaterThan (GreaterEqual v.a v.b) v.c) v.d)"
            );
            assert_eq!(
                tree("v.a==v.b!=v.c"),
                "(LogicalNotEqual (LogicalEqual v.a v.b) v.c)"
            );
        }

        #[test]
        fn addition_and_subtraction() {
            assert_eq!(tree("v.a+v.b*v.c"), "(Add v.a (Mul v.b v.c))");
            assert_eq!(tree("v.a-v.b-v.c"), "(Add v.a [v.b*-1+0] [v.c*-1+0])");
            assert_eq!(tree("v.a - v.b + v.c"), "(Add v.a [v.b*-1+0] v.c)");
            assert_eq!(tree("v.a/-v.b"), "(Div v.a [v.b*-1+0])");
            assert_eq!(tree("v.a - (-v.b)"), "(Add v.a v.b)");
            assert_eq!(
                tree("(v.a)-v.b"),
                "(Add v.a [v.b*-1+0])",
                "a section before `-` is an operand"
            );
            assert_eq!(
                tree("math.abs(v.a)-v.b"),
                "(Add (Abs v.a) [v.b*-1+0])",
                "a call before `-` is an operand"
            );
        }

        /// `=` is an ordinary binary operator, so an assignment may be any operand.
        #[test]
        fn assignment_as_an_operand() {
            for (source, expected) in [
                ("return v.a = 5;", "(Semicolon (Return (Assignment v.a)))"),
                (
                    "1 ? (v.a = 7) : 0;",
                    "(Semicolon (Conditional 1 (Assignment v.a) 0))",
                ),
                (
                    "v.b = (v.a = 1);",
                    "(Semicolon (Assignment v.b (Assignment v.a)))",
                ),
                (
                    "(v.a = 1) == 1;",
                    "(Semicolon (LogicalEqual (Assignment v.a)))",
                ),
                (
                    "q.count(v.a = 1);",
                    "(Semicolon (QueryFunction (Assignment v.a)))",
                ),
            ] {
                let compiled = at(source, 13);
                assert!(
                    compiled.parses_cleanly(),
                    "{source}: {:?}",
                    messages(&compiled)
                );
                assert_eq!(
                    compiled.tree_notation(9).as_deref(),
                    Some(expected),
                    "{source}"
                );
            }
        }

        #[test]
        fn pitfall_division_before_multiplication() {
            assert_eq!(tree("v.a*v.b/v.c"), "(Mul v.a (Div v.b v.c))");
        }
    }
}
