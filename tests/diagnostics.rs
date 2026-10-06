//! The messages a compile reports: every reachable row of the parser-message table, the diagnostics
//! outside the table and their levels, the shape of a diagnostic, and the cap on how many are kept.

#![cfg(all(feature = "compiler", feature = "stdlib"))]

mod common;

use std::borrow::Cow;
use std::ops::Range;

use common::compile_support::{at, client_at, messages, messages_at, server_at};
use molangx::catalog::{QueryAdmission, QueryDecl, QuerySetMask, ReturnType, Side};
use molangx::compile::{
    CompileFailure, CompileOptions, Deviations, Expr, MAX_DIAGNOSTICS, MAX_SOURCE_LEN, compile,
};
use molangx::diag::{DiagCode, Diagnostic, Effect, LanguageMessage, Severity};
use molangx::ops::OpSet;
use molangx::version::{ExperimentMask, MolangVersion, RawVersion};

struct Case {
    row: u8,
    source: &'static str,
    version: i16,
    messages: &'static [&'static str],
    rejected: bool,
}

const fn reject(
    row: u8,
    source: &'static str,
    version: i16,
    messages: &'static [&'static str],
) -> Case {
    Case {
        row,
        source,
        version,
        messages,
        rejected: true,
    }
}

const fn keep(
    row: u8,
    source: &'static str,
    version: i16,
    messages: &'static [&'static str],
) -> Case {
    Case {
        row,
        source,
        version,
        messages,
        rejected: false,
    }
}

const TEMP_LHS: &str = "Error: left side of an assignment expression can only use temp variables if they are on their own and not part of a more complicated expression.";

const POINTER_WRITE: &str = "Error: Assignment attempted on Pointer result. Writing to another entity's public variables is not supported, only reading them.";

const COALESCE: &str = "Error: found left-hand-side of ?? expression that isn't a direct-variable reference - this is unsupported at this time.";

const POINTER_RHS: &str = "Error: right-hand-side of pointer expression did not evaluate to an entity variable or query function";

const NESTED_POINTER: &str = "Error: nested pointer statements (eg: A->B->C) are not yet supported.  Store A->B in a variable (eg: D), then use D->C";

const ASSIGNMENT_FORM: &str = "Error: assignment expressions may either be of the form 'A->B = D' or 'C = D' where B and C may be an entity variable or member accessor, or C may be a temp variable.  Found an expression where C is a Context Variable 'context.' or 'c.'";

const CASES: &[Case] = &[
    reject(1, "", 4, &["No tokens found in expression"]),
    reject(1, "  \t\n", 13, &["No tokens found in expression"]),
    reject(1, "", 3, &[]),
    reject(1, " ", -1, &[]),
    reject(2, "$", 13, &["unrecognized token: $"]),
    reject(
        3,
        "foo",
        13,
        &["Error: unknown token: foo", "unrecognized token: foo"],
    ),
    reject(
        4,
        "'abc",
        13,
        &[
            "Error: Molang string missing final ' character",
            "unrecognized token: 'abc",
        ],
    ),
    reject(
        5,
        "query.does_not_exist",
        13,
        &[
            "Failed to resolve query query.does_not_exist.  Either the query does not exist or it is not supported in this context.",
            "unrecognized token: query.does_not_exist",
        ],
    ),
    keep(
        6,
        "1e",
        13,
        &["error parsing float string, expected '+' or '-' after 'e': "],
    ),
    keep(
        6,
        "1e",
        0,
        &["error parsing float string, expected '+' or '-' after 'e': "],
    ),
    reject(
        7,
        "v.x = 1",
        13,
        &["Error: complex expressions (contains either '=' or ';') must end with a ';'"],
    ),
    reject(
        7,
        "v.x = 1",
        0,
        &["Error: complex expressions (contains either '=' or ';') must end with a ';'"],
    ),
    reject(
        8,
        "1 2",
        13,
        &[
            "found multiple operations without a combining operation between them:\n\t1.000000\n\t2.000000",
        ],
    ),
    reject(
        8,
        "v.x[0]",
        13,
        &[
            "found multiple operations without a combining operation between them:\n\tvariable.\n\t[",
        ],
    ),
    reject(
        9,
        ".foo",
        13,
        &[
            "Error: cannot start an expression with a member accessor; member accessors require a variable of which to access a member.",
        ],
    ),
    reject(
        10,
        "3 { 6",
        13,
        &[
            "Unable to find matching closing section symbol for symbol at 2(Left Brace '{') -- looking for Right Brace '}'",
            "Error: Could not find Right Brace '}' to close section started with Left Brace '{'",
        ],
    ),
    reject(
        10,
        "((1)",
        13,
        &[
            "Unable to find matching closing section symbol for symbol at 1(Left Parenthesis '(') -- looking for Right Parenthesis ')'",
            "Error: Could not find Right Parenthesis ')' to close section started with Left Parenthesis '('",
        ],
    ),
    reject(
        11,
        "[1)",
        13,
        &[
            "Unable to match closing section symbol at 1(Left Bracket '[') - looking for Right Bracket ']', found Right Parenthesis ')' at 2",
            "Error: Could not find Right Bracket ']' to close section started with Left Bracket '['",
        ],
    ),
    reject(
        11,
        "{ ( [ )",
        13,
        &[
            "Unable to match closing section symbol at 3(Left Bracket '[') - looking for Right Bracket ']', found Right Parenthesis ')' at 3",
            "Error: Could not find Right Brace '}' to close section started with Left Brace '{'",
        ],
    ),
    reject(
        13,
        "query.is_baby()",
        13,
        &[
            "Error: Query Function 'query.' or 'q.' operators with no params should not use parentheses",
        ],
    ),
    reject(
        13,
        "loop()",
        13,
        &["Error: Loop 'loop' operators with no params should not use parentheses"],
    ),
    reject(
        14,
        ";",
        13,
        &["Error: expressions can't begin with a semicolon"],
    ),
    reject(
        14,
        ";1;2;",
        13,
        &["Error: expressions can't begin with a semicolon"],
    ),
    reject(15, "array.a[ ]", 13, &["Error: array expression is empty"]),
    reject(
        16,
        "1 +",
        13,
        &["Error: binary Add '+' operator at end of expression"],
    ),
    reject(
        16,
        "* 1",
        13,
        &["Error: binary Multiply '*' operator at end of expression"],
    ),
    reject(
        16,
        "v.a - > v.b",
        13,
        &["Error: binary Greater Than '>' operator at end of expression"],
    ),
    reject(
        16,
        "math.max(1,)",
        13,
        &["Error: binary Comma ',' operator at end of expression"],
    ),
    reject(
        17,
        "math.abs",
        13,
        &[
            "Error: Absolute Value 'math.abs' operator at end of expression without a parenthesis section",
        ],
    ),
    reject(
        17,
        "math.abs -2",
        13,
        &["Error: Absolute Value 'math.abs' operator not followed by parenthesis section"],
    ),
    reject(18, "1 -", 13, &["Error: '-' not followed by expression"]),
    reject(
        18,
        "!",
        13,
        &["Error: logical-not ('!') must be followed by expression"],
    ),
    reject(
        18,
        "continue -1",
        13,
        &["Error: unknown Continue 'continue' operation in expression"],
    ),
    reject(
        19,
        "1 ?",
        5,
        &["Error: could not find sub-expressions for Conditional '?' operator"],
    ),
    reject(
        19,
        "2 : 7",
        13,
        &["Error: could not find sub-expressions for Conditional Else ':' operator"],
    ),
    reject(
        19,
        "1 ?",
        4,
        &["Error: binary Conditional '?' operator at end of expression"],
    ),
    reject(
        20,
        "return;",
        13,
        &["Error: unary Return 'return' operator not followed by expression"],
    ),
    reject(
        23,
        "math.clamp(1,2)",
        13,
        &["Unexpected number of parameters to Clamp 'math.clamp' function - expected 3, found 2."],
    ),
    reject(
        23,
        "math.min(1)",
        0,
        &["Unexpected number of parameters to Min 'math.min' function - expected 2, found 1."],
    ),
    reject(
        23,
        "math.max(1,2,3)",
        13,
        &["Unexpected number of parameters to Max 'math.max' function - expected 2, found 3."],
    ),
    reject(
        24,
        "()",
        13,
        &[
            "Malformed Left Parenthesis '(' expression. It has 0 children but should have between 1 and -1",
        ],
    ),
    reject(
        24,
        "{};",
        13,
        &[
            "Malformed Left Brace '{' expression. It has 0 children but should have between 1 and -1",
        ],
    ),
    reject(
        24,
        "math.sin()",
        13,
        &[
            "Malformed Left Parenthesis '(' expression. It has 0 children but should have between 1 and -1",
        ],
    ),
    reject(
        24,
        "array.x[1][2]",
        13,
        &["Malformed Array '[]' expression. It has 2 children but should have between 1 and 1"],
    ),
    reject(
        24,
        "1++",
        13,
        &["Malformed Add '+' expression. It has 0 children but should have between 2 and -1"],
    ),
    reject(
        25,
        "v.a?v.b?v.c:v.d:v.e",
        4,
        &["Unsupported Conditional Else ':' operator in expression optimization"],
    ),
    reject(
        31,
        "4 } 5;",
        13,
        &[
            "Error: Could not reduce sub-expression before a semicolon to a single operation to evaluate",
        ],
    ),
    reject(
        26,
        "1 , 8",
        13,
        &[
            "Error: Unexpected Comma ',' operator not inside an arguments list for a query, loop, or math function",
        ],
    ),
    reject(
        26,
        "math.abs(1, 2)",
        13,
        &[
            "Error: Unexpected Comma ',' operator not inside an arguments list for a query, loop, or math function",
        ],
    ),
    reject(
        27,
        "for_each(v.x, v.arr);",
        13,
        &[
            "Error: for_each requires three parameters - a variable to represent an element of an array, an expression resulting in an array, and an expression to run per element of that array.",
        ],
    ),
    reject(
        28,
        "c.x = 1;",
        13,
        &[
            "Error: assignment to non-variable not allowed. Expression is trying to assign to a: Context Variable 'context.' or 'c.'",
        ],
    ),
    reject(
        28,
        "c.x = 1;",
        2,
        &[
            "Error: assignment to non-variable not allowed. Expression is trying to assign to a: Context Variable 'context.' or 'c.'",
        ],
    ),
    reject(
        28,
        "v.x = v.y = 1;",
        13,
        &[
            "Error: assignment to non-variable not allowed. Expression is trying to assign to a: Assignment '='",
        ],
    ),
    reject(
        28,
        "(v.x + 1) = 0;",
        13,
        &[
            "Error: assignment to non-variable not allowed. Expression is trying to assign to a: Left Parenthesis '('",
        ],
    ),
    reject(
        28,
        "v.x->temp.x = 0;",
        13,
        &[
            "Error: assignment to non-variable not allowed. Expression is trying to assign to a: Pointer '->'",
        ],
    ),
    reject(
        29,
        "1 : 2 ? 3",
        4,
        &["Error: '?' operator couldn't find a valid preceding 'if' expression"],
    ),
    reject(
        30,
        "loop(3);",
        13,
        &[
            "Error: loop requires two parameters - an expression resulting in a number of times to loop, and a {}-delimited expression to loop.",
        ],
    ),
    reject(
        30,
        "loop(3, v.x = 1);",
        13,
        &[
            "Error: loop requires two parameters - an expression resulting in a number of times to loop, and a {}-delimited expression to loop.",
        ],
    ),
    reject(
        31,
        "v.x.1 = 1;",
        13,
        &[
            "Error: Could not reduce sub-expression before a semicolon to a single operation to evaluate",
        ],
    ),
    reject(
        32,
        "(1 2)",
        4,
        &["Error: Left Parenthesis '(' optimization expected only one child operation but found 2"],
    ),
    reject(
        32,
        "[1 2]",
        13,
        &["Error: Left Bracket '[' optimization expected only one child operation but found 2"],
    ),
    keep(32, "(1 2)", 3, &[]),
    reject(
        33,
        "for_each(c.e, v.arr, {v.x = 1;});",
        13,
        &[
            "Error: for_each expressions require either a temp or entity variable as the iteration variable (the first parameter)",
        ],
    ),
    reject(
        34,
        "{1}",
        13,
        &[
            "Brace sections must only contain semicolon-delimited expressions, even if only one expression is contained.",
        ],
    ),
    reject(
        35,
        "loop",
        13,
        &[
            "Loop 'loop' operator should have exactly one child (a left-parenthesis expression with the params as children of it) prior to optimization.",
        ],
    ),
    reject(
        35,
        "math.max()",
        13,
        &[
            "Max 'math.max' operator (math, query, loop, etc) with empty parameter list should have failed to parse",
        ],
    ),
    reject(
        35,
        "math.random( )",
        0,
        &[
            "Random 'math.random' operator (math, query, loop, etc) with empty parameter list should have failed to parse",
        ],
    ),
    reject(
        35,
        "math.max(1,,2)",
        13,
        &[
            "Error while optimizing parameters for Max 'math.max' operation: comma found without a following expression.",
        ],
    ),
    reject(
        36,
        "'a' + 1",
        3,
        &[
            "'Add '+'' expression cannot take a 'String '''' argument. It only supports numerical arguments.",
        ],
    ),
    reject(
        36,
        "1 - 'a'",
        13,
        &[
            "'Negate '-'' expression cannot take a 'String '''' argument. It only supports numerical arguments.",
        ],
    ),
    reject(
        36,
        "-material.foo",
        13,
        &[
            "'Negate '-'' expression cannot take a 'Material Variable 'material.'' argument. It only supports numerical arguments.",
        ],
    ),
    reject(
        36,
        "(v.a = 1) < 2;",
        13,
        &[
            "'Less Than '<'' expression cannot take a 'Assignment '='' argument. It only supports numerical arguments.",
        ],
    ),
    keep(36, "'a' + 1", 2, &[]),
    reject(
        38,
        "return 1; return 2;",
        13,
        &["Error: unreachable statements after Return 'return'."],
    ),
    keep(
        38,
        "loop(3, {break; v.x = 1;});",
        13,
        &["Error: unreachable statements after Break 'break'."],
    ),
    keep(39, "t.x.y = 1;", 13, &[TEMP_LHS]),
    keep(
        40,
        "c.a.b = 1;",
        13,
        &[
            "Error: cannot use Context Variable 'context.' or 'c.' operators on the left side of an assignment expression",
        ],
    ),
    reject(41, "c.a->v.b->v.c", 13, &[NESTED_POINTER]),
    reject(
        42,
        "array.test[0] + 1",
        13,
        &["Error: can't currently do math operations on resource array results"],
    ),
    keep(
        42,
        "math.abs(array.test[0] * 2)",
        13,
        &["Error: can't currently do math operations on resource array results"],
    ),
    reject(44, "1 ?? 2", 13, &[COALESCE]),
    keep(44, "v.x = 1 ?? 2;", 13, &[COALESCE]),
    reject(
        45,
        "break",
        13,
        &["Error: break encountered outside of loop"],
    ),
    keep(
        45,
        "break;",
        13,
        &["Error: break encountered outside of loop"],
    ),
    reject(46, "c.a->t.b", 13, &[POINTER_RHS]),
    reject(46, "c.a->c.b->v.x", 13, &[POINTER_RHS, NESTED_POINTER]),
];

#[test]
fn every_reachable_row() {
    let mut covered = std::collections::BTreeSet::new();
    let mut produced = std::collections::BTreeSet::new();
    for case in CASES {
        let compiled = at(case.source, case.version);
        produced.extend(
            compiled
                .diagnostics()
                .iter()
                .filter_map(|d| d.language_message().map(|message| format!("{message:?}"))),
        );
        let label = if case.source.len() > 60 {
            &case.source[..60]
        } else {
            case.source
        };
        assert_eq!(
            messages(&compiled),
            case.messages,
            "#{} {label:?} at version {}",
            case.row,
            case.version
        );
        assert_eq!(
            compiled.failure() == Some(CompileFailure::Rejected),
            case.rejected,
            "#{} {label:?} at version {}",
            case.row,
            case.version
        );
        if case.rejected {
            // A rejected expression is the constant 0 and never fails without a diagnostic.
            assert_eq!(
                compiled.expr_or_zero().and_then(Expr::as_constant),
                Some(0.0),
                "#{} {label:?}",
                case.row
            );
            assert!(
                !compiled.diagnostics().is_empty(),
                "#{} {label:?} is rejected without a diagnostic",
                case.row
            );
        }
        covered.insert(case.row);
    }
    // #12 follows #10 and #11 above; the link-stage rows and the rows that need options are tested
    // below.
    covered.extend([12, 21, 22, 37, 43, 47, 48, 49]);
    let missing: Vec<u8> = molangx::diag::LanguageMessage::ALL
        .iter()
        .map(|message| message.row())
        .filter(|row| !covered.contains(row))
        .collect();
    assert!(missing.is_empty(), "rows without a test: {missing:?}");
    // Every message, not only every row, comes from source text; these ones in the tests below.
    use LanguageMessage as M;
    let elsewhere = [
        M::OperationNotAllowed,
        M::DepthOverflow,
        M::QueryNotNumerical,
        M::AssignToPointer,
        M::AssignmentForm,
        M::CompileFailed,
        M::WriteToOtherMob,
    ];
    produced.extend(elsewhere.iter().map(|message| format!("{message:?}")));
    let unproduced: Vec<&LanguageMessage> = LanguageMessage::ALL
        .iter()
        .filter(|message| !produced.contains(&format!("{message:?}")))
        .collect();
    assert!(
        unproduced.is_empty(),
        "messages no source text produces here: {unproduced:?}"
    );
}

#[test]
fn depth_overflow() {
    let nested = |depth: usize| format!("{}1{}", "(".repeat(depth), ")".repeat(depth));
    assert_eq!(at(&nested(255), 13).failure(), None);
    let compiled = at(&nested(256), 13);
    assert_eq!(
        messages(&compiled),
        [
            "Error: Expression could not be parsed due to stack depth overflow (too many sub-expressions)"
        ]
    );
    assert_eq!(compiled.failure(), Some(CompileFailure::Rejected));
    assert_eq!(compiled.diagnostics()[0].code(), DiagCode::DepthLimit);
}

#[test]
fn operation_not_allowed() {
    let no_assignment = CompileOptions {
        allowed_ops: OpSet::all().without_assignments(),
        ..client_at(13)
    };
    let compiled = compile("v.x = math.random(0, 1);", &no_assignment);
    assert_eq!(
        messages(&compiled),
        ["Expression uses operation Assignment '=' which is not allowed in this context"]
    );
    assert_eq!(compiled.failure(), Some(CompileFailure::Rejected));
    assert_eq!(compiled.diagnostics()[0].code(), DiagCode::InvalidOperation);

    let nothing_random = CompileOptions {
        allowed_ops: OpSet::all().without_assignments_or_random(),
        ..client_at(13)
    };
    let compiled = compile(
        "v.x = math.random(0, 1) + math.random_integer(0, 1);",
        &nothing_random,
    );
    assert_eq!(
        messages(&compiled),
        [
            "Expression uses operation Random 'math.random' which is not allowed in this context",
            "Expression uses operation Random Integer 'math.random_integer' which is not allowed in this context",
            "Expression uses operation Assignment '=' which is not allowed in this context",
        ]
    );
}

#[test]
fn query_must_return_a_number() {
    let default = QueryAdmission::Sets(QuerySetMask::DEFAULT);
    let non_numeric = molangx::stdlib::queries(Side::Client)
        .iter()
        .find(|decl| {
            !decl.shape().returns.intersects(ReturnType::NUMBER)
                && decl
                    .resolve(RawVersion(13), &default, ExperimentMask::empty())
                    .is_some()
                && decl
                    .resolve(RawVersion(0), &default, ExperimentMask::empty())
                    .is_some()
        })
        .map(QueryDecl::name)
        .expect("the catalogue has a query that does not return a number");
    for version in [0, 13] {
        let compiled = at(&format!("{non_numeric} + 1"), version);
        assert_eq!(
            messages(&compiled),
            ["Add '+' expressions may only contain query functions that return numbers"],
            "{non_numeric} at version {version}"
        );
        assert_eq!(compiled.failure(), Some(CompileFailure::Rejected));
        // `==` does not check its operands.
        assert_eq!(
            at(&format!("{non_numeric} == 'x'"), version).failure(),
            None
        );
    }
}

/// #43, #47, #48 and #49: findings the validation step keeps and linking then refuses.
#[test]
fn link_failures() {
    // A context variable assigned in a nested position parses, then fails to link.
    for source in ["return c.x = 1;", "(c.x = 1);", "1 ? (c.x = 1) : 0;"] {
        let compiled = at(source, 13);
        assert_eq!(
            messages(&compiled),
            [
                "Error: cannot use Context Variable 'context.' or 'c.' operators on the left side of an assignment expression".to_owned(),
                ASSIGNMENT_FORM.to_owned(),
                format!("expression '{source}' compile failed"),
            ],
            "{source}"
        );
        assert!(compiled.parsed(), "{source}: the parse itself succeeds");
        assert_eq!(
            compiled.failure(),
            Some(CompileFailure::Rejected),
            "{source}"
        );
        assert_eq!(
            compiled.expr_or_zero().and_then(Expr::as_constant),
            Some(0.0),
            "{source}"
        );
    }
    let source = "v.x->v.y = 1;";
    assert_eq!(
        messages_at(source, 13),
        [
            POINTER_WRITE.to_owned(),
            "Error: You cannot write to a variable on another mob.".to_owned(),
            format!("expression '{source}' compile failed")
        ]
    );
    // The quoted source of #48 ends at the first NUL byte.
    assert_eq!(
        messages_at("v.x->v.y = 1;\0 v.z = 2;", 13),
        [
            POINTER_WRITE.to_owned(),
            "Error: You cannot write to a variable on another mob.".to_owned(),
            "expression 'v.x->v.y = 1;' compile failed".to_owned()
        ]
    );
    assert_eq!(
        messages_at("c.other->v.x = 1;", 13)[..2],
        [
            "Error: cannot use Context Variable 'context.' or 'c.' operators on the left side of an assignment expression".to_owned(),
            POINTER_WRITE.to_owned()
        ]
    );
}

/// A nested validation finding never rejects; it is a Warning, or an Error under
/// `Deviations::NONE`.
#[test]
fn severities() {
    let rejected = at("1 +", 13);
    assert_eq!(rejected.diagnostics()[0].severity(), Severity::Error);

    let kept = at("1e", 13);
    assert_eq!(kept.diagnostics()[0].severity(), Severity::Warning);
    assert_eq!(
        kept.diagnostics()[0]
            .language_message()
            .map(LanguageMessage::effect),
        Some(Effect::Keep)
    );

    let nested = at("break;", 13);
    assert_eq!(nested.failure(), None);
    assert_eq!(nested.diagnostics()[0].severity(), Severity::Warning);
    assert_eq!(
        nested.diagnostics()[0]
            .language_message()
            .map(LanguageMessage::effect),
        Some(Effect::RootOnly)
    );
    assert!(
        !nested.parses_cleanly(),
        "a parse that logs any message is not a clean parse"
    );

    let root = at("break", 13);
    assert_eq!(root.failure(), Some(CompileFailure::Rejected));
    assert_eq!(root.diagnostics()[0].severity(), Severity::Error);

    let no_deviations = compile(
        "break;",
        &CompileOptions {
            deviations: Deviations::NONE,
            ..client_at(13)
        },
    );
    assert_eq!(no_deviations.failure(), None);
    assert_eq!(no_deviations.diagnostics()[0].severity(), Severity::Error);
}

#[test]
fn spans() {
    let source = "v.x = 1; 'abc' + 2;";
    let compiled = at(source, 13);
    let span = compiled.diagnostics()[0].span().clone();
    assert_eq!(&source[span.start as usize..span.end as usize], "'abc'");
    let source = "1 + (2 3)";
    let compiled = at(source, 13);
    let span = compiled.diagnostics()[0].span().clone();
    assert_eq!(&source[span.start as usize..span.end as usize], "(2 3");
}

#[test]
fn a_stray_colon_is_reported_at_its_own_bytes() {
    for (source, span) in [
        ("v.a v.b ? v.c : v.d : v.e", 20..21),
        ("v.a ? v.b ? v.c : v.d : v.e : v.f", 28..29),
        ("v.a ? v.b ? v.c : v.d : (v.e) : v.f", 30..31),
    ] {
        let compiled = at(source, 13);
        assert_eq!(
            compiled.failure(),
            Some(CompileFailure::Rejected),
            "{source}"
        );
        assert_eq!(compiled.diagnostics().len(), 1, "{source}");
        assert_eq!(compiled.diagnostics()[0].span(), span, "{source}");
        assert_eq!(
            &source[span.start as usize..span.end as usize],
            ":",
            "{source}"
        );
        assert_eq!(
            messages(&compiled),
            ["Error: could not find sub-expressions for Conditional Else ':' operator"],
            "{source}"
        );
    }
}

#[test]
fn own_diagnostics() {
    let blank = at("", 3);
    assert_eq!(blank.failure(), Some(CompileFailure::Rejected));
    assert_eq!(blank.diagnostics().len(), 1);
    assert_eq!(
        (
            blank.diagnostics()[0].code(),
            blank.diagnostics()[0].severity()
        ),
        (DiagCode::Syntax, Severity::Info)
    );
    assert!(blank.diagnostics()[0].language_message().is_none());

    let invalid = at("1", -1);
    assert_eq!(invalid.failure(), None);
    assert_eq!(
        (
            invalid.diagnostics()[0].code(),
            invalid.diagnostics()[0].severity()
        ),
        (DiagCode::InvalidVersion, Severity::Info)
    );

    let above = at("1", 14);
    assert_eq!(
        (
            above.diagnostics()[0].code(),
            above.diagnostics()[0].severity()
        ),
        (DiagCode::InvalidVersion, Severity::Warning)
    );
    let below = at("1", -7);
    assert_eq!(
        (
            below.diagnostics()[0].code(),
            below.diagnostics()[0].severity()
        ),
        (DiagCode::InvalidVersion, Severity::Warning)
    );

    let silent = CompileOptions {
        deviations: Deviations::NONE,
        ..server_at(14)
    };
    assert!(compile("1", &silent).diagnostics().is_empty());
}

#[test]
fn op_placeholders_use_friendly_names() {
    let cases: [(&str, &str); 5] = [
        ("1 +", "Error: binary Add '+' operator at end of expression"),
        (
            "v.x = v.y = 1;",
            "Error: assignment to non-variable not allowed. Expression is trying to assign to a: Assignment '='",
        ),
        (
            "math.abs",
            "Error: Absolute Value 'math.abs' operator at end of expression without a parenthesis section",
        ),
        (
            "loop()",
            "Error: Loop 'loop' operators with no params should not use parentheses",
        ),
        (
            "return;",
            "Error: unary Return 'return' operator not followed by expression",
        ),
    ];
    for (source, text) in cases {
        assert_eq!(messages_at(source, 13), [text], "{source:?}");
    }
}

#[test]
fn own_diagnostics_have_no_language_message_text() {
    let ours = |source: &str, opts: &CompileOptions, code: DiagCode| -> Vec<Severity> {
        let compiled = compile(source, opts);
        compiled
            .diagnostics()
            .iter()
            .filter(|d| d.code() == code)
            .map(|d| {
                assert!(
                    d.language_message().is_none(),
                    "{source:?}: {code:?} has a language message text"
                );
                d.severity()
            })
            .collect()
    };
    let latest = CompileOptions::server(MolangVersion::LATEST);
    assert_eq!(
        ours("q.log(7, 8)", &latest, DiagCode::QueryArity),
        [Severity::Warning]
    );
    assert_eq!(
        ours(
            "query.block_property('x')",
            &server_at(10),
            DiagCode::QueryDeprecated
        ),
        [Severity::Info]
    );
    assert_eq!(
        ours(
            "q.is_on_screen",
            &CompileOptions {
                catalog: molangx::stdlib::queries(Side::Server).clone(),
                ..latest.clone()
            },
            DiagCode::QueryClientOnly
        ),
        [Severity::Info]
    );
    assert_eq!(
        ours("1", &server_at(-1), DiagCode::InvalidVersion),
        [Severity::Info]
    );
    assert_eq!(
        ours("1", &server_at(14), DiagCode::InvalidVersion),
        [Severity::Warning]
    );
    let long = format!("{}1", " ".repeat(MAX_SOURCE_LEN));
    assert_eq!(
        ours(&long, &latest, DiagCode::SourceTooLong),
        [Severity::Error]
    );
}

/// Arithmetic on a `loop` / `for_each` result is rejected from version 3 (#36); on a statement list
/// it is accepted at every version.
#[test]
fn arithmetic_on_statements() {
    let looped = "v.count = 0; loop(3, {v.count = v.count + 1;}) + 1; return v.count;";
    let each =
        "v.count = 1; for_each(v.sheep, v.baa, {v.count = v.count + 1;}) + 1; return v.count;";
    for (source, kind) in [(looped, "Loop 'loop'"), (each, "For Each 'for_each'")] {
        assert_eq!(at(source, 2).failure(), None, "{source:?} at version 2");
        assert!(
            messages(&at(source, 2)).is_empty(),
            "{source:?} at version 2"
        );
        let compiled = at(source, 3);
        assert_eq!(
            compiled.failure(),
            Some(CompileFailure::Rejected),
            "{source:?} at version 3"
        );
        assert_eq!(
            messages(&compiled),
            [format!(
                "'Add '+'' expression cannot take a '{kind}' argument. It only supports numerical arguments."
            )],
            "{source:?} at version 3"
        );
    }
    for version in [2, 3, 13] {
        for source in ["v.y = {v.x = 1;} + 1;", "v.y = -{v.x = 1;};"] {
            let compiled = compile(
                source,
                &CompileOptions {
                    deviations: Deviations::NONE,
                    ..client_at(version)
                },
            );
            assert_eq!(compiled.failure(), None, "{source:?} at version {version}");
            assert!(
                messages(&compiled).is_empty(),
                "{source:?} at version {version}"
            );
        }
    }
}

#[cfg(feature = "vm")]
#[test]
fn log_once_across_evaluations() {
    use molangx::vm::{CollectSink, LogOnce, NoHostEnv};

    let compiled = compile(
        "return v.missing;",
        &CompileOptions::server(MolangVersion::LATEST),
    );
    let expr = compiled.expr().cloned().expect("compiles");
    let other = compile(
        "return v.other;",
        &CompileOptions::server(MolangVersion::LATEST),
    )
    .expr()
    .cloned()
    .expect("compiles");
    let mut env = NoHostEnv::new();
    let mut sink = LogOnce::new(CollectSink::new());
    for _ in 0..2 {
        let mut cx = env.cx();
        cx.sink = &mut sink;
        expr.eval(&mut cx);
    }
    assert_eq!(
        sink.inner().messages,
        ["Error: unhandled request for unknown variable 'variable.missing'"]
    );
    let mut cx = env.cx();
    cx.sink = &mut sink;
    other.eval(&mut cx);
    assert_eq!(sink.inner().messages.len(), 2);
}

#[test]
fn messages_refer_to_the_original_text() {
    let source = "V.X = Math.Min(1)";
    let compiled = at(source, 13);
    assert_eq!(
        messages(&compiled),
        ["Error: complex expressions (contains either '=' or ';') must end with a ';'"]
    );
    let span = &compiled.diagnostics()[0].span();
    assert!(
        span.start < span.end && span.end as usize <= source.len(),
        "{span:?}"
    );
    let source = "V.A + Math.Min(1)";
    let compiled = at(source, 13);
    assert_eq!(
        messages(&compiled),
        ["Unexpected number of parameters to Min 'math.min' function - expected 2, found 1."]
    );
    let span = &compiled.diagnostics()[0].span();
    assert!(
        source[span.start as usize..span.end as usize].contains("Math.Min"),
        "{span:?}"
    );
}

#[test]
fn diagnostic_fields() {
    let compiled = compile(
        "1 + q.no_such_query",
        &CompileOptions::server(MolangVersion::LATEST),
    );
    let first: &Diagnostic = &compiled.diagnostics()[0];
    let code: DiagCode = first.code();
    let severity: Severity = first.severity();
    let span: Range<u32> = first.span().clone();
    let message: Cow<'_, str> = first.message();
    assert_eq!((code, severity), (DiagCode::UnknownQuery, Severity::Error));
    assert_eq!(span.start, 4, "the span is in bytes of the original text");
    assert!(message.starts_with("Failed to resolve query query.no_such_query."));
    assert!(first.language_message().is_some());
    let levels = [Severity::Info, Severity::Warning, Severity::Error];
    assert!(levels.windows(2).all(|w| w[0] < w[1]));
}

#[test]
fn warnings_keep_their_severity() {
    let compiled = compile("1e;2e;", &CompileOptions::server(MolangVersion::LATEST));
    let bad: Vec<_> = compiled
        .diagnostics()
        .iter()
        .filter(|d| d.language_message() == Some(LanguageMessage::BadExponent))
        .collect();
    assert_eq!(bad.len(), 2);
    assert!(bad.iter().all(|d| d.severity() == Severity::Warning));
    assert_eq!(
        bad[0].message(),
        "error parsing float string, expected '+' or '-' after 'e': ;2e;"
    );
    assert_eq!(
        bad[1].message(),
        "error parsing float string, expected '+' or '-' after 'e': ;"
    );
    assert_eq!(bad[0].to_string(), bad[0].message());
}

/// `Deviations::ALL` keeps the first [`MAX_DIAGNOSTICS`] and a note counting the rest;
/// `Deviations::NONE` keeps all. `parses_cleanly` sees the suppressed messages either way.
#[test]
fn the_diagnostic_limit_keeps_the_first_and_counts_the_rest() {
    let src = "1e;".repeat(21_845);
    let ours = compile(&src, &CompileOptions::server(MolangVersion::LATEST));
    assert_eq!(ours.failure(), None);
    assert_eq!(ours.diagnostics().len(), MAX_DIAGNOSTICS + 1);
    assert!(
        ours.diagnostics()[..MAX_DIAGNOSTICS]
            .iter()
            .all(|d| d.language_message() == Some(LanguageMessage::BadExponent))
    );
    let note = &ours.diagnostics()[MAX_DIAGNOSTICS];
    assert_eq!(
        (note.code(), note.severity(), note.language_message()),
        (DiagCode::DiagnosticLimit, Severity::Info, None)
    );
    assert_eq!(
        note.message(),
        format!(
            "{} more diagnostics suppressed (at most 256 are kept per compile)",
            21_845 - MAX_DIAGNOSTICS
        )
    );
    assert!(!ours.parses_cleanly());

    let no_deviations = compile(
        &src,
        &CompileOptions {
            deviations: Deviations::NONE,
            ..CompileOptions::server(MolangVersion::LATEST)
        },
    );
    assert_eq!(no_deviations.diagnostics().len(), 21_845);
    assert_eq!(
        &no_deviations.diagnostics()[..MAX_DIAGNOSTICS],
        &ours.diagnostics()[..MAX_DIAGNOSTICS]
    );

    let at_limit = compile(
        &"1e;".repeat(MAX_DIAGNOSTICS),
        &CompileOptions::server(MolangVersion::LATEST),
    );
    assert_eq!(at_limit.diagnostics().len(), MAX_DIAGNOSTICS);
    assert!(
        at_limit
            .diagnostics()
            .iter()
            .all(|d| d.code() != DiagCode::DiagnosticLimit)
    );

    // Only the language message falls past the limit: compiled for `Side::Server`, each
    // `q.is_first_person` is an Info lint.
    let mut late = "q.is_first_person;".repeat(MAX_DIAGNOSTICS + 4);
    late.push_str("1e;");
    let late = compile(&late, &CompileOptions::server(MolangVersion::LATEST));
    assert_eq!(late.failure(), None);
    assert_eq!(late.diagnostics().len(), MAX_DIAGNOSTICS + 1);
    assert!(
        late.diagnostics()
            .iter()
            .all(|d| d.language_message().is_none())
    );
    assert!(!late.parses_cleanly());
}

/// Lints and warnings outside the parser-message table, each with its switch in `Deviations`.
mod own_lints {
    use crate::common::compile_support::{client_at, messages, server_at};
    use molangx::compile::{
        CompileFailure, CompileOptions, Compiled, Deviations, MAX_SOURCE_LEN, compile,
        compile_source,
    };
    use molangx::diag::{DiagCode, Severity};
    use molangx::json::MolangSource;
    use molangx::version::MolangVersion;

    fn codes(compiled: &Compiled) -> Vec<DiagCode> {
        compiled.diagnostics().iter().map(|d| d.code()).collect()
    }

    #[test]
    fn the_arity_lint_warns_about_a_call_outside_the_registered_counts() {
        let compiled = compile("q.ride_body_x_rotation(1)", &client_at(13));
        assert_eq!(compiled.failure(), None);
        assert_eq!(codes(&compiled), [DiagCode::QueryArity]);
        assert_eq!(compiled.diagnostics()[0].severity(), Severity::Warning);
        assert!(compiled.diagnostics()[0].language_message().is_none());
        assert_eq!(
            compiled.diagnostics()[0].message(),
            "query.ride_body_x_rotation is registered with 0 arguments, 1 given (this crate's check)"
        );
        assert!(
            compiled.parses_cleanly(),
            "our lints are not language messages"
        );
    }

    #[test]
    fn the_arity_lint_can_be_switched_off() {
        let off = CompileOptions {
            deviations: Deviations {
                query_arity_lint: false,
                ..Deviations::ALL
            },
            ..client_at(13)
        };
        assert!(
            compile("q.ride_body_x_rotation(1)", &off)
                .diagnostics()
                .is_empty()
        );
        assert!(
            compile(
                "q.ride_body_x_rotation(1)",
                &CompileOptions {
                    deviations: Deviations::NONE,
                    ..client_at(13)
                }
            )
            .diagnostics()
            .is_empty()
        );
    }

    #[test]
    fn the_arity_lint_is_quiet_within_the_counts_and_names_an_unbounded_maximum() {
        assert!(
            compile("q.ride_body_x_rotation", &client_at(13))
                .diagnostics()
                .is_empty()
        );
        assert!(
            compile("q.is_name_any('a','b')", &client_at(13))
                .diagnostics()
                .is_empty()
        );
        let compiled = compile("q.is_name_any", &client_at(13));
        assert!(
            messages(&compiled).is_empty(),
            "the lint is ours, not a language message"
        );
        assert_eq!(
            compiled
                .diagnostics()
                .iter()
                .map(|d| d.message().to_string())
                .collect::<Vec<_>>(),
            [
                "query.is_name_any is registered with at least 1 argument, 0 given (this crate's check)"
            ]
        );
    }

    #[test]
    fn a_source_one_byte_over_the_limit_is_rejected_without_a_parse() {
        let at_limit = format!("1{}", " ".repeat(MAX_SOURCE_LEN - 1));
        assert_eq!(at_limit.len(), MAX_SOURCE_LEN);
        let compiled = compile(&at_limit, &client_at(13));
        assert_eq!(compiled.failure(), None);
        assert!(compiled.diagnostics().is_empty());
        assert_eq!(
            compiled
                .expr()
                .cloned()
                .expect("an expression")
                .as_constant(),
            Some(1.0)
        );

        let src = format!("1{}", " ".repeat(MAX_SOURCE_LEN));
        let compiled = compile(&src, &client_at(13));
        assert_eq!(compiled.failure(), Some(CompileFailure::Rejected));
        assert_eq!(compiled.diagnostics().len(), 1);
        let diagnostic = &compiled.diagnostics()[0];
        assert_eq!(diagnostic.code(), DiagCode::SourceTooLong);
        assert_eq!(diagnostic.severity(), Severity::Error);
        assert_eq!(diagnostic.span(), 0..MAX_SOURCE_LEN as u32 + 1);
        assert_eq!(
            diagnostic.message(),
            "expression is 65537 bytes long; the limit is 65536"
        );
        assert!(diagnostic.language_message().is_none());
        assert!(!compiled.parsed());
        assert!(!compiled.parses_cleanly());
        assert_eq!(
            compiled
                .expr_or_zero()
                .cloned()
                .expect("the failed node")
                .as_constant(),
            Some(0.0)
        );
    }

    #[test]
    fn with_the_length_deviation_off_a_long_source_compiles() {
        let src = format!("1{}", " ".repeat(MAX_SOURCE_LEN));
        let off = CompileOptions {
            deviations: Deviations {
                source_length_limit: false,
                ..Deviations::ALL
            },
            ..client_at(13)
        };
        let compiled = compile(&src, &off);
        assert_eq!(compiled.failure(), None);
        assert!(compiled.diagnostics().is_empty());
        assert_eq!(
            compile(
                &src,
                &CompileOptions {
                    deviations: Deviations::NONE,
                    ..client_at(13)
                }
            )
            .failure(),
            None
        );
    }

    #[test]
    fn versions_inside_the_defined_range_log_nothing() {
        for raw in 0..=13 {
            let compiled = compile("1", &server_at(raw));
            assert!(
                compiled.diagnostics().is_empty(),
                "version {raw}: {:?}",
                messages(&compiled)
            );
        }
    }

    #[test]
    fn the_invalid_version_informs() {
        let compiled = compile("1", &server_at(-1));
        assert_eq!(codes(&compiled), [DiagCode::InvalidVersion]);
        assert_eq!(compiled.diagnostics()[0].severity(), Severity::Info);
        assert_eq!(
            compiled.diagnostics()[0].message(),
            "MolangVersion Invalid (-1): version-0 parser rules, no query resolves"
        );
        assert_eq!(compiled.diagnostics()[0].span(), 0..1);
        assert_eq!(compiled.failure(), None);
    }

    #[test]
    fn a_version_outside_the_range_warns_with_the_rules_it_gets() {
        for (raw, rules) in [(14, 13), (i16::MAX, 13), (-2, -1), (-300, -1)] {
            let compiled = compile("12", &server_at(raw));
            assert_eq!(codes(&compiled), [DiagCode::InvalidVersion], "raw {raw}");
            assert_eq!(compiled.diagnostics()[0].severity(), Severity::Warning);
            assert_eq!(
                compiled.diagnostics()[0].message(),
                format!(
                    "MolangVersion {raw} is outside -1..=13: parser rules as version {rules}, no query resolves"
                )
            );
            assert_eq!(compiled.diagnostics()[0].span(), 0..2);
            assert!(compiled.diagnostics()[0].language_message().is_none());
            assert_eq!(compiled.failure(), None);
        }
    }

    #[test]
    fn the_version_warning_can_be_switched_off() {
        let off = Deviations {
            object_version_warning: false,
            ..Deviations::ALL
        };
        for raw in [-2, -1, 14] {
            let compiled = compile(
                "1",
                &CompileOptions {
                    deviations: off,
                    ..server_at(raw)
                },
            );
            assert!(compiled.diagnostics().is_empty(), "raw {raw}");
        }
        let no_deviations = compile(
            "1",
            &CompileOptions {
                deviations: Deviations::NONE,
                ..server_at(14)
            },
        );
        assert!(no_deviations.diagnostics().is_empty());
    }

    #[test]
    fn an_unresolved_context_version_warns_whatever_the_deviations() {
        let unresolved = MolangSource::string_without_context("1");
        for deviations in [Deviations::ALL, Deviations::NONE] {
            let compiled = compile_source(
                &unresolved,
                &CompileOptions {
                    deviations,
                    ..CompileOptions::server(MolangVersion::LATEST)
                },
            );
            assert_eq!(codes(&compiled), [DiagCode::InvalidVersion]);
            assert_eq!(compiled.diagnostics()[0].severity(), Severity::Warning);
            assert!(compiled.diagnostics()[0].message().starts_with(
                "this string source has no MolangVersion: it was made without its load context"
            ));
            assert!(
                compiled.diagnostics()[0]
                    .message()
                    .ends_with("compiled as MolangVersion Invalid, no query resolves")
            );
            assert_eq!(compiled.failure(), None);
        }
    }
}
