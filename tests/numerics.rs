//! Numeric behaviour through the compiler and the VM: the math library, the two architectures,
//! the division guard per version, NaN and zero rules, and the random operators.

#![cfg(all(feature = "vm", feature = "stdlib"))]

mod common;

mod math_functions {
    // Expected values are written with nine significant digits.
    #![allow(clippy::excessive_precision)]

    use crate::common::{Samples, assert_bits, compile_support::server_at, per_arch};
    use molangx::catalog::{QueryAdmission, QuerySetMask};
    use molangx::compile::{CompileFailure, CompileOptions, compile};
    use molangx::numeric::{ARCH, Arch, PostOp};
    use molangx::ops::OpSet;
    use molangx::stdlib::{MathFn, math};
    use molangx::vm::{NoHostEnv, Value, VariableName};

    const NAN: f32 = f32::NAN;

    /// Compiles (which must succeed) and evaluates with `v.<name>` preset and a random source that
    /// plays back `samples` (0 once they run out). Returns the value and the number of draws.
    fn run(source: &str, vars: &[(&str, f32)], samples: &[f32]) -> (f32, usize) {
        let compiled = compile(source, &server_at(13));
        assert_eq!(
            compiled.failure(),
            None,
            "{source:?}: {:?}",
            compiled.diagnostics()
        );
        let expr = compiled.expr().cloned().expect("an expression");
        let mut env = NoHostEnv::new();
        for (name, value) in vars {
            env.variables
                .set(VariableName::new(name), Value::Float(*value));
        }
        let mut rng = Samples::new(samples);
        let value = {
            let mut cx = env.cx();
            cx.rng = &mut rng;
            expr.eval_f32(&mut cx)
        };
        assert!(env.sink.is_empty(), "{source:?}: {:?}", env.sink);
        (value, rng.draws)
    }

    fn eval(source: &str) -> f32 {
        run(source, &[], &[]).0
    }

    /// The constant `source` folds to.
    #[track_caller]
    fn folded_constant(source: &str) -> f32 {
        let compiled = compile(source, &server_at(13));
        let constant = compiled.expr().and_then(|expr| expr.as_constant());
        constant.unwrap_or_else(|| panic!("{source:?} does not fold"))
    }

    fn eval_vars(source: &str, vars: &[(&str, f32)]) -> f32 {
        run(source, vars, &[]).0
    }

    /// The `f64` function of an `f32` argument, rounded to `f32`.
    fn rounded(f: fn(f64) -> f64, x: f32) -> f32 {
        f(f64::from(x)) as f32
    }

    /// A float that is not exactly representable in a short literal: `1/3` in f32.
    fn third() -> f32 {
        1.0 / 3.0
    }

    /// Post-ops folded at compile time choose the NaN the same operations choose at run time: `∞ −
    /// ∞`, `∞·0`, two different NaNs, and the bounds of a literal `math.random`.
    #[test]
    fn folded_post_ops_choose_the_nans_of_their_run_time_forms() {
        let (sqrt, ln) = (eval("math.sqrt(-1)"), eval("math.ln(-1)"));
        assert_ne!(sqrt.to_bits(), ln.to_bits());
        let vars = [
            ("x", 2.0),
            ("p", f32::INFINITY),
            ("q", f32::NEG_INFINITY),
            ("z", 0.0),
            ("s", sqrt),
            ("a", ln),
        ];
        for (folded, run_time) in [
            ("(v.x + 1e39) + (v.x - 1e39)", "(v.x + v.p) + (v.x + v.q)"),
            ("(v.x - 1e39) + (v.x + 1e39)", "(v.x + v.q) + (v.x + v.p)"),
            ("(v.x * 1e39) * 0", "(v.x * v.p) * v.z"),
            ("1e39 / 1e39", "v.p / v.p"),
            (
                "(v.x + math.sqrt(-1)) + (v.x + math.ln(-1))",
                "(v.x + v.s) + (v.x + v.a)",
            ),
            (
                "(v.x + math.ln(-1)) + (v.x + math.sqrt(-1))",
                "(v.x + v.a) + (v.x + v.s)",
            ),
            ("v.x + 1e39 + v.x - 1e39", "v.x + v.p + v.x + v.q"),
            ("math.random(1e39, 1e39)", "math.random(v.p, v.p)"),
        ] {
            let (constant, run) = (eval_vars(folded, &vars), eval_vars(run_time, &vars));
            assert!(constant.is_nan(), "{folded} = {constant}");
            assert_eq!(
                constant.to_bits(),
                run.to_bits(),
                "{folded} against {run_time}"
            );
        }
    }

    /// Math functions are operations, not queries: they compile with no query set and at version
    /// −1.
    #[test]
    fn math_functions_ignore_query_versions_and_sets() {
        for function in MathFn::all() {
            let meta = function.meta();
            let arguments = vec!["1"; usize::from(meta.min_args)].join(", ");
            let source = if *function == MathFn::Pi {
                function.token().to_owned()
            } else {
                format!("{}({arguments})", function.token())
            };
            for version in [-1_i16, 1, 13] {
                let options = CompileOptions {
                    admission: QueryAdmission::Sets(QuerySetMask::empty()),
                    ..server_at(version)
                };
                let compiled = compile(&source, &options);
                assert_eq!(
                    compiled.failure(),
                    None,
                    "{source} at v{version}: {:?}",
                    compiled.diagnostics()
                );
            }
        }
        // A query, by contrast, does not resolve without a set.
        let options = CompileOptions {
            admission: QueryAdmission::Sets(QuerySetMask::empty()),
            ..server_at(13)
        };
        assert_eq!(
            compile("query.is_baby", &options).failure(),
            Some(CompileFailure::Rejected)
        );
    }

    /// The post-op `raw·S + O` applies to every value-producing function, folded or run-time.
    #[test]
    fn every_math_result_takes_the_post_op() {
        let cases: &[(&str, f32)] = &[
            ("math.abs(v.a)", 1.5),
            ("math.ceil(v.a)", 2.0),
            ("math.floor(v.a)", 1.0),
            ("math.round(v.a)", 2.0),
            ("math.trunc(v.a)", 1.0),
            ("math.sqrt(v.b)", 2.0),
            ("math.max(v.a, v.b)", 4.0),
            ("math.min(v.a, v.b)", 1.5),
            ("math.clamp(v.b, 0, v.a)", 1.5),
            ("math.lerp(0, v.b, 0.5)", 2.0),
            ("math.mod(v.b, v.a)", 1.0),
            ("math.pow(v.b, 0.5)", 2.0),
            ("math.copy_sign(v.a, -1)", -1.5),
            ("math.hermite_blend(0.5)", 0.5),
            ("math.inverse_lerp(0, v.b, 1)", 0.25),
            ("math.min_angle(v.a * 100 + 220)", 10.0),
            ("math.ease_in_quad(0, v.b, 0.5)", 1.0),
            ("math.pi", math::PI),
        ];
        for (call, raw) in cases {
            let vars = [("a", 1.5), ("b", 4.0)];
            assert_eq!(eval_vars(call, &vars), *raw, "{call}");
            for (s, o) in [(2.0_f32, 1.0_f32), (-3.0, 0.5), (1.0, -1.0)] {
                let source = format!("({call}) * {s} + {o}");
                assert_eq!(eval_vars(&source, &vars), raw * s + o, "{source}");
            }
        }
    }

    #[test]
    fn abs_clears_the_sign_bit() {
        assert_eq!(eval("math.abs(-2)"), 2.0);
        assert_bits(eval_vars("math.abs(v.x)", &[("x", -0.0)]), 0.0, "abs(-0)");
        assert_eq!(
            eval_vars("math.abs(v.x)", &[("x", f32::NEG_INFINITY)]),
            f32::INFINITY
        );
        assert!(eval_vars("math.abs(v.x)", &[("x", NAN)]).is_nan());
    }

    /// Arguments up to ±1.0005 clamp to ±1; 1.0006 is NaN; results are in degrees.
    #[test]
    fn inverse_trig_clamp_window() {
        assert_eq!(eval("math.acos(1.0005)"), 0.0);
        assert_eq!(eval_vars("math.acos(v.x)", &[("x", 1.0005)]), 0.0);
        assert_eq!(eval_vars("math.acos(v.x)", &[("x", -1.0001)]), 180.0);
        assert_eq!(eval_vars("math.acos(v.x)", &[("x", -1.0005)]), 180.0);
        assert_eq!(eval_vars("math.asin(v.x)", &[("x", 1.0005)]), 90.0);
        assert_eq!(eval_vars("math.asin(v.x)", &[("x", -1.0004)]), -90.0);
        for x in [1.0006_f32, -1.0006, 2.0] {
            assert!(
                eval_vars("math.acos(v.x)", &[("x", x)]).is_nan(),
                "acos({x})"
            );
            assert!(
                eval_vars("math.asin(v.x)", &[("x", x)]).is_nan(),
                "asin({x})"
            );
        }
        assert!(eval("math.acos(1.0006)").is_nan());
        let x = 0.3_f32;
        assert_bits(
            eval_vars("math.acos(v.x)", &[("x", x)]),
            rounded(libm::acos, x) * math::RAD_TO_DEG,
            "acos(0.3)",
        );
        assert_bits(
            eval_vars("math.asin(v.x)", &[("x", x)]),
            rounded(libm::asin, x) * math::RAD_TO_DEG,
            "asin(0.3)",
        );
    }

    /// On arm64 a NaN argument is clamped to −1; on x86-64 it passes through.
    #[test]
    fn inverse_trig_of_nan() {
        let nan = [("x", NAN)];
        if ARCH == Arch::Arm64 {
            assert_eq!(eval_vars("math.acos(v.x)", &nan), 180.0);
            assert_eq!(eval_vars("math.asin(v.x)", &nan), -90.0);
        }
        if ARCH == Arch::X86_64 {
            assert!(eval_vars("math.acos(v.x)", &nan).is_nan());
            assert!(eval_vars("math.asin(v.x)", &nan).is_nan());
        }
    }

    /// `atan` and `atan2(y, x)` in degrees, y first; `atan2(0, 0)` = 0.
    #[test]
    fn atan_and_atan2() {
        let x = 0.3_f32;
        assert_bits(
            eval_vars("math.atan(v.x)", &[("x", x)]),
            rounded(libm::atan, x) * math::RAD_TO_DEG,
            "atan(0.3)",
        );
        assert_eq!(eval("math.atan(1)"), 45.0);
        assert_eq!(eval("math.atan2(1, 0)"), 90.0);
        assert_eq!(eval("math.atan2(0, 1)"), 0.0);
        assert_eq!(eval("math.atan2(0, -1)"), 180.0);
        assert_eq!(
            eval_vars("math.atan2(v.y, v.x)", &[("y", 0.0), ("x", 0.0)]),
            0.0
        );
        assert_eq!(eval("math.atan2(0, 0)"), 0.0);
        let (y, x) = (-1.0_f32, -7.0_f32);
        assert_bits(
            eval_vars("math.atan2(v.y, v.x)", &[("y", y), ("x", x)]),
            libm::atan2(f64::from(y), f64::from(x)) as f32 * math::RAD_TO_DEG,
            "atan2(-1, -7)",
        );
    }

    /// Ceil toward +∞ (keeping −0), floor toward −∞, round half away from zero, trunc toward zero.
    #[test]
    fn rounding() {
        for (source, x, expected) in [
            ("math.ceil(v.x)", -0.5_f32, -0.0_f32),
            ("math.ceil(v.x)", 1.1, 2.0),
            ("math.ceil(v.x)", -1.1, -1.0),
            ("math.floor(v.x)", -0.5, -1.0),
            ("math.floor(v.x)", 1.9, 1.0),
            ("math.floor(v.x)", -1.1, -2.0),
            ("math.round(v.x)", 0.5, 1.0),
            ("math.round(v.x)", -0.5, -1.0),
            ("math.round(v.x)", -1.5, -2.0),
            ("math.round(v.x)", -2.5, -3.0),
            ("math.round(v.x)", 2.5, 3.0),
            ("math.round(v.x)", 2.49, 2.0),
            ("math.trunc(v.x)", 1.7, 1.0),
            ("math.trunc(v.x)", -1.7, -1.0),
            ("math.trunc(v.x)", -0.5, -0.0),
            ("math.trunc(v.x)", 1.0e10, 1.0e10),
        ] {
            assert_bits(
                eval_vars(source, &[("x", x)]),
                expected,
                &format!("{source} with {x}"),
            );
        }
        assert_bits(eval("math.ceil(-0.5)"), -0.0, "folded ceil(-0.5)");
        assert_eq!(eval("math.round(-2.5)"), -3.0);
        assert_eq!(eval("math.trunc(-2.9)"), -2.0);
    }

    /// Clamp tests the upper bound first; a NaN value gives the
    /// lower bound on both platforms; a NaN lower bound is returned on x86-64 and ignored on arm64.
    #[test]
    fn clamp_order_and_nan() {
        assert_eq!(eval("math.clamp(3, 2, 1)"), 1.0);
        assert_eq!(eval("math.clamp(-1, -2, -3)"), -3.0);
        let vars = [("v", 3.0), ("lo", 2.0), ("hi", 1.0)];
        assert_eq!(eval_vars("math.clamp(v.v, v.lo, v.hi)", &vars), 1.0);
        assert_eq!(
            eval_vars(
                "math.clamp(v.v, v.lo, v.hi)",
                &[("v", 1.5), ("lo", 2.0), ("hi", 1.0)]
            ),
            1.0
        );
        assert_eq!(
            eval_vars(
                "math.clamp(v.v, v.lo, v.hi)",
                &[("v", 0.5), ("lo", 2.0), ("hi", 1.0)]
            ),
            2.0
        );
        assert_eq!(eval_vars("math.clamp(v.v, 1, 2)", &[("v", NAN)]), 1.0);
        assert_eq!(eval_vars("math.clamp(v.v, 1, 5)", &[("v", NAN)]), 1.0);
        // A NaN upper bound never matches `v > hi`.
        assert_eq!(eval_vars("math.clamp(9, 1, v.hi)", &[("hi", NAN)]), 9.0);
        if ARCH == Arch::X86_64 {
            assert!(eval_vars("math.clamp(4, v.lo, 5)", &[("lo", NAN)]).is_nan());
            assert!(eval_vars("math.clamp(v.v, v.lo, 5)", &[("v", 4.0), ("lo", NAN)]).is_nan());
        }
        if ARCH == Arch::Arm64 {
            assert_eq!(eval_vars("math.clamp(4, v.lo, 5)", &[("lo", NAN)]), 4.0);
            assert_eq!(
                eval_vars("math.clamp(v.v, v.lo, 5)", &[("v", 4.0), ("lo", NAN)]),
                4.0
            );
        }
    }

    /// `copy_sign(a, b)`: the magnitude of `a`, the sign bit of `b` (−0 included).
    #[test]
    fn copy_sign_is_a_bit_select() {
        assert_eq!(eval("math.copy_sign(-1.1, 3.1)"), 1.1);
        assert_eq!(eval("math.copy_sign(2, -0.5)"), -2.0);
        assert_eq!(eval_vars("math.copy_sign(2, v.b)", &[("b", -0.0)]), -2.0);
        assert_eq!(eval_vars("math.copy_sign(2, v.b)", &[("b", 0.0)]), 2.0);
        assert_bits(
            eval_vars("math.copy_sign(v.a, -1)", &[("a", 0.0)]),
            -0.0,
            "copy_sign(0, -1)",
        );
        let nan = eval_vars("math.copy_sign(v.a, -1)", &[("a", NAN)]);
        assert!(nan.is_nan() && nan.is_sign_negative());
    }

    /// Not quantised like the easings: the bits equal the sine / cosine of `x·0.017453292`, in
    /// `f64` rounded to `f32`.
    #[test]
    fn sin_and_cos_are_plain_in_degrees() {
        let deg = f32::from_bits(0x3c8e_fa35);
        assert_eq!(deg, 0.017_453_292);
        for x in [
            0.0_f32,
            1.0,
            30.0,
            45.0,
            90.0 / 1.3,
            138.461_54,
            180.0,
            -77.7,
            1000.0,
            12_345.6,
        ] {
            let radians = x * deg;
            assert_bits(
                eval_vars("math.sin(v.x)", &[("x", x)]),
                rounded(libm::sin, radians),
                &format!("sin({x})"),
            );
            assert_bits(
                eval_vars("math.cos(v.x)", &[("x", x)]),
                rounded(libm::cos, radians),
                &format!("cos({x})"),
            );
        }
        assert_eq!(eval("math.sin(180)"), -8.742_278e-8);
        assert_eq!(eval_vars("math.sin(v.x)", &[("x", 180.0)]), -8.742_278e-8);
        // The plain sine is not the quantised one: sin(90/1.3) differs from the table sine.
        let x = 90.0_f32 / 1.3;
        let table_index = ((x * deg * f32::from_bits(0x4622_f983)) as i32 & 0xffff) as f32;
        assert_ne!(
            eval_vars("math.sin(v.x)", &[("x", x)]),
            rounded(libm::sin, table_index / f32::from_bits(0x4622_f983))
        );
    }

    /// `die_roll(n, a, b)`: `trunc(n)` rolls (none below 1), bounds floored and sorted, one draw
    /// per roll; the sum need not be integral. The per-roll arithmetic is each architecture's:
    /// `r·(hi − lo) + (sum + lo)` rounded once under `Arm64`, `sum + (r·hi + (1 − r)·lo)` under
    /// `X86_64`.
    #[test]
    fn die_roll() {
        assert_eq!(
            run("math.die_roll(3, 1, 6)", &[], &[0.0, 0.0, 0.0]),
            (3.0, 3)
        );
        assert_eq!(
            run("math.die_roll(3, 1, 6)", &[], &[1.0, 1.0, 1.0]),
            (18.0, 3)
        );
        assert_eq!(
            run("math.die_roll(3, 6, 1)", &[], &[0.5, 0.5, 0.5]),
            (10.5, 3),
            "inverted bounds, not integral"
        );
        // trunc(2.9) = 2 rolls; floor(1.9) = 1, floor(6.9) = 6.
        assert_eq!(
            run(
                "math.die_roll(v.n, v.a, v.b)",
                &[("n", 2.9), ("a", 1.9), ("b", 6.9)],
                &[1.0, 1.0]
            ),
            (12.0, 2)
        );
        for n in [0.9_f32, 0.0, -2.0] {
            assert_eq!(
                run("math.die_roll(v.n, 1, 6)", &[("n", n)], &[0.5]),
                (0.0, 0),
                "n = {n}"
            );
        }
        assert_eq!(
            run("math.die_roll(v.n, 1, 6) * 2 + 1.5", &[("n", 0.0)], &[]),
            (1.5, 0)
        );
        let r = third();
        let (lo, hi) = (1.0_f32, 7.0_f32);
        let vars = [("a", 1.1), ("b", 7.3)];
        let x86 = (0.0 + (r * hi + (1.0 - r) * lo)) + (r * hi + (1.0 - r) * lo);
        let first = r.mul_add(hi - lo, 0.0 + lo);
        let arm = r.mul_add(hi - lo, first + lo);
        assert_bits(
            run("math.die_roll(2, v.a, v.b)", &vars, &[r, r]).0,
            per_arch(x86, arm),
            "x86-64 die_roll",
        );
    }

    /// `die_roll_integer`: each roll is an integer in [lo, hi] (floored, clamped), summed; x86-64
    /// interpolates as `floor((1 − r)·lo + r·top)`, arm64 as `floor(r·span + lo)` with one
    /// rounding.
    #[test]
    fn die_roll_integer() {
        assert_eq!(
            run("math.die_roll_integer(3, 1, 6)", &[], &[0.0, 0.0, 0.0]),
            (3.0, 3)
        );
        assert_eq!(
            run("math.die_roll_integer(3, 1, 6)", &[], &[1.0, 1.0, 1.0]),
            (18.0, 3)
        );
        assert_eq!(
            run("math.die_roll_integer(3, 6, 1)", &[], &[0.5, 0.5, 0.5]),
            (9.0, 3)
        );
        assert_eq!(
            run("math.die_roll_integer(2, 1.9, 6.9)", &[], &[0.0, 1.0]),
            (7.0, 2)
        );
        assert_eq!(
            run("math.die_roll_integer(0.5, 1, 6)", &[], &[0.5]),
            (0.0, 0)
        );
        for sample in [0.0_f32, 0.25, 0.5, 0.999_999_9, 1.0] {
            let (value, _) = run("math.die_roll_integer(1, 2, 4)", &[], &[sample]);
            assert!(
                (2.0..=4.0).contains(&value) && value.fract() == 0.0,
                "{sample} → {value}"
            );
        }
        // The two architectures disagree at (−3, 2) with the sample 0.5.
        let vars = [("a", -3.0), ("b", 2.0)];
        assert_eq!(
            run("math.die_roll_integer(1, v.a, v.b)", &vars, &[0.5]).0,
            per_arch(-1.0, 0.0)
        );
        let (lo, hi, r) = (-3.0_f32, 2.0_f32, 0.5_f32);
        let top = (-f32::EPSILON * hi + 1.0) + hi;
        assert_eq!(((1.0 - r) * lo + top * r).floor(), -1.0);
        let span = hi.mul_add(-f32::EPSILON, (hi + 1.0) - lo);
        assert_eq!(r.mul_add(span, lo).floor(), 0.0);
    }

    #[test]
    fn exp_ln_pow_sqrt() {
        for x in [0.0_f32, 1.0, -1.0, 0.3, 10.0, 88.0] {
            assert_bits(
                eval_vars("math.exp(v.x)", &[("x", x)]),
                rounded(libm::exp, x),
                &format!("exp({x})"),
            );
        }
        for x in [1.0_f32, 2.0, 0.5, 1.0e-3, 1.0e30] {
            assert_bits(
                eval_vars("math.ln(v.x)", &[("x", x)]),
                rounded(libm::log, x),
                &format!("ln({x})"),
            );
        }
        assert_eq!(eval("math.ln(0)"), f32::NEG_INFINITY);
        assert!(eval("math.ln(-1)").is_nan());
        for (a, b) in [
            (2.0_f32, 10.0_f32),
            (2.0, 0.5),
            (-2.0, 3.0),
            (0.0, 0.0),
            (10.0, -2.0),
        ] {
            assert_bits(
                eval_vars("math.pow(v.a, v.b)", &[("a", a), ("b", b)]),
                libm::pow(f64::from(a), f64::from(b)) as f32,
                &format!("pow({a}, {b})"),
            );
        }
        assert_eq!(eval("math.sqrt(16)"), 4.0);
        assert!(eval("math.sqrt(-1)").is_nan());
        assert!(eval_vars("math.sqrt(v.x)", &[("x", -1.0)]).is_nan());
        assert_bits(
            eval_vars("math.sqrt(v.x)", &[("x", 2.0)]),
            2.0_f32.sqrt(),
            "sqrt(2)",
        );
        assert_bits(
            eval_vars("math.sqrt(v.x)", &[("x", -0.0)]),
            -0.0,
            "sqrt(-0)",
        );
    }

    /// `3t² − 2t³`, `t` not clamped: on x86-64 as the terms `(3t)·t − ((t + t)·t)·t`, on arm64 as
    /// the factored `(3 − 2t)·(t·t)`.
    #[test]
    fn hermite_blend() {
        assert_eq!(eval("math.hermite_blend(0.5)"), 0.5);
        assert_eq!(eval_vars("math.hermite_blend(v.t)", &[("t", 2.0)]), -4.0);
        assert_eq!(eval_vars("math.hermite_blend(v.t)", &[("t", -1.0)]), 5.0);
        let t = f32::from_bits(0x3fde_6363);
        assert_bits(
            eval_vars("math.hermite_blend(v.t)", &[("t", t)]),
            per_arch((3.0 * t) * t - ((t + t) * t) * t, (3.0 - (t + t)) * (t * t)),
            "",
        );
    }

    /// `lerp(a, b, t)` = `a + t·(b − a)` with `t` not clamped, unfused on
    /// x86-64 and rounded once on arm64.
    #[test]
    fn lerp() {
        assert_eq!(eval("math.lerp(0, 10, 0.5)"), 5.0);
        assert_eq!(eval_vars("math.lerp(0, 10, v.t)", &[("t", 2.0)]), 20.0);
        assert_eq!(eval_vars("math.lerp(0, 10, v.t)", &[("t", -0.5)]), -5.0);
        assert_eq!(eval("math.lerp(10, 0, 0.25)"), 7.5);
        let la = 1.1_f32;
        let lb = la * 3.3;
        let t = third();
        let vars = [("a", la), ("b", lb), ("t", t)];
        let unfused = la + t * (lb - la);
        let fused = t.mul_add(lb - la, la);
        assert_ne!(unfused, fused);
        assert_bits(
            eval_vars("math.lerp(v.a, v.b, v.t)", &vars),
            per_arch(unfused, fused),
            "x86-64 lerp",
        );
    }

    /// `lerprotate` interpolates over the wrapped difference and is not
    /// re-wrapped; `min_angle` wraps to [−180, 180).
    #[test]
    fn angles() {
        assert_eq!(eval("math.lerprotate(350, 10, 0.5)"), 360.0);
        assert_eq!(
            eval_vars("math.lerprotate(v.a, 10, 0.5)", &[("a", 350.0)]),
            360.0
        );
        assert_eq!(eval("math.lerprotate(10, 350, 0.5)"), 0.0);
        assert_eq!(eval("math.lerprotate(0, 90, 2)"), 180.0);
        for (x, wrapped) in [
            (180.0_f32, -180.0_f32),
            (370.0, 10.0),
            (-370.0, -10.0),
            (-180.0, -180.0),
            (540.0, -180.0),
            (179.5, 179.5),
            (0.0, 0.0),
        ] {
            assert_eq!(
                eval_vars("math.min_angle(v.x)", &[("x", x)]),
                wrapped,
                "min_angle({x})"
            );
            assert_eq!(
                eval(&format!("math.min_angle({x})")),
                wrapped,
                "folded min_angle({x})"
            );
        }
    }

    /// `min_angle` shifts its argument by +180 before the remainder, and that sum is rounded: 2^-14
    /// above 1000 is a tie at 1180 (grid 2^-13) that rounds away, so the wrapped angle is exactly
    /// -80.
    #[test]
    fn min_angle_rounds_the_argument_shifted_by_plus_180() {
        let step = 2.0_f32.powi(-14);
        assert_bits(
            eval_vars("math.min_angle(v.x)", &[("x", 1000.0 + step)]),
            -80.0,
            "min_angle(1000 + 2^-14)",
        );
        assert_bits(
            eval_vars("math.min_angle(v.x)", &[("x", -1000.0 - step)]),
            79.999_94,
            "min_angle(-1000 - 2^-14)",
        );
    }

    /// On x86-64 a NaN gives the second operand when both are variables and is dropped against a
    /// constant; arm64 drops the NaN in either position.
    #[test]
    fn min_max_with_nan() {
        let nan = [("n", NAN), ("f", 4.0)];
        for function in ["max", "min"] {
            let first = format!("math.{function}(v.n, v.f)");
            let second = format!("math.{function}(v.f, v.n)");
            if ARCH == Arch::X86_64 {
                assert_eq!(eval_vars(&first, &nan), 4.0, "{first}");
                assert!(eval_vars(&second, &nan).is_nan(), "{second}");
            }
            if ARCH == Arch::Arm64 {
                assert_eq!(eval_vars(&first, &nan), 4.0, "{first}");
                assert_eq!(eval_vars(&second, &nan), 4.0, "{second}");
            }
            for constant in [
                format!("math.{function}(4, v.n)"),
                format!("math.{function}(v.n, 4)"),
            ] {
                assert_eq!(eval_vars(&constant, &nan), 4.0, "{constant}");
            }
            if ARCH == Arch::X86_64 {
                assert!(eval_vars(&format!("math.{function}(v.n, v.n)"), &nan).is_nan());
            }
        }
        assert_eq!(eval("math.max(0, 1)"), 1.0);
        assert_eq!(eval("math.max(-1, -2)"), -1.0);
        assert_eq!(eval("math.min(0, 1)"), 0.0);
        assert_eq!(eval("math.min(-1, -2)"), -2.0);
    }

    /// `mod` is the truncated remainder (sign of the dividend); a run-time zero
    /// divisor gives the post-op offset; a literal zero divisor has no test and gives NaN.
    #[test]
    fn mod_rules() {
        assert_eq!(
            eval_vars("math.mod(v.a, 3)", &[("a", -5.1)]),
            -5.1_f32 % 3.0
        );
        assert!((eval_vars("math.mod(v.a, 3)", &[("a", -5.1)]) + 2.1).abs() < 1e-6);
        assert_eq!(
            eval_vars("math.mod(v.a, v.b)", &[("a", 7.5), ("b", -2.0)]),
            1.5
        );
        let zero = [("x", 1.0), ("y", 0.0)];
        assert_eq!(eval_vars("math.mod(v.x, v.y)", &zero), 0.0);
        assert_eq!(eval_vars("math.mod(v.x, v.y) + 0.125", &zero), 0.125);
        assert_eq!(eval_vars("math.mod(v.x, v.y) * 3 + 0.125", &zero), 0.125);
        assert_eq!(
            eval_vars("math.mod(v.x, v.y * 2)", &zero),
            0.0,
            "an expression divisor"
        );
        assert!(eval_vars("math.mod(v.x, 0)", &zero).is_nan());
        assert!(eval("math.mod(1, 0)").is_nan());
        assert!(eval("math.mod(5, 0)").is_nan());
    }

    /// `math.pi` is the f32 π and takes no parentheses: `math.pi()` is rejected (#8).
    #[test]
    fn pi() {
        assert_bits(eval("math.pi"), f32::from_bits(0x4049_0fdb), "math.pi");
        assert_eq!(eval("math.pi"), 3.141_592_74);
        assert_eq!(eval("math.PI * 2"), 6.283_185_5);
        assert_eq!(eval("Math.Pi / 2"), std::f32::consts::FRAC_PI_2);
        let compiled = compile("math.pi()", &server_at(13));
        assert_eq!(compiled.failure(), Some(CompileFailure::Rejected));
        assert!(
            compiled
                .diagnostics()
                .iter()
                .any(|d| d.message().starts_with(
                    "found multiple operations without a combining operation between them"
                )),
            "{:?}",
            compiled.diagnostics()
        );
    }

    /// Sorted bounds, a closed interval; literal bounds fold into the post-op (`S = hi − lo`, `O =
    /// lo`). Run-time bounds interpolate as `(hi − lo)·r + lo` in one rounding on arm64 and `hi·r +
    /// (1 − r)·lo` on x86-64.
    #[test]
    fn random() {
        for (sample, plain, scaled) in [
            (0.0_f32, 2.0_f32, 10.0_f32),
            (0.5, 4.5, 17.5),
            (1.0, 7.0, 25.0),
        ] {
            assert_eq!(run("math.random(2.0, 7.0)", &[], &[sample]), (plain, 1));
            assert_eq!(
                run("math.random(7.0, 2.0)", &[], &[sample]),
                (plain, 1),
                "inverted"
            );
            assert_eq!(
                run("3.0 * math.random(2.0, 7.0) + 4.0", &[], &[sample]),
                (scaled, 1)
            );
            let vars = [("a", 2.0), ("b", 7.0)];
            assert_eq!(run("math.random(v.a, v.b)", &vars, &[sample]).0, plain);
            assert_eq!(
                run("math.random(v.b, v.a)", &vars, &[sample]).0,
                plain,
                "inverted"
            );
        }
        // Literal bounds: the folded post-op (S', O') = ((hi − lo)·S, lo·S + O), applied to r.
        let r = third();
        let folded = PostOp::new(15.0, 10.0);
        assert_bits(
            run("math.random(7, 2) * 3 + 4", &[], &[r]).0,
            folded.apply(r),
            "folded random",
        );
        assert_bits(
            run("math.random(2, 7)", &[], &[r]).0,
            PostOp::new(5.0, 2.0).apply(r),
            "folded random",
        );
        let r = third();
        assert_bits(
            run("math.random(7, 2) * 3 + 4", &[], &[r]).0,
            per_arch(r * 15.0 + 10.0, r.mul_add(15.0, 10.0)),
            "x86-64 folded",
        );
        let (lo, hi) = (third(), f32::from_bits(0x3f91_bc0d));
        let r = f32::from_bits(0x3ecf_a978);
        let vars = [("a", lo), ("b", hi)];
        assert_bits(
            run("math.random(v.a, v.b)", &vars, &[r]).0,
            per_arch(hi * r + (1.0 - r) * lo, (hi - lo).mul_add(r, lo)),
            "x86-64 run-time",
        );
        // An infinite bound survives the x86-64 form only.
        let vars = [("a", -180.0), ("b", f32::NEG_INFINITY)];
        if ARCH == Arch::X86_64 {
            assert_eq!(
                run("math.random(v.a, v.b)", &vars, &[0.25]).0,
                f32::NEG_INFINITY
            );
        }
        if ARCH == Arch::Arm64 {
            assert!(run("math.random(v.a, v.b)", &vars, &[0.25]).0.is_nan());
        }
    }

    /// A NaN bound is dropped on arm64 and kept on x86-64, in either position.
    #[test]
    fn random_with_a_nan_bound() {
        let nan = [("n", NAN)];
        for sample in [0.0_f32, 0.5, 1.0] {
            if ARCH == Arch::Arm64 {
                assert_eq!(run("math.random(v.n, 5)", &nan, &[sample]).0, 5.0);
                assert_eq!(run("math.random(5, v.n)", &nan, &[sample]).0, 5.0);
            }
            if ARCH == Arch::X86_64 {
                assert!(run("math.random(v.n, 4)", &nan, &[sample]).0.is_nan());
                assert!(run("math.random(4, v.n)", &nan, &[sample]).0.is_nan());
            }
        }
    }

    /// An infinite bound: on arm64 the literal-bounds span is infinite and so is a draw above 0;
    /// the run-time form, and x86-64 with either, gives 0.
    #[test]
    fn random_integer_with_an_infinite_bound() {
        let vars = [("a", 0.0), ("b", f32::INFINITY)];
        for sample in [0.0_f32, 0.5, 1.0] {
            for source in [
                "math.random_integer(0, 1e39)",
                "math.random_integer(1e39, 0)",
            ] {
                let (value, _) = run(source, &[], &[sample]);
                let arm64 = if sample > 0.0 { f32::INFINITY } else { 0.0 };
                assert_eq!(value, per_arch(0.0, arm64), "{source} at {sample}");
            }
            let (value, _) = run("math.random_integer(v.a, v.b)", &vars, &[sample]);
            assert_eq!(value, 0.0, "run-time bounds at {sample}");
        }
    }

    /// Sorted, unrounded bounds, an integer draw clamped to [lo, hi]; the two architectures
    /// interpolate differently.
    #[test]
    fn random_integer() {
        for (sample, plain, scaled) in [
            (0.0_f32, 2.0_f32, 10.0_f32),
            (0.5, 4.0, 16.0),
            (1.0, 7.0, 25.0),
        ] {
            assert_eq!(run("math.random_integer(2, 7)", &[], &[sample]), (plain, 1));
            assert_eq!(
                run("math.random_integer(7, 2)", &[], &[sample]).0,
                plain,
                "inverted"
            );
            assert_eq!(
                run("3 * math.random_integer(2, 7) + 4", &[], &[sample]).0,
                scaled
            );
            let vars = [("a", 7.0), ("b", 2.0)];
            assert_eq!(
                run("math.random_integer(v.a, v.b)", &vars, &[sample]).0,
                plain,
                "inverted variables"
            );
        }
        // The bounds are not rounded: at r = 0 the floor falls below lo, and the clamp returns lo.
        assert_eq!(run("math.random_integer(0.1, 1000001)", &[], &[0.0]).0, 0.1);
        assert_eq!(
            run("math.random_integer(v.a, 1000001)", &[("a", 0.1)], &[0.0]).0,
            0.1
        );
        for sample in [0.0_f32, 0.3, 0.999_999_9, 1.0] {
            let value = run(
                "math.random_integer(v.a, v.b)",
                &[("a", -2.0), ("b", 3.0)],
                &[sample],
            )
            .0;
            assert!(
                (-2.0..=3.0).contains(&value) && value.fract() == 0.0,
                "{sample} → {value}"
            );
        }
        let vars = [("a", -3.0), ("b", 2.0)];
        assert_eq!(
            run("math.random_integer(v.a, v.b)", &vars, &[0.5]).0,
            per_arch(-1.0, 0.0)
        );
    }

    /// `sign(x)` is `x < 0 ? −1 : 1`, so both zeros give 1; `sign(NaN)` is 1 on
    /// x86-64 and −1 on arm64.
    #[test]
    fn sign() {
        assert_eq!(eval("math.sign(0)"), 1.0);
        assert_eq!(eval_vars("math.sign(v.x)", &[("x", 0.0)]), 1.0);
        assert_eq!(eval_vars("math.sign(v.x)", &[("x", -0.0)]), 1.0);
        assert_eq!(eval_vars("math.sign(-v.x)", &[("x", 0.0)]), 1.0);
        assert_eq!(eval_vars("math.sign(v.x)", &[("x", -1.0e-30)]), -1.0);
        assert_eq!(eval_vars("math.sign(v.x)", &[("x", 7.0)]), 1.0);
        assert_eq!(
            eval_vars("math.sign(v.x)", &[("x", f32::NEG_INFINITY)]),
            -1.0
        );
        assert_eq!(
            eval_vars("math.sign(v.x)", &[("x", NAN)]),
            per_arch(1.0, -1.0)
        );
    }

    /// With a post-op, the negative branch of `sign` is `−(S + O)`, not `O − S`, under both
    /// architectures.
    #[test]
    fn sign_post_op_quirk() {
        let neg = [("x", -1.0)];
        assert_eq!(eval_vars("math.sign(v.x) + 1", &neg), -2.0);
        assert_eq!(eval_vars("math.sign(v.x) * -2 + 1", &neg), 1.0);
        assert_eq!(eval_vars("math.sign(v.x) * 3 + 0.5", &neg), -3.5);
        let pos = [("x", 1.0)];
        assert_eq!(eval_vars("math.sign(v.x) + 1", &pos), 2.0);
        assert_eq!(eval_vars("math.sign(v.x) * -2 + 1", &pos), -1.0);
    }

    /// A constant argument folds to its value first, so the factors, terms and negations around
    /// the call are constant arithmetic: a negative argument with a term gives `−S + O`, where the
    /// run-time post-op gives `−(S + O)`.
    #[test]
    fn sign_of_a_constant_folds_to_its_value_first() {
        for (folded, constant, run, x, run_time) in [
            (
                "math.sign(-1) * 2 + 1",
                -1.0,
                "math.sign(v.x) * 2 + 1",
                -1.0,
                -3.0,
            ),
            ("math.sign(-1) + 1", 0.0, "math.sign(v.x) + 1", -1.0, -2.0),
            (
                "math.sign(-1) * -2 + 1",
                3.0,
                "math.sign(v.x) * -2 + 1",
                -1.0,
                1.0,
            ),
            (
                "math.sign(-1) * 2 * 3 + 1",
                -5.0,
                "math.sign(v.x) * 2 * 3 + 1",
                -1.0,
                -7.0,
            ),
            (
                "1 - math.sign(-1) * 2",
                3.0,
                "1 - math.sign(v.x) * 2",
                -1.0,
                1.0,
            ),
            (
                "-math.sign(-1) * 2 + 1",
                3.0,
                "-math.sign(v.x) * 2 + 1",
                -1.0,
                1.0,
            ),
            (
                "math.sign(1) * -2 + 1",
                -1.0,
                "math.sign(v.x) * -2 + 1",
                1.0,
                -1.0,
            ),
        ] {
            assert_eq!(folded_constant(folded), constant, "{folded}");
            assert_eq!(eval_vars(run, &[("x", x)]), run_time, "{run}");
        }
        // sign(NaN) is the negative branch on arm64.
        assert_eq!(
            folded_constant("math.sign(math.sqrt(-1)) * 2 + 1"),
            per_arch(3.0, -1.0)
        );
        assert_eq!(
            eval_vars("math.sign(v.x) * 2 + 1", &[("x", NAN)]),
            per_arch(3.0, -3.0)
        );
    }

    /// `inverse_lerp(a, b, v)` = `(v − a)/(b − a)`, unclamped and with no zero guard.
    #[test]
    fn inverse_lerp() {
        assert_eq!(eval("math.inverse_lerp(1, 5, 3)"), 0.5);
        assert_eq!(
            eval_vars("math.inverse_lerp(1, 5, v.v)", &[("v", 9.0)]),
            2.0
        );
        assert_eq!(
            eval_vars("math.inverse_lerp(1, 5, v.v)", &[("v", -3.0)]),
            -1.0
        );
        assert_eq!(
            eval_vars("math.inverse_lerp(5, 5, v.v)", &[("v", 4.0)]),
            f32::NEG_INFINITY
        );
        assert_eq!(
            eval_vars("math.inverse_lerp(5, 5, v.v)", &[("v", 6.0)]),
            f32::INFINITY
        );
        assert!(eval_vars("math.inverse_lerp(5, 5, v.v)", &[("v", 5.0)]).is_nan());
    }

    type Curve = fn(f64) -> f64;

    fn powi(t: f64, n: i32) -> f64 {
        t.powi(n)
    }

    fn in_out(t: f64, first: impl Fn(f64) -> f64, second: impl Fn(f64) -> f64) -> f64 {
        let t2 = 2.0 * t;
        if t2 < 1.0 { first(t2) } else { second(t2) }
    }

    fn out_bounce(t: f64) -> f64 {
        let (n1, d1) = (7.5625, 2.75);
        if t < 1.0 / d1 {
            n1 * t * t
        } else if t < 2.0 / d1 {
            let u = t - 1.5 / d1;
            n1 * u * u + 0.75
        } else if t < 2.5 / d1 {
            let u = t - 2.25 / d1;
            n1 * u * u + 0.9375
        } else {
            let u = t - 2.625 / d1;
            n1 * u * u + 0.984_375
        }
    }

    const C1: f64 = 1.701_58;
    const C3: f64 = C1 + 1.0;
    const C2: f64 = C1 * 1.525;
    const ELASTIC: f64 = 2.0 * std::f64::consts::PI / 3.0;

    /// Every curve as a formula (f(t) of `start + (end − start)·f(t)`).
    fn curves() -> Vec<(&'static str, Curve, f64)> {
        use std::f64::consts::PI;
        // (function, f, tolerance as a fraction of the span)
        vec![
            ("ease_in_quad", |t| powi(t, 2), 1e-6),
            ("ease_out_quad", |t| 1.0 - powi(1.0 - t, 2), 1e-6),
            (
                "ease_in_out_quad",
                |t| in_out(t, |u| 0.5 * powi(u, 2), |u| 1.0 - 0.5 * powi(2.0 - u, 2)),
                1e-6,
            ),
            ("ease_in_cubic", |t| powi(t, 3), 1e-6),
            ("ease_out_cubic", |t| 1.0 - powi(1.0 - t, 3), 1e-6),
            (
                "ease_in_out_cubic",
                |t| in_out(t, |u| 0.5 * powi(u, 3), |u| 1.0 - 0.5 * powi(2.0 - u, 3)),
                1e-6,
            ),
            ("ease_in_quart", |t| powi(t, 4), 1e-6),
            ("ease_out_quart", |t| 1.0 - powi(1.0 - t, 4), 1e-6),
            (
                "ease_in_out_quart",
                |t| in_out(t, |u| 0.5 * powi(u, 4), |u| 1.0 - 0.5 * powi(2.0 - u, 4)),
                1e-6,
            ),
            ("ease_in_quint", |t| powi(t, 5), 1e-6),
            ("ease_out_quint", |t| 1.0 - powi(1.0 - t, 5), 1e-6),
            (
                "ease_in_out_quint",
                |t| in_out(t, |u| 0.5 * powi(u, 5), |u| 1.0 - 0.5 * powi(2.0 - u, 5)),
                1e-6,
            ),
            // The sine curves are quantised to the 65,536-entry table: within one entry of the
            // plain sine.
            ("ease_in_sine", |t| 1.0 - (t * PI / 2.0).cos(), 2e-4),
            ("ease_out_sine", |t| (t * PI / 2.0).sin(), 2e-4),
            ("ease_in_out_sine", |t| -((PI * t).cos() - 1.0) / 2.0, 2e-4),
            ("ease_in_expo", |t| (10.0 * t - 10.0).exp2(), 1e-6),
            ("ease_out_expo", |t| 1.0 - (-10.0 * t).exp2(), 1e-6),
            (
                "ease_in_out_expo",
                |t| {
                    if t < 0.5 {
                        (20.0 * t - 10.0).exp2() / 2.0
                    } else {
                        (2.0 - (-20.0 * t + 10.0).exp2()) / 2.0
                    }
                },
                1e-6,
            ),
            ("ease_in_circ", |t| 1.0 - (1.0 - t * t).sqrt(), 1e-6),
            (
                "ease_out_circ",
                |t| (1.0 - (t - 1.0) * (t - 1.0)).sqrt(),
                1e-6,
            ),
            (
                "ease_in_out_circ",
                |t| {
                    if t < 0.5 {
                        (1.0 - (1.0 - powi(2.0 * t, 2)).sqrt()) / 2.0
                    } else {
                        ((1.0 - powi(-2.0 * t + 2.0, 2)).sqrt() + 1.0) / 2.0
                    }
                },
                1e-6,
            ),
            ("ease_in_back", |t| t * t * (C3 * t - C1), 1e-6),
            (
                "ease_out_back",
                |t| 1.0 + C3 * powi(t - 1.0, 3) + C1 * powi(t - 1.0, 2),
                1e-6,
            ),
            (
                "ease_in_out_back",
                |t| {
                    if t < 0.5 {
                        powi(2.0 * t, 2) * ((C2 + 1.0) * 2.0 * t - C2) / 2.0
                    } else {
                        (powi(2.0 * t - 2.0, 2) * ((C2 + 1.0) * (2.0 * t - 2.0) + C2) + 2.0) / 2.0
                    }
                },
                1e-6,
            ),
            // The elastic curves read the table too, and their angle is (10t − 10.75)·2π/3.
            (
                "ease_in_elastic",
                |t| -(10.0 * t - 10.0).exp2() * ((10.0 * t - 10.75) * ELASTIC).sin(),
                2e-4,
            ),
            (
                "ease_out_elastic",
                |t| (-10.0 * t).exp2() * ((10.0 * t - 0.75) * ELASTIC).sin() + 1.0,
                2e-4,
            ),
            (
                "ease_in_out_elastic",
                |t| {
                    let s = ((20.0 * t - 10.75) * ELASTIC).sin();
                    if 2.0 * t < 1.0 {
                        -0.5 * (20.0 * t - 10.0).exp2() * s
                    } else {
                        1.0 + 0.5 * (10.0 - 20.0 * t).exp2() * s
                    }
                },
                2e-4,
            ),
            ("ease_out_bounce", out_bounce, 1e-6),
            ("ease_in_bounce", |t| 1.0 - out_bounce(1.0 - t), 1e-6),
            (
                "ease_in_out_bounce",
                |t| {
                    if t < 0.5 {
                        (1.0 - out_bounce(1.0 - 2.0 * t)) / 2.0
                    } else {
                        (1.0 + out_bounce(2.0 * t - 1.0)) / 2.0
                    }
                },
                1e-6,
            ),
        ]
    }

    fn check_curve(function: &str, f: Curve, tolerance: f64, ts: &[f32]) {
        for &(start, end) in &[(1.0_f32, 5.0_f32), (-2.0, 0.5), (3.0, -1.0)] {
            let span = f64::from(end) - f64::from(start);
            for &t in ts {
                // The endpoints of the elastic curves are special cases (`elastic_endpoints`).
                if function.contains("elastic") && (t == 0.0 || t == 1.0) {
                    continue;
                }
                let source = format!("math.{function}(v.s, v.e, v.t)");
                let actual = f64::from(eval_vars(&source, &[("s", start), ("e", end), ("t", t)]));
                let expected = f64::from(start) + span * f(f64::from(t));
                assert!(
                    (actual - expected).abs() <= tolerance * span.abs().max(1.0) + 1e-6,
                    "{function}({start}, {end}, {t}) = {actual}, the formula gives {expected}"
                );
            }
        }
        // Literal operands fold to the same value as run-time ones.
        let folded = eval(&format!("math.{function}(1, 5, 0.3)"));
        let runtime = eval_vars(&format!("math.{function}(1, 5, v.t)"), &[("t", 0.3)]);
        assert_bits(folded, runtime, &format!("folded {function}"));
    }

    const UNIT: [f32; 13] = [
        0.0, 0.05, 0.1, 0.2, 0.3, 0.4, 0.45, 0.55, 0.6, 0.75, 0.9, 0.97, 1.0,
    ];

    #[test]
    fn easing_curves_follow_their_formulas() {
        for (function, f, tolerance) in curves() {
            check_curve(function, f, tolerance, &UNIT);
        }
    }

    /// The polynomial, expo and back curves keep their formula outside [0, 1].
    #[test]
    fn easings_do_not_clamp_t() {
        let outside = [-1.0_f32, -0.5, 1.25, 2.0];
        for (function, f, tolerance) in curves() {
            let unclamped = ["quad", "cubic", "quart", "quint", "expo", "back"]
                .iter()
                .any(|kind| function.ends_with(kind));
            if unclamped {
                check_curve(function, f, tolerance.max(1e-5), &outside);
            }
        }
        assert_eq!(eval("math.ease_in_quad(0, 1, 2)"), 4.0);
        assert_eq!(
            eval_vars("math.ease_in_quad(0, 1, v.t)", &[("t", -2.0)]),
            4.0
        );
        assert_eq!(
            eval_vars("math.ease_in_cubic(0, 1, v.t)", &[("t", -2.0)]),
            -8.0
        );
        assert_eq!(
            eval_vars("math.ease_out_quad(1, 5, v.t)", &[("t", 2.0)]),
            1.0
        );
    }

    /// The sine and elastic curves read a 65,536-entry sine table at the truncated index of an
    /// angle the two architectures round differently (x86-64 in three roundings, arm64 once), so
    /// for some `t` the architectures land one entry apart.
    #[test]
    fn the_table_easings_follow_each_architectures_angle_rounding() {
        let x86: [(&str, f32, f32, f32, u32); 6] = [
            ("math.ease_in_elastic", 0.0, 1.0, 0.2235, 0xbb80_080a),
            ("math.ease_in_elastic", 0.0, 1.0, 0.4485, 0x3c3c_cdfe),
            ("math.ease_out_elastic", 0.0, 1.0, 0.0375, 0x3ee8_d482),
            ("math.ease_out_elastic", 0.0, 1.0, 0.1875, 0x3f98_ad80),
            ("math.ease_in_out_elastic", 0.0, 1.0, 0.0125, 0x2e5f_42b2),
            ("math.ease_in_out_elastic", 0.0, 1.0, 0.0875, 0xb429_1280),
        ];
        let arm64: [(&str, f32, f32, f32, u32); 4] = [
            ("math.ease_out_elastic", 0.0, 1.0, 0.0278, 0x3e9f_558a),
            ("math.ease_out_elastic", 1.0, 5.0, 0.1028, 0x40c2_8459),
            ("math.ease_in_sine", -3.0, 7.5, 0.0005, 0xc03f_fff3),
            ("math.ease_in_sine", -3.0, 7.5, 0.3, 0xbfed_8644),
        ];
        for (function, start, end, t, bits) in per_arch(x86.as_slice(), arm64.as_slice()) {
            let source = format!("{function}(v.s, v.e, v.t)");
            let vars = [("s", *start), ("e", *end), ("t", *t)];
            assert_eq!(
                eval_vars(&source, &vars).to_bits(),
                *bits,
                "{function}({start}, {end}, {t})"
            );
            let folded = format!("{function}({start}, {end}, {t})");
            assert_eq!(eval(&folded).to_bits(), *bits, "folded {folded}");
        }
    }

    /// The start as a variable holding −0.0 and as the literal `-0.0` give the same result; the
    /// other operands cannot be literals, since a decimal literal does not reach the smallest
    /// subnormal.
    #[test]
    fn in_out_bounce_second_half_with_a_negative_zero_start_evaluated() {
        let tiny = f32::from_bits(1); // 1.4e-45
        assert_eq!(eval("math.copy_sign(1, -0.0)"), -1.0);
        let vars = [("s", -0.0_f32), ("e", -tiny), ("t", 0.5)];
        for source in [
            "math.ease_in_out_bounce(v.s, v.e, v.t)",
            "math.ease_in_out_bounce(-0.0, v.e, v.t)",
            "math.ease_in_out_bounce(-0.0, v.e, 0.5)",
        ] {
            assert_eq!(
                eval_vars(source, &vars).to_bits(),
                per_arch(0.0_f32.to_bits(), (-0.0_f32).to_bits()),
                "{source}"
            );
        }
    }

    #[test]
    fn expo_has_no_endpoint_special_cases() {
        assert_eq!(
            eval_vars("math.ease_in_expo(1, 5, v.t)", &[("t", 0.0)]),
            1.003_906_25
        );
        assert_eq!(eval("math.ease_in_expo(1, 5, 0)"), 1.003_906_25);
        // e − (e − s)·2^−10
        assert_eq!(
            eval_vars("math.ease_out_expo(1, 5, v.t)", &[("t", 1.0)]),
            5.0 - 4.0 / 1024.0
        );
        assert_eq!(
            eval_vars("math.ease_in_out_expo(1, 5, v.t)", &[("t", 0.0)]),
            1.0 + 4.0 / 2048.0
        );
        assert_eq!(
            eval_vars("math.ease_in_out_expo(1, 5, v.t)", &[("t", 1.0)]),
            5.0 - 4.0 / 2048.0
        );
    }

    /// The elastic curves special-case `t == 0` (start) and `t == 1` (end); on x86-64 that end is
    /// `start + (end − start)`, on arm64 `end`.
    #[test]
    fn elastic_endpoints() {
        let (start, end) = (0.3_f32, 0.1_f32);
        assert_ne!(start + (end - start), end);
        for function in ["ease_in_elastic", "ease_out_elastic", "ease_in_out_elastic"] {
            let source = format!("math.{function}(v.s, v.e, v.t)");
            assert_eq!(
                eval_vars(&source, &[("s", start), ("e", end), ("t", 0.0)]),
                start,
                "{function} at 0"
            );
            assert_eq!(
                eval_vars(&source, &[("s", 1.0), ("e", 5.0), ("t", 1.0)]),
                5.0,
                "{function} at 1"
            );
            assert_eq!(
                eval_vars(&source, &[("s", start), ("e", end), ("t", 1.0)]),
                per_arch(start + (end - start), end),
                "{function}"
            );
        }
        let value = eval_vars("math.ease_in_out_elastic(1, 5, v.t)", &[("t", 0.3)]);
        assert!((value - 0.937_503_457).abs() <= 1e-6, "{value}");
    }

    /// At `t = 0.5` the x86-64 in-out elastic takes its second branch, `start + (−c·0.5 + c)`: for
    /// the smallest subnormal range `c` the half `c/2` ties to even (0), so the result is `c`,
    /// where the first branch (`c·0.5 + start`) would give 0.
    #[test]
    fn in_out_elastic_midpoint_on_a_subnormal_range_keeps_the_rounding_of_the_second_branch() {
        let tiny = f32::from_bits(1);
        let three = f32::from_bits(3);
        let source = "math.ease_in_out_elastic(v.s, v.e, v.t)";
        let at = |end: f32| eval_vars(source, &[("s", 0.0), ("e", end), ("t", 0.5)]);
        assert_eq!(at(tiny).to_bits(), tiny.to_bits());
        assert_eq!(at(-tiny).to_bits(), (-tiny).to_bits());
        assert_eq!(at(three).to_bits(), tiny.to_bits());
        assert_eq!(at(4.0), 2.0);
    }

    /// The sine and elastic easings read a 65,536-entry table at `trunc(rad·10430.378) mod 65536`
    /// (cosine `+ 16384`); the entry is the sine of `i·9.58738e-5` on arm64 and of `i / 10430.378`
    /// on x86-64. `math.sin` itself is not quantised.
    #[test]
    fn sine_table() {
        let scale = f32::from_bits(0x4622_f983);
        assert_eq!(scale, 10_430.378);
        let step = f32::from_bits(0x38c9_0fdb);
        assert_eq!(step, 9.587_38e-5);
        let half_pi = f32::from_bits(0x3fc9_0fdb);
        for step_count in [1_u8, 7, 30, 50, 81, 99] {
            let t = f32::from(step_count) / 100.0;
            let radians = t * half_pi;
            let index = ((radians * scale) as i32 & 0xffff) as f32;
            // ease_out_sine(0, 1, t) is the table sine of t·π/2.
            assert_bits(
                eval_vars("math.ease_out_sine(0, 1, v.t)", &[("t", t)]),
                per_arch(
                    rounded(libm::sin, index / scale),
                    rounded(libm::sin, index * step),
                ),
                "x86-64 entry",
            );
        }
        // The cosine: ease_in_sine(0, 1, t) is 1 minus the entry trunc(t·π/2·10430.378 + 16384)
        // mod 65536.
        let t = 0.3_f32;
        let index = (((t * half_pi) * scale + 16_384.0) as i32 & 0xffff) as f32;
        if ARCH == Arch::X86_64 {
            assert_bits(
                eval_vars("math.ease_in_sine(0, 1, v.t)", &[("t", t)]),
                1.0 - rounded(libm::sin, index / scale),
                "x86-64 cosine",
            );
        }
    }

    /// Every random-family sample is clamped to [0, 1] (NaN to 0) before use. A generator's samples
    /// lie in [0, 1], so raw samples outside it reach the functions the evaluator calls only when
    /// given directly.
    #[test]
    fn samples_are_clamped() {
        const ID: PostOp = PostOp::IDENTITY;
        for (sample, expected) in [(1.0_f32, 7.0_f32), (0.0, 2.0), (0.5, 4.5)] {
            assert_eq!(
                run("math.random(2, 7)", &[], &[sample]).0,
                expected,
                "random, {sample}"
            );
            assert_eq!(
                run("math.random(v.a, 7)", &[("a", 2.0)], &[sample]).0,
                expected,
                "run-time random, {sample}"
            );
        }
        for (sample, expected) in [
            (2.0_f32, 7.0_f32),
            (-1.0, 2.0),
            (NAN, 2.0),
            (f32::INFINITY, 7.0),
            (0.5, 4.5),
        ] {
            assert_eq!(
                math::random_folded(sample, math::random_const_bounds(2.0, 7.0, ID)),
                expected,
                "random, {sample}"
            );
            assert_eq!(
                math::random(2.0, 7.0, sample, ID),
                expected,
                "run-time random, {sample}"
            );
        }
        assert_eq!(math::random_integer_const_bounds(2.0, 7.0, 5.0, ID), 7.0);
        assert_eq!(math::random_integer_const_bounds(2.0, 7.0, -5.0, ID), 2.0);
        let rolled = |mut roll: math::DieRoll, samples: [f32; 2]| {
            for sample in samples {
                roll.roll(sample);
            }
            roll.finish(ID)
        };
        assert_eq!(rolled(math::DieRoll::new(2.0, 1.0, 6.0), [9.0, -9.0]), 7.0);
        assert_eq!(
            rolled(math::DieRoll::new_integer(2.0, 1.0, 6.0), [9.0, NAN]),
            7.0
        );
        assert_eq!(run("math.random_integer(2, 7)", &[], &[1.0]).0, 7.0);
        assert_eq!(
            run("math.die_roll_integer(2, 1, 6)", &[], &[1.0, 0.0]).0,
            7.0
        );
    }

    /// Draw count and order: a random operator draws once, after its bounds (and their
    /// own draws) are evaluated; `die_roll*` draws `k` times after both bounds.
    #[test]
    fn draw_count_and_order() {
        // The inner (bound) draw is first: 0.5 → lo = 0.5, then the outer draw 0.0 → 0.5.
        assert_eq!(
            run("math.random(math.random(0, 1), 10)", &[], &[0.5, 0.0]),
            (0.5, 2)
        );
        // Both bounds' draws, then three rolls of [lo, hi] = [floor(0.2·10), floor(0.9·10)] = [2,
        // 9].
        assert_eq!(
            run(
                "math.die_roll(3, math.random(0, 10), math.random(0, 10))",
                &[],
                &[0.2, 0.9, 0.0, 0.0, 1.0]
            ),
            (13.0, 5)
        );
        assert_eq!(
            run(
                "math.die_roll_integer(2, math.random(0, 10), 6)",
                &[],
                &[0.1, 0.0, 1.0]
            ),
            (7.0, 3)
        );
        // One draw per operator, none for a skipped branch.
        assert_eq!(
            run("v.c = 0; return v.c ? math.random(0, 1) : 2;", &[], &[0.5]),
            (2.0, 0)
        );
        assert_eq!(
            run(
                "math.random_integer(0, 3) + math.random(0, 1)",
                &[],
                &[1.0, 0.5]
            ),
            (3.5, 2)
        );
    }

    /// At every version; the rejected expression evaluates to 0.
    #[test]
    fn wrong_arity_is_rejected_before_evaluation() {
        for version in [1_i16, 2, 13] {
            for source in [
                "math.max(3)",
                "math.min(3)",
                "math.mod(7)",
                "math.pow(2)",
                "math.clamp(1, 2)",
                "math.abs(1, 2)",
                "math.sin()",
            ] {
                let compiled = compile(source, &server_at(version));
                assert_eq!(
                    compiled.failure(),
                    Some(CompileFailure::Rejected),
                    "{source} at v{version}"
                );
                let mut env = NoHostEnv::new();
                assert_eq!(
                    compiled
                        .expr()
                        .cloned()
                        .map_or(0.0, |e| e.eval_f32(&mut env.cx())),
                    0.0
                );
            }
            let compiled = compile("math.max(3)", &server_at(version));
            assert!(
                compiled
                    .diagnostics()
                    .iter()
                    .any(|d| d.message().starts_with("Unexpected number of parameters to Max 'math.max' function - expected 2, found 1.")),
                "{:?}",
                compiled.diagnostics()
            );
        }
    }

    /// With assignments and random draws disallowed, `math.random` and
    /// `math.random_integer` are rejected, `math.die_roll*` never are.
    #[test]
    fn random_is_a_side_effect_die_roll_is_not() {
        let strict = CompileOptions {
            allowed_ops: OpSet::all().without_assignments_or_random(),
            ..server_at(13)
        };
        let lenient = CompileOptions {
            allowed_ops: OpSet::all().without_assignments(),
            ..server_at(13)
        };
        for source in [
            "math.random(0, 1)",
            "math.random_integer(0, 1)",
            "1 + math.random(v.a, 2)",
        ] {
            assert_eq!(
                compile(source, &strict).failure(),
                Some(CompileFailure::Rejected),
                "{source}"
            );
            assert_eq!(compile(source, &lenient).failure(), None, "{source}");
        }
        for source in ["math.die_roll(1, 0, 1)", "math.die_roll_integer(2, 1, 6)"] {
            assert_eq!(compile(source, &strict).failure(), None, "{source}");
            assert_eq!(compile(source, &lenient).failure(), None, "{source}");
        }
    }
}

mod architectures {
    //! The architectures end to end: source text compiled under `X86_64` or `Arm64` and run.

    // Expected values are written with nine significant digits.
    #![allow(clippy::excessive_precision)]

    use crate::common::{assert_bits, compile_support::server_at, per_arch};
    use molangx::compile::compile;
    use molangx::numeric::{ARCH, Arch};
    use molangx::vm::{NoHostEnv, Value, VariableName};

    const NAN: f32 = f32::NAN;

    /// Compiles at `version` (which must succeed) and evaluates with `v.<name>` preset: the value
    /// and the run-time messages.
    fn run_at(version: i16, source: &str, vars: &[(&str, f32)]) -> (f32, Vec<String>) {
        let compiled = compile(source, &server_at(version));
        assert_eq!(
            compiled.failure(),
            None,
            "{source:?}: {:?}",
            compiled.diagnostics()
        );
        let expr = compiled.expr().cloned().expect("an expression");
        let mut env = NoHostEnv::new();
        for (name, value) in vars {
            env.variables
                .set(VariableName::new(name), Value::Float(*value));
        }
        let value = expr.eval_f32(&mut env.cx());
        (value, env.sink.take())
    }

    fn eval_vars(source: &str, vars: &[(&str, f32)]) -> f32 {
        let (value, messages) = run_at(13, source, vars);
        assert!(messages.is_empty(), "{source:?}: {messages:?}");
        value
    }

    fn eval(source: &str) -> f32 {
        eval_vars(source, &[])
    }

    /// The constant `source` folds to.
    #[track_caller]
    fn folded_constant(source: &str) -> f32 {
        let compiled = compile(source, &server_at(13));
        let constant = compiled.expr().and_then(|expr| expr.as_constant());
        constant.unwrap_or_else(|| panic!("{source:?} does not fold"))
    }

    /// Rounded at every step (no wider intermediate); boolean results are exactly 1.0 / 0.0.
    #[test]
    fn binary32_arithmetic_and_boolean_results() {
        // 2^24 + 1 is not an f32: in f32 the sum is 2^24 and the product exactly 1; a wider
        // intermediate would give 1 + 2^-24, which rounds to 1.0000001.
        let vars = [("a", 16_777_216.0), ("b", 1.0), ("c", 1.0 / 16_777_216.0)];
        assert_eq!(eval_vars("(v.a + v.b) * v.c", &vars), 1.0);
        let vars = [("a", 1.0), ("b", 3.0)];
        assert_bits(eval_vars("v.a / v.b", &vars), 1.0_f32 / 3.0, "1/3");
        assert_bits(
            eval_vars("v.a / v.b * v.b", &vars),
            (1.0_f32 / 3.0) * 3.0,
            "(1/3)·3",
        );
        assert_eq!(eval("7 / 2"), 3.5);
        assert_eq!(eval_vars("v.a / 2", &[("a", 7.0)]), 3.5);
        for (source, expected) in [
            ("v.a < v.b", 1.0_f32),
            ("v.a > v.b", 0.0),
            ("v.a == v.a", 1.0),
            ("v.a != v.a", 0.0),
            ("v.a && v.b", 1.0),
            ("v.a || v.b", 1.0),
            ("!v.a", 0.0),
            ("!!v.b", 1.0),
            ("2 < 3", 1.0),
            ("!5", 0.0),
        ] {
            assert_bits(
                eval_vars(source, &[("a", 2.0), ("b", 3.0)]),
                expected,
                source,
            );
        }
        assert_bits(eval("0.1"), 0.1_f32, "0.1");
        assert_bits(
            eval("1.17549435e-38"),
            f32::MIN_POSITIVE,
            "f32::MIN_POSITIVE",
        );
    }

    /// The post-op is `raw·S` then `+ O` on x86-64 and rounded once on arm64.
    #[test]
    fn post_op_rounding() {
        for source in [
            "v.x = 1 / 3; return v.x * 3 - 1;",
            "v.x = 1; v.y = 3; v.q = v.x / v.y; return v.q * 3 - 1;",
            "v.x = 1 / 3; return (v.x + 0) * 3 - 1;",
        ] {
            assert_bits(eval(source), per_arch(0.0, 2.980_232_24e-8), source);
        }
        let x = 1.0_f32 / 3.0;
        assert_bits(
            eval_vars("v.x * 3 - 1", &[("x", x)]),
            per_arch(x * 3.0 - 1.0, x.mul_add(3.0, -1.0)),
            "",
        );
    }

    /// `a / b * S + O` divides first on x86-64, `(a/b)·S + O`, and scales the
    /// numerator first on arm64, `(a·S)/b + O`, unfused.
    #[test]
    fn division_post_op() {
        let (a, b) = (1000.0_f32, 13.0_f32);
        let vars = [("a", a), ("b", b)];
        assert_ne!((a / b) * 3.0, (a * 3.0) / b);
        if ARCH == Arch::X86_64 {
            assert_bits(eval_vars("v.a / v.b * 3", &vars), (a / b) * 3.0, "x86-64");
            assert_bits(
                eval_vars("v.a / v.b * 3 + 0.5", &vars),
                (a / b) * 3.0 + 0.5,
                "x86-64 with offset",
            );
        }
        if ARCH == Arch::Arm64 {
            assert_bits(eval_vars("v.a / v.b * 3", &vars), (a * 3.0) / b, "arm64");
            assert_bits(
                eval_vars("v.a / v.b * 3 + 0.5", &vars),
                (a * 3.0) / b + 0.5,
                "arm64 with offset",
            );
        }
        // Unfused on arm64 too: a fused multiply-add of the quotient would differ here.
        let (a, b) = (1.0_f32, 3.0_f32);
        let vars = [("a", a), ("b", b)];
        if ARCH == Arch::Arm64 {
            assert_bits(
                eval_vars("v.a / v.b * 3 - 1", &vars),
                (a * 3.0) / b - 1.0,
                "arm64 scale first",
            );
        }
        assert_ne!((a * 3.0) / b - 1.0, (a / b).mul_add(3.0, -1.0));
    }

    /// `a * b * S + O` is `(a·b)·S + O` under `X86_64` and `acc·(top·S) + O` rounded once under
    /// `Arm64`, with `top·S` rounded first.
    #[test]
    fn multiplication_post_op() {
        let (k, j) = (1.4_f32, 0.75_f32);
        let vars = [("k", k), ("j", j)];
        assert_ne!((k * j) * 7.0, k * (j * 7.0));
        assert_ne!((k * j) * 7.0, j * (k * 7.0));
        if ARCH == Arch::X86_64 {
            assert_bits(eval_vars("v.k * v.j * 7", &vars), (k * j) * 7.0, "x86-64");
            assert_bits(
                eval_vars("v.k * v.j * 7 + 0.25", &vars),
                (k * j) * 7.0 + 0.25,
                "x86-64 with offset",
            );
        }
        // `top` is the left operand, `acc` the right one.
        if ARCH == Arch::Arm64 {
            assert_bits(eval_vars("v.k * v.j * 7", &vars), j * (k * 7.0), "arm64");
            assert_bits(
                eval_vars("v.k * v.j * 7 + 0.25", &vars),
                j.mul_add(k * 7.0, 0.25),
                "arm64 with offset",
            );
        }
    }

    /// A constant operand folds to its value before its parent adds a term to it or multiplies it:
    /// the product and the term are two roundings, where a run-time operand takes them into its
    /// post-op and arm64 rounds once.
    #[test]
    fn a_constant_operand_folds_to_its_value_before_a_term_or_factor() {
        let third = 1.0_f32 / 3.0;
        let bits = f32::from_bits;
        let rounded_once = bits(0xb300_0000);
        for (folded, constant, run, vars, run_time) in [
            (
                "(-1/3) * 3 + 1",
                0.0,
                "v.d * 3 + 1",
                &[("d", -third)][..],
                per_arch(0.0, rounded_once),
            ),
            (
                "(-1/3) * 3 + 1",
                0.0,
                "v.d * v.e + 1",
                &[("d", -third), ("e", 3.0)],
                per_arch(0.0, rounded_once),
            ),
            (
                "(1/3) * 3 - 1",
                0.0,
                "v.d * 3 - 1",
                &[("d", third)],
                per_arch(0.0, -rounded_once),
            ),
            (
                "1 - (1/3) * 3",
                0.0,
                "1 - v.d * 3",
                &[("d", third)],
                per_arch(0.0, rounded_once),
            ),
            (
                "math.sqrt(0.1) * 3 - 1",
                bits(0xbd52_3180),
                "math.sqrt(v.d) * 3 - 1",
                &[("d", 0.1)],
                per_arch(bits(0xbd52_3180), bits(0xbd52_3178)),
            ),
            (
                "math.inverse_lerp(0.5, 2, 0) * 3 + 1",
                0.0,
                "math.inverse_lerp(v.d, 2, v.e) * 3 + 1",
                &[("d", 0.5), ("e", 0.0)],
                per_arch(0.0, rounded_once),
            ),
            (
                "math.acos(0) + (1/3)",
                bits(0x42b4_aaab),
                "math.acos(v.x) + (1/3)",
                &[("x", 0.0)],
                per_arch(bits(0x42b4_aaab), bits(0x42b4_aaaa)),
            ),
            (
                "math.acos(1/3) * (1/3)",
                bits(0x41bc_13a6),
                "math.acos(v.x) * (1/3)",
                &[("x", third)],
                per_arch(bits(0x41bc_13a6), bits(0x41bc_13a5)),
            ),
            (
                "math.atan2(1, -3) * (1/3)",
                bits(0x4257_6b8a),
                "math.atan2(v.x, -3) * (1/3)",
                &[("x", 1.0)],
                per_arch(bits(0x4257_6b8a), bits(0x4257_6b89)),
            ),
            // The run-time form adds its constant terms together first.
            (
                "1 * 1 + (1/3) + 2",
                bits(0x4055_5556),
                "v.a * v.b + (1/3) + 2",
                &[("a", 1.0), ("b", 1.0)],
                bits(0x4055_5555),
            ),
        ] {
            assert_bits(folded_constant(folded), constant, folded);
            assert_bits(eval_vars(run, vars), run_time, run);
        }
        // Without a term the product is the plain one, and a further factor multiplies its value.
        assert_bits(folded_constant("(-1) * 0"), -0.0, "(-1) * 0");
        assert_bits(
            folded_constant("(-1/3) * 3 * 3 + 1"),
            -2.0,
            "(-1/3) * 3 * 3 + 1",
        );
    }

    /// A folded constant operand keeps its sign of zero and of NaN: a negation flips the sign bit,
    /// a zero term adds and a factor multiplies, where a run-time operand computes `S·x + O`.
    #[test]
    fn a_folded_constant_operand_keeps_its_signs_of_zero_and_nan() {
        let bits = f32::from_bits;
        for (folded, constant, run, vars, run_time) in [
            (
                "-((0) * (1))",
                -0.0,
                "-(v.a * v.b)",
                &[("a", 0.0), ("b", 1.0)][..],
                0.0,
            ),
            (
                "((-1) * (0)) + 0",
                0.0,
                "(v.a * v.b) + 0",
                &[("a", -1.0), ("b", 0.0)],
                -0.0,
            ),
            (
                "math.abs(0) * (-1)",
                -0.0,
                "math.abs(v.x) * (-1)",
                &[("x", 0.0)],
                0.0,
            ),
            (
                "-((math.sqrt(-1)) * (1))",
                bits(per_arch(0x7fc0_0000, 0xffc0_0000)),
                "-(math.sqrt(v.a) * v.b)",
                &[("a", -1.0), ("b", 1.0)],
                bits(per_arch(0xffc0_0000, 0x7fc0_0000)),
            ),
            // A constant divisor of 0 folds the division to 0 and is a factor 0 at run time.
            (
                "math.sign(-1) / 0",
                0.0,
                "math.sign(v.x) / 0",
                &[("x", -1.0)],
                -0.0,
            ),
        ] {
            assert_bits(folded_constant(folded), constant, folded);
            assert_bits(eval_vars(run, vars), run_time, run);
        }
        // An assigned constant −0 is stored as +0.
        assert_bits(eval("t.x = -0; return t.x;"), 0.0, "t.x = -0");
        assert_bits(
            eval_vars("t.x = v.z; return t.x;", &[("z", -0.0)]),
            -0.0,
            "t.x = v.z",
        );
    }

    /// An infinite constant factor folds into its operand's post-op and makes the offset `∞·O`, a
    /// NaN for an operand without an offset, where the same infinity read from a variable
    /// multiplies.
    #[test]
    fn an_infinite_constant_factor_folds_into_a_nan_offset() {
        let nan = f32::from_bits(per_arch(0xffc0_0000, 0x7fc0_0000));
        let vars = [("o", 1.0), ("inf", f32::INFINITY)];
        for (folded, run) in [
            ("(v.o < 2) * math.exp(1000) + 1", "(v.o < 2) * v.inf + 1"),
            ("(v.o < 2) * 1e39 + 1", "(v.o < 2) * v.inf + 1"),
            ("(v.o && 1) * math.exp(1000) + 1", "(v.o && 1) * v.inf + 1"),
            ("v.o * math.exp(1000) + 1", "v.o * v.inf + 1"),
            ("v.o * math.exp(1000)", "v.o * v.inf"),
        ] {
            assert_bits(eval_vars(folded, &vars), nan, folded);
            assert_bits(eval_vars(run, &vars), f32::INFINITY, run);
        }
        // An operand with an offset of 1 gets the offset ∞.
        let source = "(v.x + 1) * math.exp(1000)";
        assert_bits(eval_vars(source, &[("x", 1.0)]), f32::INFINITY, source);
    }

    /// One example of each of these fold / run-time differences from the `numeric` module docs: a
    /// literal divisor, chained constant factors, two NaNs with a constant term on the left, a NaN
    /// divisor and `math.mod`.
    #[test]
    fn literal_divisors_constant_factors_nan_terms_and_mod_differ_as_listed() {
        let bits = f32::from_bits;
        let sqrt_nan = bits(per_arch(0xffc0_0000, 0x7fc0_0000));
        let big = folded_constant("3.4e38");
        for (folded, constant, run, vars, run_time) in [
            (
                "(1/3)/3",
                0x3de3_8e39,
                "v.k/3",
                &[("k", 1.0 / 3.0)][..],
                0x3de3_8e3a,
            ),
            (
                "100/3.4e38",
                0x02c8_2a85,
                "v.k/3.4e38",
                &[("k", 100.0)],
                0x02c8_2a88,
            ),
            (
                "math.sqrt(-1)/0",
                0,
                "v.k/0",
                &[("k", sqrt_nan)],
                sqrt_nan.to_bits(),
            ),
            (
                "(0.1*3)*(1/3)",
                0x3dcc_ccce,
                "(v.k*3)*(1/3)",
                &[("k", 0.1)],
                0x3dcc_cccd,
            ),
            (
                "3.4e38*3.4e38*1e-45+1",
                0x7f80_0000,
                "v.k*3.4e38*1e-45+1",
                &[("k", big)],
                0x74ff_934a,
            ),
            (
                "math.ln(-1) + 3*math.sqrt(-1)",
                per_arch(0x7fc0_0000, 0xffc0_0000),
                "math.ln(-1) + v.k*math.sqrt(-1)",
                &[("k", 3.0)],
                sqrt_nan.to_bits(),
            ),
            (
                "1/math.sqrt(-1)",
                0,
                "1/v.k",
                &[("k", sqrt_nan)],
                per_arch(0xffc0_0000, 0),
            ),
            (
                "math.mod(3, 0)",
                0xffc0_0000,
                "math.mod(3, v.k)",
                &[("k", 0.0)],
                0,
            ),
            (
                "math.mod(-4, 2)",
                0x8000_0000,
                "math.mod(v.k, 2)",
                &[("k", -4.0)],
                0,
            ),
        ] {
            assert_bits(folded_constant(folded), bits(constant), folded);
            assert_bits(eval_vars(run, vars), bits(run_time), run);
        }
    }

    /// `/` groups before `*`; `0.1 + 0.2` is 0.30000001.
    #[test]
    fn grouping_and_f32_sums() {
        assert_eq!(eval("7 * 3 / 9"), 2.333_333_49);
        assert_eq!(eval("v.x = 1; return v.x * 3 / 9;"), 0.333_333_343);
        assert_eq!(eval("v.a = 7; return v.a * 3 / 9;"), 2.333_333_49);
        assert_eq!(eval("2 * 3 / 4 * 5"), 7.5);
        assert_eq!(eval("0.1 + 0.2"), 0.300_000_01);
        assert_eq!(eval_vars("v.a + 0.2", &[("a", 0.1)]), 0.300_000_01);
        assert_eq!(
            eval_vars("v.a + v.b", &[("a", 0.1), ("b", 0.2)]),
            0.300_000_01
        );
    }

    /// −0 is false, NaN true, and a value that is not a number (a struct) false.
    #[test]
    fn truthiness() {
        assert_eq!(eval("!-0.0"), 1.0);
        assert_eq!(eval("-0.5 ? 5 : 6"), 5.0);
        assert_eq!(eval("0.0000001 ? 5 : 6"), 5.0);
        assert_eq!(eval_vars("v.x ? 5 : 6", &[("x", -0.0)]), 6.0);
        let nan = [("n", NAN)];
        assert_eq!(eval_vars("v.n ? 5 : 6", &nan), 5.0);
        assert_eq!(eval_vars("!v.n", &nan), 0.0);
        assert_eq!(eval_vars("v.n && 1", &nan), 1.0);
        assert_eq!(eval_vars("0 || v.n", &nan), 1.0);
        assert_eq!(eval("v.s.a = 1; return v.s ? 5 : 6;"), 6.0);
        assert_eq!(eval("v.s.a = 1; return !v.s;"), 1.0);
    }

    /// `==` / `!=` are exact, and NaN is unequal to everything, itself included.
    #[test]
    fn equality_is_exact() {
        let close = [("a", 1.0), ("b", 1.000_000_1)];
        assert_eq!(eval_vars("v.a == v.b", &close), 0.0);
        assert_eq!(eval_vars("v.a != v.b", &close), 1.0);
        assert_eq!(eval_vars("v.a == v.a", &close), 1.0);
        assert_eq!(eval_vars("v.z == 0", &[("z", -0.0)]), 1.0);
        let nan = [("n", NAN), ("o", 1.0)];
        for (source, expected) in [
            ("v.n == v.n", 0.0),
            ("v.n != v.n", 1.0),
            ("v.n == v.o", 0.0),
            ("v.o == v.n", 0.0),
            ("v.n != 1", 1.0),
            ("1 != v.n", 1.0),
        ] {
            assert_eq!(eval_vars(source, &nan), expected, "{source}");
        }
    }

    /// All false on x86-64; on arm64 `<` and `<=` true, `>` and `>=` false.
    #[test]
    fn nan_comparisons() {
        let vars = [("n", NAN), ("o", 4.0)];
        let cases = [
            ("v.n < v.o", 0.0, 1.0),
            ("v.n <= v.o", 0.0, 1.0),
            ("v.n > v.o", 0.0, 0.0),
            ("v.n >= v.o", 0.0, 0.0),
            ("v.o < v.n", 0.0, 1.0),
            ("v.o <= v.n", 0.0, 1.0),
            ("v.o > v.n", 0.0, 0.0),
            ("v.o >= v.n", 0.0, 0.0),
            ("v.n < 4", 0.0, 1.0),
            ("4 > v.n", 0.0, 0.0),
            ("v.n <= v.n", 0.0, 1.0),
            ("v.n >= v.n", 0.0, 0.0),
        ];
        for (source, x86, arm) in cases {
            assert_eq!(eval_vars(source, &vars), per_arch(x86, arm), "{source} on");
        }
    }

    /// A comparison or logical result under a post-op is one of the precomputed
    /// constants `F = O`, `T = S + O`.
    #[test]
    fn comparison_results_are_precomputed_constants() {
        let vars = [("a", 1.0), ("b", 2.0)];
        assert_bits(
            eval_vars("(v.a < v.b) * 0.1 + 0.2", &vars),
            0.1_f32 + 0.2,
            "T",
        );
        assert_bits(eval_vars("(v.a > v.b) * 0.1 + 0.2", &vars), 0.2, "F");
        assert_bits(eval_vars("(v.a == v.a) * -2 - 1", &vars), -3.0, "T");
        assert_bits(eval_vars("(v.a && v.b) * 3 + 1", &vars), 4.0, "T");
        assert_bits(eval_vars("(!v.a) * 3 + 1", &vars), 1.0, "F");
        assert_bits(eval_vars("(v.a > v.b) * 3", &vars), 0.0, "F without offset");
    }

    /// On x86-64 `min`, `max` and the lower bound of `clamp` give the second operand when a NaN is
    /// involved (both operands variables; a constant operand counts as the second), and a NaN bound
    /// of `random` survives in either position.
    #[test]
    fn x86_64_nan_selects() {
        let vars = [("n", NAN), ("f", 4.0)];
        if ARCH == Arch::X86_64 {
            assert_eq!(eval_vars("math.max(v.n, v.f)", &vars), 4.0);
            assert!(eval_vars("math.max(v.f, v.n)", &vars).is_nan());
            assert_eq!(eval_vars("math.min(v.n, v.f)", &vars), 4.0);
            assert!(eval_vars("math.min(v.f, v.n)", &vars).is_nan());
            assert!(eval_vars("math.clamp(v.f, v.n, 5)", &vars).is_nan());
            assert_eq!(eval_vars("math.clamp(v.n, 1, 5)", &vars), 1.0);
            assert!(eval_vars("math.random(v.n, v.f)", &vars).is_nan());
            assert!(eval_vars("math.random(v.f, v.n)", &vars).is_nan());
            assert_eq!(eval_vars("math.max(4, v.n)", &vars), 4.0);
            assert_eq!(eval_vars("math.min(4, v.n)", &vars), 4.0);
        }
    }

    /// Arm64 `min` / `max` drop a NaN on the left or the right against a number.
    #[test]
    fn arm64_nan_selects() {
        let vars = [("n", NAN), ("f", 4.0)];
        for source in [
            "math.max(v.n, v.f)",
            "math.max(v.f, v.n)",
            "math.min(v.n, v.f)",
            "math.min(v.f, v.n)",
            "math.clamp(v.f, v.n, 5)",
            "math.random(v.n, v.f)",
            "math.random(v.f, v.n)",
        ] {
            if ARCH == Arch::Arm64 {
                assert_eq!(eval_vars(source, &vars), 4.0, "{source}");
            }
        }
        if ARCH == Arch::Arm64 {
            assert!(eval_vars("math.max(v.n, v.n)", &vars).is_nan());
        }
    }

    /// The architectures agree on all of these except a NaN run-time divisor, which takes the guard
    /// on arm64 only.
    #[test]
    fn shared_behaviour() {
        assert_eq!(eval("math.sign(0)"), 1.0);
        assert_eq!(eval_vars("math.sign(v.z)", &[("z", -0.0)]), 1.0);
        assert_eq!(eval_vars("5 / v.g", &[("g", 0.000_000_1)]), 0.0);
        assert_eq!(eval_vars("5 / v.g", &[("g", 0.0)]), 0.0);
        for (version, expected) in [(13_i16, -5.0_f32), (7, -5.0), (6, 5.0)] {
            assert_eq!(
                run_at(version, "5 / v.h", &[("h", -1.0)]).0,
                expected,
                "v{version}"
            );
        }
        assert_eq!(eval_vars("math.asin(v.w)", &[("w", 1.0004)]), 90.0);
        assert!(eval_vars("math.asin(v.w)", &[("w", 1.001)]).is_nan());
        assert_eq!(eval_vars("math.round(v.m)", &[("m", -2.5)]), -3.0);
        assert_eq!(eval("math.min_angle(180)"), -180.0);
        assert_eq!(eval_vars("math.min_angle(v.d)", &[("d", 180.0)]), -180.0);
        assert!(eval("math.mod(1, 0)").is_nan());
        assert!(eval_vars("math.mod(v.one, 0)", &[("one", 1.0)]).is_nan());
        assert_eq!(
            eval_vars("math.mod(v.one, v.zero)", &[("one", 1.0), ("zero", 0.0)]),
            0.0
        );
        assert_eq!(
            eval("t.i = 0; loop(2.5, {t.i = t.i + 1;}); return t.i;"),
            3.0
        );
        assert_eq!(
            eval("t.i = 0; loop(-1, {t.i = t.i + 1;}); return t.i;"),
            0.0
        );
        let (value, messages) = run_at(13, "v.q = 1; v.r = v.missing; v.s = 2; return 3;", &[]);
        assert_eq!(value, 0.0);
        assert_eq!(messages.len(), 1, "{messages:?}");
        assert!(messages[0].contains("variable.missing"), "{messages:?}");
        assert_eq!(eval("7 * 3 / 9"), 7.0 * 0.333_333_34);
        // Constant NaN divisors fold to 0 on both.
        assert_eq!(eval("1 / math.sqrt(-1)"), 0.0);
        assert_eq!(eval_vars("v.o / math.sqrt(-1)", &[("o", 1.0)]), 0.0);
        let nan = [("o", 1.0), ("n", NAN)];
        if ARCH == Arch::X86_64 {
            assert!(eval_vars("v.o / v.n", &nan).is_nan());
        }
        if ARCH == Arch::Arm64 {
            assert_eq!(eval_vars("v.o / v.n", &nan), 0.0);
        }
    }

    #[test]
    fn sign_of_nan() {
        assert_eq!(
            eval_vars("math.sign(v.n)", &[("n", NAN)]),
            per_arch(1.0, -1.0)
        );
    }

    /// The operand consumed last is evaluated first: a division evaluates its divisor before its
    /// numerator, a comparison and a function's arguments left to right. The first missing read
    /// aborts, so its message names the operand evaluated first.
    #[test]
    fn operand_evaluation_order() {
        for (source, first) in [
            ("v.num / v.den", "den"),
            ("v.l < v.r", "l"),
            ("v.l >= v.r", "l"),
            ("v.l * v.r", "l"),
            ("math.max(v.l, v.r)", "l"),
            ("math.pow(v.l, v.r)", "l"),
            ("math.mod(v.l, v.r)", "l"),
            ("math.atan2(v.l, v.r)", "l"),
            ("math.random(v.l, v.r)", "l"),
            ("math.clamp(v.l, v.r, v.s)", "l"),
            ("math.lerp(v.l, v.r, v.s)", "l"),
            ("math.die_roll(v.l, v.r, v.s)", "l"),
        ] {
            let (value, messages) = run_at(13, source, &[]);
            assert_eq!(value, 0.0);
            assert_eq!(messages.len(), 1, "{source}: {messages:?}");
            assert!(
                messages[0].ends_with(&format!("'variable.{first}'")),
                "{source}: {messages:?}"
            );
        }
    }
}

mod operators {
    use crate::common::{
        Samples,
        compile_support::{server_at, server_expr, server_expr_at},
        eval_with_samples, per_arch,
    };
    use molangx::compile::{CompileFailure, CompileOptions, compile};
    use molangx::numeric::{self, ARCH, Arch, PostOp};
    use molangx::rng::FixedRng;
    use molangx::stdlib::math;
    use molangx::version::MolangVersion;
    use molangx::vm::NoHostEnv;

    fn eval(source: &str) -> f32 {
        server_expr(source).eval_f32(&mut NoHostEnv::new().cx())
    }

    /// The architecture is fixed at compile time: NaN ordered comparisons are false on `X86_64`; on
    /// `Arm64` `<` and `<=` are true.
    #[test]
    fn nan_comparisons_follow_the_architecture() {
        let cases = [
            ("v.n < 1", 0.0, 1.0),
            ("v.n <= 1", 0.0, 1.0),
            ("v.n > 1", 0.0, 0.0),
            ("v.n >= 1", 0.0, 0.0),
            ("1 < v.n", 0.0, 1.0),
            ("1 > v.n", 0.0, 0.0),
        ];
        for (source, x86, arm) in cases {
            let options = CompileOptions::server(MolangVersion::LATEST);
            let expr = compile(&format!("v.n = math.sqrt(-1); return {source};"), &options)
                .expr()
                .cloned()
                .expect("an expression");
            assert_eq!(
                expr.eval_f32(&mut NoHostEnv::new().cx()),
                per_arch(x86, arm),
                "{source}"
            );
        }
    }

    /// On `X86_64` a NaN run-time divisor does not take the guard; a constant NaN divisor folds
    /// to 0.
    #[test]
    fn nan_divisor_per_architecture() {
        let source = "v.o = 1; v.n = math.sqrt(-1); return v.o / v.n;";
        let expr = compile(source, &CompileOptions::server(MolangVersion::LATEST))
            .expr()
            .cloned()
            .expect("an expression");
        let value = expr.eval_f32(&mut NoHostEnv::new().cx());
        assert_eq!(value.is_nan(), per_arch(true, false));
        if ARCH == Arch::Arm64 {
            assert_eq!(value, 0.0);
        }
        assert_eq!(eval("v.o = 1; return v.o / math.sqrt(-1);"), 0.0);
        assert_eq!(eval("1 / math.sqrt(-1)"), 0.0);
    }

    /// `math.random` with two literal bounds is folded into the node's own post-op, whose offset
    /// `lo·S + O` is fused on `Arm64` only; a draw of 0 returns that offset. With `lo` = 0.1, `S` =
    /// 3.1 and `O` = 1.1 the fused and the separately rounded offsets differ in the last bit.
    #[test]
    fn a_folded_random_draw_of_zero_returns_the_architecture_rounded_offset() {
        let expr = compile(
            "math.random(0.1, 0.5) * 3.1 + 1.1",
            &CompileOptions::server(MolangVersion::LATEST),
        )
        .expr()
        .cloned()
        .expect("an expression");
        let mut env = NoHostEnv::new();
        let mut rng = FixedRng::ZERO;
        let mut cx = env.cx();
        cx.rng = &mut rng;
        assert_eq!(0.1_f32.mul_add(3.1, 1.1).to_bits(), 0x3fb4_7ae1);
        assert_eq!((0.1_f32 * 3.1 + 1.1).to_bits(), 0x3fb4_7ae2);
        assert_eq!(
            expr.eval_f32(&mut cx).to_bits(),
            per_arch(0x3fb4_7ae2, 0x3fb4_7ae1)
        );
    }

    /// A generator's samples lie in `[0, 1]`; the random functions clamp a raw sample.
    #[test]
    fn samples_are_clamped() {
        let expr = server_expr("math.random(10, 20)");
        let mut env = NoHostEnv::new();
        for (mut rng, expected) in [
            (FixedRng::ONE, 20.0),
            (FixedRng(u32::MAX), 20.0),
            (FixedRng::ZERO, 10.0),
            (FixedRng(0x8000_0000), 10.0),
            (FixedRng::HALF, 15.0),
        ] {
            let mut cx = env.cx();
            cx.rng = &mut rng;
            assert_eq!(expr.eval_f32(&mut cx), expected, "{rng:?}");
        }
        for (sample, expected) in [(2.0, 20.0), (-1.0, 10.0), (f32::NAN, 10.0), (0.5, 15.0)] {
            assert_eq!(
                math::random(10.0, 20.0, sample, PostOp::IDENTITY),
                expected,
                "{sample}"
            );
        }
    }

    /// A random operator draws after its bounds are evaluated, one draw each, `die_roll` one per
    /// roll.
    #[test]
    fn evaluation_order_fixes_the_draws() {
        // Divisor first: 0.25·4 = 1 for the divisor, then 0.5·8 = 4 for the numerator.
        assert_eq!(
            eval_with_samples("math.random(0, 8) / math.random(0, 4)", &[0.25, 0.5]),
            (4.0, 2)
        );
        // Comparison: left first.
        assert_eq!(
            eval_with_samples("math.random(0, 1) < math.random(0, 1)", &[0.2, 0.7]),
            (1.0, 2)
        );
        assert_eq!(
            eval_with_samples("math.random(0, 1) < math.random(0, 1)", &[0.7, 0.2]),
            (0.0, 2)
        );
        // Function arguments in order.
        assert_eq!(
            eval_with_samples(
                "math.max(math.random(0, 10), 0) - math.random(0, 1)",
                &[0.5, 1.0]
            ),
            (4.0, 2)
        );
        // The bound's draw comes first, then the operator's own.
        assert_eq!(
            eval_with_samples("math.random(math.random(0, 1), 10)", &[0.5, 0.0]),
            (0.5, 2)
        );
        assert_eq!(
            eval_with_samples("math.die_roll(3, 0, 10)", &[0.1, 0.2, 0.3]).1,
            3
        );
        assert_eq!(
            eval_with_samples("math.die_roll_integer(2, 1, 6)", &[0.0, 1.0]),
            (7.0, 2)
        );
    }

    /// A run-time divisor below `f32::EPSILON` (2^-23) ends the division with 0: the numerator is
    /// never evaluated and the post-op not applied.
    #[test]
    fn division_guard_skips_the_numerator_and_the_post_op() {
        let run = |source: &str| {
            let expr = server_expr_at(source, 13);
            let mut env = NoHostEnv::new();
            let mut rng = Samples::repeat(0.5);
            let value = {
                let mut cx = env.cx();
                cx.rng = &mut rng;
                expr.eval_f32(&mut cx)
            };
            (value, rng.draws)
        };
        assert_eq!(
            run("v.g = 0.0000001; return math.random(0, 8) / v.g;"),
            (0.0, 0)
        );
        assert_eq!(run("v.g = 0.0000001; return (5 / v.g) * 2 + 3;"), (0.0, 0));
        assert_eq!(run("v.g = -0.0000001; return 5 / v.g + 3;"), (0.0, 0));
        assert_eq!(
            run("v.g = 0.25; return math.random(0, 8) / v.g;"),
            (16.0, 1)
        );
        // Exactly `f32::EPSILON` (2^-23, folded from `1 / 8388608`) is not guarded.
        assert_eq!(run("v.g = 1 / 8388608; return 1 / v.g;"), (8_388_608.0, 0));
        // On arm64 a NaN divisor gives 0 too; on x86-64 it gives NaN.
        let nan = compile(
            "v.n = math.sqrt(-1); return (1 / v.n) * 2 + 3;",
            &server_at(13),
        )
        .expr()
        .cloned()
        .expect("compiles");
        let value = nan.eval_f32(&mut NoHostEnv::new().cx());
        assert_eq!(value.is_nan(), per_arch(true, false));
        if ARCH == Arch::Arm64 {
            assert_eq!(value, 0.0);
        }
    }

    /// The crate has no array resolver, so an expression with an array stops at the link; the index
    /// rule `numeric::array_index` is `max(0, trunc(i))` modulo the length, with no element for an
    /// empty array.
    #[test]
    fn array_index_rule() {
        assert_eq!(numeric::array_index(1.9, 3), Some(1));
        assert_eq!(numeric::array_index(4.0, 3), Some(1));
        assert_eq!(numeric::array_index(-2.5, 3), Some(0));
        assert_eq!(numeric::array_index(f32::NAN, 3), Some(0));
        assert_eq!(numeric::array_index(0.0, 0), None);
        assert_eq!(
            compile("array.a[v.i]", &server_at(13)).failure(),
            Some(CompileFailure::UsesArrays)
        );
    }
}

mod division_per_version {
    use crate::common::compile_support::server_expr_at;
    use molangx::vm::{NoHostEnv, Value, VariableName};

    fn divide(version: i16, a: f32, d: f32) -> f32 {
        let expr = server_expr_at("v.a / v.d", version);
        let mut env = NoHostEnv::new();
        env.variables.set(VariableName::new("a"), Value::Float(a));
        env.variables.set(VariableName::new("d"), Value::Float(d));
        expr.eval_f32(&mut env.cx())
    }

    #[test]
    fn a_positive_divisor_divides_the_same_at_every_version() {
        for version in [-1, 0, 6, 7, 13] {
            assert_eq!(divide(version, 6.0, 2.0), 3.0, "v{version} ");
            assert_eq!(divide(version, 1.0, 4.0), 0.25, "v{version} ");
        }
    }

    #[test]
    fn a_negative_divisor_is_divided_by_in_absolute_value_up_to_version_6() {
        assert_eq!(divide(6, 6.0, -2.0), 3.0);
        assert_eq!(divide(0, 6.0, -2.0), 3.0);
        assert_eq!(divide(7, 6.0, -2.0), -3.0);
        assert_eq!(divide(13, 6.0, -2.0), -3.0);
    }

    #[test]
    fn a_tiny_divisor_ends_the_division_with_zero_at_every_version() {
        for version in [0, 6, 7, 13] {
            assert_eq!(divide(version, 5.0, 0.0), 0.0, "v{version} ");
            assert_eq!(divide(version, 5.0, 1.0e-8), 0.0, "v{version} ");
            assert_eq!(divide(version, 5.0, -1.0e-8), 0.0, "v{version} ");
        }
    }
}

mod fixed_generator {
    use crate::common::compile_support::server_expr;
    use molangx::rng::FixedRng;
    use molangx::vm::NoHostEnv;

    fn draw(source: &str, sample: f32) -> f32 {
        let expr = server_expr(source);
        let mut env = NoHostEnv::new();
        let mut rng = FixedRng::from_sample(sample).expect("a word's sample");
        let mut cx = env.cx();
        cx.rng = &mut rng;
        expr.eval_f32(&mut cx)
    }

    #[test]
    fn random_spans_its_closed_interval() {
        assert_eq!(draw("math.random(10, 20)", 0.0), 10.0);
        assert_eq!(draw("math.random(10, 20)", 0.5), 15.0);
        assert_eq!(draw("math.random(10, 20)", 1.0), 20.0);
        // Inverted bounds are sorted.
        assert_eq!(draw("math.random(20, 10)", 0.5), 15.0);
    }

    #[test]
    fn random_integer_stays_within_its_bounds() {
        for sample in [0.0, 0.25, 0.5, 0.75, 1.0] {
            let value = draw("math.random_integer(1, 6)", sample);
            assert_eq!(value.fract(), 0.0, "{sample}");
            assert!((1.0..=6.0).contains(&value), "{sample}: {value}");
        }
        assert_eq!(draw("math.random_integer(1, 6)", 0.0), 1.0);
        assert_eq!(draw("math.random_integer(1, 6)", 1.0), 6.0);
    }

    #[test]
    fn a_die_roll_sums_its_rolls() {
        assert_eq!(draw("math.die_roll(4, 10, 20)", 0.0), 40.0);
        assert_eq!(draw("math.die_roll(4, 10, 20)", 1.0), 80.0);
        assert_eq!(draw("math.die_roll(4, 10, 20)", 0.5), 60.0);
        assert_eq!(draw("math.die_roll(0, 10, 20)", 0.5), 0.0);
        assert_eq!(draw("math.die_roll_integer(3, 1, 6)", 0.0), 3.0);
        assert_eq!(draw("math.die_roll_integer(3, 1, 6)", 1.0), 18.0);
    }
}

mod generator {
    use molangx::rng::{
        FixedRng, Xorshift128,
        rand_core::{Rng, SeedableRng},
        sample,
    };

    #[test]
    fn standard_seeds() {
        assert_eq!(
            Xorshift128::STANDARD_SEED,
            [123_456_789, 362_436_069, 521_288_629, 88_675_123]
        );
        assert_eq!(Xorshift128::new().state(), Xorshift128::STANDARD_SEED);
        assert_eq!(Xorshift128::default(), Xorshift128::new());
    }

    #[test]
    fn first_outputs_from_the_standard_seeds() {
        // t = x ^ (x << 11); x = y; y = z; z = w; w = w ^ (w >> 19) ^ t ^ (t >> 8)
        let mut raw = Xorshift128::new();
        let words: Vec<u32> = (0..6).map(|_| raw.next_u32()).collect();
        assert_eq!(
            words,
            [
                3_701_687_786,
                458_299_110,
                2_500_872_618,
                3_633_119_408,
                516_391_518,
                2_377_269_574
            ]
        );

        // sample = (w & 0x7fffffff) · 2^-31 as f32
        let mut rng = Xorshift128::new();
        let samples: Vec<u32> = (0..6).map(|_| sample(&mut rng).to_bits()).collect();
        assert_eq!(
            samples,
            [
                0x3f39_468c,
                0x3e5a_88b7,
                0x3e28_824d,
                0x3f31_1a01,
                0x3e76_3c13,
                0x3ddb_2414
            ]
        );
    }

    #[test]
    fn samples_stay_in_the_closed_unit_interval() {
        let mut rng = Xorshift128::new();
        for _ in 0..100_000 {
            let sample = sample(&mut rng);
            assert!((0.0..=1.0).contains(&sample), "{sample}");
        }
    }

    #[test]
    fn a_sample_can_round_to_exactly_one() {
        // With x = 0 the next output is w ^ (w >> 19); 0xffffe000 maps to 0xffffffff, whose low 31
        // bits round up to 2^31 when converted to f32.
        let mut rng = Xorshift128::with_state([0, 1, 2, 0xffff_e000]);
        assert_eq!(rng.clone().next_u32(), 0xffff_ffff);
        assert_eq!(sample(&mut rng), 1.0);
    }

    #[test]
    fn state_and_seed_constructors() {
        let state = [1, 2, 3, 4];
        assert_eq!(Xorshift128::with_state(state).state(), state);
        // The all-zero state is a fixed point of xorshift; it falls back to the standard seeds.
        assert_eq!(Xorshift128::with_state([0; 4]), Xorshift128::new());

        let (mut a, mut b, mut c) = (
            Xorshift128::seed_from_u64(1),
            Xorshift128::seed_from_u64(1),
            Xorshift128::seed_from_u64(2),
        );
        assert_ne!(a.state(), [0; 4]);
        assert_ne!(Xorshift128::seed_from_u64(0).state(), [0; 4]);
        let (xs, ys, zs): (Vec<u32>, Vec<u32>, Vec<u32>) = (
            (0..8).map(|_| a.next_u32()).collect(),
            (0..8).map(|_| b.next_u32()).collect(),
            (0..8).map(|_| c.next_u32()).collect(),
        );
        assert_eq!(xs, ys);
        assert_ne!(xs, zs);
    }

    #[test]
    fn fixed_sources() {
        for (mut source, expected) in [
            (FixedRng::ZERO, 0.0),
            (FixedRng::HALF, 0.5),
            (FixedRng::ONE, 1.0),
        ] {
            assert_eq!(sample(&mut source), expected);
        }
        let mut fixed = FixedRng::from_sample(0.25).expect("a word's sample");
        assert_eq!((sample(&mut fixed), sample(&mut fixed)), (0.25, 0.25));
        assert_eq!(FixedRng::from_sample(f32::NAN), None);
    }

    #[test]
    fn a_mutable_reference_is_a_source() {
        fn draw_two(mut rng: impl Rng) -> (f32, f32) {
            (sample(&mut rng), sample(&mut rng))
        }
        let mut rng = Xorshift128::new();
        let first = draw_two(&mut rng);
        let dynamic: &mut dyn Rng = &mut rng;
        let second = draw_two(dynamic);
        let mut reference = Xorshift128::new();
        assert_eq!(first, (sample(&mut reference), sample(&mut reference)));
        assert_eq!(second, (sample(&mut reference), sample(&mut reference)));
    }
}

mod math_library {
    //! The functions of `molangx::stdlib::math` called directly, on this build's architecture.

    // Expected values are written with nine significant digits.
    #![allow(clippy::excessive_precision)]

    use crate::common::per_arch;
    use molangx::numeric::{ARCH, Arch, arith};
    use molangx::stdlib::math;

    use molangx::numeric::PostOp;

    use crate::common::word;
    use molangx::rng::rand_core::{Infallible, TryRng};

    /// `|actual − expected| <= tolerance`; non-finite expectations must match to the bit.
    fn within(actual: f32, expected: f32, tolerance: f32) -> bool {
        if !expected.is_finite() {
            actual.to_bits() == expected.to_bits()
        } else {
            (actual - expected).abs() <= tolerance
        }
    }

    const ID: PostOp = PostOp::IDENTITY;

    const NAN: f32 = f32::NAN;

    fn close(actual: f32, expected: f32, tolerance: f32) {
        assert!(
            within(actual, expected, tolerance),
            "{actual:e} is not {expected:e} ± {tolerance:e}"
        );
    }

    struct Scripted {
        samples: Vec<f32>,
        draws: usize,
    }

    /// Plays back the words of `samples` in a loop and counts its draws.
    impl TryRng for Scripted {
        type Error = Infallible;

        fn try_next_u32(&mut self) -> Result<u32, Infallible> {
            let sample = self.samples[self.draws % self.samples.len()];
            self.draws += 1;
            Ok(word(sample))
        }

        fn try_next_u64(&mut self) -> Result<u64, Infallible> {
            unreachable!("a sample is one next_u32")
        }

        fn try_fill_bytes(&mut self, _: &mut [u8]) -> Result<(), Infallible> {
            unreachable!("a sample is one next_u32")
        }
    }

    #[test]
    fn constants() {
        assert_eq!(math::PI, 3.141_592_74);
        assert_eq!(math::DEG_TO_RAD.to_bits(), 0x3c8e_fa35);
        assert_eq!(math::RAD_TO_DEG.to_bits(), 0x4265_2ee0);
        assert_eq!(math::INVERSE_TRIG_TOLERANCE.to_bits(), 0x3f80_1062);
        assert_eq!(math::pi(ID), 3.141_592_74);
        assert_eq!(math::pi(PostOp::new(2.0, 0.0)), 6.283_185_5);
    }

    #[test]
    fn inverse_trigonometry_in_degrees_with_the_tolerance_window() {
        close(math::acos(1.0005, ID), 0.0, 5e-4);
        close(math::acos(-1.0005, ID), 180.0, 5e-4);
        close(math::acos(-1.0001, ID), 180.0, 5e-4);
        assert!(math::acos(1.0006, ID).is_nan());
        assert!(math::acos(-1.0006, ID).is_nan());
        close(math::asin(1.0005, ID), 90.0, 5e-4);
        close(math::asin(-1.0, ID), -90.0, 5e-4);
        assert!(math::asin(1.0006, ID).is_nan());
        close(math::atan(1.0, ID), 45.0, 5e-4);
        close(math::atan2(-7.0, -7.0, ID), -135.0, 5e-4);
        assert_eq!(math::atan2(0.0, 0.0, ID), 0.0);
        // The post-op scale is merged into the degree conversion: rad·(57.29578·S) + O.
        close(math::asin(1.0, PostOp::new(2.0, 1.0)), 181.0, 5e-4);
        close(math::atan2(1.0, 1.0, PostOp::new(-2.0, 0.5)), -89.5, 5e-4);
        // A NaN argument counts as −1 on arm64 …
        if ARCH == Arch::Arm64 {
            assert_eq!(math::acos(NAN, ID), 180.0);
            assert_eq!(math::asin(NAN, ID), -90.0);
        }
        // … and gives NaN on x86-64.
        if ARCH == Arch::X86_64 {
            assert!(math::acos(NAN, ID).is_nan());
            assert!(math::asin(NAN, PostOp::new(3.0, 0.5)).is_nan());
        }
        // The post-op: on arm64 the result is rad·(57.29578·S) + O with the product of the
        // constants rounded and the rest rounded once; on x86-64 ((rad·57.29578)·S) + O, each step
        // rounded.
        let (x, post) = (
            f32::from_bits(0xbe85_e4a7),
            PostOp::new(f32::from_bits(0xbfa0_820c), f32::from_bits(0x3ffb_1d93)),
        );
        let rad = libm::atan(f64::from(x)) as f32;
        assert_eq!(
            math::atan(x, post),
            per_arch(
                (rad * math::RAD_TO_DEG) * post.scale + post.offset,
                rad.mul_add(math::RAD_TO_DEG * post.scale, post.offset)
            )
        );
        if ARCH == Arch::X86_64 {
            assert_eq!(math::atan(x, post).to_bits(), 0x41a2_b659);
        }
        assert_eq!(
            (rad * (math::RAD_TO_DEG * post.scale) + post.offset).to_bits(),
            0x41a2_b65a
        );
    }

    #[test]
    fn trigonometry_takes_degrees() {
        close(math::sin(90.0, ID), 1.0, 1e-6);
        close(math::cos(180.0, ID), -1.0, 1e-6);
        close(math::sin(180.0, ID), -8.742_278e-8, 1e-12);
        assert_eq!(math::sin(0.0, ID), 0.0);
        assert_eq!(math::cos(0.0, ID), 1.0);
        close(math::cos(0.0, PostOp::new(-2.0, 1.1)), -0.9, 1e-6);
    }

    #[test]
    fn rounding_functions() {
        assert_eq!(math::ceil(-0.5, ID).to_bits(), (-0.0_f32).to_bits());
        assert_eq!(math::ceil(1.1, ID), 2.0);
        assert_eq!(math::floor(-0.5, ID), -1.0);
        assert_eq!(math::floor(1.9, ID), 1.0);
        assert_eq!(math::round(0.5, ID), 1.0);
        assert_eq!(math::round(-0.5, ID), -1.0);
        assert_eq!(math::round(-1.5, ID), -2.0);
        assert_eq!(math::round(-2.5, ID), -3.0);
        assert_eq!(math::round(2.4, ID), 2.0);
        assert_eq!(math::trunc(1.7, ID), 1.0);
        assert_eq!(math::trunc(-1.7, ID), -1.0);
        assert_eq!(math::ceil(1.1, PostOp::new(-2.0, -1.0)), -5.0);
    }

    #[test]
    fn simple_functions() {
        assert_eq!(math::abs(-2.0, ID), 2.0);
        assert_eq!(math::abs(-0.0, ID).to_bits(), 0.0_f32.to_bits());
        assert_eq!(math::clamp(3.0, 2.0, 1.0, ID), 1.0);
        assert_eq!(math::clamp(-1.0, -2.0, -3.0, ID), -3.0);
        assert_eq!(math::clamp(1.0, 2.0, 3.0, ID), 2.0);
        assert_eq!(
            math::clamp(2.1, 0.0, 1.1, PostOp::new(2.0, 1.0)),
            arith::mul_add(1.1, 2.0, 1.0)
        );
        assert_eq!(math::copy_sign(-1.1, 3.1, ID), 1.1);
        assert_eq!(math::copy_sign(2.0, -0.0, ID), -2.0);
        assert_eq!(
            math::copy_sign(0.0, -2.0, ID).to_bits(),
            (-0.0_f32).to_bits()
        );
        assert_eq!(math::exp(0.0, ID), 1.0);
        assert_eq!(math::ln(1.0, ID), 0.0);
        assert_eq!(math::ln(0.0, ID), f32::NEG_INFINITY);
        assert!(math::sqrt(-1.0, ID).is_nan());
        assert_eq!(math::sqrt(16.0, ID), 4.0);
        assert_eq!(math::pow(2.0, 10.0, ID), 1024.0);
        assert_eq!(math::hermite_blend(0.5, ID), 0.5);
        assert_eq!(math::hermite_blend(2.0, ID), -4.0);
        assert_eq!(math::hermite_blend(0.0, ID), 0.0);
        assert_eq!(math::hermite_blend(1.0, ID), 1.0);
        assert_eq!(math::max(0.0, 1.0, ID), 1.0);
        assert_eq!(math::min(0.0, 1.0, ID), 0.0);
        assert_eq!(math::max(1.0, 2.0, PostOp::new(3.0, 1.0)), 7.0);
    }

    #[test]
    fn hermite_blend_is_factored_on_arm64_and_term_by_term_on_x86_64() {
        let t = f32::from_bits(0x3fde_6363);
        if ARCH == Arch::X86_64 {
            assert_eq!(
                math::hermite_blend(t, ID),
                (3.0 * t) * t - ((t + t) * t) * t
            );
            assert_eq!(math::hermite_blend(t, ID).to_bits(), 0xbfb7_7588);
        }
        if ARCH == Arch::Arm64 {
            assert_eq!(math::hermite_blend(t, ID), (3.0 - (t + t)) * (t * t));
            assert_eq!(math::hermite_blend(t, ID).to_bits(), 0xbfb7_7581);
        }
    }

    #[test]
    fn interpolation_and_angles() {
        assert_eq!(math::lerp(0.0, 10.0, 0.5, ID), 5.0);
        assert_eq!(math::lerp(0.0, 10.0, 2.0, ID), 20.0);
        assert_eq!(math::lerp(0.0, 10.0, 0.5, PostOp::new(2.0, 1.0)), 11.0);
        assert_eq!(math::lerprotate(350.0, 10.0, 0.5, ID), 360.0);
        assert_eq!(math::lerprotate(10.0, 350.0, 0.5, ID), 0.0);
        assert_eq!(math::inverse_lerp(1.0, 5.0, 3.0, ID), 0.5);
        assert_eq!(math::inverse_lerp(1.0, 5.0, 9.0, ID), 2.0);
        assert_eq!(math::inverse_lerp(5.0, 5.0, 4.0, ID), f32::NEG_INFINITY);
        assert!(math::inverse_lerp(5.0, 5.0, 5.0, ID).is_nan());
        for (x, wrapped) in [
            (180.0, -180.0),
            (-180.0, -180.0),
            (540.0, -180.0),
            (370.0, 10.0),
            (-370.0, -10.0),
            (0.0, 0.0),
            (179.0, 179.0),
        ] {
            assert_eq!(math::min_angle(x, ID), wrapped, "min_angle({x})");
        }
    }

    #[test]
    fn easing_endpoints_and_special_cases() {
        // Expo has no endpoint special cases.
        assert_eq!(math::ease_in_expo(1.0, 5.0, 0.0, ID), 1.003_906_25);
        assert_eq!(math::ease_out_expo(1.0, 5.0, 1.0, ID), 4.996_093_75);
        assert_eq!(math::ease_in_out_expo(1.0, 5.0, 0.0, ID), 1.001_953_12);
        // Elastic special-cases t == 0 and t == 1 (2t == 2 for in-out).
        for ease in [
            math::ease_in_elastic,
            math::ease_out_elastic,
            math::ease_in_out_elastic,
        ] {
            assert_eq!(ease(1.0, 5.0, 0.0, ID), 1.0);
            assert_eq!(ease(1.0, 5.0, 1.0, ID), 5.0);
            assert_eq!(ease(1.0, 5.0, -0.0, ID), 1.0);
        }
        close(
            math::ease_in_out_elastic(1.0, 5.0, 0.3, ID),
            0.937_503_457,
            1e-6,
        );
        // t is not clamped.
        assert_eq!(math::ease_in_quad(0.0, 1.0, 2.0, ID), 4.0);
        assert_eq!(math::ease_in_cubic(0.0, 1.0, -2.0, ID), -8.0);
        assert_eq!(math::ease_out_quad(0.0, 1.0, 2.0, ID), 0.0);
        // The post-op applies to the eased value.
        assert_eq!(
            math::ease_in_quad(0.0, 1.0, 2.0, PostOp::new(2.0, 1.0)),
            9.0
        );
        assert_eq!(
            math::ease_in_elastic(1.0, 5.0, 1.0, PostOp::new(2.0, 1.0)),
            11.0
        );
    }

    #[test]
    fn the_two_architectures_use_different_easing_formulas() {
        // x86-64: c·t·t·t·t rounded left to right; arm64: t² squared, the last step rounded once.
        assert_eq!(
            math::ease_in_quart(0.0, 1.0, 0.1, ID).to_bits(),
            per_arch(0x38d1_b718, 0x38d1_b719)
        );
        if ARCH == Arch::X86_64 {
            assert_eq!(
                math::ease_in_quart(0.0, 1.0, 0.1, ID),
                (((1.0_f32 * 0.1) * 0.1) * 0.1) * 0.1
            );
        }
        // The elastic angle is ((x − 0.075)·2π)/0.3 on x86-64 and t·20.943951 − 22.514748 rounded
        // once on arm64: the table index can differ by one entry.
        if ARCH == Arch::X86_64 {
            assert_eq!(
                math::ease_in_elastic(1.0, 5.0, 0.9, ID).to_bits(),
                0x386a_4000
            );
        }
        if ARCH == Arch::Arm64 {
            assert_ne!(
                math::ease_in_elastic(1.0, 5.0, 0.9, ID).to_bits(),
                0x386a_4000
            );
        }
        // The elastic end point is start + (end − start) on x86-64 and `end` on arm64.
        let (start, end) = (0.3_f32, 0.1_f32);
        assert_ne!(start + (end - start), end);
        for ease in [
            math::ease_in_elastic,
            math::ease_out_elastic,
            math::ease_in_out_elastic,
        ] {
            assert_eq!(
                ease(start, end, 1.0, ID),
                per_arch(start + (end - start), end)
            );
        }
        // Infinite and huge operands: the left-to-right form keeps what the reassociated form
        // loses.
        if ARCH == Arch::X86_64 {
            assert_eq!(math::ease_in_quad(-0.0, 0.0, -1.0e38, ID), 0.0);
        }
        if ARCH == Arch::Arm64 {
            assert!(math::ease_in_quad(-0.0, 0.0, -1.0e38, ID).is_nan());
        }
        if ARCH == Arch::X86_64 {
            assert_eq!(
                math::ease_in_out_quad(1.0, 5.0, f32::INFINITY, ID),
                f32::NEG_INFINITY
            );
        }
    }

    /// In the second half of `in_out_bounce` the bounce product is zero-normalised on x86-64 before
    /// the halves are added. With `start = -0` and a `d` so small that `d * 0.5` underflows to -0,
    /// the sign of the result is the sign of that `+0` (on arm64 the product is rounded with the
    /// sum, which keeps the other sign).
    #[test]
    fn in_out_bounce_second_half_with_a_negative_zero_start() {
        let tiny = f32::from_bits(1); // 1.4e-45
        assert_eq!(
            math::ease_in_out_bounce(-0.0, -tiny, 0.5, ID).to_bits(),
            per_arch(0.0_f32.to_bits(), (-0.0_f32).to_bits())
        );
    }

    type Ease = fn(f32, f32, f32, PostOp) -> f32;

    #[test]
    fn every_easing_runs_from_start_to_end() {
        let exact: [(&str, Ease); 21] = [
            ("in_quad", math::ease_in_quad),
            ("out_quad", math::ease_out_quad),
            ("in_out_quad", math::ease_in_out_quad),
            ("in_cubic", math::ease_in_cubic),
            ("out_cubic", math::ease_out_cubic),
            ("in_out_cubic", math::ease_in_out_cubic),
            ("in_quart", math::ease_in_quart),
            ("out_quart", math::ease_out_quart),
            ("in_out_quart", math::ease_in_out_quart),
            ("in_quint", math::ease_in_quint),
            ("out_quint", math::ease_out_quint),
            ("in_out_quint", math::ease_in_out_quint),
            ("in_circ", math::ease_in_circ),
            ("out_circ", math::ease_out_circ),
            ("in_out_circ", math::ease_in_out_circ),
            ("in_bounce", math::ease_in_bounce),
            ("out_bounce", math::ease_out_bounce),
            ("in_out_bounce", math::ease_in_out_bounce),
            ("in_elastic", math::ease_in_elastic),
            ("out_elastic", math::ease_out_elastic),
            ("in_out_elastic", math::ease_in_out_elastic),
        ];
        // Table quantisation, the 2^-10 residue of expo and the rounded back constants keep these
        // slightly off their endpoints.
        let approximate: [(&str, Ease); 9] = [
            ("in_sine", math::ease_in_sine),
            ("out_sine", math::ease_out_sine),
            ("in_out_sine", math::ease_in_out_sine),
            ("in_expo", math::ease_in_expo),
            ("out_expo", math::ease_out_expo),
            ("in_out_expo", math::ease_in_out_expo),
            ("in_back", math::ease_in_back),
            ("out_back", math::ease_out_back),
            ("in_out_back", math::ease_in_out_back),
        ];
        for (name, ease) in exact {
            assert!(within(ease(1.0, 5.0, 0.0, ID), 1.0, 1e-6), "{name} at 0");
            assert!(within(ease(1.0, 5.0, 1.0, ID), 5.0, 1e-6), "{name} at 1");
            assert!(ease(1.0, 5.0, NAN, ID).is_nan(), "{name} of NaN");
        }
        for (name, ease) in approximate {
            assert!(within(ease(1.0, 5.0, 0.0, ID), 1.0, 5e-3), "{name} at 0");
            assert!(within(ease(1.0, 5.0, 1.0, ID), 5.0, 5e-3), "{name} at 1");
        }
        // Every in-out curve passes through the midpoint.
        let in_out: [(&str, Ease); 9] = [
            ("quad", math::ease_in_out_quad),
            ("cubic", math::ease_in_out_cubic),
            ("quart", math::ease_in_out_quart),
            ("quint", math::ease_in_out_quint),
            ("sine", math::ease_in_out_sine),
            ("expo", math::ease_in_out_expo),
            ("circ", math::ease_in_out_circ),
            ("bounce", math::ease_in_out_bounce),
            ("back", math::ease_in_out_back),
        ];
        for (name, ease) in in_out {
            assert!(
                within(ease(1.0, 5.0, 0.5, ID), 3.0, 1e-3),
                "in_out_{name} at 0.5"
            );
        }
    }

    /// The table's entries follow the architecture.
    #[test]
    fn sine_easings_are_quantised_by_the_sine_table() {
        let scale = f32::from_bits(0x4622_f983);
        let step_angle = f32::from_bits(0x38c9_0fdb);
        let half_pi = f32::from_bits(0x3fc9_0fdb);
        let sin = |x: f32| libm::sin(f64::from(x)) as f32;
        let mut differs_from_plain_sine = false;
        for step in 1..100_u8 {
            let t = f32::from(step) / 100.0;
            let radians = t * half_pi;
            // ease_out_sine(0, 1, t) is the table sine itself: the entry trunc(x·10430.378)
            // mod 65536.
            let index = ((radians * scale) as i32 & 0xffff) as f32;
            assert_eq!(
                math::ease_out_sine(0.0, 1.0, t, ID),
                per_arch(sin(index / scale), sin(index * step_angle))
            );
            let eased = math::ease_out_sine(0.0, 1.0, t, ID);
            close(eased, sin(radians), 1e-4);
            differs_from_plain_sine |= eased != sin(radians);
        }
        assert!(
            differs_from_plain_sine,
            "the table sine should not equal the plain sine everywhere"
        );
    }

    #[test]
    fn random_samples_are_clamped() {
        assert_eq!(math::clamp_sample(0.25), 0.25);
        assert_eq!(math::clamp_sample(2.0), 1.0);
        assert_eq!(math::clamp_sample(-1.0), 0.0);
        assert_eq!(math::clamp_sample(-0.0).to_bits(), 0.0_f32.to_bits());
        assert_eq!(math::clamp_sample(NAN), 0.0);
        assert_eq!(math::clamp_sample(1.0), 1.0);
        assert_eq!(math::random(2.0, 7.0, 9.0, ID), 7.0);
        assert_eq!(math::random(2.0, 7.0, -9.0, ID), 2.0);
    }

    #[test]
    fn random_and_random_integer() {
        for (sample, real, integer) in [(0.0, 2.0, 2.0), (0.5, 4.5, 4.0), (1.0, 7.0, 7.0)] {
            assert_eq!(math::random(2.0, 7.0, sample, ID), real);
            assert_eq!(math::random(7.0, 2.0, sample, ID), real, "inverted bounds");
            assert_eq!(math::random_integer(2.0, 7.0, sample, ID), integer);
            assert_eq!(
                math::random_integer(7.0, 2.0, sample, ID),
                integer,
                "inverted bounds"
            );
            assert_eq!(
                math::random_integer_const_bounds(2.0, 7.0, sample, ID),
                integer
            );
            assert_eq!(
                math::random(2.0, 7.0, sample, PostOp::new(2.0, 1.0)),
                real * 2.0 + 1.0
            );
            // Literal bounds fold into the post-op: S' = (hi − lo)·S, O' = lo·S + O.
            let folded = math::random_const_bounds(7.0, 2.0, PostOp::new(2.0, 1.0));
            assert_eq!(folded, PostOp::new(10.0, 5.0));
            assert_eq!(math::random_folded(sample, folded), real * 2.0 + 1.0);
            assert_eq!(
                math::random_folded(sample, math::random_const_bounds(2.0, 7.0, ID)),
                real
            );
        }
        // The bounds are not rounded: the sample floors below the lower bound, which is returned.
        assert_eq!(math::random_integer(0.1, 1_000_001.0, 0.0, ID), 0.1);
        assert_eq!(
            math::random_integer_const_bounds(0.1, 1_000_001.0, 0.0, ID),
            0.1
        );
        assert_eq!(
            math::random_integer(0.0, 3.0, 1.0, PostOp::new(2.0, 1.0)),
            7.0
        );
    }

    #[test]
    fn die_rolls_draw_once_per_roll() {
        const NO_LIMIT: u32 = u32::MAX;
        for (sample, real, integer) in [(0.0, 3.0, 3.0), (0.5, 10.5, 9.0), (1.0, 18.0, 18.0)] {
            let mut rng = Scripted {
                samples: vec![sample],
                draws: 0,
            };
            assert_eq!(
                math::die_roll(3.0, 1.0, 6.0, NO_LIMIT, &mut rng, ID),
                Some(real)
            );
            assert_eq!(rng.draws, 3);
            assert_eq!(
                math::die_roll(3.0, 6.0, 1.0, NO_LIMIT, &mut rng, ID),
                Some(real),
                "inverted bounds"
            );
            assert_eq!(rng.draws, 6);
            assert_eq!(
                math::die_roll_integer(3.0, 1.0, 6.0, NO_LIMIT, &mut rng, ID),
                Some(integer)
            );
            assert_eq!(rng.draws, 9);
        }
        // The count is truncated and the bounds are floored.
        let mut rng = Scripted {
            samples: vec![1.0],
            draws: 0,
        };
        assert_eq!(
            math::die_roll(2.9, 1.9, 6.9, NO_LIMIT, &mut rng, ID),
            Some(12.0)
        );
        assert_eq!(rng.draws, 2);
        // Fewer than one roll: no draw, result 0 (then the post-op).
        for n in [0.9, 0.0, -1.0, NAN] {
            assert_eq!(
                math::die_roll(n, 1.0, 6.0, NO_LIMIT, &mut rng, ID),
                Some(0.0)
            );
            assert_eq!(
                math::die_roll_integer(n, 1.0, 6.0, 0, &mut rng, PostOp::new(2.0, 1.5)),
                Some(1.5)
            );
        }
        assert_eq!(rng.draws, 2);
        let mut rng = Scripted {
            samples: vec![0.0, 1.0],
            draws: 0,
        };
        assert_eq!(
            math::die_roll(2.0, 1.0, 6.0, NO_LIMIT, &mut rng, ID),
            Some(7.0)
        );
        assert_eq!(
            math::die_roll_integer(2.0, 1.0, 6.0, NO_LIMIT, &mut rng, PostOp::new(2.0, 1.0)),
            Some(15.0)
        );
        assert_eq!(math::die_roll_count(3.9), 3);
        assert_eq!(math::die_roll_count(-3.9), 0);
        assert_eq!(math::die_roll_count(NAN), 0);
        assert_eq!(math::die_roll_count(1.0e10), per_arch(0, i32::MAX as u32));
    }

    #[test]
    fn die_rolls_are_bounded_by_the_caller() {
        // More rolls than the limit: nothing is drawn and the caller decides what happens.
        let mut rng = Scripted {
            samples: vec![0.5],
            draws: 0,
        };
        assert_eq!(math::die_roll(5.0, 1.0, 6.0, 4, &mut rng, ID), None);
        assert_eq!(
            math::die_roll_integer(2_000_000_000.0, 1.0, 6.0, 1_024, &mut rng, ID),
            None
        );
        assert_eq!(rng.draws, 0);
        assert_eq!(math::die_roll(5.0, 1.0, 6.0, 5, &mut rng, ID), Some(17.5));
        assert_eq!(rng.draws, 5);

        // Rolling step by step gives the same bits as the one-call form, for both operators.
        let samples = vec![0.1, 0.9, 0.333_333_34, 0.5, 1.0, 0.0, 0.777];
        let post = PostOp::new(1.5, -0.25);
        for integer in [false, true] {
            let mut rng = Scripted {
                samples: samples.clone(),
                draws: 0,
            };
            let whole = if integer {
                math::die_roll_integer(7.0, -2.5, 9.25, u32::MAX, &mut rng, post)
            } else {
                math::die_roll(7.0, -2.5, 9.25, u32::MAX, &mut rng, post)
            };
            let mut roll = if integer {
                math::DieRoll::new_integer(7.0, -2.5, 9.25)
            } else {
                math::DieRoll::new(7.0, -2.5, 9.25)
            };
            assert_eq!(roll.remaining(), 7);
            for (made, sample) in samples.iter().enumerate() {
                roll.roll(*sample);
                assert_eq!(roll.remaining() as usize, 6 - made);
            }
            let before = roll;
            roll.roll(0.5);
            assert_eq!(roll, before);
            assert_eq!(Some(roll.finish(post).to_bits()), whole.map(f32::to_bits));
        }
        // A caller may stop early: the sum of the rolls made so far.
        let mut roll = math::DieRoll::new(100.0, 1.0, 6.0);
        roll.roll(0.0);
        roll.roll(1.0);
        assert_eq!(roll.remaining(), 98);
        assert_eq!(roll.finish(ID), 7.0);
        // No rolls: 0, then the post-op.
        assert_eq!(math::DieRoll::new(0.5, 1.0, 6.0).remaining(), 0);
        assert_eq!(
            math::DieRoll::new_integer(NAN, 1.0, 6.0).finish(PostOp::new(2.0, 1.5)),
            1.5
        );
    }
}

mod architecture_primitives {
    //! The primitives of the two architectures: every difference between `X86_64` and `Arm64` and
    //! what they share.

    // Expected values are written with nine significant digits.
    #![allow(clippy::excessive_precision)]

    use crate::common::per_arch;
    use molangx::numeric::{self, ARCH, Arch, PostOp, arith};
    use molangx::stdlib::math;

    const ID: PostOp = PostOp::IDENTITY;

    const NAN: f32 = f32::NAN;

    /// The literal `1 / 3` as it folds.
    fn third() -> f32 {
        numeric::fold_const_div(1.0, 3.0)
    }

    #[test]
    fn identity_post_op_returns_the_raw_value() {
        assert_eq!(ID.apply(-0.0).to_bits(), (-0.0_f32).to_bits());
        // A real post-op of (1, 0) would turn -0 into +0; the plain instruction form does not.
        assert_eq!(math::ceil(-0.5, ID).to_bits(), (-0.0_f32).to_bits());
        assert!(ID.is_identity());
        assert!(PostOp::new(1.0, -0.0).is_identity());
        assert!(!PostOp::new(1.0, 1.0).is_identity());
        assert_eq!(PostOp::default(), ID);
    }

    #[test]
    fn negate_and_add_apply_the_post_op() {
        assert_eq!(numeric::negate(2.5, ID), -2.5);
        assert_eq!(numeric::negate(0.0, ID).to_bits(), (-0.0_f32).to_bits());
        // O − x·S
        assert_eq!(numeric::negate(2.5, PostOp::new(2.0, 1.0)), -4.0);
        assert_eq!(numeric::add(0.1, 0.2, ID), 0.300_000_01);
        assert_eq!(numeric::add(1.5, 2.5, PostOp::new(2.0, 1.0)), 9.0);
        let x = third();
        assert_eq!(
            numeric::negate(x, PostOp::new(3.0, 1.0)),
            per_arch(1.0 - x * 3.0, (-x).mul_add(3.0, 1.0))
        );
        assert_ne!(1.0 - x * 3.0, (-x).mul_add(3.0, 1.0));
    }

    #[test]
    fn optimiser_folds_compose_post_ops() {
        // (x·2 + 1)·3 under a node (·5 + 7): k = 5·3, scale = 15·2, offset = 15·1 + 7.
        assert_eq!(
            PostOp::new(5.0, 7.0).fold_scaled(3.0, PostOp::new(2.0, 1.0)),
            PostOp::new(30.0, 22.0)
        );
        assert_eq!(ID.fold_scaled(3.0, ID), PostOp::new(3.0, 0.0));
        // −(x·2 + 1) under a node (·5 + 7): scale = −(5·2), offset = 7 − 1·5.
        assert_eq!(
            PostOp::new(5.0, 7.0).fold_negated(PostOp::new(2.0, 1.0)),
            PostOp::new(-10.0, 2.0)
        );
        assert_eq!(ID.fold_negated(ID), PostOp::new(-1.0, 0.0));
        // x86-64 scales the child's offset by the constant first: ((c·child.O)·S) + O, where arm64
        // computes (S·c)·child.O + O in one rounding. They differ once the node's own scale is
        // not 1.
        let (parent, c, child) = (PostOp::new(0.1, 0.25), 0.3_f32, PostOp::new(2.0, 0.7));
        assert_eq!(
            parent.fold_scaled(c, child).offset,
            per_arch(
                (c * child.offset) * parent.scale + parent.offset,
                (parent.scale * c).mul_add(child.offset, parent.offset)
            )
        );
        assert_ne!(
            (c * child.offset) * parent.scale,
            (parent.scale * c) * child.offset
        );
        assert_eq!(
            parent.fold_scaled(c, child).scale,
            (parent.scale * c) * child.scale
        );
        // The offset is rounded once on arm64 only.
        let (parent, child) = (PostOp::new(3.0, -1.0), PostOp::new(1.0, third()));
        assert_eq!(
            parent.fold_scaled(1.0, child).offset,
            per_arch(0.0, 2.980_232_2e-8)
        );
        // x86-64 multiplies the constant into the child's offset before the node's scale: with S =
        // 0.1, O = 0.1, c = 0.3 and O' = 0.7 that is 0.12100001, where the other grouping gives
        // 0.121.
        let (parent, child) = (PostOp::new(0.1, 0.1), PostOp::new(1.0, 0.7));
        if ARCH == Arch::X86_64 {
            assert_eq!(parent.fold_scaled(0.3, child).offset.to_bits(), 0x3df7_ceda);
        }
    }

    #[test]
    fn nan_comparisons_select_on_lt_on_arm64() {
        // NaN <, <=, >, >=, ==, != 1 → 1 / 1 / 0 / 0 / 0 / 1, in both operand orders.
        for (a, b) in [(NAN, 1.0), (1.0, NAN), (NAN, NAN)] {
            if ARCH == Arch::Arm64 {
                assert!(numeric::lt(a, b));
                assert!(numeric::le(a, b));
                assert!(!numeric::gt(a, b));
                assert!(!numeric::ge(a, b));
            }
        }
    }

    #[test]
    fn ordinary_comparisons_agree_on_both_architectures() {
        let samples = [
            -2.0_f32,
            -0.0,
            0.0,
            1.0,
            1.000_000_1,
            f32::INFINITY,
            f32::NEG_INFINITY,
        ];
        for a in samples {
            for b in samples {
                assert_eq!(numeric::lt(a, b), a < b);
                assert_eq!(numeric::le(a, b), a <= b);
                assert_eq!(numeric::gt(a, b), a > b);
                assert_eq!(numeric::ge(a, b), a >= b);
            }
        }
    }

    #[test]
    fn comparison_results_are_precomputed_constants() {
        // F = O, T = S + O; no multiplication happens at run time.
        assert_eq!(ID.select(true), 1.0);
        assert_eq!(ID.select(false), 0.0);
        let post = PostOp::new(2.0, 1.1);
        assert_eq!(post.select(true), 2.0 + 1.1);
        assert_eq!(post.select(false), 1.1);
        assert_eq!(post.truthy_value(), 3.1);
        assert_eq!(post.falsy_value(), 1.1);
    }

    #[test]
    fn min_max_ignore_nan_on_arm64() {
        if ARCH == Arch::Arm64 {
            assert_eq!(math::max(NAN, 4.0, ID), 4.0);
            assert_eq!(math::max(4.0, NAN, ID), 4.0);
            assert_eq!(math::min(NAN, 4.0, ID), 4.0);
            assert_eq!(math::min(4.0, NAN, ID), 4.0);
            assert!(math::max(NAN, NAN, ID).is_nan());
        }
        // Arm64 `max` / `min` order the zeros.
        if ARCH == Arch::Arm64 {
            assert_eq!(math::max(-0.0, 0.0, ID).to_bits(), 0.0_f32.to_bits());
            assert_eq!(math::max(0.0, -0.0, ID).to_bits(), 0.0_f32.to_bits());
            assert_eq!(math::min(0.0, -0.0, ID).to_bits(), (-0.0_f32).to_bits());
            assert_eq!(math::min(-0.0, 0.0, ID).to_bits(), (-0.0_f32).to_bits());
        }
    }

    #[test]
    fn sign_post_op_quirk() {
        // v.x = -1; math.sign(v.x) + 1 → −2 (not 0); math.sign(v.x) * -2 + 1 → 1 (not 3).
        assert_eq!(math::sign(-1.0, PostOp::new(1.0, 1.0)), -2.0);
        assert_eq!(math::sign(-1.0, PostOp::new(-2.0, 1.0)), 1.0);
        assert_eq!(math::sign(1.0, PostOp::new(1.0, 1.0)), 2.0);
        assert_eq!(math::sign(1.0, PostOp::new(-2.0, 1.0)), -1.0);
    }

    #[test]
    fn random_interpolation_differs_between_the_architectures() {
        // arm64: lo + r·(hi − lo) rounded once; x86-64: hi·r + (1 − r)·lo.
        let (lo, hi, r) = (
            third(),
            f32::from_bits(0x3f91_bc0d),
            f32::from_bits(0x3ecf_a978),
        );
        assert_eq!(
            math::random(lo, hi, r, ID),
            per_arch(hi * r + (1.0 - r) * lo, (hi - lo).mul_add(r, lo))
        );
        if ARCH == Arch::X86_64 {
            assert_eq!(math::random(lo, hi, r, ID).to_bits(), 0x3f28_f09f);
        }
        // The same operands in arm64's shape but rounded step by step are one ulp lower.
        assert_eq!(((hi - lo) * r + lo).to_bits(), 0x3f28_f09e);
        // An infinite bound survives the x86-64 form and not the difference hi − lo.
        if ARCH == Arch::X86_64 {
            assert_eq!(
                math::random(-180.0, f32::NEG_INFINITY, 0.0, ID),
                f32::NEG_INFINITY
            );
        }
        if ARCH == Arch::Arm64 {
            assert!(math::random(f32::NEG_INFINITY, f32::INFINITY, 0.5, ID).is_nan());
        }
        // The integer draw: floor((1 − r)·lo + r·top) with top = (hi·(−ε) + 1) + hi on x86-64,
        // floor(lo + r·span) with span = (hi + 1 − lo) − hi·ε on arm64.
        let (lo, hi, r) = (-3.0_f32, 2.0_f32, 0.5_f32);
        let top = (-f32::EPSILON * hi + 1.0) + hi;
        if ARCH == Arch::X86_64 {
            assert_eq!(
                math::random_integer(lo, hi, r, ID),
                ((1.0 - r) * lo + top * r).floor()
            );
            assert_eq!(
                math::random_integer_const_bounds(lo, hi, r, ID),
                ((1.0 - r) * lo + top * r).floor()
            );
        }
        let span = hi.mul_add(-f32::EPSILON, (hi + 1.0) - lo);
        if ARCH == Arch::Arm64 {
            assert_eq!(
                math::random_integer(lo, hi, r, ID),
                span.mul_add(r, lo).floor()
            );
        }
        // The architectures return different integers here: 6 − 2ε rounds to 6 under Arm64 and −3 +
        // 6·0.5 is 0, while the x86-64 form, 0.5·(−3) + 0.5·2.9999998, is just below 0.
        assert_eq!(math::random_integer(lo, hi, r, ID), per_arch(-1.0, 0.0));
        let mut half = molangx::rng::FixedRng::HALF;
        assert_eq!(
            math::die_roll_integer(1.0, lo, hi, 1, &mut half, ID),
            per_arch(Some(-1.0), Some(0.0))
        );
        // One roll of a die is one such draw.
        let mut rng = molangx::rng::FixedRng(crate::common::word(third()));
        assert_eq!(
            math::die_roll(1.0, 1.1, 7.3, 1, &mut rng, ID),
            per_arch(
                Some(0.0 + (third() * 7.0 + (1.0 - third()) * 1.0)),
                Some(third().mul_add(6.0, 0.0 + 1.0))
            )
        );
    }

    #[test]
    fn float_to_int_conversion() {
        assert_eq!(arith::to_int(2.9), 2);
        assert_eq!(arith::to_int(-2.9), -2);
        assert_eq!(arith::to_int(2_147_483_520.0), 2_147_483_520);
        assert_eq!(arith::to_int(-2_147_483_648.0), i32::MIN);
        // arm64 saturates and maps NaN to 0; x86-64 answers i32::MIN for NaN and out-of-range
        // values.
        if ARCH == Arch::Arm64 {
            assert_eq!(arith::to_int(NAN), 0);
            assert_eq!(arith::to_int(3.0e9), i32::MAX);
            assert_eq!(arith::to_int(-3.0e9), i32::MIN);
        }
        if ARCH == Arch::X86_64 {
            assert_eq!(arith::to_int(NAN), i32::MIN);
            assert_eq!(arith::to_int(3.0e9), i32::MIN);
            assert_eq!(arith::to_int(-3.0e9), i32::MIN);
        }
    }

    #[test]
    fn array_index_wraps_and_never_goes_out_of_range() {
        assert_eq!(numeric::array_index(0.0, 3), Some(0));
        assert_eq!(numeric::array_index(2.9, 3), Some(2));
        assert_eq!(numeric::array_index(3.0, 3), Some(0));
        assert_eq!(numeric::array_index(7.5, 3), Some(1));
        assert_eq!(numeric::array_index(-1.0, 3), Some(0));
        assert_eq!(numeric::array_index(NAN, 3), Some(0));
        assert_eq!(numeric::array_index(f32::NEG_INFINITY, 3), Some(0));
        // An empty array has no element to select, whatever the index.
        for index in [
            0.0,
            2.0,
            -1.0,
            NAN,
            f32::INFINITY,
            f32::NEG_INFINITY,
            1.0e30,
        ] {
            assert_eq!(numeric::array_index(index, 0), None);
        }
        assert_eq!(numeric::array_index(1.0e30, 1), Some(0));
        assert_eq!(
            numeric::array_index(f32::INFINITY, 3),
            per_arch(Some(0), Some((i32::MAX as usize) % 3))
        );
    }

    #[test]
    fn architecture_primitives() {
        let x = third();
        assert_eq!(arith::mul_add(x, 3.0, -1.0), per_arch(0.0, 2.980_232_2e-8));
        assert_eq!(arith::mul_sub(x, 3.0, 1.0), per_arch(0.0, -2.980_232_2e-8));
        if ARCH == Arch::Arm64 {
            assert_eq!(arith::neg_mul_add(x, 3.0, -1.0), -2.980_232_2e-8);
            assert_eq!(arith::neg_mul_sub(x, 3.0, 1.0), 2.980_232_2e-8);
        }
        // neg_mul_add is −(a·b) − c, not −(a·b + c): an exact cancellation gives +0.
        assert_eq!(
            arith::neg_mul_add(2.0, 3.0, -6.0).to_bits(),
            0.0_f32.to_bits()
        );
        assert_eq!(
            arith::neg_mul_add(-2.0, 0.0, -0.0).to_bits(),
            0.0_f32.to_bits()
        );
        // The second half of the in-out quad curve ends in −(a·b) − c: (-0, 0, t >= 1.5) is +0.
        if ARCH == Arch::Arm64 {
            assert_eq!(
                math::ease_in_out_quad(-0.0, 0.0, 16.5, ID).to_bits(),
                0.0_f32.to_bits()
            );
        }
        assert_eq!(arith::mul_add(2.0, 3.0, 1.0), 7.0);
        assert_eq!(arith::mul_sub(2.0, 3.0, 1.0), -5.0);
        assert_eq!(arith::neg_mul_add(2.0, 3.0, 1.0), -7.0);
        assert_eq!(arith::neg_mul_sub(2.0, 3.0, 1.0), 5.0);
        assert_eq!(arith::max(1.0, 2.0), 2.0);
        assert_eq!(arith::min(1.0, 2.0), 1.0);
    }
}

mod nan_signs {
    //! Which NaN comes out where two NaNs meet, and the NaN of `math.ln`. The sign is what
    //! `math.copy_sign` shows of a NaN.

    use crate::common::per_arch;
    use molangx::compile::{CompileOptions, compile};
    use molangx::version::MolangVersion;
    use molangx::vm::{NoHostEnv, Value, VariableName};

    /// The default NaN with its sign bit set.
    const NEG: u32 = 0xffc0_0000;
    /// The same NaN with its sign bit clear.
    const POS: u32 = 0x7fc0_0000;

    /// The bits `source` evaluates to with `v.n` = `NEG`, `v.p` = `POS`, `v.h` = 0.5.
    fn bits(source: &str) -> u32 {
        let compiled = compile(source, &CompileOptions::server(MolangVersion::LATEST));
        assert_eq!(
            compiled.failure(),
            None,
            "{source:?}: {:?}",
            compiled.diagnostics()
        );
        let mut env = NoHostEnv::new();
        for (name, value) in [
            ("n", f32::from_bits(NEG)),
            ("p", f32::from_bits(POS)),
            ("h", 0.5),
            ("one", 1.0),
        ] {
            env.variables
                .set(VariableName::new(name), Value::Float(value));
        }
        compiled
            .expr()
            .cloned()
            .expect("an expression")
            .eval_f32(&mut env.cx())
            .to_bits()
    }

    /// In `+`, `-` and `*` of two quiet NaNs the left one wins, at run time and in the constant
    /// fold, and in a longer sum or product the leftmost. `math.sqrt(-1)` is `NEG` on x86-64 and
    /// `POS` on arm64.
    #[test]
    fn the_left_quiet_nan_wins_in_sums_and_products() {
        for (source, expected) in [
            ("v.n + v.p", NEG),
            ("v.p + v.n", POS),
            ("v.n - v.p", NEG),
            ("v.p - v.n", POS),
            ("v.n * v.p", NEG),
            ("v.p * v.n", POS),
            ("v.one + v.n + v.p", NEG),
            ("v.one + v.p + v.n", POS),
            ("v.p * v.one * v.n", POS),
            ("(v.n + v.p) * 2 + 1", NEG),
            ("-(v.p + v.n)", POS),
            (
                "math.sqrt(-1) + math.copy_sign(math.sqrt(-1), 1)",
                per_arch(NEG, POS),
            ),
            ("math.copy_sign(math.sqrt(-1), 1) + math.sqrt(-1)", POS),
            (
                "math.sqrt(-1) * math.copy_sign(math.sqrt(-1), 1)",
                per_arch(NEG, POS),
            ),
            ("math.copy_sign(math.sqrt(-1), 1) * math.sqrt(-1)", POS),
            (
                "1 + math.sqrt(-1) + math.copy_sign(math.sqrt(-1), 1)",
                per_arch(NEG, POS),
            ),
        ] {
            assert_eq!(bits(source), expected, "{source}");
        }
    }

    /// The NaN an interpolation or easing returns depends on its formula's operand order.
    #[test]
    fn interpolation_and_easing_operand_order() {
        for (source, expected) in [
            ("math.lerp(v.h, v.n, v.p)", per_arch(NEG, POS)),
            ("math.lerp(v.n, v.h, v.p)", NEG),
            ("math.lerp(v.p, v.h, v.n)", POS),
            ("math.lerprotate(v.h, v.p, v.n)", per_arch(POS, NEG)),
            ("math.ease_in_quad(v.n, v.p, v.h)", per_arch(POS, NEG)),
            ("math.ease_out_cubic(v.n, v.h, v.p)", per_arch(POS, NEG)),
            ("math.ease_in_out_sine(v.n, v.p, v.h)", POS),
            ("math.ease_in_out_circ(v.h, v.n, v.p)", NEG),
            ("math.ease_in_back(v.n, v.h, v.p)", per_arch(POS, NEG)),
            ("math.ease_out_quad(v.n, v.p, v.h)", NEG),
            ("math.ease_out_elastic(v.n, v.p, v.h)", per_arch(NEG, POS)),
            ("math.ease_in_out_bounce(v.n, v.p, v.h)", NEG),
            ("math.ease_in_out_elastic(v.n, v.p, 0.25)", NEG),
            (
                "math.ease_in_out_elastic(v.n, v.p, 0.75)",
                per_arch(NEG, POS),
            ),
        ] {
            assert_eq!(bits(source), expected, "{source}");
        }
    }

    /// Run time and folded: `math.ln` of a negative number is `POS` on x86-64 and `NEG` on arm64;
    /// a NaN argument comes back unchanged.
    #[test]
    fn the_logarithm_of_a_negative_number_has_the_architecture_s_nan() {
        for source in [
            "math.ln(-1)",
            "math.ln(math.ln(0))",
            "math.ln(-1e-30)",
            "math.ln(-v.h)",
        ] {
            assert_eq!(bits(source), per_arch(POS, NEG), "{source}");
        }
        assert_eq!(bits("math.ln(v.n)"), NEG);
        assert_eq!(bits("math.ln(v.p)"), POS);
        assert_eq!(f32::from_bits(bits("math.ln(0)")), f32::NEG_INFINITY);
        assert_eq!(
            f32::from_bits(bits("math.ln(math.copy_sign(0, -1))")),
            f32::NEG_INFINITY
        );
    }
}
