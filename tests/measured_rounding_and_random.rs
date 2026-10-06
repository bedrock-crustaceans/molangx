//! Rounding and random draws: `ceil`, `floor`, `round`, `clamp`, `copy_sign`, `mod`, `sign`, `min`,
//! `max`, the random functions and dice, and the sign of zero.

#![cfg(all(feature = "compiler", feature = "stdlib"))]
// Expected values are written as the shortest decimal that gives their bits; some are close to a
// constant.
#![allow(clippy::approx_constant)]

mod common;

use common::measured::*;

/// The sign of a zero, read through `math.atan2(x, -1)`: negative for −0, positive for +0.
fn run_12() -> ServerRun {
    let mut run = ServerRun::new(
        "run_12",
        FIRST_RELEASE,
        "the sign of zero: controls for the −0 detector, then the math.mod questions of run_09 again with math.atan2(x, -1) as detector",
    );
    run.checks_load_messages();
    run.arm64_differs(&[]);
    run.probe(
        "z01",
        "v.h = -0.5; v.z = math.ceil(v.h); v.p1 = math.atan2(v.z, -1); v.z == 0 ? (v.p1 < 0 ? v.z01_negzero : (v.p1 > 0 ? v.z01_poszero : v.z01_other)) : v.z01_nonzero;",
    )
    .answers("z01_negzero");
    run.probe(
        "z02",
        "v.h = -0.5; v.p1 = math.atan2(math.ceil(v.h), -1); math.ceil(v.h) == 0 ? (v.p1 < 0 ? v.z02_negzero : (v.p1 > 0 ? v.z02_poszero : v.z02_other)) : v.z02_nonzero;",
    )
    .answers("z02_negzero");
    run.probe(
        "z03",
        "v.zz = 0; v.p1 = math.atan2(v.zz, -1); v.zz == 0 ? (v.p1 < 0 ? v.z03_negzero : (v.p1 > 0 ? v.z03_poszero : v.z03_other)) : v.z03_nonzero;",
    )
    .answers("z03_poszero");
    run.probe(
        "z04",
        "v.nz = math.copy_sign(0, -1); v.p1 = math.atan2(v.nz, -1); v.nz == 0 ? (v.p1 < 0 ? v.z04_negzero : (v.p1 > 0 ? v.z04_poszero : v.z04_other)) : v.z04_nonzero;",
    )
    .answers("z04_poszero");
    run.probe(
        "z05",
        "v.zz = 0; v.m1 = -1; v.nz = math.copy_sign(v.zz, v.m1); v.p1 = math.atan2(v.nz, -1); v.nz == 0 ? (v.p1 < 0 ? v.z05_negzero : (v.p1 > 0 ? v.z05_poszero : v.z05_other)) : v.z05_nonzero;",
    )
    .answers("z05_negzero");
    run.probe(
        "z06",
        "v.h = -0.5; v.z = math.ceil(v.h); v.p1 = math.copy_sign(1, v.z); v.p1 == -1 ? v.z06_copysign_sees_negzero : (v.p1 == 1 ? v.z06_copysign_blind : v.z06_other);",
    )
    .answers("z06_copysign_sees_negzero");
    run.probe(
        "z07",
        "v.zz = 0; v.nz = -v.zz; v.p1 = math.atan2(v.nz, -1); v.nz == 0 ? (v.p1 < 0 ? v.z07_negzero : (v.p1 > 0 ? v.z07_poszero : v.z07_other)) : v.z07_nonzero;",
    )
    .answers("z07_poszero");
    run.probe(
        "z08",
        "v.zz = 0; v.nz = v.zz * -1; v.p1 = math.atan2(v.nz, -1); v.nz == 0 ? (v.p1 < 0 ? v.z08_negzero : (v.p1 > 0 ? v.z08_poszero : v.z08_other)) : v.z08_nonzero;",
    )
    .answers("z08_poszero");
    run.probe(
        "m01",
        "v.a = -3; v.b = 3; v.m = math.mod(v.a, v.b); v.p1 = math.atan2(v.m, -1); v.m == 0 ? (v.p1 < 0 ? v.m01_negzero : (v.p1 > 0 ? v.m01_poszero : v.m01_other)) : v.m01_nonzero;",
    )
    .answers("m01_poszero");
    run.probe(
        "m02",
        "v.a = -3; v.m = math.mod(v.a, 3); v.p1 = math.atan2(v.m, -1); v.m == 0 ? (v.p1 < 0 ? v.m02_negzero : (v.p1 > 0 ? v.m02_poszero : v.m02_other)) : v.m02_nonzero;",
    )
    .answers("m02_poszero");
    run.probe(
        "m03",
        "v.m = math.mod(-4, 2); v.p1 = math.atan2(v.m, -1); v.m == 0 ? (v.p1 < 0 ? v.m03_negzero : (v.p1 > 0 ? v.m03_poszero : v.m03_other)) : v.m03_nonzero;",
    )
    .answers("m03_poszero");
    run.probe(
        "m04",
        "v.m = math.mod(-3, 3); v.p1 = math.atan2(v.m, -1); v.m == 0 ? (v.p1 < 0 ? v.m04_negzero : (v.p1 > 0 ? v.m04_poszero : v.m04_other)) : v.m04_nonzero;",
    )
    .answers("m04_poszero");
    run.probe(
        "m05",
        "v.a = -4; v.b = 2; v.m = math.mod(v.a, v.b); v.p1 = math.atan2(v.m, -1); v.m == 0 ? (v.p1 < 0 ? v.m05_negzero : (v.p1 > 0 ? v.m05_poszero : v.m05_other)) : v.m05_nonzero;",
    )
    .answers("m05_poszero");
    run.probe("z99_marker_end", "v.z99_marker_end;")
        .answers("z99_marker_end");
    run
}

/// A negated or −1-scaled +0 and every `math.mod` zero give +0; a folded call assigned to a
/// variable reads +0.
#[test]
fn run_time_ceil_and_copy_sign_give_negative_zero_and_mod_gives_positive_zero_run_12() {
    run_12().replay(14);
}

#[test]
fn ceil_rounds_up() {
    let mut case = EvalCase::new("evaluation-013");
    case.also_on_a_fresh_state();
    case.eval("math.ceil(0.0f)", 0.0);
    case.eval("math.ceil(1.0f)", 1.0);
    case.eval("math.ceil(-1.0f)", -1.0);
    case.eval("math.ceil((-1.0f))", -1.0);
    case.eval("math.ceil(2.0f)", 2.0);
    case.eval("math.ceil(-2.0f)", -2.0);
    case.eval("math.ceil(0.5f)", 1.0);
    case.eval("math.ceil(-0.5f)", -0.0);
    case.eval("math.ceil(1.5f)", 2.0);
    case.eval("math.ceil(-1.5f)", -1.0);
    case.eval("math.ceil(1.51f)", 2.0);
    case.eval("math.ceil(-1.51f)", -1.0);
    case.eval("math.ceil(1.4f)", 2.0);
    case.eval("math.ceil(-1.4f)", -1.0);
    case.eval("math.ceil(1.4999f)", 2.0);
    case.eval("math.ceil(-1.4999f)", -1.0);
    case.eval("math.ceil(1.000001f)", 2.0);
    case.eval("math.ceil(-1.000001f)", -1.0);
    case.eval("math.ceil(0.99999f)", 1.0);
    case.eval("math.ceil(-0.99999f)", -0.0);
    case.eval("math.ceil(100000.000001f)", 100000.0);
    case.eval("math.ceil(-100000.000001f)", -100000.0);
    case.eval("math.ceil(100000.99999f)", 100001.0);
    case.eval("math.ceil(-100000.99999f)", -100001.0);
    case.check(24);

    let mut case = EvalCase::new("evaluation-014");
    case.also_on_a_fresh_state();
    case.eval("math.ceil(1.1f) + 1", 3.0);
    case.eval("math.ceil(1.1f) * 2", 4.0);
    case.eval("math.ceil(1.1f) * 2 + 1", 5.0);
    case.eval("math.ceil(1.1f) * -2", -4.0);
    case.eval("math.ceil(1.1f) * -2 + 1", -3.0);
    case.eval("math.ceil(1.1f) * -2 - 1", -5.0);
    case.check(6);

    let mut case = EvalCase::new("evaluation-015");
    case.also_on_a_fresh_state();
    case.eval("v.x = 0.0f; return math.ceil(v.x);", 0.0);
    case.eval("v.x = 1.0f; return math.ceil(v.x);", 1.0);
    case.eval("v.x = -1.0f; return math.ceil(v.x);", -1.0);
    case.eval("v.x = 2.0f; return math.ceil(v.x);", 2.0);
    case.eval("v.x = -2.0f; return math.ceil(v.x);", -2.0);
    case.eval("v.x = 0.5f; return math.ceil(v.x);", 1.0);
    case.eval("v.x = -0.5f; return math.ceil(v.x);", -0.0);
    case.eval("v.x = 1.5f; return math.ceil(v.x);", 2.0);
    case.eval("v.x = -1.5f; return math.ceil(v.x);", -1.0);
    case.eval("v.x = 1.51f; return math.ceil(v.x);", 2.0);
    case.eval("v.x = -1.51f; return math.ceil(v.x);", -1.0);
    case.eval("v.x = 1.4f; return math.ceil(v.x);", 2.0);
    case.eval("v.x = -1.4f; return math.ceil(v.x);", -1.0);
    case.eval("v.x = 1.4999f; return math.ceil(v.x);", 2.0);
    case.eval("v.x = -1.4999f; return math.ceil(v.x);", -1.0);
    case.eval("v.x = 1.000001f; return math.ceil(v.x);", 2.0);
    case.eval("v.x = -1.000001f; return math.ceil(v.x);", -1.0);
    case.eval("v.x = 0.99999f; return math.ceil(v.x);", 1.0);
    case.eval("v.x = -0.99999f; return math.ceil(v.x);", -0.0);
    case.eval("v.x = 100000.000001f; return math.ceil(v.x);", 100000.0);
    case.eval("v.x = -100000.000001f; return math.ceil(v.x);", -100000.0);
    case.eval("v.x = 100000.99999f; return math.ceil(v.x);", 100001.0);
    case.eval("v.x = -100000.99999f; return math.ceil(v.x);", -100001.0);
    case.check(23);
}

#[test]
fn clamp_limits_a_value_to_its_bounds() {
    let mut case = EvalCase::new("evaluation-016");
    case.also_on_a_fresh_state();
    case.eval("math.clamp(1.0f, 2.0f, 3.0f)", 2.0);
    case.eval("math.clamp(2.0f, 1.0f, 3.0f)", 2.0);
    case.eval("math.clamp(3.0f, 1.0f, 2.0f)", 2.0);
    case.eval("math.clamp(3.0f, 2.0f, 1.0f)", 1.0);
    case.eval("math.clamp(-1.0f, -2.0f, -3.0f)", -3.0);
    case.eval("math.clamp(-2.0f, -1.0f, -3.0f)", -3.0);
    case.eval("math.clamp(-3.0f, -1.0f, -2.0f)", -1.0);
    case.eval("math.clamp(-3.0f, -2.0f, -1.0f)", -2.0);
    case.check(8);

    let mut case = EvalCase::new("evaluation-017");
    case.also_on_a_fresh_state();
    case.eval("math.clamp(2.1f, 0, 1.1) + 1", 2.1);
    case.eval("math.clamp(2.1f, 0, 1.1) * 2", 2.2);
    case.eval("math.clamp(2.1f, 0, 1.1) * 2 + 1", 3.2);
    case.eval("math.clamp(2.1f, 0, 1.1) * -2", -2.2);
    case.eval("math.clamp(2.1f, 0, 1.1) * -2 + 1", -1.2);
    case.eval("math.clamp(2.1f, 0, 1.1) * -2 - 1", -3.2);
    case.check(6);

    let mut case = EvalCase::new("evaluation-018");
    case.also_on_a_fresh_state();
    case.eval(
        "v.x =  1.0f; v.y =  2.0f; v.z =  3.0f; return math.clamp(v.x, v.y, v.z);",
        2.0,
    );
    case.eval(
        "v.x =  2.0f; v.y =  1.0f; v.z =  3.0f; return math.clamp(v.x, v.y, v.z);",
        2.0,
    );
    case.eval(
        "v.x =  3.0f; v.y =  1.0f; v.z =  2.0f; return math.clamp(v.x, v.y, v.z);",
        2.0,
    );
    case.eval(
        "v.x =  3.0f; v.y =  2.0f; v.z =  1.0f; return math.clamp(v.x, v.y, v.z);",
        1.0,
    );
    case.eval(
        "v.x = -1.0f; v.y = -2.0f; v.z = -3.0f; return math.clamp(v.x, v.y, v.z);",
        -3.0,
    );
    case.eval(
        "v.x = -2.0f; v.y = -1.0f; v.z = -3.0f; return math.clamp(v.x, v.y, v.z);",
        -3.0,
    );
    case.eval(
        "v.x = -3.0f; v.y = -1.0f; v.z = -2.0f; return math.clamp(v.x, v.y, v.z);",
        -1.0,
    );
    case.eval(
        "v.x = -3.0f; v.y = -2.0f; v.z = -1.0f; return math.clamp(v.x, v.y, v.z);",
        -2.0,
    );
    case.eval("v.x = 1.23; v.y = 2.34; return v.x + v.y;", 3.57);
    case.eval(
        "v.x = 1.23; v.y = 2.34; return v.x + v.y + math.pi;",
        6.7115927,
    );
    case.check(10);
}

/// A second operand of 0 counts as positive.
#[test]
fn copy_sign_takes_the_sign_of_the_second_operand() {
    let mut case = EvalCase::new("evaluation-019");
    case.also_on_a_fresh_state();
    case.eval("math.copy_sign(1.0f, 0.0f)", 1.0);
    case.eval("math.copy_sign(0.0f, 1.0f)", 0.0);
    case.eval("math.copy_sign(1.0f, -1.0f)", -1.0);
    case.eval("math.copy_sign(1.0f, (-1.0f))", -1.0);
    case.eval("math.copy_sign(2.0f, 0.0f)", 2.0);
    case.eval("math.copy_sign(0.0f, 2.0f)", 0.0);
    case.eval("math.copy_sign(2.0f, -2.0f)", -2.0);
    case.eval("math.copy_sign(2.0f, (-2.0f))", -2.0);
    case.eval("math.copy_sign(0.5f, 0.0f)", 0.5);
    case.eval("math.copy_sign(0.0f, 0.5f)", 0.0);
    case.eval("math.copy_sign((2.0f), 0.0f)", 2.0);
    case.eval("math.copy_sign((0.0f), 2.0f)", 0.0);
    case.eval("math.copy_sign((2.0f), -2.0f)", -2.0);
    case.eval("math.copy_sign((2.0f), (-2.0f))", -2.0);
    case.check(14);

    let mut case = EvalCase::new("evaluation-020");
    case.also_on_a_fresh_state();
    case.eval("math.copy_sign(-1.1f, 3.1) + 1", 2.1);
    case.eval("math.copy_sign(-1.1f, 3.1) * 2", 2.2);
    case.eval("math.copy_sign(-1.1f, 3.1) * 2 + 1", 3.2);
    case.eval("math.copy_sign(-1.1f, 3.1) * -2", -2.2);
    case.eval("math.copy_sign(-1.1f, 3.1) * -2 + 1", -1.2);
    case.eval("math.copy_sign(-1.1f, 3.1) * -2 - 1", -3.2);
    case.check(6);
}

#[test]
fn floor_rounds_down() {
    let mut case = EvalCase::new("evaluation-027");
    case.also_on_a_fresh_state();
    case.eval("math.floor(0.0f)", 0.0);
    case.eval("math.floor(1.0f)", 1.0);
    case.eval("math.floor(-1.0f)", -1.0);
    case.eval("math.floor((-1.0f))", -1.0);
    case.eval("math.floor(2.0f)", 2.0);
    case.eval("math.floor(-2.0f)", -2.0);
    case.eval("math.floor(0.5f)", 0.0);
    case.eval("math.floor(-0.5f)", -1.0);
    case.eval("math.floor(1.5f)", 1.0);
    case.eval("math.floor(-1.5f)", -2.0);
    case.eval("math.floor(1.51f)", 1.0);
    case.eval("math.floor(-1.51f)", -2.0);
    case.eval("math.floor(1.4f)", 1.0);
    case.eval("math.floor(-1.4f)", -2.0);
    case.eval("math.floor(1.4999f)", 1.0);
    case.eval("math.floor(-1.4999f)", -2.0);
    case.eval("math.floor(1.000001f)", 1.0);
    case.eval("math.floor(-1.000001f)", -2.0);
    case.eval("math.floor(0.99999f)", 0.0);
    case.eval("math.floor(-0.99999f)", -1.0);
    case.eval("math.floor(100000.000001f)", 100000.0);
    case.eval("math.floor(-100000.000001f)", -100000.0);
    case.eval("math.floor(100000.99999f)", 100001.0);
    case.eval("math.floor(-100000.99999f)", -100001.0);
    case.check(24);

    let mut case = EvalCase::new("evaluation-028");
    case.also_on_a_fresh_state();
    case.eval("math.floor(1.9) + 1.0", 2.0);
    case.eval("math.floor(1.9) * 2.2", 2.2);
    case.eval("math.floor(1.9) * 2.2 + 1", 3.2);
    case.eval("math.floor(1.9) * -2.2", -2.2);
    case.eval("math.floor(1.9) * -2.2 + 1", -1.2);
    case.eval("math.floor(1.9) * -2.2 - 1", -3.2);
    case.check(6);

    let mut case = EvalCase::new("evaluation-029");
    case.also_on_a_fresh_state();
    case.eval("v.x = 0.0f; return math.floor(v.x);", 0.0);
    case.eval("v.x = 1.0f; return math.floor(v.x);", 1.0);
    case.eval("v.x = -1.0f; return math.floor(v.x);", -1.0);
    case.eval("v.x = 2.0f; return math.floor(v.x);", 2.0);
    case.eval("v.x = -2.0f; return math.floor(v.x);", -2.0);
    case.eval("v.x = 0.5f; return math.floor(v.x);", 0.0);
    case.eval("v.x = -0.5f; return math.floor(v.x);", -1.0);
    case.eval("v.x = 1.5f; return math.floor(v.x);", 1.0);
    case.eval("v.x = -1.5f; return math.floor(v.x);", -2.0);
    case.eval("v.x = 1.51f; return math.floor(v.x);", 1.0);
    case.eval("v.x = -1.51f; return math.floor(v.x);", -2.0);
    case.eval("v.x = 1.4f; return math.floor(v.x);", 1.0);
    case.eval("v.x = -1.4f; return math.floor(v.x);", -2.0);
    case.eval("v.x = 1.4999f; return math.floor(v.x);", 1.0);
    case.eval("v.x = -1.4999f; return math.floor(v.x);", -2.0);
    case.eval("v.x = 1.000001f; return math.floor(v.x);", 1.0);
    case.eval("v.x = -1.000001f; return math.floor(v.x);", -2.0);
    case.eval("v.x = 0.99999f; return math.floor(v.x);", 0.0);
    case.eval("v.x = -0.99999f; return math.floor(v.x);", -1.0);
    case.eval("v.x = 100000.000001f; return math.floor(v.x);", 100000.0);
    case.eval("v.x = -100000.000001f; return math.floor(v.x);", -100000.0);
    case.eval("v.x = 100000.99999f; return math.floor(v.x);", 100001.0);
    case.eval("v.x = -100000.99999f; return math.floor(v.x);", -100001.0);
    case.check(23);
}

#[test]
fn max_and_min_pick_the_larger_and_the_smaller_operand() {
    let mut case = EvalCase::new("evaluation-034");
    case.also_on_a_fresh_state();
    case.eval("math.max(0.0f, 1.0f)", 1.0);
    case.eval("math.max(1.0f, 0.0f)", 1.0);
    case.eval("math.max(-1.0f, 1.0f)", 1.0);
    case.eval("math.max((-1.0f), 1.0f)", 1.0);
    case.check(4);

    let mut case = EvalCase::new("evaluation-035");
    case.also_on_a_fresh_state();
    case.eval("v.x =  0.0; v.y = 1.0; return math.max( v.x,  v.y);", 1.0);
    case.eval("v.x =  1.0; v.y = 0.0; return math.max( v.x,  v.y);", 1.0);
    case.eval("v.x = -0.0; v.y = 1.0; return math.max( v.x,  v.y);", 1.0);
    case.eval("v.x = -1.0; v.y = 1.0; return math.max((v.x), v.y);", 1.0);
    case.check(4);

    let mut case = EvalCase::new("evaluation-036");
    case.also_on_a_fresh_state();
    case.eval("math.min(0.0f, 1.0f)", 0.0);
    case.eval("math.min(1.0f, 0.0f)", 0.0);
    case.eval("math.min(-1.0f, 1.0f)", -1.0);
    case.eval("math.min((-1.0f), 1.0f)", -1.0);
    case.check(4);

    let mut case = EvalCase::new("evaluation-037");
    case.also_on_a_fresh_state();
    case.eval("v.x =  0.0; v.y = 1.0; return math.min( v.x,  v.y);", 0.0);
    case.eval("v.x =  1.0; v.y = 0.0; return math.min( v.x,  v.y);", 0.0);
    case.eval("v.x = -1.0; v.y = 1.0; return math.min( v.x,  v.y);", -1.0);
    case.eval("v.x = -1.0; v.y = 1.0; return math.min((v.x), v.y);", -1.0);
    case.check(4);
}

#[test]
fn mod_by_zero_is_zero_and_the_result_follows_the_dividend() {
    let mut case = EvalCase::new("evaluation-038");
    case.also_on_a_fresh_state();
    case.eval(
        "v.x =  0.0f ; v.y = 1.0; return math.mod(v.x, v.y) + 0.125;",
        0.125,
    );
    case.eval(
        "v.x =  0.0f ; v.y = 2.0; return math.mod(v.x, v.y) + 0.125;",
        0.125,
    );
    case.eval(
        "v.x =  0.0f ; v.y = 0.0; return math.mod(v.x, v.y) + 0.125;",
        0.125,
    );
    case.eval(
        "v.x =  1.0f ; v.y = 0.0; return math.mod(v.x, v.y) + 0.125;",
        0.125,
    );
    case.eval(
        "v.x =  2.0f ; v.y = 0.0; return math.mod(v.x, v.y) + 0.125;",
        0.125,
    );
    case.eval(
        "v.x = -2.0f ; v.y = 0.0; return math.mod(v.x, v.y) + 0.125;",
        0.125,
    );
    case.eval("v.x =  1.0f ; v.y = 3.0; return math.mod(v.x, v.y);", 1.0);
    case.eval("v.x =  2.0f ; v.y = 3.0; return math.mod(v.x, v.y);", 2.0);
    case.eval(
        "v.x =  3.0f ; v.y = 3.0; return math.mod(v.x, v.y) + 0.125;",
        0.125,
    );
    case.eval("v.x =  4.0f ; v.y = 3.0; return math.mod(v.x, v.y);", 1.0);
    case.eval("v.x =  0.25f; v.y = 0.5; return math.mod(v.x, v.y);", 0.25);
    case.eval("v.x =  0.4f ; v.y = 0.5; return math.mod(v.x, v.y);", 0.4);
    case.eval(
        "v.x =  0.5f ; v.y = 0.5; return math.mod(v.x, v.y) + 0.125;",
        0.125,
    );
    case.eval("v.x =  0.6f ; v.y = 0.5; return math.mod(v.x, v.y);", 0.1);
    case.eval("v.x = -5.1f ; v.y = 3.0; return math.mod(v.x, v.y);", -2.1);
    case.eval(
        "v.x =  0.0f ; v.y = 1.0; return math.mod(v.x, v.y) - 1;",
        -1.0,
    );
    case.eval(
        "v.x =  0.0f ; v.y = 1.0; return 1 - math.mod(v.x, v.y);",
        1.0,
    );
    case.eval(
        "v.x =  0.0f ; v.y = 1.0; return math.mod(v.x, v.y) - math.mod(v.x, v.y) + 0.125;",
        0.125,
    );
    case.check(18);
}

#[test]
fn round_rounds_halves_away_from_zero() {
    let mut case = EvalCase::new("evaluation-040");
    case.also_on_a_fresh_state();
    case.eval("math.round(0.0f)", 0.0);
    case.eval("math.round(1.0f)", 1.0);
    case.eval("math.round(-1.0f)", -1.0);
    case.eval("math.round((-1.0f))", -1.0);
    case.eval("math.round(2.0f)", 2.0);
    case.eval("math.round(-2.0f)", -2.0);
    case.eval("math.round(0.5f)", 1.0);
    case.eval("math.round(-0.5f)", -1.0);
    case.eval("math.round(1.5f)", 2.0);
    case.eval("math.round(-1.5f)", -2.0);
    case.eval("math.round(1.51f)", 2.0);
    case.eval("math.round(-1.51f)", -2.0);
    case.eval("math.round(1.4f)", 1.0);
    case.eval("math.round(-1.4f)", -1.0);
    case.eval("math.round(1.4999f)", 1.0);
    case.eval("math.round(-1.4999f)", -1.0);
    case.eval("math.round(1.000001f)", 1.0);
    case.eval("math.round(-1.000001f)", -1.0);
    case.eval("math.round(0.99999f)", 1.0);
    case.eval("math.round(-0.99999f)", -1.0);
    case.eval("math.round(100000.000001f)", 100000.0);
    case.eval("math.round(-100000.000001f)", -100000.0);
    case.eval("math.round(100000.99999f)", 100001.0);
    case.eval("math.round(-100000.99999f)", -100001.0);
    case.check(24);

    let mut case = EvalCase::new("evaluation-041");
    case.also_on_a_fresh_state();
    case.eval("v.x = 0.0f; return math.round(v.x);", 0.0);
    case.eval("v.x = 1.0f; return math.round(v.x);", 1.0);
    case.eval("v.x = -1.0f; return math.round(v.x);", -1.0);
    case.eval("v.x = 2.0f; return math.round(v.x);", 2.0);
    case.eval("v.x = -2.0f; return math.round(v.x);", -2.0);
    case.eval("v.x = 0.5f; return math.round(v.x);", 1.0);
    case.eval("v.x = -0.5f; return math.round(v.x);", -1.0);
    case.eval("v.x = 1.5f; return math.round(v.x);", 2.0);
    case.eval("v.x = -1.5f; return math.round(v.x);", -2.0);
    case.eval("v.x = 1.51f; return math.round(v.x);", 2.0);
    case.eval("v.x = -1.51f; return math.round(v.x);", -2.0);
    case.eval("v.x = 1.4f; return math.round(v.x);", 1.0);
    case.eval("v.x = -1.4f; return math.round(v.x);", -1.0);
    case.eval("v.x = 1.4999f; return math.round(v.x);", 1.0);
    case.eval("v.x = -1.4999f; return math.round(v.x);", -1.0);
    case.eval("v.x = 1.000001f; return math.round(v.x);", 1.0);
    case.eval("v.x = -1.000001f; return math.round(v.x);", -1.0);
    case.eval("v.x = 0.99999f; return math.round(v.x);", 1.0);
    case.eval("v.x = -0.99999f; return math.round(v.x);", -1.0);
    case.eval("v.x = 100000.000001f; return math.round(v.x);", 100000.0);
    case.eval("v.x = -100000.000001f; return math.round(v.x);", -100000.0);
    case.eval("v.x = 100000.99999f; return math.round(v.x);", 100001.0);
    case.eval("v.x = -100000.99999f; return math.round(v.x);", -100001.0);
    case.check(23);
}

#[test]
fn sign_of_zero_is_one() {
    let mut case = EvalCase::new("evaluation-045");
    case.also_on_a_fresh_state();
    case.tolerance(0.0001);
    case.eval("math.sign(0.0f)", 1.0);
    case.eval("math.sign(math.pi/1.3f)", 1.0);
    case.eval("math.sign(math.pi)", 1.0);
    case.eval("math.sign(-math.pi/2.0f)", -1.0);
    case.eval("math.sign(-math.pi/2.0f)", -1.0);
    case.eval("math.sign(123.456f)", 1.0);
    case.eval("math.sign(-123.456f)", -1.0);
    case.check(7);

    let mut case = EvalCase::new("evaluation-046");
    case.also_on_a_fresh_state();
    case.eval("math.sign(1.0) + 1", 2.0);
    case.eval("math.sign(1.0) * 2", 2.0);
    case.eval("math.sign(1.0) * 2 + 1", 3.0);
    case.eval("math.sign(1.0) * -2 + 1", -1.0);
    case.eval("math.sign(1.0) * -2 - 1", -3.0);
    case.check(5);

    let mut case = EvalCase::new("evaluation-047");
    case.also_on_a_fresh_state();
    case.eval("v.x = 0.0f; return math.sign(v.x);", 1.0);
    case.eval("v.x = math.pi / 1.3f; return math.sign(v.x);", 1.0);
    case.eval("v.x = math.pi; return math.sign(v.x);", 1.0);
    case.eval("v.x = -math.pi / 2.0f; return math.sign(v.x);", -1.0);
    case.eval("v.x = -math.pi / 2.0f; return math.sign(v.x);", -1.0);
    case.eval("v.x = 123.456f; return math.sign(v.x);", 1.0);
    case.eval("v.x = -123.456f; return math.sign(v.x);", -1.0);
    case.check(7);
}

#[test]
fn random_stays_between_its_bounds_in_either_order() {
    let mut case = EvalCase::new("evaluation-054");
    case.range("math.random(1.0f, 3.0f)", 1.0, 3.0);
    case.range("math.random(2.0f, 3.0f)", 2.0, 3.0);
    case.range("math.random(3.0f, 2.0f)", 3.0, 2.0);
    case.range("math.random(3.0f, 1.0f)", 3.0, 1.0);
    case.range("math.random(-1.0f, -3.0f)", -1.0, -3.0);
    case.range("math.random(-2.0f, -3.0f)", -2.0, -3.0);
    case.range("math.random(-3.0f, -2.0f)", -3.0, -2.0);
    case.range("math.random(-3.0f, -1.0f)", -3.0, -1.0);
    case.check(8);

    let mut case = EvalCase::new("evaluation-055");
    case.range(
        "v.x = 1.0; v.y = 3.0; return math.random(v.x, v.y);",
        1.0,
        3.0,
    );
    case.range(
        "v.x = 2.0; v.y = 3.0; return math.random(v.x, v.y);",
        2.0,
        3.0,
    );
    case.range(
        "v.x = 3.0; v.y = 2.0; return math.random(v.x, v.y);",
        3.0,
        2.0,
    );
    case.range(
        "v.x = 3.0; v.y = 1.0; return math.random(v.x, v.y);",
        3.0,
        1.0,
    );
    case.range(
        "v.x = -1.0; v.y = -3.0; return math.random(v.x, v.y);",
        -1.0,
        -3.0,
    );
    case.range(
        "v.x = -2.0; v.y = -3.0; return math.random(v.x, v.y);",
        -2.0,
        -3.0,
    );
    case.range(
        "v.x = -3.0; v.y = -2.0; return math.random(v.x, v.y);",
        -3.0,
        -2.0,
    );
    case.range(
        "v.x = -3.0; v.y = -1.0; return math.random(v.x, v.y);",
        -3.0,
        -1.0,
    );
    case.check(8);

    let mut case = EvalCase::new("evaluation-056");
    case.also_on_a_fresh_state();
    case.eval("math.random(1.0, 1.0) + 1", 2.0);
    case.eval("math.random(1.0, 1.0) * 2", 2.0);
    case.eval("math.random(1.0, 1.0) * 2 + 1", 3.0);
    case.eval("math.random(1.0, 1.0) * -2 + 1", -1.0);
    case.eval("math.random(1.0, 1.0) * -2 - 1", -3.0);
    case.check(5);
}

/// A negative count gives 0.
#[test]
fn die_roll_sums_its_draws_between_the_bounds() {
    let mut case = EvalCase::new("evaluation-057");
    case.range("math.die_roll(1, 1, 6)", 1.0, 6.0);
    case.range("math.die_roll(2, 1, 6)", 2.0, 12.0);
    case.range("math.die_roll(3, 1, 6)", 3.0, 18.0);
    case.range("math.die_roll(1, 2, 6)", 2.0, 6.0);
    case.range("math.die_roll(2, 2, 6)", 4.0, 12.0);
    case.range("math.die_roll(3, 2, 6)", 6.0, 18.0);
    case.range("math.die_roll(3, 8, 8)", 24.0, 24.0);
    case.range("math.die_roll(1, 0, 4)", 0.0, 4.0);
    case.range("math.die_roll(2, 0, 4)", 0.0, 8.0);
    case.range("math.die_roll(3, 0, 4)", 0.0, 12.0);
    case.range("math.die_roll(-3, 0, 4)", 0.0, 0.0);
    case.check(11);

    let mut case = EvalCase::new("evaluation-058");
    case.also_on_a_fresh_state();
    case.eval("math.die_roll(1, 1, 1) + 1", 2.0);
    case.eval("math.die_roll(1, 1, 1) * 2", 2.0);
    case.eval("math.die_roll(1, 1, 1) * 2 + 1", 3.0);
    case.eval("math.die_roll(1, 1, 1) * -2 + 1", -1.0);
    case.eval("math.die_roll(1, 1, 1) * -2 - 1", -3.0);
    case.check(5);
}

/// With fixed draws, non-integer bounds and counts read as their integer part.
#[test]
fn die_roll_integer_sums_integer_draws() {
    let mut case = EvalCase::new("evaluation-059");
    case.fixed_random(0.0);
    case.eval("math.die_roll_integer(1, 1, 6)", 1.0);
    case.eval("math.die_roll_integer(1, 1.1, 6)", 1.0);
    case.eval("math.die_roll_integer(1, 1.9, 6)", 1.0);
    case.eval("math.die_roll_integer(2, 1, 6)", 2.0);
    case.eval("math.die_roll_integer(3, 1, 6)", 3.0);
    case.eval("math.die_roll_integer(1, 2, 6)", 2.0);
    case.eval("math.die_roll_integer(2, 2, 6)", 4.0);
    case.eval("math.die_roll_integer(3, 2, 6)", 6.0);
    case.eval("math.die_roll_integer(3, 8, 8)", 24.0);
    case.eval("math.die_roll_integer(1, 0, 4)", 0.0);
    case.eval("math.die_roll_integer(2, 0, 4)", 0.0);
    case.eval("math.die_roll_integer(3, 0, 4)", 0.0);
    case.eval("math.die_roll_integer(-3, 0, 4)", 0.0);
    case.check(13);

    let mut case = EvalCase::new("evaluation-060");
    case.also_on_a_fresh_state();
    case.eval("math.die_roll_integer(1, 1, 1) + 1", 2.0);
    case.eval("math.die_roll_integer(1, 1, 1) * 2", 2.0);
    case.eval("math.die_roll_integer(1, 1, 1) * 2 + 1", 3.0);
    case.eval("math.die_roll_integer(1, 1, 1) * -2 + 1", -1.0);
    case.eval("math.die_roll_integer(1, 1, 1) * -2 - 1", -3.0);
    case.check(5);

    let mut case = EvalCase::new("evaluation-061");
    case.fixed_random(1.0);
    case.eval("math.die_roll_integer(1, 1, 6)", 6.0);
    case.eval("math.die_roll_integer(2, 1, 6)", 12.0);
    case.eval("math.die_roll_integer(3, 1, 6)", 18.0);
    case.eval("math.die_roll_integer(1, 2, 6)", 6.0);
    case.eval("math.die_roll_integer(2, 2, 6)", 12.0);
    case.eval("math.die_roll_integer(3, 2, 6)", 18.0);
    case.eval("math.die_roll_integer(3, 8, 8)", 24.0);
    case.eval("math.die_roll_integer(1, 0, 4)", 4.0);
    case.eval("math.die_roll_integer(2, 0, 4)", 8.0);
    case.eval("math.die_roll_integer(3, 0, 4)", 12.0);
    case.eval("math.die_roll_integer(-3, 0, 4)", 0.0);
    case.eval("math.die_roll_integer(3, 0, 4.1)", 12.0);
    case.eval("math.die_roll_integer(3, 0, 4.9)", 12.0);
    case.check(13);

    let mut case = EvalCase::new("evaluation-062");
    case.range("math.die_roll_integer(1, 1, 6)", 1.0, 6.0);
    case.range("math.die_roll_integer(2, 1, 6)", 2.0, 12.0);
    case.range("math.die_roll_integer(3, 1, 6)", 3.0, 18.0);
    case.range("math.die_roll_integer(1, 2, 6)", 2.0, 6.0);
    case.range("math.die_roll_integer(2, 2, 6)", 4.0, 12.0);
    case.range("math.die_roll_integer(3, 2, 6)", 6.0, 18.0);
    case.range("math.die_roll_integer(3, 8, 8)", 24.0, 24.0);
    case.range("math.die_roll_integer(1, 0, 4)", 0.0, 4.0);
    case.range("math.die_roll_integer(2, 0, 4)", 0.0, 8.0);
    case.range("math.die_roll_integer(3, 0, 4)", 0.0, 12.0);
    case.range("math.die_roll_integer(-3, 0, 4)", 0.0, 0.0);
    case.check(11);

    let mut case = EvalCase::new("evaluation-063");
    case.fixed_random(0.5);
    case.eval("math.die_roll_integer(1, 1, 1)", 1.0);
    case.eval("math.die_roll_integer(1, 1, 2)", 1.0);
    case.eval("math.die_roll_integer(1, 1, 3)", 2.0);
    case.eval("math.die_roll_integer(1, 1, 4)", 2.0);
    case.eval("math.die_roll_integer(1, 1, 5)", 3.0);
    case.eval("math.die_roll_integer(1, 1, 6)", 3.0);
    case.eval("math.die_roll_integer(1.9, 1.9, 1.9)", 1.0);
    case.eval("math.die_roll_integer(1.9, 1.9, 2.9)", 1.0);
    case.eval("math.die_roll_integer(1.9, 1.9, 3.9)", 2.0);
    case.eval("math.die_roll_integer(1.9, 1.9, 4.9)", 2.0);
    case.eval("math.die_roll_integer(1.9, 1.9, 5.9)", 3.0);
    case.eval("math.die_roll_integer(1.9, 1.9, 6.9)", 3.0);
    case.eval("math.die_roll_integer(2, 1, 7)", 8.0);
    case.eval("math.die_roll_integer(3, 1, 6)", 9.0);
    case.eval("math.die_roll_integer(1, 2, 6)", 4.0);
    case.eval("math.die_roll_integer(2, 2, 6)", 8.0);
    case.eval("math.die_roll_integer(3, 2, 6)", 12.0);
    case.eval("math.die_roll_integer(3, 8, 8)", 24.0);
    case.eval("math.die_roll_integer(1, 0, 4)", 2.0);
    case.eval("math.die_roll_integer(2, 0, 4)", 4.0);
    case.eval("math.die_roll_integer(3, 0, 4)", 6.0);
    case.eval("math.die_roll_integer(-3, 0, 4)", 0.0);
    case.check(22);
}

/// A lower bound of 0.1 comes back as 0.1 under a draw of 0.
#[test]
fn random_integer_with_fixed_draws() {
    let mut case = EvalCase::new("evaluation-064");
    case.fixed_random(0.0);
    case.eval("math.random_integer(0.0, 3.0)", 0.0);
    case.eval("math.random_integer(1.0, 3.0)", 1.0);
    case.eval("math.random_integer(4.0, 3.0)", 3.0);
    case.eval("math.random_integer(-1.0, 3.0)", -1.0);
    case.eval("math.random_integer(-1.0, -3.0)", -3.0);
    case.eval("math.random_integer(-1000000.0, 0.0)", -1000000.0);
    case.eval("math.random_integer(1000000.0, 0.0)", 0.0);
    case.eval("math.random_integer(1000000.0, 1000000.0)", 1000000.0);
    case.eval("math.random_integer(1000000.0, 1000001.0)", 1000000.0);
    case.eval("math.random_integer(0.1, 1000001.0)", 0.1);
    case.check(10);

    let mut case = EvalCase::new("evaluation-065");
    case.fixed_random(1.0);
    case.eval("math.random_integer(0.0, 3.0)", 3.0);
    case.eval("math.random_integer(1.0, 3.0)", 3.0);
    case.eval("math.random_integer(4.0, 3.0)", 4.0);
    case.eval("math.random_integer(-1.0, 3.0)", 3.0);
    case.eval("math.random_integer(-1.0, -3.0)", -1.0);
    case.eval("math.random_integer(-1000000.0, 0.0)", 0.0);
    case.eval("math.random_integer(1000000.0, 0.0)", 1000000.0);
    case.eval("math.random_integer(1000000.0, 1000000.0)", 1000000.0);
    case.eval("math.random_integer(1000000.0, 1000001.0)", 1000001.0);
    case.eval("math.random_integer(0.1, 1000001.0)", 1000001.0);
    case.check(10);

    let mut case = EvalCase::new("evaluation-066");
    case.fixed_random(0.5);
    case.eval(
        "v.x = 0.0; v.y = 3.0; return math.random_integer(v.x, v.y);",
        1.0,
    );
    case.eval(
        "v.x = 1.0; v.y = 3.0; return math.random_integer(v.x, v.y);",
        2.0,
    );
    case.eval(
        "v.x = 4.0; v.y = 3.0; return math.random_integer(v.x, v.y);",
        3.0,
    );
    case.eval(
        "v.x = -1.0; v.y = 3.0; return math.random_integer(v.x, v.y);",
        1.0,
    );
    case.eval(
        "v.x = -1.0; v.y = -3.0; return math.random_integer(v.x, v.y);",
        -2.0,
    );
    case.eval(
        "v.x = -1000000.0; v.y = 0.0; return math.random_integer(v.x, v.y);",
        -500000.0,
    );
    case.eval(
        "v.x = 1000000.0; v.y = 0.0; return math.random_integer(v.x, v.y);",
        500000.0,
    );
    case.eval(
        "v.x = 1000000.0; v.y = 1000000.0; return math.random_integer(v.x, v.y);",
        1000000.0,
    );
    case.eval(
        "v.x = 1000000.0; v.y = 1000001.0; return math.random_integer(v.x, v.y);",
        1000000.0,
    );
    case.check(9);

    let mut case = EvalCase::new("evaluation-067");
    case.also_on_a_fresh_state();
    case.eval("math.random_integer(1.0, 1.0) + 1", 2.0);
    case.eval("math.random_integer(1.0, 1.0) * 2", 2.0);
    case.eval("math.random_integer(1.0, 1.0) * 2 + 1", 3.0);
    case.eval("math.random_integer(1.0, 1.0) * -2 + 1", -1.0);
    case.eval("math.random_integer(1.0, 1.0) * -2 - 1", -3.0);
    case.check(5);
}
