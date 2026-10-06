//! The operator table (`ExpressionOp`, `OP_META`): its invariants, the side-effect flag, and one
//! test per version gate at the versions on both sides of it. `src/ops/table.rs` is maintained by
//! hand.

#![cfg(feature = "stdlib")]

mod common;

use molangx::ops::{ExpressionOp, OP_META, OpFlags};

#[test]
fn the_table_lists_111_ops_in_index_order() {
    assert_eq!(ExpressionOp::COUNT, 111);
    assert_eq!(OP_META.len(), 111);
    assert_eq!(ExpressionOp::all().len(), 111);
    for (index, op) in ExpressionOp::all().iter().enumerate() {
        let ordinal = u8::try_from(index).expect("an index fits u8");
        assert_eq!(op.ordinal(), ordinal, "{op:?}");
        assert_eq!(ExpressionOp::from_ordinal(ordinal), Some(*op));
        let meta = op.meta();
        assert_eq!(meta.op, *op);
        assert_eq!(OP_META[index].op, *op);
        assert_eq!(format!("{op:?}"), meta.name);
    }
    assert_eq!(ExpressionOp::from_ordinal(111), None);
}

#[test]
fn no_op_is_behind_an_experiment() {
    let known = OpFlags::MATH_FUNCTION
        .union(OpFlags::RESOURCE_REFERENCE)
        .union(OpFlags::SIDE_EFFECT);
    for meta in &OP_META {
        assert_eq!(
            meta.flags.bits() & !known.bits(),
            0,
            "{:?}: {:?}",
            meta.op,
            meta.flags
        );
    }
}

#[test]
fn the_side_effect_ops_are_random_random_integer_assignment_and_volatile_host_math() {
    let flagged: Vec<ExpressionOp> = ExpressionOp::all()
        .iter()
        .copied()
        .filter(|op| op.meta().flags.contains(OpFlags::SIDE_EFFECT))
        .collect();
    assert_eq!(
        flagged,
        [
            ExpressionOp::Random,
            ExpressionOp::RandomInt,
            ExpressionOp::Assignment,
            ExpressionOp::HostMathVolatile
        ]
    );
}

#[cfg(feature = "compiler")]
mod rules {
    //! Rules that compile text, at the latest version unless a gate says otherwise.

    use crate::common::compile_support::{client_at, message_ids, messages_at};
    use molangx::compile::{CompileFailure, CompileOptions, Compiled, Expr, compile};

    use molangx::ops::ExpressionOp::{self, *};

    fn at(source: &str, version: i16) -> Compiled {
        compile(source, &client_at(version))
    }

    /// The folded value of `source` at `version`, `None` when it is not one constant.
    fn constant_at(source: &str, version: i16) -> Option<f32> {
        let compiled = at(source, version);
        compiled
            .expr()
            .and_then(Expr::as_constant)
            .filter(|_| compiled.parses_cleanly())
    }

    /// Asserts that `source` parses cleanly at `kept` and is rejected at `rejected` with the
    /// message `id` first.
    fn gated(source: &str, kept: i16, rejected: i16, id: &str) {
        let before = at(source, kept);
        assert!(
            before.parses_cleanly(),
            "{source:?} at {kept}: {:?}",
            messages_at(source, kept)
        );
        let after = at(source, rejected);
        assert_eq!(
            after.failure(),
            Some(CompileFailure::Rejected),
            "{source:?} at {rejected}"
        );
        assert_eq!(
            message_ids(&after).first(),
            Some(&id),
            "{source:?} at {rejected}: {:?}",
            messages_at(source, rejected)
        );
    }

    /// The text of #36 for `parent` given a `child` operand.
    fn non_numerical(parent: ExpressionOp, child: ExpressionOp) -> String {
        format!(
            "'{}' expression cannot take a '{}' argument. It only supports numerical arguments.",
            parent.friendly_name(),
            child.friendly_name()
        )
    }

    /// `math.random` and `math.random_integer` are rejected (#21) only where random numbers are
    /// disallowed; `=` is rejected under both settings.
    #[test]
    fn side_effect_ops_are_rejected_where_side_effects_are_disallowed() {
        let options = client_at(13);
        let no_assignments = CompileOptions {
            allowed_ops: options.allowed_ops.without_assignments(),
            ..options.clone()
        };
        let nothing_random = CompileOptions {
            allowed_ops: options.allowed_ops.without_assignments_or_random(),
            ..options.clone()
        };
        for source in ["math.random(0, 1)", "math.random_integer(0, 1)"] {
            assert!(
                compile(source, &no_assignments).parses_cleanly(),
                "{source}"
            );
            let strict = compile(source, &nothing_random);
            assert_eq!(strict.failure(), Some(CompileFailure::Rejected), "{source}");
            assert_eq!(message_ids(&strict), ["E21"], "{source}");
        }
        for options in [no_assignments, nothing_random] {
            let assignment = compile("v.x = 1;", &options);
            assert_eq!(assignment.failure(), Some(CompileFailure::Rejected));
            assert_eq!(message_ids(&assignment), ["E21"]);
        }
        assert!(compile("v.x = 1;", &options).parses_cleanly());
    }

    /// The ops whose operands must be numeric from version 3.
    const NUMERIC_CHILDREN: [ExpressionOp; 69] = [
        Negate,
        LogicalNot,
        Abs,
        Add,
        Acos,
        Asin,
        Atan,
        Atan2,
        Ceil,
        Clamp,
        CopySign,
        Cos,
        DieRoll,
        DieRollInt,
        Div,
        Exp,
        Floor,
        HermiteBlend,
        Lerp,
        LerpRotate,
        Ln,
        Max,
        Min,
        MinAngle,
        Mod,
        Mul,
        Pow,
        Random,
        RandomInt,
        Round,
        Sin,
        Sign,
        Sqrt,
        Trunc,
        LessThan,
        LessEqual,
        GreaterEqual,
        GreaterThan,
        InverseLerp,
        EaseInQuad,
        EaseOutQuad,
        EaseInOutQuad,
        EaseInCubic,
        EaseOutCubic,
        EaseInOutCubic,
        EaseInQuart,
        EaseOutQuart,
        EaseInOutQuart,
        EaseInQuint,
        EaseOutQuint,
        EaseInOutQuint,
        EaseInSine,
        EaseOutSine,
        EaseInOutSine,
        EaseInExpo,
        EaseOutExpo,
        EaseInOutExpo,
        EaseInCirc,
        EaseOutCirc,
        EaseInOutCirc,
        EaseInBounce,
        EaseOutBounce,
        EaseInOutBounce,
        EaseInBack,
        EaseOutBack,
        EaseInOutBack,
        EaseInElastic,
        EaseOutElastic,
        EaseInOutElastic,
    ];

    /// `op` with a string literal as its first operand and `1` for the others.
    fn with_a_string_operand(op: ExpressionOp) -> String {
        match (op, op.math_fn()) {
            (Negate | LogicalNot, _) => format!("{}'a'", op.token().expect("a token")),
            (_, Some(function)) => format!(
                "{}('a'{})",
                function.token(),
                ", 1".repeat(usize::from(function.meta().min_args) - 1)
            ),
            (_, None) => format!("'a' {} 1", op.token().expect("a token")),
        }
    }

    /// From version 3 a string operand of arithmetic, `!`, an ordering comparison or a math
    /// function is rejected with
    /// #36; at version 2 it is used as it is.
    #[test]
    fn operands_are_numeric_from_version_3() {
        for op in NUMERIC_CHILDREN {
            let source = with_a_string_operand(op);
            gated(&source, 2, 3, "E36");
            assert_eq!(
                messages_at(&source, 3).first(),
                Some(&non_numerical(op, StringLiteral)),
                "{source}"
            );
        }
    }

    /// The ops that are no arithmetic operand from version 3, each written as an operand of `+`,
    /// and as an operand of `==`.
    const NOT_ARITHMETIC: [(ExpressionOp, &str, &str); 8] = [
        (StringLiteral, "'a' + 1", "'a' == 1"),
        (GeometryVariable, "geometry.foo + 1", "geometry.foo == 1"),
        (MaterialVariable, "material.foo + 1", "material.foo == 1"),
        (TextureVariable, "texture.foo + 1", "texture.foo == 1"),
        (
            Loop,
            "loop(3, {v.x = 1;}) + 1;",
            "loop(3, {v.x = 1;}) == 1;",
        ),
        (
            ForEach,
            "for_each(t.x, v.a, {v.x = 1;}) + 1;",
            "for_each(t.x, v.a, {v.x = 1;}) == 1;",
        ),
        (
            Break,
            "loop(2, {v.x = 1 + break;});",
            "loop(2, {v.x = 1 == break;});",
        ),
        (
            Continue,
            "loop(2, {v.x = 1 + continue;});",
            "loop(2, {v.x = 1 == continue;});",
        ),
    ];

    /// From version 3 these are rejected as an operand of `+` with #36 naming them; `==` takes them
    /// at versions 2 and 3.
    #[test]
    fn statement_and_resource_nodes_are_no_arithmetic_operand_from_version_3() {
        for (op, arithmetic, equality) in NOT_ARITHMETIC {
            gated(arithmetic, 2, 3, "E36");
            assert_eq!(
                messages_at(arithmetic, 3).first(),
                Some(&non_numerical(Add, op)),
                "{arithmetic}"
            );
            for version in [2, 3] {
                assert!(
                    at(equality, version).parses_cleanly(),
                    "{equality} at {version}: {:?}",
                    messages_at(equality, version)
                );
            }
        }
    }

    #[test]
    fn an_assignment_is_no_arithmetic_operand_from_version_3() {
        let source = "(v.foo = 1) + 2; return v.foo;";
        gated(source, 2, 3, "E36");
        assert_eq!(
            messages_at(source, 3).first(),
            Some(&non_numerical(Add, Assignment)),
            "{source}"
        );
    }

    /// Below version 4 a `( )` or `[ ]` section with several children is its first child; from 4 it
    /// is rejected with #32.
    #[test]
    fn a_section_has_one_child_from_version_4() {
        for (op, source) in [(LeftParenthesis, "(1 2)"), (LeftBracket, "[1 2]")] {
            gated(source, 3, 4, "E32");
            assert_eq!(constant_at(source, 3), Some(1.0), "{op:?}: the first child");
        }
    }

    /// Up to version 4 `:` groups before `?` (`a ? b : c ? d : e` is `(a ? b : c) ? d : e`) and a
    /// conditional in the then-branch is rejected with #25; a dangling `:` logs #16 up to 4 and #19
    /// from 5.
    #[test]
    fn the_conditional_is_right_associative_from_version_5() {
        let then_branch = "1 ? 1 ? 2 : 3 : 4";
        let old = at(then_branch, 4);
        assert_eq!(
            old.failure(),
            Some(CompileFailure::Rejected),
            "{then_branch} at 4"
        );
        assert_eq!(message_ids(&old), ["E25"], "{then_branch} at 4");
        assert_eq!(constant_at(then_branch, 5), Some(2.0), "{then_branch} at 5");
        let else_branch = "1 ? 0 : 1 ? 2 : 3";
        assert_eq!(constant_at(else_branch, 4), Some(3.0), "{else_branch} at 4");
        assert_eq!(constant_at(else_branch, 5), Some(0.0), "{else_branch} at 5");
        assert_eq!(message_ids(&at("1 :", 4)), ["E16"]);
        assert_eq!(message_ids(&at("1 :", 5)), ["E19"]);
    }

    /// Up to version 5 `||` binds tighter than `&&` and the comparisons sit on six levels, tightest
    /// first `<`, `==`, `>=`, `>`, `<=`, `!=`; from version 6 `&&` binds tighter than `||` and the
    /// ordering comparisons are one level above `==` / `!=`, each level grouping from the left.
    #[test]
    fn the_precedence_of_logical_and_comparison_operators_changes_at_version_6() {
        for (op, source, at_5, at_6) in [
            (LessThan, "1 > 1 < 2", 0.0, 1.0),
            (LessEqual, "2 <= 1 < 2", 0.0, 1.0),
            (GreaterThan, "2 <= 3 > 0", 0.0, 1.0),
            (GreaterEqual, "1 > 1 >= 2", 1.0, 0.0),
            (LogicalEqual, "0 >= 1 == 2", 1.0, 0.0),
            (LogicalNotEqual, "0 != 2 == 2", 1.0, 0.0),
            (LogicalOr, "1 || 0 && 0", 0.0, 1.0),
            (LogicalAnd, "0 && 1 || 1", 0.0, 1.0),
        ] {
            assert_eq!(constant_at(source, 5), Some(at_5), "{op:?}: {source} at 5");
            assert_eq!(constant_at(source, 6), Some(at_6), "{op:?}: {source} at 6");
        }
    }

    #[cfg(feature = "vm")]
    #[test]
    fn a_negative_divisor_keeps_its_sign_from_version_7() {
        use molangx::vm::NoHostEnv;

        let source = "v.a = -1; return 5 / v.a;";
        let eval = |version: i16| {
            let expr = at(source, version)
                .expr()
                .cloned()
                .unwrap_or_else(|| panic!("{source} at {version}"));
            expr.eval_f32(&mut NoHostEnv::new().cx())
        };
        assert_eq!((eval(6), eval(7)), (5.0, -5.0), "{Div:?}");
    }
}
