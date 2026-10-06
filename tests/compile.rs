//! Compiling text through the public API: results, folded constants, messages, version-dependent
//! outcomes, the options and restrictions of a compile, and `compile_source`.

#![cfg(all(feature = "compiler", feature = "stdlib"))]

mod common;

use common::compile_support::{at, message_ids};
use molangx::compile::{CompileFailure, Expr};

/// `source` is rejected at `version` with the language messages `expected` (by id), and its
/// failed node is 0.
fn assert_rejected(source: &str, version: i16, expected: &[&str]) {
    let compiled = at(source, version);
    assert!(
        !compiled.parsed(),
        "{source:?} v{version} should be rejected"
    );
    assert_eq!(
        compiled.failure(),
        Some(CompileFailure::Rejected),
        "{source:?} v{version}"
    );
    assert_eq!(message_ids(&compiled), expected, "{source:?} v{version}");
    assert_eq!(
        compiled.expr_or_zero().and_then(Expr::as_constant),
        Some(0.0),
        "{source:?}: the failed node is 0"
    );
}

/// `source` parses at `version` with the language messages `expected` (by id).
fn assert_parsed(source: &str, version: i16, expected: &[&str]) {
    let compiled = at(source, version);
    assert!(
        compiled.parsed(),
        "{source:?} v{version} should parse: {:?}",
        compiled.diagnostics()
    );
    assert_eq!(message_ids(&compiled), expected, "{source:?} v{version}");
}

mod lexing {
    //! The lexer: case folding, whitespace, numbers, strings, names, and what is no token.

    use crate::common::compile_support::{at, client_at, constant, messages, messages_at};
    use molangx::compile::{CompileFailure, CompileOptions, Deviations, Expr, compile};
    use molangx::ops::{ExpressionOp, OpSet};
    #[cfg(feature = "vm")]
    use molangx::vm::NoHostEnv;

    fn bits(source: &str) -> u32 {
        constant(source).to_bits()
    }

    #[test]
    fn case_is_folded_outside_strings() {
        assert_eq!(constant("TRUE"), 1.0);
        assert_eq!(constant("1E3"), 1000.0);
        assert_eq!(constant("1.5F"), 1.5);
    }

    /// The lowering pass and the string scanner disagree about escapes.
    #[test]
    fn backslash_protects_the_next_byte_from_lowering() {
        // `\A` outside a string: the `A` is not lowered, and `\` is no token.
        assert_eq!(messages_at("\\A", 13), ["unrecognized token: \\A"]);
    }

    /// Space, tab, newline and carriage return are whitespace; nothing else is.
    #[test]
    fn whitespace() {
        assert_eq!(messages_at("1\u{c}", 13), ["unrecognized token: \u{c}"]);
        assert_eq!(messages_at("1\u{b}", 13), ["unrecognized token: \u{b}"]);
        assert_eq!(at("\u{a0} 1", 13).failure(), Some(CompileFailure::Rejected));
    }

    #[test]
    fn nul_ends_the_expression() {
        assert_eq!(constant("1\0+5"), 1.0);
        assert_eq!(messages_at("\0 1", 13), ["No tokens found in expression"]);
    }

    /// Longest prefix, with no word boundary.
    #[test]
    fn operators_are_prefix_matched() {
        assert_eq!(
            messages_at("loopy", 13),
            ["Error: unknown token: y", "unrecognized token: y"]
        );
        assert_eq!(
            messages_at("thisx", 13),
            ["Error: unknown token: x", "unrecognized token: x"]
        );
        assert_eq!(
            messages_at("math.pix", 13),
            ["Error: unknown token: x", "unrecognized token: x"]
        );
    }

    #[test]
    fn aliases() {
        // There is no `m.` alias.
        assert_eq!(
            messages_at("m.floor(1.5)", 13),
            [
                "Error: unknown token: m.floor(1.5)",
                "unrecognized token: m.floor(1.5)"
            ]
        );
    }

    /// Resource names may contain dots; member accessors carry a name; identifiers are `[a-z0-9_]`.
    #[test]
    fn names() {
        assert_eq!(at("v.a:b", 13).failure(), Some(CompileFailure::Rejected));
        assert_eq!(
            messages_at("v.\u{17c}", 13),
            [
                "Error: unknown token: v.\u{17c}",
                "unrecognized token: v.\u{17c}"
            ]
        );
    }

    /// Names are never separated from their prefix.
    #[test]
    fn prefix_without_a_name() {
        assert_eq!(
            messages_at("v.1x", 13),
            ["Error: unknown token: v.1x", "unrecognized token: v.1x"]
        );
        assert_eq!(
            messages_at("variable.", 13),
            [
                "Error: unknown token: variable.",
                "unrecognized token: variable."
            ]
        );
        assert_eq!(messages_at("v.x . y", 13), ["unrecognized token: . y"]);
        assert_eq!(messages_at(". 1", 13), ["unrecognized token: . 1"]);
    }

    /// A miss is #5 and fails the token.
    #[test]
    fn queries_resolve_at_lex_time() {
        assert_eq!(
            messages_at("query.no_such_query + 1", 13),
            [
                "Failed to resolve query query.no_such_query.  Either the query does not exist or it is not supported in this context.",
                "unrecognized token: query.no_such_query + 1"
            ]
        );
        assert_eq!(
            messages_at("Q.No_Such_Query", 13),
            [
                "Failed to resolve query query.no_such_query.  Either the query does not exist or it is not supported in this context.",
                "unrecognized token: q.no_such_query"
            ]
        );
        // A query prefix without a name fails without a message of its own.
        assert_eq!(
            messages_at("q->v.x", 13),
            ["Error: unknown token: q->v.x", "unrecognized token: q->v.x"]
        );
        assert_eq!(messages_at("query.", 13), ["unrecognized token: query."]);
        assert_eq!(messages_at("q.", 13), ["unrecognized token: q."]);
        assert_eq!(messages_at("q.1", 13), ["unrecognized token: q.1"]);
    }

    #[test]
    fn number_grammar() {
        assert_eq!(constant("1"), 1.0);
        assert_eq!(constant(".5"), 0.5);
        assert_eq!(constant("5."), 5.0);
        assert_eq!(constant("007"), 7.0);
        assert_eq!(constant("1e3"), 1000.0);
        assert_eq!(constant("1e+3"), 1000.0);
        assert_eq!(constant("1e-3"), 0.001);
        assert_eq!(constant("1.5f"), 1.5);
        assert_eq!(constant("-1"), -1.0);
        assert_eq!(constant("-.25"), -0.25);
        assert_eq!(constant("-1e10f"), -1e10);
        assert_eq!(
            messages_at("1.5d", 13),
            ["Error: unknown token: d", "unrecognized token: d"]
        );
    }

    /// The integer part is a wrapping 32-bit signed accumulator.
    #[test]
    fn integer_part_wraps() {
        assert_eq!(constant("2147483647"), 2_147_483_647_i32 as f32);
        assert_eq!(constant("2147483648"), -2_147_483_648.0);
        assert_eq!(constant("3000000000"), -1_294_967_296.0);
        assert_eq!(constant("4294967296"), 0.0);
        assert_eq!(constant("99999999999"), 1_215_752_191_i32 as f32);
    }

    /// Eight fraction digits, divided in f32.
    #[test]
    fn fraction_digits() {
        assert_eq!(bits("0.00000001"), 1e-8_f32.to_bits());
        assert_eq!(constant("0.000000001"), 0.0);
        assert_eq!(
            bits("1.123456789"),
            (12_345_678.0_f32 / 1e8_f32 + 1.0).to_bits()
        );
        assert_eq!(
            bits("123.4567890123456789f"),
            (45_678_901.0_f32 / 1e8_f32 + 123.0).to_bits()
        );
        assert_eq!(bits("123.456f"), (456.0_f32 / 1000.0 + 123.0).to_bits());
        // Not the correctly rounded literal: the division is done in f32.
        assert_eq!(bits("0.1"), (1.0_f32 / 10.0).to_bits());
    }

    /// The exponent is the only f64 step and is a 32-bit integer.
    #[test]
    fn exponents() {
        assert_eq!(constant("1e39"), f32::INFINITY);
        assert_eq!(bits("1e-45"), 1);
        assert_eq!(constant("1e-50"), 0.0);
        assert_eq!(constant("1e+"), 1.0);
        assert_eq!(constant("1e-"), 1.0);
        assert_eq!(constant("1e2147483648"), 0.0);
        assert_eq!(bits("1.5e2"), 150.0_f32.to_bits());
        assert_eq!(
            messages_at("1e-x", 13),
            ["Error: unknown token: x", "unrecognized token: x"]
        );
    }

    /// `e` without a sign or digit is logged, makes the literal 0, and does not fail.
    #[test]
    fn malformed_exponent_is_kept() {
        let compiled = at("1e", 13);
        assert_eq!(compiled.failure(), None);
        assert_eq!(compiled.expr().and_then(Expr::as_constant), Some(0.0));
        assert_eq!(
            messages(&compiled),
            ["error parsing float string, expected '+' or '-' after 'e': "]
        );
        assert_eq!(
            compiled.diagnostics()[0].severity(),
            molangx::diag::Severity::Warning
        );
        assert_eq!(
            messages_at("1ex", 13),
            [
                "error parsing float string, expected '+' or '-' after 'e': x",
                "Error: unknown token: x",
                "unrecognized token: x"
            ]
        );
    }

    #[test]
    fn not_numbers() {
        assert_eq!(
            messages_at("0x10", 13),
            ["Error: unknown token: x10", "unrecognized token: x10"]
        );
        assert_eq!(
            messages_at("1_000", 13),
            ["Error: unknown token: _000", "unrecognized token: _000"]
        );
        assert_eq!(
            messages_at("1.5.5", 13),
            [
                "found multiple operations without a combining operation between them:\n\t1.500000\n\t0.500000"
            ]
        );
        assert_eq!(
            messages_at("+1", 13),
            ["Error: binary Add '+' operator at end of expression"]
        );
        assert_eq!(at("1.#QNAN", 13).failure(), Some(CompileFailure::Rejected));
        assert_eq!(at("1.#qnan", 13).failure(), Some(CompileFailure::Rejected));
    }

    /// Every prefix of `true` / `false` reads as the word.
    #[test]
    fn true_and_false() {
        for source in ["true", "tru", "tr", "t", "TRUE"] {
            assert_eq!(constant(source), 1.0, "{source}");
        }
        for source in ["false", "fals", "fal", "fa", "f", "False"] {
            assert_eq!(constant(source), 0.0, "{source}");
        }
        assert_eq!(
            messages_at("truex", 13),
            ["Error: unknown token: truex", "unrecognized token: truex"]
        );
    }

    /// `Deviations::ALL` advances by the identifier's length; switched off, the characters that
    /// would complete the word are dropped too, so `t;` loses its `;`.
    #[test]
    fn true_false_advance_deviation() {
        assert_eq!(constant("tr*5"), 5.0);
        assert_eq!(constant("t + 1"), 2.0);
        let no_deviations = CompileOptions {
            deviations: Deviations::NONE,
            ..client_at(13)
        };
        let value = |source: &str| {
            compile(source, &no_deviations)
                .expr()
                .cloned()
                .and_then(|e| e.as_constant())
        };
        assert_eq!(value("tr*5"), Some(1.0));
        assert_eq!(value("t;"), Some(1.0));
        let swallowed = compile("t + 1", &no_deviations);
        assert_eq!(
            messages(&swallowed),
            [
                "found multiple operations without a combining operation between them:\n\t1.000000\n\t1.000000"
            ]
        );
        assert_eq!(value("true"), Some(1.0));
        assert_eq!(value("false"), Some(0.0));
    }

    #[test]
    fn never_tokens() {
        for (source, rest) in [
            ("1 # comment", "# comment"),
            ("\"abc\"", "\"abc\""),
            ("$x", "$x"),
            ("v.a % v.b", "% v.b"),
            ("v.a ^ v.b", "^ v.b"),
            ("v.a & v.b", "& v.b"),
            ("v.a | v.b", "| v.b"),
            ("v.a @ v.b", "@ v.b"),
            ("~1", "~1"),
            ("`", "`"),
        ] {
            assert_eq!(
                messages_at(source, 13),
                [format!("unrecognized token: {rest}")],
                "{source}"
            );
        }
        assert_eq!(
            messages_at("1 // comment", 13),
            [
                "Error: unknown token: comment",
                "unrecognized token: comment"
            ]
        );
    }

    /// A backslash skips the next byte; nothing is unescaped.
    #[test]
    fn strings() {
        let missing = ["Error: Molang string missing final ' character"];
        for invalid in [
            "'a\\b'",
            "'\\''",
            "'ab\\'",
            "'\\x'",
            "'a\\'' == 1",
            "'unterminated",
            "'",
            "'\\",
        ] {
            assert_eq!(messages_at(invalid, 13)[0], missing[0], "{invalid}");
            assert_eq!(
                at(invalid, 13).failure(),
                Some(CompileFailure::Rejected),
                "{invalid}"
            );
        }
    }

    /// `math.pi` is the only math constant.
    #[test]
    fn math_constants() {
        assert_eq!(bits("math.pi"), std::f32::consts::PI.to_bits());
        assert_eq!(
            messages_at("math.e", 13),
            ["Error: unknown token: math.e", "unrecognized token: math.e"]
        );
        assert_eq!(
            messages_at("math.foo(1)", 13),
            [
                "Error: unknown token: math.foo(1)",
                "unrecognized token: math.foo(1)"
            ]
        );
    }

    #[test]
    fn spans_index_the_original_text() {
        let source = "V.X + Query.No_Such + 1";
        let compiled = at(source, 13);
        let unresolved = &compiled.diagnostics()[0];
        assert_eq!(
            &source[unresolved.span().start as usize..unresolved.span().end as usize],
            "Query.No_Such"
        );
    }

    #[test]
    fn minus_is_never_part_of_a_number() {
        assert_eq!(constant("- 1"), -1.0);
        assert_eq!(
            constant("1 -1"),
            0.0,
            "`-1` after an operand is a binary minus"
        );
    }

    #[test]
    fn only_the_f_suffix() {
        assert_eq!(constant("2f"), 2.0);
        assert_eq!(constant("2.5F"), 2.5);
        for (source, rest) in [("1l", "l"), ("1u", "u"), ("1.5d", "d")] {
            assert_eq!(
                messages_at(source, 13),
                [
                    format!("Error: unknown token: {rest}"),
                    format!("unrecognized token: {rest}")
                ],
                "{source}"
            );
        }
        // A second `f` is a token of its own (`false`), not a suffix.
        assert_eq!(at("1.5ff", 13).failure(), Some(CompileFailure::Rejected));
    }

    #[test]
    fn no_other_tokens_and_no_comments() {
        // `<>` is `<` followed by `>`.
        assert_eq!(
            messages_at("v.a <> v.b", 13),
            [
                "found multiple operations without a combining operation between them:\n\t<\n\tvariable."
            ]
        );
        assert_eq!(
            messages_at("v.a => v.b", 13),
            ["Error: complex expressions (contains either '=' or ';') must end with a ';'"]
        );
        for source in [
            "1 /* comment */",
            "1 // comment",
            "1 # comment",
            "\\1",
            "v.a \\ v.b",
        ] {
            assert_eq!(
                at(source, 13).failure(),
                Some(CompileFailure::Rejected),
                "{source}"
            );
        }
    }

    /// The allowed-operation check names each used operation once, and never an unused one.
    #[test]
    fn used_operations_are_a_set() {
        let no_assignment = CompileOptions {
            allowed_ops: OpSet::all()
                .without(ExpressionOp::Assignment)
                .without(ExpressionOp::Mul),
            ..client_at(13)
        };
        assert_eq!(
            messages(&compile("v.x = 1; v.y = 2; v.z = 3;", &no_assignment)),
            ["Expression uses operation Assignment '=' which is not allowed in this context"]
        );
        assert!(compile("v.x + v.y", &no_assignment).parses_cleanly());
        assert_eq!(
            messages(&compile("v.x * 2; v.y = v.x * 3;", &no_assignment)),
            [
                "Expression uses operation Multiply '*' which is not allowed in this context",
                "Expression uses operation Assignment '=' which is not allowed in this context",
            ]
        );
    }

    #[test]
    fn each_opener_has_its_closer() {
        for (source, closer, opener) in [
            ("3 { 6", "Right Brace '}'", "Left Brace '{'"),
            ("5 [ 4", "Right Bracket ']'", "Left Bracket '['"),
            ("7 ( 2", "Right Parenthesis ')'", "Left Parenthesis '('"),
        ] {
            let lines = messages_at(source, 13);
            assert_eq!(
                lines.last().map(String::as_str),
                Some(
                    format!(
                        "Error: Could not find {closer} to close section started with {opener}"
                    )
                    .as_str()
                ),
                "{source}"
            );
        }
        assert_eq!(
            messages_at("[1)", 13)[0],
            "Unable to match closing section symbol at 1(Left Bracket '[') - looking for Right Bracket ']', found Right Parenthesis ')' at 2"
        );
    }

    #[cfg(feature = "vm")]
    #[test]
    fn variable_tokens_name_themselves_canonically() {
        for (source, name) in [
            ("v.Missing", "variable.missing"),
            ("variable.missing", "variable.missing"),
            ("t.missing", "temp.missing"),
            ("temp.missing", "temp.missing"),
            ("c.missing", "context.missing"),
            ("Context.Missing", "context.missing"),
        ] {
            let compiled = at(source, 13);
            let mut env = NoHostEnv::new();
            assert_eq!(
                compiled
                    .expr()
                    .cloned()
                    .expect("expr")
                    .eval_f32(&mut env.cx()),
                0.0,
                "{source}"
            );
            assert_eq!(
                env.sink.take(),
                [format!(
                    "Error: unhandled request for unknown variable '{name}'"
                )],
                "{source}"
            );
        }
    }
}

mod grouping {
    //! The messages a malformed operator layout gets.

    use crate::common::compile_support::{at, messages_at};
    use molangx::compile::CompileFailure;

    /// `- -` collapses into `+`.
    #[test]
    fn double_minus() {
        for source in ["--1", "- -1", "---1", "2 * - -3", "!--3"] {
            assert_eq!(
                messages_at(source, 13),
                ["Error: binary Add '+' operator at end of expression"],
                "{source}"
            );
        }
    }

    #[test]
    fn dangling_unary_operators() {
        assert_eq!(
            messages_at("1 -", 13),
            ["Error: '-' not followed by expression"]
        );
        assert_eq!(
            messages_at("1 + !", 13),
            ["Error: logical-not ('!') must be followed by expression"]
        );
        assert_eq!(
            messages_at("continue - 1", 13),
            ["Error: unknown Continue 'continue' operation in expression"]
        );
    }

    #[test]
    fn an_unfolded_operator_is_found_by_an_enclosing_fold() {
        assert_eq!(
            messages_at("(1 + (2 + +)) + 3", 13),
            ["Error: binary Add '+' operator at end of expression"]
        );
        // The `||` level runs once on either side of version 6: a second run would report the
        // dangling one.
        let malformed = [
            "Malformed Logical Or '||' expression. It has 0 children but should have between 2 and 2",
        ];
        assert_eq!(messages_at("1 || ||", 5), malformed);
        assert_eq!(messages_at("1 || ||", 13), malformed);
    }

    #[test]
    fn null_coalescing_is_looser_than_the_conditional() {
        let lhs = "Error: found left-hand-side of ?? expression that isn't a direct-variable reference - this is unsupported at this time.";
        assert_eq!(messages_at("v.a?v.b:v.c??v.d", 13), [lhs]);
        assert_eq!(messages_at("v.a ?? v.b ?? 5", 13), [lhs]);
        assert_eq!(messages_at("(1 ?? 2) * 2", 13), [lhs]);
        assert_eq!(
            at("v.a ?? v.b ?? 5", 2).failure(),
            Some(CompileFailure::Rejected)
        );
    }

    /// `->` runs before the math-call pass.
    #[test]
    fn pointer() {
        assert_eq!(
            messages_at("math.floor(1.5)->v.hp", 13),
            ["Error: Floor 'math.floor' operator not followed by parenthesis section"]
        );
        // A failing `->` pass clears the tree and the remaining passes run on the empty list.
        for source in ["v.x->", "->v.x"] {
            assert_eq!(
                messages_at(source, 13),
                [
                    "Error: binary Pointer '->' operator at end of expression",
                    "found multiple operations without a combining operation between them:"
                ],
                "{source}"
            );
        }
    }

    #[test]
    fn structure_passes() {
        assert_eq!(
            messages_at(".foo", 13),
            [
                "Error: cannot start an expression with a member accessor; member accessors require a variable of which to access a member."
            ]
        );
        assert_eq!(
            messages_at("(1", 13),
            [
                "Unable to find matching closing section symbol for symbol at 1(Left Parenthesis '(') -- looking for Right Parenthesis ')'",
                "Error: Could not find Right Parenthesis ')' to close section started with Left Parenthesis '('"
            ]
        );
        assert_eq!(
            messages_at("(1]", 13),
            [
                "Unable to match closing section symbol at 1(Left Parenthesis '(') - looking for Right Parenthesis ')', found Right Bracket ']' at 2",
                "Error: Could not find Right Parenthesis ')' to close section started with Left Parenthesis '('"
            ]
        );
        assert_eq!(
            messages_at("1)", 13),
            [
                "found multiple operations without a combining operation between them:\n\t1.000000\n\t)"
            ]
        );
        assert_eq!(
            messages_at("loop()", 13),
            ["Error: Loop 'loop' operators with no params should not use parentheses"]
        );
        assert_eq!(
            messages_at("array.a[]", 13),
            [
                "found multiple operations without a combining operation between them:\n\tarray.\n\t[]"
            ]
        );
    }

    #[test]
    fn statements() {
        let must_end =
            ["Error: complex expressions (contains either '=' or ';') must end with a ';'"];
        for source in ["v.x = 1", "v.x = 1; return v.x", "{v.x = 1;}", "1;2"] {
            assert_eq!(messages_at(source, 13), must_end, "{source}");
        }
        assert_eq!(
            messages_at(";", 13),
            ["Error: expressions can't begin with a semicolon"]
        );
        assert_eq!(
            messages_at("return;", 13),
            ["Error: unary Return 'return' operator not followed by expression"]
        );
        assert_eq!(
            messages_at("(1, 2)", 13),
            [
                "Error: Unexpected Comma ',' operator not inside an arguments list for a query, loop, or math function"
            ]
        );
        assert_eq!(
            messages_at("break 11", 13),
            [
                "found multiple operations without a combining operation between them:\n\tbreak\n\t11.000000"
            ]
        );
    }

    #[test]
    fn passes_are_gated_by_the_used_operators() {
        // No `?` and no `:` → the conditional pass does not run; the leftover tokens fail the root
        // check.
        assert_eq!(
            messages_at("1 2", 13),
            [
                "found multiple operations without a combining operation between them:\n\t1.000000\n\t2.000000"
            ]
        );
        // `loop` alone uses no section, so the call pass has nothing to attach.
        assert_eq!(
            messages_at("loop", 13),
            [
                "Loop 'loop' operator should have exactly one child (a left-parenthesis expression with the params as children of it) prior to optimization."
            ]
        );
    }
}

mod folding {
    //! Constant folding and the affine post-op: what folds, what never does, and the values the
    //! optimised programs have.

    use crate::common::compile_support::{at, client_at, constant};
    use crate::common::per_arch;
    use molangx::compile::{CompileFailure, Expr, compile};
    use molangx::numeric::{ARCH, Arch};
    #[cfg(feature = "vm")]
    use molangx::vm::{NoHostEnv, Value, VariableName};

    fn is_constant(source: &str) -> bool {
        let compiled = at(source, 13);
        assert_eq!(compiled.failure(), None, "{source}");
        compiled
            .expr()
            .cloned()
            .is_some_and(|e| e.as_constant().is_some())
    }

    /// Random functions, variables and queries never fold.
    #[test]
    fn what_folds() {
        assert_eq!(constant("1 + 1"), 2.0);
        assert_eq!(constant("1 * 2 + 3 / (4 + 2) + (1 ? 2 : 3)"), 4.5);
        assert_eq!(
            constant("math.cos(15 + math.sqrt(100))"),
            libm::cos(f64::from(25.0_f32 * f32::from_bits(0x3c8e_fa35))) as f32
        );
        assert_eq!(constant("math.min_angle(5 + 10)"), 15.0);
        assert_eq!(constant("math.lerp(0, 1, 0.5)"), 0.5);
        assert_eq!(constant("math.clamp(3, 2, 1)"), 1.0);
        assert_eq!(constant("math.max(2, 3)"), 3.0);
        assert_eq!(constant("math.pow(2, 3)"), 8.0);
        assert_eq!(constant("math.mod(-5, 3)"), -2.0);
        assert_eq!(constant("3 > 2"), 1.0);
        assert_eq!(constant("3 && 0.5"), 1.0);
        assert_eq!(constant("0 || 0"), 0.0);
        assert_eq!(constant("!0.5"), 0.0);
        assert_eq!(constant("0 ? 5 : 6"), 6.0);
        assert_eq!(constant("0 ? 5"), 0.0);
        assert_eq!(constant("true"), 1.0);
        for source in [
            "math.random(1, 2)",
            "math.random_integer(1, 6)",
            "math.die_roll(2, 1, 6)",
            "math.die_roll_integer(1, 6, 6)",
            "variable.foo",
            "math.cos(15 + math.sqrt(variable.bar))",
            "query.position(0)",
            "this",
        ] {
            assert!(!is_constant(source), "{source}");
        }
        // A string is a constant, but not a float.
        let string = at("'some text'", 13);
        assert_eq!(string.expr().map(Expr::is_constant), Some(true));
        assert_eq!(string.expr().and_then(Expr::as_constant), None);
    }

    /// Identical terms whose scales sum to zero fold to the constant of the sum's own offset; the
    /// offsets the terms carry are not added.
    #[test]
    fn identical_terms_of_scale_zero_fold_to_a_constant() {
        assert_eq!(constant("v.x*0+3+(v.x*0+3)"), 0.0);
        assert_eq!(constant("v.x*0+(v.x*0)"), 0.0);
    }

    /// The comparison keeps the string's hash in its value, yet only a lone string literal is a
    /// constant.
    #[test]
    fn a_comparison_with_a_string_is_not_a_constant() {
        for source in [
            "v.x == 'a'",
            "'a' == v.x",
            "v.x != 'a'",
            "'a' != v.x",
            "q.is_baby == 'a'",
        ] {
            let compiled = at(source, 13);
            assert_eq!(compiled.failure(), None, "{source}");
            let expr = compiled.expr().cloned().expect("an expression");
            assert!(!expr.is_constant(), "{source}");
            assert_eq!(expr.as_constant(), None, "{source}");
        }
        assert!(
            at("'a'", 13)
                .expr()
                .cloned()
                .expect("an expression")
                .is_constant()
        );
    }

    /// Never folds to its operand: a falsy −0 last operand gives +0.
    #[test]
    fn logic_folds_to_a_boolean() {
        for (source, bits) in [
            ("1 && (-0)", 0u32),
            ("0 || (-0)", 0),
            ("1 && 0", 0),
            ("1 && 5", 1.0f32.to_bits()),
            ("0 || -2", 1.0f32.to_bits()),
            ("-0 && 1", 0),
        ] {
            assert_eq!(constant(source).to_bits(), bits, "{source}");
        }
    }

    /// A literal divisor becomes a multiplication by its reciprocal; an (almost) zero one, a
    /// multiplication by zero.
    #[test]
    fn division() {
        assert_eq!(constant("1/0"), 0.0);
        assert_eq!(constant("-1/0"), 0.0);
        assert_eq!(constant("0/0"), 0.0);
        assert_eq!(constant("1 / 0 * 5"), 0.0);
        assert_eq!(constant("5 / 0.0000001"), 0.0, "below f32::EPSILON");
        assert_eq!(
            constant("7 * 3 / 9"),
            7.0_f32 * (3.0_f32 / 9.0),
            "`/` is grouped before `*`"
        );
    }

    /// With x = 2 and y = 3, a cancelled sum assigned or moved into its parent is worth `S·v + O` =
    /// 2; as a conditional branch or the whole expression, its value `v` = 1.
    #[cfg(feature = "vm")]
    #[test]
    fn a_float_with_a_post_op_reads_as_the_server_rows_show() {
        let x = "((v.x + v.y + 1) + (-v.x - v.y))";
        let run = |source: &str| {
            let compiled = at(source, 13);
            assert_eq!(compiled.failure(), None, "{source}");
            let mut env = NoHostEnv::new();
            for (name, value) in [("x", 2.0), ("y", 3.0), ("z", 5.0), ("w", 0.5), ("c", 1.0)] {
                env.variables
                    .set(VariableName::new(name), Value::Float(value));
            }
            let result = compiled
                .expr()
                .cloned()
                .expect("expr")
                .eval_f32(&mut env.cx());
            let stored = env.variables.get(VariableName::new("r")).map(Value::as_f32);
            (result, stored)
        };
        assert_eq!(
            run(x).0,
            1.0,
            "the whole expression: its value (this crate's choice)"
        );
        assert_eq!(
            run(&format!("v.r = {x};")).1,
            Some(2.0),
            "o12: an assignment stores 2"
        );
        assert_eq!(run(&format!("v.z * {x}")).0, 10.0, "o13");
        assert_eq!(run(&format!("v.z + {x}")).0, 7.0, "o14");
        assert_eq!(run(&format!("math.max(v.w, {x})")).0, 2.0, "o16");
        assert_eq!(
            run(&format!("v.c ? {x} : 7")).0,
            1.0,
            "g10: a branch is worth its value"
        );
        assert_eq!(run(&format!("1 ? {x} : 7")).0, 1.0, "a folded branch too");
    }

    /// The `Float` keeps the sum's post-op. An operand of an all-constant fold and the condition of
    /// a conditional read its value `v`.
    #[test]
    fn a_cancelled_sum_is_a_float_with_a_post_op() {
        let x = "((v.x + v.y + 1) + (-v.x - v.y))";
        assert_eq!(constant(x), 1.0);
        assert_eq!(constant(&format!("1 ? {x} : 7")), 1.0);
    }

    #[test]
    fn strings_in_arithmetic_before_version_3() {
        assert_eq!(at("'a' + 3", 3).failure(), Some(CompileFailure::Rejected));
    }

    #[test]
    fn folding_follows_the_architecture() {
        let value = |source: &str| {
            compile(source, &client_at(13))
                .expr()
                .cloned()
                .and_then(|e| e.as_constant())
                .expect("constant")
        };
        // A NaN operand: on arm64 `<` and `<=` are true when an operand is NaN.
        assert_eq!(value("math.sqrt(-1) < 4"), per_arch(0.0, 1.0));
        assert_eq!(
            value("math.max(4, math.sqrt(-1))").is_nan(),
            per_arch(true, false)
        );
        assert_eq!(value("math.sign(math.sqrt(-1))"), per_arch(1.0, -1.0));
        assert_eq!(
            value("math.asin(math.sqrt(-1))").is_nan(),
            per_arch(true, false)
        );
        assert_eq!(
            value("math.acos(math.sqrt(-1))").is_nan(),
            per_arch(true, false)
        );
        if ARCH == Arch::Arm64 {
            assert_eq!(value("math.asin(math.sqrt(-1))"), -90.0);
            assert_eq!(value("math.acos(math.sqrt(-1))"), 180.0);
        }
        assert_eq!(value("math.round(-2.5)"), -3.0);
        assert_eq!(value("math.min_angle(180)"), -180.0);
        assert_eq!(value("math.sign(0)"), 1.0);
        assert!(
            value("math.mod(1, 0)").is_nan(),
            "a literal zero divisor is not guarded"
        );
        assert_eq!(value("math.asin(1.0004)"), 90.0);
        assert!(value("math.asin(1.001)").is_nan());
        assert_eq!(value("math.clamp(math.sqrt(-1), 1, 2)"), 1.0);
        assert_eq!(
            value("-!1").to_bits(),
            (-0.0_f32).to_bits(),
            "negating a folded zero keeps its sign"
        );
    }

    #[test]
    fn link_results() {
        assert_eq!(at("1 + 1", 13).failure(), None);
        for source in ["array.foo", "array.foo[v.x]", "v.x = array.test;"] {
            let compiled = at(source, 13);
            assert_eq!(
                compiled.failure(),
                Some(CompileFailure::UsesArrays),
                "{source}"
            );
            assert!(compiled.expr().is_none() && compiled.parsed(), "{source}");
        }
        for source in [
            "geometry.default",
            "texture.a == texture.b",
            "v.x ? material.a : material.b",
        ] {
            let compiled = at(source, 13);
            assert_eq!(
                compiled.failure(),
                Some(CompileFailure::UsesResources),
                "{source}"
            );
            assert!(
                compiled.expr().is_none() && compiled.diagnostics().is_empty(),
                "{source}"
            );
        }
    }

    #[test]
    fn side_effects() {
        let effects = |source: &str, include_random: bool| {
            at(source, 13)
                .expr()
                .cloned()
                .expect("expr")
                .has_side_effects(include_random)
        };
        assert!(effects("variable.foo = 3;", false));
        assert!(effects(
            "(3 > variable.foo) ? {variable.bar = 10; return 1;} : {return 0;};",
            true
        ));
        assert!(!effects("query.position(0)", true));
        assert!(effects("2 + math.random(1, 2)", true));
        assert!(!effects("2 + math.random(1, 2)", false));
        assert!(at("v.x = 1;", 13).expr().cloned().expect("expr").assigns());
        assert!(!at("v.x + 1", 13).expr().cloned().expect("expr").assigns());
    }
}

mod parsing {
    //! Which texts the parser accepts or rejects, at which version, with which messages.

    use crate::common::compile_support::{at, client_at, constant, messages, messages_at};
    use molangx::catalog::{QueryAdmission, QueryDecl, QuerySetMask, Side};
    use molangx::compile::{CompileFailure, CompileOptions, Expr, compile};
    use molangx::ops::{ExpressionOp, OpSet};
    use molangx::version::{ExperimentMask, RawVersion};
    #[cfg(feature = "vm")]
    use molangx::vm::{NoHostEnv, Value, VariableName};

    fn rejected_with_zero(source: &str, version: i16) -> Vec<String> {
        let compiled = at(source, version);
        assert_eq!(
            compiled.failure(),
            Some(CompileFailure::Rejected),
            "{source} at {version}"
        );
        assert_eq!(
            compiled.expr_or_zero().and_then(Expr::as_constant),
            Some(0.0),
            "{source}: a failed parse is worth 0"
        );
        messages(&compiled)
    }

    /// The statement pass enters only `loop`, `for_each`, `(` and `{`: a statement list in `[ ]` or
    /// a query's argument `( )` stays ungrouped and fails.
    #[test]
    fn passes_recurse_into_nested_lists() {
        let malformed = [
            "Malformed Semicolon ';' expression. It has 0 children but should have between 1 and -1",
        ];
        assert_eq!(messages_at("[v.x = 1;];", 13), malformed);
        assert_eq!(messages_at("v.y = [v.x = 1;];", 13), malformed);
        assert_eq!(messages_at("v.y = q.is_baby((v.a = 1;));", 13), malformed);
    }

    /// Tree building, the allowed-operation check, folding, then validation: an earlier stage's
    /// failure hides what a later one would report.
    #[test]
    fn core_parse_stage_order() {
        let no_min = CompileOptions {
            allowed_ops: OpSet::all().without(ExpressionOp::Min),
            ..client_at(13)
        };
        // Tree building before the allowed-operation check.
        assert_eq!(
            messages(&compile("math.min(1, 2) + ", &no_min)),
            ["Error: binary Add '+' operator at end of expression"]
        );
        // The allowed-operation check before folding (whose argument count would fail).
        assert_eq!(
            messages(&compile("math.min(1)", &no_min)),
            ["Expression uses operation Min 'math.min' which is not allowed in this context"]
        );
        // Folding before validation (whose root finding would be the unreachable statement).
        assert_eq!(
            messages_at("return 1; math.min(1);", 13),
            ["Unexpected number of parameters to Min 'math.min' function - expected 2, found 1."]
        );
        assert_eq!(
            messages_at("return 1; math.min(1, 2);", 13),
            ["Error: unreachable statements after Return 'return'."]
        );
    }

    /// Without `=` or `;` the pass that requires the closing `;` does not run.
    #[test]
    fn passes_run_only_for_their_operators() {
        assert!(at("v.x + 1", 13).parses_cleanly());
        assert!(at("v.x ? 1 : 2", 13).parses_cleanly());
        assert_eq!(
            messages_at("v.x = 1", 13),
            ["Error: complex expressions (contains either '=' or ';') must end with a ';'"]
        );
        assert_eq!(
            messages_at("1; 2", 13),
            ["Error: complex expressions (contains either '=' or ';') must end with a ';'"]
        );
    }

    /// An empty argument section is #13.
    #[test]
    fn calls_take_their_argument_section() {
        assert_eq!(
            messages_at("q.is_baby()", 13),
            [
                "Error: Query Function 'query.' or 'q.' operators with no params should not use parentheses"
            ]
        );
        assert_eq!(
            messages_at("loop()", 13),
            ["Error: Loop 'loop' operators with no params should not use parentheses"]
        );
        assert_eq!(
            messages_at("for_each()", 13),
            ["Error: For Each 'for_each' operators with no params should not use parentheses"]
        );
    }

    /// The `->` pass runs before the math-call pass, so a call's result needs parentheses to be a
    /// `->` base.
    #[test]
    fn pointer_bases() {
        assert_eq!(
            messages_at("math.floor(1.5)->v.hp", 13),
            ["Error: Floor 'math.floor' operator not followed by parenthesis section"]
        );
    }

    #[test]
    fn null_coalescing_chain_at_the_root() {
        let lhs = "Error: found left-hand-side of ?? expression that isn't a direct-variable reference - this is unsupported at this time.";
        for version in [2, 13] {
            for source in [
                "v.a ?? v.b ?? 5",
                "(v.a ?? v.b) ?? 5",
                "1 + (v.a ?? v.b ?? 5)",
                "(1 ?? 2) * 2",
            ] {
                assert_eq!(
                    rejected_with_zero(source, version),
                    [lhs],
                    "{source} at {version}"
                );
            }
        }
    }

    /// Logged (#44) and kept, and works fully at run time; a right-nested chain is silent.
    #[cfg(feature = "vm")]
    #[test]
    fn null_coalescing_chain_below_the_root() {
        let lhs = "Error: found left-hand-side of ?? expression that isn't a direct-variable reference - this is unsupported at this time.";
        let run = |source: &str, set: &[(&str, f32)]| {
            let compiled = at(source, 13);
            assert_eq!(compiled.failure(), None, "{source}");
            let logged = messages(&compiled);
            let mut env = NoHostEnv::new();
            for (name, value) in set {
                env.variables
                    .set(VariableName::new(name), Value::Float(*value));
            }
            let value = compiled
                .expr()
                .cloned()
                .expect("expr")
                .eval_f32(&mut env.cx());
            assert!(
                env.sink.is_empty(),
                "{source}: no run-time message, got {:?}",
                env.sink.take()
            );
            (value, logged)
        };
        let chain = "return v.a ?? v.b ?? 5;";
        assert_eq!(run(chain, &[]), (5.0, vec![lhs.to_owned()]));
        assert_eq!(run(chain, &[("b", 7.0)]).0, 7.0);
        assert_eq!(run(chain, &[("a", 3.0)]).0, 3.0);
        let temp = "t.x = v.a ?? v.b ?? 5; return t.x;";
        assert_eq!(run(temp, &[]), (5.0, vec![lhs.to_owned()]));
        assert_eq!(run(temp, &[("a", 3.0)]).0, 3.0);
        let nested = "v.a ?? (v.b ?? 5)";
        assert_eq!(run(nested, &[]), (5.0, vec![]));
        assert_eq!(run(nested, &[("b", 7.0)]).0, 7.0);
        assert_eq!(run(nested, &[("a", 3.0)]).0, 3.0);
        for source in [
            "math.abs(1 ?? 2)",
            "1 ? (1 ?? 2) : 0",
            "(1 ?? 2) == 1",
            "!(1 ?? 2)",
        ] {
            assert_eq!(run(source, &[]).1, [lhs], "{source}");
        }
        assert_eq!(
            run("return 1 + (1 ?? 2);", &[]),
            (2.0, vec![lhs.to_owned()])
        );
    }

    #[test]
    fn binary_operator_without_a_right_operand() {
        for (source, friendly) in [
            ("0*", "Multiply '*'"),
            ("1*", "Multiply '*'"),
            ("1/", "Divide '/'"),
            ("1+", "Add '+'"),
            ("1<", "Less Than '<'"),
            ("1<=", "Less Than Or Equal '<='"),
            ("1==", "Logical Equal '=='"),
            ("1!=", "Logical Not Equal '!='"),
            ("1&&", "Logical And '&&'"),
            ("1||", "Logical Or '||'"),
            ("v.a??", "Null Coalescing '??'"),
            ("v.a->", "Pointer '->'"),
        ] {
            let lines = rejected_with_zero(source, 13);
            assert_eq!(
                lines[0],
                format!("Error: binary {friendly} operator at end of expression"),
                "{source}"
            );
        }
        assert_eq!(
            messages_at("v.a =;", 13)[0],
            "Error: binary Assignment '=' operator at end of expression"
        );
    }

    /// #32 from version 4; before, the first child is used.
    #[test]
    fn sections_with_several_children() {
        for version in [4, 13] {
            assert_eq!(
                rejected_with_zero("(1 2)", version),
                [
                    "Error: Left Parenthesis '(' optimization expected only one child operation but found 2"
                ],
                "version {version}"
            );
            assert_eq!(
                rejected_with_zero("[1 2]", version),
                [
                    "Error: Left Bracket '[' optimization expected only one child operation but found 2"
                ],
                "version {version}"
            );
        }
    }

    /// From version 4 an empty or blank expression logs #1; below, it fails silently.
    #[test]
    fn empty_expressions() {
        for source in ["", " ", "\t\n\r "] {
            for version in [4, 13] {
                assert_eq!(
                    rejected_with_zero(source, version),
                    ["No tokens found in expression"],
                    "{source:?} at {version}"
                );
            }
            for version in [-1, 0, 3] {
                assert!(
                    rejected_with_zero(source, version).is_empty(),
                    "{source:?} at {version}"
                );
            }
        }
    }

    #[test]
    fn deep_errors_reject() {
        for source in [
            "v.x = math.min(1); v.y = 1;",
            "v.x = 'a' + 1;",
            "v.x = 1; loop(2, {v.y = (1 + ($));});",
            "v.c ? {v.x = 1; v.y = (2 +);};",
            "v.c ? {v.x = (1 2);};",
        ] {
            let lines = rejected_with_zero(source, 13);
            assert!(!lines.is_empty(), "{source}");
        }
    }

    /// Counted while folding, at every version.
    #[test]
    fn math_function_arity() {
        for version in [-1, 0, 1, 2, 3, 6, 13] {
            for (source, name, expected, found) in [
                ("math.min(1)", "Min 'math.min'", 2, 1),
                ("math.max(3)", "Max 'math.max'", 2, 1),
                ("math.pow(2)", "Power 'math.pow'", 2, 1),
                ("math.mod(7)", "Mod 'math.mod'", 2, 1),
                ("math.atan2(1, 2, 3)", "atan2 'math.atan2'", 2, 3),
                ("math.copy_sign(1)", "Copy Sign 'math.copy_sign'", 2, 1),
                ("math.random(1)", "Random 'math.random'", 2, 1),
                (
                    "math.random_integer(1, 2, 3)",
                    "Random Integer 'math.random_integer'",
                    2,
                    3,
                ),
                ("math.clamp(1, 2)", "Clamp 'math.clamp'", 3, 2),
                ("math.lerp(1, 2)", "Lerp 'math.lerp'", 3, 2),
                (
                    "math.ease_in_quad(1, 2, 3, 4)",
                    "Ease In Quad 'math.ease_in_quad'",
                    3,
                    4,
                ),
            ] {
                assert_eq!(
                    rejected_with_zero(source, version),
                    [format!(
                        "Unexpected number of parameters to {name} function - expected {expected}, found {found}."
                    )],
                    "{source} at {version}"
                );
            }
        }
    }

    /// Two arguments log #26, none #24.
    #[test]
    fn one_argument_math_functions() {
        for name in ["sin", "abs", "floor", "sqrt"] {
            assert_eq!(
                rejected_with_zero(&format!("math.{name}(1, 2)"), 13),
                [
                    "Error: Unexpected Comma ',' operator not inside an arguments list for a query, loop, or math function"
                ],
                "{name}"
            );
            let lines = rejected_with_zero(&format!("math.{name}()"), 13);
            assert_eq!(lines.len(), 1, "{name}: {lines:?}");
            assert!(
                lines[0]
                    .starts_with("Malformed Left Parenthesis '(' expression. It has 0 children"),
                "{name}: {lines:?}"
            );
        }
    }

    /// From version 3 arithmetic, comparison, unary and math nodes require numeric direct children
    /// (#36); a query that returns no number is refused at every version (#37).
    #[test]
    fn numeric_children() {
        for (source, operator, child) in [
            ("'a' + 1", "Add '+'", "String '''"),
            (
                "geometry.a * 2",
                "Multiply '*'",
                "Geometry Variable 'geometry.'",
            ),
            ("loop(2, {v.x = 1;}) < 1;", "Less Than '<'", "Loop 'loop'"),
            (
                "-for_each(t.x, v.a, {v.x = 1;});",
                "Negate '-'",
                "For Each 'for_each'",
            ),
            (
                "math.abs(v.x = 1);",
                "Absolute Value 'math.abs'",
                "Assignment '='",
            ),
        ] {
            let lines = rejected_with_zero(source, 3);
            assert_eq!(lines.len(), 1, "{source}: {lines:?}");
            assert!(
                lines[0].starts_with(&format!(
                    "'{operator}' expression cannot take a '{child}' argument"
                )),
                "{source}: {lines:?}"
            );
            assert!(
                at(source, 2).parses_cleanly(),
                "{source} at 2: {:?}",
                messages_at(source, 2)
            );
        }
        let default = QueryAdmission::Sets(QuerySetMask::DEFAULT);
        let name = molangx::stdlib::queries(Side::Client)
            .iter()
            .find(|decl| {
                !decl
                    .shape()
                    .returns
                    .intersects(molangx::catalog::ReturnType::NUMBER)
                    && decl
                        .resolve(RawVersion(0), &default, ExperimentMask::empty())
                        .is_some()
                    && decl
                        .resolve(RawVersion(13), &default, ExperimentMask::empty())
                        .is_some()
            })
            .map(QueryDecl::name)
            .expect("a query that returns no number");
        for version in [0, 2, 13] {
            let lines = rejected_with_zero(&format!("{name} + 1"), version);
            assert_eq!(lines.len(), 1, "{name} at {version}: {lines:?}");
            assert_eq!(
                lines[0],
                "Add '+' expressions may only contain query functions that return numbers",
                "{name} at {version}"
            );
        }
    }

    #[test]
    fn numeric_check_is_shallow() {
        for source in [
            "(1 ? 'a' : 2) + 1",
            "math.abs(1 ? 'a' : 2)",
            "-(v.x ?? 'a')",
        ] {
            assert!(
                at(source, 13).parses_cleanly(),
                "{source}: {:?}",
                messages_at(source, 13)
            );
        }
    }

    #[test]
    fn string_operands_where_allowed() {
        for version in [-1, 0, 3, 6, 13] {
            for source in [
                "'abc' == 'abc'",
                "v.x != 'abc'",
                "v.x ? 1 : 'b'",
                "v.x ? 'a'",
                "v.x ?? 'b'",
                "v.x = 'abc';",
            ] {
                let compiled = at(source, version);
                assert!(
                    compiled.parses_cleanly(),
                    "{source} at {version}: {:?}",
                    messages(&compiled)
                );
            }
        }
    }

    /// Below version 3 the post-op is folded silently onto a loop, `for_each`, assignment, `break`
    /// or `continue`; from 3, #36 refuses it. A statement list is an operand at every version.
    #[test]
    fn arithmetic_on_statement_nodes() {
        assert!(at("for_each(t.x, v.a, {v.x = 1;}) * 2;", 2).parses_cleanly());
        for source in [
            "loop(3, {v.x = 1;}) + 1;",
            "for_each(t.x, v.a, {v.x = 1;}) * 2;",
        ] {
            let lines = rejected_with_zero(source, 3);
            assert_eq!(lines.len(), 1, "{source}: {lines:?}");
            assert!(
                lines[0].contains("expression cannot take a"),
                "{source}: {lines:?}"
            );
        }
        for version in [0, 3, 13] {
            for source in ["v.y = {v.x = 1;} + 1;", "v.y = -{v.x = 1;};"] {
                assert!(
                    at(source, version).parses_cleanly(),
                    "{source} at {version}: {:?}",
                    messages_at(source, version)
                );
            }
        }
    }

    #[test]
    fn interpolation_functions_fold() {
        for source in [
            "math.lerp(0, 10, 0.25)",
            "math.lerprotate(10, 350, 0.5)",
            "math.inverse_lerp(0, 10, 2.5)",
            "math.ease_in_quad(0, 10, 0.5)",
            "math.ease_out_cubic(0, 10, 0.5)",
            "math.ease_in_out_back(0, 10, 0.5)",
            "math.ease_out_bounce(0, 10, 0.5)",
            "math.min_angle(5 + 10)",
        ] {
            let compiled = at(source, 13);
            assert!(
                compiled.expr().and_then(Expr::as_constant).is_some(),
                "{source}"
            );
        }
        assert_eq!(constant("math.lerp(0, 10, 0.25)"), 2.5);
        assert_eq!(constant("math.inverse_lerp(0, 10, 2.5)"), 0.25);
        assert_eq!(constant("math.min_angle(5 + 10)"), 15.0);
    }

    #[test]
    fn link_folds_constants() {
        let float = at("42.7", 13);
        assert_eq!(float.failure(), None);
        assert_eq!(float.expr().and_then(Expr::as_constant), Some(42.7));
        let string = at("'abc'", 13);
        assert_eq!(string.failure(), None);
        let string = string.expr().cloned().expect("expr");
        assert!(string.is_constant() && string.as_constant().is_none());
        let program = at("v.x + 1", 13).expr().cloned().expect("expr");
        assert!(!program.is_constant() && program.as_constant().is_none());
    }
}

mod strings {
    //! String literals through a compile and, with the evaluator, at run time.

    use crate::common::compile_support::{at, messages_at};
    use molangx::compile::CompileFailure;

    /// The text of #36 for an operator applied to a string literal.
    fn non_numerical(operator: &str) -> String {
        format!(
            "'{operator}' expression cannot take a 'String '''' argument. It only supports numerical arguments."
        )
    }

    #[test]
    fn string_operands_fail_with_36_from_version_3() {
        let rows = [
            ("!'a'", "Logical Not '!'"),
            ("-'a'", "Negate '-'"),
            ("'a' < 'b'", "Less Than '<'"),
            ("'abc' * 2", "Multiply '*'"),
            ("'a' + 3", "Add '+'"),
        ];
        for version in 3..=13 {
            for (source, operator) in rows {
                assert_eq!(
                    at(source, version).failure(),
                    Some(CompileFailure::Rejected),
                    "{source:?} at {version}"
                );
                assert_eq!(
                    messages_at(source, version).first(),
                    Some(&non_numerical(operator)),
                    "{source:?} at {version}"
                );
            }
        }
    }

    #[cfg(feature = "vm")]
    mod eval {
        use molangx::compile::CompileFailure;
        use molangx::hash::HashedStr;
        use molangx::vm::{NoHost, NoHostEnv, Value};

        use super::{at, non_numerical};

        fn eval(source: &str, version: i16) -> Value<NoHost> {
            let compiled = at(source, version);
            assert_eq!(
                compiled.failure(),
                None,
                "{source:?} at {version}: {:?}",
                compiled.diagnostics()
            );
            let mut env = NoHostEnv::new();
            let value = compiled
                .expr()
                .cloned()
                .expect("compiled")
                .eval(&mut env.cx());
            assert!(
                env.sink.messages().is_empty(),
                "{source:?}: {:?}",
                env.sink.messages()
            );
            value
        }

        /// `''` is the hash 0 at run time, not the FNV offset basis.
        #[test]
        fn empty_string_literal_is_hash_zero() {
            assert_eq!(eval("''", 13), Value::Hash(HashedStr::EMPTY));
            assert_eq!(
                eval("v.s = ''; return v.s;", 13),
                Value::Hash(HashedStr::EMPTY)
            );
            assert_eq!(eval("v.s = ''; return v.s == 0;", 13), Value::ONE);
        }

        /// Reads the low 32 bits of the hash as an `f32`: at version 2 on a literal, at any version
        /// through a variable.
        #[test]
        fn string_arithmetic_reinterprets_the_low_bits() {
            let low = f32::from_bits(HashedStr::new("a").as_u64() as u32);
            for version in [2, 13] {
                let Value::Float(x) = eval("v.s = 'a'; v.one = 1; return v.s * v.one;", version)
                else {
                    panic!("not a float")
                };
                assert_eq!(x.to_bits(), low.to_bits(), "version {version}");
            }
            let Value::Float(x) = eval("v.one = 1; return 'a' * v.one;", 2) else {
                panic!("not a float")
            };
            assert_eq!(x.to_bits(), low.to_bits());
        }

        /// At version 2 `'a' + 3` and `3 + 'a'` are the hash of `'a'`, and `'a' + 'b'` is about 0.
        #[test]
        fn string_arithmetic_at_versions_2_and_3() {
            let a = Value::Hash(HashedStr::new("a"));
            assert_eq!(eval("'a' + 3", 2), a);
            assert_eq!(eval("3 + 'a'", 2), a);
            // The sum of the two hashes' low bits as floats.
            let Value::Float(sum) = eval("'a' + 'b'", 2) else {
                panic!("not a float")
            };
            assert!(sum.abs() <= 1e-6, "{sum}");
            for source in ["'a' + 3", "3 + 'a'", "'a' + 'b'"] {
                let compiled = at(source, 3);
                assert_eq!(
                    compiled.failure(),
                    Some(CompileFailure::Rejected),
                    "{source:?}"
                );
                assert_eq!(
                    super::messages_at(source, 3).first(),
                    Some(&non_numerical("Add '+'")),
                    "{source:?}"
                );
            }
        }
    }
}

mod versions {
    //! The version of a compile: the lexer ignores it, `Invalid` takes the version-0 branches.

    #[cfg(feature = "vm")]
    use crate::common::compile_support::client_expr_at;
    use crate::common::compile_support::{at, client_at, messages_at, server_at};
    use molangx::compile::{CompileFailure, compile};
    use molangx::diag::{DiagCode, Severity};
    #[cfg(feature = "vm")]
    use molangx::vm::NoHostEnv;

    #[test]
    fn unknown_tokens_do_not_depend_on_the_version() {
        for source in ["foo", "1 $"] {
            let latest = messages_at(source, 13);
            assert!(
                !latest.is_empty(),
                "{source:?} must report an unknown token"
            );
            assert!(
                latest.iter().any(|m| m.starts_with("unrecognized token")),
                "{source:?}: {latest:?}"
            );
            for version in -1..=13 {
                assert_eq!(
                    messages_at(source, version),
                    latest,
                    "{source:?} at version {version}"
                );
                assert_eq!(
                    at(source, version).failure(),
                    Some(CompileFailure::Rejected),
                    "{source:?} at version {version}"
                );
            }
        }
    }

    #[test]
    fn invalid_parses_string_arithmetic_like_version_zero() {
        for version in [-1, 0, 2] {
            assert_eq!(at("'a' + 3", version).failure(), None, "version {version}");
        }
        for version in [3, 13] {
            assert_eq!(
                at("'a' + 3", version).failure(),
                Some(CompileFailure::Rejected),
                "version {version}"
            );
        }
    }

    /// At `Invalid` a negative run-time divisor divides as before the version-7 fix.
    #[cfg(feature = "vm")]
    #[test]
    fn invalid_divides_like_version_zero() {
        let source = "v.x = 5; v.z = -1; return v.x / v.z;";
        let eval =
            |version: i16| client_expr_at(source, version).eval_f32(&mut NoHostEnv::new().cx());
        assert_eq!(eval(6), 5.0);
        assert_eq!(eval(-1), 5.0);
        assert_eq!(eval(0), 5.0);
        assert_eq!(eval(7), -5.0);
        assert_eq!(eval(13), -5.0);
    }

    /// An `Invalid` version is an Info; the text compiles with the version-0 rules and no query
    /// resolves.
    #[test]
    fn invalid_version() {
        let invalid = client_at(-1);
        let number = compile("'a' + 1", &invalid);
        assert_eq!(
            number.failure(),
            None,
            "below version 3 string arithmetic is not checked"
        );
        assert!(
            number
                .diagnostics()
                .iter()
                .any(|d| d.code() == DiagCode::InvalidVersion && d.severity() == Severity::Info)
        );
        let at_zero = compile("'a' + 1", &server_at(0));
        assert_eq!(at_zero.failure(), number.failure());
        assert_eq!(
            compile("q.is_baby", &invalid).failure(),
            Some(CompileFailure::Rejected)
        );
    }
}

mod restrictions {
    //! Restricted contexts: query sets, allow-lists, the allowed-operation mask, the raw version
    //! and the side.

    use crate::common::compile_support::{client_at, messages, server_at};
    use molangx::catalog::{QueryAdmission, QueryAllowList, QuerySetMask, Side};
    use molangx::compile::{CompileFailure, CompileOptions, Deviations, compile};
    use molangx::diag::{DiagCode, Severity};
    use molangx::json::MolangSource;
    use molangx::ops::OpSet;
    use molangx::stdlib::query;
    use molangx::version::{ExperimentMask, MolangVersion, RawVersion};

    fn unresolved(name: &str, rest: &str) -> Vec<String> {
        vec![
            format!("Failed to resolve query {name}.{UNRESOLVED}"),
            format!("unrecognized token: {rest}"),
        ]
    }

    const UNRESOLVED: &str =
        "  Either the query does not exist or it is not supported in this context.";

    #[test]
    fn default_options() {
        let options = CompileOptions::server(MolangVersion::LATEST);
        assert_eq!(
            options.admission,
            QueryAdmission::Sets(QuerySetMask::DEFAULT)
        );
        assert_eq!(options.allowed_ops, OpSet::all());
        assert_eq!(options.raw_version, RawVersion(13));
        assert_eq!(options.catalog.side(), Side::Server);
        assert!(!options.keep_source);
    }

    #[test]
    fn query_sets() {
        let default = CompileOptions::server(MolangVersion::LATEST);
        let in_sets = |sets| CompileOptions {
            admission: QueryAdmission::Sets(sets),
            ..default.clone()
        };
        assert_eq!(compile("query.is_baby", &default).failure(), None);
        // `query.noise` is in `world_gen` only, `query.any_tag` in `tags` only.
        assert_eq!(
            messages(&compile("query.noise(1, 2)", &default)),
            unresolved("query.noise", "query.noise(1, 2)")
        );
        assert_eq!(
            messages(&compile("q.any_tag('x')", &default)),
            unresolved("query.any_tag", "q.any_tag('x')")
        );
        assert_eq!(
            compile("query.noise(1, 2)", &in_sets(QuerySetMask::WORLD_GEN)).failure(),
            None
        );
        assert_eq!(
            compile("q.any_tag('x')", &in_sets(QuerySetMask::TAGS)).failure(),
            None
        );
        assert_eq!(
            messages(&compile("query.is_baby", &in_sets(QuerySetMask::WORLD_GEN))),
            unresolved("query.is_baby", "query.is_baby")
        );
        assert_eq!(
            messages(&compile("query.is_baby", &in_sets(QuerySetMask::empty()))),
            unresolved("query.is_baby", "query.is_baby")
        );
        assert_eq!(
            compile("math.sin(v.x) + 1", &in_sets(QuerySetMask::empty())).failure(),
            None
        );
    }

    /// An allow-list replaces the sets.
    #[test]
    fn allow_lists() {
        // `without_assignments_or_random` keeps the dice allowed.
        let server = molangx::stdlib::queries(Side::Server);
        let allowed = QueryAllowList::new(server, [query::BLOCK_STATE]).expect("declared");
        let block = CompileOptions {
            admission: QueryAdmission::Only(allowed.clone()),
            allowed_ops: OpSet::all().without_assignments_or_random(),
            ..CompileOptions::server(MolangVersion::LATEST)
        };
        assert_eq!(block.admission, QueryAdmission::Only(allowed));
        assert_eq!(
            compile("query.block_state('facing') == 'west'", &block).failure(),
            None
        );
        assert_eq!(
            messages(&compile("q.is_baby", &block)),
            unresolved("query.is_baby", "q.is_baby")
        );
        assert_eq!(
            messages(&compile("math.random(0, 1)", &block)),
            ["Expression uses operation Random 'math.random' which is not allowed in this context"]
        );
        assert_eq!(
            compile("math.die_roll(1, 0, 1)", &block).failure(),
            None,
            "die_roll stays allowed"
        );

        let allowed = QueryAllowList::new(server, [query::HAD_COMPONENT_GROUP]).expect("declared");
        let property = CompileOptions {
            admission: QueryAdmission::Only(allowed.clone()),
            allowed_ops: OpSet::all().without_assignments(),
            ..CompileOptions::server(MolangVersion::LATEST)
        };
        assert_eq!(
            compile(
                "q.had_component_group('minecraft:baby') ? 1 : math.random(0, 1)",
                &property
            )
            .failure(),
            None
        );
        assert_eq!(
            messages(&compile("v.x = 1;", &property)),
            ["Expression uses operation Assignment '=' which is not allowed in this context"]
        );
        // The allow-listed query still has to exist at the version.
        let old = CompileOptions {
            admission: QueryAdmission::Only(allowed),
            ..CompileOptions::from_raw_version(server.clone(), RawVersion(-1))
        };
        assert_eq!(
            compile("q.had_component_group('x')", &old).failure(),
            Some(CompileFailure::Rejected)
        );
    }

    /// Checked before anything is optimised.
    #[test]
    fn operation_mask() {
        let no_loops = CompileOptions {
            allowed_ops: OpSet::all().without(molangx::ops::ExpressionOp::Loop),
            ..CompileOptions::server(MolangVersion::LATEST)
        };
        assert_eq!(
            messages(&compile("loop(2, {v.x = 1;});", &no_loops)),
            ["Expression uses operation Loop 'loop' which is not allowed in this context"]
        );
        // A denied operation is reported even when the expression has another error the optimiser
        // would find.
        assert_eq!(
            messages(&compile("loop(2);", &no_loops)),
            ["Expression uses operation Loop 'loop' which is not allowed in this context"]
        );
        // Tree-building errors come first.
        assert_eq!(
            messages(&compile("loop(2, {v.x = 1;})", &no_loops)),
            ["Error: complex expressions (contains either '=' or ';') must end with a ';'"]
        );
    }

    /// Parser gates use the effective version and queries the raw one, so nothing outside 0..=13
    /// resolves a query.
    #[test]
    fn raw_version() {
        for raw in [-1_i16, -7, 14, 200] {
            let options = client_at(raw);
            assert_eq!(options.raw_version, RawVersion(raw));
            let compiled = compile("query.is_baby", &options);
            assert_eq!(
                messages(&compiled),
                unresolved("query.is_baby", "query.is_baby"),
                "raw version {raw}"
            );
            assert_eq!(
                compile("v.x + 1", &options).failure(),
                None,
                "raw version {raw}"
            );
        }
        // Above 13 the parser gates are those of 13 …
        assert_eq!(server_at(14).version(), MolangVersion::LATEST);
        assert_eq!(
            compile("'a' + 1", &server_at(14)).failure(),
            Some(CompileFailure::Rejected)
        );
        // … and below 0 those of version 0 and earlier.
        assert_eq!(server_at(-7).version(), MolangVersion::Invalid);
        assert_eq!(compile("'a' + 1", &server_at(-7)).failure(), None);

        let source = MolangSource::object("query.is_baby", 9);
        assert_eq!(
            CompileOptions::for_source(molangx::stdlib::queries(Side::Server).clone(), &source)
                .expect("a source with a version")
                .raw_version,
            RawVersion(9)
        );
        assert_eq!(
            compile(
                source.as_str(),
                &CompileOptions::for_source(
                    molangx::stdlib::queries(Side::Server).clone(),
                    &source
                )
                .expect("a source with a version")
            )
            .failure(),
            None
        );
    }

    #[test]
    fn version_ranges() {
        // `query.block_property` ends at version 9.
        let name = "query.block_property('x')";
        assert_eq!(compile(name, &server_at(9)).failure(), None);
        let compiled = compile(name, &server_at(10));
        assert_eq!(
            messages(&compiled),
            unresolved("query.block_property", "query.block_property('x')")
        );
        // An Info says the name exists at other versions.
        assert!(
            compiled
                .diagnostics()
                .iter()
                .any(|d| d.code() == DiagCode::QueryDeprecated && d.severity() == Severity::Info)
        );
        assert_eq!(
            compile("query.block_state('x')", &server_at(9)).failure(),
            None
        );
    }

    /// Client-only queries compiled for the server are flagged with an Info.
    #[test]
    fn side() {
        let server = CompileOptions::server(MolangVersion::LATEST);
        let client = CompileOptions {
            catalog: molangx::stdlib::queries(Side::Client).clone(),
            ..server.clone()
        };
        let on_server = compile("q.is_on_screen", &server);
        assert_eq!(
            messages(&on_server),
            unresolved("query.is_on_screen", "q.is_on_screen")
        );
        // An Info says why it did not resolve, before the two language lines.
        let ours: Vec<_> = on_server
            .diagnostics()
            .iter()
            .filter(|d| d.language_message().is_none())
            .map(|d| (d.code(), d.severity(), d.span().clone()))
            .collect();
        assert_eq!(ours, [(DiagCode::QueryClientOnly, Severity::Info, 0..14)]);
        assert!(
            on_server.diagnostics()[0]
                .message()
                .contains("not registered on the dedicated server")
        );
        let lenient = CompileOptions {
            deviations: Deviations::NONE,
            ..server.clone()
        };
        let no_deviations = compile("q.is_on_screen", &lenient);
        assert!(
            no_deviations
                .diagnostics()
                .iter()
                .all(|d| d.language_message().is_some()),
            "no lint with the deviation off"
        );
        assert_eq!(compile("q.is_on_screen", &client).failure(), None);
        // A query the server lacks that does not resolve on the client either gets no such lint.
        let no_sets = CompileOptions {
            admission: QueryAdmission::Sets(QuerySetMask::empty()),
            ..server.clone()
        };
        let elsewhere = compile("q.is_on_screen", &no_sets);
        assert!(
            elsewhere
                .diagnostics()
                .iter()
                .all(|d| d.language_message().is_some()),
            "{:?}",
            elsewhere.diagnostics()
        );

        let name = molangx::stdlib::queries(Side::Server).iter().find(|decl| {
            decl.shape().side == molangx::catalog::QuerySide::CLIENT
                && decl
                    .resolve(
                        RawVersion(13),
                        &QueryAdmission::Sets(QuerySetMask::DEFAULT),
                        ExperimentMask::empty(),
                    )
                    .is_some()
        });
        let name = name.expect("the catalogue has client-only queries").name();
        let on_server = compile(name, &server);
        assert_eq!(
            on_server.failure(),
            None,
            "{name}: a client-only query compiles with this crate's server catalogue"
        );
        assert!(
            on_server
                .diagnostics()
                .iter()
                .any(|d| d.code() == DiagCode::QueryClientOnly && d.severity() == Severity::Info),
            "{name}"
        );
        assert!(compile(name, &client).diagnostics().is_empty(), "{name}");
    }

    #[test]
    fn query_arity_is_a_lint() {
        let options = CompileOptions::server(MolangVersion::LATEST);
        // `query.log` is declared with one argument.
        let compiled = compile("q.log(7, 8)", &options);
        assert_eq!(compiled.failure(), None);
        assert!(compiled.parses_cleanly());
        assert_eq!(compiled.diagnostics().len(), 1);
        assert_eq!(
            (
                compiled.diagnostics()[0].code(),
                compiled.diagnostics()[0].severity()
            ),
            (DiagCode::QueryArity, Severity::Warning)
        );
        assert!(compile("q.log(7)", &options).diagnostics().is_empty());
        let no_deviations = CompileOptions {
            deviations: Deviations::NONE,
            ..options
        };
        assert!(
            compile("q.log(7, 8)", &no_deviations)
                .diagnostics()
                .is_empty()
        );
    }

    #[test]
    fn keep_source() {
        let options = CompileOptions::server(MolangVersion::LATEST);
        assert_eq!(
            compile("V.X + 1", &options)
                .expr()
                .cloned()
                .expect("expr")
                .source(),
            None
        );
        let kept = compile(
            "V.X + 1",
            &CompileOptions {
                keep_source: true,
                ..options
            },
        );
        assert_eq!(
            kept.expr().cloned().expect("expr").source(),
            Some("V.X + 1")
        );
    }
}

mod queries {
    //! Query resolution as a compile result: the `q.` alias and what every kind of miss logs.

    use crate::common::compile_support::{message_ids, server_at};
    use molangx::catalog::{
        Arity, QueryAdmission, QueryAllowList, QueryDecl, QuerySetMask, QueryShape, QuerySide,
        ReturnType, Side,
    };
    use molangx::compile::{CompileFailure, CompileOptions, compile};
    use molangx::diag::DiagCode;
    use molangx::stdlib::query;
    use molangx::version::{ExperimentMask, MolangVersion};

    use crate::common;

    /// In any letter case; messages name the full query.
    #[test]
    fn q_is_a_lexer_alias() {
        let options = CompileOptions::server(MolangVersion::LATEST);
        for source in ["q.is_baby", "Q.IS_BABY", "query.is_baby", "QUERY.Is_Baby"] {
            let compiled = compile(source, &options);
            assert_eq!(compiled.failure(), None, "{source}");
            assert!(
                compiled.diagnostics().is_empty(),
                "{source}: {:?}",
                compiled.diagnostics()
            );
        }
        let long = compile("query.is_baby * 2", &options)
            .expr()
            .cloned()
            .expect("expr")
            .flags();
        let short = compile("q.is_baby * 2", &options)
            .expr()
            .cloned()
            .expect("expr")
            .flags();
        assert_eq!(long, short);
        let miss = compile("q.no_such_query", &options);
        assert!(
            miss.diagnostics()[0]
                .message()
                .starts_with("Failed to resolve query query.no_such_query."),
            "{:?}",
            miss.diagnostics()
        );
    }

    /// Every kind of miss rejects with #5 followed by `unrecognized token`, and nothing else.
    #[test]
    fn every_miss_is_message_5() {
        let latest = CompileOptions::server(MolangVersion::LATEST);
        let block_list =
            QueryAllowList::new(&latest.catalog, [query::BLOCK_STATE]).expect("declared");
        let world_gen = CompileOptions {
            admission: QueryAdmission::Sets(QuerySetMask::WORLD_GEN),
            ..latest.clone()
        };
        let listed = CompileOptions {
            admission: QueryAdmission::Only(block_list),
            ..latest.clone()
        };
        let cases: Vec<(&str, CompileOptions)> = vec![
            ("q.no_such_query", latest.clone()),
            ("q.noise(1, 2)", latest.clone()),
            ("q.any_tag('x')", latest.clone()),
            ("q.is_baby", world_gen),
            ("q.is_baby", listed),
            ("q.block_property('x')", server_at(10)),
            ("q.is_scenting", server_at(11)),
            ("q.is_on_screen", latest),
            ("q.is_baby", server_at(-1)),
        ];
        for (source, options) in cases {
            let compiled = compile(source, &options);
            assert_eq!(
                compiled.failure(),
                Some(CompileFailure::Rejected),
                "{source}"
            );
            assert_eq!(
                message_ids(&compiled),
                ["E05", "E02"],
                "{source}: {:?}",
                compiled.diagnostics()
            );
        }
    }

    #[test]
    fn experiment_off_is_message_5() {
        let options = CompileOptions {
            admission: QueryAdmission::Sets(common::REFERENCE_SETS),
            ..CompileOptions::new(common::reference_catalog().clone(), MolangVersion::LATEST)
        };
        let off = compile("query.experimental_test", &options);
        assert_eq!(off.failure(), Some(CompileFailure::Rejected));
        assert_eq!(message_ids(&off), ["E05", "E02"]);
        let enabled = CompileOptions {
            experiments: common::reference_experiments(),
            ..options
        };
        let on = compile("query.experimental_test", &enabled);
        assert_eq!(on.failure(), None);
    }

    #[test]
    fn an_enabled_experiment_is_not_blamed_for_a_set_miss() {
        let options = CompileOptions {
            experiments: common::reference_experiments(),
            ..CompileOptions::new(common::reference_catalog().clone(), MolangVersion::LATEST)
        };
        let compiled = compile("query.experimental_test", &options);
        assert_eq!(compiled.failure(), Some(CompileFailure::Rejected));
        assert_eq!(message_ids(&compiled), ["E05", "E02"]);
        assert!(
            compiled
                .diagnostics()
                .iter()
                .all(|d| d.code() != DiagCode::QueryExperiment),
            "{:?}",
            compiled.diagnostics()
        );
        assert_eq!(compiled.diagnostics().len(), 2);
        let disabled = CompileOptions {
            experiments: ExperimentMask::empty(),
            ..options
        };
        assert_eq!(ExperimentMask::empty(), disabled.experiments);
    }

    #[test]
    fn a_host_declared_query_compiles_like_a_built_in_one() {
        let my_name = QueryDecl::new(
            "query.my_name",
            QueryShape {
                args: Arity::between(0, 1),
                returns: ReturnType::STRING,
                side: QuerySide::CLIENT,
                ..QueryShape::DEFAULT
            },
        )
        .unwrap();
        let catalog = molangx::stdlib::queries(Side::Server)
            .extended([my_name])
            .unwrap();
        let options = CompileOptions::new(catalog.clone(), MolangVersion::LATEST);
        let compiled = compile("q.my_name == 'x'", &options);
        assert_eq!(compiled.failure(), None);
        assert_eq!(
            compiled
                .diagnostics()
                .iter()
                .map(|d| d.code())
                .collect::<Vec<_>>(),
            [DiagCode::QueryClientOnly],
            "the side lint: {:?}",
            compiled.diagnostics()
        );
        let expr = compiled.expr().cloned().expect("expr");
        assert_eq!(expr.catalog(), &catalog);
        assert_eq!(expr.queries().collect::<Vec<_>>(), ["query.my_name"]);
        // The return-type check: a string is no operand of `+`.
        let sum = compile("q.my_name + 1", &options);
        assert_eq!(sum.failure(), Some(CompileFailure::Rejected));
        assert!(
            sum.diagnostics().iter().any(|d| d.message()
                == "Add '+' expressions may only contain query functions that return numbers"),
            "{:?}",
            sum.diagnostics()
        );
        let arity = compile("q.my_name(1, 2)", &options);
        assert!(
            arity
                .diagnostics()
                .iter()
                .any(|d| d.code() == DiagCode::QueryArity),
            "{:?}",
            arity.diagnostics()
        );
        assert_eq!(
            compile("q.my_name", &CompileOptions::server(MolangVersion::LATEST)).failure(),
            Some(CompileFailure::Rejected)
        );
    }

    /// Each query once, in the order of its first call.
    #[test]
    fn an_expression_names_the_queries_it_calls() {
        let options = CompileOptions::client(MolangVersion::LATEST);
        let expr = compile(
            "q.is_baby + q.position(q.is_baby) + q.life_time + q.position(0)",
            &options,
        )
        .expr_or_zero()
        .cloned()
        .expect("expr");
        assert_eq!(
            expr.queries().collect::<Vec<_>>(),
            [query::IS_BABY, query::POSITION, query::LIFE_TIME]
        );
        assert_eq!(
            compile("1 + 2", &options)
                .expr_or_zero()
                .cloned()
                .expect("expr")
                .queries()
                .count(),
            0
        );
        assert_eq!(
            compile("q.no_such", &options)
                .expr_or_zero()
                .cloned()
                .expect("the constant 0")
                .queries()
                .count(),
            0
        );
        assert_eq!(QuerySetMask::DEFAULT, QuerySetMask::DEFAULT);
    }
}

mod compile_source {
    //! `compile_source` applies a `MolangSource`'s version to a field's options; `CompileOptions`
    //! keeps its gate and raw versions consistent.

    use crate::common::compile_support::server_at;
    use molangx::catalog::Side;
    use molangx::compile::{
        CompileFailure, CompileOptions, Compiled, Deviations, Expr, compile, compile_source,
    };
    use molangx::diag::{DiagCode, LanguageMessage, Severity};
    use molangx::json::MolangSource;
    use molangx::ops::OpSet;
    use molangx::version::{MolangVersion, RawVersion};

    /// `'a' + 1` is an error from version 3.
    const STRING_MATH: &str = "'a' + 1";

    #[test]
    fn compile_source_uses_the_source_version() {
        let field = CompileOptions::server(MolangVersion::LATEST);
        let old = MolangSource::string(STRING_MATH, 2);
        assert_eq!(compile_source(&old, &field).failure(), None);
        assert_eq!(
            compile_source(&MolangSource::string(STRING_MATH, 13), &field).failure(),
            Some(CompileFailure::Rejected)
        );
        // The object form's own version, raw: 14 gates like 13 and resolves no query.
        assert_eq!(
            compile_source(&MolangSource::object(STRING_MATH, 2), &field).failure(),
            None
        );
        let raw14 = compile_source(&MolangSource::object("query.is_baby", 14), &field);
        assert!(
            raw14
                .diagnostics()
                .iter()
                .any(|d| d.language_message() == Some(LanguageMessage::QueryUnresolved))
        );
        // `compile` keeps the options' version.
        assert_eq!(
            compile(old.as_str(), &field).failure(),
            Some(CompileFailure::Rejected)
        );
        assert_eq!(
            compile(
                old.as_str(),
                &CompileOptions::for_source(molangx::stdlib::queries(Side::Server).clone(), &old)
                    .expect("a source with a version")
            )
            .failure(),
            None
        );
    }

    /// Every version gate reads the compile's version, not the options': `compile_source` with the
    /// options at another version compiles and logs exactly as `compile` at the source's.
    #[test]
    fn every_gate_reads_the_compile_version() {
        let catalog = molangx::stdlib::queries(Side::Server);
        let at = |raw: i16| CompileOptions::from_raw_version(catalog.clone(), RawVersion(raw));
        let messages = |compiled: &Compiled| {
            compiled
                .diagnostics()
                .iter()
                .map(|d| d.message().into_owned())
                .collect::<Vec<_>>()
        };
        // The empty-input message, the second child of a section, a string argument of a sum.
        for text in ["", "1+(2 3)", "v.a + v.b + 'a'"] {
            for (source, field) in [(2, 13), (13, 2), (-1, 13)] {
                let through = compile_source(&MolangSource::object(text, source), &at(field));
                let direct = compile(text, &at(source));
                let what = format!("{text:?} at {source}, the options at {field}");
                assert_eq!(through.failure(), direct.failure(), "{what}");
                assert_eq!(messages(&through), messages(&direct), "{what}");
            }
        }
    }

    #[test]
    fn compile_source_keeps_the_field_restrictions() {
        let field = CompileOptions {
            allowed_ops: OpSet::all().without_assignments(),
            ..CompileOptions::server(MolangVersion::LATEST)
        };
        let compiled = compile_source(&MolangSource::string("v.x = 1;", 2), &field);
        assert_eq!(compiled.failure(), Some(CompileFailure::Rejected));
        assert_eq!(
            compiled.diagnostics()[0].language_message(),
            Some(LanguageMessage::OperationNotAllowed)
        );
        assert_eq!(
            compiled.expr_or_zero().cloned().map(|e| e.version()),
            Some(MolangVersion::V2)
        );
    }

    #[test]
    fn the_version_pair_cannot_disagree() {
        let opts = CompileOptions {
            raw_version: MolangVersion::V2.into(),
            ..CompileOptions::server(MolangVersion::LATEST)
        };
        assert_eq!(
            (opts.version(), opts.raw_version),
            (MolangVersion::V2, RawVersion(2))
        );
        assert_eq!(compile(STRING_MATH, &opts).failure(), None);
        for raw in [-7_i16, -1, 0, 5, 13, 14, 200] {
            let opts = CompileOptions {
                raw_version: RawVersion(raw),
                ..CompileOptions::server(MolangVersion::LATEST)
            };
            assert_eq!(
                (opts.version(), opts.raw_version),
                (RawVersion(raw).effective(), RawVersion(raw))
            );
            assert_eq!(opts, server_at(raw));
        }
        for version in (-1..=13).filter_map(MolangVersion::from_i16) {
            assert_eq!(CompileOptions::server(version), server_at(version.as_i16()));
        }
    }

    /// A string-form source compiled before its context version was applied gets its own warning,
    /// whatever the deviations, instead of the out-of-range one.
    #[test]
    fn an_unresolved_source_says_so() {
        let src = MolangSource::string_without_context("query.is_baby");
        for deviations in [Deviations::ALL, Deviations::NONE] {
            let field = CompileOptions {
                deviations,
                ..CompileOptions::server(MolangVersion::LATEST)
            };
            let compiled = compile_source(&src, &field);
            let first = &compiled.diagnostics()[0];
            assert_eq!(
                (first.code(), first.severity(), first.language_message()),
                (DiagCode::InvalidVersion, Severity::Warning, None)
            );
            assert!(
                first
                    .message()
                    .starts_with("this string source has no MolangVersion"),
                "{first}"
            );
            assert!(!first.message().contains("-32768"), "{first}");
            assert_eq!(
                compiled
                    .diagnostics()
                    .iter()
                    .filter(|d| d.code() == DiagCode::InvalidVersion)
                    .count(),
                1
            );
            // It compiles as `Invalid`: no query resolves.
            assert!(
                compiled
                    .diagnostics()
                    .iter()
                    .any(|d| d.language_message() == Some(LanguageMessage::QueryUnresolved))
            );
        }
        let mut applied = src.clone();
        applied.set_context_version(13);
        assert!(
            compile_source(&applied, &CompileOptions::server(MolangVersion::LATEST))
                .parses_cleanly()
        );
    }

    #[test]
    fn compile_source_of_an_object_form_equals_compile_with_the_raw_version() {
        let field = CompileOptions::client(MolangVersion::LATEST);
        let value = |compiled: &Compiled| compiled.expr().and_then(Expr::as_constant);
        for (text, version) in [
            ("'a' + 1", 2),
            ("'a' + 1", 3),
            ("1 ?", 4),
            ("1 ?", 5),
            ("(1 ? 0 : 1 ? 2 : 3) + (1 || 0 && 0)", 4),
            ("(1 ? 0 : 1 ? 2 : 3) + (1 || 0 && 0)", 6),
            ("q.is_baby", 13),
            ("q.is_baby", 14),
            ("q.is_baby", -1),
            ("q.is_baby", -7),
        ] {
            let source = MolangSource::object(text, version);
            let via_source = compile_source(&source, &field);
            let direct = compile(
                text,
                &CompileOptions {
                    raw_version: RawVersion(version),
                    ..field.clone()
                },
            );
            assert_eq!(
                via_source.failure(),
                direct.failure(),
                "{text:?} at {version}"
            );
            assert_eq!(
                format!("{:?}", via_source.diagnostics()),
                format!("{:?}", direct.diagnostics()),
                "{text:?} at {version}"
            );
            assert_eq!(value(&via_source), value(&direct), "{text:?} at {version}");
        }
    }

    #[test]
    fn the_source_version_decides_the_grouping_of_the_same_text() {
        let field = CompileOptions::server(MolangVersion::LATEST);
        let text = "(1 ? 0 : 1 ? 2 : 3) + (1 || 0 && 0)";
        let value = |version: i16| {
            compile_source(&MolangSource::object(text, version), &field)
                .expr()
                .cloned()
                .and_then(|e| e.as_constant())
        };
        assert_eq!(
            (value(4), value(5), value(6), value(13)),
            (Some(3.0), Some(0.0), Some(1.0), Some(1.0))
        );
        // A string-form source takes the version of its load context in the same way.
        let string = |version: i16| {
            compile_source(&MolangSource::string(text, version), &field)
                .expr()
                .cloned()
                .and_then(|e| e.as_constant())
        };
        assert_eq!(
            (string(4), string(5), string(6)),
            (Some(3.0), Some(0.0), Some(1.0))
        );
    }
}

mod deviations {
    //! Deviations without a switch behave alike under both settings; a switched-off one behaves as
    //! under `Deviations::NONE`.

    use crate::common::compile_support::{messages, server_at};
    use molangx::compile::{
        CompileFailure, CompileOptions, Compiled, Deviations, Expr, MAX_DIAGNOSTICS,
        MAX_SOURCE_LEN, compile,
    };
    use molangx::diag::{DiagCode, Severity};
    use molangx::version::RawVersion;

    fn under(source: &str, raw_version: i16, deviations: Deviations) -> Compiled {
        compile(
            source,
            &CompileOptions {
                deviations,
                ..server_at(raw_version)
            },
        )
    }

    fn constant(compiled: &Compiled) -> Option<f32> {
        compiled.expr().and_then(Expr::as_constant)
    }

    #[test]
    fn escape_at_end_of_input_reports_the_missing_quote() {
        for source in ["'ab\\", "'a\\", "'\\", "'ab\\c"] {
            for deviations in [Deviations::ALL, Deviations::NONE] {
                let compiled = under(source, 13, deviations);
                assert_eq!(
                    compiled.failure(),
                    Some(CompileFailure::Rejected),
                    "{source:?}"
                );
                assert_eq!(
                    messages(&compiled)[0],
                    "Error: Molang string missing final ' character",
                    "{source:?}"
                );
            }
        }
    }

    /// `1.#QNAN` is the number `1.` followed by characters that are no token.
    #[test]
    fn qnan_literal_is_not_special() {
        let reference = messages(&under("1.#qnan", 13, Deviations::ALL));
        assert!(!reference.is_empty());
        for source in ["1.#QNAN", "1.#qnan", "1.#QnaN"] {
            for deviations in [Deviations::ALL, Deviations::NONE] {
                let compiled = under(source, 13, deviations);
                assert_eq!(
                    compiled.failure(),
                    Some(CompileFailure::Rejected),
                    "{source:?}"
                );
                assert_eq!(messages(&compiled).len(), reference.len(), "{source:?}");
            }
        }
    }

    /// Above 13 the parser gates are those of 13, below −1 those of version 0, and no query
    /// resolves at either.
    #[test]
    fn object_version_out_of_range_is_silent_without_the_warning_deviation() {
        // Groups as `(1 ? 0 : 1) ? 2 : 3` and `(1 || 0) && 0` below 5 / 6: 3; from 6 on: 0 + 1.
        let grouping = "(1 ? 0 : 1 ? 2 : 3) + (1 || 0 && 0)";
        let above = under(grouping, 14, Deviations::NONE);
        assert!(above.diagnostics().is_empty(), "{:?}", above.diagnostics());
        assert_eq!(constant(&above), Some(1.0));
        assert_eq!(
            constant(&above),
            constant(&under(grouping, 13, Deviations::NONE))
        );

        let below = under(grouping, -7, Deviations::NONE);
        assert!(below.diagnostics().is_empty(), "{:?}", below.diagnostics());
        assert_eq!(constant(&below), Some(3.0));
        assert_eq!(
            constant(&below),
            constant(&under(grouping, 0, Deviations::NONE))
        );

        let query = under("query.is_baby", -7, Deviations::NONE);
        assert_eq!(query.failure(), Some(CompileFailure::Rejected));
        assert!(
            query
                .diagnostics()
                .iter()
                .all(|d| d.code() != DiagCode::InvalidVersion)
        );
        assert!(!messages(&query).is_empty(), "the query does not resolve");

        // `Deviations::ALL` warns about the same inputs; the raw value is kept either way.
        for raw in [14, -7] {
            assert!(
                under("1", raw, Deviations::ALL)
                    .diagnostics()
                    .iter()
                    .any(|d| d.code() == DiagCode::InvalidVersion)
            );
            let options = CompileOptions {
                deviations: Deviations::NONE,
                ..server_at(raw)
            };
            assert_eq!(options.raw_version, RawVersion(raw));
        }
    }

    #[test]
    fn each_switch_changes_exactly_its_own_behaviour() {
        let run = |text: &str, raw: i16, deviations: Deviations| {
            compile(
                text,
                &CompileOptions {
                    deviations,
                    ..server_at(raw)
                },
            )
        };
        let codes = |compiled: &Compiled| {
            compiled
                .diagnostics()
                .iter()
                .map(|d| (d.code(), d.severity()))
                .collect::<Vec<_>>()
        };
        let constant = |compiled: &Compiled| compiled.expr().and_then(Expr::as_constant);

        // `true_false_prefix_advance`: on, the identifier's length; off, the full word's.
        let off = Deviations {
            true_false_prefix_advance: false,
            ..Deviations::ALL
        };
        assert_eq!(constant(&run("tr*5", 13, Deviations::ALL)), Some(5.0));
        assert_eq!(constant(&run("tr*5", 13, off)), Some(1.0));

        // `source_length_limit`: on, a source longer than `MAX_SOURCE_LEN` is rejected.
        let off = Deviations {
            source_length_limit: false,
            ..Deviations::ALL
        };
        let long = format!("{}1", " ".repeat(MAX_SOURCE_LEN));
        let ours = run(&long, 13, Deviations::ALL);
        assert_eq!(ours.failure(), Some(CompileFailure::Rejected));
        assert_eq!(codes(&ours), [(DiagCode::SourceTooLong, Severity::Error)]);
        let lenient = run(&long, 13, off);
        assert_eq!((lenient.failure(), constant(&lenient)), (None, Some(1.0)));

        // `validate_nested`: a nested finding is a Warning on, an Error off.
        let off = Deviations {
            validate_nested: false,
            ..Deviations::ALL
        };
        assert_eq!(
            codes(&run("break;", 13, Deviations::ALL)),
            [(DiagCode::StatementForm, Severity::Warning)]
        );
        assert_eq!(
            codes(&run("break;", 13, off)),
            [(DiagCode::StatementForm, Severity::Error)]
        );

        // `query_arity_lint`: on, a call outside the declared argument counts warns.
        let off = Deviations {
            query_arity_lint: false,
            ..Deviations::ALL
        };
        assert_eq!(
            codes(&run("q.log(7, 8)", 13, Deviations::ALL)),
            [(DiagCode::QueryArity, Severity::Warning)]
        );
        assert_eq!(codes(&run("q.log(7, 8)", 13, off)), []);

        // `query_client_only`: on, a client-only query compiled for the server gets an Info.
        let off = Deviations {
            query_client_only: false,
            ..Deviations::ALL
        };
        assert_eq!(
            codes(&run("q.is_first_person;", 13, Deviations::ALL)),
            [(DiagCode::QueryClientOnly, Severity::Info)]
        );
        assert_eq!(codes(&run("q.is_first_person;", 13, off)), []);

        // `object_version_warning`: on, a raw version outside -1..=13 warns.
        let off = Deviations {
            object_version_warning: false,
            ..Deviations::ALL
        };
        assert_eq!(
            codes(&run("1", 14, Deviations::ALL)),
            [(DiagCode::InvalidVersion, Severity::Warning)]
        );
        assert_eq!(codes(&run("1", 14, off)), []);

        // `diagnostic_limit`: on, the first `MAX_DIAGNOSTICS` and one note are kept; off, all of
        // them.
        let off = Deviations {
            diagnostic_limit: false,
            ..Deviations::ALL
        };
        let many = "1e;".repeat(MAX_DIAGNOSTICS + 5);
        assert_eq!(
            run(&many, 13, Deviations::ALL).diagnostics().len(),
            MAX_DIAGNOSTICS + 1
        );
        assert_eq!(run(&many, 13, off).diagnostics().len(), MAX_DIAGNOSTICS + 5);
    }
}

mod link {
    //! The link stage: the results it decides and the two messages it logs, in order.

    use crate::common::compile_support::server_at;
    use molangx::compile::{CompileFailure, Compiled, Expr, ProgramFlags, compile};
    use molangx::diag::LanguageMessage;

    fn at(source: &str, version: i16) -> Compiled {
        compile(source, &server_at(version))
    }

    fn language_messages(compiled: &Compiled) -> Vec<(LanguageMessage, String)> {
        compiled
            .diagnostics()
            .iter()
            .filter_map(|d| d.language_message().map(|v| (v, d.message().to_string())))
            .collect()
    }

    #[test]
    fn compile_failed_prints_the_source_up_to_the_first_nul() {
        let compiled = at("return c.x = 1;\0 trailing bytes", 13);
        assert_eq!(compiled.failure(), Some(CompileFailure::Rejected));
        let failed: Vec<_> = language_messages(&compiled)
            .into_iter()
            .filter(|(v, _)| *v == LanguageMessage::CompileFailed)
            .collect();
        assert_eq!(
            failed,
            vec![(
                LanguageMessage::CompileFailed,
                "expression 'return c.x = 1;' compile failed".to_owned()
            )]
        );
    }

    #[test]
    fn nested_context_assignment_fails_to_link() {
        for source in ["return c.x = 1;", "(c.x = 1);", "1 ? (c.x = 1) : 0;"] {
            let compiled = at(source, 13);
            assert!(compiled.parsed(), "{source}");
            assert_eq!(
                compiled.failure(),
                Some(CompileFailure::Rejected),
                "{source}"
            );
            let last = language_messages(&compiled).pop().expect("a message");
            assert_eq!(
                last,
                (
                    LanguageMessage::CompileFailed,
                    format!("expression '{source}' compile failed")
                ),
                "{source}"
            );
            assert_eq!(
                compiled.expr_or_zero().and_then(Expr::as_constant),
                Some(0.0)
            );
        }
    }

    #[test]
    fn pointer_write_logs_e49_then_e48() {
        let compiled = at("v.pigpig->v.x = 1; return 1;", 13);
        assert_eq!(compiled.failure(), Some(CompileFailure::Rejected));
        let ids: Vec<LanguageMessage> = language_messages(&compiled)
            .into_iter()
            .map(|(v, _)| v)
            .collect();
        let tail = &ids[ids.len() - 2..];
        assert_eq!(
            tail,
            [
                LanguageMessage::WriteToOtherMob,
                LanguageMessage::CompileFailed
            ]
        );
    }

    #[test]
    fn arrays_and_resources_need_resolution() {
        let cases = [
            ("array.skins[0]", Some(CompileFailure::UsesArrays)),
            ("array.foo", Some(CompileFailure::UsesArrays)),
            ("geometry.default", Some(CompileFailure::UsesResources)),
            ("material.default", Some(CompileFailure::UsesResources)),
            ("texture.default", Some(CompileFailure::UsesResources)),
        ];
        for (source, result) in cases {
            let compiled = at(source, 13);
            assert_eq!(compiled.failure(), result, "{source}");
            assert!(compiled.expr().is_none(), "{source}");
            assert!(compiled.parsed(), "{source}");
            // These results carry no diagnostic of their own.
            assert!(
                compiled.diagnostics().is_empty(),
                "{source}: {:?}",
                compiled.diagnostics()
            );
        }
    }

    /// A division builds its denominator first.
    #[test]
    fn link_result_follows_the_build_order() {
        // Version 2 lets resources take part in arithmetic.
        assert_eq!(
            at("geometry.a / array.b[0]", 2).failure(),
            Some(CompileFailure::UsesArrays)
        );
        assert_eq!(
            at("array.b[0] / geometry.a", 2).failure(),
            Some(CompileFailure::UsesResources)
        );
        assert_eq!(
            at("array.b[0] * geometry.a", 2).failure(),
            Some(CompileFailure::UsesArrays)
        );
        assert_eq!(
            at("geometry.a * array.b[0]", 2).failure(),
            Some(CompileFailure::UsesResources)
        );
    }

    #[test]
    fn program_flags() {
        let flags = |source: &str| {
            at(source, 13)
                .expr()
                .cloned()
                .expect("an expression")
                .flags()
        };
        assert!(flags("1 + 2").contains(ProgramFlags::CONSTANT.union(ProgramFlags::FLOAT_ONLY)));
        assert!(
            flags("v.x * 2")
                .contains(ProgramFlags::FLOAT_ONLY.union(ProgramFlags::READS_ACTOR_VARS))
        );
        assert!(flags("v.x = 1;").contains(ProgramFlags::HAS_ASSIGNMENT));
        assert!(
            flags("math.random(1, 2)")
                .contains(ProgramFlags::USES_RANDOM.union(ProgramFlags::USES_RANDOM_OP))
        );
        let die = flags("math.die_roll(1, 1, 6)");
        assert!(
            die.contains(ProgramFlags::USES_RANDOM) && !die.contains(ProgramFlags::USES_RANDOM_OP)
        );
        assert!(flags("c.other->v.x").contains(ProgramFlags::USES_ARROW));
        assert!(!flags("c.other->v.x").contains(ProgramFlags::FLOAT_ONLY));
        assert!(flags("q.is_baby").contains(ProgramFlags::USES_QUERIES));
        assert!(!flags("'a' == v.x").contains(ProgramFlags::FLOAT_ONLY));
        assert!(
            flags("t.x = 1; return t.x;")
                .contains(ProgramFlags::USES_TEMPS.union(ProgramFlags::FLOAT_ONLY))
        );
        assert!(flags("c.x").contains(ProgramFlags::READS_CONTEXT));
        assert!(flags("loop(2, {v.x = 1;});").contains(ProgramFlags::HAS_LOOPS));
        assert!(flags("v.a.b").contains(ProgramFlags::USES_MEMBERS));
    }
}

mod options {
    //! The public surface of a compile: the fields of `CompileOptions`, the program's flags.

    use molangx::catalog::{QueryAdmission, QueryAllowList, QuerySetMask, Side};
    use molangx::compile::{CompileOptions, Deviations, ProgramFlags, compile};
    use molangx::ops::OpSet;
    use molangx::stdlib::query;
    use molangx::version::{ExperimentMask, MolangVersion, RawVersion};

    #[test]
    fn compile_options() {
        let new = CompileOptions::server(MolangVersion::LATEST);
        // One value behind two getters.
        assert_eq!(
            (new.version(), new.raw_version),
            (MolangVersion::LATEST, RawVersion(13))
        );
        assert_eq!(new.admission, QueryAdmission::Sets(QuerySetMask::DEFAULT));
        assert_eq!(new.allowed_ops, OpSet::all());
        assert_eq!(new.experiments, ExperimentMask::empty());
        assert!(!new.keep_source);
        assert_eq!(new.catalog, molangx::stdlib::queries(Side::Server).clone());
        assert_eq!(new.deviations, Deviations::ALL);
        assert_eq!(new.allowed_ops.len(), 111);
        assert_eq!(compile("v.x = math.random(0, 1);", &new).failure(), None);
        let allow_list = QueryAllowList::new(&new.catalog, [query::BLOCK_STATE]).expect("declared");
        let restricted = CompileOptions {
            admission: QueryAdmission::Only(allow_list.clone()),
            ..new
        };
        assert_eq!(restricted.admission, QueryAdmission::Only(allow_list));
    }

    #[test]
    fn program_flags() {
        let options = CompileOptions::server(MolangVersion::LATEST);
        let flags = |source: &str| {
            compile(source, &options)
                .expr()
                .cloned()
                .expect("expr")
                .flags()
        };
        assert!(flags("v.x = 1;").contains(ProgramFlags::HAS_ASSIGNMENT));
        assert!(flags("v.x * 2").contains(ProgramFlags::READS_ACTOR_VARS));
        assert!(flags("v.x * 2").contains(ProgramFlags::FLOAT_ONLY));
        assert!(flags("math.random(0, v.x)").contains(ProgramFlags::USES_RANDOM));
        assert!(flags("v.a->v.b").contains(ProgramFlags::USES_ARROW));
        assert!(!flags("v.a->v.b").contains(ProgramFlags::FLOAT_ONLY));
        assert!(!flags("v.x * 2").contains(ProgramFlags::HAS_ASSIGNMENT));
        let expr = compile(
            "V.X",
            &CompileOptions {
                keep_source: true,
                ..options
            },
        )
        .expr()
        .cloned()
        .expect("expr");
        assert_eq!(
            (expr.version(), expr.source()),
            (MolangVersion::LATEST, Some("V.X"))
        );
    }
}

mod fields {
    //! Each field of `CompileOptions`, checked through the result of a compile.

    use crate::common::compile_support::messages;
    use crate::common::per_arch;
    use molangx::catalog::{QueryAdmission, QueryAllowList, QueryDecl, QuerySetMask, Side};
    use molangx::compile::{CompileFailure, CompileOptions, Deviations, compile};
    use molangx::ops::{ExpressionOp, OpSet};
    use molangx::stdlib::query;
    use molangx::version::{Experiment, ExperimentMask, MolangVersion, RawVersion};

    /// Version 4 reports a lone `?` as a binary operator without a right side, version 5 as a
    /// conditional without sub-expressions.
    #[test]
    fn a_version_sets_the_gates_and_the_raw_version() {
        let base = CompileOptions::server(MolangVersion::LATEST);
        let v4 = CompileOptions {
            raw_version: MolangVersion::V4.into(),
            ..base.clone()
        };
        let v5 = CompileOptions {
            raw_version: MolangVersion::V5.into(),
            ..base.clone()
        };
        assert_eq!(
            (v4.version(), v4.raw_version),
            (MolangVersion::V4, RawVersion(4))
        );
        assert_eq!(
            messages(&compile("1 ?", &v4)),
            ["Error: binary Conditional '?' operator at end of expression"]
        );
        assert_eq!(
            messages(&compile("1 ?", &v5)),
            ["Error: could not find sub-expressions for Conditional '?' operator"]
        );
        assert_eq!(
            messages(&compile("1 ?", &base)),
            messages(&compile("1 ?", &v5))
        );
    }

    #[test]
    fn a_raw_version_gates_with_the_effective_version_and_resolves_with_the_raw_one() {
        let base = CompileOptions::server(MolangVersion::LATEST);
        let above = CompileOptions {
            raw_version: RawVersion(14),
            ..base.clone()
        };
        assert_eq!(
            (above.version(), above.raw_version),
            (MolangVersion::LATEST, RawVersion(14))
        );
        assert_eq!(
            compile("'a' + 1", &above).failure(),
            Some(CompileFailure::Rejected),
            "the gates of 13"
        );
        assert_eq!(
            messages(&compile("q.is_baby", &above)).len(),
            2,
            "no query resolves at a raw 14"
        );
        let below = CompileOptions {
            raw_version: RawVersion(-7),
            ..base.clone()
        };
        assert_eq!(
            (below.version(), below.raw_version),
            (MolangVersion::Invalid, RawVersion(-7))
        );
        assert_eq!(
            compile("'a' + 1", &below).failure(),
            None,
            "the gates of version 0"
        );
        assert_eq!(compile("q.is_baby", &base).failure(), None);
    }

    #[test]
    fn query_sets_decide_what_resolves() {
        let base = CompileOptions::server(MolangVersion::LATEST);
        let in_sets = |sets| CompileOptions {
            admission: QueryAdmission::Sets(sets),
            ..base.clone()
        };
        assert_eq!(
            compile("query.noise(1, 2)", &base).failure(),
            Some(CompileFailure::Rejected)
        );
        assert_eq!(
            compile("query.noise(1, 2)", &in_sets(QuerySetMask::WORLD_GEN)).failure(),
            None
        );
        assert_eq!(
            compile("query.is_baby", &in_sets(QuerySetMask::WORLD_GEN)).failure(),
            Some(CompileFailure::Rejected)
        );
        assert_eq!(
            compile("query.is_baby", &in_sets(QuerySetMask::BUILTIN)).failure(),
            None
        );
    }

    #[test]
    fn allowed_ops_reject_the_operations_outside_the_mask() {
        let base = CompileOptions::server(MolangVersion::LATEST);
        let no_random = CompileOptions {
            allowed_ops: OpSet::all().without(ExpressionOp::Random),
            ..base.clone()
        };
        assert_eq!(compile("math.random(0, 1)", &base).failure(), None);
        let compiled = compile("math.random(0, 1) + math.random(2, 3)", &no_random);
        assert_eq!(compiled.failure(), Some(CompileFailure::Rejected));
        assert_eq!(
            messages(&compiled),
            ["Expression uses operation Random 'math.random' which is not allowed in this context"]
        );
        assert_eq!(
            compile("math.random_integer(0, 1)", &no_random).failure(),
            None,
            "a different operation"
        );
    }

    #[test]
    fn an_allow_list_admits_only_the_listed_queries() {
        let allowed = QueryAllowList::new(molangx::stdlib::queries(Side::Server), [query::IS_BABY])
            .expect("declared");
        let listed = CompileOptions {
            admission: QueryAdmission::Only(allowed),
            ..CompileOptions::server(MolangVersion::LATEST)
        };
        assert_eq!(compile("q.is_baby", &listed).failure(), None);
        assert_eq!(
            compile("q.variant", &listed).failure(),
            Some(CompileFailure::Rejected)
        );
        assert_eq!(
            compile("q.variant", &CompileOptions::server(MolangVersion::LATEST)).failure(),
            None
        );
        assert_eq!(
            compile("1 + 2", &listed).failure(),
            None,
            "a text without queries does not care"
        );
    }

    /// A list built from one catalogue restricts a compile against another to the names it lists.
    #[test]
    fn an_allow_list_and_the_catalogue_are_independent() {
        let server = molangx::stdlib::queries(Side::Server);
        let client = molangx::stdlib::queries(Side::Client);
        let list = QueryAllowList::new(server, [query::BLOCK_STATE]).expect("declared");
        let base = CompileOptions::server(MolangVersion::LATEST);
        let o = CompileOptions {
            catalog: client.clone(),
            admission: QueryAdmission::Only(list),
            ..base.clone()
        };
        assert_eq!(o.catalog, client.clone());
        assert_eq!(
            compile("q.is_baby", &o).failure(),
            Some(CompileFailure::Rejected)
        );
        assert_eq!(
            compile("q.block_state('facing') == 'west'", &o).failure(),
            None
        );
        // A host catalogue is kept, so its own queries resolve once listed.
        let own = client
            .extended([
                QueryDecl::new("query.mine", molangx::catalog::QueryShape::DEFAULT)
                    .expect("a name"),
            ])
            .expect("new");
        let mine = QueryAllowList::new(&own, ["query.mine"]).expect("declared");
        let o = CompileOptions {
            catalog: own.clone(),
            admission: QueryAdmission::Only(mine.clone()),
            ..base.clone()
        };
        assert_eq!(o.catalog, own);
        assert_eq!(compile("q.mine", &o).failure(), None);
        assert_eq!(
            compile("q.is_baby", &o).failure(),
            Some(CompileFailure::Rejected)
        );
        // The same list against the standard catalogue, which does not declare the name, admits
        // nothing.
        let elsewhere = CompileOptions {
            admission: QueryAdmission::Only(mine),
            ..base
        };
        assert_eq!(
            compile("q.mine", &elsewhere).failure(),
            Some(CompileFailure::Rejected)
        );
        assert_eq!(
            compile("q.is_baby", &elsewhere).failure(),
            Some(CompileFailure::Rejected)
        );
    }

    #[test]
    fn the_side_effect_masks_remove_the_assignment_and_optionally_the_random_functions() {
        let base = CompileOptions::server(MolangVersion::LATEST);
        let no_assignment = CompileOptions {
            allowed_ops: base.allowed_ops.without_assignments(),
            ..base.clone()
        };
        assert_eq!(
            compile("v.x = 1;", &no_assignment).failure(),
            Some(CompileFailure::Rejected)
        );
        assert_eq!(compile("math.random(0, 1)", &no_assignment).failure(), None);
        let no_effects = CompileOptions {
            allowed_ops: base.allowed_ops.without_assignments_or_random(),
            ..base.clone()
        };
        assert_eq!(
            compile("math.random(0, 1)", &no_effects).failure(),
            Some(CompileFailure::Rejected)
        );
        assert_eq!(
            compile("math.die_roll(1, 0, 1)", &no_effects).failure(),
            None,
            "the dice stay allowed"
        );
        assert_eq!(compile("v.x = 1;", &base).failure(), None);
    }

    #[test]
    fn experiments_change_nothing_for_the_built_in_queries() {
        let base = CompileOptions::server(MolangVersion::LATEST);
        let enabled = CompileOptions {
            experiments: ExperimentMask::empty()
                .with(Experiment::new(5).expect("an experiment id")),
            ..base.clone()
        };
        for source in ["q.is_baby", "q.variant", "v.x + 1", "math.pi"] {
            let (a, b) = (compile(source, &base), compile(source, &enabled));
            assert_eq!(
                (a.failure(), format!("{:?}", a.diagnostics())),
                (b.failure(), format!("{:?}", b.diagnostics())),
                "{source}"
            );
        }
    }

    #[test]
    fn the_catalog_selects_the_catalogue() {
        let base = CompileOptions::server(MolangVersion::LATEST);
        assert_eq!(base.catalog.side(), Side::Server);
        assert_eq!(
            compile("q.is_on_screen", &base).failure(),
            Some(CompileFailure::Rejected)
        );
        let client = CompileOptions {
            catalog: molangx::stdlib::queries(Side::Client).clone(),
            ..base.clone()
        };
        assert_eq!(compile("q.is_on_screen", &client).failure(), None);
        assert_eq!(compile("q.is_baby", &base).failure(), None);
    }

    #[test]
    fn keep_source_keeps_the_text_as_written() {
        let base = CompileOptions::server(MolangVersion::LATEST);
        let kept = CompileOptions {
            keep_source: true,
            ..base.clone()
        };
        assert_eq!(
            compile("V.X * 2", &base)
                .expr()
                .cloned()
                .expect("expr")
                .source(),
            None
        );
        assert_eq!(
            compile("V.X * 2", &kept)
                .expr()
                .cloned()
                .expect("expr")
                .source(),
            Some("V.X * 2")
        );
        assert_eq!(
            compile(
                "V.X * 2",
                &CompileOptions {
                    keep_source: false,
                    ..kept
                }
            )
            .expr()
            .cloned()
            .expect("expr")
            .source(),
            None
        );
    }

    #[test]
    fn the_folding_arithmetic_is_the_architectures() {
        let folded = compile(
            "math.sign(math.sqrt(-1))",
            &CompileOptions::server(MolangVersion::LATEST),
        )
        .expr()
        .cloned()
        .and_then(|e| e.as_constant());
        assert_eq!(folded, Some(per_arch(1.0, -1.0)));
    }

    /// Switched off, `tr*5` also drops the `*5`.
    #[test]
    fn deviations_switch_the_behaviour() {
        let base = CompileOptions::server(MolangVersion::LATEST);
        assert_eq!(base.deviations, Deviations::ALL);
        let folded = |deviations| {
            compile(
                "tr*5",
                &CompileOptions {
                    deviations,
                    ..base.clone()
                },
            )
            .expr()
            .cloned()
            .and_then(|e| e.as_constant())
        };
        assert_eq!(folded(Deviations::ALL), Some(5.0));
        assert_eq!(folded(Deviations::NONE), Some(1.0));
    }
}

mod pitfalls {
    use crate::common::compile_support::{at, constant, messages, messages_at};
    #[cfg(feature = "vm")]
    use crate::common::{
        compile_support::{server_at, server_expr},
        per_arch,
    };
    #[cfg(feature = "vm")]
    use molangx::compile::compile;
    use molangx::compile::{CompileFailure, Expr};
    #[cfg(feature = "vm")]
    use molangx::numeric::{ARCH, Arch};
    #[cfg(feature = "vm")]
    use molangx::rng::FixedRng;
    #[cfg(feature = "vm")]
    use molangx::vm::{EvalLimits, NoHostEnv, Temps, Value, VariableMap, VariableName};

    fn constant_at(source: &str, version: i16) -> Option<f32> {
        let compiled = at(source, version);
        compiled.expr().and_then(Expr::as_constant)
    }

    #[cfg(feature = "vm")]
    fn eval(source: &str) -> f32 {
        server_expr(source).eval_f32(&mut NoHostEnv::new().cx())
    }

    #[cfg(feature = "vm")]
    fn eval_forced(source: &str, sample: f32) -> f32 {
        let expr = server_expr(source);
        let mut env = NoHostEnv::new();
        let mut rng = FixedRng::from_sample(sample).expect("a word's sample");
        let mut cx = env.cx();
        cx.rng = &mut rng;
        expr.eval_f32(&mut cx)
    }

    #[cfg(feature = "vm")]
    fn eval_limited(source: &str, limits: EvalLimits) -> (f32, Vec<String>) {
        let expr = server_expr(source);
        let mut env = NoHostEnv {
            limits,
            ..NoHostEnv::new()
        };
        let value = expr.eval_f32(&mut env.cx());
        (value, env.sink.take())
    }

    #[test]
    fn pitfall_f_suffix() {
        assert_eq!(constant("1.5f"), 1.5);
    }

    #[test]
    fn pitfall_number_forms() {
        assert_eq!(constant(".5"), 0.5);
        assert_eq!(constant("5."), 5.0);
        assert_eq!(constant("1e3"), 1000.0);
        assert_eq!(constant("2147483648"), -2_147_483_648.0);
        assert_eq!(constant("4294967296"), 0.0);
    }

    /// `?:` is right-associative from version 5; `&&` binds tighter than `||` from 6.
    #[test]
    fn pitfall_grouping_per_version() {
        let source = "(1 ? 0 : 1 ? 2 : 3) + (1 || 0 && 0)";
        assert_eq!(constant_at(source, 4), Some(3.0));
        assert_eq!(constant_at(source, 5), Some(0.0));
        assert_eq!(constant_at(source, 6), Some(1.0));
    }

    /// `7 * 3 / 9` is `7 · (3 / 9)`.
    #[test]
    fn pitfall_division_before_multiplication() {
        assert_eq!(constant("7 * 3 / 9"), 2.333_333_5);
    }

    /// Rejected at the root, logged and kept when nested, silently kept for `-v.x ?? 1`.
    #[test]
    fn pitfall_null_coalescing_on_non_variables() {
        let lhs = "Error: found left-hand-side of ?? expression that isn't a direct-variable reference - this is unsupported at this time.";
        assert_eq!(messages_at("(1 ?? 2) * 2", 13), [lhs]);
        assert_eq!(messages_at("v.a ?? v.b ?? 5", 13), [lhs]);
        let unary = at("-v.x ?? 1", 13);
        assert_eq!(unary.failure(), None);
        assert!(messages(&unary).is_empty());
    }

    /// `math.round(-2.5)` rounds away from zero.
    #[test]
    fn pitfall_round_half() {
        assert_eq!(constant("math.round(-2.5)"), -3.0);
        assert_eq!(constant("math.round(2.5)"), 3.0);
    }

    /// `asin` accepts inputs just above 1 (1.0005 → 90), not 1.001.
    #[test]
    fn pitfall_asin_tolerance() {
        assert_eq!(constant("math.asin(1.0004)"), 90.0);
        assert!((constant("math.asin(1.0005)") - 90.0).abs() <= 5e-4);
        assert!(constant("math.asin(1.001)").is_nan());
    }

    /// Trigonometry takes and returns degrees.
    #[test]
    fn pitfall_trig_units() {
        assert!((constant("math.sin(90) * 2") - 2.0).abs() <= 1e-6);
        assert_eq!(constant("math.atan2(1, 1)").to_bits(), 45.0_f32.to_bits());
    }

    /// Both endpoints are reachable, and inverted bounds are accepted.
    #[cfg(feature = "vm")]
    #[test]
    fn pitfall_random_integer_endpoints() {
        assert_eq!(eval_forced("math.random_integer(1.0, 3.0)", 0.0), 1.0);
        assert_eq!(eval_forced("math.random_integer(1.0, 3.0)", 1.0), 3.0);
        assert_eq!(eval_forced("math.random_integer(4.0, 3.0)", 0.0), 3.0);
        assert_eq!(eval_forced("math.random_integer(4.0, 3.0)", 1.0), 4.0);
        assert_eq!(
            eval_forced(
                "v.x = -1.0; v.y = -3.0; return math.random_integer(v.x, v.y);",
                0.5
            ),
            -2.0
        );
    }

    /// The step budget ends a huge count with 0; a negative count rolls nothing.
    #[cfg(feature = "vm")]
    #[test]
    fn pitfall_die_roll_terminates() {
        let (value, messages) =
            eval_limited("math.die_roll(1000000000, 1, 6)", EvalLimits::DEFAULT);
        assert_eq!(value, 0.0);
        assert_eq!(
            messages,
            ["molangx: evaluation stopped after its budget of 1048576 steps"]
        );
        assert_eq!(eval_forced("math.die_roll(-3, 0, 4)", 0.5), 0.0);
    }

    /// Without limits a loop runs to its count; the default guard stops it at 1,024.
    #[cfg(feature = "vm")]
    #[test]
    fn pitfall_loops_above_1024() {
        let source = "t.i = 0; loop(1025, {t.i = t.i + 1;}); return t.i;";
        assert_eq!(eval_limited(source, EvalLimits::NONE).0, 1025.0);
        assert_eq!(eval_limited(source, EvalLimits::DEFAULT).0, 1024.0);
    }

    #[test]
    fn pitfall_depth_255_256() {
        let nested = |depth: usize| format!("{}1{}", "(".repeat(depth), ")".repeat(depth));
        assert_eq!(at(&nested(255), 13).failure(), None);
        assert_eq!(
            messages(&at(&nested(256), 13)),
            [
                "Error: Expression could not be parsed due to stack depth overflow (too many sub-expressions)"
            ]
        );
    }

    #[test]
    fn pitfall_unknown_characters_and_trailing_tokens() {
        for version in [0, 4, 13] {
            assert_eq!(
                messages_at("v.a @ v.b", version),
                ["unrecognized token: @ v.b"],
                "version {version}"
            );
            assert_eq!(
                at("v.a @ v.b", version).failure(),
                Some(CompileFailure::Rejected),
                "version {version}"
            );
            assert_eq!(
                messages_at("9 10", version),
                [
                    "found multiple operations without a combining operation between them:\n\t9.000000\n\t10.000000"
                ],
                "version {version}"
            );
        }
    }

    #[test]
    fn pitfall_string_arithmetic() {
        assert_eq!(at("'a' + 3", 2).failure(), None);
        assert_eq!(at("'a' + 3", 3).failure(), Some(CompileFailure::Rejected));
    }

    /// A condition is true for every value but ±0, NaN included.
    #[cfg(feature = "vm")]
    #[test]
    fn pitfall_ternary_truthiness() {
        assert_eq!(eval("v.x = 0.5; return v.x ? 2 : 3;"), 2.0);
        assert_eq!(eval("v.x = -2; return v.x ? 2 : 3;"), 2.0);
        assert_eq!(eval("v.x = -0.0; return v.x ? 2 : 3;"), 3.0);
        assert_eq!(eval("v.n = math.sqrt(-1); return v.n ? 2 : 3;"), 2.0);
    }

    /// With a NaN operand under `X86_64`, `<`, `<=` and `>=` are false and `!=` is true; under
    /// `Arm64`, `<` and `<=` are true.
    #[cfg(feature = "vm")]
    #[test]
    fn pitfall_nan_rules() {
        assert_eq!(
            eval("v.n = math.sqrt(-1); return v.n <= 4;"),
            per_arch(0.0, 1.0)
        );
        assert_eq!(eval("v.n = math.sqrt(-1); return v.n >= 4;"), 0.0);
        assert_eq!(
            eval("v.n = math.sqrt(-1); return v.n < 4;"),
            per_arch(0.0, 1.0)
        );
        assert_eq!(eval("v.n = math.sqrt(-1); return v.n != 4;"), 1.0);
    }

    /// Complete booleans under `true_false_prefix_advance`.
    #[test]
    fn pitfall_true_false_prefixes() {
        assert_eq!(constant("t"), 1.0);
        assert_eq!(constant("fa"), 0.0);
        assert_eq!(constant("tr*5"), 5.0);
    }

    /// A divisor smaller than `f32::EPSILON` in magnitude gives 0, folded or at run time.
    #[cfg(feature = "vm")]
    #[test]
    fn pitfall_division_by_tiny_divisors() {
        assert_eq!(constant("5 / 0.0000001"), 0.0);
        assert_eq!(eval("v.g = 0.0000001; return 5 / v.g;"), 0.0);
        assert_eq!(eval("v.g = 0; return 5 / v.g;"), 0.0);
    }

    /// An unset read ends the expression with 0 before the later statements run.
    #[cfg(feature = "vm")]
    #[test]
    fn pitfall_unset_read() {
        let compiled = at("v.q = 1; v.r = v.missing; v.s = 2; return 3;", 13);
        let mut env = NoHostEnv::new();
        assert_eq!(
            compiled
                .expr()
                .cloned()
                .expect("compiles")
                .eval(&mut env.cx()),
            Value::ZERO
        );
        assert_eq!(
            env.variables.get(VariableName::new("q")),
            Some(&Value::Float(1.0))
        );
        assert_eq!(env.variables.get(VariableName::new("s")), None);
    }

    /// NaN in `max` and the fused post-op differ by architecture.
    #[cfg(feature = "vm")]
    #[test]
    fn pitfall_architecture_specific_nan_and_rounding() {
        let max = compile("math.max(4, math.sqrt(-1))", &server_at(13))
            .expr()
            .cloned()
            .and_then(|e| e.as_constant())
            .expect("constant");
        assert_eq!(max.is_nan(), per_arch(true, false));
        if ARCH == Arch::Arm64 {
            assert_eq!(max, 4.0);
        }
        let post = "v.x = 1/3; return v.x * 3 - 1;";
        assert_eq!(
            eval(post).to_bits(),
            per_arch(0.0_f32, 2.980_232_2e-8).to_bits()
        );
    }

    /// `math.mod` by a literal 0 is NaN, by a run-time 0 it is 0.
    #[cfg(feature = "vm")]
    #[test]
    fn pitfall_mod_by_zero() {
        assert!(constant("math.mod(1, 0)").is_nan());
        assert!(eval("v.x = 1; return math.mod(v.x, 0);").is_nan());
        assert_eq!(eval("v.x = 1; v.y = 0; return math.mod(v.x, v.y);"), 0.0);
    }

    #[test]
    fn pitfall_min_angle_180() {
        assert_eq!(constant("math.min_angle(180)"), -180.0);
        assert_eq!(constant("math.min_angle(-180)"), -180.0);
    }

    /// The statement after a `for_each` over a number does not take effect.
    #[cfg(feature = "vm")]
    #[test]
    fn temp_write_after_for_each_over_a_number() {
        let compiled = at(
            "t.st = 1; v.x = 0; v.a = 0; for_each(v.x, v.a, 1); t.st = 2; return t.st;",
            13,
        );
        let mut env = NoHostEnv {
            temps: Temps::Kept(VariableMap::new()),
            ..NoHostEnv::new()
        };
        assert_eq!(
            compiled
                .expr()
                .cloned()
                .expect("compiles")
                .eval_f32(&mut env.cx()),
            1.0
        );
    }
}

mod names_and_targets {
    //! Namespaces and names: aliases, assignable and read-only variables, `this`, `->`, `??` left
    //! sides, arrays, resources and query calls.

    use super::{assert_parsed, assert_rejected};
    use crate::common::compile_support::{at, message_ids, messages_at};
    use molangx::compile::Expr;

    use molangx::compile::CompileFailure;

    const CONTEXT_ASSIGNMENT: &str = "Error: assignment to non-variable not allowed. Expression is trying to assign to a: Context Variable 'context.' or 'c.'";

    #[test]
    fn namespace_aliases_parse_alike() {
        for (long, short) in [
            ("variable.x", "v.x"),
            ("temp.x", "t.x"),
            ("context.x", "c.x"),
        ] {
            let (long_form, short_form) = (at(long, 13), at(short, 13));
            assert_eq!(long_form.failure(), short_form.failure(), "{long}");
            assert_eq!(message_ids(&long_form), message_ids(&short_form), "{long}");
            assert_eq!(messages_at(long, 13), messages_at(short, 13), "{long}");
        }
        assert_parsed("V.X", 13, &[]);
        assert_parsed("Variable.X", 13, &[]);
    }

    #[test]
    fn member_chains_are_assignable() {
        assert_parsed("v.x.y.z = 1;", 13, &[]);
        assert_parsed("t.x = 1; return t.x;", 13, &[]);
    }

    #[test]
    fn temp_member_assignment_is_kept() {
        assert_parsed("t.x.y = 1;", 13, &["E39"]);
        assert_parsed("t.a.b = 1; return 1;", 13, &["E39"]);
        assert_eq!(at("t.a.b = 1; return 1;", 13).failure(), None);
    }

    #[test]
    fn context_assignment_statement_is_rejected() {
        for version in [13, 2] {
            for source in ["c.x = 1;", "context.x = 1;"] {
                assert_rejected(source, version, &["E28"]);
                assert_eq!(
                    messages_at(source, version),
                    [CONTEXT_ASSIGNMENT],
                    "{source} v{version}"
                );
            }
        }
    }

    /// Without the `;` only #7 is logged.
    #[test]
    fn nested_context_assignment() {
        for source in [
            "return c.x = 1;",
            "(c.x = 1);",
            "1 ? (c.x = 1) : 0;",
            "v.y = (c.x = 1);",
        ] {
            let compiled = at(source, 13);
            assert!(compiled.parsed(), "{source}");
            assert_eq!(
                compiled.failure(),
                Some(CompileFailure::Rejected),
                "{source}"
            );
            assert_eq!(message_ids(&compiled), ["E40", "E47", "E48"], "{source}");
            assert_eq!(
                compiled.expr_or_zero().and_then(Expr::as_constant),
                Some(0.0),
                "{source}"
            );
        }
        for source in ["return c.x = 1", "(c.x = 1)", "1 ? (c.x = 1) : 0"] {
            assert_rejected(source, 13, &["E07"]);
        }
    }

    #[test]
    fn this_is_read_only() {
        assert_rejected("this = 1;", 13, &["E28"]);
    }

    #[test]
    fn assignment_targets() {
        for source in [
            "v.x + 1 = 2;",
            "-v.x = 1;",
            "math.pi = 2;",
            "array.a[0] = 2;",
            "v.x = v.y = 1;",
        ] {
            assert_rejected(source, 13, &["E28"]);
        }
        assert_eq!(
            message_ids(&at("return v.b = v.a = 1;", 13)),
            ["E40", "E47", "E48"]
        );
    }

    #[test]
    fn pointer_right_side() {
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

    /// #46 joins #41 when `B` of `A->B->C` is no entity variable.
    #[test]
    fn nested_pointers() {
        assert_rejected("c.a->c.b->v.x", 13, &["E46", "E41"]);
        assert_rejected("v.x->v.y->v.z", 13, &["E41"]);
    }

    #[test]
    fn parenthesised_pointer_right_side() {
        assert_parsed("c.other->(v.x)", 13, &[]);
    }

    /// Other left sides are rejected at the root and kept when nested.
    #[test]
    fn null_coalescing_left_side() {
        for source in ["v.a ?? 1", "t.a ?? 1", "c.a ?? 1"] {
            assert_parsed(source, 13, &[]);
        }
        for source in [
            "1 ?? 2",
            "v.a.b ?? 1",
            "v.x->v.y ?? 1",
            "v.x->v.x ?? 1",
            "math.pi ?? 1",
            "array.a[0] ?? 1",
            "q.is_baby ?? 1",
            "v.a ?? v.b ?? 1",
        ] {
            assert_rejected(source, 13, &["E44"]);
        }
        assert_parsed("0 ?? 1;", 13, &["E44"]);
    }

    #[test]
    fn null_coalescing_ignores_a_folded_post_op() {
        for source in [
            "-v.x ?? 1",
            "v.x * 2 ?? 1",
            "v.x - 3 ?? 1",
            "(v.x + 0) ?? 1",
        ] {
            assert_parsed(source, 13, &[]);
        }
    }

    #[test]
    fn array_index_forms() {
        assert_parsed("array.foo[v.x]", 13, &[]);
        assert_parsed("array.foo[0]", 13, &[]);
        assert_rejected("array.a[1][2]", 13, &["E24"]);
        assert_rejected("array.a[1,2]", 13, &["E26"]);
        assert_rejected("array.a[]", 13, &["E08"]);
    }

    /// Resource references stay unresolved, and from version 3 are no arithmetic operands.
    #[test]
    fn resources_in_arithmetic() {
        assert_rejected("geometry.foo + 1", 13, &["E36"]);
        let v2 = at("geometry.foo + 1", 2);
        assert!(v2.parsed());
        assert_eq!(v2.failure(), Some(CompileFailure::UsesResources));
    }

    #[test]
    fn query_with_empty_parentheses() {
        assert_rejected("q.is_baby()", 13, &["E13"]);
        assert_parsed("q.is_baby", 13, &[]);
    }
}

mod statement_rules {
    //! Statements: simple and complex expressions, `;` lists, braces, `return`, `loop`, `for_each`,
    //! `break` / `continue`.

    use super::{assert_parsed, assert_rejected};
    use crate::common::compile_support::{at, constant, message_ids, messages_at};

    fn ids_at(source: &str, version: i16) -> Vec<&'static str> {
        message_ids(&at(source, version))
    }

    #[test]
    fn simple_expressions() {
        assert_parsed("v.x+1", 13, &[]);
        assert_eq!(constant("1"), 1.0);
    }

    /// The root list rejects, a nested list keeps it; a `break` operand logs #38 too.
    #[test]
    fn unreachable_statements() {
        assert_rejected("return 1; return 2;", 13, &["E38"]);
        assert_rejected("return 0; return 0;", 13, &["E38"]);
        assert_parsed("loop(3, {break; v.x = 1;});", 13, &["E38"]);
        assert_parsed("v.x ? break : 1;", 13, &["E38"]);
        assert_parsed("loop(2, {v.x ? break : continue;});", 13, &["E38"]);
    }

    #[test]
    fn brace_sections_need_a_semicolon() {
        assert_rejected("{1}", 13, &["E34"]);
        assert_rejected("{}", 13, &["E24"]);
        assert_rejected("{};", 13, &["E24"]);
        assert_rejected("loop(2, {});", 13, &["E24"]);
    }

    #[test]
    fn last_statement_of_a_block() {
        assert_parsed("{v.a=1; v.b=2};", 13, &[]);
    }

    #[test]
    fn block_statement_needs_its_own_semicolon() {
        assert_rejected("{v.x = 1;}", 13, &["E07"]);
    }

    #[test]
    fn parenthesised_statement_list() {
        assert_parsed("(v.x = 1;);", 13, &[]);
    }

    #[test]
    fn blocks_as_branches() {
        assert_parsed("v.x ? {v.y = 1;} : {v.y = 2;};", 13, &[]);
        assert_parsed(
            "variable.direction ?? { variable.direction.x = 0.0; };",
            13,
            &[],
        );
    }

    /// Any count expression parses, even a string.
    #[test]
    fn loop_parameters() {
        assert_rejected("loop(3, v.x = v.x + 1);", 13, &["E30"]);
        assert_rejected("loop(3);", 13, &["E30"]);
        assert_rejected("loop(3, {v.x = 1;}, 2);", 13, &["E30"]);
        assert_parsed("loop (3, {v.x = 1;});", 13, &[]);
        assert_parsed("loop('a', {v.y = 1;});", 13, &[]);
        assert_parsed("loop(v.n, {v.c = v.c + 1;});", 13, &[]);
    }

    /// A third argument that is not a block is accepted, unless it is an assignment.
    #[test]
    fn for_each_parameters() {
        assert_rejected("for_each(v.x, v.arr);", 13, &["E27"]);
        assert_rejected("for_each(v.x, v.arr, v.y = 1);", 13, &["E27"]);
        assert_parsed("for_each(v.x, v.a, 1);", 13, &[]);
    }

    #[test]
    fn for_each_variable() {
        assert_parsed("for_each(t.x, v.arr, {t.y = t.x;});", 13, &[]);
        for source in [
            "for_each(c.e, v.arr, {v.x = 1;});",
            "for_each(1, v.arr, {v.x = 1;});",
            "for_each(v.a.b, v.list, {v.n = 1;});",
            "for_each(math.pi, v.arr, {v.n = 1;});",
            "for_each(this, v.arr, {v.n = 1;});",
        ] {
            assert_rejected(source, 13, &["E33"]);
        }
    }

    #[test]
    fn for_each_array_is_not_checked() {
        assert_parsed("for_each(v.x, 1, {v.y = 1;});", 13, &[]);
    }

    /// Only the root `break` is rejected.
    #[test]
    fn break_outside_a_loop() {
        assert_rejected("break", 13, &["E45"]);
        assert_parsed("break;", 13, &["E45"]);
        assert_parsed("v.x = 1; break;", 13, &["E45"]);
        assert_eq!(
            messages_at("break;", 13),
            ["Error: break encountered outside of loop"]
        );
        assert_parsed("loop(3, {break;});", 13, &[]);
    }

    #[test]
    fn continue_outside_a_loop() {
        assert_parsed("continue;", 13, &[]);
        assert_parsed("v.x = 1; continue;", 13, &[]);
    }

    #[test]
    fn conditional_break_and_continue() {
        assert_parsed("loop(3, {v.c = 1; (v.c == 2) ? break;});", 13, &[]);
        assert_parsed("loop(3, {(v.c == 1) ? continue; v.c = 1;});", 13, &[]);
    }

    #[test]
    fn statements_are_no_operands_from_version_3() {
        for version in [3, 13] {
            for source in [
                "loop(3, {v.i = v.i + 1;}) + 1;",
                "for_each(v.s, v.b, {v.c = 1;}) + 1;",
                "loop(3, {v.c = 1; (v.c == 1) ? break + 1;});",
                "loop(3, {(v.c == 1) ? continue + 1;});",
                "(v.foo = 1) + 2;",
                "v.x < (v.y = 1);",
                "!(v.y = 1);",
                "-loop(3, {v.i = 1;});",
                "math.abs(loop(3, {v.i = 1;}));",
            ] {
                assert_rejected(source, version, &["E36"]);
            }
        }
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
    }

    /// Under `return` a chained assignment parses, then fails to link.
    #[test]
    fn chained_assignment() {
        assert_rejected("v.x = v.y = 1;", 13, &["E28"]);
        let compiled = at("return v.b = v.a = 1;", 13);
        assert!(compiled.parsed());
        assert_eq!(message_ids(&compiled), ["E40", "E47", "E48"]);
    }

    #[test]
    fn break_with_an_operand() {
        assert_rejected("break 11", 13, &["E08"]);
    }

    #[test]
    fn nested_loops() {
        assert_parsed("loop(2, {loop(2, {v.c = v.c + 1;});});", 13, &[]);
    }

    #[test]
    fn return_in_a_loop_body() {
        assert_parsed("loop(3, {return 1;});", 13, &[]);
    }

    #[test]
    fn value_statements() {
        assert_eq!(ids_at("v.x = 1; v.x;", 13), Vec::<String>::new());
    }
}

mod shapes_by_value {
    //! Grouping, folding and lexing checked through results, messages and evaluated values.

    use crate::common::compile_support::{at, constant, messages, messages_at};
    #[cfg(feature = "vm")]
    use crate::common::compile_support::{client_expr, client_expr_at};
    use molangx::compile::CompileFailure;
    #[cfg(feature = "vm")]
    use molangx::hash::HashedStr;
    #[cfg(feature = "vm")]
    use molangx::rng::Xorshift128;
    #[cfg(feature = "vm")]
    use molangx::vm::{NoHost, NoHostEnv, StructValue, Value, VariableName};

    #[cfg(feature = "vm")]
    fn run(source: &str, version: i16, vars: &[(&str, f32)]) -> f32 {
        let expr = client_expr_at(source, version);
        let mut env = NoHostEnv::new();
        for &(name, value) in vars {
            env.variables
                .set(VariableName::new(name), Value::Float(value));
        }
        expr.eval_f32(&mut env.cx())
    }

    #[cfg(feature = "vm")]
    fn value_of(source: &str) -> Value<NoHost> {
        client_expr(source).eval(&mut NoHostEnv::new().cx())
    }

    #[test]
    fn binary_levels_are_left_associative() {
        assert_eq!(constant("8/4/2"), 1.0, "(8/4)/2, not 8/(4/2)");
        assert_eq!(constant("5<4<3"), 1.0, "(5<4)<3, not 5<(4<3)");
        assert_eq!(constant("0>=0>0<=0"), 0.0, "((0>=0)>0)<=0");
        assert_eq!(constant("2==2!=0"), 1.0, "(2==2)!=0");
        #[cfg(feature = "vm")]
        {
            assert_eq!(
                run("v.a/v.b/v.c", 13, &[("a", 8.0), ("b", 4.0), ("c", 2.0)]),
                1.0
            );
            assert_eq!(
                run("v.a<v.b<v.c", 13, &[("a", 5.0), ("b", 4.0), ("c", 3.0)]),
                1.0
            );
            assert_eq!(
                run(
                    "v.a>=v.b>v.c<=v.d",
                    13,
                    &[("a", 0.0), ("b", 0.0), ("c", 0.0), ("d", 0.0)]
                ),
                0.0
            );
            assert_eq!(
                run("v.a==v.b!=v.c", 13, &[("a", 2.0), ("b", 2.0), ("c", 0.0)]),
                1.0
            );
        }
    }

    #[test]
    fn division_binds_tighter_than_multiplication() {
        assert_eq!(constant("7 * 3 / 9"), 7.0_f32 * (3.0_f32 / 9.0));
        #[cfg(feature = "vm")]
        for version in [-1, 0, 5, 6, 13] {
            // These values round differently as `a * (b / c)` and as `(a * b) / c`.
            let vars = [("a", 3.0), ("b", 1.0), ("c", 7.0)];
            assert_eq!(
                run("v.a*v.b/v.c", version, &vars),
                3.0_f32 * (1.0_f32 / 7.0),
                "version {version}"
            );
            assert_ne!(
                run("v.a*v.b/v.c", version, &vars),
                (3.0_f32 * 1.0) / 7.0,
                "version {version}"
            );
            assert_eq!(
                run("v.a/v.b*v.c", version, &vars),
                (3.0_f32 / 1.0) * 7.0,
                "version {version}"
            );
            assert_eq!(
                run(
                    "v.a * v.b / v.c * v.d",
                    version,
                    &[("a", 3.0), ("b", 1.0), ("c", 7.0), ("d", 5.0)]
                ),
                (3.0_f32 * (1.0_f32 / 7.0)) * 5.0,
                "version {version}"
            );
        }
    }

    /// Binary minus is `Add` of a negation; `+` is one n-ary node.
    #[test]
    fn addition_and_subtraction() {
        assert_eq!(constant("1--1"), 2.0);
        #[cfg(feature = "vm")]
        {
            let vars = [("a", 10.0), ("b", 3.0), ("c", 2.0)];
            assert_eq!(run("v.a+v.b*v.c", 13, &vars), 16.0);
            assert_eq!(run("v.a-v.b-v.c", 13, &vars), 5.0, "(a-b)-c, not a-(b-c)");
            assert_eq!(run("v.a - v.b + v.c", 13, &vars), 9.0);
            assert_eq!(run("v.a/-v.b", 13, &[("a", 6.0), ("b", 3.0)]), -2.0);
            assert_eq!(run("v.a - (-v.b)", 13, &vars), 13.0);
            assert_eq!(
                run("(v.a)-v.b", 13, &vars),
                7.0,
                "a section before `-` is an operand"
            );
            assert_eq!(
                run("math.abs(v.a)-v.b", 13, &[("a", -4.0), ("b", 1.0)]),
                3.0,
                "a call before `-` is an operand"
            );
        }
    }

    /// Unary operators stack and bind tighter than every binary level.
    #[test]
    fn unary_operators() {
        assert_eq!(constant("-!!!0"), -1.0);
        assert_eq!(constant("!-!!!0"), 0.0);
        assert_eq!(constant("1+!-!!!0"), 1.0);
        #[cfg(feature = "vm")]
        {
            assert_eq!(
                run("!v.a+v.b", 13, &[("a", 0.0), ("b", 5.0)]),
                6.0,
                "(!a)+b, not !(a+b)"
            );
            assert_eq!(
                run("!v.a==v.b", 13, &[("a", 0.0), ("b", 2.0)]),
                0.0,
                "(!a)==b, not !(a==b)"
            );
            assert_eq!(run("-v.a*v.b", 13, &[("a", 2.0), ("b", 3.0)]), -6.0);
            assert_eq!(run("!!v.a", 13, &[("a", 5.0)]), 1.0);
        }
    }

    /// Six levels below version 6, two from 6.
    #[cfg(feature = "vm")]
    #[test]
    fn comparison_bands() {
        for version in [-1, 0, 5] {
            assert_eq!(
                run(
                    "v.a>v.b<v.c",
                    version,
                    &[("a", 1.0), ("b", 2.0), ("c", 3.0)]
                ),
                0.0,
                "a>(b<c) at {version}"
            );
            assert_eq!(
                run(
                    "v.a!=v.b==v.c",
                    version,
                    &[("a", 1.0), ("b", 2.0), ("c", 0.0)]
                ),
                1.0,
                "a!=(b==c) at {version}"
            );
        }
        for version in [6, 13] {
            assert_eq!(
                run(
                    "v.a>v.b<v.c",
                    version,
                    &[("a", 1.0), ("b", 2.0), ("c", 3.0)]
                ),
                1.0,
                "(a>b)<c at {version}"
            );
            assert_eq!(
                run(
                    "v.a!=v.b==v.c",
                    version,
                    &[("a", 1.0), ("b", 2.0), ("c", 0.0)]
                ),
                0.0,
                "(a!=b)==c at {version}"
            );
            assert_eq!(
                run(
                    "v.a < v.b == v.c < v.d",
                    version,
                    &[("a", 1.0), ("b", 2.0), ("c", 3.0), ("d", 4.0)]
                ),
                1.0,
                "(a<b)==(c<d) at {version}"
            );
        }
    }

    /// `||` binds tighter than `&&` below version 6, looser from 6.
    #[cfg(feature = "vm")]
    #[test]
    fn logical_bands() {
        for version in [-1, 0, 5] {
            assert_eq!(
                run(
                    "v.a||v.b&&v.c",
                    version,
                    &[("a", 1.0), ("b", 0.0), ("c", 0.0)]
                ),
                0.0,
                "(a||b)&&c at {version}"
            );
            assert_eq!(
                run(
                    "v.a&&v.b||v.c",
                    version,
                    &[("a", 0.0), ("b", 0.0), ("c", 1.0)]
                ),
                0.0,
                "a&&(b||c) at {version}"
            );
        }
        for version in [6, 13] {
            assert_eq!(
                run(
                    "v.a||v.b&&v.c",
                    version,
                    &[("a", 1.0), ("b", 0.0), ("c", 0.0)]
                ),
                1.0,
                "a||(b&&c) at {version}"
            );
            assert_eq!(
                run(
                    "v.a&&v.b||v.c",
                    version,
                    &[("a", 0.0), ("b", 0.0), ("c", 1.0)]
                ),
                1.0,
                "(a&&b)||c at {version}"
            );
        }
        let all = [("a", 1.0), ("b", 1.0), ("c", 1.0), ("d", 0.0)];
        assert_eq!(run("v.a && v.b && v.c", 13, &all), 1.0);
        assert_eq!(run("v.a || v.b || v.c || v.d", 13, &all), 1.0);
        assert_eq!(run("v.a && (v.b && v.d)", 13, &all), 0.0);
    }

    /// Two left-associative binary operators below version 5, right-associative from 5.
    #[test]
    fn conditional_bands() {
        for version in [-1, 0, 4] {
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
            #[cfg(feature = "vm")]
            assert_eq!(
                run(
                    "v.a?v.b:v.c?v.d:v.e",
                    version,
                    &[("a", 1.0), ("b", 0.0), ("c", 1.0), ("d", 7.0), ("e", 9.0)]
                ),
                9.0,
                "(a?b:c)?d:e at {version}"
            );
        }
        for version in [5, 13] {
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
            #[cfg(feature = "vm")]
            {
                assert_eq!(
                    run(
                        "v.a?v.b:v.c?v.d:v.e",
                        version,
                        &[("a", 1.0), ("b", 0.0), ("c", 1.0), ("d", 7.0), ("e", 9.0)]
                    ),
                    0.0,
                    "a?b:(c?d:e) at {version}"
                );
                assert_eq!(
                    run(
                        "v.a?v.b?v.c:v.d:v.e",
                        version,
                        &[("a", 1.0), ("b", 0.0), ("c", 5.0), ("d", 6.0), ("e", 7.0)]
                    ),
                    6.0,
                    "a?(b?c:d):e at {version}"
                );
            }
        }
        assert!(at("v.a ? v.b", 13).parsed(), "the else branch is optional");
        assert!(at("v.a ? v.b ? v.c ? v.d : v.e : v.f : v.g", 13).parsed());
        #[cfg(feature = "vm")]
        {
            assert_eq!(run("v.a ? v.b", 13, &[("a", 0.0), ("b", 5.0)]), 0.0);
            assert_eq!(run("v.a ? v.b", 13, &[("a", 1.0), ("b", 5.0)]), 5.0);
            assert_eq!(
                run(
                    "v.a||v.b?v.c:v.d",
                    13,
                    &[("a", 0.0), ("b", 1.0), ("c", 3.0), ("d", 4.0)]
                ),
                3.0,
                "(a||b)?c:d"
            );
        }
    }

    /// `x·c`, `x + c` and `−x` fold into the post-op of the node they apply to without changing the
    /// value.
    #[cfg(feature = "vm")]
    #[test]
    fn post_op_folding() {
        let vars = [("x", 3.0), ("y", 5.0), ("z", 11.0)];
        for (source, expected) in [
            ("v.x + 1", 4.0),
            ("1 + v.x", 4.0),
            ("v.x - 1", 2.0),
            ("1 - v.x", -2.0),
            ("-v.x", -3.0),
            ("-(-v.x)", 3.0),
            ("v.x * 2 + 1", 7.0),
            ("(v.x + 1) * 2 + 3", 11.0),
            ("3 - 2 * v.x", -3.0),
            ("v.x * 1", 3.0),
            ("v.x + 0", 3.0),
            ("v.x * 0", 0.0),
            ("math.abs(v.x) * 2 + 1", 7.0),
            ("(v.x < 1) * 2 + 1", 1.0),
            ("(v.x ? v.y : 2) * 3", 15.0),
            ("v.x * v.y * 2", 30.0),
            ("2 * v.x * v.y", 30.0),
            ("v.x + 1 + v.y + 2", 11.0),
            ("(v.x + v.y) * 2 + 1", 17.0),
            ("v.z - (v.x + 1)", 7.0),
        ] {
            assert_eq!(run(source, 13, &vars), expected, "{source}");
            assert!(messages_at(source, 13).is_empty(), "{source}");
        }
    }

    /// Equal `+` terms merge; terms that cancel disappear — the value is the sum all the same.
    #[cfg(feature = "vm")]
    #[test]
    fn merging_of_equal_terms() {
        let vars = [("x", 3.0), ("y", 5.0)];
        for (source, expected) in [
            ("v.x + v.x + 3", 9.0),
            ("v.x + v.y + v.x", 11.0),
            ("v.x * 2 + v.x * 3", 15.0),
            ("v.x - v.x", 0.0),
            ("v.x + v.y - v.x", 5.0),
            ("v.x + v.y - v.x - v.y", 0.0),
            ("v.x - v.x + 1", 1.0),
            ("v.x * v.y + v.y * v.x", 30.0),
            // An odd count of equal terms: the odd one out is added last, with its offset (3*3+3,
            // 5*3+10).
            ("(v.x+1)+(v.x+1)+(v.x+1)", 12.0),
            ("(v.x+2)+(v.x+2)+(v.x+2)+(v.x+2)+(v.x+2)", 25.0),
        ] {
            assert_eq!(run(source, 13, &vars), expected, "{source}");
        }
        assert_eq!(run(&vec!["v.x"; 255].join("+"), 13, &vars), 765.0);
        assert_eq!(
            run("math.floor(v.x) + math.floor(v.x)", 13, &[("x", 3.5)]),
            6.0
        );
        assert_eq!(
            run(
                "math.abs(v.x) + math.abs(v.x) + math.abs(v.x)",
                13,
                &[("x", -2.5)]
            ),
            7.5
        );
        assert_eq!(run("t.x = 2; return t.x + t.x;", 13, &[]), 4.0);
        // Random functions are never merged: each is its own draw.
        let mut generator = Xorshift128::new();
        let (first, second) = {
            use molangx::rng::sample;
            (sample(&mut generator), sample(&mut generator))
        };
        assert_eq!(
            run("math.random(0,1) + math.random(0,1)", 13, &[]),
            first + second
        );
    }

    /// Equal terms are compared with every member of a `v.a.b…` chain.
    #[test]
    fn merging_of_long_member_chains() {
        let chain = |members: usize, last: &str| format!("v.a{}{last}", ".b".repeat(members));
        for members in [3, 8, 9, 10, 40, 200] {
            for source in [
                format!("{0} + {0}", chain(members, ".c")),
                format!("{} + {}", chain(members, ".c"), chain(members, ".d")),
                format!("{0} + {0} * 2", chain(members, ".c")),
            ] {
                let compiled = at(&source, 13);
                assert_eq!(compiled.failure(), None, "{members} members");
                assert!(messages_at(&source, 13).is_empty(), "{members} members");
            }
        }
        #[cfg(feature = "vm")]
        for members in [3, 8, 9, 10] {
            let setup = format!(
                "{} = 2; {} = 5; ",
                chain(members, ".c"),
                chain(members, ".d")
            );
            assert_eq!(
                run(
                    &format!("{setup}return {0} + {0};", chain(members, ".c")),
                    13,
                    &[]
                ),
                4.0,
                "{members} members"
            );
            assert_eq!(
                run(
                    &format!(
                        "{setup}return {} + {};",
                        chain(members, ".c"),
                        chain(members, ".d")
                    ),
                    13,
                    &[]
                ),
                7.0,
                "{members} members"
            );
            assert_eq!(
                run(
                    &format!("{setup}return {0} + {0} * 2;", chain(members, ".c")),
                    13,
                    &[]
                ),
                6.0,
                "{members} members"
            );
        }
    }

    /// Values of `+` and `&&` / `||` expressions that differ from plain arithmetic.
    #[cfg(feature = "vm")]
    #[test]
    fn addition_quirks() {
        // The terms merge into twice the first.
        let x2 = [("x", 2.0)];
        assert_eq!(run("(v.x == 1) + (v.x == 2)", 13, &x2), 0.0);
        assert_eq!(run("math.max(v.x, 1) + math.max(v.x, 5)", 13, &x2), 4.0);
        assert_eq!(run("math.pow(v.x, 2) + math.pow(v.x, 3)", 13, &x2), 8.0);
        assert_eq!(run("(v.x < 1) + (v.x < 3)", 13, &x2), 0.0);
        // Merged terms add their offsets.
        assert_eq!(run("(v.x + 1) + (v.x + 2)", 13, &[("x", 3.0)]), 9.0);
        // A term whose scale cancels is dropped with its offset.
        assert_eq!(run("(v.x + 1) - v.x", 13, &[("x", 3.0)]), 0.0);
        assert_eq!(
            run("(v.x + v.y + 1) - v.x - v.y", 13, &[("x", 3.0), ("y", 4.0)]),
            0.0
        );
        // A nested `&&` / `||` flattens into its parent, dropping its post-op.
        let flags = [("a", 1.0), ("b", 1.0), ("c", 1.0), ("z", 0.0)];
        assert_eq!(run("v.a && ((v.b && v.c) - 1)", 13, &flags), 1.0);
        assert_eq!(run("v.a && (1 - (v.b && v.c))", 13, &flags), 1.0);
        assert_eq!(run("v.z || ((v.b || v.c) - 1)", 13, &flags), 1.0);
    }

    #[cfg(feature = "vm")]
    #[test]
    fn constants_move_into_the_node() {
        for (source, x, expected) in [
            ("v.x < 1", 3.0, 0.0),
            ("1 < v.x", 3.0, 1.0),
            ("v.x == 1", 1.0, 1.0),
            ("1 == v.x", 1.0, 1.0),
            ("math.max(2, v.x)", 3.0, 3.0),
            ("math.mod(v.x, 2)", 3.0, 1.0),
            ("math.mod(2, v.x)", 3.0, 2.0),
            ("math.pow(v.x, 2)", 3.0, 9.0),
        ] {
            assert_eq!(run(source, 13, &[("x", x)]), expected, "{source}");
        }
        let mut env = NoHostEnv::new();
        env.variables
            .set(VariableName::new("x"), Value::string("abc"));
        env.variables
            .set(VariableName::new("s"), Value::string("abd"));
        for (source, expected) in [
            ("v.x == 'abc'", 1.0),
            ("'abc' != v.s", 1.0),
            ("'abc' != v.x", 0.0),
        ] {
            let expr = at(source, 13).expr().cloned().expect("compiles");
            assert_eq!(expr.eval_f32(&mut env.cx()), expected, "{source}");
        }
        let assign = at("v.y = 1;", 13).expr().cloned().expect("compiles");
        assign.eval_f32(&mut env.cx());
        assert_eq!(
            env.variables.get(VariableName::new("y")),
            Some(&Value::Float(1.0))
        );
        at("v.z = 'abc';", 13)
            .expr()
            .cloned()
            .expect("compiles")
            .eval_f32(&mut env.cx());
        assert_eq!(
            env.variables.get(VariableName::new("z")),
            Some(&Value::string("abc")),
            "a string stays a child of `=`"
        );
        assert_eq!(
            at("array.foo[0]", 13).failure(),
            Some(CompileFailure::UsesArrays)
        );
    }

    /// The lowering pass steps over one byte after a backslash, the string scanner over two: in
    /// `'\\'X'` the lowering pass ends a string at the second quote (so `X` is lowered) while the
    /// scanner reads `\\'X` as its content.
    #[cfg(feature = "vm")]
    #[test]
    fn lowering_and_string_scanning_disagree_about_backslashes() {
        assert_eq!(value_of("'\\\\'X'"), Value::string("\\\\'x"));
        // Outside that disagreement, a protected byte inside a string keeps its case.
        assert_eq!(value_of("'\\'X'"), Value::string("\\'X"));
    }

    /// An escape skips three bytes and the string still ends at the quote after them.
    #[cfg(feature = "vm")]
    #[test]
    fn a_string_with_an_escape_far_from_the_start_is_one_string() {
        assert_eq!(at("'ab\\xy'", 13).failure(), None);
        assert_eq!(value_of("'ab\\xy'"), Value::string("ab\\xy"));
        assert_eq!(value_of("'abcd\\xy'"), Value::string("abcd\\xy"));
    }

    /// The hash covers the raw bytes, escapes included.
    #[cfg(feature = "vm")]
    #[test]
    fn string_hashes_of_raw_bytes() {
        assert_eq!(HashedStr::new("a\\'b").as_u64(), 3_327_885_792_095_995_213);
        assert_eq!(HashedStr::new("a\\\\b").as_u64(), 3_327_983_648_630_905_864);
        assert_eq!(value_of("'a\\'b'"), Value::string("a\\'b"));
        assert_eq!(value_of("'a\\\\b'"), Value::string("a\\\\b"));
    }

    #[cfg(feature = "vm")]
    #[test]
    fn member_accessors_carry_their_name() {
        let mut env = NoHostEnv::new();
        env.variables.set(
            VariableName::new("x"),
            Value::structure(StructValue::from([("y", 2.0), ("z", 5.0)])),
        );
        for (source, expected) in [
            ("v.x.y + v.x.y", 4.0),
            ("v.x.y + v.x.z", 7.0),
            ("v.x.Y + v.x.y", 4.0),
        ] {
            let expr = at(source, 13).expr().cloned().expect("compiles");
            assert_eq!(expr.eval_f32(&mut env.cx()), expected, "{source}");
        }
    }

    #[test]
    fn minus_after_an_operand_is_binary() {
        #[cfg(feature = "vm")]
        {
            let vars = [("a", 7.0), ("b", 2.0)];
            for (source, expected) in [
                ("(v.a)-v.b", 5.0),
                ("math.abs(v.a)-v.b", 5.0),
                ("v.a-v.b", 5.0),
                ("2-v.b", 0.0),
                (
                    "math.inverse_lerp(v.a, 1, 2)-v.b",
                    (2.0_f32 - 7.0) / (1.0 - 7.0) - 2.0,
                ),
            ] {
                assert_eq!(run(source, 13, &vars), expected, "{source}");
            }
            assert_eq!(run("math.pi-v.b", 13, &vars), std::f32::consts::PI - 2.0);
        }
        for source in [
            "v.a.c-v.b",
            "q.count(1)-v.b",
            "array.a[0]-v.b",
            "c.o->v.a-v.b",
            "this-v.b",
        ] {
            assert!(at(source, 13).parsed(), "{source}");
        }
        // Below version 3 a string and a resource variable are operands too.
        assert!(at("'a'-v.b", 2).parsed());
        assert!(at("geometry.a-v.b", 2).parsed());
        assert_eq!(
            at("geometry.a-v.b", 2).failure(),
            Some(CompileFailure::UsesResources)
        );
    }

    #[cfg(feature = "vm")]
    #[test]
    fn minus_after_an_operator_is_unary() {
        let vars = [("a", 2.0), ("b", 5.0), ("c", 7.0)];
        for (source, expected) in [
            ("-v.a", -2.0),
            ("!-v.a", 0.0),
            ("v.b+-v.a", 3.0),
            ("v.b/-v.a", -2.5),
            ("v.b*-v.a", -10.0),
            ("v.b<-v.a", 0.0),
            ("v.b==-v.a", 0.0),
            ("v.b&&-v.a", 1.0),
            ("v.b||-v.a", 1.0),
            ("v.b??-v.a", 5.0),
            ("v.b?-v.a:-v.c", -2.0),
            ("v.b=1;-v.a;", 0.0),
            ("return -v.a;", -2.0),
            ("math.max(-v.a,-v.b)", -2.0),
        ] {
            assert_eq!(run(source, 13, &vars), expected, "{source}");
            assert!(messages_at(source, 13).is_empty(), "{source}");
        }
        assert_eq!(run("v.b=-v.a; return v.b;", 13, &vars), -2.0);
        assert_eq!(run("{-v.a;}; return 3;", 13, &vars), 3.0);
    }

    #[test]
    fn assignment_as_an_operand() {
        for source in [
            "return v.a = 5;",
            "1 ? (v.a = 7) : 0;",
            "v.b = (v.a = 1);",
            "(v.a = 1) == 1;",
            "q.count(v.a = 1);",
        ] {
            let compiled = at(source, 13);
            assert!(
                compiled.parses_cleanly(),
                "{source}: {:?}",
                messages(&compiled)
            );
        }
        #[cfg(feature = "vm")]
        {
            assert_eq!(run("return v.a = 5;", 13, &[]), 5.0);
            assert_eq!(run("1 ? (v.a = 7) : 0; return v.a;", 13, &[]), 7.0);
            assert_eq!(run("v.b = (v.a = 1); return v.a + v.b;", 13, &[]), 2.0);
            assert_eq!(run("return (v.a = 1) == 1;", 13, &[]), 1.0);
        }
    }

    #[cfg(feature = "vm")]
    #[test]
    fn literal_values_are_fnv1_hashes() {
        for (literal, hash) in [
            ("'a'", 12_638_153_115_695_167_422_u64),
            ("'abc'", 15_626_587_013_303_479_755),
            ("'ABC'", 15_595_941_425_208_037_995),
            ("' '", 12_638_153_115_695_167_487),
        ] {
            let inner = &literal[1..literal.len() - 1];
            assert_eq!(HashedStr::new(inner).as_u64(), hash, "{literal}");
            assert_eq!(value_of(literal), Value::string(inner), "{literal}");
        }
    }

    #[cfg(feature = "vm")]
    #[test]
    fn pitfall_lowering_inside_strings() {
        assert_ne!(value_of("'ABC'"), value_of("'abc'"));
        assert_eq!(value_of("'abc'"), Value::string("abc"));
        assert_eq!(HashedStr::new("abc").as_u64(), 15_626_587_013_303_479_755);
        let vars = [("a", -3.0)];
        assert_eq!(
            run("MATH.ABS(V.A) + Q.IS_BABY", 13, &vars),
            run("math.abs(v.a) + q.is_baby", 13, &vars)
        );
        assert_eq!(run("MATH.ABS(V.A) + Q.IS_BABY", 13, &vars), 3.0);
    }

    #[test]
    fn pitfall_unary_binding() {
        assert_eq!(constant("-!!!0"), -1.0);
        #[cfg(feature = "vm")]
        {
            assert_eq!(run("!v.a==v.b", 13, &[("a", 0.0), ("b", 2.0)]), 0.0);
            assert_eq!(run("-v.a*v.b", 13, &[("a", 2.0), ("b", 3.0)]), -6.0);
        }
    }

    #[test]
    fn every_surviving_op_links() {
        let sources = [
            "v.x",
            "t.x = 1; return t.x;",
            "c.x",
            "this",
            "'abc'",
            "v.a.b",
            "q.is_baby",
            "!v.x",
            "v.x + v.y + v.z",
            "v.x * v.y",
            "v.x / v.y",
            "math.mod(v.x, v.y)",
            "math.mod(v.x, 3)",
            "math.abs(v.x) + math.acos(v.x) + math.asin(v.x) + math.atan(v.x) + math.ceil(v.x) + math.cos(v.x)",
            "math.exp(v.x) + math.floor(v.x) + math.hermite_blend(v.x) + math.ln(v.x) + math.min_angle(v.x)",
            "math.round(v.x) + math.sin(v.x) + math.sign(v.x) + math.sqrt(v.x) + math.trunc(v.x)",
            "math.atan2(v.x, v.y) + math.copy_sign(v.x, v.y) + math.max(v.x, v.y) + math.min(v.x, v.y) + math.pow(v.x, v.y)",
            "math.max(v.x, 2) + math.min(2, v.x) + math.pow(v.x, 2)",
            "math.clamp(v.x, 0, 1) + math.lerp(v.a, v.b, v.t) + math.lerprotate(v.a, v.b, v.t) + math.inverse_lerp(v.a, v.b, v.t)",
            "math.ease_in_out_elastic(v.a, v.b, v.t) + math.ease_out_bounce(v.a, v.b, v.t)",
            "math.random(1, 2) + math.random(v.x, 2) + math.random_integer(1, 6) + math.random_integer(v.x, 6)",
            "math.die_roll(2, 1, 6) + math.die_roll_integer(2, 1, 6)",
            "v.x < v.y",
            "v.x < 1",
            "v.x <= 1 && v.x >= 0 || v.x > 5",
            "v.x == v.y",
            "v.x == 1",
            "v.x != 'abc'",
            "v.x ?? 1",
            "v.x ? 1 : 2",
            "v.x ? 1",
            "loop(3, {v.x = v.x + 1; v.x > 1 ? break; v.x < 0 ? continue;});",
            "for_each(t.e, v.arr, {t.n = 1;});",
            "c.other->v.x",
            "c.other->q.is_baby",
            "v.a.b = 1;",
            "t.a.b = 1;",
            "return 1;",
            "break;",
        ];
        for source in sources {
            let compiled = at(source, 13);
            assert_eq!(
                compiled.failure(),
                None,
                "{source}: {:?}",
                compiled.diagnostics()
            );
            assert!(compiled.expr().is_some(), "{source}");
        }
    }
}

mod deviation_switches {
    //! Which deviations and budgets switch off.

    use molangx::compile::Deviations;
    #[cfg(feature = "vm")]
    use molangx::vm::EvalLimits;

    #[test]
    fn every_compiler_deviation_switches_off() {
        let Deviations {
            true_false_prefix_advance,
            source_length_limit,
            validate_nested,
            query_arity_lint,
            query_client_only,
            object_version_warning,
            diagnostic_limit,
            ..
        } = Deviations::NONE;
        assert!(
            ![
                true_false_prefix_advance,
                source_length_limit,
                validate_nested,
                query_arity_lint,
                query_client_only,
                object_version_warning,
                diagnostic_limit
            ]
            .contains(&true)
        );
        let Deviations {
            true_false_prefix_advance,
            source_length_limit,
            validate_nested,
            query_arity_lint,
            query_client_only,
            object_version_warning,
            diagnostic_limit,
        } = Deviations::ALL;
        assert!(
            [
                true_false_prefix_advance,
                source_length_limit,
                validate_nested,
                query_arity_lint,
                query_client_only,
                object_version_warning,
                diagnostic_limit
            ]
            .iter()
            .all(|&on| on)
        );
    }

    /// The operand-stack cap has no switch.
    #[cfg(feature = "vm")]
    #[test]
    fn every_evaluator_budget_switches_off() {
        assert_eq!(EvalLimits::NONE.loop_iterations, None);
        assert_eq!(EvalLimits::NONE.total_steps, None);
        assert_eq!(EvalLimits::NONE.struct_depth, None);
        assert_eq!(EvalLimits::DEFAULT.loop_iterations, Some(1_024));
        assert_eq!(EvalLimits::DEFAULT.total_steps, Some(1_048_576));
        assert_eq!(EvalLimits::DEFAULT.struct_depth, Some(32));
        assert_eq!(EvalLimits::OPERAND_STACK_CAP, 65_536);
    }
}

mod public_fields {
    //! These tests name every field, so a field becoming private, or a new one, fails to compile.

    use molangx::compile::Deviations;
    #[cfg(feature = "vm")]
    use molangx::vm::EvalLimits;

    #[test]
    fn deviations_are_a_struct_of_public_switches() {
        let Deviations {
            true_false_prefix_advance,
            source_length_limit,
            validate_nested,
            query_arity_lint,
            query_client_only,
            object_version_warning,
            diagnostic_limit,
        } = Deviations::ALL;
        assert!(
            true_false_prefix_advance
                && source_length_limit
                && validate_nested
                && query_arity_lint
                && query_client_only
                && object_version_warning
                && diagnostic_limit
        );
        let all_off = Deviations {
            true_false_prefix_advance: false,
            source_length_limit: false,
            validate_nested: false,
            query_arity_lint: false,
            query_client_only: false,
            object_version_warning: false,
            diagnostic_limit: false,
        };
        assert_eq!(all_off, Deviations::NONE);
    }

    #[cfg(feature = "vm")]
    #[test]
    fn eval_limits_are_a_struct_of_public_budgets() {
        let EvalLimits {
            loop_iterations,
            total_steps,
            struct_depth,
            struct_members,
            query_depth,
        } = EvalLimits::DEFAULT;
        assert_eq!(
            (
                loop_iterations,
                total_steps,
                struct_depth,
                struct_members,
                query_depth
            ),
            (Some(1_024), Some(1_048_576), Some(32), Some(256), None)
        );
        let EvalLimits {
            loop_iterations,
            total_steps,
            struct_depth,
            struct_members,
            query_depth,
        } = EvalLimits::NONE;
        assert_eq!(
            (
                loop_iterations,
                total_steps,
                struct_depth,
                struct_members,
                query_depth
            ),
            (None, None, None, None, None)
        );
        let custom = EvalLimits {
            loop_iterations: Some(3),
            ..EvalLimits::NONE
        };
        assert_eq!(custom.loop_iterations, Some(3));
    }
}
