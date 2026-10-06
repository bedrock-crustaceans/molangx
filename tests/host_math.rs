//! Host math functions: a `MathCatalog` in the compile options, its functions called like the
//! standard ones.

#![cfg(all(feature = "compiler", feature = "stdlib"))]

use molangx::catalog::{Arity, MathCatalog, MathDecl, MathError};
use molangx::compile::{CompileFailure, CompileOptions, Compiled, Expr, ProgramFlags, compile};
use molangx::diag::{DiagCode, Severity};
use molangx::ops::{ExpressionOp, OpSet};
use molangx::rng::sample;
use molangx::version::MolangVersion;

/// `math.twice` (`2·x`), `math.tag` (`x + 0.25`), `math.neg_zero` (−0), `math.pair` (`a − b`),
/// `math.span` (2 or 3 arguments, their sum), `math.sinh` (`x + 0.5`), `math.weigh3` (`a − 2b +
/// 4c`), `math.weigh8` (`Σ aᵢ·2ⁱ`) and the volatile `math.draw` (`x` plus one draw).
fn catalog() -> MathCatalog {
    let one = Arity::exactly(1);
    MathCatalog::new([
        MathDecl::pure("math.twice", one, |a| a[0] * 2.0).unwrap(),
        MathDecl::pure("math.tag", one, |a| a[0] + 0.25).unwrap(),
        MathDecl::pure("math.neg_zero", one, |_| -0.0).unwrap(),
        MathDecl::pure("math.pair", Arity::exactly(2), |a| a[0] - a[1]).unwrap(),
        MathDecl::pure("math.span", Arity::between(2, 3), |a| a.iter().sum()).unwrap(),
        MathDecl::pure("math.sinh", one, |a| a[0] + 0.5).unwrap(),
        MathDecl::pure("math.weigh3", Arity::exactly(3), |a| {
            a[0] - 2.0 * a[1] + 4.0 * a[2]
        })
        .unwrap(),
        MathDecl::pure("math.weigh8", Arity::exactly(8), |a| {
            a.iter().rev().fold(0.0, |sum, x| sum * 2.0 + x)
        })
        .unwrap(),
        MathDecl::volatile("math.draw", one, |rng, a| a[0] + sample(rng)).unwrap(),
    ])
    .unwrap()
}

fn options(math: &MathCatalog) -> CompileOptions {
    CompileOptions {
        math: Some(math.clone()),
        ..CompileOptions::server(MolangVersion::LATEST)
    }
}

/// The diagnostics of a compile as `(code, severity, text)`.
fn diagnostics(compiled: &Compiled) -> Vec<(DiagCode, Severity, String)> {
    compiled
        .diagnostics()
        .iter()
        .map(|d| (d.code(), d.severity(), d.to_string()))
        .collect()
}

#[test]
fn a_pure_function_of_constants_folds_to_the_bits_it_returns() {
    let math = catalog();
    let options = options(&math);
    let constant = |src: &str| {
        compile(src, &options)
            .expr()
            .and_then(Expr::as_constant)
            .map(f32::to_bits)
    };
    assert_eq!(constant("math.tag(1)"), Some(1.25_f32.to_bits()));
    assert_eq!(constant("math.neg_zero(1)"), Some((-0.0_f32).to_bits()));
    assert_eq!(
        constant("math.pair(1 + 2, math.twice(4))"),
        Some((-5.0_f32).to_bits())
    );
    assert_eq!(constant("math.twice(3) * 2 + 1"), Some(13.0_f32.to_bits()));
}

/// `a − 2b + 4c` and `Σ aᵢ·2ⁱ` tell every order of their arguments apart.
#[test]
fn folded_arguments_arrive_in_source_order() {
    let math = catalog();
    let constant = |src: &str| {
        compile(src, &options(&math))
            .expr()
            .and_then(Expr::as_constant)
    };
    assert_eq!(constant("math.weigh3(1, 2, 3)"), Some(9.0));
    assert_eq!(constant("math.weigh3(3, 2, 1)"), Some(3.0));
    assert_eq!(
        constant("math.weigh8(1, 2, 3, 4, 5, 6, 7, 8)"),
        Some(1793.0)
    );
    assert_eq!(constant("math.weigh8(8, 7, 6, 5, 4, 3, 2, 1)"), Some(502.0));
}

#[test]
fn a_volatile_function_is_never_folded() {
    let math = catalog();
    let compiled = compile("math.draw(1)", &options(&math));
    let expr = compiled.expr().expect("compiles");
    assert_eq!(expr.as_constant(), None);
    assert!(expr.has_side_effects(true) && !expr.has_side_effects(false));
    assert!(
        expr.flags()
            .contains(ProgramFlags::USES_RANDOM.union(ProgramFlags::USES_RANDOM_OP))
    );
    let pure = compile("math.twice(v.x)", &options(&math))
        .expr()
        .cloned()
        .expect("compiles");
    assert!(!pure.has_side_effects(true) && !pure.flags().contains(ProgramFlags::USES_RANDOM));
}

#[test]
fn the_argument_count_is_checked_against_the_declaration() {
    let math = catalog();
    for (src, text) in [
        ("math.twice(1, 2)", "math.twice takes 1 argument, 2 given"),
        ("math.pair(1, 2, 3)", "math.pair takes 2 arguments, 3 given"),
        ("math.pair(v.x)", "math.pair takes 2 arguments, 1 given"),
        ("math.span(1)", "math.span takes 2 to 3 arguments, 1 given"),
        (
            "math.span(1, 2, 3, 4)",
            "math.span takes 2 to 3 arguments, 4 given",
        ),
        ("1 + math.draw(1, 2)", "math.draw takes 1 argument, 2 given"),
    ] {
        let compiled = compile(src, &options(&math));
        assert_eq!(compiled.failure(), Some(CompileFailure::Rejected), "{src}");
        assert_eq!(
            diagnostics(&compiled),
            [(DiagCode::StatementForm, Severity::Error, text.to_owned())],
            "{src}"
        );
        assert_eq!(
            compiled.expr_or_zero().and_then(Expr::as_constant),
            Some(0.0),
            "{src}"
        );
    }
    for src in ["math.span(1, 2)", "math.span(1, 2, 3)"] {
        assert_eq!(compile(src, &options(&math)).failure(), None, "{src}");
    }
}

#[test]
fn an_empty_catalogue_compiles_like_none() {
    let empty = MathCatalog::new([]).unwrap();
    let without = CompileOptions::server(MolangVersion::LATEST);
    for src in ["math.sin(90) + v.x", "math.nope(1)", "math.twice(1)"] {
        let (with, plain) = (compile(src, &options(&empty)), compile(src, &without));
        assert_eq!(
            (with.failure(), with.diagnostics()),
            (plain.failure(), plain.diagnostics()),
            "{src}"
        );
    }
}

#[test]
fn an_undeclared_name_fails_as_without_a_catalogue() {
    let math = catalog();
    let without = CompileOptions::server(MolangVersion::LATEST);
    for src in ["math.nope(1)", "math.twicex(1)", "1 + math.nope"] {
        let with = compile(src, &options(&math));
        assert_eq!(with.failure(), Some(CompileFailure::Rejected), "{src}");
        assert_eq!(
            with.diagnostics(),
            compile(src, &without).diagnostics(),
            "{src}"
        );
    }
    // Without a catalogue a declared name is as unknown as any other.
    let unknown = compile("math.twice(1)", &without);
    assert_eq!(unknown.failure(), Some(CompileFailure::Rejected));
    assert_eq!(
        unknown.diagnostics()[0].to_string(),
        "Error: unknown token: math.twice(1)"
    );
}

#[test]
fn a_declared_name_with_a_built_in_prefix_is_the_host_function_and_the_built_in_stays() {
    let math = catalog();
    let constant = |src: &str| {
        compile(src, &options(&math))
            .expr()
            .and_then(Expr::as_constant)
    };
    assert_eq!(constant("math.sinh(1)"), Some(1.5));
    assert_eq!(constant("math.sin(90)"), Some(1.0));
}

/// `math.sin` (`x + 100`), a pure `math.random` (`a + b`) and a volatile `math.random_integer`
/// (`1000` plus a draw) in place of the standard functions.
fn overrides() -> MathCatalog {
    MathCatalog::new([
        MathDecl::pure("math.sin", Arity::exactly(1), |a| a[0] + 100.0).unwrap(),
        MathDecl::pure("math.random", Arity::exactly(2), |a| a[0] + a[1]).unwrap(),
        MathDecl::volatile("math.random_integer", Arity::exactly(2), |rng, a| {
            1000.0 + a[0] + sample(rng)
        })
        .unwrap(),
    ])
    .unwrap()
}

#[test]
fn math_pi_is_a_constant_and_cannot_be_declared() {
    assert_eq!(
        MathDecl::pure("math.pi", Arity::exactly(1), |a| a[0]).err(),
        Some(MathError::Constant("math.pi".into()))
    );
    assert_eq!(
        MathError::Constant("math.pi".into()).to_string(),
        "math.pi is a constant, not a function, and cannot be declared"
    );
}

/// An overriding function replaces the standard function wherever its full name appears; without
/// the catalogue the standard function is back.
#[test]
fn a_host_function_overrides_a_built_in_one_of_its_name() {
    let math = overrides();
    let constant = |src: &str, options: &CompileOptions| {
        compile(src, options).expr().and_then(Expr::as_constant)
    };
    assert_eq!(constant("math.sin(90)", &options(&math)), Some(190.0));
    assert_eq!(
        constant("MATH.SIN(90)", &options(&math)),
        Some(190.0),
        "upper case"
    );
    assert_eq!(constant("Math.Sin(90) * 2", &options(&math)), Some(380.0));
    assert_eq!(
        constant("math.random(1, 2)", &options(&math)),
        Some(3.0),
        "a pure override folds"
    );
    let without = CompileOptions::server(MolangVersion::LATEST);
    assert_eq!(constant("math.sin(90)", &without), Some(1.0));
    assert_eq!(constant("math.random(1, 2)", &without), None);
    // `math.sin1` is not the full name: the standard `math.sin` followed by `1`.
    for src in ["math.sin1(90)", "math.sin1"] {
        assert_eq!(
            texts(src, &options(&math)),
            ["Error: Sine 'math.sin' operator not followed by parenthesis section"],
            "{src}"
        );
    }
    // The argument count is the declaration's.
    assert_eq!(
        texts("math.sin(1, 2)", &options(&math)),
        ["math.sin takes 1 argument, 2 given"]
    );
}

/// To `OpSet`, an override is a host call: a pure `math.random` passes where the standard function
/// is forbidden, a volatile `math.random_integer` does not.
#[test]
fn the_allowed_operations_treat_an_override_as_a_host_call() {
    let math = overrides();
    let block = CompileOptions {
        allowed_ops: OpSet::all().without_assignments_or_random(),
        ..options(&math)
    };
    assert_eq!(compile("math.random(v.x, 2)", &block).failure(), None);
    assert_eq!(
        texts("math.random_integer(v.x, 2)", &block),
        [
            "Expression uses operation Volatile Host Math Function 'math.random_integer' which is not allowed in this context"
        ]
    );
    let without_math = CompileOptions {
        math: None,
        ..block
    };
    assert_eq!(
        compile("math.random(v.x, 2)", &without_math).failure(),
        Some(CompileFailure::Rejected)
    );
    let no_host = CompileOptions {
        allowed_ops: OpSet::all().without(ExpressionOp::HostMath),
        ..options(&math)
    };
    assert_eq!(
        texts("math.sin(v.x)", &no_host),
        [
            "Expression uses operation Host Math Function 'math.sin' which is not allowed in this context"
        ]
    );
}

#[cfg(feature = "fuzz")]
#[test]
fn the_tree_notation_and_the_disassembly_name_an_override() {
    let math = overrides();
    let compiled = compile("math.sin(v.x) * 2", &options(&math));
    assert_eq!(
        compiled.tree_notation(9).as_deref(),
        Some("[(math.sin v.x)*2+0]")
    );
    let listing = compiled.expr().expect("compiles").disassemble();
    assert!(listing.contains("host-math math.sin args 1"), "{listing}");
    let plain = compile(
        "math.sin(v.x) * 2",
        &CompileOptions::server(MolangVersion::LATEST),
    );
    assert_eq!(plain.tree_notation(9).as_deref(), Some("[(Sin v.x)*2+0]"));
}

#[test]
fn upper_case_source_resolves() {
    let math = catalog();
    assert_eq!(
        compile("MATH.TWICE(1)", &options(&math))
            .expr()
            .and_then(Expr::as_constant),
        Some(2.0)
    );
    assert_eq!(
        compile("Math.Span(1, 2)", &options(&math))
            .expr()
            .and_then(Expr::as_constant),
        Some(3.0)
    );
}

/// The texts of a compile's diagnostics.
fn texts(src: &str, options: &CompileOptions) -> Vec<String> {
    compile(src, options)
        .diagnostics()
        .iter()
        .map(ToString::to_string)
        .collect()
}

/// A denied host op is reported once per function it calls, named like a standard function.
#[test]
fn the_allowed_operations_cover_host_functions() {
    let math = catalog();
    let not_allowed = |name: &str| {
        format!("Expression uses operation {name} which is not allowed in this context")
    };
    let no_host = CompileOptions {
        allowed_ops: OpSet::all().without(ExpressionOp::HostMath),
        ..options(&math)
    };
    let compiled = compile("math.twice(v.x)", &no_host);
    assert_eq!(compiled.failure(), Some(CompileFailure::Rejected));
    assert_eq!(
        texts("math.twice(v.x)", &no_host),
        [not_allowed("Host Math Function 'math.twice'")]
    );
    assert_eq!(
        texts(
            "math.twice(v.x) + math.pair(1, v.x) + math.twice(2) + math.draw(1)",
            &no_host
        ),
        [
            not_allowed("Host Math Function 'math.twice'"),
            not_allowed("Host Math Function 'math.pair'")
        ]
    );
    assert_eq!(compile("math.draw(v.x)", &no_host).failure(), None);

    let block = CompileOptions {
        allowed_ops: OpSet::all().without_assignments_or_random(),
        ..options(&math)
    };
    let volatile = compile("math.draw(v.x)", &block);
    assert_eq!(volatile.failure(), Some(CompileFailure::Rejected));
    assert_eq!(
        texts("math.draw(v.x)", &block),
        [not_allowed("Volatile Host Math Function 'math.draw'")]
    );
    assert_eq!(compile("math.twice(v.x)", &block).failure(), None);
}

/// Every message about a host call names the function, the way `Random 'math.random'` is named.
#[test]
fn messages_name_the_host_function() {
    let math = catalog();
    let options = options(&math);
    for (src, text) in [
        (
            "math.twice('a')",
            "'Host Math Function 'math.twice'' expression cannot take a 'String '''' argument. It only supports numerical arguments.",
        ),
        (
            "math.draw('a')",
            "'Volatile Host Math Function 'math.draw'' expression cannot take a 'String '''' argument. It only supports numerical arguments.",
        ),
        (
            "math.twice(q.combine_entities)",
            "Host Math Function 'math.twice' expressions may only contain query functions that return numbers",
        ),
        (
            "math.twice()",
            "Host Math Function 'math.twice' operator (math, query, loop, etc) with empty parameter list should have failed to parse",
        ),
        (
            "math.twice",
            "Error: Host Math Function 'math.twice' operator at end of expression without a parenthesis section",
        ),
        (
            "math.twice(1) = 3;",
            "Error: assignment to non-variable not allowed. Expression is trying to assign to a: Host Math Function 'math.twice'",
        ),
    ] {
        assert_eq!(texts(src, &options), [text], "{src}");
    }
}

#[cfg(feature = "fuzz")]
#[test]
fn the_disassembly_and_the_tree_notation_name_the_function() {
    let math = catalog();
    let compiled = compile("math.span(v.x, 2) * 2 + 1", &options(&math));
    assert_eq!(
        compiled.tree_notation(9).as_deref(),
        Some("[(math.span v.x 2)*2+1]")
    );
    let listing = compiled.expr().expect("compiles").disassemble();
    assert!(
        listing.contains("host-math math.span args 2 *2+1"),
        "{listing}"
    );
    let volatile = compile("math.draw(1)", &options(&math));
    assert!(
        volatile
            .expr()
            .expect("compiles")
            .disassemble()
            .contains("host-math math.draw args 1")
    );
    assert!(format!("{:?}", volatile.expr().expect("compiles")).contains("(math.draw 1)"));
}

#[cfg(feature = "vm")]
mod evaluation {
    use super::*;
    use molangx::numeric::PostOp;
    use molangx::rng::{Xorshift128, rand_core::SeedableRng, sample};
    use molangx::stdlib::math;
    use molangx::vm::{NoHostEnv, Value, VariableName};
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn expr(src: &str, math: &MathCatalog) -> Expr {
        let compiled = compile(src, &options(math));
        compiled
            .expr()
            .cloned()
            .unwrap_or_else(|| panic!("{src}: {:?}", diagnostics(&compiled)))
    }

    fn env_with_x(x: f32) -> NoHostEnv {
        let mut env = NoHostEnv::new();
        env.variables.set(VariableName::new("x"), Value::Float(x));
        env
    }

    #[test]
    fn a_pure_function_of_a_run_time_argument_is_called_with_the_post_op_applied() {
        let math = catalog();
        let call = expr("math.twice(v.x) * 2 + 1", &math);
        assert_eq!(call.as_constant(), None);
        assert!(call.flags().contains(ProgramFlags::FLOAT_ONLY));
        let mut env = env_with_x(3.0);
        assert_eq!(call.eval_f32(&mut env.cx()), 13.0);
        assert_eq!(call.eval(&mut env.cx()), Value::Float(13.0));
        // Arguments arrive in source order.
        assert_eq!(
            expr("math.pair(v.x, 1)", &math).eval_f32(&mut env.cx()),
            2.0
        );
        assert_eq!(
            expr("math.span(v.x, 1, 10)", &math).eval_f32(&mut env.cx()),
            14.0
        );
        assert_eq!(
            expr("math.weigh3(v.x, 1, 10)", &math).eval_f32(&mut env.cx()),
            41.0
        );
        // The plain form returns the function's bits untouched.
        assert_eq!(
            expr("math.neg_zero(v.x)", &math)
                .eval_f32(&mut env.cx())
                .to_bits(),
            (-0.0_f32).to_bits()
        );
    }

    /// Run-time and constant arguments of three and eight arguments, on the float-only loop and
    /// on the general one (a string elsewhere in the program).
    #[test]
    fn run_time_arguments_arrive_in_source_order() {
        let math = catalog();
        let mut env = env_with_x(1.0);
        env.variables.set(VariableName::new("y"), 3.0);
        for (src, value) in [
            ("math.weigh3(v.x, 2, v.y)", 9.0),
            ("math.weigh3(v.y, v.x * 2, v.x)", 3.0),
            ("math.weigh3(3, v.x + 1, 1)", 3.0),
            ("math.weigh8(v.x, 2, v.y, 4, 5, 6, 7, v.x * 8)", 1793.0),
            ("math.weigh8(8, 7, 6, 5, 4, v.y, 2, v.x)", 502.0),
        ] {
            let call = expr(src, &math);
            assert!(call.flags().contains(ProgramFlags::FLOAT_ONLY), "{src}");
            assert_eq!(call.eval_f32(&mut env.cx()), value, "{src}");
            let general = expr(&format!("t.s = 'a'; return {src};"), &math);
            assert!(!general.flags().contains(ProgramFlags::FLOAT_ONLY), "{src}");
            assert_eq!(general.eval(&mut env.cx()), Value::Float(value), "{src}");
        }
    }

    #[test]
    fn volatile_draws_interleave_with_math_random_in_evaluation_order() {
        let math = catalog();
        let call = expr(
            "v.a = math.random(0, 1); v.b = math.draw(10); v.c = math.random(0, 1); return v.b;",
            &math,
        );
        let mut env = NoHostEnv::new();
        env.rng = Xorshift128::seed_from_u64(7);
        let mut samples = Xorshift128::seed_from_u64(7);
        let (first, second, third) = (
            sample(&mut samples),
            sample(&mut samples),
            sample(&mut samples),
        );
        assert_eq!(call.eval_f32(&mut env.cx()), 10.0 + second);
        let read = |name: &str| {
            env.variables
                .get(VariableName::new(name))
                .map(Value::as_f32)
        };
        let random = |sample| math::random(0.0, 1.0, sample, PostOp::IDENTITY);
        assert_eq!(
            (read("a"), read("b"), read("c")),
            (
                Some(random(first)),
                Some(10.0 + second),
                Some(random(third))
            )
        );
        // Each evaluation draws anew.
        assert_ne!(call.eval_f32(&mut env.cx()), 10.0 + second);
    }

    #[test]
    fn an_override_runs_at_evaluation_and_the_built_in_without_the_catalogue() {
        let math = overrides();
        let mut env = env_with_x(90.0);
        assert_eq!(
            expr("math.sin(v.x) * 2", &math).eval_f32(&mut env.cx()),
            380.0
        );
        assert_eq!(
            expr("math.random(v.x, 1)", &math).eval_f32(&mut env.cx()),
            91.0
        );
        env.rng = Xorshift128::seed_from_u64(3);
        let draw = sample(&mut Xorshift128::seed_from_u64(3));
        assert_eq!(
            expr("math.random_integer(v.x, 1)", &math).eval_f32(&mut env.cx()),
            1090.0 + draw
        );
        let standard = compile(
            "math.sin(v.x) * 2",
            &CompileOptions::server(MolangVersion::LATEST),
        )
        .expr()
        .cloned()
        .expect("compiles");
        assert_eq!(standard.eval_f32(&mut env.cx()), 2.0);
    }

    /// A pure function counting its calls.
    fn counted(name: &str, calls: &Arc<AtomicUsize>) -> MathDecl {
        let calls = Arc::clone(calls);
        MathDecl::pure(name, Arity::exactly(1), move |a| {
            calls.fetch_add(1, Ordering::Relaxed);
            a[0] * 2.0
        })
        .unwrap()
    }

    #[test]
    fn equal_pure_calls_merge_and_volatile_or_different_ones_do_not() {
        let (f_calls, g_calls, h_calls) = (
            Arc::default(),
            Arc::default(),
            Arc::<AtomicUsize>::default(),
        );
        let h = {
            let calls = Arc::clone(&h_calls);
            MathDecl::volatile("math.h", Arity::exactly(1), move |_, a| {
                calls.fetch_add(1, Ordering::Relaxed);
                a[0] * 2.0
            })
            .unwrap()
        };
        let math = MathCatalog::new([counted("math.f", &f_calls), counted("math.g", &g_calls), h])
            .unwrap();
        let mut env = env_with_x(3.0);
        let calls = |counter: &Arc<AtomicUsize>| counter.swap(0, Ordering::Relaxed);

        assert_eq!(
            expr("math.f(v.x) + math.f(v.x)", &math).eval_f32(&mut env.cx()),
            12.0
        );
        assert_eq!(calls(&f_calls), 1, "merged into one call");
        assert_eq!(
            expr("math.h(v.x) + math.h(v.x)", &math).eval_f32(&mut env.cx()),
            12.0
        );
        assert_eq!(calls(&h_calls), 2, "a volatile call never merges");
        assert_eq!(
            expr("math.f(v.x) + math.g(v.x)", &math).eval_f32(&mut env.cx()),
            12.0
        );
        assert_eq!(
            (calls(&f_calls), calls(&g_calls)),
            (1, 1),
            "two functions never merge"
        );
    }

    /// The smallest step budget under which `expr` completes without a message.
    fn steps_needed(expr: &Expr) -> u64 {
        (0..100)
            .find(|&steps| {
                let mut env = env_with_x(0.5);
                env.variables.set(VariableName::new("y"), Value::Float(1.0));
                env.limits.total_steps = Some(steps);
                expr.eval(&mut env.cx());
                env.sink.is_empty()
            })
            .expect("completes within 100 steps")
    }

    #[test]
    fn a_host_call_costs_one_step_like_a_built_in_function() {
        let math = catalog();
        let standard = |src: &str| {
            compile(src, &CompileOptions::server(MolangVersion::LATEST))
                .expr()
                .cloned()
                .expect("compiles")
        };
        assert_eq!(
            steps_needed(&expr("math.twice(v.x)", &math)),
            steps_needed(&standard("math.abs(v.x)"))
        );
        assert_eq!(
            steps_needed(&expr("math.span(v.x, v.y, v.x)", &math)),
            steps_needed(&standard("math.clamp(v.x, v.y, v.x)"))
        );
        assert_eq!(
            steps_needed(&expr("math.draw(v.x)", &math)),
            steps_needed(&standard("math.abs(v.x)"))
        );
    }
}

#[cfg(feature = "cache")]
#[test]
fn the_compile_cache_keys_the_catalogue_by_identity() {
    use molangx::cache::CompileCache;
    use std::sync::Arc;
    let (math, same_names) = (catalog(), catalog());
    let cache = CompileCache::new();
    let first = cache.compile("math.twice(v.x)", &options(&math));
    assert!(Arc::ptr_eq(
        &first,
        &cache.compile("math.twice(v.x)", &options(&math))
    ));
    let clone = math.clone();
    assert!(Arc::ptr_eq(
        &first,
        &cache.compile("math.twice(v.x)", &options(&clone))
    ));
    let other = cache.compile("math.twice(v.x)", &options(&same_names));
    assert!(!Arc::ptr_eq(&first, &other));
    assert_eq!(other.expr().and_then(Expr::math), Some(&same_names));
    assert_eq!((cache.len(), cache.misses()), (2, 2));
}

#[cfg(feature = "cache")]
#[test]
fn the_compile_cache_keeps_an_override_and_the_built_in_apart() {
    use molangx::cache::CompileCache;
    let math = overrides();
    let cache = CompileCache::new();
    let overridden = cache.compile("math.sin(90)", &options(&math));
    let standard = cache.compile(
        "math.sin(90)",
        &CompileOptions::server(MolangVersion::LATEST),
    );
    assert_eq!(overridden.expr().and_then(Expr::as_constant), Some(190.0));
    assert_eq!(standard.expr().and_then(Expr::as_constant), Some(1.0));
    assert_eq!(
        cache
            .compile("math.sin(90)", &options(&math))
            .expr()
            .and_then(Expr::as_constant),
        Some(190.0)
    );
    assert_eq!((cache.len(), cache.misses()), (2, 2));
}
