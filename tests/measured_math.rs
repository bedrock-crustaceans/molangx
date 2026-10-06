//! The math functions, NaN through math functions and comparisons, the rounding of a scale or
//! offset on a product, quotient or call, and the architectures' primitives.

#![cfg(all(feature = "compiler", feature = "stdlib"))]
// Expected values are written as the shortest decimal that gives their bits; some are close to a
// constant.
#![allow(clippy::approx_constant)]

mod common;

use common::measured::math_call::parse_call;
use common::measured::*;
use common::per_arch;
use molangx::numeric::{self, ARCH, Arch, PostOp};
use molangx::stdlib::math;

/// NaN through `math.max` / `math.min`, `math.mod` by zero and other questions asked through
/// `query.log`, and a missing read.
fn run_01() -> ServerRun {
    let mut run = ServerRun::new(
        "run_01",
        FIRST_RELEASE,
        "NaN through max / min, mod by zero, min_angle, a long loop, division order, clamp / round / sign / inverse_lerp, asked through query.log; a missing read",
    );
    run.arm64_differs(&[]);
    run.probe(
        "i00",
        "v.n = math.sqrt(-1); v.f = 4; v.r = q.log(1001, math.max(v.n, v.f), math.max(v.f, v.n), math.min(v.n, v.f), math.min(v.f, v.n), math.max(math.sqrt(-1), 4), math.max(4, math.sqrt(-1)));",
    )
    .silent()
    .inconclusive();
    run.probe("i01", "v.one = 1; v.zero = 0; v.r = q.log(1002, math.mod(1, 0), math.mod(v.one, 0), math.mod(v.one, v.zero));")
        .silent()
        .inconclusive();
    run.probe("i02", "v.d = 180; v.r = q.log(1003, math.min_angle(180), math.min_angle(v.d), math.min_angle(-180));")
        .silent()
        .inconclusive();
    run.probe(
        "i03",
        "v.c = 0; loop(5000, {v.c = v.c + 1;}); v.r = q.log(1004, v.c);",
    )
    .silent()
    .inconclusive();
    run.probe(
        "i04",
        "v.a = 1000; v.b = 13; v.r = q.log(1005, v.a / v.b * 3, (v.a * 3) / v.b, (v.a / v.b) * 3);",
    )
    .silent()
    .inconclusive();
    run.probe("i05", "v.r = q.log(7, 8); v.r = q.log(1006, v.r);")
        .silent()
        .inconclusive();
    run.probe("i06", "v.r = q.log(1007, math.clamp(v.n, 1, 2), math.round(-2.5), math.sign(0), math.inverse_lerp(5, 5, v.f));")
        .silent()
        .inconclusive();
    run.probe(
        "i07",
        "t.seen = 1; v.missing_never_set; t.seen = 2; v.r = q.log(1008, 99);",
    )
    .answers("missing_never_set");
    run.probe("i08", "v.r = q.log(1009, t.seen);")
        .silent()
        .inconclusive();
    run
}

#[test]
fn query_log_prints_nothing_and_a_missing_read_is_logged_run_01() {
    run_01().replay(9);
}

/// The same questions again.
fn run_02() -> ServerRun {
    let mut run = ServerRun::new(
        "run_02",
        FIRST_RELEASE,
        "the probes of run_01 again: NaN through max / min, mod by zero, min_angle, a long loop, division order, clamp / round / sign / inverse_lerp, asked through query.log; a missing read",
    );
    run.arm64_differs(&[]);
    run.probe(
        "i00",
        "v.n = math.sqrt(-1); v.f = 4; v.r = q.log(1001, math.max(v.n, v.f), math.max(v.f, v.n), math.min(v.n, v.f), math.min(v.f, v.n), math.max(math.sqrt(-1), 4), math.max(4, math.sqrt(-1)));",
    )
    .silent()
    .inconclusive();
    run.probe("i01", "v.one = 1; v.zero = 0; v.r = q.log(1002, math.mod(1, 0), math.mod(v.one, 0), math.mod(v.one, v.zero));")
        .silent()
        .inconclusive();
    run.probe("i02", "v.d = 180; v.r = q.log(1003, math.min_angle(180), math.min_angle(v.d), math.min_angle(-180));")
        .silent()
        .inconclusive();
    run.probe(
        "i03",
        "v.c = 0; loop(5000, {v.c = v.c + 1;}); v.r = q.log(1004, v.c);",
    )
    .silent()
    .inconclusive();
    run.probe(
        "i04",
        "v.a = 1000; v.b = 13; v.r = q.log(1005, v.a / v.b * 3, (v.a * 3) / v.b, (v.a / v.b) * 3);",
    )
    .silent()
    .inconclusive();
    run.probe("i05", "v.r = q.log(7, 8); v.r = q.log(1006, v.r);")
        .silent()
        .inconclusive();
    run.probe("i06", "v.r = q.log(1007, math.clamp(v.n, 1, 2), math.round(-2.5), math.sign(0), math.inverse_lerp(5, 5, v.f));")
        .silent()
        .inconclusive();
    run.probe(
        "i07",
        "t.seen = 1; v.missing_never_set; t.seen = 2; v.r = q.log(1008, 99);",
    )
    .answers("missing_never_set");
    run.probe("i08", "v.r = q.log(1009, t.seen);")
        .silent()
        .inconclusive();
    run
}

#[test]
fn query_log_prints_nothing_and_a_missing_read_is_logged_run_02() {
    run_02().replay(9);
}

/// The same questions again, with the content log file enabled.
fn run_03() -> ServerRun {
    let mut run = ServerRun::new(
        "run_03",
        FIRST_RELEASE,
        "the probes of run_01 again with the content log file also enabled: NaN through max / min, mod by zero, min_angle, a long loop, division order, clamp / round / sign / inverse_lerp, asked through query.log; a missing read",
    );
    run.arm64_differs(&[]);
    run.probe(
        "i00",
        "v.n = math.sqrt(-1); v.f = 4; v.r = q.log(1001, math.max(v.n, v.f), math.max(v.f, v.n), math.min(v.n, v.f), math.min(v.f, v.n), math.max(math.sqrt(-1), 4), math.max(4, math.sqrt(-1)));",
    )
    .silent()
    .inconclusive();
    run.probe("i01", "v.one = 1; v.zero = 0; v.r = q.log(1002, math.mod(1, 0), math.mod(v.one, 0), math.mod(v.one, v.zero));")
        .silent()
        .inconclusive();
    run.probe("i02", "v.d = 180; v.r = q.log(1003, math.min_angle(180), math.min_angle(v.d), math.min_angle(-180));")
        .silent()
        .inconclusive();
    run.probe(
        "i03",
        "v.c = 0; loop(5000, {v.c = v.c + 1;}); v.r = q.log(1004, v.c);",
    )
    .silent()
    .inconclusive();
    run.probe(
        "i04",
        "v.a = 1000; v.b = 13; v.r = q.log(1005, v.a / v.b * 3, (v.a * 3) / v.b, (v.a / v.b) * 3);",
    )
    .silent()
    .inconclusive();
    run.probe("i05", "v.r = q.log(7, 8); v.r = q.log(1006, v.r);")
        .silent()
        .inconclusive();
    run.probe("i06", "v.r = q.log(1007, math.clamp(v.n, 1, 2), math.round(-2.5), math.sign(0), math.inverse_lerp(5, 5, v.f));")
        .silent()
        .inconclusive();
    run.probe(
        "i07",
        "t.seen = 1; v.missing_never_set; t.seen = 2; v.r = q.log(1008, 99);",
    )
    .answers("missing_never_set");
    run.probe("i08", "v.r = q.log(1009, t.seen);")
        .silent()
        .inconclusive();
    run
}

#[test]
fn query_log_prints_nothing_and_a_missing_read_is_logged_run_03() {
    run_03().replay(9);
}

/// The same questions, each answered by the never-set variable it reads.
fn run_04() -> ServerRun {
    let mut run = ServerRun::new(
        "run_04",
        FIRST_RELEASE,
        "the questions of runs 1-3 answered by never-set variables: NaN through min / max / clamp, mod by zero, min_angle, a long loop, division order, query.log's value, round / sign, a missing read and temps, the division guard, inverse_lerp",
    );
    run.arm64_differs(&["r02", "r04", "r12"]);
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
        "r03",
        "v.p1 = math.min(v.n, v.f); v.p1 == 4 ? v.r03_minnanfirst_is4 : (v.p1 == v.p1 ? v.r03_minnanfirst_other : v.r03_minnanfirst_isnan);",
    )
    .answers("r03_minnanfirst_is4");
    run.probe(
        "r04",
        "v.p1 = math.min(v.f, v.n); v.p1 == 4 ? v.r04_minnansecond_is4 : (v.p1 == v.p1 ? v.r04_minnansecond_other : v.r04_minnansecond_isnan);",
    )
    .answers("r04_minnansecond_isnan");
    run.probe(
        "r05",
        "v.p1 = math.max(math.sqrt(-1), 4); v.p1 == 4 ? v.r05_maxconstnanfirst_is4 : (v.p1 == v.p1 ? v.r05_other : v.r05_maxconstnanfirst_isnan);",
    )
    .answers("r05_maxconstnanfirst_is4");
    run.probe(
        "r06",
        "v.p1 = math.mod(1, 0); v.p1 == 0 ? v.r06_modlitlit_is0 : (v.p1 == v.p1 ? v.r06_modlitlit_other : v.r06_modlitlit_isnan);",
    )
    .answers("r06_modlitlit_isnan");
    run.probe(
        "r07",
        "v.one = 1; v.p1 = math.mod(v.one, 0); v.p1 == 0 ? v.r07_modvarlit_is0 : (v.p1 == v.p1 ? v.r07_modvarlit_other : v.r07_modvarlit_isnan);",
    )
    .answers("r07_modvarlit_isnan");
    run.probe(
        "r08",
        "v.zero = 0; v.p1 = math.mod(v.one, v.zero); v.p1 == 0 ? v.r08_modvarvar_is0 : (v.p1 == v.p1 ? v.r08_modvarvar_other : v.r08_modvarvar_isnan);",
    )
    .answers("r08_modvarvar_is0");
    run.probe(
        "r09",
        "v.d = 180; v.p1 = math.min_angle(v.d); v.p1 == -180 ? v.r09_minangle180var_isneg180 : (v.p1 == 180 ? v.r09_minangle180var_is180 : v.r09_other);",
    )
    .answers("r09_minangle180var_isneg180");
    run.probe(
        "r10",
        "v.p1 = math.min_angle(180); v.p1 == -180 ? v.r10_minangle180lit_isneg180 : (v.p1 == 180 ? v.r10_minangle180lit_is180 : v.r10_other);",
    )
    .answers("r10_minangle180lit_isneg180");
    run.probe(
        "r11",
        "v.c = 0; loop(5000, {v.c = v.c + 1;}); v.c == 5000 ? v.r11_loop5000_ran5000 : (v.c == 1024 ? v.r11_loop5000_ran1024 : v.r11_loop5000_other);",
    )
    .answers("r11_loop5000_ran5000");
    run.probe("r12", "v.a = 1000; v.b = 13; v.x = v.a * 3; v.y = v.x / v.b; v.q = v.a / v.b; v.z = v.q * 3; v.p1 = v.a / v.b * 3; v.p1 == v.y ? (v.y == v.z ? v.r12_indistinct : v.r12_div_scale_before_divide) : (v.p1 == v.z ? v.r12_div_divide_then_scale : v.r12_neither);").answers("r12_div_divide_then_scale");
    run.probe(
        "r13",
        "v.p1 = q.log(7, 8); v.p1 == 7 ? v.r13_log_returns_first : (v.p1 == 0 ? v.r13_log_returns_zero : v.r13_log_other);",
    )
    .answers("r13_log_returns_first");
    run.probe(
        "r14",
        "v.p1 = math.clamp(v.n, 1, 2); v.p1 == 1 ? v.r14_clampnan_is_lo : (v.p1 == 2 ? v.r14_clampnan_is_hi : (v.p1 == v.p1 ? v.r14_other : v.r14_clampnan_isnan));",
    )
    .answers("r14_clampnan_is_lo");
    run.probe(
        "r15",
        "v.m = -2.5; v.p1 = math.round(v.m); v.p1 == -3 ? v.r15_round_neg2p5_isneg3 : (v.p1 == -2 ? v.r15_round_neg2p5_isneg2 : v.r15_other);",
    )
    .answers("r15_round_neg2p5_isneg3");
    run.probe("r16", "t.seen = 1; v.missing_never_set_r16; t.seen = 2;")
        .answers("missing_never_set_r16");
    run.probe("r17", "t.seen == 1 ? v.r17_abort_and_temp_persisted : (t.seen == 2 ? v.r17_no_abort : v.r17_other);")
        .answers("r17_abort_and_temp_persisted");
    run.probe(
        "r18",
        "v.p1 = math.sign(0); v.p1 == 1 ? v.r18_sign0_is1 : v.r18_sign0_other;",
    )
    .answers("r18_sign0_is1");
    run.probe(
        "r19",
        "v.g = 0.0000001; v.p1 = 5 / v.g; v.p1 == 0 ? v.r19_div_tiny_is0 : v.r19_div_tiny_nonzero;",
    )
    .answers("r19_div_tiny_is0");
    run.probe(
        "r20",
        "v.h = -1; v.p1 = 5 / v.h; v.p1 == -5 ? v.r20_div_neg_v13_isneg5 : (v.p1 == 5 ? v.r20_div_neg_is5 : v.r20_other);",
    )
    .answers("r20_div_neg_v13_isneg5");
    run.probe(
        "r21",
        "v.p1 = math.inverse_lerp(5, 5, v.f); v.p1 > 1000000 ? v.r21_invlerp_zero_span_inf : (v.p1 == 0 ? v.r21_invlerp_zero_span_0 : v.r21_other);",
    )
    .answers("r21_other")
    .inconclusive();
    run.probe("r22", "v.p1 = 7 * 3 / 9; v.p2 = 7 * 0.33333334; v.p1 == v.p2 ? v.r22_slash_before_star : v.r22_other;")
        .answers("r22_slash_before_star");
    run
}

/// With a NaN `math.max` and `math.min` return their second operand; `math.mod` by a literal 0 is
/// NaN and by a variable 0 is 0; `clamp(NaN, 1, 2)` is 1; `sign(0)` is 1.
#[test]
fn nan_through_min_max_mod_and_clamp_and_the_division_guard_run_04() {
    run_04().replay(22);
}

/// NaN in comparisons, `?:`, `!` and `&&`; `math.sign(NaN)`; the rounding of a scaled product;
/// `math.asin` just outside [-1, 1]; `math.random` with a NaN bound.
fn run_05() -> ServerRun {
    let mut run = ServerRun::new(
        "run_05",
        FIRST_RELEASE,
        "NaN in comparisons, ?:, ! and &&; sign(NaN); the rounding of a scaled product with an offset and of a product chain; asin just outside [-1, 1]; random with a NaN bound",
    );
    run.arm64_differs(&["s01", "s04", "s08", "s09", "s13"]);
    run.probe(
        "s01",
        "v.n = math.sqrt(-1); v.n <= 4 ? v.s01_nan_le_true : v.s01_nan_le_false;",
    )
    .answers("s01_nan_le_false");
    run.probe("s02", "v.n >= 4 ? v.s02_nan_ge_true : v.s02_nan_ge_false;")
        .answers("s02_nan_ge_false");
    run.probe("s03", "v.n != 4 ? v.s03_nan_ne_true : v.s03_nan_ne_false;")
        .answers("s03_nan_ne_true");
    run.probe("s04", "v.n < 4 ? v.s04_nan_lt_true : v.s04_nan_lt_false;")
        .answers("s04_nan_lt_false");
    run.probe("s05", "v.n ? v.s05_nan_truthy : v.s05_nan_falsy;")
        .answers("s05_nan_truthy");
    run.probe("s06", "!v.n ? v.s06_notnan_true : v.s06_notnan_false;")
        .answers("s06_notnan_false");
    run.probe(
        "s07",
        "(v.n && 1) ? v.s07_nan_and_true : v.s07_nan_and_false;",
    )
    .answers("s07_nan_and_true");
    run.probe("s08", "v.p1 = math.sign(v.n); v.p1 == 1 ? v.s08_signnan_1 : (v.p1 == -1 ? v.s08_signnan_neg1 : v.s08_signnan_other);")
        .answers("s08_signnan_1");
    run.probe(
        "s09",
        "v.x = 1 / 3; v.p1 = v.x * 3 - 1; v.p1 == 0 ? v.s09_postop_unfused : (v.p1 > 0 ? v.s09_postop_fused : v.s09_other);",
    )
    .answers("s09_postop_unfused");
    run.probe(
        "s10",
        "v.w = 1.0004; v.p1 = math.asin(v.w); v.p1 == 90 ? v.s10_asin_tol_clamped90 : (v.p1 == v.p1 ? v.s10_other : v.s10_asin_nan);",
    )
    .answers("s10_asin_tol_clamped90");
    run.probe("s11", "v.w = 1.001; v.p1 = math.asin(v.w); v.p1 == v.p1 ? v.s11_asin1001_notnan : v.s11_asin1001_nan;")
        .answers("s11_asin1001_nan");
    run.probe(
        "s12",
        "v.p1 = math.max(v.n, v.f) ; v.f = 4; v.p1 = math.clamp(v.f, v.n, 5); v.p1 == 4 ? v.s12_clamp_lonan_is_v : (v.p1 == v.p1 ? v.s12_other : v.s12_clamp_lonan_nan);",
    )
    .answers("f")
    .inconclusive();
    run.probe("s13", "v.p1 = math.random(v.n, 4); v.p1 == v.p1 ? v.s13_random_nanlo_notnan : v.s13_random_nanlo_nan;")
        .answers("s13_random_nanlo_nan");
    run.probe(
        "s14",
        "v.k = 3; v.j = 0.1; v.p1 = v.k * v.j * 7; v.e1 = v.k * v.j; v.e2 = v.e1 * 7; v.p1 == v.e2 ? v.s14_mul_chain_matches_stepwise : v.s14_mul_chain_differs;",
    )
    .answers("s14_mul_chain_matches_stepwise");
    run.probe(
        "s15",
        "v.p1 = math.min(v.n, v.f); v.p2 = math.max(v.n, v.n); v.p2 == v.p2 ? v.s15_max_nan_nan_notnan : v.s15_max_nan_nan_nan;",
    )
    .answers("f")
    .inconclusive();
    run
}

/// NaN compares false except with `!=` and is truthy; `math.sign(NaN)` is 1; `v.x * 3 - 1` rounds
/// the multiply and the add separately.
#[test]
fn nan_comparisons_truthiness_and_an_unfused_scale_and_offset_run_05() {
    run_05().replay(15);
}

/// `math.clamp` with a NaN bound, `math.max(NaN, NaN)`, and the rounding order of products,
/// `math.lerp` and a scaled quotient.
fn run_06() -> ServerRun {
    let mut run = ServerRun::new(
        "run_06",
        FIRST_RELEASE,
        "clamp with a NaN bound, max(NaN, NaN), the rounding order of a product chain, of lerp and of a scaled sum or quotient with an offset",
    );
    run.arm64_differs(&["u01", "u04", "u05", "u06", "u07"]);
    run.probe(
        "u01",
        "v.n = math.sqrt(-1); v.f = 4; v.p1 = math.clamp(v.f, v.n, 5); v.p1 == 4 ? v.u01_clamp_lonan_is_v : (v.p1 == v.p1 ? v.u01_other : v.u01_clamp_lonan_nan);",
    )
    .answers("u01_clamp_lonan_nan");
    run.probe("u02", "v.p2 = math.max(v.n, v.n); v.p2 == v.p2 ? v.u02_max_nan_nan_notnan : v.u02_max_nan_nan_nan;")
        .answers("u02_max_nan_nan_nan");
    run.probe(
        "u03",
        "v.p1 = math.clamp(v.f, 1, v.n); v.p1 == 4 ? v.u03_clamp_hinan_is_v : (v.p1 == v.p1 ? v.u03_other : v.u03_clamp_hinan_nan);",
    )
    .answers("u03_clamp_hinan_is_v");
    run.probe("u04", "v.k = 1.4; v.j = 0.75; v.ja = v.j * 7; v.A = v.k * v.ja; v.ka = v.k * 7; v.B = v.j * v.ka; v.kj = v.k * v.j; v.C = v.kj * 7; v.p1 = v.k * v.j * 7; v.p1 == v.A ? (v.p1 == v.B || v.p1 == v.C ? v.u04_ambig : v.u04_mul_k_times_jS) : (v.p1 == v.B ? v.u04_mul_j_times_kS : (v.p1 == v.C ? v.u04_mul_kj_then_S : v.u04_other));").answers("u04_mul_kj_then_s");
    run.probe("u05", "v.la = 1.1; v.lb = v.la * 3.3; v.t = 1 / 3; v.d = v.lb - v.la; v.td = v.t * v.d; v.un = v.la + v.td; v.p1 = math.lerp(v.la, v.lb, v.t); v.p1 == v.un ? v.u05_lerp_unfused : v.u05_lerp_fused_or_other;").answers("u05_lerp_unfused");
    run.probe("u06", "v.x = 1 / 3; v.p1 = (v.x + 0) * 3 - 1; v.p1 == 0 ? v.u06_postop_unfused2 : v.u06_postop_fused2;")
        .answers("u06_postop_unfused2");
    run.probe(
        "u07",
        "v.a = 1000; v.b = 13; v.p1 = v.a / v.b * 3 + 0.5; v.q = v.a / v.b; v.z = v.q * 3; v.zz = v.z + 0.5; v.p1 == v.zz ? v.u07_div_unfused_divide_first : v.u07_other;",
    )
    .answers("u07_div_unfused_divide_first");
    run
}

/// A NaN lower bound of `math.clamp` is returned and a NaN upper bound ignored; every step of a
/// product chain and of `math.lerp` is rounded.
#[test]
fn clamp_with_nan_bounds_and_every_step_of_lerp_and_products_rounded_run_06() {
    run_06().replay(7);
}

/// The easings, `hermite_blend`, the scaled `atan`, NaN operands, the sign of a zero remainder and
/// a folded offset, with load messages checked.
fn run_09() -> ServerRun {
    let mut run = ServerRun::new(
        "run_09",
        FIRST_RELEASE,
        "numeric questions (easings, hermite_blend, angle conversions, division by NaN, random bounds, the sign of a zero remainder, folded offsets) and NaN comparisons",
    );
    run.checks_load_messages();
    run.arm64_differs(&[
        "n01", "n02", "n04", "n05", "n06", "n07", "n08", "n09", "n10", "n11", "n12", "n15", "n24",
        "n25",
    ]);
    run.probe("n01", "v.t = 0.65; v.c = v.t * v.t; v.c = v.c * v.t; v.c = v.c * v.t; v.d = v.t * v.t; v.d = v.d * v.d; v.p1 = math.ease_in_quart(0, 1, v.t); v.p1 == v.c ? (v.c == v.d ? v.n01_indistinct : v.n01_quart_chain) : (v.p1 == v.d ? v.n01_quart_squared : v.n01_other);").answers("n01_quart_chain");
    run.probe("n02", "v.s = 0.3; v.e = 1.7; v.t = 0.72; v.d = v.e - v.s; v.a = v.d * v.t; v.a = v.a * v.t; v.a = v.a * v.t; v.a = v.a * v.t; v.a = v.s + v.a; v.q = v.t * v.t; v.q = v.q * v.q; v.b = v.q * v.d; v.b = v.b + v.s; v.p1 = math.ease_in_quart(v.s, v.e, v.t); v.p1 == v.a ? (v.a == v.b ? v.n02_indistinct : v.n02_quart_d_chain) : (v.p1 == v.b ? v.n02_quart_sq_times_d : v.n02_other);").answers("n02_quart_d_chain");
    run.probe("n03", "v.s = 0.3; v.e = 1.7; v.t = 0.4; v.d = v.e - v.s; v.u = v.t - 1; v.c = v.u * v.u; v.c = v.c * v.u; v.a = v.c + 1; v.a = v.a * v.d; v.a = v.s + v.a; v.b = v.c * v.d; v.b = v.b + v.d; v.b = v.b + v.s; v.p1 = math.ease_out_cubic(v.s, v.e, v.t); v.p1 == v.a ? (v.a == v.b ? v.n03_indistinct : v.n03_cube_plus1_times_d) : (v.p1 == v.b ? v.n03_cube_times_d_plus_d : v.n03_other);").answers("n03_cube_plus1_times_d");
    run.probe("n04", "v.s = 0.3; v.e = 0.1; v.one = 1; v.d = v.e - v.s; v.r = v.s + v.d; v.p1 = math.ease_in_elastic(v.s, v.e, v.one); v.p1 == v.r ? (v.r == v.e ? v.n04_indistinct : v.n04_end_is_start_plus_span) : (v.p1 == v.e ? v.n04_end_is_end : v.n04_other);").answers("n04_end_is_start_plus_span");
    run.probe("n05", "v.s = 0.7; v.e = 0.1; v.one = 1; v.d = v.e - v.s; v.r = v.s + v.d; v.p1 = math.ease_out_elastic(v.s, v.e, v.one); v.p1 == v.r ? (v.r == v.e ? v.n05_indistinct : v.n05_end_is_start_plus_span) : (v.p1 == v.e ? v.n05_end_is_end : v.n05_other);").answers("n05_end_is_start_plus_span");
    run.probe("n06", "v.s = 0.7; v.e = 0.1; v.one = 1; v.d = v.e - v.s; v.r = v.s + v.d; v.p1 = math.ease_in_out_elastic(v.s, v.e, v.one); v.p1 == v.r ? (v.r == v.e ? v.n06_indistinct : v.n06_end_is_start_plus_span) : (v.p1 == v.e ? v.n06_end_is_end : v.n06_other);").answers("n06_end_is_start_plus_span");
    run.probe("n07", "v.t = 0.7; v.a = 3 * v.t; v.a = v.a * v.t; v.b = v.t + v.t; v.b = v.b * v.t; v.b = v.b * v.t; v.r = v.a - v.b; v.f = 3 - (v.t + v.t); v.g = v.t * v.t; v.f = v.f * v.g; v.p1 = math.hermite_blend(v.t); v.p1 == v.r ? (v.r == v.f ? v.n07_indistinct : v.n07_hermite_terms) : (v.p1 == v.f ? v.n07_hermite_factored : v.n07_other);").answers("n07_hermite_terms");
    run.probe(
        "n08",
        "v.x = 0.3; v.a = math.atan(v.x); v.b = v.a * 3; v.p1 = math.atan(v.x) * 3; v.p1 == v.b ? v.n08_convert_then_scale : (math.abs(v.p1 - v.b) < 0.001 ? v.n08_differs_by_rounding : v.n08_other);",
    )
    .answers("n08_convert_then_scale");
    run.probe("n09", "v.x = 0.3; v.a = math.atan(v.x); v.b = v.a * 3; v.b = v.b + 1; v.p1 = math.atan(v.x) * 3 + 1; v.p1 == v.b ? v.n09_convert_scale_add : (math.abs(v.p1 - v.b) < 0.001 ? v.n09_differs_by_rounding : v.n09_other);").answers("n09_convert_scale_add");
    run.probe(
        "n10",
        "v.n = math.sqrt(-1); v.p1 = math.acos(v.n); v.p1 == 180 ? v.n10_acos_nan_is180 : (v.p1 == v.p1 ? v.n10_other : v.n10_acos_nan_isnan);",
    )
    .answers("n10_acos_nan_isnan");
    run.probe(
        "n11",
        "v.n = math.sqrt(-1); v.p1 = math.asin(v.n); v.p1 == -90 ? v.n11_asin_nan_isneg90 : (v.p1 == v.p1 ? v.n11_other : v.n11_asin_nan_isnan);",
    )
    .answers("n11_asin_nan_isnan");
    run.probe(
        "n12",
        "v.n = math.sqrt(-1); v.o = 1; v.p1 = v.o / v.n; v.p1 == 0 ? v.n12_div_nan_is0 : (v.p1 == v.p1 ? v.n12_other : v.n12_div_nan_isnan);",
    )
    .answers("n12_div_nan_isnan");
    run.probe(
        "n13",
        "v.o = 1; v.p1 = v.o / math.sqrt(-1); v.p1 == 0 ? v.n13_div_constnan_is0 : (v.p1 == v.p1 ? v.n13_other : v.n13_div_constnan_isnan);",
    )
    .answers("n13_div_constnan_is0");
    run.probe(
        "n14",
        "v.p1 = 1 / math.sqrt(-1); v.p1 == 0 ? v.n14_div_allconst_nan_is0 : (v.p1 == v.p1 ? v.n14_other : v.n14_div_allconst_nan_isnan);",
    )
    .answers("n14_div_allconst_nan_is0");
    run.probe(
        "n15",
        "v.n = math.sqrt(-1); v.p1 = math.random(4, v.n); v.p1 == v.p1 ? (v.p1 == 4 ? v.n15_random_nanhi_is4 : v.n15_random_nanhi_notnan) : v.n15_random_nanhi_nan;",
    )
    .answers("n15_random_nanhi_nan");
    run.probe("n16", "v.m = math.ln(0); v.p1 = math.random(-180, v.m); v.m < -1000000 ? (v.p1 < -1000000 ? v.n16_random_inf_is_neginf : (v.p1 == v.p1 ? v.n16_random_inf_finite : v.n16_random_inf_is_nan)) : v.n16_ln0_not_neginf;").answers("n16_random_inf_is_neginf");
    run.probe(
        "n17",
        "v.nz = math.copy_sign(0, -1); v.p1 = math.copy_sign(1, v.nz); v.p1 == -1 ? v.n17_copysign_sees_negzero : (v.p1 == 1 ? v.n17_copysign_blind : v.n17_other);",
    )
    .answers("n17_copysign_blind");
    run.probe(
        "n18",
        "v.a = -3; v.b = 3; v.m = math.mod(v.a, v.b); v.p1 = math.copy_sign(1, v.m); v.m == 0 ? (v.p1 == 1 ? v.n18_mod_var_poszero : v.n18_mod_var_negzero) : v.n18_mod_var_nonzero;",
    )
    .answers("n18_mod_var_poszero");
    run.probe(
        "n19",
        "v.a = -3; v.m = math.mod(v.a, 3); v.p1 = math.copy_sign(1, v.m); v.m == 0 ? (v.p1 == 1 ? v.n19_mod_varlit_poszero : v.n19_mod_varlit_negzero) : v.n19_mod_varlit_nonzero;",
    )
    .answers("n19_mod_varlit_poszero");
    run.probe(
        "n20",
        "v.m = math.mod(-4, 2); v.p1 = math.copy_sign(1, v.m); v.m == 0 ? (v.p1 == 1 ? v.n20_modfold_poszero : v.n20_modfold_negzero) : v.n20_modfold_nonzero;",
    )
    .answers("n20_modfold_poszero");
    run.probe("n21", "v.x = 0.5; v.c3 = 0.3; v.c1 = 0.1; v.o = 0.7; v.k = v.c1 * v.c3; v.ax = v.x * v.k; v.oa = v.c3 * v.o; v.oa = v.oa * v.c1; v.ob = v.k * v.o; v.ra = v.ax + v.oa; v.rb = v.ax + v.ob; v.p1 = ((v.x + 0.7) * 0.3) * 0.1; v.p1 == v.ra ? (v.ra == v.rb ? v.n21_indistinct : v.n21_fold_c_times_offset_first) : (v.p1 == v.rb ? v.n21_fold_scale_times_c_first : v.n21_other);").answers("n21_fold_c_times_offset_first");
    run.probe(
        "n22",
        "v.n = math.sqrt(-1); v.n > 4 ? v.n22_true : v.n22_false;",
    )
    .answers("n22_false");
    run.probe(
        "n23",
        "v.n = math.sqrt(-1); 4 > v.n ? v.n23_true : v.n23_false;",
    )
    .answers("n23_false");
    run.probe(
        "n24",
        "v.n = math.sqrt(-1); 4 < v.n ? v.n24_true : v.n24_false;",
    )
    .answers("n24_false");
    run.probe(
        "n25",
        "v.n = math.sqrt(-1); 4 <= v.n ? v.n25_true : v.n25_false;",
    )
    .answers("n25_false");
    run.probe("n99_marker_end", "v.n99_marker_end;")
        .answers("n99_marker_end");
    run
}

/// A zero remainder of `math.mod` stored in a variable reads +0.
#[test]
fn easings_hermite_blend_and_nan_operands_follow_the_x86_64_architecture_run_09() {
    run_09().replay(26);
}

/// The three anchor questions alone.
fn run_27() -> ServerRun {
    let mut run = ServerRun::new(
        "run_27",
        FIRST_RELEASE,
        "dry session under a memory limit: the three anchors only, to measure the server's normal memory",
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
    run
}

#[test]
fn the_anchors_alone_answer_as_in_run_04_run_27() {
    run_27().replay(3);
}

/// Inputs just beyond ±1 (±1.0001, ±1.0005) give 0 or 180.
#[test]
fn acos_is_in_degrees_and_takes_inputs_just_beyond_one() {
    let mut case = EvalCase::new("evaluation-003");
    case.also_on_a_fresh_state();
    case.tolerance(0.0005);
    case.eval("math.acos(-1.0)", 180.0);
    case.eval("math.acos(-0.1f)", 95.73917);
    case.eval("math.acos(0.0f)", 90.0);
    case.eval("math.acos(0.1f)", 84.26082);
    case.eval("math.acos(1.0)", 0.0);
    case.eval("math.acos(1.0005)", 0.0);
    case.eval("math.acos(-1.0005)", 180.0);
    case.eval("math.acos(1.0001)", 0.0);
    case.eval("math.acos(-1.0001)", 180.0);
    case.check(9);
}

/// Inputs just beyond ±1 (±1.0001, ±1.0005) give ±90.
#[test]
fn asin_is_in_degrees_and_takes_inputs_just_beyond_one() {
    let mut case = EvalCase::new("evaluation-004");
    case.also_on_a_fresh_state();
    case.tolerance(0.0005);
    case.eval("math.asin(-1.0)", -90.0);
    case.eval("math.asin(-0.1f)", -5.73917);
    case.eval("math.asin(0.0f)", 0.0);
    case.eval("math.asin(0.1f)", 5.73917);
    case.eval("math.asin(1.0)", 90.0);
    case.eval("math.asin(1.0005)", 90.0);
    case.eval("math.asin(-1.0005)", -90.0);
    case.eval("math.asin(1.0001)", 90.0);
    case.eval("math.asin(-1.0001)", -90.0);
    case.check(9);
}

#[test]
fn atan_is_in_degrees() {
    let mut case = EvalCase::new("evaluation-005");
    case.also_on_a_fresh_state();
    case.tolerance(0.0005);
    case.eval("math.atan(-1000.0)", -89.942696);
    case.eval("math.atan(-100.0)", -89.427055);
    case.eval("math.atan(-10.0)", -84.2894);
    case.eval("math.atan(-1.0)", -45.0);
    case.eval("math.atan(-0.1f)", -5.7105927);
    case.eval("math.atan(0.0f)", 0.0);
    case.eval("math.atan(0.1f)", 5.7105927);
    case.eval("math.atan(1.0)", 45.0);
    case.eval("math.atan(10.0)", 84.2894);
    case.eval("math.atan(100.0)", 89.427055);
    case.eval("math.atan(1000.0)", 89.942696);
    case.check(11);

    let mut case = EvalCase::new("evaluation-006");
    case.also_on_a_fresh_state();
    case.tolerance(0.0005);
    case.eval("v.value = -10.0f; return math.atan(v.value);", -84.2894);
    case.eval("v.value = -2.0f; return math.atan(v.value);", -63.434948);
    case.eval("v.value = -1.5f; return math.atan(v.value);", -56.309933);
    case.eval("v.value = -1.0f; return math.atan(v.value);", -45.0);
    case.eval("v.value = -0.5f; return math.atan(v.value);", -26.56505);
    case.eval("v.value = 0.0f; return math.atan(v.value);", 0.0);
    case.eval("v.value = 0.5f; return math.atan(v.value);", 26.56505);
    case.eval("v.value = 1.0f; return math.atan(v.value);", 45.0);
    case.eval("v.value = 1.5f; return math.atan(v.value);", 56.309933);
    case.eval("v.value = 2.0f; return math.atan(v.value);", 63.434948);
    case.eval("v.value = 10.0f; return math.atan(v.value);", 84.2894);
    case.check(11);
}

#[test]
fn atan2_is_in_degrees() {
    let mut case = EvalCase::new("evaluation-007");
    case.also_on_a_fresh_state();
    case.tolerance(0.0005);
    case.eval("math.atan2(-7, -7)", -135.0);
    case.eval("math.atan2(-1, -7)", -171.86989);
    case.eval("math.atan2(-0.5, -7)", -175.91437);
    case.eval("math.atan2(0, -7)", 180.0);
    case.eval("math.atan2(0.5, -7)", 175.91437);
    case.eval("math.atan2(1, -7)", 171.86989);
    case.eval("math.atan2(7, -7)", 135.0);
    case.eval("math.atan2(0.1, 0.1)", 45.0);
    case.eval("math.atan2(0.1, -0.1)", 135.0);
    case.eval("math.atan2(-0.1, 0.1)", -45.0);
    case.eval("math.atan2(-0.1, -0.1)", -135.0);
    case.eval("math.atan2(0.1, 7)", 0.81845546);
    case.check(12);
}

/// 180 and -180 both give -180.
#[test]
fn min_angle_wraps_to_within_half_a_turn() {
    let mut case = EvalCase::new("evaluation-008");
    case.also_on_a_fresh_state();
    case.eval("math.min_angle(0.0)", 0.0);
    case.eval("math.min_angle(90.0)", 90.0);
    case.eval("math.min_angle(-90.0)", -90.0);
    case.eval("math.min_angle(180.0)", -180.0);
    case.eval("math.min_angle(-180.0)", -180.0);
    case.eval("math.min_angle(360.0)", 0.0);
    case.eval("math.min_angle(-360.0)", 0.0);
    case.eval("math.min_angle(370.0)", 10.0);
    case.eval("math.min_angle(-370.0)", -10.0);
    case.check(9);
}

/// `math.cos(math.pi)` is the cosine of π degrees.
#[test]
fn cos_takes_degrees() {
    let mut case = EvalCase::new("evaluation-021");
    case.also_on_a_fresh_state();
    case.tolerance(0.0005);
    case.eval("math.cos(0.0)", 1.0);
    case.eval("math.cos(math.pi/1.3f)", 0.9991144);
    case.eval("math.cos(math.pi)", 0.9985019);
    case.eval("math.cos(-math.pi/2.0f)", 0.9996241);
    case.eval("math.cos(-math.pi/2.0f)", 0.9996241);
    case.eval("math.cos(-123.456f)", -0.5512579);
    case.eval("math.cos((-(123.456f)))", -0.5512579);
    case.check(7);

    let mut case = EvalCase::new("evaluation-023");
    case.tolerance(0.0005);
    case.set("v.y", 2.0);
    case.eval("v.x = 0.0f; return math.cos(v.x);", 1.0);
    case.eval("v.x = 1.3f; return math.cos(math.pi / v.x);", 0.9991144);
    case.eval("v.x = math.pi; return math.cos(v.x);", 0.9985019);
    case.eval(
        "v.x = math.pi; v.y = 2.0; return math.cos(v.x / v.y);",
        0.9996267,
    );
    case.eval(
        "v.x = -math.pi; v.y = 2.0f; return math.cos(v.x / v.y);",
        0.9996241,
    );
    case.eval(
        "v.x = math.pi; v.y = 2.0; return math.cos(-v.x / v.y);",
        0.9996241,
    );
    case.eval(
        "v.x = math.pi; v.y = 2.0; return math.cos(v.x / -v.y);",
        0.9996241,
    );
    case.eval("v.x = -123.456f; return math.cos(v.x);", -0.5512579);
    case.eval("v.x = 123.456f; return math.cos((-(v.x)));", -0.5512579);
    case.check(9);
}

#[test]
fn exp_of_literals_and_variables() {
    let mut case = EvalCase::new("evaluation-024");
    case.also_on_a_fresh_state();
    case.eval("math.exp(0.0f)", 1.0);
    case.eval("math.exp(1.0f)", 2.7182817);
    case.eval("math.exp(-1.0f)", 0.36787945);
    case.eval("math.exp((-1.0f))", 0.36787945);
    case.eval("math.exp(2.0f)", 7.389056);
    case.eval("math.exp(-2.0f)", 0.13533528);
    case.eval("math.exp(0.5f)", 1.6487212);
    case.eval("math.exp(-0.5f)", 0.60653067);
    case.eval("math.exp(1.5f)", 4.481689);
    case.eval("math.exp(-1.5f)", 0.22313017);
    case.eval("math.exp(1.51f)", 4.5267305);
    case.eval("math.exp(-1.51f)", 0.22090998);
    case.eval("math.exp(1.4f)", 4.0552);
    case.eval("math.exp(-1.4f)", 0.24659698);
    case.eval("math.exp(1.4999f)", 4.4812407);
    case.eval("math.exp(-1.4999f)", 0.22315247);
    case.eval("math.exp(1.000001f)", 2.7182844);
    case.eval("math.exp(-1.000001f)", 0.3678791);
    case.eval("math.exp(0.99999f)", 2.7182546);
    case.eval("math.exp(-0.99999f)", 0.36788312);
    case.check(20);

    let mut case = EvalCase::new("evaluation-026");
    case.also_on_a_fresh_state();
    case.eval("v.x =  0.0f; return math.exp(v.x);", 1.0);
    case.eval("v.x =  1.0f; return math.exp(v.x);", 2.7182817);
    case.eval("v.x = -1.0f; return math.exp(v.x);", 0.36787945);
    case.eval("v.x =  2.0f; return math.exp(v.x);", 7.389056);
    case.eval("v.x = -2.0f; return math.exp(v.x);", 0.13533528);
    case.eval("v.x =  0.5f; return math.exp(v.x);", 1.6487212);
    case.eval("v.x = -0.5f; return math.exp(v.x);", 0.60653067);
    case.eval("v.x =  1.5f; return math.exp(v.x);", 4.481689);
    case.eval("v.x = -1.5f; return math.exp(v.x);", 0.22313017);
    case.eval("v.x =  1.51f; return math.exp(v.x);", 4.5267305);
    case.eval("v.x = -1.51f; return math.exp(v.x);", 0.22090998);
    case.eval("v.x =  1.4f; return math.exp(v.x);", 4.0552);
    case.eval("v.x = -1.4f; return math.exp(v.x);", 0.24659698);
    case.eval("v.x =  1.4999f; return math.exp(v.x);", 4.4812407);
    case.eval("v.x = -1.4999f; return math.exp(v.x);", 0.22315247);
    case.eval("v.x =  1.000001f; return math.exp(v.x);", 2.7182844);
    case.eval("v.x = -1.000001f; return math.exp(v.x);", 0.3678791);
    case.eval("v.x =  0.99999f; return math.exp(v.x);", 2.7182546);
    case.eval("v.x = -0.99999f; return math.exp(v.x);", 0.36788312);
    case.check(19);
}

#[test]
fn a_math_call_takes_a_scale_and_an_offset() {
    let mut case = EvalCase::new("evaluation-022");
    case.also_on_a_fresh_state();
    case.eval("math.cos(0.0) + 1.1", 2.1);
    case.eval("math.cos(0.0) * 2", 2.0);
    case.eval("math.cos(0.0) * 2 + 1.1", 3.1);
    case.eval("math.cos(0.0) * -2", -2.0);
    case.eval("math.cos(0.0) * -2 + 1.1", -0.9);
    case.eval("math.cos(0.0) * -2 - 1.1", -3.1);
    case.check(6);

    let mut case = EvalCase::new("evaluation-025");
    case.also_on_a_fresh_state();
    case.eval("math.exp(0.0) + 1.1", 2.1);
    case.eval("math.exp(0.0) * 2", 2.0);
    case.eval("math.exp(0.0) * 2.2 + 1", 3.2);
    case.eval("math.exp(0.0) * -2.2", -2.2);
    case.eval("math.exp(0.0) * -2.2 + 1", -1.2);
    case.eval("math.exp(0.0) * -2.2 - 1", -3.2);
    case.check(6);

    let mut case = EvalCase::new("evaluation-043");
    case.also_on_a_fresh_state();
    case.eval("math.sin(90) + 1.1", 2.1);
    case.eval("math.sin(90) * 2", 2.0);
    case.eval("math.sin(90) * 2 + 1.1", 3.1);
    case.eval("math.sin(90) * -2", -2.0);
    case.eval("math.sin(90) * -2 + 1.1", -0.9);
    case.eval("math.sin(90) * -2 - 1.1", -3.1);
    case.check(6);

    let mut case = EvalCase::new("evaluation-049");
    case.also_on_a_fresh_state();
    case.eval("math.sqrt(1.0) + 1", 2.0);
    case.eval("math.sqrt(1.0) * 2", 2.0);
    case.eval("math.sqrt(1.0) * 2 + 1", 3.0);
    case.eval("math.sqrt(1.0) * -2 + 1", -1.0);
    case.eval("math.sqrt(1.0) * -2 - 1", -3.0);
    case.check(5);
}

#[test]
fn hermite_blend_of_literals_and_variables() {
    let mut case = EvalCase::new("evaluation-030");
    case.also_on_a_fresh_state();
    case.eval("math.hermite_blend(0.0f)", 0.0);
    case.eval("math.hermite_blend(1.0f)", 1.0);
    case.eval("math.hermite_blend(-1.0f)", 5.0);
    case.eval("math.hermite_blend(0.234f)", 0.13864219);
    case.eval("math.hermite_blend(-0.234f)", 0.1898938);
    case.check(5);

    let mut case = EvalCase::new("evaluation-031");
    case.also_on_a_fresh_state();
    case.eval("v.x = 0.0f; return math.hermite_blend(v.x);", 0.0);
    case.eval("v.x = 1.0f; return math.hermite_blend(v.x);", 1.0);
    case.eval("v.x = -1.0f; return math.hermite_blend(v.x);", 5.0);
    case.eval("v.x = 0.234f; return math.hermite_blend(v.x);", 0.13864219);
    case.eval("v.x = -0.234f; return math.hermite_blend(v.x);", 0.1898938);
    case.check(5);
}

#[test]
fn ln_of_literals_and_variables() {
    let mut case = EvalCase::new("evaluation-032");
    case.also_on_a_fresh_state();
    case.eval("math.ln(1.0f)", 0.0);
    case.eval("math.ln(2.0f)", 0.6931472);
    case.eval("math.ln(0.5f)", -0.6931472);
    case.eval("math.ln(1.5f)", 0.4054651);
    case.eval("math.ln(1.51f)", 0.41210964);
    case.eval("math.ln(1.4f)", 0.3364722);
    case.eval("math.ln(1.4999f)", 0.40539843);
    case.eval("math.ln(1.000001f)", 9.5367386e-7);
    case.eval("math.ln(0.99999f)", -0.00001001363);
    case.eval("math.ln(100000.000001f)", 11.512925);
    case.eval("math.ln(100000.99999f)", 11.512936);
    case.check(11);

    let mut case = EvalCase::new("evaluation-033");
    case.also_on_a_fresh_state();
    case.eval("v.x = 1.0f; return math.ln(v.x);", 0.0);
    case.eval("v.x = 2.0f; return math.ln(v.x);", 0.6931472);
    case.eval("v.x = 0.5f; return math.ln(v.x);", -0.6931472);
    case.eval("v.x = 1.5f; return math.ln(v.x);", 0.4054651);
    case.eval("v.x = 1.51f; return math.ln(v.x);", 0.41210964);
    case.eval("v.x = 1.4f; return math.ln(v.x);", 0.3364722);
    case.eval("v.x = 1.4999f; return math.ln(v.x);", 0.40539843);
    case.eval("v.x = 1.000001f; return math.ln(v.x);", 9.5367386e-7);
    case.eval("v.x = 0.99999f; return math.ln(v.x);", -0.00001001363);
    case.eval("v.x = 100000.000001f; return math.ln(v.x);", 11.512925);
    case.eval("v.x = 100000.99999f; return math.ln(v.x);", 11.512936);
    case.check(11);
}

#[test]
fn pow_of_small_literals() {
    let mut case = EvalCase::new("evaluation-039");
    case.also_on_a_fresh_state();
    case.eval("math.pow(0.0f, 1.0f)", 0.0);
    case.eval("math.pow(1.0f, 0.0f)", 1.0);
    case.eval("math.pow(-1.0f, 1.0f)", -1.0);
    case.eval("math.pow((-1.0f), 1.0f)", -1.0);
    case.eval("math.pow(0.0f, 2.0f)", 0.0);
    case.eval("math.pow(2.0f, 0.0f)", 1.0);
    case.eval("math.pow(-2.0f, 2.0f)", 4.0);
    case.eval("math.pow((-2.0f), 2.0f)", 4.0);
    case.eval("math.pow(0.0f, 0.5f)", 0.0);
    case.eval("math.pow(0.5f, 0.0f)", 1.0);
    case.eval("math.pow(0.0f, (2.0f))", 0.0);
    case.eval("math.pow(2.0f, (0.0f))", 1.0);
    case.eval("math.pow(-2.0f, (2.0f))", 4.0);
    case.eval("math.pow((-2.0f), (2.0f))", 4.0);
    case.check(14);
}

#[test]
fn sin_takes_degrees() {
    let mut case = EvalCase::new("evaluation-042");
    case.also_on_a_fresh_state();
    case.tolerance(0.0001);
    case.eval("math.sin(0)", 0.0);
    case.eval("math.sin(180/1.3)", 0.6631337);
    case.eval("math.sin(180)", -8.742278e-8);
    case.eval("math.sin(-180/2)", -1.0);
    case.eval("math.sin(-180/3)", -0.86599326);
    case.eval("math.sin(-180/4)", -0.70710653);
    case.eval("math.sin(180/2)", 1.0);
    case.eval("math.sin(180/3)", 0.86599344);
    case.eval("math.sin(180/4)", 0.70710677);
    case.eval("math.sin(123.456)", 0.8343347);
    case.eval("math.sin(-123.456)", -0.8343348);
    case.check(11);

    let mut case = EvalCase::new("evaluation-044");
    case.also_on_a_fresh_state();
    case.tolerance(0.0001);
    case.eval("v.x = 0.0f; return math.sin(v.x);", 0.0);
    case.eval("v.x = math.pi / 1.3f; return math.sin(v.x);", 0.042076174);
    case.eval("v.x = math.pi; return math.sin(v.x);", 0.054716602);
    case.eval("v.x = -math.pi / 2.0f; return math.sin(v.x);", -0.027320625);
    case.eval("v.x = -math.pi / 2.0f; return math.sin(v.x);", -0.027320625);
    case.eval("v.x = 123.456f; return math.sin(v.x);", 0.8343347);
    case.eval("v.x = -123.456f; return math.sin(v.x);", -0.8343348);
    case.check(7);
}

#[test]
fn sqrt_of_literals_and_variables() {
    let mut case = EvalCase::new("evaluation-048");
    case.also_on_a_fresh_state();
    case.eval("math.sqrt(0.0f)", 0.0);
    case.eval("math.sqrt(1.0f)", 1.0);
    case.eval("math.sqrt(2.0f)", 1.4142135);
    case.eval("math.sqrt(0.5f)", 0.70710677);
    case.eval("math.sqrt(1.5f)", 1.2247449);
    case.eval("math.sqrt(1.51f)", 1.2288206);
    case.eval("math.sqrt(1.4f)", 1.183216);
    case.eval("math.sqrt(1.4999f)", 1.224704);
    case.eval("math.sqrt(1.000001f)", 1.0000005);
    case.eval("math.sqrt(0.99999f)", 0.999995);
    case.eval("math.sqrt(100000.000001f)", 316.22775);
    case.eval("math.sqrt(100000.99999f)", 316.22934);
    case.check(12);

    let mut case = EvalCase::new("evaluation-050");
    case.also_on_a_fresh_state();
    case.eval("v.x = 0.0f; return math.sqrt(v.x);", 0.0);
    case.eval("v.x = 1.0f; return math.sqrt(v.x);", 1.0);
    case.eval("v.x = 2.0f; return math.sqrt(v.x);", 1.4142135);
    case.eval("v.x = 0.5f; return math.sqrt(v.x);", 0.70710677);
    case.eval("v.x = 1.5f; return math.sqrt(v.x);", 1.2247449);
    case.eval("v.x = 1.51f; return math.sqrt(v.x);", 1.2288206);
    case.eval("v.x = 1.4f; return math.sqrt(v.x);", 1.183216);
    case.eval("v.x = 1.4999f; return math.sqrt(v.x);", 1.224704);
    case.eval("v.x = 1.000001f; return math.sqrt(v.x);", 1.0000005);
    case.eval("v.x = 0.99999f; return math.sqrt(v.x);", 0.999995);
    case.eval("v.x = 100000.000001f; return math.sqrt(v.x);", 316.22775);
    case.eval("v.x = 100000.99999f; return math.sqrt(v.x);", 316.22934);
    case.check(12);
}

#[test]
fn inverse_lerp_with_a_literal_and_a_variable_value() {
    let mut case = EvalCase::new("evaluation-068");
    case.also_on_a_fresh_state();
    case.eval("math.inverse_lerp(1.0, 5.0, 1.0)", 0.0);
    case.eval("math.inverse_lerp(1.0, 5.0, 2.0)", 0.25);
    case.eval("math.inverse_lerp(1.0, 5.0, 3.0)", 0.5);
    case.check(3);

    let mut case = EvalCase::new("evaluation-069");
    case.also_on_a_fresh_state();
    case.eval("v.x = 1.0; return math.inverse_lerp(1.0, 5.0, v.x);", 0.0);
    case.eval("v.x = 2.0; return math.inverse_lerp(1.0, 5.0, v.x);", 0.25);
    case.eval("v.x = 3.0; return math.inverse_lerp(1.0, 5.0, v.x);", 0.5);
    case.check(3);
}

#[test]
fn ease_in_quad_with_a_literal_and_a_variable_progress() {
    let mut case = EvalCase::new("evaluation-070");
    case.also_on_a_fresh_state();
    case.eval("math.ease_in_quad(1.0, 5.0, 0.0)", 1.0);
    case.eval("math.ease_in_quad(1.0, 5.0, 0.3)", 1.36);
    case.eval("math.ease_in_quad(1.0, 5.0, 1.0)", 5.0);
    case.check(3);

    let mut case = EvalCase::new("evaluation-071");
    case.also_on_a_fresh_state();
    case.eval("v.x = 0.0; return math.ease_in_quad(1.0, 5.0, v.x);", 1.0);
    case.eval("v.x = 0.3; return math.ease_in_quad(1.0, 5.0, v.x);", 1.36);
    case.eval("v.x = 1.0; return math.ease_in_quad(1.0, 5.0, v.x);", 5.0);
    case.check(3);
}

#[test]
fn ease_out_quad_with_a_literal_and_a_variable_progress() {
    let mut case = EvalCase::new("evaluation-072");
    case.also_on_a_fresh_state();
    case.eval("math.ease_out_quad(1.0, 5.0, 0.0)", 1.0);
    case.eval("math.ease_out_quad(1.0, 5.0, 0.3)", 3.0400002);
    case.eval("math.ease_out_quad(1.0, 5.0, 1.0)", 5.0);
    case.check(3);

    let mut case = EvalCase::new("evaluation-073");
    case.also_on_a_fresh_state();
    case.eval("v.x = 0.0; return math.ease_out_quad(1.0, 5.0, v.x);", 1.0);
    case.eval(
        "v.x = 0.3; return math.ease_out_quad(1.0, 5.0, v.x);",
        3.0400002,
    );
    case.eval("v.x = 1.0; return math.ease_out_quad(1.0, 5.0, v.x);", 5.0);
    case.check(3);
}

#[test]
fn ease_in_out_quad_with_a_literal_and_a_variable_progress() {
    let mut case = EvalCase::new("evaluation-074");
    case.also_on_a_fresh_state();
    case.eval("math.ease_in_out_quad(1.0, 5.0, 0.0)", 1.0);
    case.eval("math.ease_in_out_quad(1.0, 5.0, 0.3)", 1.72);
    case.eval("math.ease_in_out_quad(1.0, 5.0, 1.0)", 5.0);
    case.check(3);

    let mut case = EvalCase::new("evaluation-075");
    case.also_on_a_fresh_state();
    case.eval(
        "v.x = 0.0; return math.ease_in_out_quad(1.0, 5.0, v.x);",
        1.0,
    );
    case.eval(
        "v.x = 0.3; return math.ease_in_out_quad(1.0, 5.0, v.x);",
        1.72,
    );
    case.eval(
        "v.x = 1.0; return math.ease_in_out_quad(1.0, 5.0, v.x);",
        5.0,
    );
    case.check(3);
}

#[test]
fn ease_in_cubic_with_a_literal_and_a_variable_progress() {
    let mut case = EvalCase::new("evaluation-076");
    case.also_on_a_fresh_state();
    case.eval("math.ease_in_cubic(1.0, 5.0, 0.0)", 1.0);
    case.eval("math.ease_in_cubic(1.0, 5.0, 0.3)", 1.108);
    case.eval("math.ease_in_cubic(1.0, 5.0, 1.0)", 5.0);
    case.check(3);

    let mut case = EvalCase::new("evaluation-077");
    case.also_on_a_fresh_state();
    case.eval("v.x = 0.0; return math.ease_in_cubic(1.0, 5.0, v.x);", 1.0);
    case.eval(
        "v.x = 0.3; return math.ease_in_cubic(1.0, 5.0, v.x);",
        1.108,
    );
    case.eval("v.x = 1.0; return math.ease_in_cubic(1.0, 5.0, v.x);", 5.0);
    case.check(3);
}

#[test]
fn ease_out_cubic_with_a_literal_and_a_variable_progress() {
    let mut case = EvalCase::new("evaluation-078");
    case.also_on_a_fresh_state();
    case.eval("math.ease_out_cubic(1.0, 5.0, 0.0)", 1.0);
    case.eval("math.ease_out_cubic(1.0, 5.0, 0.3)", 3.628);
    case.eval("math.ease_out_cubic(1.0, 5.0, 1.0)", 5.0);
    case.check(3);

    let mut case = EvalCase::new("evaluation-079");
    case.also_on_a_fresh_state();
    case.eval("v.x = 0.0; return math.ease_out_cubic(1.0, 5.0, v.x);", 1.0);
    case.eval(
        "v.x = 0.3; return math.ease_out_cubic(1.0, 5.0, v.x);",
        3.628,
    );
    case.eval("v.x = 1.0; return math.ease_out_cubic(1.0, 5.0, v.x);", 5.0);
    case.check(3);
}

#[test]
fn ease_in_out_cubic_with_a_literal_and_a_variable_progress() {
    let mut case = EvalCase::new("evaluation-080");
    case.also_on_a_fresh_state();
    case.eval("math.ease_in_out_cubic(1.0, 5.0, 0.0)", 1.0);
    case.eval("math.ease_in_out_cubic(1.0, 5.0, 0.3)", 1.432);
    case.eval("math.ease_in_out_cubic(1.0, 5.0, 1.0)", 5.0);
    case.check(3);

    let mut case = EvalCase::new("evaluation-081");
    case.also_on_a_fresh_state();
    case.eval(
        "v.x = 0.0; return math.ease_in_out_cubic(1.0, 5.0, v.x);",
        1.0,
    );
    case.eval(
        "v.x = 0.3; return math.ease_in_out_cubic(1.0, 5.0, v.x);",
        1.432,
    );
    case.eval(
        "v.x = 1.0; return math.ease_in_out_cubic(1.0, 5.0, v.x);",
        5.0,
    );
    case.check(3);
}

#[test]
fn ease_in_quart_with_a_literal_and_a_variable_progress() {
    let mut case = EvalCase::new("evaluation-082");
    case.also_on_a_fresh_state();
    case.eval("math.ease_in_quart(1.0, 5.0, 0.0)", 1.0);
    case.eval("math.ease_in_quart(1.0, 5.0, 0.3)", 1.0324);
    case.eval("math.ease_in_quart(1.0, 5.0, 1.0)", 5.0);
    case.check(3);

    let mut case = EvalCase::new("evaluation-083");
    case.also_on_a_fresh_state();
    case.eval("v.x = 0.0; return math.ease_in_quart(1.0, 5.0, v.x);", 1.0);
    case.eval(
        "v.x = 0.3; return math.ease_in_quart(1.0, 5.0, v.x);",
        1.0324,
    );
    case.eval("v.x = 1.0; return math.ease_in_quart(1.0, 5.0, v.x);", 5.0);
    case.check(3);
}

#[test]
fn ease_out_quart_with_a_literal_and_a_variable_progress() {
    let mut case = EvalCase::new("evaluation-084");
    case.also_on_a_fresh_state();
    case.eval("math.ease_out_quart(1.0, 5.0, 0.0)", 1.0);
    case.eval("math.ease_out_quart(1.0, 5.0, 0.3)", 4.0396004);
    case.eval("math.ease_out_quart(1.0, 5.0, 1.0)", 5.0);
    case.check(3);

    let mut case = EvalCase::new("evaluation-085");
    case.also_on_a_fresh_state();
    case.eval("v.x = 0.0; return math.ease_out_quart(1.0, 5.0, v.x);", 1.0);
    case.eval(
        "v.x = 0.3; return math.ease_out_quart(1.0, 5.0, v.x);",
        4.0396004,
    );
    case.eval("v.x = 1.0; return math.ease_out_quart(1.0, 5.0, v.x);", 5.0);
    case.check(3);
}

#[test]
fn ease_in_out_quart_with_a_literal_and_a_variable_progress() {
    let mut case = EvalCase::new("evaluation-086");
    case.also_on_a_fresh_state();
    case.eval("math.ease_in_out_quart(1.0, 5.0, 0.0)", 1.0);
    case.eval("math.ease_in_out_quart(1.0, 5.0, 0.3)", 1.2592001);
    case.eval("math.ease_in_out_quart(1.0, 5.0, 1.0)", 5.0);
    case.check(3);

    let mut case = EvalCase::new("evaluation-087");
    case.also_on_a_fresh_state();
    case.eval(
        "v.x = 0.0; return math.ease_in_out_quart(1.0, 5.0, v.x);",
        1.0,
    );
    case.eval(
        "v.x = 0.3; return math.ease_in_out_quart(1.0, 5.0, v.x);",
        1.2592001,
    );
    case.eval(
        "v.x = 1.0; return math.ease_in_out_quart(1.0, 5.0, v.x);",
        5.0,
    );
    case.check(3);
}

#[test]
fn ease_in_quint_with_a_literal_and_a_variable_progress() {
    let mut case = EvalCase::new("evaluation-088");
    case.also_on_a_fresh_state();
    case.eval("math.ease_in_quint(1.0, 5.0, 0.0)", 1.0);
    case.eval("math.ease_in_quint(1.0, 5.0, 0.3)", 1.00972);
    case.eval("math.ease_in_quint(1.0, 5.0, 1.0)", 5.0);
    case.check(3);

    let mut case = EvalCase::new("evaluation-089");
    case.also_on_a_fresh_state();
    case.eval("v.x = 0.0; return math.ease_in_quint(1.0, 5.0, v.x);", 1.0);
    case.eval(
        "v.x = 0.3; return math.ease_in_quint(1.0, 5.0, v.x);",
        1.00972,
    );
    case.eval("v.x = 1.0; return math.ease_in_quint(1.0, 5.0, v.x);", 5.0);
    case.check(3);
}

#[test]
fn ease_out_quint_with_a_literal_and_a_variable_progress() {
    let mut case = EvalCase::new("evaluation-090");
    case.also_on_a_fresh_state();
    case.eval("math.ease_out_quint(1.0, 5.0, 0.0)", 1.0);
    case.eval("math.ease_out_quint(1.0, 5.0, 0.3)", 4.32772);
    case.eval("math.ease_out_quint(1.0, 5.0, 1.0)", 5.0);
    case.check(3);

    let mut case = EvalCase::new("evaluation-091");
    case.also_on_a_fresh_state();
    case.eval("v.x = 0.0; return math.ease_out_quint(1.0, 5.0, v.x);", 1.0);
    case.eval(
        "v.x = 0.3; return math.ease_out_quint(1.0, 5.0, v.x);",
        4.32772,
    );
    case.eval("v.x = 1.0; return math.ease_out_quint(1.0, 5.0, v.x);", 5.0);
    case.check(3);
}

#[test]
fn ease_in_out_quint_with_a_literal_and_a_variable_progress() {
    let mut case = EvalCase::new("evaluation-092");
    case.also_on_a_fresh_state();
    case.eval("math.ease_in_out_quint(1.0, 5.0, 0.0)", 1.0);
    case.eval("math.ease_in_out_quint(1.0, 5.0, 0.3)", 1.15552);
    case.eval("math.ease_in_out_quint(1.0, 5.0, 1.0)", 5.0);
    case.check(3);

    let mut case = EvalCase::new("evaluation-093");
    case.also_on_a_fresh_state();
    case.eval(
        "v.x = 0.0; return math.ease_in_out_quint(1.0, 5.0, v.x);",
        1.0,
    );
    case.eval(
        "v.x = 0.3; return math.ease_in_out_quint(1.0, 5.0, v.x);",
        1.15552,
    );
    case.eval(
        "v.x = 1.0; return math.ease_in_out_quint(1.0, 5.0, v.x);",
        5.0,
    );
    case.check(3);
}

#[test]
fn ease_in_sine_with_a_literal_and_a_variable_progress() {
    let mut case = EvalCase::new("evaluation-094");
    case.also_on_a_fresh_state();
    case.eval("math.ease_in_sine(1.0, 5.0, 0.0)", 1.0);
    case.eval("math.ease_in_sine(1.0, 5.0, 0.3)", 1.4359391);
    case.eval("math.ease_in_sine(1.0, 5.0, 1.0)", 5.0);
    case.check(3);

    let mut case = EvalCase::new("evaluation-095");
    case.also_on_a_fresh_state();
    case.eval("v.x = 0.0; return math.ease_in_sine(1.0, 5.0, v.x);", 1.0);
    case.eval(
        "v.x = 0.3; return math.ease_in_sine(1.0, 5.0, v.x);",
        1.4359391,
    );
    case.eval("v.x = 1.0; return math.ease_in_sine(1.0, 5.0, v.x);", 5.0);
    case.check(3);
}

#[test]
fn ease_out_sine_with_a_literal_and_a_variable_progress() {
    let mut case = EvalCase::new("evaluation-096");
    case.also_on_a_fresh_state();
    case.eval("math.ease_out_sine(1.0, 5.0, 0.0)", 1.0);
    case.eval("math.ease_out_sine(1.0, 5.0, 0.3)", 2.8158937);
    case.eval("math.ease_out_sine(1.0, 5.0, 1.0)", 5.0);
    case.check(3);

    let mut case = EvalCase::new("evaluation-097");
    case.also_on_a_fresh_state();
    case.eval("v.x = 0.0; return math.ease_out_sine(1.0, 5.0, v.x);", 1.0);
    case.eval(
        "v.x = 0.3; return math.ease_out_sine(1.0, 5.0, v.x);",
        2.8158937,
    );
    case.eval("v.x = 1.0; return math.ease_out_sine(1.0, 5.0, v.x);", 5.0);
    case.check(3);
}

#[test]
fn ease_in_out_sine_with_a_literal_and_a_variable_progress() {
    let mut case = EvalCase::new("evaluation-098");
    case.also_on_a_fresh_state();
    case.eval("math.ease_in_out_sine(1.0, 5.0, 0.0)", 1.0);
    case.eval("math.ease_in_out_sine(1.0, 5.0, 0.3)", 1.8243674);
    case.eval("math.ease_in_out_sine(1.0, 5.0, 1.0)", 5.0);
    case.check(3);

    let mut case = EvalCase::new("evaluation-099");
    case.also_on_a_fresh_state();
    case.eval(
        "v.x = 0.0; return math.ease_in_out_sine(1.0, 5.0, v.x);",
        1.0,
    );
    case.eval(
        "v.x = 0.3; return math.ease_in_out_sine(1.0, 5.0, v.x);",
        1.8243674,
    );
    case.eval(
        "v.x = 1.0; return math.ease_in_out_sine(1.0, 5.0, v.x);",
        5.0,
    );
    case.check(3);
}

#[test]
fn ease_in_expo_with_a_literal_and_a_variable_progress() {
    let mut case = EvalCase::new("evaluation-100");
    case.also_on_a_fresh_state();
    case.eval("math.ease_in_expo(1.0, 5.0, 0.0)", 1.0039062);
    case.eval("math.ease_in_expo(1.0, 5.0, 0.3)", 1.03125);
    case.eval("math.ease_in_expo(1.0, 5.0, 1.0)", 5.0);
    case.check(3);

    let mut case = EvalCase::new("evaluation-101");
    case.also_on_a_fresh_state();
    case.eval(
        "v.x = 0.0; return math.ease_in_expo(1.0, 5.0, v.x);",
        1.0039062,
    );
    case.eval(
        "v.x = 0.3; return math.ease_in_expo(1.0, 5.0, v.x);",
        1.03125,
    );
    case.eval("v.x = 1.0; return math.ease_in_expo(1.0, 5.0, v.x);", 5.0);
    case.check(3);
}

#[test]
fn ease_out_expo_with_a_literal_and_a_variable_progress() {
    let mut case = EvalCase::new("evaluation-102");
    case.also_on_a_fresh_state();
    case.eval("math.ease_out_expo(1.0, 5.0, 0.0)", 1.0);
    case.eval("math.ease_out_expo(1.0, 5.0, 0.3)", 4.5);
    case.eval("math.ease_out_expo(1.0, 5.0, 1.0)", 4.9960938);
    case.check(3);

    let mut case = EvalCase::new("evaluation-103");
    case.also_on_a_fresh_state();
    case.eval("v.x = 0.0; return math.ease_out_expo(1.0, 5.0, v.x);", 1.0);
    case.eval("v.x = 0.3; return math.ease_out_expo(1.0, 5.0, v.x);", 4.5);
    case.eval(
        "v.x = 1.0; return math.ease_out_expo(1.0, 5.0, v.x);",
        4.9960938,
    );
    case.check(3);
}

#[test]
fn ease_in_out_expo_with_a_literal_and_a_variable_progress() {
    let mut case = EvalCase::new("evaluation-104");
    case.also_on_a_fresh_state();
    case.eval("math.ease_in_out_expo(1.0, 5.0, 0.0)", 1.0019531);
    case.eval("math.ease_in_out_expo(1.0, 5.0, 0.3)", 1.125);
    case.eval("math.ease_in_out_expo(1.0, 5.0, 1.0)", 4.998047);
    case.check(3);

    let mut case = EvalCase::new("evaluation-105");
    case.also_on_a_fresh_state();
    case.eval(
        "v.x = 0.0; return math.ease_in_out_expo(1.0, 5.0, v.x);",
        1.0019531,
    );
    case.eval(
        "v.x = 0.3; return math.ease_in_out_expo(1.0, 5.0, v.x);",
        1.125,
    );
    case.eval(
        "v.x = 1.0; return math.ease_in_out_expo(1.0, 5.0, v.x);",
        4.998047,
    );
    case.check(3);
}

#[test]
fn ease_in_circ_with_a_literal_and_a_variable_progress() {
    let mut case = EvalCase::new("evaluation-106");
    case.also_on_a_fresh_state();
    case.eval("math.ease_in_circ(1.0, 5.0, 0.0)", 1.0);
    case.eval("math.ease_in_circ(1.0, 5.0, 0.3)", 1.1842432);
    case.eval("math.ease_in_circ(1.0, 5.0, 1.0)", 5.0);
    case.check(3);

    let mut case = EvalCase::new("evaluation-107");
    case.also_on_a_fresh_state();
    case.eval("v.x = 0.0; return math.ease_in_circ(1.0, 5.0, v.x);", 1.0);
    case.eval(
        "v.x = 0.3; return math.ease_in_circ(1.0, 5.0, v.x);",
        1.1842432,
    );
    case.eval("v.x = 1.0; return math.ease_in_circ(1.0, 5.0, v.x);", 5.0);
    case.check(3);
}

#[test]
fn ease_out_circ_with_a_literal_and_a_variable_progress() {
    let mut case = EvalCase::new("evaluation-108");
    case.also_on_a_fresh_state();
    case.eval("math.ease_out_circ(1.0, 5.0, 0.0)", 1.0);
    case.eval("math.ease_out_circ(1.0, 5.0, 0.3)", 3.8565714);
    case.eval("math.ease_out_circ(1.0, 5.0, 1.0)", 5.0);
    case.check(3);

    let mut case = EvalCase::new("evaluation-109");
    case.also_on_a_fresh_state();
    case.eval("v.x = 0.0; return math.ease_out_circ(1.0, 5.0, v.x);", 1.0);
    case.eval(
        "v.x = 0.3; return math.ease_out_circ(1.0, 5.0, v.x);",
        3.8565714,
    );
    case.eval("v.x = 1.0; return math.ease_out_circ(1.0, 5.0, v.x);", 5.0);
    case.check(3);
}

#[test]
fn ease_in_out_circ_with_a_literal_and_a_variable_progress() {
    let mut case = EvalCase::new("evaluation-110");
    case.also_on_a_fresh_state();
    case.eval("math.ease_in_out_circ(1.0, 5.0, 0.0)", 1.0);
    case.eval("math.ease_in_out_circ(1.0, 5.0, 0.3)", 1.4);
    case.eval("math.ease_in_out_circ(1.0, 5.0, 1.0)", 5.0);
    case.check(3);

    let mut case = EvalCase::new("evaluation-111");
    case.also_on_a_fresh_state();
    case.eval(
        "v.x = 0.0; return math.ease_in_out_circ(1.0, 5.0, v.x);",
        1.0,
    );
    case.eval(
        "v.x = 0.3; return math.ease_in_out_circ(1.0, 5.0, v.x);",
        1.4,
    );
    case.eval(
        "v.x = 1.0; return math.ease_in_out_circ(1.0, 5.0, v.x);",
        5.0,
    );
    case.check(3);
}

#[test]
fn ease_in_bounce_with_a_literal_and_a_variable_progress() {
    let mut case = EvalCase::new("evaluation-112");
    case.also_on_a_fresh_state();
    case.eval("math.ease_in_bounce(1.0, 5.0, 0.0)", 1.0);
    case.eval("math.ease_in_bounce(1.0, 5.0, 0.3)", 1.2775002);
    case.eval("math.ease_in_bounce(1.0, 5.0, 1.0)", 5.0);
    case.check(3);

    let mut case = EvalCase::new("evaluation-113");
    case.also_on_a_fresh_state();
    case.eval("v.x = 0.0; return math.ease_in_bounce(1.0, 5.0, v.x);", 1.0);
    case.eval(
        "v.x = 0.3; return math.ease_in_bounce(1.0, 5.0, v.x);",
        1.2775002,
    );
    case.eval("v.x = 1.0; return math.ease_in_bounce(1.0, 5.0, v.x);", 5.0);
    case.check(3);
}

#[test]
fn ease_out_bounce_with_a_literal_and_a_variable_progress() {
    let mut case = EvalCase::new("evaluation-114");
    case.also_on_a_fresh_state();
    case.eval("math.ease_out_bounce(1.0, 5.0, 0.0)", 1.0);
    case.eval("math.ease_out_bounce(1.0, 5.0, 0.3)", 3.7225003);
    case.eval("math.ease_out_bounce(1.0, 5.0, 1.0)", 5.0);
    case.check(3);

    let mut case = EvalCase::new("evaluation-115");
    case.also_on_a_fresh_state();
    case.eval(
        "v.x = 0.0; return math.ease_out_bounce(1.0, 5.0, v.x);",
        1.0,
    );
    case.eval(
        "v.x = 0.3; return math.ease_out_bounce(1.0, 5.0, v.x);",
        3.7225003,
    );
    case.eval(
        "v.x = 1.0; return math.ease_out_bounce(1.0, 5.0, v.x);",
        5.0,
    );
    case.check(3);
}

#[test]
fn ease_in_out_bounce_with_a_literal_and_a_variable_progress() {
    let mut case = EvalCase::new("evaluation-116");
    case.also_on_a_fresh_state();
    case.eval("math.ease_in_out_bounce(1.0, 5.0, 0.0)", 1.0);
    case.eval("math.ease_in_out_bounce(1.0, 5.0, 0.3)", 1.1799998);
    case.eval("math.ease_in_out_bounce(1.0, 5.0, 1.0)", 5.0);
    case.check(3);

    let mut case = EvalCase::new("evaluation-117");
    case.also_on_a_fresh_state();
    case.eval(
        "v.x = 0.0; return math.ease_in_out_bounce(1.0, 5.0, v.x);",
        1.0,
    );
    case.eval(
        "v.x = 0.3; return math.ease_in_out_bounce(1.0, 5.0, v.x);",
        1.1799998,
    );
    case.eval(
        "v.x = 1.0; return math.ease_in_out_bounce(1.0, 5.0, v.x);",
        5.0,
    );
    case.check(3);
}

#[test]
fn ease_in_back_with_a_literal_and_a_variable_progress() {
    let mut case = EvalCase::new("evaluation-118");
    case.also_on_a_fresh_state();
    case.eval("math.ease_in_back(1.0, 5.0, 0.0)", 1.0);
    case.eval("math.ease_in_back(1.0, 5.0, 0.3)", 0.67920184);
    case.eval("math.ease_in_back(1.0, 5.0, 1.0)", 5.0);
    case.check(3);

    let mut case = EvalCase::new("evaluation-119");
    case.also_on_a_fresh_state();
    case.eval("v.x = 0.0; return math.ease_in_back(1.0, 5.0, v.x);", 1.0);
    case.eval(
        "v.x = 0.3; return math.ease_in_back(1.0, 5.0, v.x);",
        0.67920184,
    );
    case.eval("v.x = 1.0; return math.ease_in_back(1.0, 5.0, v.x);", 5.0);
    case.check(3);
}

#[test]
fn ease_out_back_with_a_literal_and_a_variable_progress() {
    let mut case = EvalCase::new("evaluation-120");
    case.also_on_a_fresh_state();
    case.eval("math.ease_out_back(1.0, 5.0, 0.0)", 1.0);
    case.eval("math.ease_out_back(1.0, 5.0, 0.3)", 4.628529);
    case.eval("math.ease_out_back(1.0, 5.0, 1.0)", 5.0);
    case.check(3);

    let mut case = EvalCase::new("evaluation-121");
    case.also_on_a_fresh_state();
    case.eval("v.x = 0.0; return math.ease_out_back(1.0, 5.0, v.x);", 1.0);
    case.eval(
        "v.x = 0.3; return math.ease_out_back(1.0, 5.0, v.x);",
        4.628529,
    );
    case.eval("v.x = 1.0; return math.ease_out_back(1.0, 5.0, v.x);", 5.0);
    case.check(3);
}

#[test]
fn ease_in_out_back_with_a_literal_and_a_variable_progress() {
    let mut case = EvalCase::new("evaluation-122");
    case.also_on_a_fresh_state();
    case.eval("math.ease_in_out_back(1.0, 5.0, 0.0)", 1.0);
    case.eval("math.ease_in_out_back(1.0, 5.0, 0.3)", 0.6846661);
    case.eval("math.ease_in_out_back(1.0, 5.0, 1.0)", 5.0);
    case.check(3);

    let mut case = EvalCase::new("evaluation-123");
    case.also_on_a_fresh_state();
    case.eval(
        "v.x = 0.0; return math.ease_in_out_back(1.0, 5.0, v.x);",
        1.0,
    );
    case.eval(
        "v.x = 0.3; return math.ease_in_out_back(1.0, 5.0, v.x);",
        0.6846661,
    );
    case.eval(
        "v.x = 1.0; return math.ease_in_out_back(1.0, 5.0, v.x);",
        5.0,
    );
    case.check(3);
}

#[test]
fn ease_in_elastic_with_a_literal_and_a_variable_progress() {
    let mut case = EvalCase::new("evaluation-124");
    case.also_on_a_fresh_state();
    case.eval("math.ease_in_elastic(1.0, 5.0, 0.0)", 1.0);
    case.eval("math.ease_in_elastic(1.0, 5.0, 0.3)", 0.9843759);
    case.eval("math.ease_in_elastic(1.0, 5.0, 1.0)", 5.0);
    case.check(3);

    let mut case = EvalCase::new("evaluation-125");
    case.also_on_a_fresh_state();
    case.eval(
        "v.x = 0.0; return math.ease_in_elastic(1.0, 5.0, v.x);",
        1.0,
    );
    case.eval(
        "v.x = 0.3; return math.ease_in_elastic(1.0, 5.0, v.x);",
        0.9843759,
    );
    case.eval(
        "v.x = 1.0; return math.ease_in_elastic(1.0, 5.0, v.x);",
        5.0,
    );
    case.check(3);
}

#[test]
fn ease_out_elastic_with_a_literal_and_a_variable_progress() {
    let mut case = EvalCase::new("evaluation-126");
    case.also_on_a_fresh_state();
    case.eval("math.ease_out_elastic(1.0, 5.0, 0.0)", 1.0);
    case.eval("math.ease_out_elastic(1.0, 5.0, 0.3)", 4.5);
    case.eval("math.ease_out_elastic(1.0, 5.0, 1.0)", 5.0);
    case.check(3);

    let mut case = EvalCase::new("evaluation-127");
    case.also_on_a_fresh_state();
    case.eval(
        "v.x = 0.0; return math.ease_out_elastic(1.0, 5.0, v.x);",
        1.0,
    );
    case.eval(
        "v.x = 0.3; return math.ease_out_elastic(1.0, 5.0, v.x);",
        4.5,
    );
    case.eval(
        "v.x = 1.0; return math.ease_out_elastic(1.0, 5.0, v.x);",
        5.0,
    );
    case.check(3);
}

#[test]
fn ease_in_out_elastic_with_a_literal_and_a_variable_progress() {
    let mut case = EvalCase::new("evaluation-128");
    case.also_on_a_fresh_state();
    case.eval("math.ease_in_out_elastic(1.0, 5.0, 0.0)", 1.0);
    case.eval("math.ease_in_out_elastic(1.0, 5.0, 0.3)", 0.93750346);
    case.eval("math.ease_in_out_elastic(1.0, 5.0, 1.0)", 5.0);
    case.check(3);

    let mut case = EvalCase::new("evaluation-129");
    case.also_on_a_fresh_state();
    case.eval(
        "v.x = 0.0; return math.ease_in_out_elastic(1.0, 5.0, v.x);",
        1.0,
    );
    case.eval(
        "v.x = 0.3; return math.ease_in_out_elastic(1.0, 5.0, v.x);",
        0.93750346,
    );
    case.eval(
        "v.x = 1.0; return math.ease_in_out_elastic(1.0, 5.0, v.x);",
        5.0,
    );
    case.check(3);
}

#[test]
fn nan_and_zero_through_division_and_math_functions_and_missing_arguments() {
    let mut group = RunGroup::new("nan_division_and_arity");
    group.row(0, "v.x = 5; return math.mod(v.x, 0);", f32::NAN);
    group.row(1, "v.x = 5; v.z = 0; return math.mod(v.x, v.z);", 0.0);
    group.row(2, "math.mod(5, 0)", f32::NAN);
    group.row(3, "v.x = -5.1; return math.mod(v.x, 3);", -2.1);
    group.row(4, "v.x = 1; return v.x / 0;", 0.0);
    group.row(5, "v.x = math.sqrt(-1); return v.x / 0;", f32::NAN);
    group.row(6, "v.x = 1; v.z = 0; return v.x / v.z;", 0.0);
    group.row(7, "v.x = math.sqrt(-1); v.z = 0; return v.x / v.z;", 0.0);
    group.row(8, "v.x = 1; v.z = math.sqrt(-1); return v.x / v.z;", 0.0);
    group.row(9, "v.x = 5; return v.x / -1;", -5.0);
    group.row(10, "v.x = 5; return v.x / -1;", -5.0).at(6);
    group
        .row(11, "v.x = 5; v.z = -1; return v.x / v.z;", 5.0)
        .at(6);
    group
        .row(12, "v.x = 5; v.z = -1; return v.x / v.z;", -5.0)
        .at(7);
    group
        .row(13, "v.x = 5; v.z = -1; return v.x / v.z;", 5.0)
        .at(-1);
    group.row(14, "v.x = 1/3; return v.x*3-1;", 2.9802322e-8);
    group.row(
        15,
        "v.x = 1; v.y = 3; v.q = v.x / v.y; return v.q*3-1;",
        2.9802322e-8,
    );
    group.row(16, "v.x = -1; return math.sign(v.x) + 1;", -2.0);
    group.row(17, "v.x = -1; return math.sign(v.x) * -2 + 1;", 1.0);
    group.row(18, "v.x = 0; return math.sign(v.x);", 1.0);
    group.row(19, "v.x = -0.0; return math.sign(-v.x);", 1.0);
    group.row(20, "v.x = math.sqrt(-1); return math.sign(v.x);", -1.0);
    group.row(21, "v.x = math.sqrt(-1); return math.acos(v.x);", 180.0);
    group.row(22, "v.x = math.sqrt(-1); return math.asin(v.x);", -90.0);
    group.row(23, "v.x = 1.0005; return math.acos(v.x);", 0.0);
    group.row(24, "v.x = 1.0006; return math.acos(v.x);", f32::NAN);
    group.row(25, "v.x = math.sqrt(-1); return v.x <= 1;", 1.0);
    group.row(26, "v.x = math.sqrt(-1); return v.x >= 1;", 0.0);
    group.row(27, "v.x = math.sqrt(-1); return v.x < 1;", 1.0);
    group.row(28, "v.x = math.sqrt(-1); return v.x == v.x;", 0.0);
    group.row(29, "v.x = math.sqrt(-1); return v.x != v.x;", 1.0);
    group.row(30, "v.x = math.sqrt(-1); return v.x ? 1 : 2;", 1.0);
    group.row(31, "v.x = math.sqrt(-1); return !v.x;", 0.0);
    group.row(32, "v.x = math.sqrt(-1); return math.max(v.x, 4);", 4.0);
    group.row(33, "v.x = math.sqrt(-1); return math.max(4, v.x);", 4.0);
    group.row(34, "v.x = math.sqrt(-1); return math.min(v.x, 4);", 4.0);
    group.row(
        35,
        "v.x = math.sqrt(-1); return math.clamp(4, v.x, 5);",
        4.0,
    );
    group.row(
        36,
        "v.x = math.sqrt(-1); return math.clamp(v.x, 1, 5);",
        1.0,
    );
    group.row(
        37,
        "v.x = math.sqrt(-1); return math.clamp(9, 1, v.x);",
        9.0,
    );
    group.row(38, "math.clamp(3, 2, 1)", 1.0);
    group.row(39, "math.clamp(-1, -2, -3)", -3.0);
    group.row(40, "v.x = math.sqrt(-1); return math.random(v.x, 5);", 5.0);
    group.row(41, "math.max(3)", 0.0).not_compiled().logs(&[
        "Unexpected number of parameters to Max 'math.max' function - expected 2, found 1.",
    ]);
    group.row(42, "math.min(3)", 0.0).not_compiled().logs(&[
        "Unexpected number of parameters to Min 'math.min' function - expected 2, found 1.",
    ]);
    group.row(43, "math.pow(2)", 0.0).not_compiled().logs(&[
        "Unexpected number of parameters to Power 'math.pow' function - expected 2, found 1.",
    ]);
    group.row(44, "math.mod(7)", 0.0).not_compiled().logs(&[
        "Unexpected number of parameters to Mod 'math.mod' function - expected 2, found 1.",
    ]);
    group
        .row(45, "v.a = 3; return math.max(v.a);", 0.0)
        .not_compiled()
        .logs(&[
            "Unexpected number of parameters to Max 'math.max' function - expected 2, found 1.",
        ]);
    group.x86_64_differs(&[8, 14, 15, 20, 21, 22, 25, 27, 35, 40]);
    group.check(46);
}

const ID: PostOp = PostOp::IDENTITY;
const NAN: f32 = f32::NAN;

/// Asserts that one of the first six runs answers `branch` conclusively.
fn measured(branch: &str) {
    let runs = [run_01(), run_02(), run_03(), run_04(), run_05(), run_06()];
    runs.iter().for_each(ServerRun::read_without_replay);
    assert!(
        runs.iter().any(|run| run.conclusive_branch(branch)),
        "no conclusive probe of runs 1–6 observed `{branch}`"
    );
}

/// The literal `1 / 3` as folded.
fn third() -> f32 {
    numeric::fold_const_div(1.0, 3.0)
}

#[test]
fn post_op_is_unfused_on_x86_64_and_fused_on_arm64() {
    // v.x = 1/3; v.x * 3 - 1
    measured("s09_postop_unfused");
    measured("u06_postop_unfused2");
    let post = PostOp::new(3.0, -1.0);
    if ARCH == Arch::X86_64 {
        assert_eq!(post.apply(third()).to_bits(), 0.0_f32.to_bits());
    }
    if ARCH == Arch::Arm64 {
        assert_eq!(post.apply(third()), 2.980_232_2e-8);
    }
}

#[test]
fn division_scales_after_dividing_on_x86_64_and_before_on_arm64() {
    // v.a / v.b * 3 with a = 1000, b = 13
    measured("r12_div_divide_then_scale");
    measured("u07_div_unfused_divide_first");
    let (a, b) = (1000.0_f32, 13.0_f32);
    let divide_first = (a / b) * 3.0;
    let scale_first = (a * 3.0) / b;
    assert_ne!(divide_first, scale_first);
    if ARCH == Arch::X86_64 {
        assert_eq!(numeric::div(a, b, PostOp::new(3.0, 0.0)), divide_first);
        assert_eq!(
            numeric::div(a, b, PostOp::new(3.0, 0.5)),
            divide_first + 0.5
        );
    }
    if ARCH == Arch::Arm64 {
        assert_eq!(numeric::div(a, b, PostOp::new(3.0, 0.0)), scale_first);
        assert_eq!(numeric::div(a, b, PostOp::new(3.0, 0.5)), scale_first + 0.5);
    }
    assert_eq!(numeric::div(a, b, ID), a / b);
}

#[test]
fn multiplication_scales_the_product_on_x86_64_and_the_operand_on_arm64() {
    // v.k * v.j * 7
    measured("s14_mul_chain_matches_stepwise");
    measured("u04_mul_kj_then_s");
    let (k, j) = (1.4_f32, 0.75_f32);
    let product_then_scale = (k * j) * 7.0;
    assert_ne!(product_then_scale, k * (j * 7.0));
    assert_ne!(product_then_scale, j * (k * 7.0));
    if ARCH == Arch::X86_64 {
        assert_eq!(
            numeric::mul(k, j, PostOp::new(7.0, 0.0)),
            product_then_scale
        );
        assert_eq!(
            numeric::mul(3.0, 0.1, PostOp::new(7.0, 0.0)),
            (3.0_f32 * 0.1) * 7.0
        );
    }
    // arm64: acc·(top·S) + O in one fused multiply-add, with top·S rounded first.
    if ARCH == Arch::Arm64 {
        assert_eq!(numeric::mul(k, j, PostOp::new(7.0, 0.0)), k * (j * 7.0));
    }
    assert_eq!(
        numeric::mul(k, j, PostOp::new(7.0, 0.25)),
        per_arch(product_then_scale + 0.25, k.mul_add(j * 7.0, 0.25))
    );
    assert_eq!(numeric::mul(k, j, ID), k * j);
}

#[test]
fn lerp_is_unfused_on_x86_64_and_fused_on_arm64() {
    // math.lerp(v.la, v.lb, v.t) with la = 1.1, lb = la * 3.3, t = 1/3
    measured("u05_lerp_unfused");
    let la = 1.1_f32;
    let lb = la * 3.3;
    let t = third();
    let stepwise = la + t * (lb - la);
    let fused = t.mul_add(lb - la, la);
    assert_ne!(stepwise, fused);
    assert_eq!(math::lerp(la, lb, t, ID), per_arch(stepwise, fused));
}

#[test]
fn nan_comparisons_are_ordered_on_x86_64() {
    measured("s01_nan_le_false");
    measured("s02_nan_ge_false");
    measured("s03_nan_ne_true");
    measured("s04_nan_lt_false");
    for (a, b) in [(NAN, 4.0), (4.0, NAN), (NAN, NAN)] {
        if ARCH == Arch::X86_64 {
            assert!(!numeric::lt(a, b));
            assert!(!numeric::le(a, b));
            assert!(!numeric::gt(a, b));
            assert!(!numeric::ge(a, b));
        }
        assert!(!numeric::eq(a, b));
        assert!(numeric::ne(a, b));
    }
}

#[test]
fn truthiness_is_a_float_compare_with_zero() {
    measured("s05_nan_truthy");
    measured("s06_notnan_false");
    measured("s07_nan_and_true");
    assert!(numeric::truthy(NAN));
    assert!(!numeric::truthy(0.0));
    assert!(!numeric::truthy(-0.0));
    assert!(numeric::truthy(-0.5));
    assert!(numeric::truthy(0.000_000_1));
    assert_eq!(numeric::not(NAN, ID), 0.0);
    assert_eq!(numeric::not(-0.0, ID), 1.0);
    assert_eq!(numeric::not(3.0, ID), 0.0);
    assert_eq!(numeric::not(0.0, PostOp::new(2.0, 1.0)), 3.0);
    assert_eq!(numeric::not(5.0, PostOp::new(2.0, 1.0)), 1.0);
}

#[test]
fn min_max_return_the_second_operand_on_x86_64() {
    measured("r01_maxnanfirst_is4");
    measured("r02_maxnansecond_isnan");
    measured("r03_minnanfirst_is4");
    measured("r04_minnansecond_isnan");
    measured("r05_maxconstnanfirst_is4");
    measured("u02_max_nan_nan_nan");
    if ARCH == Arch::X86_64 {
        assert_eq!(math::max(NAN, 4.0, ID), 4.0);
        assert!(math::max(4.0, NAN, ID).is_nan());
        assert!(math::max(NAN, NAN, ID).is_nan());
        assert_eq!(math::min(NAN, 4.0, ID), 4.0);
        assert!(math::min(4.0, NAN, ID).is_nan());
    }
}

#[test]
fn clamp_with_nan() {
    measured("r14_clampnan_is_lo");
    measured("u01_clamp_lonan_nan");
    measured("u03_clamp_hinan_is_v");
    // A NaN value gives the lower bound on both architectures.
    assert_eq!(math::clamp(NAN, 1.0, 2.0, ID), 1.0);
    // A NaN lower bound is returned on x86-64 and ignored on arm64.
    if ARCH == Arch::X86_64 {
        assert!(math::clamp(4.0, NAN, 5.0, ID).is_nan());
    }
    if ARCH == Arch::Arm64 {
        assert_eq!(math::clamp(4.0, NAN, 5.0, ID), 4.0);
    }
    // A NaN upper bound never matches.
    assert_eq!(math::clamp(4.0, 1.0, NAN, ID), 4.0);
}

#[test]
fn sign_of_nan_and_of_zero() {
    measured("r18_sign0_is1");
    measured("s08_signnan_1");
    assert_eq!(math::sign(NAN, ID), per_arch(1.0, -1.0));
    assert_eq!(math::sign(0.0, ID), 1.0);
    assert_eq!(math::sign(-0.0, ID), 1.0);
    assert_eq!(math::sign(-3.0, ID), -1.0);
    assert_eq!(math::sign(3.0, ID), 1.0);
}

#[test]
fn random_bounds_with_nan() {
    measured("s13_random_nanlo_nan");
    if ARCH == Arch::X86_64 {
        assert!(math::random(NAN, 4.0, 0.5, ID).is_nan());
    }
    // x86-64 bounds are lo = (b < a) ? b : a and hi = (a > b) ? a : b, so a NaN in either position
    // reaches hi·r + (1 − r)·lo.
    for sample in [0.0, 0.5, 1.0] {
        if ARCH == Arch::X86_64 {
            assert!(math::random(NAN, 4.0, sample, ID).is_nan());
            assert!(math::random(4.0, NAN, sample, ID).is_nan());
            assert!(math::random_integer(NAN, 4.0, sample, ID).is_nan());
            assert_eq!(math::random_integer(4.0, NAN, sample, ID), 4.0);
        }
    }
    // Two literal bounds are sorted when the call folds: (4, NaN) folds to 4.
    if ARCH == Arch::X86_64 {
        assert!(math::random_folded(0.5, math::random_const_bounds(NAN, 4.0, ID)).is_nan());
        assert_eq!(
            math::random_folded(0.5, math::random_const_bounds(4.0, NAN, ID)),
            4.0
        );
    }
    // arm64 drops the NaN bound: math.random(NaN, 5) = 5.
    if ARCH == Arch::Arm64 {
        assert_eq!(math::random(NAN, 5.0, 0.5, ID), 5.0);
        assert_eq!(math::random(5.0, NAN, 0.5, ID), 5.0);
    }
}

#[test]
fn division_guard_and_its_version_switch() {
    measured("r19_div_tiny_is0");
    measured("r20_div_neg_v13_isneg5");
    assert_eq!(f32::EPSILON, 1.192_092_9e-7);
    // 5 / v.g with v.g = 0.0000001: the guard fires.
    assert_eq!(numeric::div_guard(true, 0.000_000_1), None);
    assert_eq!(numeric::div_guard(false, 0.000_000_1), None);
    assert_eq!(numeric::div_guard(true, -0.000_000_1), None);
    assert_eq!(numeric::div_guard(true, 0.0), None);
    // The threshold is `f32::EPSILON` (2^-23) itself, exclusive.
    assert_eq!(numeric::div_guard(true, f32::EPSILON), Some(f32::EPSILON));
    assert_eq!(
        numeric::div_guard(true, f32::from_bits(f32::EPSILON.to_bits() - 1)),
        None
    );
    // v.h = -1; 5 / v.h: −5 from version 7, 5 up to version 6 (the guard pushes |d|).
    let signed = numeric::div_guard(true, -1.0).unwrap();
    assert_eq!(numeric::div(5.0, signed, ID), -5.0);
    let unsigned = numeric::div_guard(false, -1.0).unwrap();
    assert_eq!(numeric::div(5.0, unsigned, ID), 5.0);
    // v.a = -10, v.b = -2: −5 at version 6, 5 at version 7.
    assert_eq!(
        numeric::div(-10.0, numeric::div_guard(false, -2.0).unwrap(), ID),
        -5.0
    );
    assert_eq!(
        numeric::div(-10.0, numeric::div_guard(true, -2.0).unwrap(), ID),
        5.0
    );
    // A NaN divisor takes the guard on arm64 (the division is 0) and passes it on x86-64 (the
    // division is NaN).
    for signed in [true, false] {
        let guarded = numeric::div_guard(signed, NAN);
        assert_eq!(guarded.is_none(), per_arch(false, true));
        if let Some(pushed) = guarded {
            assert!(pushed.is_nan());
            assert!(numeric::div(1.0, pushed, ID).is_nan());
            assert!(numeric::div(1.0, pushed, PostOp::new(2.0, 1.0)).is_nan());
        }
    }
}

#[test]
fn literal_divisors_fold_to_a_reciprocal() {
    measured("r22_slash_before_star");
    // x / c → x · (1/c); |c| < `f32::EPSILON` (2^-23) → x · 0. The divisor stays signed in every
    // version.
    assert_eq!(numeric::fold_const_divisor(4.0), 0.25);
    assert_eq!(numeric::fold_const_divisor(-1.0), -1.0);
    assert_eq!(numeric::fold_const_divisor(0.0), 0.0);
    assert_eq!(numeric::fold_const_divisor(1.0e-8), 0.0);
    assert_eq!(numeric::fold_const_divisor(NAN), 0.0);
    // v.x = 1; v.x / 0 → 0, and with a NaN numerator → NaN (the numerator is still evaluated).
    assert_eq!(1.0 * numeric::fold_const_divisor(0.0), 0.0);
    assert!((NAN * numeric::fold_const_divisor(0.0)).is_nan());
    // All-constant divisions: 1 / 0 = 0, 0 / 0 = 0, -10 / -2 = 5.
    assert_eq!(numeric::fold_const_div(1.0, 0.0), 0.0);
    assert_eq!(numeric::fold_const_div(0.0, 0.0), 0.0);
    assert_eq!(numeric::fold_const_div(1.0, NAN), 0.0);
    assert_eq!(numeric::fold_const_div(-10.0, -2.0), 5.0);
    // 7 * 3 / 9: the division groups first, so the result is 7 · 0.33333334.
    let grouped = 7.0 * numeric::fold_const_div(3.0, 9.0);
    assert_eq!(grouped, 7.0 * 0.333_333_34_f32);
    assert_eq!(grouped, 2.333_333_5);
    // v.x = 1; v.x * 3 / 9 → the constant folds into the scale.
    assert_eq!(
        PostOp::new(numeric::fold_const_div(3.0, 9.0), 0.0).apply(1.0),
        0.333_333_34
    );
}

#[test]
fn mod_zero_rules() {
    measured("r06_modlitlit_isnan");
    measured("r07_modvarlit_isnan");
    measured("r08_modvarvar_is0");
    assert!(math::mod_const(1.0, 0.0, ID).is_nan());
    assert!(math::mod_const(5.0, 0.0, ID).is_nan());
    assert_eq!(math::mod_runtime(1.0, 0.0, ID), 0.0);
    assert_eq!(math::mod_runtime(1.0, -0.0, ID), 0.0);
    // The zero-divisor result is the node's offset: math.mod(v.x, v.y) + 0.125.
    assert_eq!(math::mod_runtime(1.0, 0.0, PostOp::new(2.0, 0.125)), 0.125);
    // The remainder takes the sign of the dividend.
    assert_eq!(math::mod_runtime(-5.1, 3.0, ID), -5.1_f32 % 3.0);
    assert!((math::mod_runtime(-5.1, 3.0, ID) - -2.1).abs() <= 1e-6);
    assert_eq!(math::mod_const(7.5, 2.0, PostOp::new(2.0, 1.0)), 4.0);
    assert!(math::mod_runtime(1.0, NAN, ID).is_nan());
    // A run-time `math.mod` remainder of −0 comes out as +0.
    assert_eq!(-3.0_f32 % 3.0, 0.0);
    assert!((-3.0_f32 % 3.0).is_sign_negative());
    assert_eq!(math::mod_const(-3.0, 3.0, ID).to_bits(), 0.0_f32.to_bits());
    assert_eq!(
        math::mod_runtime(-3.0, 3.0, ID).to_bits(),
        0.0_f32.to_bits()
    );
}

#[test]
fn shared_function_results() {
    measured("r09_minangle180var_isneg180");
    measured("r10_minangle180lit_isneg180");
    measured("r15_round_neg2p5_isneg3");
    measured("s10_asin_tol_clamped90");
    measured("s11_asin1001_nan");
    assert_eq!(math::min_angle(180.0, ID), -180.0);
    assert_eq!(math::round(-2.5, ID), -3.0);
    assert_eq!(math::asin(1.0004, ID), 90.0);
    assert!(math::asin(1.001, ID).is_nan());
}

/// The bare-call matcher accepts literal arguments, also parenthesised and negative, and variables
/// assigned from literals; nothing else.
#[test]
fn the_row_matcher_recognises_only_what_it_can_evaluate() {
    let call = parse_call("math.clamp(1.0f, (-2.0f), 3)").unwrap();
    assert_eq!(call.name, "clamp");
    assert_eq!(
        call.args.iter().map(|a| a.value).collect::<Vec<_>>(),
        [1.0, -2.0, 3.0]
    );
    assert!(call.args.iter().all(|a| a.literal));

    let call =
        parse_call("v.x = 5.5; Variable.y = -1e1; return Math.mod(v.x, variable.y);").unwrap();
    assert_eq!(call.name, "mod");
    assert_eq!(
        call.args
            .iter()
            .map(|a| (a.value, a.literal))
            .collect::<Vec<_>>(),
        [(5.5, false), (-10.0, false)]
    );

    for rejected in [
        "math.ceil(1.1f) + 1",
        "math.cos(math.pi)",
        "v.x = 1; math.abs(v.x)",
        "math.abs(v.unset)",
        "return math.abs(1)",
        "1 + 2",
    ] {
        assert!(parse_call(rejected).is_none(), "{rejected}");
    }
}
