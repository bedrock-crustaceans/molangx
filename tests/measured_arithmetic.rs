//! Arithmetic: `+ - * /`, number literals, parentheses, folded expressions at run time and the sign
//! of a folded −0.

#![cfg(all(feature = "compiler", feature = "stdlib"))]
// Expected values are written as the shortest decimal that gives their bits; some are close to a
// constant.
#![allow(clippy::approx_constant)]

mod common;

use common::measured::*;

/// Values of folded expressions at run time.
fn run_10() -> ServerRun {
    let mut run = ServerRun::new(
        "run_10",
        FIRST_RELEASE,
        "values of folded expressions at run time: cancelled and merged sum terms, a post-op under && and ||, a cancelled sum carrying a post-op as an operand",
    );
    run.checks_load_messages();
    run.arm64_differs(&[]);
    run.probe("o01", "v.x = 3; v.r = (v.x + 1) - v.x; v.r == 1 ? v.o01_is1 : (v.r == 0 ? v.o01_is0 : v.o01_other);")
        .answers("o01_is0");
    run.probe(
        "o02",
        "v.x = 3; v.y = 4; v.r = (v.x + v.y + 1) - v.x - v.y; v.r == 1 ? v.o02_is1 : (v.r == 0 ? v.o02_is0 : v.o02_other);",
    )
    .answers("o02_is0");
    run.probe("o03", "v.x = 3; v.r = (v.x + 1) + (v.x + 2); v.r == 9 ? v.o03_is9 : (v.r == 8 ? v.o03_is8 : (v.r == 7 ? v.o03_is7 : (v.r == 6 ? v.o03_is6 : (v.r == 5 ? v.o03_is5 : (v.r == 4 ? v.o03_is4 : v.o03_other)))));").answers("o03_is9");
    run.probe(
        "o04",
        "v.x = 2; v.r = (v.x == 1) + (v.x == 2); v.r == 1 ? v.o04_is1_distinct : (v.r == 0 ? v.o04_is0_merged_first : (v.r == 2 ? v.o04_is2_merged_second : v.o04_other));",
    )
    .answers("o04_is0_merged_first");
    run.probe(
        "o05",
        "v.x = 2; v.r = math.max(v.x, 1) + math.max(v.x, 5); v.r == 7 ? v.o05_is7_distinct : (v.r == 4 ? v.o05_is4_merged_first : (v.r == 10 ? v.o05_is10_merged_second : v.o05_other));",
    )
    .answers("o05_is4_merged_first");
    run.probe(
        "o06",
        "v.x = 2; v.r = math.pow(v.x, 2) + math.pow(v.x, 3); v.r == 12 ? v.o06_is12_distinct : (v.r == 8 ? v.o06_is8_merged_first : (v.r == 16 ? v.o06_is16_merged_second : v.o06_other));",
    )
    .answers("o06_is8_merged_first");
    run.probe(
        "o07",
        "v.x = 2; v.r = (v.x < 1) + (v.x < 3); v.r == 1 ? v.o07_is1_distinct : (v.r == 0 ? v.o07_is0_merged_first : (v.r == 2 ? v.o07_is2_merged_second : v.o07_other));",
    )
    .answers("o07_is0_merged_first");
    run.probe(
        "o08",
        "v.a = 1; v.b = 1; v.c = 1; v.r = v.a && ((v.b && v.c) - 1); v.r == 0 ? v.o08_is0_postop_kept : (v.r == 1 ? v.o08_is1_flattened : v.o08_other);",
    )
    .answers("o08_is1_flattened");
    run.probe(
        "o09",
        "v.z = 0; v.b = 1; v.c = 1; v.r = v.z || ((v.b || v.c) - 1); v.r == 0 ? v.o09_is0_postop_kept : (v.r == 1 ? v.o09_is1_flattened : v.o09_other);",
    )
    .answers("o09_is1_flattened");
    run.probe(
        "o10",
        "v.a = 1; v.b = 1; v.c = 1; v.r = v.a && (1 - (v.b && v.c)); v.r == 0 ? v.o10_is0_postop_kept : (v.r == 1 ? v.o10_is1_flattened : v.o10_other);",
    )
    .answers("o10_is1_flattened");
    run.probe(
        "o11",
        "v.a = 1; v.b = 1; v.c = 1; v.r = v.a && -((v.b && v.c) - 1); v.r == 0 ? v.o11_is0_postop_kept : (v.r == 1 ? v.o11_is1_flattened : v.o11_other);",
    )
    .answers("o11_is1_flattened");
    run.probe(
        "o12",
        "v.x = 2; v.y = 3; v.r = ((v.x + v.y + 1) + (-v.x - v.y)); v.r == 1 ? v.o12_is1 : (v.r == 0 ? v.o12_is0 : (v.r == 2 ? v.o12_is2 : v.o12_other));",
    )
    .answers("o12_is2");
    run.probe(
        "o13",
        "v.x = 2; v.y = 3; v.z = 5; v.r = v.z * ((v.x + v.y + 1) + (-v.x - v.y)); v.r == 5 ? v.o13_is5_x1 : (v.r == 0 ? v.o13_is0_x0 : (v.r == 10 ? v.o13_is10_x2 : v.o13_other));",
    )
    .answers("o13_is10_x2");
    run.probe(
        "o14",
        "v.x = 2; v.y = 3; v.z = 5; v.r = v.z + ((v.x + v.y + 1) + (-v.x - v.y)); v.r == 6 ? v.o14_is6_x1 : (v.r == 5 ? v.o14_is5_x0 : (v.r == 7 ? v.o14_is7_x2 : v.o14_other));",
    )
    .answers("o14_is7_x2");
    run.probe(
        "o15",
        "v.x = 2; v.y = 3; v.z = 5; v.r = math.max(v.z, ((v.x + v.y + 1) + (-v.x - v.y))); v.r == 5 ? v.o15_is5 : (v.r == 1 ? v.o15_is1 : (v.r == 2 ? v.o15_is2 : v.o15_other));",
    )
    .answers("o15_is5");
    run.probe(
        "o16",
        "v.x = 2; v.y = 3; v.w = 0.5; v.r = math.max(v.w, ((v.x + v.y + 1) + (-v.x - v.y))); v.r == 1 ? v.o16_is1_x1 : (v.r == 0.5 ? v.o16_is0p5_xle0p5 : (v.r == 2 ? v.o16_is2_x2 : v.o16_other));",
    )
    .answers("o16_is2_x2");
    run.probe("o99_marker_end", "v.o99_marker_end;")
        .answers("o99_marker_end");
    run
}

/// Cancelled variable terms take the constant with them, two like terms sum as twice the first, and
/// a post-op on an operand of `&&` / `||` is lost.
#[test]
fn folded_sums_keep_their_own_values_at_run_time_run_10() {
    run_10().replay(17);
}

/// Where a folded −0 survives. `math.atan2(v.m, -1)` tells the sign of a zero `v.m`: −180 for −0,
/// 180 for +0.
fn run_13() -> ServerRun {
    let mut run = ServerRun::new(
        "run_13",
        FIRST_RELEASE,
        "where a folded −0 survives: all-literal folds passed through a conditional branch instead of an assignment",
    );
    run.checks_load_messages();
    run.arm64_differs(&[]);
    run.probe(
        "g01",
        "v.c = 1; v.h = -0.5; v.m = v.c ? math.ceil(v.h) : 7; v.p1 = math.atan2(v.m, -1); v.m == 0 ? (v.p1 < 0 ? v.g01_negzero : (v.p1 > 0 ? v.g01_poszero : v.g01_other)) : v.g01_nonzero;",
    )
    .answers("g01_negzero");
    run.probe(
        "g02",
        "v.m = math.ceil(-0.5); v.p1 = math.atan2(v.m, -1); v.m == 0 ? (v.p1 < 0 ? v.g02_negzero : (v.p1 > 0 ? v.g02_poszero : v.g02_other)) : v.g02_nonzero;",
    )
    .answers("g02_poszero");
    run.probe(
        "g03",
        "v.c = 1; v.m = v.c ? math.ceil(-0.5) : 7; v.p1 = math.atan2(v.m, -1); v.m == 0 ? (v.p1 < 0 ? v.g03_negzero : (v.p1 > 0 ? v.g03_poszero : v.g03_other)) : v.g03_nonzero;",
    )
    .answers("g03_negzero");
    run.probe(
        "g04",
        "v.c = 1; v.m = v.c ? math.mod(-4, 2) : 7; v.p1 = math.atan2(v.m, -1); v.m == 0 ? (v.p1 < 0 ? v.g04_negzero : (v.p1 > 0 ? v.g04_poszero : v.g04_other)) : v.g04_nonzero;",
    )
    .answers("g04_negzero");
    run.probe(
        "g05",
        "v.c = 1; v.m = v.c ? math.mod(-3, 3) : 7; v.p1 = math.atan2(v.m, -1); v.m == 0 ? (v.p1 < 0 ? v.g05_negzero : (v.p1 > 0 ? v.g05_poszero : v.g05_other)) : v.g05_nonzero;",
    )
    .answers("g05_negzero");
    run.probe(
        "g06",
        "v.c = 1; v.m = v.c ? math.copy_sign(0, -1) : 7; v.p1 = math.atan2(v.m, -1); v.m == 0 ? (v.p1 < 0 ? v.g06_negzero : (v.p1 > 0 ? v.g06_poszero : v.g06_other)) : v.g06_nonzero;",
    )
    .answers("g06_negzero");
    run.probe(
        "g07",
        "v.c = 1; v.m = v.c ? (0 * -1) : 7; v.p1 = math.atan2(v.m, -1); v.m == 0 ? (v.p1 < 0 ? v.g07_negzero : (v.p1 > 0 ? v.g07_poszero : v.g07_other)) : v.g07_nonzero;",
    )
    .answers("g07_negzero");
    run.probe(
        "g08",
        "v.c = 1; v.m = v.c ? -0 : 7; v.p1 = math.atan2(v.m, -1); v.m == 0 ? (v.p1 < 0 ? v.g08_negzero : (v.p1 > 0 ? v.g08_poszero : v.g08_other)) : v.g08_nonzero;",
    )
    .answers("g08_negzero");
    run.probe(
        "g09",
        "v.p1 = math.atan2(math.mod(-4, 2), -1); v.p1 < -179 ? v.g09_atan2_of_fold_neg180 : (v.p1 > 179 ? v.g09_atan2_of_fold_pos180 : v.g09_other);",
    )
    .answers("g09_atan2_of_fold_neg180");
    run.probe(
        "g10",
        "v.x = 2; v.y = 3; v.c = 1; v.r = v.c ? ((v.x + v.y + 1) + (-v.x - v.y)) : 7; v.r == 2 ? v.g10_is2_postop_applied : (v.r == 1 ? v.g10_is1_value_only : (v.r == 0 ? v.g10_is0 : v.g10_other));",
    )
    .answers("g10_is1_value_only");
    run.probe("g99_marker_end", "v.g99_marker_end;")
        .answers("g99_marker_end");
    run
}

/// A folded −0 is +0 when assigned directly but stays −0 through a conditional branch; a cancelled
/// sum through a branch reads 1, not 2.
#[test]
fn a_folded_negative_zero_survives_a_conditional_branch_run_13() {
    run_13().replay(11);
}

#[test]
fn sums_and_differences_of_literals() {
    let mut case = EvalCase::new("evaluation-001");
    case.also_on_a_fresh_state();
    case.eval("0 + 0", 0.0);
    case.eval("0 + 1", 1.0);
    case.eval("1 + 0", 1.0);
    case.eval("1 + 1", 2.0);
    case.eval("-1 + -1", -2.0);
    case.eval("-1 + 1", 0.0);
    case.eval("1 + -1", 0.0);
    case.eval("1 + 1", 2.0);
    case.eval("0 - 0", 0.0);
    case.eval("0 - 1", -1.0);
    case.eval("1 - 0", 1.0);
    case.eval("1 - 1", 0.0);
    case.eval("-1 - -1", 0.0);
    case.eval("-1 - 1", -2.0);
    case.eval("1 - -1", 2.0);
    case.eval("1 - 1", 0.0);
    case.eval("0 + 0", 0.0);
    case.eval("0 + 2", 2.0);
    case.eval("2 + 0", 2.0);
    case.eval("2 + 2", 4.0);
    case.eval("-2 + -2", -4.0);
    case.eval("-2 + 2", 0.0);
    case.eval("2 + -2", 0.0);
    case.eval("2 + 2", 4.0);
    case.eval("0 - 0", 0.0);
    case.eval("0 - 2", -2.0);
    case.eval("2 - 0", 2.0);
    case.eval("2 - 2", 0.0);
    case.eval("-2 - -2", 0.0);
    case.eval("-2 - 2", -4.0);
    case.eval("2 - -2", 4.0);
    case.eval("2 - 2", 0.0);
    case.eval("0 + 0 + 3.14159265", 3.1415927);
    case.eval("0 + 1 + 3.14159265", 4.1415925);
    case.eval("1 + 0 + 3.14159265", 4.1415925);
    case.eval("1 + 1 + 3.14159265", 5.1415925);
    case.eval("-1 + -1 + 3.14159265", 1.1415926);
    case.eval("-1 + 1 + 3.14159265", 3.1415927);
    case.eval("1 + -1 + 3.14159265", 3.1415927);
    case.eval("1 + 1 + 3.14159265", 5.1415925);
    case.eval("0 - 0 + 3.14159265", 3.1415927);
    case.eval("0 - 1 + 3.14159265", 2.1415927);
    case.eval("1 - 0 + 3.14159265", 4.1415925);
    case.eval("1 - 1 + 3.14159265", 3.1415927);
    case.eval("-1 - -1 + 3.14159265", 3.1415927);
    case.eval("-1 - 1 + 3.14159265", 1.1415926);
    case.eval("1 - -1 + 3.14159265", 5.1415925);
    case.eval("1 - 1 + 3.14159265", 3.1415927);
    case.eval("0 + 0 + 3.14159265", 3.1415927);
    case.eval("0 + 2 + 3.14159265", 5.1415925);
    case.eval("2 + 0 + 3.14159265", 5.1415925);
    case.eval("2 + 2 + 3.14159265", 7.1415925);
    case.eval("-2 + -2 + 3.14159265", -0.8584074);
    case.eval("-2 + 2 + 3.14159265", 3.1415927);
    case.eval("2 + -2 + 3.14159265", 3.1415927);
    case.eval("2 + 2 + 3.14159265", 7.1415925);
    case.eval("0 - 0 + 3.14159265", 3.1415927);
    case.eval("0 - 2 + 3.14159265", 1.1415926);
    case.eval("2 - 0 + 3.14159265", 5.1415925);
    case.eval("2 - 2 + 3.14159265", 3.1415927);
    case.eval("-2 - -2 + 3.14159265", 3.1415927);
    case.eval("-2 - 2 + 3.14159265", -0.8584074);
    case.eval("2 - -2 + 3.14159265", 7.1415925);
    case.eval("2 - 2 + 3.14159265", 3.1415927);
    case.check(64);
}

#[test]
fn sums_products_and_quotients_of_variables() {
    let mut case = EvalCase::new("evaluation-002");
    case.also_on_a_fresh_state();
    case.eval("v.x = 1.23; v.y = 2.34; return v.x + v.y;", 3.57);
    case.eval("v.x = 1.23; v.y = 2.34; return v.x + v.x + 1;", 3.46);
    case.eval(
        "v.x = 1.23; v.y = 2.34; return v.x + v.y + math.pi;",
        6.7115927,
    );
    case.check(3);

    let mut case = EvalCase::new("evaluation-010");
    case.also_on_a_fresh_state();
    case.eval("v.x = 1.23; v.y = 2.34; return v.x / v.y;", 0.525641);
    case.eval(
        "v.x = 1.23; v.y = 2.34; return (v.x / v.y) / math.pi;",
        0.16731673,
    );
    case.eval(
        "v.x = 1.23; v.y = 2.34; return v.x / (v.y / math.pi);",
        1.6513501,
    );
    case.eval(
        "v.x = 1.23; v.y = 2.34; return v.x / v.y / math.pi;",
        0.16731673,
    );
    case.check(4);

    let mut case = EvalCase::new("evaluation-011");
    case.also_on_a_fresh_state();
    case.eval("v.x = 1.23; return v.x + 1;", 2.23);
    case.eval("v.x = 1.23; return v.x * 2;", 2.46);
    case.eval("v.x = 1.23; return v.x * 2 + 1;", 3.46);
    case.eval("v.x = 1.23; return v.x * 2 + 1;", 3.46);
    case.eval("v.x = 1.23; return v.x * -2 + 1;", -1.46);
    case.eval("v.x = 1.23; return v.x * -2 - 1;", -3.46);
    case.check(6);

    let mut case = EvalCase::new("evaluation-171");
    case.also_on_a_fresh_state();
    case.eval("v.x = 1.23; v.y = 2.34; return v.x * v.x;", 1.5129);
    case.eval("v.x = 1.23; v.y = 2.34; return v.x * v.x + 1;", 2.5128999);
    case.eval("v.x = 1.23; v.y = 2.34; return v.x * v.y;", 2.8782);
    case.eval("v.x = 1.23; v.y = 2.34; return v.x * v.y + 1;", 3.8782);
    case.check(4);
}

#[test]
fn quotients_of_literals() {
    let mut case = EvalCase::new("evaluation-009");
    case.also_on_a_fresh_state();
    case.eval("0 / 1", 0.0);
    case.eval("1 / 1", 1.0);
    case.eval("0 / -1", 0.0);
    case.eval("-1 / -1", 1.0);
    case.eval("0 / 2", 0.0);
    case.eval("2 / 2", 1.0);
    case.eval("0 / -2", 0.0);
    case.eval("-2 / -2", 1.0);
    case.eval("3.14159265 / 3.14159265", 1.0);
    case.eval("1 / 3.14159265", 0.31830987);
    case.eval("3.14159265 / 1", 3.1415927);
    case.eval("1 / 1", 1.0);
    case.eval("3.14159265 / 3.14159265", 1.0);
    case.eval("-1 / 3.14159265", -0.31830987);
    case.eval("3.14159265 / -1", -3.1415927);
    case.eval("-1 / -1", 1.0);
    case.eval("3.14159265 / 3.14159265", 1.0);
    case.eval("2 / 3.14159265", 0.63661975);
    case.eval("3.14159265 / 2", 1.5707964);
    case.eval("2 / 2", 1.0);
    case.eval("3.14159265 / 3.14159265", 1.0);
    case.eval("-2 / 3.14159265", -0.63661975);
    case.eval("3.14159265 / -2", -1.5707964);
    case.eval("-2 / -2", 1.0);
    case.eval("12345.0f / 67890.0f", 0.18183827);
    case.check(25);
}

#[test]
fn number_literals_in_every_written_form() {
    let mut case = EvalCase::new("evaluation-012");
    case.also_on_a_fresh_state();
    case.eval("0", 0.0);
    case.eval("0.", 0.0);
    case.eval("0.0", 0.0);
    case.eval("0.0f", 0.0);
    case.eval("1", 1.0);
    case.eval("1.", 1.0);
    case.eval("1.0", 1.0);
    case.eval("1.0f", 1.0);
    case.eval("-1", -1.0);
    case.eval("-1.", -1.0);
    case.eval("-1.0", -1.0);
    case.eval("-1.0f", -1.0);
    case.eval("2", 2.0);
    case.eval("2.", 2.0);
    case.eval("2.0", 2.0);
    case.eval("2.0f", 2.0);
    case.eval("-2", -2.0);
    case.eval("-2.", -2.0);
    case.eval("-2.0", -2.0);
    case.eval("-2.0f", -2.0);
    case.eval("-1e10", -1e10);
    case.eval("-1e10f", -1e10);
    case.eval("-1e9", -1000000000.0);
    case.eval("-1e9f", -1000000000.0);
    case.eval("-1e8", -100000000.0);
    case.eval("-1e8f", -100000000.0);
    case.eval("-1e7", -10000000.0);
    case.eval("-1e7f", -10000000.0);
    case.eval("-1e6", -1000000.0);
    case.eval("-1e6f", -1000000.0);
    case.eval("-1e5", -100000.0);
    case.eval("-1e5f", -100000.0);
    case.eval("-1e4", -10000.0);
    case.eval("-1e4f", -10000.0);
    case.eval("-1e3", -1000.0);
    case.eval("-1e3f", -1000.0);
    case.eval("-1e2", -100.0);
    case.eval("-1e2f", -100.0);
    case.eval("-1e1", -10.0);
    case.eval("-1e1f", -10.0);
    case.eval("1e10", 1e10);
    case.eval("1e10f", 1e10);
    case.eval("1e9", 1000000000.0);
    case.eval("1e9f", 1000000000.0);
    case.eval("1e8", 100000000.0);
    case.eval("1e8f", 100000000.0);
    case.eval("1e7", 10000000.0);
    case.eval("1e7f", 10000000.0);
    case.eval("1e6", 1000000.0);
    case.eval("1e6f", 1000000.0);
    case.eval("1e5", 100000.0);
    case.eval("1e5f", 100000.0);
    case.eval("1e4", 10000.0);
    case.eval("1e4f", 10000.0);
    case.eval("1e3", 1000.0);
    case.eval("1e3f", 1000.0);
    case.eval("1e2", 100.0);
    case.eval("1e2f", 100.0);
    case.eval("1e1", 10.0);
    case.eval("1e1f", 10.0);
    case.eval("10.0f", 10.0);
    case.eval("123.456f", 123.456);
    case.eval("123.4567890123456789f", 123.45679);
    case.eval("-123.4567890123456789f", -123.45679);
    case.check(64);
}

#[test]
fn products_of_literals() {
    let mut case = EvalCase::new("evaluation-051");
    case.also_on_a_fresh_state();
    case.eval("0 * 0", 0.0);
    case.eval("1 * 0", 0.0);
    case.eval("0 * 1", 0.0);
    case.eval("1 * 1", 1.0);
    case.eval("0 * 0", 0.0);
    case.eval("-1 * 0", 0.0);
    case.eval("0 * -1", 0.0);
    case.eval("-1 * -1", 1.0);
    case.eval("0 * 0", 0.0);
    case.eval("2 * 0", 0.0);
    case.eval("0 * 2", 0.0);
    case.eval("2 * 2", 4.0);
    case.eval("0 * 0", 0.0);
    case.eval("-2 * 0", 0.0);
    case.eval("0 * -2", 0.0);
    case.eval("-2 * -2", 4.0);
    case.eval("3.14159265 * 3.14159265", 9.869604);
    case.eval("1 * 3.14159265", 3.1415927);
    case.eval("3.14159265 * 1", 3.1415927);
    case.eval("1 * 1", 1.0);
    case.eval("3.14159265 * 3.14159265", 9.869604);
    case.eval("-1 * 3.14159265", -3.1415927);
    case.eval("3.14159265 * -1", -3.1415927);
    case.eval("-1 * -1", 1.0);
    case.eval("3.14159265 * 3.14159265", 9.869604);
    case.eval("2 * 3.14159265", 6.2831855);
    case.eval("3.14159265 * 2", 6.2831855);
    case.eval("2 * 2", 4.0);
    case.eval("3.14159265 * 3.14159265", 9.869604);
    case.eval("-2 * 3.14159265", -6.2831855);
    case.eval("3.14159265 * -2", -6.2831855);
    case.eval("-2 * -2", 4.0);
    case.check(32);
}

#[test]
fn parentheses_and_negated_groups() {
    let mut case = EvalCase::new("evaluation-052");
    case.also_on_a_fresh_state();
    case.eval("(0)", 0.0);
    case.eval("(0 + 1)", 1.0);
    case.eval("(0 + (1))", 1.0);
    case.eval("((0) + (1))", 1.0);
    case.eval("(((0)) + (1))", 1.0);
    case.eval("3 * (4 + 5)", 27.0);
    case.eval("(3 * (4 + 5))", 27.0);
    case.eval("((1 + 2) * (4 + 5))", 27.0);
    case.eval("(2)", 2.0);
    case.eval("1 + (3)", 4.0);
    case.eval("(1) + 4", 5.0);
    case.eval("(2 + 3) + 7", 12.0);
    case.eval("(2 * 3) + 7", 13.0);
    case.eval("(2 + 3) * 7", 35.0);
    case.eval("(2 * 3) * 7", 42.0);
    case.eval("(2 * 3) * 7", 42.0);
    case.eval("(2 + 3) + (5 + 7)", 17.0);
    case.eval("(2 + 3) + (5 * 7)", 40.0);
    case.eval("(2 * 3) + (5 + 7)", 18.0);
    case.eval("(2 * 3) + (5 * 7)", 41.0);
    case.eval("(2 + 3) * (5 + 7)", 60.0);
    case.eval("(2 + 3) * (5 * 7)", 175.0);
    case.eval("(2 * 3) * (5 + 7)", 72.0);
    case.eval("(2 * 3) * (5 * 7)", 210.0);
    case.eval("(2 * 3) * (5 + 7)", 72.0);
    case.eval("(2 * 3) * (5 * 7)", 210.0);
    case.eval("2 + (3 * 5) + 7", 24.0);
    case.eval("2 * (3 + 5) * 7", 112.0);
    case.eval("-(1)", -1.0);
    case.eval("(-1)", -1.0);
    case.eval("(0 - 1)", -1.0);
    case.eval("-(0 - (1))", 1.0);
    case.eval("-(0 - (-1))", -1.0);
    case.eval("-((0) + (1))", -1.0);
    case.eval("-((0) + -(1))", 1.0);
    case.eval("-((0) - (1))", 1.0);
    case.eval("-((0) + (-1))", 1.0);
    case.eval("-(((0)) + (1))", -1.0);
    case.eval("-3 * (4 + 5)", -27.0);
    case.eval("-3 * (-4 + 5)", -3.0);
    case.eval("-(3 * (4 + 5))", -27.0);
    case.eval("-(3 * (4 - 5))", 3.0);
    case.eval("-((1 + 2) * (4 - 5))", 3.0);
    case.eval("((1 + 2) * (4 - 5))", -3.0);
    case.eval("-((1 - 2) * (4 + 5))", 9.0);
    case.eval("((1 - 2) * (4 + 5))", -9.0);
    case.eval("-((1 - 2) * (4 - 5))", -1.0);
    case.eval("((1 - 2) * (4 - 5))", 1.0);
    case.eval("-1 + (3)", 2.0);
    case.eval("-(1) + 4", 3.0);
    case.eval("-(2 + 3) + 7", 2.0);
    case.eval("-(2 * 3) + 7", 1.0);
    case.eval("-(2 + 3) * 7", -35.0);
    case.eval("-(2 * 3) * 7", -42.0);
    case.eval("-(2 * 3) * 7", -42.0);
    case.eval("-(2 + 3) + (5 + 7)", 7.0);
    case.eval("-(2 + 3) + (5 * 7)", 30.0);
    case.eval("-(2 * 3) + (5 + 7)", 6.0);
    case.eval("-(2 * 3) + (5 * 7)", 29.0);
    case.eval("-(2 + 3) * (5 + 7)", -60.0);
    case.eval("-(2 + 3) * (5 * 7)", -175.0);
    case.eval("-(2 * 3) * (5 + 7)", -72.0);
    case.eval("-(2 * 3) * (5 * 7)", -210.0);
    case.eval("-(2 * 3) * (5 + 7)", -72.0);
    case.eval("-(2 * 3) * (5 * 7)", -210.0);
    case.eval("-2 + (3 * 5) + 7", 20.0);
    case.eval("-2 * (3 + 5) * 7", -112.0);
    case.check(67);
}

#[test]
fn chains_of_products_and_quotients_with_signs_and_groups() {
    let mut case = EvalCase::new("evaluation-053");
    case.also_on_a_fresh_state();
    case.eval("2.0 * 3.0 * 4.0 * 5.0", 120.0);
    case.eval("2.0 * 3.0 * 4.0 / 5.0", 4.8);
    case.eval("2.0 * 3.0 / 4.0 * 5.0", 7.5);
    case.eval("2.0 * 3.0 / 4.0 / 5.0", 0.3);
    case.eval("2.0 / 3.0 * 4.0 * 5.0", 13.333333);
    case.eval("2.0 / 3.0 * 4.0 / 5.0", 0.53333336);
    case.eval("2.0 / 3.0 / 4.0 * 5.0", 0.8333333);
    case.eval("2.0 / 3.0 / 4.0 / 5.0", 0.033333335);
    case.eval("2.0 * 3.0 * 4.0 * -5.0", -120.0);
    case.eval("2.0 * 3.0 * 4.0 / -5.0", -4.8);
    case.eval("2.0 * 3.0 / 4.0 * -5.0", -7.5);
    case.eval("2.0 * 3.0 / 4.0 / -5.0", -0.3);
    case.eval("2.0 / 3.0 * 4.0 * -5.0", -13.333333);
    case.eval("2.0 / 3.0 * 4.0 / -5.0", -0.53333336);
    case.eval("2.0 / 3.0 / 4.0 * -5.0", -0.8333333);
    case.eval("2.0 / 3.0 / 4.0 / -5.0", -0.033333335);
    case.eval("2.0 * 3.0 * -4.0 * 5.0", -120.0);
    case.eval("2.0 * 3.0 * -4.0 / 5.0", -4.8);
    case.eval("2.0 * 3.0 / -4.0 * 5.0", -7.5);
    case.eval("2.0 * 3.0 / -4.0 / 5.0", -0.3);
    case.eval("2.0 / 3.0 * -4.0 * 5.0", -13.333333);
    case.eval("2.0 / 3.0 * -4.0 / 5.0", -0.53333336);
    case.eval("2.0 / 3.0 / -4.0 * 5.0", -0.8333333);
    case.eval("2.0 / 3.0 / -4.0 / 5.0", -0.033333335);
    case.eval("2.0 * -3.0 * 4.0 * 5.0", -120.0);
    case.eval("2.0 * -3.0 * 4.0 / 5.0", -4.8);
    case.eval("2.0 * -3.0 / 4.0 * 5.0", -7.5);
    case.eval("2.0 * -3.0 / 4.0 / 5.0", -0.3);
    case.eval("2.0 / -3.0 * 4.0 * 5.0", -13.333333);
    case.eval("2.0 / -3.0 * 4.0 / 5.0", -0.53333336);
    case.eval("2.0 / -3.0 / 4.0 * 5.0", -0.8333333);
    case.eval("2.0 / -3.0 / 4.0 / 5.0", -0.033333335);
    case.eval("-2.0 * 3.0 * 4.0 * 5.0", -120.0);
    case.eval("-2.0 * 3.0 * 4.0 / 5.0", -4.8);
    case.eval("-2.0 * 3.0 / 4.0 * 5.0", -7.5);
    case.eval("-2.0 * 3.0 / 4.0 / 5.0", -0.3);
    case.eval("-2.0 / 3.0 * 4.0 * 5.0", -13.333333);
    case.eval("-2.0 / 3.0 * 4.0 / 5.0", -0.53333336);
    case.eval("-2.0 / 3.0 / 4.0 * 5.0", -0.8333333);
    case.eval("-2.0 / 3.0 / 4.0 / 5.0", -0.033333335);
    case.eval("2.0 * 3.0 * (4.0 * 5.0)", 120.0);
    case.eval("2.0 * 3.0 * (4.0 / 5.0)", 4.8);
    case.eval("2.0 * 3.0 / (4.0 * 5.0)", 0.3);
    case.eval("2.0 * 3.0 / (4.0 / 5.0)", 7.5);
    case.eval("2.0 / 3.0 * (4.0 * 5.0)", 13.333333);
    case.eval("2.0 / 3.0 * (4.0 / 5.0)", 0.53333336);
    case.eval("2.0 / 3.0 / (4.0 * 5.0)", 0.033333335);
    case.eval("2.0 / 3.0 / (4.0 / 5.0)", 0.8333333);
    case.eval("2.0 * 3.0 * (4.0 * -5.0)", -120.0);
    case.eval("2.0 * 3.0 * (4.0 / -5.0)", -4.8);
    case.eval("2.0 * 3.0 / (4.0 * -5.0)", -0.3);
    case.eval("2.0 * 3.0 / (4.0 / -5.0)", -7.5);
    case.eval("2.0 / 3.0 * (4.0 * -5.0)", -13.333333);
    case.eval("2.0 / 3.0 * (4.0 / -5.0)", -0.53333336);
    case.eval("2.0 / 3.0 / (4.0 * -5.0)", -0.033333335);
    case.eval("2.0 / 3.0 / (4.0 / -5.0)", -0.8333333);
    case.eval("2.0 * 3.0 * (-4.0 * 5.0)", -120.0);
    case.eval("2.0 * 3.0 * (-4.0 / 5.0)", -4.8);
    case.eval("2.0 * 3.0 / (-4.0 * 5.0)", -0.3);
    case.eval("2.0 * 3.0 / (-4.0 / 5.0)", -7.5);
    case.eval("2.0 / 3.0 * (-4.0 * 5.0)", -13.333333);
    case.eval("2.0 / 3.0 * (-4.0 / 5.0)", -0.53333336);
    case.eval("2.0 / 3.0 / (-4.0 * 5.0)", -0.033333335);
    case.eval("2.0 / 3.0 / (-4.0 / 5.0)", -0.8333333);
    case.eval("2.0 * -3.0 * (4.0 * 5.0)", -120.0);
    case.eval("2.0 * -3.0 * (4.0 / 5.0)", -4.8);
    case.eval("2.0 * -3.0 / (4.0 * 5.0)", -0.3);
    case.eval("2.0 * -3.0 / (4.0 / 5.0)", -7.5);
    case.eval("2.0 / -3.0 * (4.0 * 5.0)", -13.333333);
    case.eval("2.0 / -3.0 * (4.0 / 5.0)", -0.53333336);
    case.eval("2.0 / -3.0 / (4.0 * 5.0)", -0.033333335);
    case.eval("2.0 / -3.0 / (4.0 / 5.0)", -0.8333333);
    case.eval("-2.0 * 3.0 * (4.0 * 5.0)", -120.0);
    case.eval("-2.0 * 3.0 * (4.0 / 5.0)", -4.8);
    case.eval("-2.0 * 3.0 / (4.0 * 5.0)", -0.3);
    case.eval("-2.0 * 3.0 / (4.0 / 5.0)", -7.5);
    case.eval("-2.0 / 3.0 * (4.0 * 5.0)", -13.333333);
    case.eval("-2.0 / 3.0 * (4.0 / 5.0)", -0.53333336);
    case.eval("-2.0 / 3.0 / (4.0 * 5.0)", -0.033333335);
    case.eval("-2.0 / 3.0 / (4.0 / 5.0)", -0.8333333);
    case.eval("2.0 * 3.0 * -(4.0 * 5.0)", -120.0);
    case.eval("2.0 * 3.0 * -(4.0 / 5.0)", -4.8);
    case.eval("2.0 * 3.0 / -(4.0 * 5.0)", -0.3);
    case.eval("2.0 * 3.0 / -(4.0 / 5.0)", -7.5);
    case.eval("2.0 / 3.0 * -(4.0 * 5.0)", -13.333333);
    case.eval("2.0 / 3.0 * -(4.0 / 5.0)", -0.53333336);
    case.eval("2.0 / 3.0 / -(4.0 * 5.0)", -0.033333335);
    case.eval("2.0 / 3.0 / -(4.0 / 5.0)", -0.8333333);
    case.eval("2.0 * 3.0 * -(4.0 * -5.0)", 120.0);
    case.eval("2.0 * 3.0 * -(4.0 / -5.0)", 4.8);
    case.eval("2.0 * 3.0 / -(4.0 * -5.0)", 0.3);
    case.eval("2.0 * 3.0 / -(4.0 / -5.0)", 7.5);
    case.eval("2.0 / 3.0 * -(4.0 * -5.0)", 13.333333);
    case.eval("2.0 / 3.0 * -(4.0 / -5.0)", 0.53333336);
    case.eval("2.0 / 3.0 / -(4.0 * -5.0)", 0.033333335);
    case.eval("2.0 / 3.0 / -(4.0 / -5.0)", 0.8333333);
    case.eval("2.0 * 3.0 * -(-4.0 * 5.0)", 120.0);
    case.eval("2.0 * 3.0 * -(-4.0 / 5.0)", 4.8);
    case.eval("2.0 * 3.0 / -(-4.0 * 5.0)", 0.3);
    case.eval("2.0 * 3.0 / -(-4.0 / 5.0)", 7.5);
    case.eval("2.0 / 3.0 * -(-4.0 * 5.0)", 13.333333);
    case.eval("2.0 / 3.0 * -(-4.0 / 5.0)", 0.53333336);
    case.eval("2.0 / 3.0 / -(-4.0 * 5.0)", 0.033333335);
    case.eval("2.0 / 3.0 / -(-4.0 / 5.0)", 0.8333333);
    case.eval("2.0 * -3.0 * -(4.0 * 5.0)", 120.0);
    case.eval("2.0 * -3.0 * -(4.0 / 5.0)", 4.8);
    case.eval("2.0 * -3.0 / -(4.0 * 5.0)", 0.3);
    case.eval("2.0 * -3.0 / -(4.0 / 5.0)", 7.5);
    case.eval("2.0 / -3.0 * -(4.0 * 5.0)", 13.333333);
    case.eval("2.0 / -3.0 * -(4.0 / 5.0)", 0.53333336);
    case.eval("2.0 / -3.0 / -(4.0 * 5.0)", 0.033333335);
    case.eval("2.0 / -3.0 / -(4.0 / 5.0)", 0.8333333);
    case.eval("-2.0 * 3.0 * -(4.0 * 5.0)", 120.0);
    case.eval("-2.0 * 3.0 * -(4.0 / 5.0)", 4.8);
    case.eval("-2.0 * 3.0 / -(4.0 * 5.0)", 0.3);
    case.eval("-2.0 * 3.0 / -(4.0 / 5.0)", 7.5);
    case.eval("-2.0 / 3.0 * -(4.0 * 5.0)", 13.333333);
    case.eval("-2.0 / 3.0 * -(4.0 / 5.0)", 0.53333336);
    case.eval("-2.0 / 3.0 / -(4.0 * 5.0)", 0.033333335);
    case.eval("-2.0 / 3.0 / -(4.0 / 5.0)", 0.8333333);
    case.eval("2.0 * (3.0 * 4.0) * 5.0", 120.0);
    case.eval("2.0 * (3.0 * 4.0) / 5.0", 4.8);
    case.eval("2.0 * (3.0 / 4.0) * 5.0", 7.5);
    case.eval("2.0 * (3.0 / 4.0) / 5.0", 0.3);
    case.eval("2.0 / (3.0 * 4.0) * 5.0", 0.8333333);
    case.eval("2.0 / (3.0 * 4.0) / 5.0", 0.033333335);
    case.eval("2.0 / (3.0 / 4.0) * 5.0", 13.333333);
    case.eval("2.0 / (3.0 / 4.0) / 5.0", 0.53333336);
    case.eval("2.0 * (3.0 * 4.0) * -5.0", -120.0);
    case.eval("2.0 * (3.0 * 4.0) / -5.0", -4.8);
    case.eval("2.0 * (3.0 / 4.0) * -5.0", -7.5);
    case.eval("2.0 * (3.0 / 4.0) / -5.0", -0.3);
    case.eval("2.0 / (3.0 * 4.0) * -5.0", -0.8333333);
    case.eval("2.0 / (3.0 * 4.0) / -5.0", -0.033333335);
    case.eval("2.0 / (3.0 / 4.0) * -5.0", -13.333333);
    case.eval("2.0 / (3.0 / 4.0) / -5.0", -0.53333336);
    case.eval("2.0 * (3.0 * -4.0) * 5.0", -120.0);
    case.eval("2.0 * (3.0 * -4.0) / 5.0", -4.8);
    case.eval("2.0 * (3.0 / -4.0) * 5.0", -7.5);
    case.eval("2.0 * (3.0 / -4.0) / 5.0", -0.3);
    case.eval("2.0 / (3.0 * -4.0) * 5.0", -0.8333333);
    case.eval("2.0 / (3.0 * -4.0) / 5.0", -0.033333335);
    case.eval("2.0 / (3.0 / -4.0) * 5.0", -13.333333);
    case.eval("2.0 / (3.0 / -4.0) / 5.0", -0.53333336);
    case.eval("2.0 * (-3.0 * 4.0) * 5.0", -120.0);
    case.eval("2.0 * (-3.0 * 4.0) / 5.0", -4.8);
    case.eval("2.0 * (-3.0 / 4.0) * 5.0", -7.5);
    case.eval("2.0 * (-3.0 / 4.0) / 5.0", -0.3);
    case.eval("2.0 / (-3.0 * 4.0) * 5.0", -0.8333333);
    case.eval("2.0 / (-3.0 * 4.0) / 5.0", -0.033333335);
    case.eval("2.0 / (-3.0 / 4.0) * 5.0", -13.333333);
    case.eval("2.0 / (-3.0 / 4.0) / 5.0", -0.53333336);
    case.eval("-2.0 * (3.0 * 4.0) * 5.0", -120.0);
    case.eval("-2.0 * (3.0 * 4.0) / 5.0", -4.8);
    case.eval("-2.0 * (3.0 / 4.0) * 5.0", -7.5);
    case.eval("-2.0 * (3.0 / 4.0) / 5.0", -0.3);
    case.eval("-2.0 / (3.0 * 4.0) * 5.0", -0.8333333);
    case.eval("-2.0 / (3.0 * 4.0) / 5.0", -0.033333335);
    case.eval("-2.0 / (3.0 / 4.0) * 5.0", -13.333333);
    case.eval("-2.0 / (3.0 / 4.0) / 5.0", -0.53333336);
    case.eval("(2.0 * 3.0) * 4.0 * 5.0", 120.0);
    case.eval("(2.0 * 3.0) * 4.0 / 5.0", 4.8);
    case.eval("(2.0 * 3.0) / 4.0 * 5.0", 7.5);
    case.eval("(2.0 * 3.0) / 4.0 / 5.0", 0.3);
    case.eval("(2.0 / 3.0) * 4.0 * 5.0", 13.333333);
    case.eval("(2.0 / 3.0) * 4.0 / 5.0", 0.53333336);
    case.eval("(2.0 / 3.0) / 4.0 * 5.0", 0.8333333);
    case.eval("(2.0 / 3.0) / 4.0 / 5.0", 0.033333335);
    case.eval("(2.0 * 3.0) * 4.0 * -5.0", -120.0);
    case.eval("(2.0 * 3.0) * 4.0 / -5.0", -4.8);
    case.eval("(2.0 * 3.0) / 4.0 * -5.0", -7.5);
    case.eval("(2.0 * 3.0) / 4.0 / -5.0", -0.3);
    case.eval("(2.0 / 3.0) * 4.0 * -5.0", -13.333333);
    case.eval("(2.0 / 3.0) * 4.0 / -5.0", -0.53333336);
    case.eval("(2.0 / 3.0) / 4.0 * -5.0", -0.8333333);
    case.eval("(2.0 / 3.0) / 4.0 / -5.0", -0.033333335);
    case.eval("(2.0 * 3.0) * -4.0 * 5.0", -120.0);
    case.eval("(2.0 * 3.0) * -4.0 / 5.0", -4.8);
    case.eval("(2.0 * 3.0) / -4.0 * 5.0", -7.5);
    case.eval("(2.0 * 3.0) / -4.0 / 5.0", -0.3);
    case.eval("(2.0 / 3.0) * -4.0 * 5.0", -13.333333);
    case.eval("(2.0 / 3.0) * -4.0 / 5.0", -0.53333336);
    case.eval("(2.0 / 3.0) / -4.0 * 5.0", -0.8333333);
    case.eval("(2.0 / 3.0) / -4.0 / 5.0", -0.033333335);
    case.eval("(2.0 * -3.0) * 4.0 * 5.0", -120.0);
    case.eval("(2.0 * -3.0) * 4.0 / 5.0", -4.8);
    case.eval("(2.0 * -3.0) / 4.0 * 5.0", -7.5);
    case.eval("(2.0 * -3.0) / 4.0 / 5.0", -0.3);
    case.eval("(2.0 / -3.0) * 4.0 * 5.0", -13.333333);
    case.eval("(2.0 / -3.0) * 4.0 / 5.0", -0.53333336);
    case.eval("(2.0 / -3.0) / 4.0 * 5.0", -0.8333333);
    case.eval("(2.0 / -3.0) / 4.0 / 5.0", -0.033333335);
    case.eval("(-2.0 * 3.0) * 4.0 * 5.0", -120.0);
    case.eval("(-2.0 * 3.0) * 4.0 / 5.0", -4.8);
    case.eval("(-2.0 * 3.0) / 4.0 * 5.0", -7.5);
    case.eval("(-2.0 * 3.0) / 4.0 / 5.0", -0.3);
    case.eval("(-2.0 / 3.0) * 4.0 * 5.0", -13.333333);
    case.eval("(-2.0 / 3.0) * 4.0 / 5.0", -0.53333336);
    case.eval("(-2.0 / 3.0) / 4.0 * 5.0", -0.8333333);
    case.eval("(-2.0 / 3.0) / 4.0 / 5.0", -0.033333335);
    case.check(200);
}

#[test]
fn a_long_expression_over_variables_follows_precedence() {
    let mut case = EvalCase::new("evaluation-172");
    case.also_on_a_fresh_state();
    case.eval(
        "v.x = 1; v.y = 2; v.z = 3; return (((v.x * v.y * 3.0 + 1.0 * 2.0) * (v.x - v.x + 1)) * v.z + 4) + v.x - v.x + 2 * v.y * 2 + v.y - v.y * (v.z + v.y);",
        28.0,
    );
    case.eval(
        "v.x = 1; v.y = 2; v.z = 3; return (((v.x * v.y * 3.0 + 1.0 * 2.0) * (v.x - v.x + 1)) * v.z + 4) + v.x - v.x + 2 * v.y * 2 + v.y - v.y * -(v.z + v.y);",
        48.0,
    );
    case.check(2);
}

#[test]
fn nots_and_comparisons_are_numbers_in_arithmetic() {
    let mut case = EvalCase::new("evaluation-175");
    case.also_on_a_fresh_state();
    case.eval("return -!!0;", 0.0);
    case.eval("return -!!!0;", -1.0);
    case.eval("return -!!!0 + 1;", 0.0);
    case.eval("return !-!!!0;", 0.0);
    case.eval("return 1+!-!!!0;", 1.0);
    case.eval("return !1+!-!!!0;", 0.0);
    case.eval("(1 < 0) + 1", 1.0);
    case.eval("(1 <= 0) + 1", 1.0);
    case.eval("(1 > 0) + 1", 2.0);
    case.eval("(1 >= 0) + 1", 2.0);
    case.eval("(1 == 0) + 1", 1.0);
    case.eval("(1 != 0) + 1", 2.0);
    case.check(12);
}

/// Up to version 6 a division by a negative variable divides by its magnitude.
#[test]
fn division_by_a_negative_variable_keeps_the_divisor_sign_from_version_7() {
    let mut case = EvalCase::new("evaluation-184");
    case.eval("v.a = -1.0f; return 5/v.a;", 5.0).at(6);
    case.eval("v.a = -1.0f; return 5/v.a;", -5.0).at(7);
    case.eval("v.a = -10.0f; v.b = -2.0f; return v.a/v.b;", -5.0)
        .at(6);
    case.eval("v.a = -10.0f; v.b = -2.0f; return v.a/v.b;", 5.0)
        .at(7);
    case.eval("v.a = -1; return query.all(1, 1, 1 / v.a);", 1.0)
        .at(6);
    case.eval("v.a = -1; return query.all(-1, -1, 1 / v.a);", 1.0)
        .at(7);
    case.check(6);
}

#[test]
fn a_product_nested_126_deep_evaluates() {
    let mut case = EvalCase::new("evaluation-186");
    case.also_on_a_fresh_state();
    case.tolerance(0.0001);
    let expr = format!(
        "{}2.0{}",
        "12 * (0.08333333333333333 * ".repeat(126),
        ")".repeat(126)
    );
    assert_eq!(expr.len(), 3_657);
    assert!(
        expr.starts_with("12 * (0.08333333333333333 * 12 * (")
            && expr
                .trim_end_matches(')')
                .ends_with("12 * (0.08333333333333333 * 2.0")
    );
    assert_eq!(
        (expr.matches('(').count(), expr.matches(')').count()),
        (126, 126)
    );
    case.eval(&expr, 2.0);
    case.check(1);
}

/// Literal and variable forms of each of these expressions give the same bits.
#[test]
fn literal_and_variable_forms_give_the_same_bits() {
    let mut group = RunGroup::new("folding_and_angles");
    group
        .row(1, "return (variable.a ?? 2) + (variable.b ?? 3);", 5.0)
        .clears_variables();
    group.row(2, "math.min_angle(180)", -180.0);
    group.row(3, "v.x = 180; return math.min_angle(v.x);", -180.0);
    group.row(4, "math.min_angle(-180)", -180.0);
    group.row(5, "v.x = 540; return math.min_angle(v.x);", -180.0);
    group.row(6, "math.lerprotate(350, 10, 0.5)", 360.0);
    group.row(7, "v.a = 350; return math.lerprotate(v.a, 10, 0.5);", 360.0);
    group.row(8, "math.mod(1, 0)", f32::NAN);
    group.row(9, "v.x = 1; return math.mod(v.x, 0);", f32::NAN);
    group.row(10, "math.hermite_blend(0.5)", 0.5);
    group.row(11, "math.clamp(-2, -1, -3)", -3.0);
    group.row(12, "math.clamp(-3, -1, -2)", -1.0);
    group.row(13, "math.clamp(-3, -2, -1)", -2.0);
    group.row(14, "math.round(-0.5)", -1.0);
    group.row(15, "math.round(0.5)", 1.0);
    group.row(16, "math.ceil(-0.5)", -0.0);
    group.row(17, "v.x = 1; return v.x * 3 / 9;", 0.33333334);
    group.row(18, "7 * 3 / 9", 2.3333335);
    group.row(19, "v.a = 7; return v.a * 3 / 9;", 2.3333335);
    group.row(20, "2 * 3 / 4 * 5", 7.5);
    group.row(21, "v.a = 0.1; return v.a + 0.2;", 0.3);
    group.row(22, "0.1 + 0.2", 0.3);
    group.row(23, "v.x = 0; return v.x ? 111 : 222;", 222.0);
    group.row(24, "v.x = math.sqrt(-1); return v.x ?? 5;", f32::NAN);
    group.row(
        25,
        "v.dir ?? { v.dir.x = 0; v.dir.y = 1; }; return v.dir.y;",
        1.0,
    );
    group
        .row(26, "v.a = 1; v.b = 2; return v.a ?? v.b ?? 5;", 1.0)
        .logs(&["Error: found left-hand-side of ?? expression that isn't a direct-variable reference - this is unsupported at this time."]);
    group.row(27, "v.a = 7; return 1 ? (v.a = 7) : 0;", 7.0);
    group.row(28, "return v.a = 5;", 5.0);
    group.row(29, "v.x = 1; return -v.x * 2 + 1;", -1.0);
    group.check(29);
}

#[test]
fn a_sum_a_sine_in_degrees_and_a_division_by_a_negative_variable_give_their_values() {
    let mut group = SmokeGroup::new();
    group.row(1, "1+2", 3.0);
    group.row(2, "math.sin(90)*2", 2.0);
    group.row(3, "v.a = -1; return 5/v.a;", -5.0);
    group.row(4, "v.a = -1; return 5/v.a;", -5.0);
    group.check(4);
}
