//! Variables, temps, structs and members, `context`, actors and `->`, and `this`.

#![cfg(all(feature = "compiler", feature = "stdlib"))]
// Expected values are written as the shortest decimal that gives their bits; some are close to a
// constant.
#![allow(clippy::approx_constant)]

mod common;

use common::measured::*;

/// `query.head_is_in_water` always resolves: a compile cannot name the release that lacks it.
const NOT_IN_1_26_36: &str = "query.head_is_in_water is not in 1.26.36.1";
/// A missing read in a query argument under an outer `??` is logged, the argument is 0, and the
/// next entry runs.
const MISS_IN_QUERY_ARGUMENT: &str = "missing read in a query argument under an outer ??";

/// At Molang version 2: what an assignment inside an operand stores, and `for_each` over an unset
/// variable.
fn run_19() -> ServerRun {
    let mut run = ServerRun::new(
        "run_19",
        FIRST_RELEASE,
        "Molang version 2: assignment value under a post-op, for_each over an unset variable",
    );
    run.arm64_differs(&["r02"]);
    run.probe("r16", "t.seen = 1; v.missing_never_set_r16; t.seen = 2;")
        .at(2)
        .answers("missing_never_set_r16");
    run.probe("r17", "t.seen == 1 ? v.r17_abort_and_temp_persisted : (t.seen == 2 ? v.r17_no_abort : v.r17_other);")
        .at(2)
        .answers("r17_abort_and_temp_persisted");
    run.probe(
        "r01",
        "v.n = math.sqrt(-1); v.f = 4; v.p1 = math.max(v.n, v.f); v.p1 == 4 ? v.r01_maxnanfirst_is4 : (v.p1 == v.p1 ? v.r01_maxnanfirst_other : v.r01_maxnanfirst_isnan);",
    )
    .at(2)
    .answers("r01_maxnanfirst_is4");
    run.probe(
        "r02",
        "v.p1 = math.max(v.f, v.n); v.p1 == 4 ? v.r02_maxnansecond_is4 : (v.p1 == v.p1 ? v.r02_maxnansecond_other : v.r02_maxnansecond_isnan);",
    )
    .at(2)
    .answers("r02_maxnansecond_isnan");
    run.probe("x24", "v.one = 1; (v.foo = v.one) + 2; v.foo == 1 ? v.x24_assign_raw : (v.foo == 3 ? v.x24_assign_post : v.x24_other);")
        .at(2)
        .answers("x24_assign_raw");
    run.probe("x24c", "(v.foo24c = 1) + 2; v.foo24c == 1 ? v.x24c_assign_raw : (v.foo24c == 3 ? v.x24c_assign_post : v.x24c_other);")
        .at(2)
        .answers("x24c_assign_raw");
    run.probe(
        "x24m",
        "v.two24 = 2; (v.foo24m = v.two24) * 3; v.foo24m == 2 ? v.x24m_assign_raw : (v.foo24m == 6 ? v.x24m_assign_post : v.x24m_other);",
    )
    .at(2)
    .answers("x24m_assign_raw");
    run.probe(
        "x14v2",
        "v.c = 1; for_each(v.s, v.x14v2_never_set_array, {v.c = v.c + 1;}); v.c == 1 ? v.x14v2_foreach_unset_no_abort : v.x14v2_other;",
    )
    .at(2)
    .answers("x14v2_never_set_array");
    run.probe(
        "x14f",
        "v.count = 1; for_each(v.sheep, v.x14f_baa, {v.count = v.count + 1;}) + 1; v.count == 1 ? v.x14f_no_abort : v.x14f_other;",
    )
    .at(2)
    .answers("x14f_baa");
    run
}

/// `for_each` over an unset variable aborts the expression with the unknown-variable message.
#[test]
fn an_assignment_inside_an_operand_stores_the_raw_value_at_version_2_run_19() {
    run_19().replay(9);
}

/// A missing read inside a query argument with no `??`, struct self-nesting and copies, and
/// `for_each` over an unset variable at version 13.
fn run_24() -> ServerRun {
    let mut run = ServerRun::new(
        "run_24",
        FIRST_RELEASE,
        "the part of run_18 after x15, which crashed the server in run_18",
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
    run.probe(
        "x15n",
        "t.d15n = 1; v.r15n = 3; v.r15n = q.log(v.x15n_never_set_arg); t.d15n = 2; v.r15n == 0 ? v.x15n_arg_zero : (v.r15n == 3 ? v.x15n_unassigned : v.x15n_other);",
    )
    .continues_after_miss()
    .logs(&[miss("x15n_never_set_arg"), miss("x15n_arg_zero")]);
    run.probe("w15n", "v.k15n = t.d15n ?? 0; v.k15n == 2 ? v.w15n_completed : (v.k15n == 1 ? v.w15n_ended_inside : v.w15n_not_run);")
        .answers("w15n_completed");
    run.probe("x50", "v.s50.c = 1; loop(3, { v.s50.b = v.s50; }); v.d50 = (v.s50.b.b.b.c ?? 0); v.e50 = (v.s50.b.b.b.b.c ?? 0); v.d50 == 1 ? (v.e50 == 0 ? v.x50_depth3 : (v.e50 == 1 ? v.x50_deeper_or_cyclic : v.x50_d1_other)) : (v.d50 == 0 ? v.x50_not_nested : v.x50_other);").load_logs(&["Error: found left-hand-side of ?? expression that isn't a direct-variable reference - this is unsupported at this time.", "Error: found left-hand-side of ?? expression that isn't a direct-variable reference - this is unsupported at this time."]).logs(&[text("Error: unable to find member variable .c"), miss("x50_depth3")]);
    run.probe("x50v", "v.t50 = v.s50; v.t50.c = 5; v.s50.c == 1 ? v.x50v_byvalue : (v.s50.c == 5 ? v.x50v_shared : v.x50v_other);")
        .answers("x50v_byvalue");
    run.probe("x50d", "v.u50.c = 1; loop(20, { v.u50.b = v.u50; }); v.d50d = (v.u50.b.b.b.b.b.b.b.b.b.b.b.b.b.b.b.b.b.b.b.b.c ?? 0); v.d50d == 1 ? v.x50d_depth20_ok : (v.d50d == 0 ? v.x50d_depth20_missing : v.x50d_other);").load_logs(&["Error: found left-hand-side of ?? expression that isn't a direct-variable reference - this is unsupported at this time."]).answers("x50d_depth20_ok");
    run.probe(
        "x14",
        "v.c = 1; for_each(v.s, v.x14_never_set_array, {v.c = v.c + 1;}); v.c == 1 ? v.x14_foreach_unset_no_abort : v.x14_other;",
    )
    .answers("x14_never_set_array");
    run
}

/// A self-nesting assignment in a loop builds one level per pass; a missing read inside a query
/// argument is logged, the call gives 0 and the expression runs on.
#[test]
fn structs_are_copied_and_a_missing_query_argument_reads_as_zero_run_24() {
    run_24().replay(9);
}

/// A missing read inside a query argument under an outer `??`, between markers, with two controls.
fn run_25() -> ServerRun {
    let mut run = ServerRun::new(
        "run_25",
        FIRST_RELEASE,
        "isolating the run_18 crash: the probe of a miss in a query argument under an outer ??, between markers, with two controls",
    );
    run.arm64_differs(&["r02", "x15", "m15b"]);
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
    run.probe("x15k", "v.y15 = 2; v.r = q.log(v.y15) ?? 7; v.r == 2 ? v.x15k_ok : (v.r == 7 ? v.x15k_handler : v.x15k_other);")
        .load_logs(&["Error: found left-hand-side of ?? expression that isn't a direct-variable reference - this is unsupported at this time."])
        .answers("x15k_ok");
    run.probe(
        "x15i",
        "v.r = q.log(v.x15i_never_set ?? 3); v.r == 3 ? v.x15i_inner_handler : v.x15i_other;",
    )
    .continues_after_miss()
    .answers("x15i_inner_handler");
    run.probe("m15a", "v.m15a_before_x15;")
        .answers("m15a_before_x15");
    run.probe(
        "x15",
        "v.r = q.log(v.x15_never_set_arg) ?? 7; v.r == 7 ? v.x15_outer_handler : (v.r == 0 ? v.x15_arg_zero : v.x15_other);",
    )
    .continues_after_miss()
    .load_logs(&["Error: found left-hand-side of ?? expression that isn't a direct-variable reference - this is unsupported at this time."])
    .silent()
    .inconclusive()
    .expected_failure(
        Release::V1_26_36_1,
        MISS_IN_QUERY_ARGUMENT,
        &[
            "Error: found left-hand-side of ?? expression that isn't a direct-variable reference - this is unsupported at this time.",
            "Error: unhandled request for unknown variable 'variable.x15_never_set_arg'",
            "Error: unhandled request for unknown variable 'variable.x15_arg_zero'",
        ],
    );
    run.probe("m15b", "v.m15b_after_x15;")
        .silent()
        .inconclusive()
        .expected_failure(
            Release::V1_26_36_1,
            MISS_IN_QUERY_ARGUMENT,
            &["Error: unhandled request for unknown variable 'variable.m15b_after_x15'"],
        );
    run
}

#[test]
fn a_missing_read_in_a_query_argument_under_an_outer_coalescing_has_no_answer_run_25() {
    run_25().replay(8);
}

/// The sign of a negated NaN, folds of a zero scale, a statement list in a math call, member writes
/// through a float, newer queries and a cancelled sum in other operand positions.
fn run_31() -> ServerRun {
    let mut run = ServerRun::new(
        "run_31",
        BOTH_RELEASES,
        "the sign of a negated NaN, folds of a cancelled multiply, a statement list in a math call, for_each over a number, a root `c ? break : x`, member writes through a float, the queries added in 1.26.50, a cancelled sum in other operand positions",
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
    run.probe("t01", "v.n = math.sqrt(-1); v.a = math.copy_sign(1, -v.n); v.b = math.copy_sign(1, -math.sqrt(-1)); v.a == 1 ? (v.b == 1 ? v.t01_rt_pos_fold_pos : v.t01_rt_pos_fold_neg) : (v.b == 1 ? v.t01_rt_neg_fold_pos : v.t01_rt_neg_fold_neg);").answers("t01_rt_neg_fold_pos");
    run.probe("t02", "v.z = 0; v.n = math.sqrt(-1); v.a = math.copy_sign(1, v.z - v.n); v.b = math.copy_sign(1, 0 - math.sqrt(-1)); v.a == 1 ? (v.b == 1 ? v.t02_rt_pos_fold_pos : v.t02_rt_pos_fold_neg) : (v.b == 1 ? v.t02_rt_neg_fold_pos : v.t02_rt_neg_fold_neg);").answers("t02_rt_neg_fold_pos");
    run.probe(
        "t03",
        "v.x = 5; v.r = v.x*0+3 + (v.x*0+3); v.r == 0 ? v.t03_zero_scale_drops_offsets : (v.r == 6 ? v.t03_zero_scale_keeps_offsets : (v.r == 3 ? v.t03_three : v.t03_other));",
    )
    .answers("t03_zero_scale_drops_offsets");
    run.probe(
        "t04",
        "v.x = 5; v.r = (v.x*0+3)+v.x*0+3; v.r == 3 ? v.t04_three : (v.r == 6 ? v.t04_six : (v.r == 0 ? v.t04_zero : v.t04_other));",
    )
    .answers("t04_three");
    run.probe(
        "t05",
        "v.x = 5; v.y = 1; v.r = v.y+v.x*0+3+(v.x*0+3); v.r == 4 ? v.t05_four : (v.r == 7 ? v.t05_seven : (v.r == 1 ? v.t05_one : v.t05_other));",
    )
    .answers("t05_four");
    run.probe("t06", "v.a = 0; v.y = 7; v.y = math.abs((v.a = 1;)); v.a == 1 ? (v.y == 0 ? v.t06_a1_y0 : (v.y == 1 ? v.t06_a1_y1 : (v.y == 7 ? v.t06_a1_y7 : v.t06_a1_yother))) : (v.a == 0 ? v.t06_a0 : v.t06_aother);").answers("t06_a1_y0");
    run.probe("t07", "for_each(1, v.a, 1); v.t07_ran;")
        .load_logs(&[
            "Error: for_each requires three parameters - a variable to represent an element of an array, an expression resulting in an array, and an expression to run per element of that array.",
        ])
        .silent();
    run.probe("t08", "v.x ? break : 1")
        .load_logs(&["Error: unreachable statements after Break 'break'."])
        .silent();
    run.probe("t08m", "v.t08m_marker;").answers("t08m_marker");
    run.probe(
        "t09",
        "v.q9 = 5; v.q9.b = 3; v.q9.b == 3 ? v.t09_struct_replaces_float : v.t09_other;",
    )
    .answers("t09_struct_replaces_float");
    run.probe("t10", "v.r = q.head_is_in_water; v.r == 0 ? v.t10_resolves_0 : (v.r == 1 ? v.t10_resolves_1 : v.t10_resolves_other);")
        .load_logs_on(
            Release::V1_26_36_1,
            &[
                "Failed to resolve query query.head_is_in_water.  Either the query does not exist or it is not supported in this context.",
                "unrecognized token: q.head_is_in_water; v.r == 0 ? v.t10_resolves_0 : (v.r == 1 ? v.t10_resolves_1 : v.t10_resolves_other);",
            ],
        )
        .silent_on(Release::V1_26_36_1)
        .answers_on(Release::V1_26_52_3, "t10_resolves_0")
        .expected_failure(Release::V1_26_36_1, NOT_IN_1_26_36, &["Error: unhandled request for unknown variable 'variable.t10_resolves_0'"]);
    run.probe("t11", "v.r = q.has_any_biome_tags('plains'); v.t11_ran;")
        .load_logs(&[
            "Failed to resolve query query.has_any_biome_tags.  Either the query does not exist or it is not supported in this context.",
            "unrecognized token: q.has_any_biome_tags('plains'); v.t11_ran;",
        ])
        .silent();
    run.probe("t12", "v.r = q.has_all_biome_tags('plains'); v.t12_ran;")
        .load_logs(&[
            "Failed to resolve query query.has_all_biome_tags.  Either the query does not exist or it is not supported in this context.",
            "unrecognized token: q.has_all_biome_tags('plains'); v.t12_ran;",
        ])
        .silent();
    run.probe("t13", "v.x2 = 2; v.x3 = 3; v.one = 1; v.t13_setup;")
        .answers("t13_setup");
    run.probe("t14", "v.r = v.one ? (math.abs(((v.x2 + v.x3 + 1) + (-v.x2 - v.x3)))) : 0; v.r == 0 ? v.t14_0 : (v.r == 1 ? v.t14_1 : (v.r == 2 ? v.t14_2 : (v.r == 3 ? v.t14_3 : (v.r == 4 ? v.t14_4 : (v.r == 5 ? v.t14_5 : (v.r == 7 ? v.t14_7 : (v.r == 10 ? v.t14_10 : (v.r == -1 ? v.t14_m1 : (v.r == -2 ? v.t14_m2 : (v.r == 0.5 ? v.t14_half : (v.r == 20 ? v.t14_20 : v.t14_other)))))))))));").answers("t14_1");
    run.probe("t15", "v.r = v.one ? (math.floor(((v.x2 + v.x3 + 1) + (-v.x2 - v.x3)))) : 0; v.r == 0 ? v.t15_0 : (v.r == 1 ? v.t15_1 : (v.r == 2 ? v.t15_2 : (v.r == 3 ? v.t15_3 : (v.r == 4 ? v.t15_4 : (v.r == 5 ? v.t15_5 : (v.r == 7 ? v.t15_7 : (v.r == 10 ? v.t15_10 : (v.r == -1 ? v.t15_m1 : (v.r == -2 ? v.t15_m2 : (v.r == 0.5 ? v.t15_half : (v.r == 20 ? v.t15_20 : v.t15_other)))))))))));").answers("t15_1");
    run.probe("t16", "v.r = v.one ? (((v.x2 + v.x3 + 1) + (-v.x2 - v.x3)) == 2) : 0; v.r == 0 ? v.t16_0 : (v.r == 1 ? v.t16_1 : (v.r == 2 ? v.t16_2 : (v.r == 3 ? v.t16_3 : (v.r == 4 ? v.t16_4 : (v.r == 5 ? v.t16_5 : (v.r == 7 ? v.t16_7 : (v.r == 10 ? v.t16_10 : (v.r == -1 ? v.t16_m1 : (v.r == -2 ? v.t16_m2 : (v.r == 0.5 ? v.t16_half : (v.r == 20 ? v.t16_20 : v.t16_other)))))))))));").answers("t16_0");
    run.probe("t17", "v.r = v.one ? (((v.x2 + v.x3 + 1) + (-v.x2 - v.x3)) == 1) : 0; v.r == 0 ? v.t17_0 : (v.r == 1 ? v.t17_1 : (v.r == 2 ? v.t17_2 : (v.r == 3 ? v.t17_3 : (v.r == 4 ? v.t17_4 : (v.r == 5 ? v.t17_5 : (v.r == 7 ? v.t17_7 : (v.r == 10 ? v.t17_10 : (v.r == -1 ? v.t17_m1 : (v.r == -2 ? v.t17_m2 : (v.r == 0.5 ? v.t17_half : (v.r == 20 ? v.t17_20 : v.t17_other)))))))))));").answers("t17_1");
    run.probe("t18", "v.r = v.one ? (-((v.x2 + v.x3 + 1) + (-v.x2 - v.x3))) : 0; v.r == 0 ? v.t18_0 : (v.r == 1 ? v.t18_1 : (v.r == 2 ? v.t18_2 : (v.r == 3 ? v.t18_3 : (v.r == 4 ? v.t18_4 : (v.r == 5 ? v.t18_5 : (v.r == 7 ? v.t18_7 : (v.r == 10 ? v.t18_10 : (v.r == -1 ? v.t18_m1 : (v.r == -2 ? v.t18_m2 : (v.r == 0.5 ? v.t18_half : (v.r == 20 ? v.t18_20 : v.t18_other)))))))))));").answers("t18_m1");
    run.probe("t19", "v.r = v.one ? (!((v.x2 + v.x3 + 1) + (-v.x2 - v.x3))) : 0; v.r == 0 ? v.t19_0 : (v.r == 1 ? v.t19_1 : (v.r == 2 ? v.t19_2 : (v.r == 3 ? v.t19_3 : (v.r == 4 ? v.t19_4 : (v.r == 5 ? v.t19_5 : (v.r == 7 ? v.t19_7 : (v.r == 10 ? v.t19_10 : (v.r == -1 ? v.t19_m1 : (v.r == -2 ? v.t19_m2 : (v.r == 0.5 ? v.t19_half : (v.r == 20 ? v.t19_20 : v.t19_other)))))))))));").answers("t19_0");
    run.probe("t20", "v.r = v.one ? (((v.x2 + v.x3 + 1) + (-v.x2 - v.x3)) ? 5 : 7) : 0; v.r == 0 ? v.t20_0 : (v.r == 1 ? v.t20_1 : (v.r == 2 ? v.t20_2 : (v.r == 3 ? v.t20_3 : (v.r == 4 ? v.t20_4 : (v.r == 5 ? v.t20_5 : (v.r == 7 ? v.t20_7 : (v.r == 10 ? v.t20_10 : (v.r == -1 ? v.t20_m1 : (v.r == -2 ? v.t20_m2 : (v.r == 0.5 ? v.t20_half : (v.r == 20 ? v.t20_20 : v.t20_other)))))))))));").answers("t20_5");
    run.probe("t21", "v.r = v.one ? (((v.x2 + v.x3 + 1) + (-v.x2 - v.x3)) - 1) : 0; v.r == 0 ? v.t21_0 : (v.r == 1 ? v.t21_1 : (v.r == 2 ? v.t21_2 : (v.r == 3 ? v.t21_3 : (v.r == 4 ? v.t21_4 : (v.r == 5 ? v.t21_5 : (v.r == 7 ? v.t21_7 : (v.r == 10 ? v.t21_10 : (v.r == -1 ? v.t21_m1 : (v.r == -2 ? v.t21_m2 : (v.r == 0.5 ? v.t21_half : (v.r == 20 ? v.t21_20 : v.t21_other)))))))))));").answers("t21_0");
    run.probe("t22", "v.r = v.one ? (math.pow(((v.x2 + v.x3 + 1) + (-v.x2 - v.x3)), 2)) : 0; v.r == 0 ? v.t22_0 : (v.r == 1 ? v.t22_1 : (v.r == 2 ? v.t22_2 : (v.r == 3 ? v.t22_3 : (v.r == 4 ? v.t22_4 : (v.r == 5 ? v.t22_5 : (v.r == 7 ? v.t22_7 : (v.r == 10 ? v.t22_10 : (v.r == -1 ? v.t22_m1 : (v.r == -2 ? v.t22_m2 : (v.r == 0.5 ? v.t22_half : (v.r == 20 ? v.t22_20 : v.t22_other)))))))))));").answers("t22_1");
    run.probe("t23", "v.r = v.one ? (math.clamp(((v.x2 + v.x3 + 1) + (-v.x2 - v.x3)), 0, 5)) : 0; v.r == 0 ? v.t23_0 : (v.r == 1 ? v.t23_1 : (v.r == 2 ? v.t23_2 : (v.r == 3 ? v.t23_3 : (v.r == 4 ? v.t23_4 : (v.r == 5 ? v.t23_5 : (v.r == 7 ? v.t23_7 : (v.r == 10 ? v.t23_10 : (v.r == -1 ? v.t23_m1 : (v.r == -2 ? v.t23_m2 : (v.r == 0.5 ? v.t23_half : (v.r == 20 ? v.t23_20 : v.t23_other)))))))))));").answers("t23_1");
    run.probe("t24", "v.r = v.one ? (math.lerp(0, 10, ((v.x2 + v.x3 + 1) + (-v.x2 - v.x3)))) : 0; v.r == 0 ? v.t24_0 : (v.r == 1 ? v.t24_1 : (v.r == 2 ? v.t24_2 : (v.r == 3 ? v.t24_3 : (v.r == 4 ? v.t24_4 : (v.r == 5 ? v.t24_5 : (v.r == 7 ? v.t24_7 : (v.r == 10 ? v.t24_10 : (v.r == -1 ? v.t24_m1 : (v.r == -2 ? v.t24_m2 : (v.r == 0.5 ? v.t24_half : (v.r == 20 ? v.t24_20 : v.t24_other)))))))))));").answers("t24_10");
    run.probe("t25", "v.r = v.one ? (math.mod(5, ((v.x2 + v.x3 + 1) + (-v.x2 - v.x3)))) : 0; v.r == 0 ? v.t25_0 : (v.r == 1 ? v.t25_1 : (v.r == 2 ? v.t25_2 : (v.r == 3 ? v.t25_3 : (v.r == 4 ? v.t25_4 : (v.r == 5 ? v.t25_5 : (v.r == 7 ? v.t25_7 : (v.r == 10 ? v.t25_10 : (v.r == -1 ? v.t25_m1 : (v.r == -2 ? v.t25_m2 : (v.r == 0.5 ? v.t25_half : (v.r == 20 ? v.t25_20 : v.t25_other)))))))))));").answers("t25_0");
    run.probe("t26", "v.r = v.one ? (((v.x2 + v.x3 + 1) + (-v.x2 - v.x3)) * ((v.x2 + v.x3 + 1) + (-v.x2 - v.x3))) : 0; v.r == 0 ? v.t26_0 : (v.r == 1 ? v.t26_1 : (v.r == 2 ? v.t26_2 : (v.r == 3 ? v.t26_3 : (v.r == 4 ? v.t26_4 : (v.r == 5 ? v.t26_5 : (v.r == 7 ? v.t26_7 : (v.r == 10 ? v.t26_10 : (v.r == -1 ? v.t26_m1 : (v.r == -2 ? v.t26_m2 : (v.r == 0.5 ? v.t26_half : (v.r == 20 ? v.t26_20 : v.t26_other)))))))))));").answers("t26_1");
    run.probe("t27", "v.r = v.one ? (((v.x2 + v.x3 + 1) + (-v.x2 - v.x3)) + ((v.x2 + v.x3 + 1) + (-v.x2 - v.x3))) : 0; v.r == 0 ? v.t27_0 : (v.r == 1 ? v.t27_1 : (v.r == 2 ? v.t27_2 : (v.r == 3 ? v.t27_3 : (v.r == 4 ? v.t27_4 : (v.r == 5 ? v.t27_5 : (v.r == 7 ? v.t27_7 : (v.r == 10 ? v.t27_10 : (v.r == -1 ? v.t27_m1 : (v.r == -2 ? v.t27_m2 : (v.r == 0.5 ? v.t27_half : (v.r == 20 ? v.t27_20 : v.t27_other)))))))))));").answers("t27_2");
    run.probe("t28", "v.r = v.one ? (math.max(((v.x2 + v.x3 + 1) + (-v.x2 - v.x3)), 0)) : 0; v.r == 0 ? v.t28_0 : (v.r == 1 ? v.t28_1 : (v.r == 2 ? v.t28_2 : (v.r == 3 ? v.t28_3 : (v.r == 4 ? v.t28_4 : (v.r == 5 ? v.t28_5 : (v.r == 7 ? v.t28_7 : (v.r == 10 ? v.t28_10 : (v.r == -1 ? v.t28_m1 : (v.r == -2 ? v.t28_m2 : (v.r == 0.5 ? v.t28_half : (v.r == 20 ? v.t28_20 : v.t28_other)))))))))));").answers("t28_1");
    run.probe("t29", "v.r = v.one ? (((v.x2 + v.x3 + 1) + (-v.x2 - v.x3)) / 2) : 0; v.r == 0 ? v.t29_0 : (v.r == 1 ? v.t29_1 : (v.r == 2 ? v.t29_2 : (v.r == 3 ? v.t29_3 : (v.r == 4 ? v.t29_4 : (v.r == 5 ? v.t29_5 : (v.r == 7 ? v.t29_7 : (v.r == 10 ? v.t29_10 : (v.r == -1 ? v.t29_m1 : (v.r == -2 ? v.t29_m2 : (v.r == 0.5 ? v.t29_half : (v.r == 20 ? v.t29_20 : v.t29_other)))))))))));").answers("t29_half");
    run.probe("t30", "v.r = v.one ? (2 / ((v.x2 + v.x3 + 1) + (-v.x2 - v.x3))) : 0; v.r == 0 ? v.t30_0 : (v.r == 1 ? v.t30_1 : (v.r == 2 ? v.t30_2 : (v.r == 3 ? v.t30_3 : (v.r == 4 ? v.t30_4 : (v.r == 5 ? v.t30_5 : (v.r == 7 ? v.t30_7 : (v.r == 10 ? v.t30_10 : (v.r == -1 ? v.t30_m1 : (v.r == -2 ? v.t30_m2 : (v.r == 0.5 ? v.t30_half : (v.r == 20 ? v.t30_20 : v.t30_other)))))))))));").answers("t30_2");
    run
}

/// A negated NaN keeps its sign at run time and loses it when folded; a cancelled sum reads 1 in
/// every operand position.
#[test]
fn a_member_write_through_a_float_makes_a_struct_and_other_follow_ups_run_31() {
    run_31().replay(34);
}

#[test]
fn a_variable_condition_picks_the_branch_by_its_value() {
    let mut case = EvalCase::new("evaluation-149");
    case.set("variable.moo", 1.0);
    case.eval("variable.moo ? 2 : 3", 2.0);
    case.set("variable.moo", 0.0);
    case.eval("variable.moo ? 2 : 3", 3.0);
    case.check(2);
}

#[test]
fn a_written_variable_reads_back() {
    let mut case = EvalCase::new("evaluation-153");
    case.set("variable.moo", 1.0);
    case.eval("variable.moo", 1.0);
    case.eval("variable.moo + 1.1", 2.1);
    case.check(2);
}

#[test]
fn query_count_counts_the_actors_of_an_array_and_one_per_other_argument() {
    let mut case = EvalCase::new("evaluation-154");
    case.actors(Actors::Live);
    case.set("variable.moo", 1.0);
    case.eval("query.count(variable.baa)", 3.0);
    case.eval("query.count(variable.baa, 0)", 4.0);
    case.eval("query.count(variable.baa, 1)", 4.0);
    case.eval("query.count(-1, variable.baa)", 4.0);
    case.eval("query.count(0, variable.baa)", 4.0);
    case.eval("query.count(0, variable.baa, 0)", 5.0);
    case.eval("query.count(1, variable.baa, 0)", 5.0);
    case.eval("query.count(0, variable.baa, 1)", 5.0);
    case.eval("query.count(1, variable.baa, 1)", 5.0);
    case.eval("query.count(1, variable.baa, variable.baa)", 7.0);
    case.eval("query.count(variable.moo)", 1.0);
    case.eval("query.count(variable.moo, 0)", 2.0);
    case.eval("query.count(variable.moo, 1)", 2.0);
    case.eval("query.count(-1, variable.moo)", 2.0);
    case.eval("query.count(0, variable.moo)", 2.0);
    case.eval("query.count(0, variable.moo, 0)", 3.0);
    case.eval("query.count(1, variable.moo, 0)", 3.0);
    case.eval("query.count(0, variable.moo, 1)", 3.0);
    case.eval("query.count(1, variable.moo, 1)", 3.0);
    case.eval("query.count(1, variable.moo, variable.moo)", 3.0);
    case.eval("query.count(1, variable.moo, variable.baa)", 5.0);
    case.check(21);
}

#[test]
fn variables_among_query_arguments_are_read() {
    let mut case = EvalCase::new("evaluation-160");
    case.set("variable.a", 1.0);
    case.set("variable.b", 1.1);
    case.eval("q.sum_test(1, 2, 3) + v.a", 7.0);
    case.eval("q.sum_test(1, 2, 3, v.a)", 7.0);
    case.eval("q.sum_test(1, 2, v.a, 3)", 7.0);
    case.eval("q.sum_test(v.a, 1, 2, 3)", 7.0);
    case.eval("v.a + q.sum_test(1, 2, 3)", 7.0);
    case.eval("q.sum_test(1, 2, 3) + v.a + v.b", 8.1);
    case.eval("q.sum_test(1, 2, v.a, v.b, 3)", 8.1);
    case.check(7);
}

/// A read of the subject's own variable sees the latest value.
#[test]
fn a_read_through_an_arrow_sees_the_public_variable_of_the_last_refresh() {
    let mut case = EvalCase::new("evaluation-161");
    case.set_baby_flag(CaseActor::Second);
    case.set_public("variable.baa", 1.23);
    case.refresh_snapshots();
    case.set("variable.baa", 2.34);
    case.context_actor("context.moo", ContextActor::Live);
    case.eval("context.moo->variable.baa", 1.23);
    case.eval("c.moo->variable.baa", 1.23);
    case.eval("c.moo->v.baa", 1.23);
    case.eval("variable.baa", 2.34);
    case.check(4);

    let mut case = EvalCase::new("evaluation-162");
    case.set_baby_flag(CaseActor::Second);
    case.set_public("variable.baa", 1.23);
    case.refresh_snapshots();
    case.context_actor("context.moo", ContextActor::Live);
    case.eval("context.moo->v.baa + 1", 2.23);
    case.eval("c.moo->v.baa - 1", 0.23);
    case.check(2);

    let mut case = EvalCase::new("evaluation-163");
    case.set_public("variable.baa", 1.23);
    case.refresh_snapshots();
    case.set_public("variable.baa", 2.34);
    case.context_actor("context.moo", ContextActor::Live);
    case.eval("context.moo->variable.baa", 1.23);
    case.refresh_snapshots();
    case.eval("context.moo->variable.baa", 2.34);
    case.check(2);

    let mut case = EvalCase::new("evaluation-164");
    case.set_public("variable.baa", 1.23);
    case.refresh_snapshots();
    case.set_public("variable.baa", 2.34);
    case.eval("variable.baa", 2.34);
    case.refresh_snapshots();
    case.eval("variable.baa", 2.34);
    case.check(2);
}

#[test]
fn an_unset_variable_read_through_an_arrow_reads_as_zero() {
    let mut case = EvalCase::new("evaluation-165");
    case.context_actor("context.moo", ContextActor::Live);
    case.eval(
        "math.abs(context.moo->variable.this_var_does_not_exist_yet + 1) + 1",
        2.0,
    );
    case.check(1);
}

/// Alone, `query.is_baby` asks the subject, which has no actor.
#[test]
fn query_is_baby_through_an_arrow_asks_the_actor_it_points_at() {
    let mut case = EvalCase::new("evaluation-166");
    case.set_baby_flag(CaseActor::Second);
    case.context_actor("context.moo", ContextActor::Live);
    case.eval("context.moo->query.is_baby", 0.0);
    case.eval("context.moo->query.is_baby + 1", 1.0);
    case.eval("context.moo->query.is_baby", 0.0);
    case.set_baby_flag(CaseActor::Live);
    case.eval("context.moo->query.is_baby", 1.0);
    case.eval("context.moo->query.is_baby + 1", 2.0);
    case.eval("context.moo->query.is_baby", 1.0);
    case.check(6);

    let mut case = EvalCase::new("evaluation-169");
    case.set_baby_flag(CaseActor::Second);
    case.set_baby_flag(CaseActor::Live);
    case.context_actor("context.moo", ContextActor::Live);
    case.eval("             q.is_baby +              q.is_baby", 0.0);
    case.eval("context.moo->q.is_baby +              q.is_baby", 1.0);
    case.eval("             q.is_baby + context.moo->q.is_baby", 1.0);
    case.eval("context.moo->q.is_baby + context.moo->q.is_baby", 2.0);
    case.check(4);
}

#[test]
fn a_read_through_a_null_actor_is_zero() {
    let mut case = EvalCase::new("evaluation-167");
    case.set("variable.should_become_zero", 3.0);
    case.context_actor("context.null_actor", ContextActor::Null);
    case.eval(
        "variable.should_become_zero = context.null_actor->variable.foo; return 1;",
        1.0,
    );
    case.eval("variable.should_become_zero", 0.0);
    case.check(2);

    let mut case = EvalCase::new("evaluation-168");
    case.set("variable.should_become_one", 3.0);
    case.context_actor("context.null_actor", ContextActor::Null);
    case.eval(
        "variable.should_become_one = context.null_actor->variable.foo + 1; return 2;",
        2.0,
    );
    case.eval("variable.should_become_one", 1.0);
    case.check(2);
}

#[test]
fn a_temp_reads_back_within_its_expression() {
    let mut case = EvalCase::new("evaluation-170");
    case.also_on_a_fresh_state();
    case.eval("t.x = 1; v.x = 2; v.y = t.x; return v.y;", 1.0);
    case.eval("t.x = 1; v.x = 2; v.y = t.x; return t.x;", 1.0);
    case.check(2);
}

#[test]
fn this_and_returns_inside_nested_conditional_blocks() {
    let mut case = EvalCase::new("evaluation-173");
    case.also_on_a_fresh_state();
    case.eval("return this;", 2.34);
    case.eval("return -this;", -2.34);
    case.eval("return -this * 2;", -4.68);
    case.check(3);

    let mut case = EvalCase::new("evaluation-174");
    case.also_on_a_fresh_state();
    case.eval("v.x = 1.23f; return this;", 2.34);
    case.eval("v.x = 1.23f; return -this;", -2.34);
    case.eval("v.x = 1.23f; return v.x;", 1.23);
    case.eval("v.x = 1.23f; return -v.x;", -1.23);
    case.eval("v.x = 1; v.y = 1; v.x ? (v.y ? {return 3;} : {return 1;}) : (v.y ? {return 2;} : {return 0;}); return 4;", 3.0);
    case.eval("v.x = 0; v.y = 1; v.x ? (v.y ? {return 3;} : {return 1;}) : (v.y ? {return 2;} : {return 0;}); return 4;", 2.0);
    case.eval("v.x = 1; v.y = 0; v.x ? (v.y ? {return 3;} : {return 1;}) : (v.y ? {return 2;} : {return 0;}); return 4;", 1.0);
    case.eval("v.x = 0; v.y = 0; v.x ? (v.y ? {return 3;} : {return 1;}) : (v.y ? {return 2;} : {return 0;}); return 4;", 0.0);
    case.check(8);
}

#[test]
fn struct_members_are_written_and_read_by_their_path() {
    let mut case = EvalCase::new("evaluation-195");
    case.also_on_a_fresh_state();
    case.eval("v.x.x = 1; return v.x.x;", 1.0);
    case.check(1);

    let mut case = EvalCase::new("evaluation-196");
    case.also_on_a_fresh_state();
    case.eval("v.x.x = 1; v.x.y = 2; return v.x.x + v.x.y;", 3.0);
    case.check(1);

    let mut case = EvalCase::new("evaluation-197");
    case.also_on_a_fresh_state();
    case.eval(
        "v.x.x = 1; v.x.y = 2; v.xx = v.x.x; v.xy = v.x.y; return v.xx + v.xy;",
        3.0,
    );
    case.check(1);

    let mut case = EvalCase::new("evaluation-199");
    case.also_on_a_fresh_state();
    case.eval(
        "v.test.a.b.c = 1; v.testabc = v.test.a.b.c; return v.testabc;",
        1.0,
    );
    case.check(1);

    let mut case = EvalCase::new("evaluation-200");
    case.also_on_a_fresh_state();
    case.eval(
        "v.testabc = 2; v.test.a.b.c = v.testabc; return v.test.a.b.c;",
        2.0,
    );
    case.check(1);
}

#[test]
fn assigning_a_struct_copies_its_members() {
    let mut case = EvalCase::new("evaluation-198");
    case.also_on_a_fresh_state();
    case.eval(
        "v.x.x = 1; v.x.y = 2; v.y = v.x; return v.y.x + v.y.y;",
        3.0,
    );
    case.check(1);
}

#[test]
fn struct_members_and_coalesced_reads_on_one_state() {
    let mut group = RunGroup::new("struct_members");
    group
        .row(1, "v.x.x = 1; v.x.y = 2; return v.x.x + v.x.y;", 3.0)
        .clears_variables();
    group.row(
        2,
        "v.x.x = 1; v.x.y = 2; v.xx = v.x.x; v.xy = v.x.y; return v.xx + v.xy;",
        3.0,
    );
    group.row(
        3,
        "v.test.a.b.c = 1; v.testabc = v.test.a.b.c; return v.testabc;",
        1.0,
    );
    group.row(4, "v.x.x = 1; return v.x.x;", 1.0);
    group.row(
        5,
        "v.testabc = 2; v.test.a.b.c = v.testabc; return v.test.a.b.c;",
        2.0,
    );
    group.row(
        6,
        "variable.a = 0.1; variable.b = 0.2; return (variable.a ?? 2) + (variable.b ?? 3);",
        0.3,
    );
    group.row(
        7,
        "                                    return (variable.a ?? 2) + (variable.b ?? 3);",
        0.3,
    );
    group.check(7);
}

#[test]
fn a_struct_assigned_to_another_variable_is_copied() {
    let mut group = RunGroup::new("struct_copy");
    group
        .row(
            1,
            "v.x.x = 1; v.x.y = 2; v.y = v.x; return v.y.x + v.y.y;",
            3.0,
        )
        .clears_variables();
    group.row(2, "math.min_angle(90.0)", 90.0);
    group.check(2);
}
