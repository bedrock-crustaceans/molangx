//! Queries: query calls, the helper queries, the string hashes a query returns, which queries
//! resolve in an animation controller, and the Molang version a controller's `format_version`
//! selects.

#![cfg(all(feature = "compiler", feature = "stdlib"))]
// Expected values are written as the shortest decimal that gives their bits; some are close to a
// constant.
#![allow(clippy::approx_constant)]

mod common;

use common::measured::*;
use common::{HASH_OF_MOO, HASH_OF_RABBIT};

/// `query.is_on_screen`, `query.fuse_time` and `query.client_memory_tier` in an animation
/// controller.
fn run_07() -> ServerRun {
    let mut run = ServerRun::new(
        "run_07",
        FIRST_RELEASE,
        "which queries resolve in an animation controller: query.is_on_screen, query.fuse_time, query.client_memory_tier",
    );
    run.arm64_differs(&[]);
    run.probe("w01", "v.p1 = q.is_on_screen; v.w01_is_on_screen_resolved;")
        .load_logs(&[
            "Failed to resolve query query.is_on_screen.  Either the query does not exist or it is not supported in this context.",
            "unrecognized token: q.is_on_screen; v.w01_is_on_screen_resolved;",
        ])
        .silent();
    run.probe("w02", "v.p1 = q.fuse_time; v.w02_fuse_time_resolved;")
        .answers("w02_fuse_time_resolved");
    run.probe(
        "w03",
        "v.p1 = q.client_memory_tier; v.w03_client_memory_tier_resolved;",
    )
    .logs(&[
        text("Error: client_memory_tier isn't supported on the server (headless mode)."),
        miss("w03_client_memory_tier_resolved"),
    ]);
    run.probe("w04", "v.w04_marker_after;")
        .answers("w04_marker_after");
    run
}

#[test]
fn is_on_screen_does_not_resolve_and_client_memory_tier_logs_a_headless_message_run_07() {
    run_07().replay(4);
}

/// A controller with `format_version` "1.20.0" in a pack whose manifest says 1.18.20. In `v01`, `5
/// / v.h` (h = −1) is −5 from version 7 and 5 below; `query.is_baby` resolves at every valid
/// version, `query.is_scenting` up to 10 and `query.has_block_property` up to 9. The controller's
/// `format_version` overrides every `.at(6)`.
fn run_36() -> ServerRun {
    let mut run = ServerRun::new(
        "run_36",
        BOTH_RELEASES,
        r#"the Molang version an animation controller with format_version "1.20.0" runs at (the manifest says 1.18.20; with it, the controller's format_version decides)"#,
    );
    run.controller_format_version("1.20.0");
    run.probe(
        "r18",
        "v.p1 = math.sign(0); v.p1 == 1 ? v.r18_sign0_is1 : v.r18_sign0_other;",
    )
    .at(6)
    .answers("r18_sign0_is1");
    run.probe(
        "r01",
        "v.n = math.sqrt(-1); v.f = 4; v.p1 = math.max(v.n, v.f); v.p1 == 4 ? v.r01_maxnanfirst_is4 : (v.p1 == v.p1 ? v.r01_maxnanfirst_other : v.r01_maxnanfirst_isnan);",
    )
    .at(6)
    .answers("r01_maxnanfirst_is4");
    run.probe(
        "r02",
        "v.p1 = math.max(v.f, v.n); v.p1 == 4 ? v.r02_maxnansecond_is4 : (v.p1 == v.p1 ? v.r02_maxnansecond_other : v.r02_maxnansecond_isnan);",
    )
    .at(6)
    .answers("r02_maxnansecond_isnan");
    run.probe("v01", "v.h = -1; v.d = 5 / v.h; v.d == -5 ? v.v01_v7up : (v.d == 5 ? v.v01_v6down : v.v01_other);")
        .at(6)
        .answers("v01_v7up");
    run.probe("v02", "v.q = q.is_baby; v.v02_resolved;")
        .at(6)
        .answers("v02_resolved");
    run.probe("v03", "v.q = q.is_scenting; v.v03_resolved;")
        .at(6)
        .answers("v03_resolved");
    run.probe("v04", "v.q = q.has_block_property('x'); v.v04_resolved;")
        .at(6)
        .logs(&[
            text("Error: query.has_block_property does not have a block."),
            miss("v04_resolved"),
        ]);
    run.probe("v99", "v.v99_marker_end;")
        .at(6)
        .answers("v99_marker_end");
    run
}

#[test]
fn format_version_1_20_0_selects_a_version_from_7_to_9_run_36() {
    run_36().replay(8);
}

/// A controller with `format_version` "1.020.0".
fn run_37() -> ServerRun {
    let mut run = ServerRun::new(
        "run_37",
        BOTH_RELEASES,
        r#"the Molang version an animation controller with format_version "1.020.0" runs at (the manifest says 1.18.20; with it, the controller's format_version decides)"#,
    );
    run.controller_format_version("1.020.0");
    run.probe(
        "r18",
        "v.p1 = math.sign(0); v.p1 == 1 ? v.r18_sign0_is1 : v.r18_sign0_other;",
    )
    .at(6)
    .not_run();
    run.probe(
        "r01",
        "v.n = math.sqrt(-1); v.f = 4; v.p1 = math.max(v.n, v.f); v.p1 == 4 ? v.r01_maxnanfirst_is4 : (v.p1 == v.p1 ? v.r01_maxnanfirst_other : v.r01_maxnanfirst_isnan);",
    )
    .at(6)
    .not_run();
    run.probe(
        "r02",
        "v.p1 = math.max(v.f, v.n); v.p1 == 4 ? v.r02_maxnansecond_is4 : (v.p1 == v.p1 ? v.r02_maxnansecond_other : v.r02_maxnansecond_isnan);",
    )
    .at(6)
    .not_run();
    run.probe("v01", "v.h = -1; v.d = 5 / v.h; v.d == -5 ? v.v01_v7up : (v.d == 5 ? v.v01_v6down : v.v01_other);")
        .at(6)
        .not_run();
    run.probe("v02", "v.q = q.is_baby; v.v02_resolved;")
        .at(6)
        .not_run();
    run.probe("v03", "v.q = q.is_scenting; v.v03_resolved;")
        .at(6)
        .not_run();
    run.probe("v04", "v.q = q.has_block_property('x'); v.v04_resolved;")
        .at(6)
        .not_run();
    run.probe("v99", "v.v99_marker_end;").at(6).not_run();
    run
}

#[test]
fn a_controller_with_format_version_1_020_0_is_skipped_run_37() {
    run_37().replay(8);
}

/// A controller with `format_version` "01.20.0".
fn run_38() -> ServerRun {
    let mut run = ServerRun::new(
        "run_38",
        BOTH_RELEASES,
        r#"the Molang version an animation controller with format_version "01.20.0" runs at (the manifest says 1.18.20; with it, the controller's format_version decides)"#,
    );
    run.controller_format_version("01.20.0");
    run.probe(
        "r18",
        "v.p1 = math.sign(0); v.p1 == 1 ? v.r18_sign0_is1 : v.r18_sign0_other;",
    )
    .at(6)
    .not_run();
    run.probe(
        "r01",
        "v.n = math.sqrt(-1); v.f = 4; v.p1 = math.max(v.n, v.f); v.p1 == 4 ? v.r01_maxnanfirst_is4 : (v.p1 == v.p1 ? v.r01_maxnanfirst_other : v.r01_maxnanfirst_isnan);",
    )
    .at(6)
    .not_run();
    run.probe(
        "r02",
        "v.p1 = math.max(v.f, v.n); v.p1 == 4 ? v.r02_maxnansecond_is4 : (v.p1 == v.p1 ? v.r02_maxnansecond_other : v.r02_maxnansecond_isnan);",
    )
    .at(6)
    .not_run();
    run.probe("v01", "v.h = -1; v.d = 5 / v.h; v.d == -5 ? v.v01_v7up : (v.d == 5 ? v.v01_v6down : v.v01_other);")
        .at(6)
        .not_run();
    run.probe("v02", "v.q = q.is_baby; v.v02_resolved;")
        .at(6)
        .not_run();
    run.probe("v03", "v.q = q.is_scenting; v.v03_resolved;")
        .at(6)
        .not_run();
    run.probe("v04", "v.q = q.has_block_property('x'); v.v04_resolved;")
        .at(6)
        .not_run();
    run.probe("v99", "v.v99_marker_end;").at(6).not_run();
    run
}

#[test]
fn a_controller_with_format_version_01_20_0_is_skipped_run_38() {
    run_38().replay(8);
}

/// A controller with `format_version` "1.19.50".
fn run_39() -> ServerRun {
    let mut run = ServerRun::new(
        "run_39",
        BOTH_RELEASES,
        r#"the Molang version an animation controller with format_version "1.19.50" runs at (the manifest says 1.18.20; with it, the controller's format_version decides)"#,
    );
    run.controller_format_version("1.19.50");
    run.probe(
        "r18",
        "v.p1 = math.sign(0); v.p1 == 1 ? v.r18_sign0_is1 : v.r18_sign0_other;",
    )
    .at(6)
    .answers("r18_sign0_is1");
    run.probe(
        "r01",
        "v.n = math.sqrt(-1); v.f = 4; v.p1 = math.max(v.n, v.f); v.p1 == 4 ? v.r01_maxnanfirst_is4 : (v.p1 == v.p1 ? v.r01_maxnanfirst_other : v.r01_maxnanfirst_isnan);",
    )
    .at(6)
    .answers("r01_maxnanfirst_is4");
    run.probe(
        "r02",
        "v.p1 = math.max(v.f, v.n); v.p1 == 4 ? v.r02_maxnansecond_is4 : (v.p1 == v.p1 ? v.r02_maxnansecond_other : v.r02_maxnansecond_isnan);",
    )
    .at(6)
    .answers("r02_maxnansecond_isnan");
    run.probe("v01", "v.h = -1; v.d = 5 / v.h; v.d == -5 ? v.v01_v7up : (v.d == 5 ? v.v01_v6down : v.v01_other);")
        .at(6)
        .answers("v01_v6down");
    run.probe("v02", "v.q = q.is_baby; v.v02_resolved;")
        .at(6)
        .answers("v02_resolved");
    run.probe("v03", "v.q = q.is_scenting; v.v03_resolved;")
        .at(6)
        .answers("v03_resolved");
    run.probe("v04", "v.q = q.has_block_property('x'); v.v04_resolved;")
        .at(6)
        .logs(&[
            text("Error: query.has_block_property does not have a block."),
            miss("v04_resolved"),
        ]);
    run.probe("v99", "v.v99_marker_end;")
        .at(6)
        .answers("v99_marker_end");
    run
}

#[test]
fn format_version_1_19_50_selects_a_version_below_7_run_39() {
    run_39().replay(8);
}

/// A controller with `format_version` "1.19.60-beta".
fn run_40() -> ServerRun {
    let mut run = ServerRun::new(
        "run_40",
        BOTH_RELEASES,
        r#"the Molang version an animation controller with format_version "1.19.60-beta" runs at (the manifest says 1.18.20; with it, the controller's format_version decides)"#,
    );
    run.controller_format_version("1.19.60-beta");
    run.probe(
        "r18",
        "v.p1 = math.sign(0); v.p1 == 1 ? v.r18_sign0_is1 : v.r18_sign0_other;",
    )
    .at(6)
    .answers("r18_sign0_is1");
    run.probe(
        "r01",
        "v.n = math.sqrt(-1); v.f = 4; v.p1 = math.max(v.n, v.f); v.p1 == 4 ? v.r01_maxnanfirst_is4 : (v.p1 == v.p1 ? v.r01_maxnanfirst_other : v.r01_maxnanfirst_isnan);",
    )
    .at(6)
    .answers("r01_maxnanfirst_is4");
    run.probe(
        "r02",
        "v.p1 = math.max(v.f, v.n); v.p1 == 4 ? v.r02_maxnansecond_is4 : (v.p1 == v.p1 ? v.r02_maxnansecond_other : v.r02_maxnansecond_isnan);",
    )
    .at(6)
    .answers("r02_maxnansecond_isnan");
    run.probe("v01", "v.h = -1; v.d = 5 / v.h; v.d == -5 ? v.v01_v7up : (v.d == 5 ? v.v01_v6down : v.v01_other);")
        .at(6)
        .answers("v01_v6down");
    run.probe("v02", "v.q = q.is_baby; v.v02_resolved;")
        .at(6)
        .answers("v02_resolved");
    run.probe("v03", "v.q = q.is_scenting; v.v03_resolved;")
        .at(6)
        .answers("v03_resolved");
    run.probe("v04", "v.q = q.has_block_property('x'); v.v04_resolved;")
        .at(6)
        .logs(&[
            text("Error: query.has_block_property does not have a block."),
            miss("v04_resolved"),
        ]);
    run.probe("v99", "v.v99_marker_end;")
        .at(6)
        .answers("v99_marker_end");
    run
}

#[test]
fn format_version_1_19_60_beta_selects_a_version_below_7_run_40() {
    run_40().replay(8);
}

/// A controller with `format_version` "1.19.60-01".
fn run_41() -> ServerRun {
    let mut run = ServerRun::new(
        "run_41",
        BOTH_RELEASES,
        r#"the Molang version an animation controller with format_version "1.19.60-01" runs at (the manifest says 1.18.20; with it, the controller's format_version decides)"#,
    );
    run.controller_format_version("1.19.60-01");
    run.probe(
        "r18",
        "v.p1 = math.sign(0); v.p1 == 1 ? v.r18_sign0_is1 : v.r18_sign0_other;",
    )
    .at(6)
    .not_run();
    run.probe(
        "r01",
        "v.n = math.sqrt(-1); v.f = 4; v.p1 = math.max(v.n, v.f); v.p1 == 4 ? v.r01_maxnanfirst_is4 : (v.p1 == v.p1 ? v.r01_maxnanfirst_other : v.r01_maxnanfirst_isnan);",
    )
    .at(6)
    .not_run();
    run.probe(
        "r02",
        "v.p1 = math.max(v.f, v.n); v.p1 == 4 ? v.r02_maxnansecond_is4 : (v.p1 == v.p1 ? v.r02_maxnansecond_other : v.r02_maxnansecond_isnan);",
    )
    .at(6)
    .not_run();
    run.probe("v01", "v.h = -1; v.d = 5 / v.h; v.d == -5 ? v.v01_v7up : (v.d == 5 ? v.v01_v6down : v.v01_other);")
        .at(6)
        .not_run();
    run.probe("v02", "v.q = q.is_baby; v.v02_resolved;")
        .at(6)
        .not_run();
    run.probe("v03", "v.q = q.is_scenting; v.v03_resolved;")
        .at(6)
        .not_run();
    run.probe("v04", "v.q = q.has_block_property('x'); v.v04_resolved;")
        .at(6)
        .not_run();
    run.probe("v99", "v.v99_marker_end;").at(6).not_run();
    run
}

#[test]
fn a_controller_with_format_version_1_19_60_01_is_skipped_run_41() {
    run_41().replay(8);
}

/// A controller with `format_version` "1.2.3-01".
fn run_42() -> ServerRun {
    let mut run = ServerRun::new(
        "run_42",
        BOTH_RELEASES,
        r#"the Molang version an animation controller with format_version "1.2.3-01" runs at (the manifest says 1.18.20; with it, the controller's format_version decides)"#,
    );
    run.controller_format_version("1.2.3-01");
    run.probe(
        "r18",
        "v.p1 = math.sign(0); v.p1 == 1 ? v.r18_sign0_is1 : v.r18_sign0_other;",
    )
    .at(6)
    .not_run();
    run.probe(
        "r01",
        "v.n = math.sqrt(-1); v.f = 4; v.p1 = math.max(v.n, v.f); v.p1 == 4 ? v.r01_maxnanfirst_is4 : (v.p1 == v.p1 ? v.r01_maxnanfirst_other : v.r01_maxnanfirst_isnan);",
    )
    .at(6)
    .not_run();
    run.probe(
        "r02",
        "v.p1 = math.max(v.f, v.n); v.p1 == 4 ? v.r02_maxnansecond_is4 : (v.p1 == v.p1 ? v.r02_maxnansecond_other : v.r02_maxnansecond_isnan);",
    )
    .at(6)
    .not_run();
    run.probe("v01", "v.h = -1; v.d = 5 / v.h; v.d == -5 ? v.v01_v7up : (v.d == 5 ? v.v01_v6down : v.v01_other);")
        .at(6)
        .not_run();
    run.probe("v02", "v.q = q.is_baby; v.v02_resolved;")
        .at(6)
        .not_run();
    run.probe("v03", "v.q = q.is_scenting; v.v03_resolved;")
        .at(6)
        .not_run();
    run.probe("v04", "v.q = q.has_block_property('x'); v.v04_resolved;")
        .at(6)
        .not_run();
    run.probe("v99", "v.v99_marker_end;").at(6).not_run();
    run
}

#[test]
fn a_controller_with_format_version_1_2_3_01_is_skipped_run_42() {
    run_42().replay(8);
}

/// A controller with `format_version` "1.2.3".
fn run_43() -> ServerRun {
    let mut run = ServerRun::new(
        "run_43",
        BOTH_RELEASES,
        r#"the Molang version an animation controller with format_version "1.2.3" runs at (the manifest says 1.18.20; with it, the controller's format_version decides)"#,
    );
    run.controller_format_version("1.2.3");
    run.probe(
        "r18",
        "v.p1 = math.sign(0); v.p1 == 1 ? v.r18_sign0_is1 : v.r18_sign0_other;",
    )
    .at(6)
    .not_run();
    run.probe(
        "r01",
        "v.n = math.sqrt(-1); v.f = 4; v.p1 = math.max(v.n, v.f); v.p1 == 4 ? v.r01_maxnanfirst_is4 : (v.p1 == v.p1 ? v.r01_maxnanfirst_other : v.r01_maxnanfirst_isnan);",
    )
    .at(6)
    .not_run();
    run.probe(
        "r02",
        "v.p1 = math.max(v.f, v.n); v.p1 == 4 ? v.r02_maxnansecond_is4 : (v.p1 == v.p1 ? v.r02_maxnansecond_other : v.r02_maxnansecond_isnan);",
    )
    .at(6)
    .not_run();
    run.probe("v01", "v.h = -1; v.d = 5 / v.h; v.d == -5 ? v.v01_v7up : (v.d == 5 ? v.v01_v6down : v.v01_other);")
        .at(6)
        .not_run();
    run.probe("v02", "v.q = q.is_baby; v.v02_resolved;")
        .at(6)
        .not_run();
    run.probe("v03", "v.q = q.is_scenting; v.v03_resolved;")
        .at(6)
        .not_run();
    run.probe("v04", "v.q = q.has_block_property('x'); v.v04_resolved;")
        .at(6)
        .not_run();
    run.probe("v99", "v.v99_marker_end;").at(6).not_run();
    run
}

#[test]
fn a_controller_with_format_version_1_2_3_is_skipped_run_43() {
    run_43().replay(8);
}

#[test]
fn get_name_test_returns_the_hash_of_its_literal() {
    let mut case = EvalCase::new("evaluation-130");
    case.hash("query.get_name_test(0)", "moo", HASH_OF_MOO);
    case.hash("query.get_name_test(1)", "rabbit", HASH_OF_RABBIT);
    case.check(2);
}

#[test]
fn sum_test_sums_its_arguments() {
    let mut case = EvalCase::new("evaluation-190");
    case.also_on_a_fresh_state();
    case.eval("query.sum_test(1, 2, 3)", 6.0);
    case.check(1);

    let mut case = EvalCase::new("evaluation-191");
    case.also_on_a_fresh_state();
    case.eval("query.sum_test(query.sum_test(1, 2, 3, 4, 5, 6), 7, 8, query.sum_test(9, 10, 11, query.sum_test(12, 13)))", 91.0);
    case.check(1);

    let mut case = EvalCase::new("evaluation-194");
    case.also_on_a_fresh_state();
    case.eval("q.sum_test(1, 2, 3)", 6.0);
    case.check(1);
}

#[test]
fn experimental_test_needs_its_experiment() {
    let mut case = EvalCase::new("evaluation-180");
    case.also_on_a_fresh_state();
    case.parses("query.experimental_test")
        .at(-1)
        .with_experiment();
    case.eval("query.experimental_test", 1.0);
    case.parse_fails("query.experimental_test")
        .at(-1)
        .because(ParseFailure::UnresolvedQuery);
    case.check(3);
}

#[test]
fn get_name_test_needs_the_helper_set() {
    let mut case = EvalCase::new("evaluation-187");
    case.default_set_only();
    case.parse_fails("query.get_name_test(0)")
        .at(-1)
        .because(ParseFailure::UnresolvedQuery);
    case.check(1);
}

/// Fewer than three arguments compile but log an error when evaluated.
#[test]
fn query_all_is_one_when_every_later_argument_equals_the_first() {
    let mut case = EvalCase::new("query_calls-001");
    case.fails_evaluation(&["query.all", "query.all(1)", "query.all(1,2)"]);
    case.check(3);

    let mut case = EvalCase::new("query_calls-002");
    case.evaluates_to(
        1.0,
        &[
            "query.all(1,1,1)",
            "query.all('test','test','test', 'test')",
            "query.all(3, 1+2, math.max(2,3), math.clamp(1, 3, 10))",
        ],
    );
    case.check(3);

    let mut case = EvalCase::new("query_calls-003");
    case.evaluates_to(
        0.0,
        &[
            "query.all(1,1,0)",
            "query.all(1,0,1)",
            "query.all(1,0,0)",
            "query.all('test',0.5,'test')",
            "q.all(math.max(2,4), 4, 2*2.01)",
        ],
    );
    case.check(5);
}

/// Fewer than three arguments compile but log an error when evaluated.
#[test]
fn query_any_is_one_when_some_later_argument_equals_the_first() {
    let mut case = EvalCase::new("query_calls-004");
    case.fails_evaluation(&["query.any", "query.any(1)", "query.any(1,2)"]);
    case.check(3);

    let mut case = EvalCase::new("query_calls-005");
    case.evaluates_to(
        1.0,
        &[
            "query.any(1,1,1)",
            "query.any('test','a','test', 'foo')",
            "query.any(3, 10+2, math.max(2,3), math.clamp(1, 8, 10))",
        ],
    );
    case.check(3);

    let mut case = EvalCase::new("query_calls-006");
    case.evaluates_to(
        0.0,
        &[
            "query.any(1,11,0)",
            "query.any(1,0,1.1)",
            "query.any(1,0,0)",
            "query.any('test',0.5,'te st')",
            "q.any(math.max(2,4), 3, 2.1*2)",
        ],
    );
    case.check(5);
}

/// Another argument count or a string compiles but logs an error when evaluated.
#[test]
fn query_in_range_checks_three_numbers() {
    let mut case = EvalCase::new("query_calls-007");
    case.fails_evaluation(&[
        "query.in_range",
        "query.in_range(1,2)",
        "query.in_range(1,2,3,4)",
        "query.in_range(1,0.5,'something')",
        "query.in_range(1,'something',3)",
        "query.in_range('something',2,3)",
    ]);
    case.check(6);

    let mut case = EvalCase::new("query_calls-008");
    case.evaluates_to(
        1.0,
        &[
            "query.in_range(1,0,3)",
            "query.in_range(math.sqrt(3), 1, 10)",
            "query.in_range(-1, -3, -0.5)",
        ],
    );
    case.check(3);

    let mut case = EvalCase::new("query_calls-009");
    case.evaluates_to(
        0.0,
        &[
            "query.in_range(5, 1, 3.5)",
            "query.in_range(10, 11, 20)",
            "query.in_range(1,0,0)",
            "query.in_range(-5,-10,-6)",
        ],
    );
    case.check(4);
}
