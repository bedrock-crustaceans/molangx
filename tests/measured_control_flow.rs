//! Control flow: `loop`, `for_each`, `break` / `continue` / `return`, statement lists and
//! controller transitions.

#![cfg(all(feature = "compiler", feature = "stdlib"))]
// Expected values are written as the shortest decimal that gives their bits; some are close to a
// constant.
#![allow(clippy::approx_constant)]

mod common;

use common::measured::*;

/// The statements after a `for_each` over the number 0, with a `loop` as control.
fn run_14() -> ServerRun {
    let mut run = ServerRun::new(
        "run_14",
        FIRST_RELEASE,
        "for_each and temp variables: follow-up of run_11 p09/w09",
    );
    run.checks_load_messages();
    run.arm64_differs(&[]);
    run.probe(
        "k01",
        "t.st = 1; v.x = 0; v.a = 0; for_each(v.x, v.a, 1); t.st = 2; t.st == 2 ? v.k01_sees2 : (t.st == 1 ? v.k01_sees1 : v.k01_other);",
    )
    .answers("k01_sees1");
    run.probe(
        "k02",
        "v.k = t.st; t.st = 0; v.k == 2 ? v.k02_next_sees2 : (v.k == 1 ? v.k02_next_sees1 : (v.k == 0 ? v.k02_next_sees0 : v.k02_other));",
    )
    .answers("k02_next_sees1");
    run.probe(
        "k03",
        "v.after = 0; v.x = 0; v.a = 0; for_each(v.x, v.a, 1); v.after = 5; v.k03_done;",
    )
    .answers("k03_done");
    run.probe(
        "k04",
        "v.after == 5 ? v.k04_after_is5 : (v.after == 0 ? v.k04_after_is0 : v.k04_other);",
    )
    .answers("k04_after_is0");
    run.probe(
        "k05",
        "t.b = 3; v.x = 0; v.a = 0; for_each(v.x, v.a, 1); t.b == 3 ? v.k05_sees3 : v.k05_lost;",
    )
    .answers("k05_lost");
    run.probe(
        "k06",
        "t.st = 1; loop(1, {v.q = 1;}); t.st = 2; v.k = 0; v.k06_done;",
    )
    .answers("k06_done");
    run.probe("k07", "v.k = t.st; t.st = 0; v.k == 2 ? v.k07_next_sees2 : (v.k == 1 ? v.k07_next_sees1 : v.k07_other);")
        .answers("k07_next_sees2");
    run.probe("k99_marker_end", "v.k99_marker_end;")
        .answers("k99_marker_end");
    run
}

/// After a `for_each` over 0 the next write is lost and a temp set before it no longer reads back.
#[test]
fn the_write_after_a_for_each_over_zero_is_lost_run_14() {
    run_14().replay(8);
}

/// The statements after a `for_each` over the number 4.
fn run_15() -> ServerRun {
    let mut run = ServerRun::new(
        "run_15",
        FIRST_RELEASE,
        "what the statements after a for_each over a non-array store and read: follow-up of run_14",
    );
    run.checks_load_messages();
    run.arm64_differs(&[]);
    run.probe(
        "f01",
        "t.st = 1; t.b = 3; v.after = 0; v.rb = 0; v.x = 0; v.a = 4; for_each(v.x, v.a, 7); t.st = 2; v.after = 5; v.rb = t.b; v.f01_done;",
    )
    .answers("f01_done");
    run.probe(
        "f02",
        "t.st == 2 ? v.f02_is2 : (t.st == 1 ? v.f02_is1 : (t.st == 7 ? v.f02_is7 : (t.st == 4 ? v.f02_is4 : (t.st == 0 ? v.f02_is0 : v.f02_none))));",
    )
    .answers("f02_is1");
    run.probe(
        "f03",
        "v.after == 5 ? v.f03_is5 : (v.after == 0 ? v.f03_is0 : (v.after == 7 ? v.f03_is7 : (v.after == 4 ? v.f03_is4 : (v.after == 2 ? v.f03_is2 : v.f03_none))));",
    )
    .answers("f03_is5");
    run.probe(
        "f04",
        "v.rb == 3 ? v.f04_is3 : (v.rb == 0 ? v.f04_is0 : (v.rb == 7 ? v.f04_is7 : (v.rb == 4 ? v.f04_is4 : (v.rb == 5 ? v.f04_is5 : (v.rb == 2 ? v.f04_is2 : v.f04_none)))));",
    )
    .answers("f04_is3");
    run.probe(
        "f05",
        "v.x == 0 ? v.f05_is0 : (v.x == 4 ? v.f05_is4 : (v.x == 7 ? v.f05_is7 : (v.x == 2 ? v.f05_is2 : (v.x == 5 ? v.f05_is5 : (v.x == 3 ? v.f05_is3 : v.f05_none)))));",
    )
    .answers("f05_is0");
    run.probe("f99_marker_end", "v.f99_marker_end;")
        .answers("f99_marker_end");
    run
}

#[test]
fn after_a_for_each_over_four_the_next_write_is_lost_and_later_ones_land_run_15() {
    run_15().replay(6);
}

/// Root `return` and statement lists as transition conditions, with an `on_entry` list that does
/// not run.
fn run_20() -> ServerRun {
    let mut run = ServerRun::new(
        "run_20",
        FIRST_RELEASE,
        "first attempt of run_21: the on_entry list of the default state did not run (none of its probes printed) while the transition chain answered. Superseded by run_21",
    );
    run.transitions("default", &[("y01_yes", "return 1"), ("y01_no", "1")]);
    run.transitions("y01_yes", &[("y02_yes", "return 1;"), ("y02_no", "1")]);
    run.transitions("y01_no", &[("y02_yes", "return 1;"), ("y02_no", "1")]);
    run.transitions("y02_yes", &[("y03_yes", "v.ty03 = 1;"), ("y03_no", "1")]);
    run.transitions("y02_no", &[("y03_yes", "v.ty03 = 1;"), ("y03_no", "1")]);
    run.transitions("y03_yes", &[("y04_yes", "1;"), ("y04_no", "1")]);
    run.transitions("y03_no", &[("y04_yes", "1;"), ("y04_no", "1")]);
    run.arm64_differs(&[]);
    run.probe(
        "r18",
        "v.p1 = math.sign(0); v.p1 == 1 ? v.r18_sign0_is1 : v.r18_sign0_other;",
    )
    .not_run();
    run.probe(
        "r01",
        "v.n = math.sqrt(-1); v.f = 4; v.p1 = math.max(v.n, v.f); v.p1 == 4 ? v.r01_maxnanfirst_is4 : (v.p1 == v.p1 ? v.r01_maxnanfirst_other : v.r01_maxnanfirst_isnan);",
    )
    .not_run();
    run.probe(
        "r02",
        "v.p1 = math.max(v.f, v.n); v.p1 == 4 ? v.r02_maxnansecond_is4 : (v.p1 == v.p1 ? v.r02_maxnansecond_other : v.r02_maxnansecond_isnan);",
    )
    .not_run();
    run.probe(
        "x25",
        "t.st25 = 1; v.r25 = 9; v.r25 = math.abs(return 5); t.st25 = 2; v.r25 == 0 ? v.x25_return_generic0 : (v.r25 == 5 ? v.x25_return_5 : (v.r25 == 9 ? v.x25_unassigned : v.x25_other));",
    )
    .not_run();
    run.probe("w25", "v.k25 = t.st25 ?? 0; v.k25 == 2 ? v.w25_completed : (v.k25 == 1 ? v.w25_ended_inside : v.w25_not_run);")
        .not_run();
    run.probe("x88", "t.st88 = 1; t.n88 = 0; loop(3, { t.n88 = t.n88 + 1; return 1; }); t.st88 = 2; v.x88_ran_on;")
        .not_run();
    run.probe("w88", "v.k88 = t.st88 ?? 0; v.m88 = t.n88 ?? 0; v.k88 == 2 ? (v.m88 == 1 ? v.w88_ran_on_n1 : (v.m88 == 3 ? v.w88_ran_on_n3 : v.w88_ran_on_nother)) : (v.k88 == 1 ? (v.m88 == 1 ? v.w88_ended_n1 : v.w88_ended_nother) : v.w88_not_run);").not_run();
    run.probe(
        "x89",
        "t.st89 = 1; v.c89 = 1; v.c89 ? { t.st89 = 3; return 2; } : 0; t.st89 = 2; v.x89_ran_on;",
    )
    .not_run();
    run.probe(
        "w89",
        "v.k89 = t.st89 ?? 0; v.k89 == 2 ? v.w89_ran_on : (v.k89 == 3 ? v.w89_ended_in_block : (v.k89 == 1 ? v.w89_block_not_entered : v.w89_not_run));",
    )
    .not_run();
    run.probe(
        "x90",
        "t.st90 = 1; t.n90 = 0; t.m90 = 0; loop(2, { loop(2, { t.n90 = t.n90 + 1; return 1; }); t.m90 = t.m90 + 1; }); t.st90 = 2; v.x90_ran_on;",
    )
    .not_run();
    run.probe("w90", "v.k90 = t.st90 ?? 0; v.n90 = t.n90 ?? 0; v.m90 = t.m90 ?? 0; v.k90 == 1 ? ((v.n90 == 1 && v.m90 == 0) ? v.w90_ended_n1_m0 : v.w90_ended_other_counts) : (v.k90 == 2 ? ((v.n90 == 2 && v.m90 == 2) ? v.w90_ran_on_inner_left_each_time : v.w90_ran_on_other_counts) : v.w90_not_run);").not_run();
    run.probe("y01t", "v.y01_root_return_nonzero;")
        .in_state("y01_yes")
        .silent()
        .inconclusive();
    run.probe("y01f", "v.y01_root_return_zero;")
        .in_state("y01_no")
        .answers("y01_root_return_zero");
    run.probe("y02t", "v.y02_return_semicolon_nonzero;")
        .in_state("y02_yes")
        .answers("y02_return_semicolon_nonzero");
    run.probe("y02f", "v.y02_return_semicolon_zero;")
        .in_state("y02_no")
        .silent()
        .inconclusive();
    run.probe("y03t", "v.y03_assign_list_nonzero;")
        .in_state("y03_yes")
        .silent()
        .inconclusive();
    run.probe("y03f", "v.y03_assign_list_zero;")
        .in_state("y03_no")
        .answers("y03_assign_list_zero");
    run.probe("y04t", "v.y04_value_list_nonzero;")
        .in_state("y04_yes")
        .silent()
        .inconclusive();
    run.probe("y04f", "v.y04_value_list_zero;")
        .in_state("y04_no")
        .answers("y04_value_list_zero");
    run
}

/// As transition conditions, `return 1` and the statement lists `v.ty03 = 1;` and `1;` are zero;
/// `return 1;` is non-zero.
#[test]
fn a_root_return_is_worth_zero_as_a_transition_condition_run_20() {
    run_20().replay(19);
}

/// `return` as a call argument, inside loops and blocks, and as a transition condition.
fn run_21() -> ServerRun {
    let mut run = ServerRun::new(
        "run_21",
        FIRST_RELEASE,
        "return as an argument, in loops and blocks, and the value of a root `return 1` via controller transitions",
    );
    run.transitions("default", &[("ystart", "v.ygo ?? 0")]);
    run.transitions("ystart", &[("y01_yes", "return 1"), ("y01_no", "1")]);
    run.transitions("y01_yes", &[("y02_yes", "return 1;"), ("y02_no", "1")]);
    run.transitions("y01_no", &[("y02_yes", "return 1;"), ("y02_no", "1")]);
    run.transitions("y02_yes", &[("y03_yes", "v.ty03 = 1;"), ("y03_no", "1")]);
    run.transitions("y02_no", &[("y03_yes", "v.ty03 = 1;"), ("y03_no", "1")]);
    run.transitions("y03_yes", &[("y04_yes", "1;"), ("y04_no", "1")]);
    run.transitions("y03_no", &[("y04_yes", "1;"), ("y04_no", "1")]);
    run.arm64_differs(&["r02"]);
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
    run.probe(
        "r02",
        "v.p1 = math.max(v.f, v.n); v.p1 == 4 ? v.r02_maxnansecond_is4 : (v.p1 == v.p1 ? v.r02_maxnansecond_other : v.r02_maxnansecond_isnan);",
    )
    .answers("r02_maxnansecond_isnan");
    run.probe(
        "x25",
        "t.st25 = 1; v.r25 = 9; v.r25 = math.abs(return 5); t.st25 = 2; v.r25 == 0 ? v.x25_return_generic0 : (v.r25 == 5 ? v.x25_return_5 : (v.r25 == 9 ? v.x25_unassigned : v.x25_other));",
    )
    .answers("x25_return_generic0");
    run.probe("w25", "v.k25 = t.st25 ?? 0; v.k25 == 2 ? v.w25_completed : (v.k25 == 1 ? v.w25_ended_inside : v.w25_not_run);")
        .answers("w25_completed");
    run.probe("x88", "t.st88 = 1; t.n88 = 0; loop(3, { t.n88 = t.n88 + 1; return 1; }); t.st88 = 2; v.x88_ran_on;")
        .silent()
        .inconclusive();
    run.probe("w88", "v.k88 = t.st88 ?? 0; v.m88 = t.n88 ?? 0; v.k88 == 2 ? (v.m88 == 1 ? v.w88_ran_on_n1 : (v.m88 == 3 ? v.w88_ran_on_n3 : v.w88_ran_on_nother)) : (v.k88 == 1 ? (v.m88 == 1 ? v.w88_ended_n1 : v.w88_ended_nother) : v.w88_not_run);").answers("w88_ended_n1");
    run.probe(
        "x89",
        "t.st89 = 1; v.c89 = 1; v.c89 ? { t.st89 = 3; return 2; } : 0; t.st89 = 2; v.x89_ran_on;",
    )
    .silent()
    .inconclusive();
    run.probe(
        "w89",
        "v.k89 = t.st89 ?? 0; v.k89 == 2 ? v.w89_ran_on : (v.k89 == 3 ? v.w89_ended_in_block : (v.k89 == 1 ? v.w89_block_not_entered : v.w89_not_run));",
    )
    .answers("w89_ended_in_block");
    run.probe(
        "x90",
        "t.st90 = 1; t.n90 = 0; t.m90 = 0; loop(2, { loop(2, { t.n90 = t.n90 + 1; return 1; }); t.m90 = t.m90 + 1; }); t.st90 = 2; v.x90_ran_on;",
    )
    .silent()
    .inconclusive();
    run.probe("w90", "v.k90 = t.st90 ?? 0; v.n90 = t.n90 ?? 0; v.m90 = t.m90 ?? 0; v.k90 == 1 ? ((v.n90 == 1 && v.m90 == 0) ? v.w90_ended_n1_m0 : v.w90_ended_other_counts) : (v.k90 == 2 ? ((v.n90 == 2 && v.m90 == 2) ? v.w90_ran_on_inner_left_each_time : v.w90_ran_on_other_counts) : v.w90_not_run);").answers("w90_ended_n1_m0");
    // Starts the transition chain after the `on_entry` list has run.
    run.probe("ygo", "v.ygo = 1;").silent();
    run.probe("y01t", "v.y01_root_return_nonzero;")
        .in_state("y01_yes")
        .silent()
        .inconclusive();
    run.probe("y01f", "v.y01_root_return_zero;")
        .in_state("y01_no")
        .answers("y01_root_return_zero");
    run.probe("y02t", "v.y02_return_semicolon_nonzero;")
        .in_state("y02_yes")
        .answers("y02_return_semicolon_nonzero");
    run.probe("y02f", "v.y02_return_semicolon_zero;")
        .in_state("y02_no")
        .silent()
        .inconclusive();
    run.probe("y03t", "v.y03_assign_list_nonzero;")
        .in_state("y03_yes")
        .silent()
        .inconclusive();
    run.probe("y03f", "v.y03_assign_list_zero;")
        .in_state("y03_no")
        .answers("y03_assign_list_zero");
    run.probe("y04t", "v.y04_value_list_nonzero;")
        .in_state("y04_yes")
        .silent()
        .inconclusive();
    run.probe("y04f", "v.y04_value_list_zero;")
        .in_state("y04_no")
        .answers("y04_value_list_zero");
    run
}

#[test]
fn a_return_ends_the_whole_expression_and_is_worth_zero_as_an_argument_run_21() {
    run_21().replay(20);
}

/// `break` inside a query argument, and `break` / `continue` inside an operand in a loop.
fn run_22() -> ServerRun {
    let mut run = ServerRun::new(
        "run_22",
        FIRST_RELEASE,
        "break in a query argument, break / continue inside an operand",
    );
    run.arm64_differs(&["r02"]);
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
    run.probe(
        "r02",
        "v.p1 = math.max(v.f, v.n); v.p1 == 4 ? v.r02_maxnansecond_is4 : (v.p1 == v.p1 ? v.r02_maxnansecond_other : v.r02_maxnansecond_isnan);",
    )
    .answers("r02_maxnansecond_isnan");
    run.probe("x40", "t.st40 = 1; v.one = 1; v.n40 = 0; loop(2, { v.n40 = v.n40 + 1; v.r40 = q.log((v.one ? break : 1)); }); t.st40 = 2; v.n40 == 2 ? v.x40_argbreak_local : (v.n40 == 1 ? v.x40_argbreak_outer : v.x40_other);").load_logs(&["Error: unreachable statements after Break 'break'."]).answers("x40_argbreak_local");
    run.probe(
        "w40",
        "v.k40 = t.st40 ?? 0; v.m40 = v.n40 ?? 0; v.k40 == 2 ? v.w40_completed : (v.k40 == 1 ? (v.m40 == 1 ? v.w40_ended_n1 : (v.m40 == 2 ? v.w40_ended_n2 : v.w40_ended_nother)) : v.w40_not_run);",
    )
    .answers("w40_completed");
    run.probe("x21", "t.st21 = 1; v.k21 = 0; v.j21 = 0; v.t21 = 7; v.a21 = 2; v.b21 = 3; loop(3, { v.j21 = v.j21 + 1; v.t21 = v.k21 * (v.j21 > 0 ? {break;} : 0); }); t.st21 = 2; v.m21 = v.a21 * v.b21 + v.a21; v.j21 == 1 ? (v.m21 == 8 ? v.x21_brk_pending_1_arith_ok : v.x21_brk_pending_1_arith_bad) : (v.j21 == 3 ? (v.m21 == 8 ? v.x21_brk_pending_3_arith_ok : v.x21_brk_pending_3_arith_bad) : v.x21_other);").answers("x21_brk_pending_1_arith_ok");
    run.probe(
        "w21",
        "v.k21w = t.st21 ?? 0; v.k21w == 2 ? (v.t21 == 7 ? v.w21_completed_t7 : (v.t21 == 0 ? v.w21_completed_t0 : v.w21_completed_tother)) : (v.k21w == 1 ? v.w21_ended_inside : v.w21_not_run);",
    )
    .answers("w21_completed_t7");
    run.probe("x20", "t.st20 = 1; v.k = 0; v.i = 0; v.t20 = 7; v.a20 = 2; v.b20 = 3; loop(3, { v.i = v.i + 1; v.t20 = v.k * (v.i > 0 ? {continue;} : 0); }); t.st20 = 2; v.m20 = v.a20 * v.b20 + v.a20; v.i == 1 ? (v.m20 == 8 ? v.x20_cont_pending_1 : v.x20_cont_pending_1_arith_bad) : (v.i == 3 ? (v.m20 == 8 ? v.x20_cont_pending_3 : v.x20_cont_pending_3_arith_bad) : v.x20_cont_other);").answers("x20_cont_pending_1");
    run.probe(
        "w20",
        "v.k20 = t.st20 ?? 0; v.k20 == 2 ? (v.t20 == 7 ? v.w20_completed_t7 : (v.t20 == 0 ? v.w20_completed_t0 : v.w20_completed_tother)) : (v.k20 == 1 ? v.w20_ended_inside : v.w20_not_run);",
    )
    .answers("w20_completed_t7");
    run
}

/// A `break` inside a query argument ends only the argument; inside an operand it ends the loop and
/// the assignment never completes.
#[test]
fn break_or_continue_inside_an_operand_ends_the_loop_run_22() {
    run_22().replay(9);
}

/// `continue` inside an operand of a product with a negative factor or of a sum.
fn run_23() -> ServerRun {
    let mut run = ServerRun::new(
        "run_23",
        FIRST_RELEASE,
        "continue inside an operand of a product with a negative factor or of a + (follow-up of run_22 x20)",
    );
    run.arm64_differs(&["r02"]);
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
    run.probe(
        "r02",
        "v.p1 = math.max(v.f, v.n); v.p1 == 4 ? v.r02_maxnansecond_is4 : (v.p1 == v.p1 ? v.r02_maxnansecond_other : v.r02_maxnansecond_isnan);",
    )
    .answers("r02_maxnansecond_isnan");
    run.probe("x20n", "t.st20n = 1; v.kn = -1; v.in = 0; v.t20n = 7; loop(3, { v.in = v.in + 1; v.t20n = v.kn * (v.in > 0 ? {continue;} : 0); }); t.st20n = 2; v.in == 1 ? v.x20n_cont_pending_1 : (v.in == 3 ? v.x20n_cont_pending_3 : v.x20n_cont_other);").answers("x20n_cont_pending_1");
    run.probe(
        "w20n",
        "v.k20n = t.st20n ?? 0; v.k20n == 2 ? (v.t20n == 7 ? v.w20n_completed_t7 : v.w20n_completed_tother) : (v.k20n == 1 ? v.w20n_ended_inside : v.w20n_not_run);",
    )
    .answers("w20n_completed_t7");
    run.probe("x20p", "t.st20p = 1; v.kp = 0; v.ip = 0; loop(3, { v.ip = v.ip + 1; v.tp = v.kp + (v.ip > 0 ? {continue;} : 0); }); t.st20p = 2; v.ip == 1 ? v.x20p_cont_add_1 : (v.ip == 3 ? v.x20p_cont_add_3 : v.x20p_cont_add_other);").answers("x20p_cont_add_1");
    run
}

#[test]
fn continue_inside_a_product_with_a_negative_factor_or_a_sum_ends_the_loop_run_23() {
    run_23().replay(6);
}

/// `continue` inside an operand in an inner loop.
fn run_29() -> ServerRun {
    let mut run = ServerRun::new(
        "run_29",
        FIRST_RELEASE,
        "run_26 c04 again, bounded by construction, alone, under a memory limit: the suspected runaway",
    );
    run.arm64_differs(&["r02"]);
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
    run.probe(
        "r02",
        "v.p1 = math.max(v.f, v.n); v.p1 == 4 ? v.r02_maxnansecond_is4 : (v.p1 == v.p1 ? v.r02_maxnansecond_other : v.r02_maxnansecond_isnan);",
    )
    .answers("r02_maxnansecond_isnan");
    // Each loop escapes with `return 0` after 9 outer or 13 inner passes.
    run.probe("c04b", "t.stc04b = 1; v.zeroc04b = 0; v.gc04b = 0; v.hc04b = 0; v.ic04b = 0; v.jc04b = 0; loop(2, { v.gc04b = v.gc04b + 1; v.gc04b > 8 ? {return 0;} : 0; v.ic04b = v.ic04b + 1; loop(3, { v.hc04b = v.hc04b + 1; v.hc04b > 12 ? {return 0;} : 0; v.jc04b = v.jc04b + 1; v.tc04b = v.zeroc04b * (v.jc04b > 0 ? {continue;} : 0); }); }); t.stc04b = 2; v.ic04b == 2 ? (v.jc04b == 2 ? v.c04b_outer2_inner1each : (v.jc04b == 6 ? v.c04b_outer2_inner3each : v.c04b_outer2_jother)) : (v.ic04b == 1 ? v.c04b_outer1 : v.c04b_other);").silent().inconclusive();
    run.probe("w04b", "v.kc04b = t.stc04b ?? 0; v.kc04b == 2 ? v.w04b_completed : (v.kc04b == 1 ? (v.gc04b > 8 ? v.w04b_outer_runaway_escaped : (v.hc04b > 12 ? v.w04b_inner_runaway_escaped : v.w04b_ended_other)) : v.w04b_not_run);").answers("w04b_outer_runaway_escaped");
    run
}

#[test]
fn continue_inside_an_operand_in_an_inner_loop_makes_the_outer_loop_run_away_run_29() {
    run_29().replay(5);
}

/// `break` inside an operand in an inner loop.
fn run_30() -> ServerRun {
    let mut run = ServerRun::new(
        "run_30",
        FIRST_RELEASE,
        "the break counterpart of run_29, bounded, alone, under a memory limit",
    );
    run.arm64_differs(&["r02"]);
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
    run.probe(
        "r02",
        "v.p1 = math.max(v.f, v.n); v.p1 == 4 ? v.r02_maxnansecond_is4 : (v.p1 == v.p1 ? v.r02_maxnansecond_other : v.r02_maxnansecond_isnan);",
    )
    .answers("r02_maxnansecond_isnan");
    // Each loop escapes with `return 0` after 9 outer or 13 inner passes.
    run.probe("c05", "t.stc05 = 1; v.zeroc05 = 0; v.gc05 = 0; v.hc05 = 0; v.ic05 = 0; v.jc05 = 0; loop(2, { v.gc05 = v.gc05 + 1; v.gc05 > 8 ? {return 0;} : 0; v.ic05 = v.ic05 + 1; loop(3, { v.hc05 = v.hc05 + 1; v.hc05 > 12 ? {return 0;} : 0; v.jc05 = v.jc05 + 1; v.tc05 = v.zeroc05 * (v.jc05 > 0 ? {break;} : 0); }); }); t.stc05 = 2; v.ic05 == 2 ? (v.jc05 == 2 ? v.c05_outer2_inner1each : (v.jc05 == 6 ? v.c05_outer2_inner3each : v.c05_outer2_jother)) : (v.ic05 == 1 ? v.c05_outer1 : v.c05_other);").silent().inconclusive();
    run.probe("w05", "v.kc05 = t.stc05 ?? 0; v.kc05 == 2 ? v.w05_completed : (v.kc05 == 1 ? (v.gc05 > 8 ? v.w05_outer_runaway_escaped : (v.hc05 > 12 ? v.w05_inner_runaway_escaped : v.w05_ended_other)) : v.w05_not_run);").answers("w05_outer_runaway_escaped");
    run
}

#[test]
fn break_inside_an_operand_in_an_inner_loop_makes_the_outer_loop_run_away_run_30() {
    run_30().replay(5);
}

/// A `for_each` over a value that is not an actor array.
fn run_32() -> ServerRun {
    let mut run = ServerRun::new(
        "run_32",
        BOTH_RELEASES,
        "what a for_each over a value that is not an actor array does to the rest of the expression",
    );
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
    run.probe(
        "r02",
        "v.p1 = math.max(v.f, v.n); v.p1 == 4 ? v.r02_maxnansecond_is4 : (v.p1 == v.p1 ? v.r02_maxnansecond_other : v.r02_maxnansecond_isnan);",
    )
    .answers("r02_maxnansecond_isnan");
    run.probe(
        "f01",
        "v.a = 0; for_each(v.i, v.a, 1); v.f01_first; v.f01_second;",
    )
    .answers("f01_second");
    run.probe(
        "f02",
        "v.a = 4; for_each(v.i, v.a, 1); v.f02_first; v.f02_second;",
    )
    .answers("f02_second");
    run.probe(
        "f03",
        "v.a = 0; for_each(t.i, v.a, 1); v.f03_first; v.f03_second;",
    )
    .answers("f03_second");
    run.probe(
        "f04",
        "v.a = 0; for_each(v.i, v.a, {v.z = 1;}); v.f04_first; v.f04_second;",
    )
    .answers("f04_second");
    run.probe(
        "f05",
        "v.a = 'x'; for_each(v.i, v.a, 1); v.f05_first; v.f05_second;",
    )
    .answers("f05_second");
    run.probe(
        "f06",
        "v.f6 = 0; v.a = 0; for_each(v.i, v.a, 1); v.f6 = 1; v.f6 = 2; v.f6 = 3; v.f06_done;",
    )
    .answers("f06_done");
    run.probe(
        "f06r",
        "v.f6 == 3 ? v.f06r_3 : (v.f6 == 2 ? v.f06r_2 : (v.f6 == 1 ? v.f06r_1 : (v.f6 == 0 ? v.f06r_0 : v.f06r_other)));",
    )
    .answers("f06r_3");
    run.probe(
        "f07",
        "{v.a = 0; for_each(v.i, v.a, 1);}; v.f07_first; v.f07_second;",
    )
    .answers("f07_first");
    run.probe(
        "f08",
        "v.a = 0; loop(1, {for_each(v.i, v.a, 1); v.f08_first;}); v.f08_second;",
    )
    .answers("f08_second");
    run.probe(
        "f09",
        "v.a = 0; v.r9 = 0; for_each(v.i, v.a, 1); v.r9 = v.r9 + 5; v.r9 == 5 ? v.f09_5 : (v.r9 == 0 ? v.f09_0 : v.f09_other);",
    )
    .answers("f09_0");
    run.probe("f10", "v.a = 0; for_each(v.i, v.a, 1); for_each(v.i, v.a, 1); v.f10_first; v.f10_second; v.f10_third;")
        .answers("f10_second");
    run.probe(
        "f11",
        "t.f11 = 1; v.a = 0; for_each(v.i, v.a, 1); v.f11_pad = 0; t.f11 = 2; t.f11 == 2 ? v.f11_is2 : (t.f11 == 1 ? v.f11_is1 : v.f11_other);",
    )
    .answers("f11_is2");
    run.probe("f12", "t.b12 = 3; v.a = 0; for_each(v.i, v.a, 1); v.f12_pad = 0; t.b12 == 3 ? v.f12_sees3 : v.f12_lost;")
        .answers("f12_sees3");
    run.probe("f99", "v.f99_marker_end;")
        .answers("f99_marker_end");
    run
}

/// The statement after such a `for_each` is skipped, also inside a loop body, but not when the
/// `for_each` ends a `{ }` block.
#[test]
fn a_for_each_over_a_non_array_skips_the_next_statement_run_32() {
    run_32().replay(17);
}

/// A `??` left by a `break`, then loop and `math.die_roll` counts that are NaN, infinite, −0,
/// fractional or too large.
fn run_33() -> ServerRun {
    let mut run = ServerRun::new(
        "run_33",
        BOTH_RELEASES,
        "probes that may make the server run away, alone, under a memory limit: a ?? left by a break, loop and die_roll counts that are NaN, infinite, -0, fractional or beyond the int range",
    );
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
    run.probe(
        "r02",
        "v.p1 = math.max(v.f, v.n); v.p1 == 4 ? v.r02_maxnansecond_is4 : (v.p1 == v.p1 ? v.r02_maxnansecond_other : v.r02_maxnansecond_isnan);",
    )
    .answers("r02_maxnansecond_isnan");
    run.probe(
        "r1",
        "v.g = 0; v.b = 0; loop(1, { v.g = v.g + 1; v.g > 8 ? {return 0;} : 0; v.b = (v.r1_c_never ?? {break;}) ?? 2; }); v.r1_never_set;",
    )
    .not_answered();
    run.probe(
        "r1w",
        "v.b == 2 ? (v.g == 1 ? v.r1w_stale_g1 : v.r1w_stale_gmore) : (v.b == 0 ? (v.g == 1 ? v.r1w_dropped_g1 : v.r1w_dropped_gmore) : v.r1w_other);",
    )
    .not_run();
    run.probe("r2v", "v.lc = math.sqrt(-1); v.g = 0; loop(v.lc, {v.g = v.g + 1; v.g > 8 ? {break;} : 0;}); v.g == 0 ? v.r2v_0 : (v.g == 1 ? v.r2v_1 : (v.g == 2 ? v.r2v_2 : (v.g == 9 ? v.r2v_escaped : v.r2v_other)));").not_run();
    run.probe(
        "r2c",
        "v.g = 0; loop(math.sqrt(-1), {v.g = v.g + 1; v.g > 8 ? {break;} : 0;}); v.g == 0 ? v.r2c_0 : (v.g == 1 ? v.r2c_1 : (v.g == 2 ? v.r2c_2 : (v.g == 9 ? v.r2c_escaped : v.r2c_other)));",
    )
    .not_run();
    run.probe("r3v", "v.lc = -math.ln(0); v.g = 0; loop(v.lc, {v.g = v.g + 1; v.g > 8 ? {break;} : 0;}); v.g == 0 ? v.r3v_0 : (v.g == 1 ? v.r3v_1 : (v.g == 2 ? v.r3v_2 : (v.g == 9 ? v.r3v_escaped : v.r3v_other)));").not_run();
    run.probe(
        "r3c",
        "v.g = 0; loop(-math.ln(0), {v.g = v.g + 1; v.g > 8 ? {break;} : 0;}); v.g == 0 ? v.r3c_0 : (v.g == 1 ? v.r3c_1 : (v.g == 2 ? v.r3c_2 : (v.g == 9 ? v.r3c_escaped : v.r3c_other)));",
    )
    .not_run();
    run.probe("r4v", "v.lc = math.ln(0); v.g = 0; loop(v.lc, {v.g = v.g + 1; v.g > 8 ? {break;} : 0;}); v.g == 0 ? v.r4v_0 : (v.g == 1 ? v.r4v_1 : (v.g == 2 ? v.r4v_2 : (v.g == 9 ? v.r4v_escaped : v.r4v_other)));").not_run();
    run.probe(
        "r4c",
        "v.g = 0; loop(math.ln(0), {v.g = v.g + 1; v.g > 8 ? {break;} : 0;}); v.g == 0 ? v.r4c_0 : (v.g == 1 ? v.r4c_1 : (v.g == 2 ? v.r4c_2 : (v.g == 9 ? v.r4c_escaped : v.r4c_other)));",
    )
    .not_run();
    run.probe("r5v", "v.lc = math.copy_sign(0, -1); v.g = 0; loop(v.lc, {v.g = v.g + 1; v.g > 8 ? {break;} : 0;}); v.g == 0 ? v.r5v_0 : (v.g == 1 ? v.r5v_1 : (v.g == 2 ? v.r5v_2 : (v.g == 9 ? v.r5v_escaped : v.r5v_other)));").not_run();
    run.probe(
        "r5c",
        "v.g = 0; loop(math.copy_sign(0, -1), {v.g = v.g + 1; v.g > 8 ? {break;} : 0;}); v.g == 0 ? v.r5c_0 : (v.g == 1 ? v.r5c_1 : (v.g == 2 ? v.r5c_2 : (v.g == 9 ? v.r5c_escaped : v.r5c_other)));",
    )
    .not_run();
    run.probe(
        "r6v",
        "v.lc = 0.5; v.g = 0; loop(v.lc, {v.g = v.g + 1; v.g > 8 ? {break;} : 0;}); v.g == 0 ? v.r6v_0 : (v.g == 1 ? v.r6v_1 : (v.g == 2 ? v.r6v_2 : (v.g == 9 ? v.r6v_escaped : v.r6v_other)));",
    )
    .not_run();
    run.probe(
        "r6c",
        "v.g = 0; loop(0.5, {v.g = v.g + 1; v.g > 8 ? {break;} : 0;}); v.g == 0 ? v.r6c_0 : (v.g == 1 ? v.r6c_1 : (v.g == 2 ? v.r6c_2 : (v.g == 9 ? v.r6c_escaped : v.r6c_other)));",
    )
    .not_run();
    run.probe(
        "r7v",
        "v.lc = 1.5; v.g = 0; loop(v.lc, {v.g = v.g + 1; v.g > 8 ? {break;} : 0;}); v.g == 0 ? v.r7v_0 : (v.g == 1 ? v.r7v_1 : (v.g == 2 ? v.r7v_2 : (v.g == 9 ? v.r7v_escaped : v.r7v_other)));",
    )
    .not_run();
    run.probe(
        "r7c",
        "v.g = 0; loop(1.5, {v.g = v.g + 1; v.g > 8 ? {break;} : 0;}); v.g == 0 ? v.r7c_0 : (v.g == 1 ? v.r7c_1 : (v.g == 2 ? v.r7c_2 : (v.g == 9 ? v.r7c_escaped : v.r7c_other)));",
    )
    .not_run();
    run.probe(
        "r8v",
        "v.lc = 3e9; v.g = 0; loop(v.lc, {v.g = v.g + 1; v.g > 8 ? {break;} : 0;}); v.g == 0 ? v.r8v_0 : (v.g == 1 ? v.r8v_1 : (v.g == 2 ? v.r8v_2 : (v.g == 9 ? v.r8v_escaped : v.r8v_other)));",
    )
    .not_run();
    run.probe(
        "r8c",
        "v.g = 0; loop(3e9, {v.g = v.g + 1; v.g > 8 ? {break;} : 0;}); v.g == 0 ? v.r8c_0 : (v.g == 1 ? v.r8c_1 : (v.g == 2 ? v.r8c_2 : (v.g == 9 ? v.r8c_escaped : v.r8c_other)));",
    )
    .not_run();
    run.probe(
        "d1",
        "v.dc = math.sqrt(-1); v.r = math.die_roll(v.dc, 1, 2); v.r == 0 ? v.d1_0 : (v.r == v.r ? v.d1_rolled : v.d1_nan);",
    )
    .not_run();
    run.probe("d2", "v.dc = -math.ln(0); v.r = math.die_roll(v.dc, 1, 2); v.r == 0 ? v.d2_0 : (v.r == v.r ? v.d2_rolled : v.d2_nan);")
        .not_run();
    run.probe("d3", "v.dc = 3e9; v.r = math.die_roll(v.dc, 1, 2); v.r == 0 ? v.d3_0 : (v.r == v.r ? v.d3_rolled : v.d3_nan);")
        .not_run();
    run.probe("d4", "v.dc = 2147483648; v.r = math.die_roll(v.dc, 1, 2); v.r == 0 ? v.d4_0 : (v.r == v.r ? v.d4_rolled : v.d4_nan);")
        .not_run();
    run.probe("d5", "v.dc = -2147483648; v.r = math.die_roll(v.dc, 1, 2); v.r == 0 ? v.d5_0 : (v.r == v.r ? v.d5_rolled : v.d5_nan);")
        .not_run();
    run.probe("r99", "v.r99_marker_end;").not_run();
    run
}

/// A `break` out of the left side of a `??` inside a loop leaves the rest of the run unanswered.
#[test]
fn a_break_out_of_a_coalescing_left_side_in_a_loop_crashed_the_server_run_33() {
    run_33().replay(25);
}

/// Loop and `math.die_roll` counts that are NaN, infinite, −0, fractional or beyond the `i32`
/// range.
fn run_34() -> ServerRun {
    let mut run = ServerRun::new(
        "run_34",
        BOTH_RELEASES,
        "run_33 without its first probe (a ?? left by a break), which crashed both servers: loop and die_roll counts that are NaN, infinite, -0, fractional or beyond the int range, alone, under a memory limit",
    );
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
    run.probe(
        "r02",
        "v.p1 = math.max(v.f, v.n); v.p1 == 4 ? v.r02_maxnansecond_is4 : (v.p1 == v.p1 ? v.r02_maxnansecond_other : v.r02_maxnansecond_isnan);",
    )
    .answers("r02_maxnansecond_isnan");
    // Each count as a variable and as a constant; every loop escapes after 9 passes.
    run.probe("r2v", "v.lc = math.sqrt(-1); v.g = 0; loop(v.lc, {v.g = v.g + 1; v.g > 8 ? {break;} : 0;}); v.g == 0 ? v.r2v_0 : (v.g == 1 ? v.r2v_1 : (v.g == 2 ? v.r2v_2 : (v.g == 9 ? v.r2v_escaped : v.r2v_other)));").answers("r2v_escaped");
    run.probe(
        "r2c",
        "v.g = 0; loop(math.sqrt(-1), {v.g = v.g + 1; v.g > 8 ? {break;} : 0;}); v.g == 0 ? v.r2c_0 : (v.g == 1 ? v.r2c_1 : (v.g == 2 ? v.r2c_2 : (v.g == 9 ? v.r2c_escaped : v.r2c_other)));",
    )
    .answers("r2c_escaped");
    run.probe("r3v", "v.lc = -math.ln(0); v.g = 0; loop(v.lc, {v.g = v.g + 1; v.g > 8 ? {break;} : 0;}); v.g == 0 ? v.r3v_0 : (v.g == 1 ? v.r3v_1 : (v.g == 2 ? v.r3v_2 : (v.g == 9 ? v.r3v_escaped : v.r3v_other)));").answers("r3v_escaped");
    run.probe(
        "r3c",
        "v.g = 0; loop(-math.ln(0), {v.g = v.g + 1; v.g > 8 ? {break;} : 0;}); v.g == 0 ? v.r3c_0 : (v.g == 1 ? v.r3c_1 : (v.g == 2 ? v.r3c_2 : (v.g == 9 ? v.r3c_escaped : v.r3c_other)));",
    )
    .answers("r3c_escaped");
    run.probe("r4v", "v.lc = math.ln(0); v.g = 0; loop(v.lc, {v.g = v.g + 1; v.g > 8 ? {break;} : 0;}); v.g == 0 ? v.r4v_0 : (v.g == 1 ? v.r4v_1 : (v.g == 2 ? v.r4v_2 : (v.g == 9 ? v.r4v_escaped : v.r4v_other)));").answers("r4v_0");
    run.probe(
        "r4c",
        "v.g = 0; loop(math.ln(0), {v.g = v.g + 1; v.g > 8 ? {break;} : 0;}); v.g == 0 ? v.r4c_0 : (v.g == 1 ? v.r4c_1 : (v.g == 2 ? v.r4c_2 : (v.g == 9 ? v.r4c_escaped : v.r4c_other)));",
    )
    .answers("r4c_0");
    run.probe("r5v", "v.lc = math.copy_sign(0, -1); v.g = 0; loop(v.lc, {v.g = v.g + 1; v.g > 8 ? {break;} : 0;}); v.g == 0 ? v.r5v_0 : (v.g == 1 ? v.r5v_1 : (v.g == 2 ? v.r5v_2 : (v.g == 9 ? v.r5v_escaped : v.r5v_other)));").answers("r5v_0");
    run.probe(
        "r5c",
        "v.g = 0; loop(math.copy_sign(0, -1), {v.g = v.g + 1; v.g > 8 ? {break;} : 0;}); v.g == 0 ? v.r5c_0 : (v.g == 1 ? v.r5c_1 : (v.g == 2 ? v.r5c_2 : (v.g == 9 ? v.r5c_escaped : v.r5c_other)));",
    )
    .answers("r5c_0");
    run.probe(
        "r6v",
        "v.lc = 0.5; v.g = 0; loop(v.lc, {v.g = v.g + 1; v.g > 8 ? {break;} : 0;}); v.g == 0 ? v.r6v_0 : (v.g == 1 ? v.r6v_1 : (v.g == 2 ? v.r6v_2 : (v.g == 9 ? v.r6v_escaped : v.r6v_other)));",
    )
    .answers("r6v_1");
    run.probe(
        "r6c",
        "v.g = 0; loop(0.5, {v.g = v.g + 1; v.g > 8 ? {break;} : 0;}); v.g == 0 ? v.r6c_0 : (v.g == 1 ? v.r6c_1 : (v.g == 2 ? v.r6c_2 : (v.g == 9 ? v.r6c_escaped : v.r6c_other)));",
    )
    .answers("r6c_1");
    run.probe(
        "r7v",
        "v.lc = 1.5; v.g = 0; loop(v.lc, {v.g = v.g + 1; v.g > 8 ? {break;} : 0;}); v.g == 0 ? v.r7v_0 : (v.g == 1 ? v.r7v_1 : (v.g == 2 ? v.r7v_2 : (v.g == 9 ? v.r7v_escaped : v.r7v_other)));",
    )
    .answers("r7v_2");
    run.probe(
        "r7c",
        "v.g = 0; loop(1.5, {v.g = v.g + 1; v.g > 8 ? {break;} : 0;}); v.g == 0 ? v.r7c_0 : (v.g == 1 ? v.r7c_1 : (v.g == 2 ? v.r7c_2 : (v.g == 9 ? v.r7c_escaped : v.r7c_other)));",
    )
    .answers("r7c_2");
    run.probe(
        "r8v",
        "v.lc = 3e9; v.g = 0; loop(v.lc, {v.g = v.g + 1; v.g > 8 ? {break;} : 0;}); v.g == 0 ? v.r8v_0 : (v.g == 1 ? v.r8v_1 : (v.g == 2 ? v.r8v_2 : (v.g == 9 ? v.r8v_escaped : v.r8v_other)));",
    )
    .answers("r8v_escaped");
    run.probe(
        "r8c",
        "v.g = 0; loop(3e9, {v.g = v.g + 1; v.g > 8 ? {break;} : 0;}); v.g == 0 ? v.r8c_0 : (v.g == 1 ? v.r8c_1 : (v.g == 2 ? v.r8c_2 : (v.g == 9 ? v.r8c_escaped : v.r8c_other)));",
    )
    .answers("r8c_escaped");
    run.probe(
        "d1",
        "v.dc = math.sqrt(-1); v.r = math.die_roll(v.dc, 1, 2); v.r == 0 ? v.d1_0 : (v.r == v.r ? v.d1_rolled : v.d1_nan);",
    )
    .answers("d1_0");
    run.probe("d2", "v.dc = -math.ln(0); v.r = math.die_roll(v.dc, 1, 2); v.r == 0 ? v.d2_0 : (v.r == v.r ? v.d2_rolled : v.d2_nan);")
        .answers("d2_0");
    run.probe("d3", "v.dc = 3e9; v.r = math.die_roll(v.dc, 1, 2); v.r == 0 ? v.d3_0 : (v.r == v.r ? v.d3_rolled : v.d3_nan);")
        .answers("d3_0");
    run.probe("d4", "v.dc = 2147483648; v.r = math.die_roll(v.dc, 1, 2); v.r == 0 ? v.d4_0 : (v.r == v.r ? v.d4_rolled : v.d4_nan);")
        .answers("d4_0");
    run.probe("d5", "v.dc = -2147483648; v.r = math.die_roll(v.dc, 1, 2); v.r == 0 ? v.d5_0 : (v.r == v.r ? v.d5_rolled : v.d5_nan);")
        .answers("d5_0");
    run.probe("r99", "v.r99_marker_end;")
        .answers("r99_marker_end");
    run
}

/// NaN, +infinity and 3e9 loop until the escape, −infinity and −0 give no pass, 0.5 one, 1.5 two;
/// `math.die_roll` with such a count is 0.
#[test]
fn odd_loop_and_die_roll_counts_run_34() {
    run_34().replay(23);
}

/// A root `c ? break : x` that logs `unreachable statements after Break` at load.
fn run_35() -> ServerRun {
    let mut run = ServerRun::new(
        "run_35",
        BOTH_RELEASES,
        "whether a root `c ? break : x` that logs `unreachable statements after Break` at load is kept or rejected",
    );
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
    run.probe(
        "r02",
        "v.p1 = math.max(v.f, v.n); v.p1 == 4 ? v.r02_maxnansecond_is4 : (v.p1 == v.p1 ? v.r02_maxnansecond_other : v.r02_maxnansecond_isnan);",
    )
    .answers("r02_maxnansecond_isnan");
    run.probe("b01", "v.x0 = 0; v.b01_set;").answers("b01_set");
    run.probe("b02", "v.x0 ? break : v.b02_kept")
        .load_logs(&["Error: unreachable statements after Break 'break'."])
        .silent();
    run.probe("b99", "v.b99_marker_end;")
        .answers("b99_marker_end");
    run
}

#[test]
fn a_root_conditional_break_logs_at_load_and_prints_nothing_run_35() {
    run_35().replay(6);
}

#[test]
fn a_loop_runs_its_count_and_stops_at_break_or_skips_at_continue() {
    let mut case = EvalCase::new("evaluation-155");
    case.also_on_a_fresh_state();
    case.eval(
        "v.count = 0; loop(3, {v.count = v.count + 1;}); return v.count;",
        3.0,
    );
    case.eval(
        "v.count = 0; loop(3, {v.count = v.count + 1; (v.count == 2) ? break; }); return v.count;",
        2.0,
    );
    case.eval(
        "v.iterations = 0; v.count = 0; loop(3, {v.iterations = v.iterations + 1; (v.count == 1) ? continue; v.count = v.count + 1;}); return v.count + v.iterations;",
        4.0,
    );
    case.check(3);
}

#[test]
fn a_loop_with_a_variable_count_runs_it_and_honours_break_and_continue() {
    let mut case = EvalCase::new("evaluation-156");
    case.also_on_a_fresh_state();
    case.eval("v.loop_count = 3; v.count = 0; loop(v.loop_count, {v.count = v.count + 1;}); return v.count;", 3.0);
    case.eval(
        "v.loop_count = 3; v.count = 0; loop(v.loop_count, {v.count = v.count + 1; (v.count == 2) ? break; }); return v.count;",
        2.0,
    );
    case.eval(
        "v.loop_count = 3; v.iterations = 0; v.count = 0; loop(v.loop_count, {v.iterations = v.iterations + 1; (v.count == 1) ? continue; v.count = v.count + 1;}); return v.count + v.iterations;",
        4.0,
    );
    case.check(3);
}

#[test]
fn a_for_each_over_removed_actors_runs_no_pass() {
    let mut case = EvalCase::new("evaluation-157");
    case.actors(Actors::Removed);
    case.eval(
        "v.count = 0; for_each(v.sheep, v.baa, {v.count = v.count + 1;}); return v.count;",
        0.0,
    );
    case.eval("v.count = 0; for_each(v.sheep, v.baa, {v.count = v.count + 1; (v.count == 1) ? break; }); return v.count;", 0.0);
    case.eval("v.iterations = 0; v.count = 0; for_each(v.sheep, v.baa, {v.iterations = v.iterations + 1; (v.iterations == 1) ? continue; v.count = v.count + 1; (v.count == 1) ? break;}); return v.count + v.iterations;", 0.0);
    case.eval(
        "t.count = 0; for_each(t.sheep, v.baa, {t.count = t.count + 1;}); return t.count;",
        0.0,
    );
    case.eval("t.count = 0; for_each(t.sheep, v.baa, {t.count = t.count + 1; (t.count == 1) ? break; }); return t.count;", 0.0);
    case.eval("t.iterations = 0; t.count = 0; for_each(t.sheep, v.baa, {t.iterations = t.iterations + 1; (t.iterations == 1) ? continue; t.count = t.count + 1; (t.count == 1) ? break;}); return t.count + t.iterations;", 0.0);
    case.check(6);
}

#[test]
fn a_for_each_runs_once_per_live_actor_and_honours_break_and_continue() {
    let mut case = EvalCase::new("evaluation-158");
    case.actors(Actors::Mixed);
    case.eval(
        "v.count = 0; for_each(v.sheep, v.baa, {v.count = v.count + 1;}); return v.count;",
        3.0,
    );
    case.eval(
        "v.iterations = 0; v.count = 0; for_each(v.sheep, v.baa, {v.iterations = v.iterations + 1; (v.count == 1) ? continue; v.count = v.count + 1;}); return v.count + v.iterations;",
        4.0,
    );
    case.eval("v.iterations = 0; v.count = 0; for_each(v.sheep, v.baa, {v.iterations = v.iterations + 1; (v.count == 1) ? continue; v.count = v.count + 1; (v.count == 1) ? break;}); return v.count + v.iterations;", 2.0);
    case.eval(
        "t.count = 0; for_each(t.sheep, v.baa, {t.count = t.count + 1;}); return t.count;",
        3.0,
    );
    case.eval(
        "t.iterations = 0; t.count = 0; for_each(t.sheep, v.baa, {t.iterations = t.iterations + 1; (t.count == 1) ? continue; t.count = t.count + 1;}); return t.count + t.iterations;",
        4.0,
    );
    case.eval("t.iterations = 0; t.count = 0; for_each(t.sheep, v.baa, {t.iterations = t.iterations + 1; (t.count == 1) ? continue; t.count = t.count + 1; (t.count == 1) ? break;}); return t.count + t.iterations;", 2.0);
    case.check(6);
}

/// A count of 2.5 runs 3 passes, 0.5 one, −1 none. The last number of each row counts the passes of
/// all loops together.
#[test]
fn loops_of_up_to_100000_passes_run_every_pass() {
    let mut group = LoopCapGroup::new();
    group.row(
        1,
        "t.i = 0; loop(10, {t.i = t.i + 1;}); return t.i;",
        10,
        10,
    );
    group.row(
        2,
        "t.i = 0; loop(1024, {t.i = t.i + 1;}); return t.i;",
        1024,
        1024,
    );
    group.row(
        3,
        "t.i = 0; loop(1025, {t.i = t.i + 1;}); return t.i;",
        1025,
        1025,
    );
    group.row(
        4,
        "t.i = 0; loop(5000, {t.i = t.i + 1;}); return t.i;",
        5000,
        5000,
    );
    group.row(
        5,
        "t.i = 0; loop(5001, {t.i = t.i + 1;}); return t.i;",
        5001,
        5001,
    );
    group.row(
        6,
        "t.i = 0; loop(10000, {t.i = t.i + 1;}); return t.i;",
        10000,
        10000,
    );
    group.row(
        7,
        "t.i = 0; loop(100000, {t.i = t.i + 1;}); return t.i;",
        100000,
        100000,
    );
    group.row(
        8,
        "t.i = 0; loop(100, {loop(100, {t.i = t.i + 1;});}); return t.i;",
        10000,
        10100,
    );
    group.row(
        9,
        "t.i = 0; loop(300, {loop(300, {t.i = t.i + 1;});}); return t.i;",
        90000,
        90300,
    );
    group.row(
        10,
        "t.i = 0; loop(2.5, {t.i = t.i + 1;}); return t.i;",
        3,
        3,
    );
    group.row(
        11,
        "t.i = 0; loop(0.5, {t.i = t.i + 1;}); return t.i;",
        1,
        1,
    );
    group.row(12, "t.i = 0; loop(-1, {t.i = t.i + 1;}); return t.i;", 0, 0);
    group
        .row(
            13,
            "t.i = 0; loop(math.sqrt(-1), {t.i = t.i + 1;}); return t.i;",
            0,
            0,
        )
        .x86_64_runs_until_the_step_budget();
    group.row(
        14,
        "v.n = 0; loop(5000, { v.n = v.n + 1; }); return v.n;",
        5000,
        5000,
    );
    group.check(14);
}
