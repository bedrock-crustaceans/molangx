//! The check that the operands of a numerical operation are numbers.

use crate::catalog::{QueryCatalog, ReturnType};
use crate::compile::{
    Cx, Failed, Pass,
    ast::{Node, Payload},
};
use crate::diag::LanguageMessage as Msg;
use crate::ops::ExpressionOp as Op;

/// The operands of a numerical operation: a query child must return a number (every version); from
/// version 3 a string, resource, loop, `break` / `continue` or assignment child is an error.
pub(super) fn numerical_children(cx: &mut Cx<'_>, node: &Node) -> Pass {
    numerical_operands(cx, node, &node.children)
}

/// [`numerical_children`] for `operands`, some of `node`'s children.
pub(super) fn numerical_operands(cx: &mut Cx<'_>, node: &Node, operands: &[Node]) -> Pass {
    let strict = cx.version().reports_expression_errors();
    for child in operands {
        numerical_child(cx.opts.catalog, strict, child)
            .map_err(|error| error.log(cx, node, child))?;
    }
    Ok(())
}

/// Why a child of a numerical operation is not numerical.
#[derive(Debug, PartialEq, Eq)]
enum NotNumerical {
    /// A query that does not return a number.
    Query,
    /// From version 3 (`strict`): a string, resource, loop, `break` / `continue` or assignment.
    Value,
}

impl NotNumerical {
    fn log(self, cx: &mut Cx<'_>, node: &Node, child: &Node) -> Failed {
        match self {
            Self::Query => cx.language(
                Msg::QueryNotNumerical,
                child.full_span(),
                &[&cx.friendly(node)],
            ),
            Self::Value => cx.language(
                Msg::NonNumericalArgument,
                child.full_span(),
                &[&cx.friendly(node), &cx.friendly(child)],
            ),
        }
        Failed
    }
}

fn numerical_child(catalog: &QueryCatalog, strict: bool, child: &Node) -> Result<(), NotNumerical> {
    match child.op {
        Op::QueryFunction => {
            let returns_number = matches!(
                &child.value,
                Payload::Query(query)
                    if catalog.decl(query.index).shape().returns.intersects(ReturnType::NUMBER)
            );
            if returns_number {
                Ok(())
            } else {
                Err(NotNumerical::Query)
            }
        }
        Op::StringLiteral
        | Op::GeometryVariable
        | Op::MaterialVariable
        | Op::TextureVariable
        | Op::Geometry
        | Op::Material
        | Op::Texture
        | Op::Loop
        | Op::ForEach
        | Op::Break
        | Op::Continue
        | Op::Assignment
            if strict =>
        {
            Err(NotNumerical::Value)
        }
        _ => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::Side;
    use crate::compile::{
        ast::{Name, Span},
        sema::test_support::*,
    };
    use crate::diag::Severity;
    use crate::hash::HashedStr;

    fn shown_at(src: &str, raw: i16) -> String {
        tree_at(src, raw).tree_notation(9)
    }

    #[test]
    fn a_query_that_does_not_return_a_number_cannot_be_an_operand() {
        for (src, span, name) in [
            ("v.x+q.get_name", (4, 14), "Add '+'"),
            ("q.get_name+1", (0, 10), "Add '+'"),
            ("-q.get_name", (1, 11), "Negate '-'"),
        ] {
            assert_eq!(rejected(src), ("E37", span), "{src}");
            assert_eq!(
                rejected_text(src),
                format!("{name} expressions may only contain query functions that return numbers"),
                "{src}"
            );
        }
    }

    #[test]
    fn the_query_return_rule_holds_at_every_version() {
        for raw in [0, 2, 3, 13] {
            let out = rejected_at("q.get_name+1", raw);
            assert_eq!(
                out.log.iter().map(|m| m.0).collect::<Vec<_>>(),
                ["E37"],
                "version {raw}"
            );
        }
        assert!(shown("math.min(q.is_baby,1)").starts_with("(Min"));
    }

    #[test]
    fn a_string_in_arithmetic_is_an_error_from_version_3() {
        for (src, span, name, arg) in [
            ("'a'+1", (0, 3), "Add '+'", "String '''"),
            ("1+'a'", (2, 5), "Add '+'", "String '''"),
            ("'a'*2", (0, 3), "Multiply '*'", "String '''"),
            (
                "math.abs('a')",
                (9, 12),
                "Absolute Value 'math.abs'",
                "String '''",
            ),
            ("-'a'", (1, 4), "Negate '-'", "String '''"),
        ] {
            let out = rejected_at(src, 13);
            assert_eq!(out.log, [("E36", Severity::Error, span)], "{src}");
            assert_eq!(
                out.texts,
                [format!(
                    "'{name}' expression cannot take a '{arg}' argument. It only supports numerical arguments."
                )],
                "{src}"
            );
            let at_three = rejected_at(src, 3);
            assert_eq!(at_three.log[0].0, "E36", "{src} at 3");
        }
    }

    #[test]
    fn a_string_in_arithmetic_is_accepted_below_version_3() {
        let hash = HashedStr::new("a").as_u64();
        for raw in [-1, 0, 1, 2] {
            assert_eq!(
                shown_at("'a'+1", raw),
                format!("[{hash}*1+1]"),
                "version {raw}"
            );
            assert_eq!(
                shown_at("'a'*2", raw),
                format!("[{hash}*2+0]"),
                "version {raw}"
            );
            assert_eq!(
                shown_at("-'a'", raw),
                format!("[{hash}*-1+0]"),
                "version {raw}"
            );
            assert_eq!(
                shown_at("'a'<1", raw),
                format!("(LessThan {hash})"),
                "version {raw}"
            );
        }
    }

    #[test]
    fn resources_loops_and_assignments_in_arithmetic_follow_the_same_gate() {
        for (src, kind) in [
            (
                "math.abs(geometry.default)",
                "Geometry Variable 'geometry.'",
            ),
            ("geometry.default+1", "Geometry Variable 'geometry.'"),
            ("math.abs(v.x=1);", "Assignment '='"),
            ("math.abs(continue)", "Continue 'continue'"),
            ("math.abs(break)", "Break 'break'"),
            ("1+loop(2,{v.x=1;});", "Loop 'loop'"),
            ("for_each(t.a,v.l,{v.x=1;})+1;", "For Each 'for_each'"),
        ] {
            let strict = rejected_at(src, 13);
            assert_eq!(strict.log.len(), 1, "{src}");
            assert_eq!(strict.log[0].0, "E36", "{src}");
            assert!(
                strict.texts[0].contains(&format!("cannot take a '{kind}' argument")),
                "{src}: {}",
                strict.texts[0]
            );
            let loose = optimise_with(src, &client(2));
            assert!(loose.ok, "{src} at version 2");
            assert!(loose.log.is_empty(), "{src} at version 2");
        }
    }

    #[test]
    fn below_version_3_a_loop_in_arithmetic_keeps_its_post_op() {
        assert_eq!(
            shown_at("1+loop(2,{v.x=1;});", 0),
            "(Semicolon [(Loop 2 (Semicolon (Assignment v.x)))*1+1])"
        );
        assert_eq!(
            shown_at("v.y=loop(2,{v.x=1;})+1;", 0),
            "(Semicolon (Assignment v.y [(Loop 2 (Semicolon (Assignment v.x)))*1+1]))"
        );
        // A statement list used as an operand keeps its post-op at every version.
        assert_eq!(
            shown("{v.x=1;}+1;"),
            "(Semicolon [(Semicolon (Assignment v.x))*1+1])"
        );
        assert_eq!(
            shown("v.y={v.x=1;}+1;"),
            "(Semicolon (Assignment v.y [(Semicolon (Assignment v.x))*1+1]))"
        );
    }

    #[test]
    fn numerical_children_gate_the_twelve_non_numeric_ops_at_version_3() {
        let leaves = [
            Node::token(Op::StringLiteral, Payload::Hash(1), Span::new(0, 1)),
            Node::token(
                Op::GeometryVariable,
                Payload::Geometry(Name::new("geometry.a")),
                Span::new(0, 1),
            ),
            Node::token(
                Op::MaterialVariable,
                Payload::Material(Name::new("material.a")),
                Span::new(0, 1),
            ),
            Node::token(
                Op::TextureVariable,
                Payload::Texture(Name::new("texture.a")),
                Span::new(0, 1),
            ),
            Node::token(
                Op::Geometry,
                Payload::Geometry(Name::new("geometry.a")),
                Span::new(0, 1),
            ),
            Node::token(
                Op::Material,
                Payload::Material(Name::new("material.a")),
                Span::new(0, 1),
            ),
            Node::token(
                Op::Texture,
                Payload::Texture(Name::new("texture.a")),
                Span::new(0, 1),
            ),
            parent(Op::Loop, vec![]),
            parent(Op::ForEach, vec![]),
            Node::token(Op::Break, Payload::None, Span::new(0, 1)),
            Node::token(Op::Continue, Payload::None, Span::new(0, 1)),
            parent(Op::Assignment, vec![]),
        ];
        for raw in [0, 2, 3, 13] {
            let opts = client(raw);
            for leaf in &leaves {
                let node = parent(
                    Op::Abs,
                    vec![at(
                        Node::token(leaf.op, leaf.value.clone(), Span::new(3, 4)),
                        3,
                        4,
                    )],
                );
                let mut cx = Cx::for_test("abcdef", &opts);
                let result = numerical_children(&mut cx, &node);
                if raw >= 3 {
                    assert!(result.is_err(), "{:?} at version {raw}", leaf.op);
                    assert_eq!(
                        logged(&cx),
                        [("E36", Severity::Error, (3, 4))],
                        "{:?}",
                        leaf.op
                    );
                } else {
                    assert!(result.is_ok(), "{:?} at version {raw}", leaf.op);
                    assert!(cx.logged_diagnostics().is_empty());
                }
            }
        }
    }

    #[test]
    fn numerical_children_accept_numbers_variables_and_calls() {
        let node = parent(
            Op::Add,
            vec![
                float(1.0),
                entity("x"),
                temp("t"),
                parent(Op::Abs, vec![float(1.0)]),
                parent(Op::Conditional, vec![]),
            ],
        );
        let opts = client(13);
        let mut cx = Cx::for_test("abc", &opts);
        assert!(numerical_children(&mut cx, &node).is_ok());
        assert!(cx.logged_diagnostics().is_empty());
    }

    #[test]
    fn numerical_child_says_why_a_child_is_not_numerical() {
        let string = Node::token(Op::StringLiteral, Payload::Hash(1), Span::new(0, 1));
        assert_eq!(
            numerical_child(crate::stdlib::queries(Side::Client), true, &string),
            Err(NotNumerical::Value)
        );
        assert_eq!(
            numerical_child(crate::stdlib::queries(Side::Client), false, &string),
            Ok(())
        );
        for strict in [false, true] {
            assert_eq!(
                numerical_child(crate::stdlib::queries(Side::Client), strict, &float(1.0)),
                Ok(())
            );
            assert_eq!(
                numerical_child(crate::stdlib::queries(Side::Client), strict, &entity("x")),
                Ok(())
            );
        }
    }

    mod tree_shapes {
        use crate::compile::{
            CompileFailure,
            test_support::pipeline::{assert_parsed, assert_rejected, at, tree, tree_at},
        };

        #[test]
        fn strings_in_arithmetic_before_version_3() {
            assert_eq!(tree_at("'a' + 3", 2), "[12638153115695167422*1+3]");
            assert_eq!(tree_at("geometry.foo + 1", 0), "[geometry.foo*1+1]");
            assert_eq!(
                tree_at("(v.foo = 1) + 2; return v.foo;", 0),
                "(Semicolon [(Assignment v.foo)*1+2] (Return v.foo))"
            );
        }

        #[test]
        fn resources_in_arithmetic() {
            assert_eq!(tree("geometry.default"), "geometry.default");
            assert_rejected("geometry.foo + 1", 13, &["E36"]);
            let v2 = at("geometry.foo + 1", 2);
            assert!(v2.parsed());
            assert_eq!(v2.failure(), Some(CompileFailure::UsesResources));
        }

        #[test]
        fn statements_as_operands_at_version_2() {
            for source in [
                "loop(3,{v.count = v.count + 1;}) + 1; return v.count;",
                "(v.foo = 1) + 2; return v.foo;",
                "loop(3, {v.c = 1; (v.c == 1) ? break + 1;});",
            ] {
                assert_parsed(source, 2, &[]);
                assert_eq!(at(source, 2).failure(), None, "{source}");
            }
            assert_eq!(
                tree_at("loop(3, {v.x = 1;}) + 1;", 0),
                "(Semicolon [(Loop 3 (Semicolon (Assignment v.x)))*1+1])"
            );
        }
    }
}
