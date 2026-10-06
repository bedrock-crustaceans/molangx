//! Parsing: parse outcomes and their messages, rejection at load, statement lists in sections,
//! constant detection and side effects.

#![cfg(all(feature = "compiler", feature = "stdlib"))]
// Expected values are written as the shortest decimal that gives their bits; some are close to a
// constant.
#![allow(clippy::approx_constant)]

mod common;

use common::measured::*;

/// Malformed and unresolvable `on_entry` entries between valid ones.
fn run_08() -> ServerRun {
    let mut run = ServerRun::new(
        "run_08",
        FIRST_RELEASE,
        "per-entry isolation of load-time rejections, with anchors of run_04 (r18, r01, r16, r17)",
    );
    run.checks_load_messages();
    run.arm64_differs(&[]);
    run.probe("h00_syntax", "v.p1 = (1 + ; v.h00_syntax_loaded;")
        .load_logs(&[
            "Unable to find matching closing section symbol for symbol at 3(Left Parenthesis '(') -- looking for Right Parenthesis ')'",
            "Error: Could not find Right Parenthesis ')' to close section started with Left Parenthesis '('",
        ])
        .rejected_at_load()
        .silent();
    run.probe(
        "r18",
        "v.p1 = math.sign(0); v.p1 == 1 ? v.r18_sign0_is1 : v.r18_sign0_other;",
    )
    .answers("r18_sign0_is1");
    run.probe(
        "r01",
        "v.n = math.sqrt(-1); v.f = 4; v.p1 = math.max(v.n, v.f); v.p1 == 4 ? v.r01_maxnanfirst_is4 : (v.p1 == v.p1 ? v.r01_maxnanfirst_other : v.r01_maxnanfirst_isnan);",
    )
    .answers("r01_maxnanfirst_is4");
    run.probe("h01_syntax_mid", "v.p1 = 1 +* 2; v.h01_syntax_mid_loaded;")
        .load_logs(&["Error: binary Add '+' operator at end of expression"])
        .rejected_at_load()
        .silent();
    run.probe("r16", "t.seen = 1; v.missing_never_set_r16; t.seen = 2;")
        .answers("missing_never_set_r16");
    run.probe("r17", "t.seen == 1 ? v.r17_abort_and_temp_persisted : (t.seen == 2 ? v.r17_no_abort : v.r17_other);")
        .answers("r17_abort_and_temp_persisted");
    run.probe("h02_unresolved_query", "v.p1 = q.is_on_screen; v.h02_is_on_screen_resolved;")
        .load_logs(&[
            "Failed to resolve query query.is_on_screen.  Either the query does not exist or it is not supported in this context.",
            "unrecognized token: q.is_on_screen; v.h02_is_on_screen_resolved;",
        ])
        .rejected_at_load()
        .silent();
    run.probe("h03_marker_end", "v.h03_marker_end;")
        .answers("h03_marker_end");
    run
}

/// An entry rejected at load gives no answer; the other entries load and run.
#[test]
fn a_rejection_at_load_affects_only_its_own_entry_run_08() {
    run_08().replay(8);
}

/// Statement lists in sections and bare query prefixes. Each `pNN` sets `t.st = 1` before the
/// statement under test and `t.st = 2` after it; its witness `wNN` reads `t.st` to tell how far it
/// ran.
fn run_11() -> ServerRun {
    let mut run = ServerRun::new(
        "run_11",
        FIRST_RELEASE,
        "statement lists in sections and bare query prefixes, each with a witness entry",
    );
    run.checks_load_messages();
    run.arm64_differs(&[]);
    run.probe("p00", "t.st = 0; v.p00_init;")
        .answers("p00_init");
    run.probe("p01", "t.st = 1; v.x = 0; v.y = 7; v.a = 0; v.y = [v.x = 1;]; t.st = 2; v.x == 1 ? (v.y == 7 ? v.p01_a1_b7 : (v.y == 0 ? v.p01_a1_b0 : (v.y == 1 ? v.p01_a1_b1 : (v.y == 2 ? v.p01_a1_b2 : v.p01_a1_bnone)))) : (v.x == 0 ? (v.y == 7 ? v.p01_a0_b7 : (v.y == 0 ? v.p01_a0_b0 : (v.y == 1 ? v.p01_a0_b1 : (v.y == 2 ? v.p01_a0_b2 : v.p01_a0_bnone)))) : v.p01_anone);").load_logs(&["Malformed Semicolon ';' expression. It has 0 children but should have between 1 and 18446744073709551615"]).rejected_at_load().silent();
    run.probe(
        "w01",
        "v.k = t.st; t.st = 0; v.k == 2 ? v.w01_completed : (v.k == 1 ? v.w01_stopped_inside : (v.k == 0 ? v.w01_not_run : v.w01_other));",
    )
    .answers("w01_not_run");
    run.probe("p02", "t.st = 1; v.x = 0; v.y = 7; v.a = 0; [v.x = 1;]; t.st = 2; v.x == 1 ? (v.y == 7 ? v.p02_a1_b7 : (v.y == 0 ? v.p02_a1_b0 : (v.y == 1 ? v.p02_a1_b1 : (v.y == 2 ? v.p02_a1_b2 : v.p02_a1_bnone)))) : (v.x == 0 ? (v.y == 7 ? v.p02_a0_b7 : (v.y == 0 ? v.p02_a0_b0 : (v.y == 1 ? v.p02_a0_b1 : (v.y == 2 ? v.p02_a0_b2 : v.p02_a0_bnone)))) : v.p02_anone);").load_logs(&["Malformed Semicolon ';' expression. It has 0 children but should have between 1 and 18446744073709551615"]).rejected_at_load().silent();
    run.probe(
        "w02",
        "v.k = t.st; t.st = 0; v.k == 2 ? v.w02_completed : (v.k == 1 ? v.w02_stopped_inside : (v.k == 0 ? v.w02_not_run : v.w02_other));",
    )
    .answers("w02_not_run");
    run.probe("p03", "t.st = 1; v.x = 0; v.y = 7; v.a = 0; (v.x = 1;); t.st = 2; v.x == 1 ? (v.y == 7 ? v.p03_a1_b7 : (v.y == 0 ? v.p03_a1_b0 : (v.y == 1 ? v.p03_a1_b1 : (v.y == 2 ? v.p03_a1_b2 : v.p03_a1_bnone)))) : (v.x == 0 ? (v.y == 7 ? v.p03_a0_b7 : (v.y == 0 ? v.p03_a0_b0 : (v.y == 1 ? v.p03_a0_b1 : (v.y == 2 ? v.p03_a0_b2 : v.p03_a0_bnone)))) : v.p03_anone);").answers("p03_a1_b7");
    run.probe(
        "w03",
        "v.k = t.st; t.st = 0; v.k == 2 ? v.w03_completed : (v.k == 1 ? v.w03_stopped_inside : (v.k == 0 ? v.w03_not_run : v.w03_other));",
    )
    .answers("w03_completed");
    run.probe("p04", "t.st = 1; v.x = 0; v.y = 7; v.a = 0; v.y = q.is_baby((v.a = 1;)); t.st = 2; v.a == 1 ? (v.y == 7 ? v.p04_a1_b7 : (v.y == 0 ? v.p04_a1_b0 : (v.y == 1 ? v.p04_a1_b1 : (v.y == 2 ? v.p04_a1_b2 : v.p04_a1_bnone)))) : (v.a == 0 ? (v.y == 7 ? v.p04_a0_b7 : (v.y == 0 ? v.p04_a0_b0 : (v.y == 1 ? v.p04_a0_b1 : (v.y == 2 ? v.p04_a0_b2 : v.p04_a0_bnone)))) : v.p04_anone);").load_logs(&["Malformed Semicolon ';' expression. It has 0 children but should have between 1 and 18446744073709551615"]).rejected_at_load().silent();
    run.probe(
        "w04",
        "v.k = t.st; t.st = 0; v.k == 2 ? v.w04_completed : (v.k == 1 ? v.w04_stopped_inside : (v.k == 0 ? v.w04_not_run : v.w04_other));",
    )
    .answers("w04_not_run");
    run.probe("p05", "t.st = 1; v.x = 0; v.y = 7; v.a = 0; v.y = {v.x = 1;} + 1; t.st = 2; v.x == 1 ? (v.y == 7 ? v.p05_a1_b7 : (v.y == 0 ? v.p05_a1_b0 : (v.y == 1 ? v.p05_a1_b1 : (v.y == 2 ? v.p05_a1_b2 : v.p05_a1_bnone)))) : (v.x == 0 ? (v.y == 7 ? v.p05_a0_b7 : (v.y == 0 ? v.p05_a0_b0 : (v.y == 1 ? v.p05_a0_b1 : (v.y == 2 ? v.p05_a0_b2 : v.p05_a0_bnone)))) : v.p05_anone);").answers("p05_a1_b1");
    run.probe(
        "w05",
        "v.k = t.st; t.st = 0; v.k == 2 ? v.w05_completed : (v.k == 1 ? v.w05_stopped_inside : (v.k == 0 ? v.w05_not_run : v.w05_other));",
    )
    .answers("w05_completed");
    run.probe("p06", "t.st = 1; v.x = 0; v.y = 7; v.a = 0; v.y = -{v.x = 1;}; t.st = 2; v.x == 1 ? (v.y == 7 ? v.p06_a1_b7 : (v.y == 0 ? v.p06_a1_b0 : (v.y == 1 ? v.p06_a1_b1 : (v.y == 2 ? v.p06_a1_b2 : v.p06_a1_bnone)))) : (v.x == 0 ? (v.y == 7 ? v.p06_a0_b7 : (v.y == 0 ? v.p06_a0_b0 : (v.y == 1 ? v.p06_a0_b1 : (v.y == 2 ? v.p06_a0_b2 : v.p06_a0_bnone)))) : v.p06_anone);").answers("p06_a1_b0");
    run.probe(
        "w06",
        "v.k = t.st; t.st = 0; v.k == 2 ? v.w06_completed : (v.k == 1 ? v.w06_stopped_inside : (v.k == 0 ? v.w06_not_run : v.w06_other));",
    )
    .answers("w06_completed");
    run.probe("p07", "t.st = 1; v.x = 0; v.y = 7; v.a = 0; v.x ? break : 1; t.st = 2; v.x == 1 ? (v.y == 7 ? v.p07_a1_b7 : (v.y == 0 ? v.p07_a1_b0 : (v.y == 1 ? v.p07_a1_b1 : (v.y == 2 ? v.p07_a1_b2 : v.p07_a1_bnone)))) : (v.x == 0 ? (v.y == 7 ? v.p07_a0_b7 : (v.y == 0 ? v.p07_a0_b0 : (v.y == 1 ? v.p07_a0_b1 : (v.y == 2 ? v.p07_a0_b2 : v.p07_a0_bnone)))) : v.p07_anone);").load_logs(&["Error: unreachable statements after Break 'break'."]).answers("p07_a0_b7");
    run.probe(
        "w07",
        "v.k = t.st; t.st = 0; v.k == 2 ? v.w07_completed : (v.k == 1 ? v.w07_stopped_inside : (v.k == 0 ? v.w07_not_run : v.w07_other));",
    )
    .answers("w07_completed");
    run.probe("p08", "t.st = 1; v.x = 1; v.y = 7; v.a = 0; v.x ? break : 1; t.st = 2; v.x == 1 ? (v.y == 7 ? v.p08_a1_b7 : (v.y == 0 ? v.p08_a1_b0 : (v.y == 1 ? v.p08_a1_b1 : (v.y == 2 ? v.p08_a1_b2 : v.p08_a1_bnone)))) : (v.x == 0 ? (v.y == 7 ? v.p08_a0_b7 : (v.y == 0 ? v.p08_a0_b0 : (v.y == 1 ? v.p08_a0_b1 : (v.y == 2 ? v.p08_a0_b2 : v.p08_a0_bnone)))) : v.p08_anone);").load_logs(&["Error: unreachable statements after Break 'break'."]).silent();
    run.probe(
        "w08",
        "v.k = t.st; t.st = 0; v.k == 2 ? v.w08_completed : (v.k == 1 ? v.w08_stopped_inside : (v.k == 0 ? v.w08_not_run : v.w08_other));",
    )
    .answers("w08_stopped_inside");
    run.probe("p09", "t.st = 1; v.x = 0; v.y = 7; v.a = 0; for_each(v.x, v.a, 1); t.st = 2; v.x == 1 ? (v.y == 7 ? v.p09_a1_b7 : (v.y == 0 ? v.p09_a1_b0 : (v.y == 1 ? v.p09_a1_b1 : (v.y == 2 ? v.p09_a1_b2 : v.p09_a1_bnone)))) : (v.x == 0 ? (v.y == 7 ? v.p09_a0_b7 : (v.y == 0 ? v.p09_a0_b0 : (v.y == 1 ? v.p09_a0_b1 : (v.y == 2 ? v.p09_a0_b2 : v.p09_a0_bnone)))) : v.p09_anone);").answers("p09_a0_b7");
    run.probe(
        "w09",
        "v.k = t.st; t.st = 0; v.k == 2 ? v.w09_completed : (v.k == 1 ? v.w09_stopped_inside : (v.k == 0 ? v.w09_not_run : v.w09_other));",
    )
    .answers("w09_stopped_inside");
    run.probe("p10", "t.st = 1; v.x = 0; v.y = 7; v.a = 0; loop(2, {v.x ? break : continue;}); t.st = 2; v.x == 1 ? (v.y == 7 ? v.p10_a1_b7 : (v.y == 0 ? v.p10_a1_b0 : (v.y == 1 ? v.p10_a1_b1 : (v.y == 2 ? v.p10_a1_b2 : v.p10_a1_bnone)))) : (v.x == 0 ? (v.y == 7 ? v.p10_a0_b7 : (v.y == 0 ? v.p10_a0_b0 : (v.y == 1 ? v.p10_a0_b1 : (v.y == 2 ? v.p10_a0_b2 : v.p10_a0_bnone)))) : v.p10_anone);").load_logs(&["Error: unreachable statements after Break 'break'."]).answers("p10_a0_b7");
    run.probe(
        "w10",
        "v.k = t.st; t.st = 0; v.k == 2 ? v.w10_completed : (v.k == 1 ? v.w10_stopped_inside : (v.k == 0 ? v.w10_not_run : v.w10_other));",
    )
    .answers("w10_completed");
    run.probe("p11", "t.st = 1; v.x = 1; v.y = 7; v.a = 0; loop(2, {v.x ? break : continue;}); t.st = 2; v.x == 1 ? (v.y == 7 ? v.p11_a1_b7 : (v.y == 0 ? v.p11_a1_b0 : (v.y == 1 ? v.p11_a1_b1 : (v.y == 2 ? v.p11_a1_b2 : v.p11_a1_bnone)))) : (v.x == 0 ? (v.y == 7 ? v.p11_a0_b7 : (v.y == 0 ? v.p11_a0_b0 : (v.y == 1 ? v.p11_a0_b1 : (v.y == 2 ? v.p11_a0_b2 : v.p11_a0_bnone)))) : v.p11_anone);").load_logs(&["Error: unreachable statements after Break 'break'."]).answers("p11_a1_b7");
    run.probe(
        "w11",
        "v.k = t.st; t.st = 0; v.k == 2 ? v.w11_completed : (v.k == 1 ? v.w11_stopped_inside : (v.k == 0 ? v.w11_not_run : v.w11_other));",
    )
    .answers("w11_completed");
    run.probe("p40", "v.p1 = q.; v.p40_loaded;")
        .load_logs(&["unrecognized token: q.; v.p40_loaded;"])
        .rejected_at_load()
        .silent();
    run.probe("p41", "v.p1 = query.; v.p41_loaded;")
        .load_logs(&["unrecognized token: query.; v.p41_loaded;"])
        .rejected_at_load()
        .silent();
    run.probe("p42", "v.p1 = q.1; v.p42_loaded;")
        .load_logs(&["unrecognized token: q.1; v.p42_loaded;"])
        .rejected_at_load()
        .silent();
    run.probe("p99_marker_end", "v.p99_marker_end;")
        .answers("p99_marker_end");
    run.probe("q01", "v.y = [v.x = 1;];")
        .load_logs(&["Malformed Semicolon ';' expression. It has 0 children but should have between 1 and 18446744073709551615"])
        .rejected_at_load()
        .silent();
    run.probe("q01_after", "v.q01_after;").answers("q01_after");
    run.probe("q02", "[v.x = 1;];")
        .load_logs(&["Malformed Semicolon ';' expression. It has 0 children but should have between 1 and 18446744073709551615"])
        .rejected_at_load()
        .silent();
    run.probe("q02_after", "v.q02_after;").answers("q02_after");
    run.probe("q03", "(v.x = 1;);").silent();
    run.probe("q03_after", "v.q03_after;").answers("q03_after");
    run.probe("q04", "v.y = q.is_baby((v.a = 1;));")
        .load_logs(&["Malformed Semicolon ';' expression. It has 0 children but should have between 1 and 18446744073709551615"])
        .rejected_at_load()
        .silent();
    run.probe("q04_after", "v.q04_after;").answers("q04_after");
    run.probe("q05", "v.y = {v.x = 1;} + 1;").silent();
    run.probe("q05_after", "v.q05_after;").answers("q05_after");
    run.probe("q06", "v.y = -{v.x = 1;};").silent();
    run.probe("q06_after", "v.q06_after;").answers("q06_after");
    run.probe("q07", "v.x ? break : 1;")
        .load_logs(&["Error: unreachable statements after Break 'break'."])
        .silent();
    run.probe("q07_after", "v.q07_after;").answers("q07_after");
    run.probe("q08", "for_each(v.x, v.a, 1);").silent();
    run.probe("q08_after", "v.q08_after;").answers("q08_after");
    run.probe("q09", "loop(2, {v.x ? break : continue;});")
        .load_logs(&["Error: unreachable statements after Break 'break'."])
        .silent();
    run.probe("q09_after", "v.q09_after;").answers("q09_after");
    run
}

/// A statement list in `[ ]`, or in `( )` as a query argument, is rejected at load; in `( )` as a
/// statement it runs; a `{ }` block is an operand worth 0; a `break` outside any loop ends the
/// expression when taken.
#[test]
fn statement_lists_in_sections_and_bare_query_prefixes_run_11() {
    let report = run_11().replay(45);
    assert_eq!(
        report.unbounded_substitutions, 6,
        "the six #24 lines of run 11 (p01, p02, p04, q01, q02, q04)"
    );
}

#[test]
fn constant_expressions_are_detected() {
    let mut case = EvalCase::new("constant_detection-001");
    case.is_constant(
        true,
        &["1", "3+4", "math.cos(15 + math.sqrt(100))", "'some text'"],
    );
    case.is_constant(
        false,
        &[
            "math.random(1, 2)",
            "variable.foo",
            "math.cos(15 + math.sqrt(variable.bar))",
            "query.position(0)",
        ],
    );
    case.check(8);
}

/// A random call counts as a side effect only when random calls are included.
#[test]
fn assignments_and_random_calls_have_side_effects() {
    let mut case = EvalCase::new("constant_detection-002");
    case.side_effects("variable.foo = 3;", true, true);
    case.side_effects("variable.foo = 4; return 3;", true, true);
    case.side_effects(
        "(3 > variable.foo) ? {variable.bar = 10; return 1;} : {return 0;};",
        true,
        true,
    );
    case.side_effects("1", true, false);
    case.side_effects("'some text'", true, false);
    case.side_effects("variable.foo", true, false);
    case.side_effects("math.cos(15 + math.sqrt(100))", true, false);
    case.side_effects("query.position(0)", true, false);
    case.check(8);

    let mut case = EvalCase::new("constant_detection-003");
    case.side_effects("math.random(1, 2)", true, true);
    case.side_effects("math.random(1, 2)", false, false);
    case.side_effects(
        "(3 > 2) ? { return math.random(1, 2); } : { return 0; };",
        true,
        true,
    );
    case.side_effects(
        "(3 > 2) ? { return math.random(1, 2); } : { return 0; };",
        false,
        false,
    );
    case.side_effects("2 + math.random(1, 2)", true, true);
    case.side_effects("2 + math.random(1, 2)", false, false);
    case.check(6);
}

#[test]
fn malformed_expressions_do_not_parse_cleanly() {
    let mut case = EvalCase::new("evaluation-179");
    case.parse_fails("v.x->temp.x = 0;").at(-1);
    case.parse_fails("v.x->v.x ?? 1;").at(-1);
    case.parse_fails("v.x->v.x ?? 1").at(-1);
    case.parse_fails("return 0; return 0;").at(-1);
    case.parse_fails("(v.x + 1) = 0;").at(-1);
    case.parse_fails("v.x = 0; loop(3, {continue; v.x = v.x + 1;});")
        .at(-1);
    case.parse_fails("0 ?? 1;").at(-1);
    case.parse_fails("0+").at(-1);
    case.parse_fails("1+").at(-1);
    case.parse_fails("1++").at(-1);
    case.parse_fails("1+ +").at(-1);
    case.parse_fails("+1").at(-1);
    case.parse_fails("*1").at(-1);
    case.parse_fails("*0").at(-1);
    case.parse_fails("0*").at(-1);
    case.parse_fails("1*").at(-1);
    case.parse_fails("v.x->v.y->v.z").at(-1);
    case.parse_fails("array.test[1 + 1] + 1").at(-1);
    case.parse_fails("query.get_name_test(0) + 1;")
        .at(-1)
        .because(ParseFailure::StringOperand);
    case.parse_fails("query.get_name_test(0) - 1;")
        .at(-1)
        .because(ParseFailure::StringOperand);
    case.parse_fails("query.get_name_test(0) * 1;")
        .at(-1)
        .because(ParseFailure::StringOperand);
    case.parse_fails("query.get_name_test(0) / 1;")
        .at(-1)
        .because(ParseFailure::StringOperand);
    case.parse_fails("9 10").at(-1);
    case.parse_fails("'foo' 'bar'").at(-1);
    case.parse_fails("this 2").at(-1);
    case.parse_fails("variable.foo 1").at(-1);
    case.parse_fails("").at(-1);
    case.parse_fails("'text without closing single quote")
        .at(-1);
    case.parse_fails("1 + (1 .)").at(-1);
    case.parse_fails("v.cowcow.friend = v.pigpig; v.pigpig->v.x = 1; v.pigpig->v.y = 2; return v.cowcow.friend->v.x + v.cowcow.friend->v.y;")
        .at(-1);
    case.parse_fails("v.cowcow.friend = v.pigpig; v.pigpig->v.test.a.b.c = 1.32; return v.cowcow.friend->v.test.a.b.c;")
        .at(-1);
    case.parse_fails("v.cowcow.friend = v.pigpig; v.pigpig->v.test.a.b.c = 2.32; v.moo = v.cowcow.friend->v.test; return v.moo.a.b.c;")
        .at(-1);
    case.parse_fails("v.cowcow.friend = v.pigpig; v.pigpig->v.test.a.b.c = 3.32; v.moo = v.cowcow.friend->v.test.a; return v.moo.b.c;")
        .at(-1);
    case.parse_fails("v.cowcow.friend = v.pigpig; v.pigpig->v.test.a.b.c = 4.32; v.moo = v.cowcow.friend->v.test.a.b; return v.moo.c;")
        .at(-1);
    case.parse_fails("v.cowcow.friend = v.pigpig; v.pigpig->v.test.a.b.c = 5.32; v.moo = v.cowcow.friend->v.test.a.b.c; return v.moo;")
        .at(-1);
    case.check(35);
}

#[test]
fn stray_tokens_do_not_parse() {
    let mut case = EvalCase::new("evaluation-182");
    case.all_parse(
        false,
        &[
            "1 , 8",
            "2 : 7",
            "3 { 6",
            "4 } 5",
            "5 [ 4",
            "6 ] 3",
            "7 ( 2",
            "8 ) 1",
            "break 11",
            ".foo",
            "variable.",
            ". 1",
            "1 ?",
            "? 1",
            "1 :",
            ": 1",
        ],
    );
    case.check(16);
}

#[test]
fn two_operands_without_an_operator_parse_at_version_3_and_fail_at_4() {
    let mut case = EvalCase::new("evaluation-183");
    case.parses("1 + (9 10)").at(3);
    case.parse_fails("1 + (9 10)").at(4);
    case.parses("[1 2]").at(3);
    case.parse_fails("[1 2]").at(4);
    case.parses("temp.v = ('foo' 'bar'); return temp.v;").at(3);
    case.parse_fails("temp.v = ('foo' 'bar'); return temp.v;")
        .at(4);
    case.parses("1 + (this 2)").at(3);
    case.parse_fails("1 + (this 2)").at(4);
    case.parses("1 + (variable.foo 3)").at(3);
    case.parse_fails("1 + (variable.foo 3)").at(4);
    case.check(10);
}

#[test]
fn math_min_angle_parses() {
    let mut case = EvalCase::new("evaluation-188");
    case.parses("math.min_angle(90.0)").at(-1);
    case.check(1);

    let mut case = EvalCase::new("evaluation-189");
    case.parses("v.x = 1.0; v.y = 2.0; math.min_angle(v.x * v.y);")
        .at(-1);
    case.check(1);
}

#[test]
fn geometry_material_and_texture_paths_parse() {
    let mut case = EvalCase::new("evaluation-201");
    case.parses("geometry.example.name").at(-1);
    case.check(1);

    let mut case = EvalCase::new("evaluation-202");
    case.parses("material.example.name").at(-1);
    case.check(1);

    let mut case = EvalCase::new("evaluation-203");
    case.parses("texture.example.name").at(-1);
    case.check(1);
}

/// `'a' + 3` is kept silently up to version 2 and rejected from 3; `0 ?? 1;` and an unreachable
/// statement after `continue` are logged and kept.
#[test]
fn parse_outcomes_by_version() {
    let mut group = ParseGroup::new("parse_outcomes");
    group.row(1, "query.get_name_test(0) + 1;").at(-1).rejected().logs(&[
        "Failed to resolve query query.get_name_test.  Either the query does not exist or it is not supported in this context.",
        "unrecognized token: query.get_name_test(0) + 1;",
    ]);
    group.row(2, "query.get_name_test(0) + 1;").at(0).rejected().logs(&[
        "Failed to resolve query query.get_name_test.  Either the query does not exist or it is not supported in this context.",
        "unrecognized token: query.get_name_test(0) + 1;",
    ]);
    group.row(3, "query.get_name_test(0) + 1;").at(2).rejected().logs(&[
        "Failed to resolve query query.get_name_test.  Either the query does not exist or it is not supported in this context.",
        "unrecognized token: query.get_name_test(0) + 1;",
    ]);
    group.row(4, "query.get_name_test(0) + 1;").at(3).rejected().logs(&[
        "Failed to resolve query query.get_name_test.  Either the query does not exist or it is not supported in this context.",
        "unrecognized token: query.get_name_test(0) + 1;",
    ]);
    group.row(5, "query.get_name_test(0) + 1;").rejected().logs(&[
        "Failed to resolve query query.get_name_test.  Either the query does not exist or it is not supported in this context.",
        "unrecognized token: query.get_name_test(0) + 1;",
    ]);
    group.row(6, "query.life_time + 1").at(-1).rejected().logs(&[
        "Failed to resolve query query.life_time.  Either the query does not exist or it is not supported in this context.",
        "unrecognized token: query.life_time + 1",
    ]);
    group.row(7, "query.life_time + 1").at(0).rejected().logs(&[
        "Failed to resolve query query.life_time.  Either the query does not exist or it is not supported in this context.",
        "unrecognized token: query.life_time + 1",
    ]);
    group.row(8, "query.life_time + 1").at(2).rejected().logs(&[
        "Failed to resolve query query.life_time.  Either the query does not exist or it is not supported in this context.",
        "unrecognized token: query.life_time + 1",
    ]);
    group.row(9, "query.life_time + 1").at(3).rejected().logs(&[
        "Failed to resolve query query.life_time.  Either the query does not exist or it is not supported in this context.",
        "unrecognized token: query.life_time + 1",
    ]);
    group.row(10, "query.life_time + 1").rejected().logs(&[
        "Failed to resolve query query.life_time.  Either the query does not exist or it is not supported in this context.",
        "unrecognized token: query.life_time + 1",
    ]);
    group.row(11, "'a' + 3").at(-1);
    group.row(12, "'a' + 3").at(0);
    group.row(13, "'a' + 3").at(2);
    group
        .row(14, "'a' + 3")
        .at(3)
        .rejected()
        .logs(&["'Add '+'' expression cannot take a 'String '''' argument. It only supports numerical arguments."]);
    group
        .row(15, "'a' + 3")
        .rejected()
        .logs(&["'Add '+'' expression cannot take a 'String '''' argument. It only supports numerical arguments."]);
    group
        .row(16, "1+")
        .at(-1)
        .rejected()
        .logs(&["Error: binary Add '+' operator at end of expression\n"]);
    group
        .row(17, "1+")
        .at(0)
        .rejected()
        .logs(&["Error: binary Add '+' operator at end of expression\n"]);
    group
        .row(18, "1+")
        .at(2)
        .rejected()
        .logs(&["Error: binary Add '+' operator at end of expression\n"]);
    group
        .row(19, "1+")
        .at(3)
        .rejected()
        .logs(&["Error: binary Add '+' operator at end of expression\n"]);
    group
        .row(20, "1+")
        .rejected()
        .logs(&["Error: binary Add '+' operator at end of expression\n"]);
    group
        .row(21, "9 10")
        .at(-1)
        .rejected()
        .logs(&["found multiple operations without a combining operation between them:\n"]);
    group
        .row(22, "9 10")
        .at(0)
        .rejected()
        .logs(&["found multiple operations without a combining operation between them:\n"]);
    group
        .row(23, "9 10")
        .at(2)
        .rejected()
        .logs(&["found multiple operations without a combining operation between them:\n"]);
    group
        .row(24, "9 10")
        .at(3)
        .rejected()
        .logs(&["found multiple operations without a combining operation between them:\n"]);
    group
        .row(25, "9 10")
        .rejected()
        .logs(&["found multiple operations without a combining operation between them:\n"]);
    group
        .row(26, "v.x->v.y->v.z")
        .at(-1)
        .rejected()
        .logs(&["Error: nested pointer statements (eg: A->B->C) are not yet supported.  Store A->B in a variable (eg: D), then use D->C"]);
    group
        .row(27, "v.x->v.y->v.z")
        .at(0)
        .rejected()
        .logs(&["Error: nested pointer statements (eg: A->B->C) are not yet supported.  Store A->B in a variable (eg: D), then use D->C"]);
    group
        .row(28, "v.x->v.y->v.z")
        .at(2)
        .rejected()
        .logs(&["Error: nested pointer statements (eg: A->B->C) are not yet supported.  Store A->B in a variable (eg: D), then use D->C"]);
    group
        .row(29, "v.x->v.y->v.z")
        .at(3)
        .rejected()
        .logs(&["Error: nested pointer statements (eg: A->B->C) are not yet supported.  Store A->B in a variable (eg: D), then use D->C"]);
    group
        .row(30, "v.x->v.y->v.z")
        .rejected()
        .logs(&["Error: nested pointer statements (eg: A->B->C) are not yet supported.  Store A->B in a variable (eg: D), then use D->C"]);
    group
        .row(31, "0 ?? 1;")
        .at(-1)
        .logs(&["Error: found left-hand-side of ?? expression that isn't a direct-variable reference - this is unsupported at this time."]);
    group
        .row(32, "0 ?? 1;")
        .at(0)
        .logs(&["Error: found left-hand-side of ?? expression that isn't a direct-variable reference - this is unsupported at this time."]);
    group
        .row(33, "0 ?? 1;")
        .at(2)
        .logs(&["Error: found left-hand-side of ?? expression that isn't a direct-variable reference - this is unsupported at this time."]);
    group
        .row(34, "0 ?? 1;")
        .at(3)
        .logs(&["Error: found left-hand-side of ?? expression that isn't a direct-variable reference - this is unsupported at this time."]);
    group
        .row(35, "0 ?? 1;")
        .logs(&["Error: found left-hand-side of ?? expression that isn't a direct-variable reference - this is unsupported at this time."]);
    group
        .row(36, "(v.x + 1) = 0;")
        .at(-1)
        .rejected()
        .logs(&["Error: assignment to non-variable not allowed. Expression is trying to assign to a: Left Parenthesis '('"]);
    group
        .row(37, "(v.x + 1) = 0;")
        .at(0)
        .rejected()
        .logs(&["Error: assignment to non-variable not allowed. Expression is trying to assign to a: Left Parenthesis '('"]);
    group
        .row(38, "(v.x + 1) = 0;")
        .at(2)
        .rejected()
        .logs(&["Error: assignment to non-variable not allowed. Expression is trying to assign to a: Left Parenthesis '('"]);
    group
        .row(39, "(v.x + 1) = 0;")
        .at(3)
        .rejected()
        .logs(&["Error: assignment to non-variable not allowed. Expression is trying to assign to a: Left Parenthesis '('"]);
    group
        .row(40, "(v.x + 1) = 0;")
        .rejected()
        .logs(&["Error: assignment to non-variable not allowed. Expression is trying to assign to a: Left Parenthesis '('"]);
    group
        .row(41, "return 0; return 0;")
        .at(-1)
        .rejected()
        .logs(&["Error: unreachable statements after Return 'return'."]);
    group
        .row(42, "return 0; return 0;")
        .at(0)
        .rejected()
        .logs(&["Error: unreachable statements after Return 'return'."]);
    group
        .row(43, "return 0; return 0;")
        .at(2)
        .rejected()
        .logs(&["Error: unreachable statements after Return 'return'."]);
    group
        .row(44, "return 0; return 0;")
        .at(3)
        .rejected()
        .logs(&["Error: unreachable statements after Return 'return'."]);
    group
        .row(45, "return 0; return 0;")
        .rejected()
        .logs(&["Error: unreachable statements after Return 'return'."]);
    group
        .row(46, "v.x = 0; loop(3, {continue; v.x = v.x + 1;});")
        .at(-1)
        .logs(&["Error: unreachable statements after Continue 'continue'."]);
    group
        .row(47, "v.x = 0; loop(3, {continue; v.x = v.x + 1;});")
        .at(0)
        .logs(&["Error: unreachable statements after Continue 'continue'."]);
    group
        .row(48, "v.x = 0; loop(3, {continue; v.x = v.x + 1;});")
        .at(2)
        .logs(&["Error: unreachable statements after Continue 'continue'."]);
    group
        .row(49, "v.x = 0; loop(3, {continue; v.x = v.x + 1;});")
        .at(3)
        .logs(&["Error: unreachable statements after Continue 'continue'."]);
    group
        .row(50, "v.x = 0; loop(3, {continue; v.x = v.x + 1;});")
        .logs(&["Error: unreachable statements after Continue 'continue'."]);
    group
        .row(51, "array.test[1 + 1] + 1")
        .at(-1)
        .rejected()
        .logs(&["Error: can't currently do math operations on resource array results"]);
    group
        .row(52, "array.test[1 + 1] + 1")
        .at(0)
        .rejected()
        .logs(&["Error: can't currently do math operations on resource array results"]);
    group
        .row(53, "array.test[1 + 1] + 1")
        .at(2)
        .rejected()
        .logs(&["Error: can't currently do math operations on resource array results"]);
    group
        .row(54, "array.test[1 + 1] + 1")
        .at(3)
        .rejected()
        .logs(&["Error: can't currently do math operations on resource array results"]);
    group
        .row(55, "array.test[1 + 1] + 1")
        .rejected()
        .logs(&["Error: can't currently do math operations on resource array results"]);
    group.check(55);
}

#[test]
fn parse_outcomes_of_operators() {
    let mut group = ParseGroup::new("parse_outcomes_operators");
    group
        .row(1, "v.x->temp.x = 0;")
        .at(-1)
        .rejected()
        .logs(&["Error: assignment to non-variable not allowed. Expression is trying to assign to a: Pointer '->'"]);
    group
        .row(2, "v.x->temp.x = 0;")
        .rejected()
        .logs(&["Error: assignment to non-variable not allowed. Expression is trying to assign to a: Pointer '->'"]);
    group
        .row(3, "v.x->v.x ?? 1;")
        .at(-1)
        .logs(&["Error: found left-hand-side of ?? expression that isn't a direct-variable reference - this is unsupported at this time."]);
    group
        .row(4, "v.x->v.x ?? 1;")
        .logs(&["Error: found left-hand-side of ?? expression that isn't a direct-variable reference - this is unsupported at this time."]);
    group
        .row(5, "v.x->v.x ?? 1")
        .at(-1)
        .rejected()
        .logs(&["Error: found left-hand-side of ?? expression that isn't a direct-variable reference - this is unsupported at this time."]);
    group
        .row(6, "v.x->v.x ?? 1")
        .rejected()
        .logs(&["Error: found left-hand-side of ?? expression that isn't a direct-variable reference - this is unsupported at this time."]);
    group
        .row(7, "return 0; return 0;")
        .at(-1)
        .rejected()
        .logs(&["Error: unreachable statements after Return 'return'."]);
    group
        .row(8, "return 0; return 0;")
        .rejected()
        .logs(&["Error: unreachable statements after Return 'return'."]);
    group
        .row(9, "(v.x + 1) = 0;")
        .at(-1)
        .rejected()
        .logs(&["Error: assignment to non-variable not allowed. Expression is trying to assign to a: Left Parenthesis '('"]);
    group
        .row(10, "(v.x + 1) = 0;")
        .rejected()
        .logs(&["Error: assignment to non-variable not allowed. Expression is trying to assign to a: Left Parenthesis '('"]);
    group
        .row(11, "v.x = 0; loop(3, {continue; v.x = v.x + 1;});")
        .at(-1)
        .logs(&["Error: unreachable statements after Continue 'continue'."]);
    group
        .row(12, "v.x = 0; loop(3, {continue; v.x = v.x + 1;});")
        .logs(&["Error: unreachable statements after Continue 'continue'."]);
    group
        .row(13, "0 ?? 1;")
        .at(-1)
        .logs(&["Error: found left-hand-side of ?? expression that isn't a direct-variable reference - this is unsupported at this time."]);
    group
        .row(14, "0 ?? 1;")
        .logs(&["Error: found left-hand-side of ?? expression that isn't a direct-variable reference - this is unsupported at this time."]);
    group
        .row(15, "0+")
        .at(-1)
        .rejected()
        .logs(&["Error: binary Add '+' operator at end of expression\n"]);
    group
        .row(16, "0+")
        .rejected()
        .logs(&["Error: binary Add '+' operator at end of expression\n"]);
    group
        .row(17, "1+")
        .at(-1)
        .rejected()
        .logs(&["Error: binary Add '+' operator at end of expression\n"]);
    group
        .row(18, "1+")
        .rejected()
        .logs(&["Error: binary Add '+' operator at end of expression\n"]);
    group.row(19, "1++").at(-1).rejected().logs(&[
        "Malformed Add '+' expression. It has 0 children but should have between 2 and -1",
    ]);
    group.row(20, "1++").rejected().logs(&[
        "Malformed Add '+' expression. It has 0 children but should have between 2 and -1",
    ]);
    group.row(21, "1+ +").at(-1).rejected().logs(&[
        "Malformed Add '+' expression. It has 0 children but should have between 2 and -1",
    ]);
    group.row(22, "1+ +").rejected().logs(&[
        "Malformed Add '+' expression. It has 0 children but should have between 2 and -1",
    ]);
    group
        .row(23, "+1")
        .at(-1)
        .rejected()
        .logs(&["Error: binary Add '+' operator at end of expression\n"]);
    group
        .row(24, "+1")
        .rejected()
        .logs(&["Error: binary Add '+' operator at end of expression\n"]);
    group
        .row(25, "*1")
        .at(-1)
        .rejected()
        .logs(&["Error: binary Multiply '*' operator at end of expression\n"]);
    group
        .row(26, "*1")
        .rejected()
        .logs(&["Error: binary Multiply '*' operator at end of expression\n"]);
    group
        .row(27, "*0")
        .at(-1)
        .rejected()
        .logs(&["Error: binary Multiply '*' operator at end of expression\n"]);
    group
        .row(28, "*0")
        .rejected()
        .logs(&["Error: binary Multiply '*' operator at end of expression\n"]);
    group
        .row(29, "0*")
        .at(-1)
        .rejected()
        .logs(&["Error: binary Multiply '*' operator at end of expression\n"]);
    group
        .row(30, "0*")
        .rejected()
        .logs(&["Error: binary Multiply '*' operator at end of expression\n"]);
    group
        .row(31, "1*")
        .at(-1)
        .rejected()
        .logs(&["Error: binary Multiply '*' operator at end of expression\n"]);
    group
        .row(32, "1*")
        .rejected()
        .logs(&["Error: binary Multiply '*' operator at end of expression\n"]);
    group
        .row(33, "v.x->v.y->v.z")
        .at(-1)
        .rejected()
        .logs(&["Error: nested pointer statements (eg: A->B->C) are not yet supported.  Store A->B in a variable (eg: D), then use D->C"]);
    group
        .row(34, "v.x->v.y->v.z")
        .rejected()
        .logs(&["Error: nested pointer statements (eg: A->B->C) are not yet supported.  Store A->B in a variable (eg: D), then use D->C"]);
    group
        .row(35, "array.test[1 + 1] + 1")
        .at(-1)
        .rejected()
        .logs(&["Error: can't currently do math operations on resource array results"]);
    group
        .row(36, "array.test[1 + 1] + 1")
        .rejected()
        .logs(&["Error: can't currently do math operations on resource array results"]);
    group.row(37, "query.get_name_test(0) + 1;").at(-1).rejected().logs(&[
        "Failed to resolve query query.get_name_test.  Either the query does not exist or it is not supported in this context.",
        "unrecognized token: query.get_name_test(0) + 1;",
    ]);
    group.row(38, "query.get_name_test(0) + 1;").rejected().logs(&[
        "Failed to resolve query query.get_name_test.  Either the query does not exist or it is not supported in this context.",
        "unrecognized token: query.get_name_test(0) + 1;",
    ]);
    group.row(39, "query.get_name_test(0) - 1;").at(-1).rejected().logs(&[
        "Failed to resolve query query.get_name_test.  Either the query does not exist or it is not supported in this context.",
        "unrecognized token: query.get_name_test(0) - 1;",
    ]);
    group.row(40, "query.get_name_test(0) - 1;").rejected().logs(&[
        "Failed to resolve query query.get_name_test.  Either the query does not exist or it is not supported in this context.",
        "unrecognized token: query.get_name_test(0) - 1;",
    ]);
    group.row(41, "query.get_name_test(0) * 1;").at(-1).rejected().logs(&[
        "Failed to resolve query query.get_name_test.  Either the query does not exist or it is not supported in this context.",
        "unrecognized token: query.get_name_test(0) * 1;",
    ]);
    group.row(42, "query.get_name_test(0) * 1;").rejected().logs(&[
        "Failed to resolve query query.get_name_test.  Either the query does not exist or it is not supported in this context.",
        "unrecognized token: query.get_name_test(0) * 1;",
    ]);
    group.row(43, "query.get_name_test(0) / 1;").at(-1).rejected().logs(&[
        "Failed to resolve query query.get_name_test.  Either the query does not exist or it is not supported in this context.",
        "unrecognized token: query.get_name_test(0) / 1;",
    ]);
    group.row(44, "query.get_name_test(0) / 1;").rejected().logs(&[
        "Failed to resolve query query.get_name_test.  Either the query does not exist or it is not supported in this context.",
        "unrecognized token: query.get_name_test(0) / 1;",
    ]);
    group
        .row(45, "9 10")
        .at(-1)
        .rejected()
        .logs(&["found multiple operations without a combining operation between them:\n"]);
    group
        .row(46, "9 10")
        .rejected()
        .logs(&["found multiple operations without a combining operation between them:\n"]);
    group
        .row(47, "'foo' 'bar'")
        .at(-1)
        .rejected()
        .logs(&["found multiple operations without a combining operation between them:\n"]);
    group
        .row(48, "'foo' 'bar'")
        .rejected()
        .logs(&["found multiple operations without a combining operation between them:\n"]);
    group
        .row(49, "this 2")
        .at(-1)
        .rejected()
        .logs(&["found multiple operations without a combining operation between them:\n"]);
    group
        .row(50, "this 2")
        .rejected()
        .logs(&["found multiple operations without a combining operation between them:\n"]);
    group
        .row(51, "variable.foo 1")
        .at(-1)
        .rejected()
        .logs(&["found multiple operations without a combining operation between them:\n"]);
    group
        .row(52, "variable.foo 1")
        .rejected()
        .logs(&["found multiple operations without a combining operation between them:\n"]);
    group
        .row(53, "'text without closing single quote")
        .at(-1)
        .rejected()
        .logs(&[
            "Error: Molang string missing final ' character",
            "unrecognized token: 'text without closing single quote",
        ]);
    group
        .row(54, "'text without closing single quote")
        .rejected()
        .logs(&[
            "Error: Molang string missing final ' character",
            "unrecognized token: 'text without closing single quote",
        ]);
    group
        .row(55, "1 + (1 .)")
        .at(-1)
        .rejected()
        .logs(&["unrecognized token: .)"]);
    group
        .row(56, "1 + (1 .)")
        .rejected()
        .logs(&["unrecognized token: .)"]);
    group
        .row(
            57,
            "v.cowcow.friend = v.pigpig; v.pigpig->v.x = 1; v.pigpig->v.y = 2; return v.cowcow.friend->v.x + v.cowcow.friend->v.y;",
        )
        .at(-1)
        .rejected()
        .logs(&[
            "Error: Assignment attempted on Pointer result. Writing to another entity's public variables is not supported, only reading them.",
            "Error: Assignment attempted on Pointer result. Writing to another entity's public variables is not supported, only reading them.",
            "Error: You cannot write to a variable on another mob.",
            "expression 'v.cowcow.friend = v.pigpig; v.pigpig->v.x = 1; v.pigpig->v.y = 2; return v.cowcow.friend->v.x + v.cowcow.friend->v.y;' compile failed",
        ]);
    group
        .row(
            58,
            "v.cowcow.friend = v.pigpig; v.pigpig->v.x = 1; v.pigpig->v.y = 2; return v.cowcow.friend->v.x + v.cowcow.friend->v.y;",
        )
        .rejected()
        .logs(&[
            "Error: Assignment attempted on Pointer result. Writing to another entity's public variables is not supported, only reading them.",
            "Error: Assignment attempted on Pointer result. Writing to another entity's public variables is not supported, only reading them.",
            "Error: You cannot write to a variable on another mob.",
            "expression 'v.cowcow.friend = v.pigpig; v.pigpig->v.x = 1; v.pigpig->v.y = 2; return v.cowcow.friend->v.x + v.cowcow.friend->v.y;' compile failed",
        ]);
    group
        .row(59, "v.cowcow.friend = v.pigpig; v.pigpig->v.test.a.b.c = 1.32; return v.cowcow.friend->v.test.a.b.c;")
        .at(-1)
        .rejected()
        .logs(&[
            "Error: right-hand-side of pointer expression did not evaluate to an entity variable or query function",
            "Error: Assignment attempted on Pointer result. Writing to another entity's public variables is not supported, only reading them.",
            "Error: right-hand-side of pointer expression did not evaluate to an entity variable or query function",
            "Error: You cannot write to a variable on another mob.",
            "expression 'v.cowcow.friend = v.pigpig; v.pigpig->v.test.a.b.c = 1.32; return v.cowcow.friend->v.test.a.b.c;' compile failed",
        ]);
    group
        .row(60, "v.cowcow.friend = v.pigpig; v.pigpig->v.test.a.b.c = 1.32; return v.cowcow.friend->v.test.a.b.c;")
        .rejected()
        .logs(&[
            "Error: right-hand-side of pointer expression did not evaluate to an entity variable or query function",
            "Error: Assignment attempted on Pointer result. Writing to another entity's public variables is not supported, only reading them.",
            "Error: right-hand-side of pointer expression did not evaluate to an entity variable or query function",
            "Error: You cannot write to a variable on another mob.",
            "expression 'v.cowcow.friend = v.pigpig; v.pigpig->v.test.a.b.c = 1.32; return v.cowcow.friend->v.test.a.b.c;' compile failed",
        ]);
    group
        .row(61, "v.cowcow.friend = v.pigpig; v.pigpig->v.test.a.b.c = 2.32; v.moo = v.cowcow.friend->v.test; return v.moo.a.b.c;")
        .at(-1)
        .rejected()
        .logs(&[
            "Error: right-hand-side of pointer expression did not evaluate to an entity variable or query function",
            "Error: Assignment attempted on Pointer result. Writing to another entity's public variables is not supported, only reading them.",
            "Error: You cannot write to a variable on another mob.",
            "expression 'v.cowcow.friend = v.pigpig; v.pigpig->v.test.a.b.c = 2.32; v.moo = v.cowcow.friend->v.test; return v.moo.a.b.c;' compile failed",
        ]);
    group
        .row(62, "v.cowcow.friend = v.pigpig; v.pigpig->v.test.a.b.c = 2.32; v.moo = v.cowcow.friend->v.test; return v.moo.a.b.c;")
        .rejected()
        .logs(&[
            "Error: right-hand-side of pointer expression did not evaluate to an entity variable or query function",
            "Error: Assignment attempted on Pointer result. Writing to another entity's public variables is not supported, only reading them.",
            "Error: You cannot write to a variable on another mob.",
            "expression 'v.cowcow.friend = v.pigpig; v.pigpig->v.test.a.b.c = 2.32; v.moo = v.cowcow.friend->v.test; return v.moo.a.b.c;' compile failed",
        ]);
    group
        .row(63, "v.cowcow.friend = v.pigpig; v.pigpig->v.test.a.b.c = 3.32; v.moo = v.cowcow.friend->v.test.a; return v.moo.b.c;")
        .at(-1)
        .rejected()
        .logs(&[
            "Error: right-hand-side of pointer expression did not evaluate to an entity variable or query function",
            "Error: Assignment attempted on Pointer result. Writing to another entity's public variables is not supported, only reading them.",
            "Error: right-hand-side of pointer expression did not evaluate to an entity variable or query function",
            "Error: You cannot write to a variable on another mob.",
            "expression 'v.cowcow.friend = v.pigpig; v.pigpig->v.test.a.b.c = 3.32; v.moo = v.cowcow.friend->v.test.a; return v.moo.b.c;' compile failed",
        ]);
    group
        .row(64, "v.cowcow.friend = v.pigpig; v.pigpig->v.test.a.b.c = 3.32; v.moo = v.cowcow.friend->v.test.a; return v.moo.b.c;")
        .rejected()
        .logs(&[
            "Error: right-hand-side of pointer expression did not evaluate to an entity variable or query function",
            "Error: Assignment attempted on Pointer result. Writing to another entity's public variables is not supported, only reading them.",
            "Error: right-hand-side of pointer expression did not evaluate to an entity variable or query function",
            "Error: You cannot write to a variable on another mob.",
            "expression 'v.cowcow.friend = v.pigpig; v.pigpig->v.test.a.b.c = 3.32; v.moo = v.cowcow.friend->v.test.a; return v.moo.b.c;' compile failed",
        ]);
    group
        .row(65, "v.cowcow.friend = v.pigpig; v.pigpig->v.test.a.b.c = 4.32; v.moo = v.cowcow.friend->v.test.a.b; return v.moo.c;")
        .at(-1)
        .rejected()
        .logs(&[
            "Error: right-hand-side of pointer expression did not evaluate to an entity variable or query function",
            "Error: Assignment attempted on Pointer result. Writing to another entity's public variables is not supported, only reading them.",
            "Error: right-hand-side of pointer expression did not evaluate to an entity variable or query function",
            "Error: You cannot write to a variable on another mob.",
            "expression 'v.cowcow.friend = v.pigpig; v.pigpig->v.test.a.b.c = 4.32; v.moo = v.cowcow.friend->v.test.a.b; return v.moo.c;' compile failed",
        ]);
    group
        .row(66, "v.cowcow.friend = v.pigpig; v.pigpig->v.test.a.b.c = 4.32; v.moo = v.cowcow.friend->v.test.a.b; return v.moo.c;")
        .rejected()
        .logs(&[
            "Error: right-hand-side of pointer expression did not evaluate to an entity variable or query function",
            "Error: Assignment attempted on Pointer result. Writing to another entity's public variables is not supported, only reading them.",
            "Error: right-hand-side of pointer expression did not evaluate to an entity variable or query function",
            "Error: You cannot write to a variable on another mob.",
            "expression 'v.cowcow.friend = v.pigpig; v.pigpig->v.test.a.b.c = 4.32; v.moo = v.cowcow.friend->v.test.a.b; return v.moo.c;' compile failed",
        ]);
    group
        .row(67, "v.cowcow.friend = v.pigpig; v.pigpig->v.test.a.b.c = 5.32; v.moo = v.cowcow.friend->v.test.a.b.c; return v.moo;")
        .at(-1)
        .rejected()
        .logs(&[
            "Error: right-hand-side of pointer expression did not evaluate to an entity variable or query function",
            "Error: Assignment attempted on Pointer result. Writing to another entity's public variables is not supported, only reading them.",
            "Error: right-hand-side of pointer expression did not evaluate to an entity variable or query function",
            "Error: You cannot write to a variable on another mob.",
            "expression 'v.cowcow.friend = v.pigpig; v.pigpig->v.test.a.b.c = 5.32; v.moo = v.cowcow.friend->v.test.a.b.c; return v.moo;' compile failed",
        ]);
    group
        .row(68, "v.cowcow.friend = v.pigpig; v.pigpig->v.test.a.b.c = 5.32; v.moo = v.cowcow.friend->v.test.a.b.c; return v.moo;")
        .rejected()
        .logs(&[
            "Error: right-hand-side of pointer expression did not evaluate to an entity variable or query function",
            "Error: Assignment attempted on Pointer result. Writing to another entity's public variables is not supported, only reading them.",
            "Error: right-hand-side of pointer expression did not evaluate to an entity variable or query function",
            "Error: You cannot write to a variable on another mob.",
            "expression 'v.cowcow.friend = v.pigpig; v.pigpig->v.test.a.b.c = 5.32; v.moo = v.cowcow.friend->v.test.a.b.c; return v.moo;' compile failed",
        ]);
    group.row(69, "query.experimental_test").at(-1).rejected().logs(&[
        "Failed to resolve query query.experimental_test.  Either the query does not exist or it is not supported in this context.",
        "unrecognized token: query.experimental_test",
    ]);
    group.row(70, "query.experimental_test").rejected().logs(&[
        "Failed to resolve query query.experimental_test.  Either the query does not exist or it is not supported in this context.",
        "unrecognized token: query.experimental_test",
    ]);
    group.check(70);
}
