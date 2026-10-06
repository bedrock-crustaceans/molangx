//! Comparisons, `!`, `&&`, `||`, `?:`, `??`, strings and mixed-kind `==`, operand read order and
//! missing reads.

#![cfg(all(feature = "compiler", feature = "stdlib"))]

mod common;

use common::HASH_OF_A;
use common::measured::*;

/// Which operand is read first. Every operand is an unset variable, so the unknown-variable message
/// names the first one read.
fn run_16() -> ServerRun {
    let mut run = ServerRun::new(
        "run_16",
        FIRST_RELEASE,
        "operand evaluation order of binary operators, comparisons, logic, ?: and math functions",
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
    run.probe("x30_add", "v.x30_add_l + v.x30_add_r;")
        .answers("x30_add_l");
    run.probe("x64_sub", "v.x64_sub_l - v.x64_sub_r;")
        .answers("x64_sub_l");
    run.probe("x31_mul", "v.x31_mul_l * v.x31_mul_r;")
        .answers("x31_mul_l");
    run.probe("x32_div", "v.x32_div_num / v.x32_div_den;")
        .answers("x32_div_den");
    run.probe("x33_lt", "v.x33_lt_l < v.x33_lt_r;")
        .answers("x33_lt_l");
    run.probe("x68_gt", "v.x68_gt_l > v.x68_gt_r;")
        .answers("x68_gt_l");
    run.probe("x69_le", "v.x69_le_l <= v.x69_le_r;")
        .answers("x69_le_l");
    run.probe("x70_ge", "v.x70_ge_l >= v.x70_ge_r;")
        .answers("x70_ge_l");
    run.probe("x34_eq", "v.x34_eq_l == v.x34_eq_r;")
        .answers("x34_eq_l");
    run.probe("x71_ne", "v.x71_ne_l != v.x71_ne_r;")
        .answers("x71_ne_l");
    run.probe("x65_and", "v.x65_and_l && v.x65_and_r;")
        .answers("x65_and_l");
    run.probe("x72_or", "v.x72_or_l || v.x72_or_r;")
        .answers("x72_or_l");
    run.probe("x66_cond", "v.x66_cond_c ? v.x66_cond_t : v.x66_cond_e;")
        .answers("x66_cond_c");
    run.probe("x35_max", "math.max(v.x35_max_a, v.x35_max_b);")
        .answers("x35_max_a");
    run.probe("x67_min", "math.min(v.x67_min_a, v.x67_min_b);")
        .answers("x67_min_a");
    run.probe("x61_pow", "math.pow(v.x61_pow_a, v.x61_pow_b);")
        .answers("x61_pow_a");
    run.probe("x62_mod", "math.mod(v.x62_mod_a, v.x62_mod_b);")
        .answers("x62_mod_a");
    run.probe("x63_atan2", "math.atan2(v.x63_atan2_y, v.x63_atan2_x);")
        .answers("x63_atan2_y");
    run.probe(
        "x75_copysign",
        "math.copy_sign(v.x75_copysign_a, v.x75_copysign_b);",
    )
    .answers("x75_copysign_a");
    run.probe(
        "x37_random",
        "math.random(v.x37_random_lo, v.x37_random_hi);",
    )
    .answers("x37_random_lo");
    run.probe(
        "x73_randint",
        "math.random_integer(v.x73_randint_lo, v.x73_randint_hi);",
    )
    .answers("x73_randint_lo");
    run.probe("x39", "v.x39_p1 + v.x39_p2 + v.x39_p3;")
        .answers("x39_p1");
    run.probe("x39a", "v.x39a_p1 = 1; v.x39a_p1 + v.x39a_p2 + v.x39a_p3;")
        .answers("x39a_p2");
    run.probe("x39b", "v.x39b_p2 = 1; v.x39b_p1 + v.x39b_p2 + v.x39b_p3;")
        .answers("x39b_p1");
    run.probe("x39c", "v.x39c_p3 = 1; v.x39c_p1 + v.x39c_p2 + v.x39c_p3;")
        .answers("x39c_p1");
    run.probe("x77", "v.x77_p1 * v.x77_p2 * v.x77_p3;")
        .answers("x77_p1");
    run.probe("x77a", "v.x77a_p1 = 1; v.x77a_p1 * v.x77a_p2 * v.x77a_p3;")
        .answers("x77a_p2");
    run.probe("x77b", "v.x77b_p2 = 1; v.x77b_p1 * v.x77b_p2 * v.x77b_p3;")
        .answers("x77b_p1");
    run.probe("x77c", "v.x77c_p3 = 1; v.x77c_p1 * v.x77c_p2 * v.x77c_p3;")
        .answers("x77c_p1");
    run.probe("x36", "math.clamp(v.x36_a, v.x36_b, v.x36_c);")
        .answers("x36_a");
    run.probe(
        "x36a",
        "v.x36a_a = 1; math.clamp(v.x36a_a, v.x36a_b, v.x36a_c);",
    )
    .answers("x36a_b");
    run.probe(
        "x36b",
        "v.x36b_b = 1; math.clamp(v.x36b_a, v.x36b_b, v.x36b_c);",
    )
    .answers("x36b_a");
    run.probe(
        "x36c",
        "v.x36c_c = 1; math.clamp(v.x36c_a, v.x36c_b, v.x36c_c);",
    )
    .answers("x36c_a");
    run.probe("x60", "math.lerp(v.x60_a, v.x60_b, v.x60_c);")
        .answers("x60_a");
    run.probe(
        "x60a",
        "v.x60a_a = 1; math.lerp(v.x60a_a, v.x60a_b, v.x60a_c);",
    )
    .answers("x60a_b");
    run.probe(
        "x60b",
        "v.x60b_b = 1; math.lerp(v.x60b_a, v.x60b_b, v.x60b_c);",
    )
    .answers("x60b_a");
    run.probe(
        "x60c",
        "v.x60c_c = 1; math.lerp(v.x60c_a, v.x60c_b, v.x60c_c);",
    )
    .answers("x60c_a");
    run.probe("x38", "math.die_roll(v.x38_n, v.x38_a, v.x38_b);")
        .answers("x38_n");
    run.probe(
        "x38a",
        "v.x38a_n = 1; math.die_roll(v.x38a_n, v.x38a_a, v.x38a_b);",
    )
    .answers("x38a_a");
    run.probe(
        "x38b",
        "v.x38b_a = 1; math.die_roll(v.x38b_n, v.x38b_a, v.x38b_b);",
    )
    .answers("x38b_n");
    run.probe(
        "x38c",
        "v.x38c_b = 1; math.die_roll(v.x38c_n, v.x38c_a, v.x38c_b);",
    )
    .answers("x38c_n");
    run.probe("x74", "math.die_roll_integer(v.x74_n, v.x74_a, v.x74_b);")
        .answers("x74_n");
    run.probe(
        "x74a",
        "v.x74a_n = 1; math.die_roll_integer(v.x74a_n, v.x74a_a, v.x74a_b);",
    )
    .answers("x74a_a");
    run.probe(
        "x74b",
        "v.x74b_a = 1; math.die_roll_integer(v.x74b_n, v.x74b_a, v.x74b_b);",
    )
    .answers("x74b_n");
    run.probe(
        "x74c",
        "v.x74c_b = 1; math.die_roll_integer(v.x74c_n, v.x74c_a, v.x74c_b);",
    )
    .answers("x74c_n");
    run
}

/// Operands and math-call arguments are read left to right, except that `/` reads its divisor
/// first.
#[test]
fn operands_are_read_left_to_right_except_the_divisor_run_16() {
    run_16().replay(48);
}

/// Mixed-kind `==` / `!=`, strings in arithmetic and as conditions, `&&` / `||` of −0, and
/// `math.max` / `math.min` of a constant and a NaN.
fn run_17() -> ServerRun {
    let mut run = ServerRun::new(
        "run_17",
        FIRST_RELEASE,
        "mixed-kind equality and strings in arithmetic and conditions, the && / || fold and -0, a constant argument of max/min with NaN",
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
        "x27",
        "v.e = ''; (v.e == 0) ? v.x27_empty_eq0 : v.x27_empty_ne0;",
    )
    .answers("x27_empty_eq0");
    run.probe(
        "x27n",
        "v.e27 = ''; (v.e27 != 0) ? v.x27n_empty_ne0 : v.x27n_empty_eq0;",
    )
    .answers("x27n_empty_eq0");
    run.probe(
        "x29",
        "v.z = 0; (v.z == '') ? v.x29_zero_eq_empty : v.x29_zero_ne_empty;",
    )
    .answers("x29_zero_eq_empty");
    run.probe(
        "x29n",
        "v.z29 = 0; (v.z29 != '') ? v.x29n_zero_ne_empty : v.x29n_zero_eq_empty;",
    )
    .answers("x29n_zero_eq_empty");
    run.probe(
        "x29v",
        "v.e29 = ''; v.z29v = 0; (v.e29 == v.z29v) ? v.x29v_empty_eq_zero : v.x29v_empty_ne_zero;",
    )
    .answers("x29v_empty_eq_zero");
    run.probe(
        "x29w",
        "(v.z29v == v.e29) ? v.x29w_zero_eq_empty : v.x29w_zero_ne_empty;",
    )
    .answers("x29w_zero_eq_empty");
    // `'a' * 1` is the low 32 bits of the hash of 'a' read as a float, about -2.44e-35.
    run.probe("x80", "v.one = 1; v.s28 = 'a'; v.f28 = v.s28 * v.one; v.big = math.pow(10, 35); v.g80 = v.f28 * v.big; v.f28 == 0 ? v.x80_strmul_zero : (v.f28 < 0 ? ((v.g80 < -2 && v.g80 > -3) ? v.x80_strmul_hashbits : v.x80_strmul_neg) : (v.f28 > 0 ? v.x80_strmul_pos : v.x80_strmul_nan));").answers("x80_strmul_hashbits");
    run.probe("x28", "v.one = 1; v.s = 'a'; v.f = v.s * v.one; (v.s == v.f) ? v.x28_hash_eq_float : v.x28_hash_ne_float;")
        .answers("x28_hash_eq_float");
    run.probe(
        "x28r",
        "(v.f == v.s) ? v.x28r_float_eq_hash : v.x28r_float_ne_hash;",
    )
    .answers("x28r_float_ne_hash");
    run.probe(
        "x28n",
        "(v.s != v.f) ? v.x28n_hash_ne_float : v.x28n_hash_eq_float;",
    )
    .answers("x28n_hash_eq_float");
    run.probe(
        "x81",
        "v.s81 = 'a'; v.g81 = 1; (v.s81 == v.g81) ? v.x81_str_eq_one : v.x81_str_ne_one;",
    )
    .answers("x81_str_ne_one");
    run.probe(
        "x81c",
        "v.s81c = 'a'; (v.s81c == 1) ? v.x81c_str_eq_one : v.x81c_str_ne_one;",
    )
    .answers("x81c_str_ne_one");
    run.probe(
        "x82",
        "v.sa = 'a'; v.sb = 'b'; v.sa2 = 'a'; (v.sa == v.sb) ? v.x82_ab_eq : ((v.sa == v.sa2) ? v.x82_ab_ne_aa_eq : v.x82_aa_ne);",
    )
    .answers("x82_ab_ne_aa_eq");
    run.probe(
        "x83",
        "v.s83 = 'a'; v.s83 ? v.x83_str_a_true : v.x83_str_a_false;",
    )
    .answers("x83_str_a_true");
    run.probe(
        "x84",
        "v.e84 = ''; v.e84 ? v.x84_str_empty_true : v.x84_str_empty_false;",
    )
    .answers("x84_str_empty_false");
    run.probe(
        "x85",
        "v.s85 = 'a'; v.zero = 0; v.g85 = v.s85 + v.zero; v.g85 == 0 ? v.x85_strplus_zero : (v.g85 < 0 ? v.x85_strplus_negtiny : (v.g85 > 0 ? v.x85_strplus_pos : v.x85_strplus_nan));",
    )
    .answers("x85_strplus_negtiny");
    run.probe(
        "x86",
        "v.s86 = 'a'; v.g86 = v.s86 + 1; v.g86 == 1 ? v.x86_strplus1_is1 : (v.g86 == 0 ? v.x86_strplus1_is0 : v.x86_strplus1_other);",
    )
    .answers("x86_strplus1_is1");
    run.probe("x87", "v.g87 = 'a' + 1; v.x87_literal_strplus_loaded;")
        .load_logs(&["'Add '+'' expression cannot take a 'String '''' argument. It only supports numerical arguments."])
        .silent();
    // `math.atan2(x, -1)` is negative for a −0 `x`.
    run.probe(
        "x18k",
        "v.h18 = -0.5; v.nz18 = math.ceil(v.h18); v.r = math.atan2(v.nz18, -1); v.r < 0 ? v.x18k_detector_negzero : v.x18k_detector_poszero;",
    )
    .answers("x18k_detector_negzero");
    run.probe("x18", "v.c = 1; v.r = math.atan2(v.c ? (1 && (-0)) : 5, -1); v.r < 0 ? v.x18_andfold_negzero : v.x18_andfold_poszero;")
        .answers("x18_andfold_poszero");
    run.probe("x18o", "v.c = 1; v.r = math.atan2(v.c ? (0 || (-0)) : 5, -1); v.r < 0 ? v.x18o_orfold_negzero : v.x18o_orfold_poszero;")
        .answers("x18o_orfold_poszero");
    run.probe("x18r", "v.one = 1; v.r = math.atan2(v.one && v.nz18, -1); v.r < 0 ? v.x18r_and_negzero : v.x18r_and_poszero;")
        .answers("x18r_and_poszero");
    run.probe("x18s", "v.zero = 0; v.r = math.atan2(v.zero || v.nz18, -1); v.r < 0 ? v.x18s_or_negzero : v.x18s_or_poszero;")
        .answers("x18s_or_poszero");
    run.probe(
        "x19",
        "v.n19 = math.sqrt(-1); v.p = math.max(4, v.n19); v.p == 4 ? v.x19_maxcf_is4 : (v.p == v.p ? v.x19_other : v.x19_maxcf_nan);",
    )
    .answers("x19_maxcf_is4");
    run.probe("x19b", "v.p = math.min(4, v.n19); v.p == 4 ? v.x19b_mincf_is4 : (v.p == v.p ? v.x19b_other : v.x19b_mincf_nan);")
        .answers("x19b_mincf_is4");
    run.probe("x19c", "v.p = math.max(v.n19, 4); v.p == 4 ? v.x19c_maxcf_is4 : (v.p == v.p ? v.x19c_other : v.x19c_maxcf_nan);")
        .answers("x19c_maxcf_is4");
    run.probe("x19d", "v.p = math.min(v.n19, 4); v.p == 4 ? v.x19d_mincf_is4 : (v.p == v.p ? v.x19d_other : v.x19d_mincf_nan);")
        .answers("x19d_mincf_is4");
    run
}

/// An empty string equals 0 either way round; a string variable equals the float made from it only
/// when the string is on the left; `&&` and `||` of −0 give +0.
#[test]
fn mixed_kind_equality_strings_and_logic_of_negative_zero_run_17() {
    run_17().replay(30);
}

/// The post-op of `?:` and `??`, and `??` inside a loop, as an operand and around a missing read.
fn run_18() -> ServerRun {
    let mut run = ServerRun::new(
        "run_18",
        FIRST_RELEASE,
        "post-op of ?: and ??, ?? and the state it leaves, a miss in a query argument, struct self-nesting, for_each over an unset variable",
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
    run.probe("x41", "v.c = 0; v.p = (v.c ? 5) * 3 + 1; v.p == 1 ? v.x41_noelse_post : (v.p == 0 ? v.x41_noelse_raw : v.x41_other);")
        .answers("x41_noelse_post");
    run.probe(
        "x41b",
        "v.c41 = 1; v.p = (v.c41 ? 5) * 3 + 1; v.p == 16 ? v.x41b_then_post : (v.p == 5 ? v.x41b_then_raw : v.x41b_other);",
    )
    .answers("x41b_then_post");
    run.probe(
        "x42",
        "v.c42 = 0; v.p = (v.c42 ? 5 : 3) * 2 + 1; v.p == 7 ? v.x42_else_post : (v.p == 3 ? v.x42_else_raw : v.x42_other);",
    )
    .answers("x42_else_post");
    run.probe(
        "x42b",
        "v.c42b = 1; v.p = (v.c42b ? 5 : 3) * 2 + 1; v.p == 11 ? v.x42b_then_post : (v.p == 5 ? v.x42b_then_raw : v.x42b_other);",
    )
    .answers("x42b_then_post");
    run.probe(
        "x16",
        "v.c = 0; v.p1 = (v.c ? 1 : 2) * 3; v.p1 == 6 ? v.x16_else_scaled : (v.p1 == 2 ? v.x16_else_unscaled : v.x16_other);",
    )
    .answers("x16_else_scaled");
    run.probe(
        "x17",
        "v.p1 = (v.x17_unset_left ?? 2) * 3; v.p1 == 6 ? v.x17_coalesce_scaled : (v.p1 == 2 ? v.x17_coalesce_unscaled : v.x17_other);",
    )
    .continues_after_miss()
    .answers("x17_coalesce_scaled");
    run.probe(
        "x43",
        "v.p43 = (v.x43_miss ?? 5) * 2 + 1; v.p43 == 11 ? v.x43_coalesce_post : (v.p43 == 5 ? v.x43_coalesce_raw : v.x43_other);",
    )
    .continues_after_miss()
    .answers("x43_coalesce_post");
    run.probe(
        "x43b",
        "v.set43 = 4; v.p43b = (v.set43 ?? 5) * 2 + 1; v.p43b == 9 ? v.x43b_left_post : (v.p43b == 4 ? v.x43b_left_raw : v.x43b_other);",
    )
    .answers("x43b_left_post");
    run.probe("x51", "v.own51 = 7; t.n51 = 0; v.q51 = 0; loop(3, { t.n51 = t.n51 + 1; v.q51 = (v.x51_missing_a ?? v.own51); }); t.n51 == 3 ? (v.q51 == 7 ? v.x51_loop3_q7 : v.x51_loop3_qbad) : (t.n51 == 1 ? v.x51_loop1 : v.x51_other);").continues_after_miss().answers("x51_loop3_q7");
    run.probe(
        "x52",
        "v.two52 = 2; v.s52 = v.two52 + (v.x52_missing_b ?? 3); v.s52 == 5 ? v.x52_pending_kept : (v.s52 == 3 ? v.x52_pending_lost : v.x52_other);",
    )
    .continues_after_miss()
    .answers("x52_pending_kept");
    run.probe(
        "x52b",
        "v.one52 = 1; v.s52b = math.min(v.one52, (v.x52b_missing ?? 3)); v.s52b == 1 ? v.x52b_pending_kept : (v.s52b == 3 ? v.x52b_pending_lost : v.x52b_other);",
    )
    .continues_after_miss()
    .answers("x52b_pending_kept");
    run.probe("x53", "t.n53 = 0; v.q53 = 0; t.d53 = 1; v.q53 = (loop(3, { t.n53 = t.n53 + 1; v.x53_inloop_miss; }) ?? 9); t.d53 = 2; t.n53 == 1 ? (v.q53 == 9 ? v.x53_exit_to_handler : v.x53_exit_qother) : (t.n53 == 3 ? (v.q53 == 9 ? v.x53_loop_continued_q9 : v.x53_loop_continued_qother) : v.x53_other);").continues_after_miss().load_logs(&["Error: found left-hand-side of ?? expression that isn't a direct-variable reference - this is unsupported at this time."]).answers("x53_exit_to_handler");
    run.probe("w53", "v.k53 = t.d53 ?? 0; v.m53 = t.n53 ?? 0; v.k53 == 2 ? (v.m53 == 1 ? v.w53_completed_n1 : (v.m53 == 3 ? v.w53_completed_n3 : v.w53_completed_nother)) : (v.k53 == 1 ? (v.m53 == 1 ? v.w53_ended_n1 : (v.m53 == 3 ? v.w53_ended_n3 : v.w53_ended_nother)) : v.w53_not_run);").answers("w53_completed_n1");
    run.probe(
        "x54",
        "t.n54 = 0; v.q54 = 0; t.d54 = 1; v.q54 = ({ t.n54 = t.n54 + 1; v.x54_inblock_miss; } ?? 9); t.d54 = 2; v.q54 == 9 ? v.x54_block_handler : (v.q54 == 0 ? v.x54_block_zero : v.x54_other);",
    )
    .continues_after_miss()
    .load_logs(&["Error: found left-hand-side of ?? expression that isn't a direct-variable reference - this is unsupported at this time."])
    .answers("x54_block_handler");
    run.probe("w54", "v.k54 = t.d54 ?? 0; v.k54 == 2 ? v.w54_completed : (v.k54 == 1 ? v.w54_ended_inside : v.w54_not_run);")
        .answers("w54_completed");
    run.probe(
        "x15",
        "v.r = q.log(v.x15_never_set_arg) ?? 7; v.r == 7 ? v.x15_outer_handler : (v.r == 0 ? v.x15_arg_zero : v.x15_other);",
    )
    .continues_after_miss()
    .not_answered();
    run.probe(
        "x15n",
        "t.d15n = 1; v.r15n = 3; v.r15n = q.log(v.x15n_never_set_arg); t.d15n = 2; v.r15n == 0 ? v.x15n_arg_zero : (v.r15n == 3 ? v.x15n_unassigned : v.x15n_other);",
    )
    .continues_after_miss()
    .not_run();
    run.probe("w15n", "v.k15n = t.d15n ?? 0; v.k15n == 2 ? v.w15n_completed : (v.k15n == 1 ? v.w15n_ended_inside : v.w15n_not_run);")
        .not_run();
    run.probe("x50", "v.s50.c = 1; loop(3, { v.s50.b = v.s50; }); v.d50 = (v.s50.b.b.b.c ?? 0); v.e50 = (v.s50.b.b.b.b.c ?? 0); v.d50 == 1 ? (v.e50 == 0 ? v.x50_depth3 : (v.e50 == 1 ? v.x50_deeper_or_cyclic : v.x50_d1_other)) : (v.d50 == 0 ? v.x50_not_nested : v.x50_other);").not_run();
    run.probe("x50v", "v.t50 = v.s50; v.t50.c = 5; v.s50.c == 1 ? v.x50v_byvalue : (v.s50.c == 5 ? v.x50v_shared : v.x50v_other);")
        .not_run();
    run.probe("x50d", "v.u50.c = 1; loop(20, { v.u50.b = v.u50; }); v.d50d = (v.u50.b.b.b.b.b.b.b.b.b.b.b.b.b.b.b.b.b.b.b.b.c ?? 0); v.d50d == 1 ? v.x50d_depth20_ok : (v.d50d == 0 ? v.x50d_depth20_missing : v.x50d_other);").not_run();
    run.probe(
        "x14",
        "v.c = 1; for_each(v.s, v.x14_never_set_array, {v.c = v.c + 1;}); v.c == 1 ? v.x14_foreach_unset_no_abort : v.x14_other;",
    )
    .not_run();
    run
}

/// A post-op applies to the result of `?:`, also to the 0 of an untaken branch without else, and to
/// that of `??`.
#[test]
fn post_ops_apply_to_conditional_and_coalescing_results_run_18() {
    run_18().replay(25);
}

/// `continue` inside an operand, and which operand decides a mixed-kind `==`.
fn run_26() -> ServerRun {
    let mut run = ServerRun::new(
        "run_26",
        FIRST_RELEASE,
        "follow-ups: continue inside an operand (run_22 x20, run_23), and which operand's kind decides a mixed-kind == (run_17)",
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
        "c01",
        "v.i01 = 0; loop(3, { v.i01 = v.i01 + 1; (v.i01 > 0) ? {continue;} : 0; }); v.i01 == 3 ? v.c01_plain_continue_3 : (v.i01 == 1 ? v.c01_plain_continue_1 : v.c01_other);",
    )
    .answers("c01_plain_continue_3");
    run.probe("c02", "v.k02 = 0; v.i02 = 0; loop(3, { v.i02 = v.i02 + 1; v.t02 = v.k02 * (v.i02 > 1 ? {continue;} : 0); }); v.i02 == 2 ? v.c02_exit_at_pending_continue : (v.i02 == 3 ? v.c02_full : (v.i02 == 1 ? v.c02_one : v.c02_other));").answers("c02_exit_at_pending_continue");
    run.probe("c03", "v.k03 = 0; v.i03 = 0; loop(3, { v.i03 = v.i03 + 1; v.t03 = math.max(v.k03, (v.i03 > 0 ? {continue;} : 0)); }); v.i03 == 1 ? v.c03_fnarg_pending_1 : (v.i03 == 3 ? v.c03_fnarg_pending_3 : v.c03_other);").answers("c03_fnarg_pending_1");
    run.probe("c04", "v.zero04 = 0; v.i04 = 0; v.j04 = 0; loop(2, { v.i04 = v.i04 + 1; loop(3, { v.j04 = v.j04 + 1; v.t04 = v.zero04 * (v.j04 > 0 ? {continue;} : 0); }); }); v.i04 == 2 ? (v.j04 == 2 ? v.c04_outer2_inner1each : (v.j04 == 6 ? v.c04_outer2_inner3each : v.c04_outer2_jother)) : (v.i04 == 1 ? v.c04_outer1 : v.c04_other);").not_answered();
    run.probe(
        "e01",
        "v.e01 = ''; (0 == v.e01) ? v.e01_zero_eq_empty : v.e01_zero_ne_empty;",
    )
    .not_run();
    run.probe(
        "e02",
        "v.one = 1; v.s02 = 'a'; v.f02 = v.s02 * v.one; ('a' == v.f02) ? v.e02_strconst_eq_float : v.e02_strconst_ne_float;",
    )
    .not_run();
    run.probe(
        "e03",
        "(v.f02 == 'a') ? v.e03_float_eq_strconst : v.e03_float_ne_strconst;",
    )
    .not_run();
    run.probe(
        "e04",
        "v.e04 = ''; v.h04 = -0.5; v.nz04 = math.ceil(v.h04); (v.e04 == v.nz04) ? v.e04_empty_eq_negzero : v.e04_empty_ne_negzero;",
    )
    .not_run();
    run.probe(
        "e05",
        "(v.nz04 == v.e04) ? v.e05_negzero_eq_empty : v.e05_negzero_ne_empty;",
    )
    .not_run();
    run.probe(
        "e06",
        "v.s06 = 'a'; v.one06 = 1; v.f06 = v.s06 * v.one06; v.g06 = v.f06 * v.one06; (v.s06 == v.g06) ? v.e06_hash_eq_float_copy : v.e06_hash_ne_float_copy;",
    )
    .not_run();
    run
}

/// A `continue` inside an operand or a math-call argument ends the loop after that pass; a
/// `continue` statement does not.
#[test]
fn continue_inside_an_operand_ends_the_loop_run_26() {
    run_26().replay(13);
}

/// Which operand decides a mixed-kind `==`.
fn run_28() -> ServerRun {
    let mut run = ServerRun::new(
        "run_28",
        FIRST_RELEASE,
        "the part of run_26 that the session never reached after c04 (mixed-kind == e01-e06), with the continue control c01 again",
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
        "c01",
        "v.i01 = 0; loop(3, { v.i01 = v.i01 + 1; (v.i01 > 0) ? {continue;} : 0; }); v.i01 == 3 ? v.c01_plain_continue_3 : (v.i01 == 1 ? v.c01_plain_continue_1 : v.c01_other);",
    )
    .answers("c01_plain_continue_3");
    run.probe(
        "e01",
        "v.e01 = ''; (0 == v.e01) ? v.e01_zero_eq_empty : v.e01_zero_ne_empty;",
    )
    .answers("e01_zero_eq_empty");
    run.probe(
        "e02",
        "v.one = 1; v.s02 = 'a'; v.f02 = v.s02 * v.one; ('a' == v.f02) ? v.e02_strconst_eq_float : v.e02_strconst_ne_float;",
    )
    .answers("e02_strconst_ne_float");
    run.probe(
        "e03",
        "(v.f02 == 'a') ? v.e03_float_eq_strconst : v.e03_float_ne_strconst;",
    )
    .answers("e03_float_ne_strconst");
    // −0 is 0x80000000 as bits and '' hashes to 0.
    run.probe(
        "e04",
        "v.e04 = ''; v.h04 = -0.5; v.nz04 = math.ceil(v.h04); (v.e04 == v.nz04) ? v.e04_empty_eq_negzero : v.e04_empty_ne_negzero;",
    )
    .answers("e04_empty_eq_negzero");
    run.probe(
        "e05",
        "(v.nz04 == v.e04) ? v.e05_negzero_eq_empty : v.e05_negzero_ne_empty;",
    )
    .answers("e05_negzero_ne_empty");
    run.probe(
        "e06",
        "v.s06 = 'a'; v.one06 = 1; v.f06 = v.s06 * v.one06; v.g06 = v.f06 * v.one06; (v.s06 == v.g06) ? v.e06_hash_eq_float_copy : v.e06_hash_ne_float_copy;",
    )
    .answers("e06_hash_eq_float_copy");
    run
}

/// A string variable on the left equals the float made from it, a string constant on either side
/// does not; `'' == -0` is true and `-0 == ''` false.
#[test]
fn the_operands_decide_a_mixed_kind_equality_run_28() {
    run_28().replay(10);
}

#[test]
fn a_hashed_string_compares_with_a_string_literal() {
    let mut case = EvalCase::new("evaluation-131");
    case.also_on_a_fresh_state();
    case.eval("query.get_name_test(0) != 'rabbit'", 1.0);
    case.eval("query.get_name_test(1) == 'rabbit'", 1.0);
    case.eval("query.get_name_test(0) == 'rabbit'", 0.0);
    case.eval("query.get_name_test(1) != 'rabbit'", 0.0);
    case.eval("'moo' == query.get_name_test(0)", 1.0);
    case.eval("query.get_name_test == 'moo'", 1.0);
    case.check(6);
}

#[test]
fn a_string_comparison_is_one_in_arithmetic() {
    let mut case = EvalCase::new("evaluation-132");
    case.also_on_a_fresh_state();
    case.eval("(query.get_name_test(0) == 'moo') + 1", 2.0);
    case.eval("(query.get_name_test(0) == 'moo') * 2", 2.0);
    case.eval("(query.get_name_test(0) == 'moo') * 2 + 1", 3.0);
    case.eval("(query.get_name_test(0) == 'moo') * -2 + 1", -1.0);
    case.eval("(query.get_name_test(0) == 'moo') * -2 - 1", -3.0);
    case.check(5);
}

#[test]
fn a_conditional_takes_the_then_branch_for_any_non_zero_condition() {
    let mut case = EvalCase::new("evaluation-147");
    case.also_on_a_fresh_state();
    case.eval("1 ? 2 : 3", 2.0);
    case.eval("0 ? 2 : 3", 3.0);
    case.eval("3 ? 1 : 2", 1.0);
    case.check(3);

    let mut case = EvalCase::new("evaluation-148");
    case.also_on_a_fresh_state();
    case.eval("(3 ? 1 : 2) + 1", 2.0);
    case.eval("(3 ? 1 : 2) * 2", 2.0);
    case.eval("(3 ? 1 : 2) * 2 + 1", 3.0);
    case.eval("(3 ? 1 : 2) * -2 + 1", -1.0);
    case.eval("(3 ? 1 : 2) * -2 - 1", -3.0);
    case.check(5);
}

#[test]
fn a_conditional_without_else_is_zero_when_not_taken() {
    let mut case = EvalCase::new("evaluation-150");
    case.also_on_a_fresh_state();
    case.eval("1 ? 2;", 0.0);
    case.eval("return 1 ? 2;", 2.0);
    case.eval("return 0 ? 2;", 0.0);
    case.check(3);
}

#[test]
fn coalescing_gives_the_right_side_for_an_unset_variable() {
    let mut case = EvalCase::new("evaluation-151");
    case.also_on_a_fresh_state();
    case.eval(
        "                  variable.b = 0.2; return (variable.a ?? 2) + (variable.b ?? 3);",
        2.2,
    );
    case.eval(
        "variable.a = 0.1;                   return (variable.a ?? 2) + (variable.b ?? 3);",
        3.1,
    );
    case.check(2);

    let mut case = EvalCase::new("evaluation-192");
    case.also_on_a_fresh_state();
    case.eval(
        "variable.a = 0.1; variable.b = 0.2; return (variable.a ?? 2) + (variable.b ?? 3);",
        0.3,
    );
    case.check(1);

    let mut case = EvalCase::new("evaluation-193");
    case.also_on_a_fresh_state();
    case.eval(
        "                                    return (variable.a ?? 2) + (variable.b ?? 3);",
        5.0,
    );
    case.check(1);
}

#[test]
fn a_conditional_block_runs_only_the_branch_taken() {
    let mut case = EvalCase::new("evaluation-152");
    case.also_on_a_fresh_state();
    case.eval("1 ? { variable.a = 0.1; }; return variable.a ?? 0.2;", 0.1);
    case.eval("0 ? { variable.a = 0.1; }; return variable.a ?? 0.2;", 0.2);
    case.eval(
        "1 ? { variable.a = 0.1; variable.b = 0.2; } : { variable.a = 1.0; variable.b = 2.0; }; return variable.a + variable.b;",
        0.3,
    );
    case.eval(
        "0 ? { variable.a = 0.1; variable.b = 0.2; } : { variable.a = 1.0; variable.b = 2.0; }; return variable.a + variable.b;",
        3.0,
    );
    case.eval(
        "1 ? { variable.a = 0.1; 1 ? { variable.a = 0.3; variable.b = 0.2; }; } : { variable.a = 1.0; variable.b = 2.0; }; return variable.a + variable.b;",
        0.5,
    );
    case.eval(
        "variable.a = -10.0f; variable.b = 123.0f; 1 ? { variable.a = 0.1; 0 ? { variable.a = 0.3; variable.b = 0.2; }; } : { variable.a = 1.0; variable.b = 2.0; }; return variable.a + variable.b;",
        123.1,
    );
    case.eval(
        "variable.a = -10.0f; variable.b = 123.0f; 0 ? { variable.a = 0.1; 1 ? { variable.a = 0.3; variable.b = 0.2; }; } : { variable.a = 1.0; variable.b = 2.0; }; return variable.a + variable.b;",
        3.0,
    );
    case.check(7);
}

#[test]
fn a_variable_set_from_a_suffixed_literal_is_a_true_condition() {
    let mut case = EvalCase::new("evaluation-159");
    case.also_on_a_fresh_state();
    case.eval("v.a = 1.0f; return v.a ? 2.0 : 3.0;", 2.0);
    case.eval("v.b = 1.1f; return v.b ? 2.1 : 3.1;", 2.1);
    case.check(2);
}

#[test]
fn version_2_accepts_non_numeric_operands_that_version_3_rejects() {
    let mut case = EvalCase::new("evaluation-181");
    case.hash("'a' + 3", "a", HASH_OF_A).at(2);
    case.parse_fails("'a' + 3").at(3);
    case.eval("'a' + 'b'", 0.0).at(2);
    case.parse_fails("'a' + 'b'").at(3);
    case.hash("3 + 'a'", "a", HASH_OF_A).at(2);
    case.parse_fails("3 + 'a'").at(3);
    case.eval(
        "v.count = 0; loop(3, {v.count = v.count + 1;}) + 1; return v.count;",
        3.0,
    )
    .at(2);
    case.parse_fails("v.count = 0; loop(3, {v.count = v.count + 1;}) + 1; return v.count;")
        .at(3);
    // `v.baa` is unset, so the expression ends with 0; with any non-array `v.baa`, `for_each` runs
    // no pass.
    case.eval(
        "v.count = 1; for_each(v.sheep, v.baa, {v.count = v.count + 1;}) + 1; return v.count;",
        1.0,
    )
    .at(2)
    .expected_failure(
        0.0,
        &["Error: unhandled request for unknown variable 'variable.baa'"],
        ("v.baa", 0.0),
    );
    case.parse_fails(
        "v.count = 1; for_each(v.sheep, v.baa, {v.count = v.count + 1;}) + 1; return v.count;",
    )
    .at(3);
    case.eval("v.count = 0; loop(3, {v.count = v.count + 1; (v.count == 2) ? break + 1; }); return v.count;", 2.0).at(2);
    case.parse_fails("v.count = 0; loop(3, {v.count = v.count + 1; (v.count == 2) ? break + 1; }); return v.count;").at(3);
    case.eval("v.count = 0; loop(3, {(v.count == 1) ? continue + 1; v.count = v.count + 1;}); return v.count;", 1.0).at(2);
    case.parse_fails("v.count = 0; loop(3, {(v.count == 1) ? continue + 1; v.count = v.count + 1;}); return v.count;").at(3);
    case.eval("(v.foo = 1) + 2; return v.foo;", 1.0).at(2);
    case.parse_fails("(v.foo = 1) + 2; return v.foo;").at(3);
    case.parses("geometry.foo + 1").at(2);
    case.parse_fails("geometry.foo + 1").at(3);
    case.parses("material.foo + 1").at(2);
    case.parse_fails("material.foo + 1").at(3);
    case.parses("texture.foo + 1").at(2);
    case.parse_fails("texture.foo + 1").at(3);
    case.parses("math.abs('a')").at(2);
    case.parse_fails("math.abs('a')").at(3);
    case.check(24);
}

#[test]
fn nan_comparisons_and_missing_reads_on_one_state() {
    let mut group = RunGroup::new("nan_comparisons_and_missing_reads");
    // A comparison with NaN is true for `<`, `<=` and `!=`, false for the others.
    group.row(0, "v.x = math.sqrt(-1); v.o = 1; return v.x < v.o;", 1.0);
    group.row(1, "v.x = math.sqrt(-1); v.o = 1; return v.x <= v.o;", 1.0);
    group.row(2, "v.x = math.sqrt(-1); v.o = 1; return v.x > v.o;", 0.0);
    group.row(3, "v.x = math.sqrt(-1); v.o = 1; return v.x >= v.o;", 0.0);
    group.row(4, "v.x = math.sqrt(-1); v.o = 1; return v.o < v.x;", 1.0);
    group.row(5, "v.x = math.sqrt(-1); v.o = 1; return v.o >= v.x;", 0.0);
    group.row(6, "v.x = math.sqrt(-1); return v.x > 1;", 0.0);
    group.row(7, "v.x = math.sqrt(-1); return v.x == 1;", 0.0);
    group.row(8, "v.x = math.sqrt(-1); return v.x != 1;", 1.0);
    // A missing read ends the expression with 0.
    group
        .row(10, "v.q = 1; v.r = v.missing; v.s = 2; return 3;", 0.0)
        .clears_variables()
        .logs(&["Error: unhandled request for unknown variable 'variable.missing'"]);
    group.row(11, "v.s ?? 99", 99.0);
    group.row(12, "v.q", 1.0);
    group.row(14, "t.persist = 42;", 0.0).clears_variables();
    group.row(15, "return t.persist;", 42.0);
    group.row(16, "t.persist", 42.0);
    group.row(17, "v.a = (v.miss ?? 5) + 1; return v.a;", 6.0);
    group.row(18, "v.miss ?? 1 ? 2 : 3", 2.0);
    group
        .row(19, "1 ?? 2", 0.0)
        .not_compiled()
        .logs(&["Error: found left-hand-side of ?? expression that isn't a direct-variable reference - this is unsupported at this time."]);
    group.row(20, "v.a = 1; return v.a.b ?? 7;", 7.0).logs(&[
        "Error: found left-hand-side of ?? expression that isn't a direct-variable reference - this is unsupported at this time.",
        "Error: unable to find member variable .b",
    ]);
    group
        .row(21, "t.a.b = 1; return 1;", 1.0)
        .logs(&["Error: left side of an assignment expression can only use temp variables if they are on their own and not part of a more complicated expression."]);
    group.row(22, "v.a.b = 2; return v.a.b;", 2.0);
    group
        .row(23, "continue; return 5;", 0.0)
        .not_compiled()
        .logs(&["Error: unreachable statements after Continue 'continue'."]);
    group
        .row(
            24,
            "v.c = 0; loop(3, {v.c = v.c + 1; continue; v.c = 100;}); return v.c;",
            3.0,
        )
        .logs(&["Error: unreachable statements after Continue 'continue'."]);
    group
        .row(25, "break; return 5;", 0.0)
        .not_compiled()
        .logs(&["Error: unreachable statements after Break 'break'."]);
    group.row(26, "1+1;", 0.0);
    group.row(27, "return 1+1;", 2.0);
    group.row(28, "v.x = 0; return v.x ?? 5;", 0.0);
    group.x86_64_differs(&[0, 1, 4]);
    group.check(27);
}
