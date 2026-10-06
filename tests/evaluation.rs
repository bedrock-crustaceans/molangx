//! Compiling and evaluating expressions: control flow, variables, the evaluation model and the
//! result kinds.

#![cfg(all(feature = "vm", feature = "stdlib"))]

mod common;

mod control_flow {
    use std::sync::Arc;

    use crate::common::{
        compile_support::{server_at, server_expr, server_expr_at},
        eval_with_samples,
        host::{Actor, Env},
        per_arch,
    };
    use molangx::compile::{CompileFailure, CompileOptions, compile};
    use molangx::rng::{Xorshift128, sample};
    use molangx::version::MolangVersion;
    use molangx::vm::{CollectSink, EvalLimits, NoHostEnv, Value, VariableName};

    fn eval(source: &str) -> f32 {
        server_expr(source).eval_f32(&mut NoHostEnv::new().cx())
    }

    /// The post-op of a non-constant `?:` applies to either branch; `c ? a` is 0 when `c` is false.
    #[test]
    fn conditionals() {
        assert_eq!(eval("v.c = 1; return v.c ? 10 : 20;"), 10.0);
        assert_eq!(eval("v.c = 0; return v.c ? 10 : 20;"), 20.0);
        assert_eq!(eval("v.c = 1; return (v.c ? 1 : 2) * -2 + 1;"), -1.0);
        assert_eq!(eval("v.c = 0; return (v.c ? 1 : 2) * -2 + 1;"), -3.0);
        assert_eq!(eval("v.c = 0; return v.c ? 5;"), 0.0);
        assert_eq!(
            eval("v.c = 1; v.c ? {v.x = 1;} : {v.x = 2;}; return v.x;"),
            1.0
        );
        // NaN is truthy, -0 is not.
        assert_eq!(eval("v.c = math.sqrt(-1); return v.c ? 1 : 2;"), 1.0);
        assert_eq!(eval("v.c = -0.0; return v.c ? 1 : 2;"), 2.0);
    }

    #[test]
    fn logic_short_circuits() {
        assert_eq!(eval("v.a = 5; return v.a && 3;"), 1.0);
        assert_eq!(eval("v.a = 0; return v.a || v.a;"), 0.0);
        assert_eq!(eval("v.a = 2; v.b = 0; return v.a && v.a && v.b;"), 0.0);
        assert_eq!(eval("v.a = 0; v.b = 1; return v.a || v.a || v.b;"), 1.0);
        // The draws after the deciding operand never happen.
        assert_eq!(
            eval_with_samples("v.z = 0; return v.z && math.random(0, 1);", &[0.5]),
            (0.0, 0)
        );
        assert_eq!(
            eval_with_samples("v.z = 1; return v.z || math.random(0, 1);", &[0.5]),
            (1.0, 0)
        );
        assert_eq!(
            eval_with_samples("v.z = 1; return v.z && math.random(0, 1);", &[0.5]),
            (1.0, 1)
        );
        // A missing operand after the deciding one is never read.
        assert_eq!(eval("v.z = 0; return v.z && v.never_set;"), 0.0);
        // Post-ops select the precomputed constants.
        assert_eq!(eval("v.a = 1; return (v.a && v.a) * 3 + 1;"), 4.0);
        assert_eq!(eval("v.a = 0; return (v.a && v.a) * 3 + 1;"), 1.0);
    }

    /// A statement list is 0 unless a `return` runs; a `return` anywhere ends the whole expression.
    #[test]
    fn statements_and_return() {
        assert_eq!(eval("v.x = 1; 2;"), 0.0);
        assert_eq!(eval("1+1;"), 0.0);
        assert_eq!(eval("return 1+1;"), 2.0);
        assert_eq!(eval("1; return 2;"), 2.0);
        assert_eq!(eval("v.x = 1; v.x;"), 0.0);
        assert_eq!(eval("loop(3, {return 7;}); return 1;"), 7.0);
        assert_eq!(eval("v.x = 1; v.x ? {v.x ? {return 3;};}; return 4;"), 3.0);
        // `break;` / `continue;` with no loop around them end the expression with 0.
        assert_eq!(eval("v.x = 5; break;"), 0.0);
        assert_eq!(eval("v.x = 5; continue;"), 0.0);
    }

    /// Over a number or a string, the `for_each` gives its value to the one instruction after it: a
    /// read there is skipped and a store loses its value; the statements after that one run.
    #[test]
    fn for_each_over_a_non_array_skips_the_instruction_after_it() {
        // Of three stores, the last lands.
        assert_eq!(
            eval("v.f = 0; v.a = 0; for_each(v.i, v.a, 1); v.f = 1; v.f = 2; v.f = 3; return v.f;"),
            3.0
        );
        assert_eq!(
            eval("v.f = 0; v.a = 4; for_each(v.i, v.a, 1); v.f = 1; return v.f;"),
            0.0
        );
        assert_eq!(
            eval(
                "v.f = 0; v.a = 'x'; for_each(t.i, v.a, {v.z = 1;}); v.f = 1; v.f = v.f + 1; return v.f;"
            ),
            1.0
        );
        // The store writes 0, the `for_each`'s value.
        assert_eq!(
            eval("v.a = 0; v.r = 3; for_each(v.i, v.a, 1); v.r = v.r + 5; return v.r;"),
            0.0
        );
        // `t.b == 3` takes the false branch although `t.b` was set to 3.
        assert_eq!(
            eval("t.b = 3; v.a = 0; for_each(v.i, v.a, 1); return t.b == 3 ? 1 : 2;"),
            2.0
        );
        // Two in a row skip one instruction, not two.
        assert_eq!(
            eval(
                "v.f = 0; v.a = 0; for_each(v.i, v.a, 1); for_each(v.i, v.a, 1); v.f = 1; v.f = 2; return v.f;"
            ),
            2.0
        );
        // An actor array is iterated and nothing after it is skipped.
        let mut env = Env::new();
        env.world.alive.extend([1]);
        env.vars.set(
            VariableName::new("arr"),
            Value::ActorArray(Arc::new(vec![Actor::Handle(1)])),
        );
        let expr = server_expr(
            "v.n = 0; for_each(t.e, v.arr, {v.n = v.n + 1;}); v.n = v.n + 10; return v.n;",
        );
        let (mut rng, mut sink) = (Xorshift128::new(), CollectSink::new());
        assert_eq!(
            env.with_cx(&mut rng, &mut sink, |cx| expr.eval_f32(cx)),
            11.0
        );
    }

    /// `ceil(n)` iterations for finite `n > 0`, none for `n <= 0`; a NaN count never runs out on
    /// x86-64 and makes no pass on arm64.
    #[test]
    fn loops() {
        assert_eq!(
            eval("t.i = 0; loop(2.5, {t.i = t.i + 1;}); return t.i;"),
            3.0
        );
        assert_eq!(
            eval("t.i = 0; loop(0.5, {t.i = t.i + 1;}); return t.i;"),
            1.0
        );
        assert_eq!(
            eval("t.i = 0; loop(-1, {t.i = t.i + 1;}); return t.i;"),
            0.0
        );
        assert_eq!(
            eval("t.i = 0; loop(math.sqrt(-1), {t.i = t.i + 1; t.i > 8 ? break;}); return t.i;"),
            per_arch(9.0, 0.0)
        );
        assert_eq!(
            eval(
                "t.i = 0; v.n = math.sqrt(-1); loop(v.n, {t.i = t.i + 1; t.i > 8 ? break;}); return t.i;"
            ),
            per_arch(9.0, 0.0)
        );
        assert_eq!(
            eval("t.i = 0; loop(10, {t.i = t.i + 1; t.i == 4 ? break;}); return t.i;"),
            4.0
        );
        assert_eq!(
            eval(
                "t.i = 0; t.n = 0; loop(10, {t.i = t.i + 1; t.i > 3 ? continue; t.n = t.n + 1;}); return t.n * 100 + t.i;"
            ),
            310.0
        );
        // Nested: `break` leaves only the inner loop.
        assert_eq!(
            eval("t.n = 0; loop(3, {loop(5, {t.n = t.n + 1; break;});}); return t.n;"),
            3.0
        );
        assert_eq!(
            eval("t.n = 0; loop(4, {loop(5, {t.n = t.n + 1;});}); return t.n;"),
            20.0
        );
    }

    /// A `continue` inside an operand ends the loop when the operand evaluated before it is 0 or
    /// −1; a `break` inside an operand leaves the loop. The expression runs on with sound
    /// arithmetic and the interrupted assignment never stores.
    #[test]
    fn a_jump_with_an_operand_pending() {
        let eval = |source: &str| server_expr(source).eval_f32(&mut NoHostEnv::new().cx());
        // Pending `v.k *` (0): one iteration; `v.t` keeps 7.
        let x20 = "v.k = 0; v.i = 0; v.t = 7; loop(3, { v.i = v.i + 1; v.t = v.k * (v.i > 0 ? {continue;} : 0); }); \
                   return v.i * 1000 + (2 * 3 + 2) * 10 + v.t;";
        assert_eq!(eval(x20), 1087.0);
        // Pending −1.
        assert_eq!(
            eval(
                "v.k = -1; v.i = 0; loop(3, { v.i = v.i + 1; v.t = v.k * (v.i > 0 ? {continue;} : 0); }); return v.i;"
            ),
            1.0
        );
        // A pending `+` operand.
        assert_eq!(
            eval(
                "v.k = 0; v.i = 0; loop(3, { v.i = v.i + 1; v.t = v.k + (v.i > 0 ? {continue;} : 0); }); return v.i;"
            ),
            1.0
        );
        // The pending `continue`, first taken in the 2nd iteration: two iterations.
        assert_eq!(
            eval(
                "v.k = 0; v.i = 0; loop(3, { v.i = v.i + 1; v.t = v.k * (v.i > 1 ? {continue;} : 0); }); return v.i;"
            ),
            2.0
        );
        // A pending function argument.
        assert_eq!(
            eval(
                "v.k = 0; v.i = 0; loop(3, { v.i = v.i + 1; v.t = math.max(v.k, (v.i > 0 ? {continue;} : 0)); }); return v.i;"
            ),
            1.0
        );
        // Nothing pending: every iteration.
        assert_eq!(
            eval("v.i = 0; loop(3, { v.i = v.i + 1; (v.i > 0) ? {continue;} : 0; }); return v.i;"),
            3.0
        );
        // `break` inside an operand: one iteration, `v.t` keeps 7.
        let x21 = "v.k = 0; v.j = 0; v.t = 7; loop(3, { v.j = v.j + 1; v.t = v.k * (v.j > 0 ? {break;} : 0); }); \
                   return v.j * 1000 + (2 * 3 + 2) * 10 + v.t;";
        assert_eq!(eval(x21), 1087.0);
        // A `break` in a query argument ends only the argument.
        let mut env = Env::new();
        let expr = server_expr(
            "v.one = 1; v.n = 0; loop(2, { v.n = v.n + 1; v.r = q.log((v.one ? break : 1)); }); return v.n;",
        );
        let mut rng = Xorshift128::new();
        let mut sink = CollectSink::new();
        assert_eq!(
            env.with_cx(&mut rng, &mut sink, |cx| expr.eval_f32(cx)),
            2.0
        );
    }

    /// A `continue` / `break` inside an operand in an inner loop leaves the inner counter behind;
    /// the outer loop's check reads it as its own and runs away (escaped here after 8 passes).
    #[test]
    fn a_pending_jump_out_of_an_inner_loop_runs_the_outer_loop_away() {
        for jump in ["continue", "break"] {
            let source = format!(
                "v.g = 0; v.h = 0; v.i = 0; v.j = 0; v.z = 0; loop(2, {{ v.g = v.g + 1; v.g > 8 ? {{return 0;}} : 0; v.i = v.i + 1; \
                 loop(3, {{ v.h = v.h + 1; v.h > 12 ? {{return 0;}} : 0; v.j = v.j + 1; v.t = v.z * (v.j > 0 ? {{{jump};}} : 0); }}); }}); return 1;"
            );
            let mut env = NoHostEnv {
                limits: EvalLimits::NONE,
                ..NoHostEnv::new()
            };
            assert_eq!(
                server_expr(&source).eval_f32(&mut env.cx()),
                0.0,
                "{jump}: the escape ended it"
            );
            assert_eq!(
                env.variables.get(VariableName::new("g")),
                Some(&Value::Float(9.0)),
                "{jump}: the outer loop passed more than 8 times"
            );
            assert!(env.sink.is_empty());
        }
    }

    /// `for_each` iterates only a non-empty actor array, skips entries that resolve to nothing and
    /// stores each actor as its id.
    #[test]
    fn for_each() {
        let mut env = Env::new();
        env.temps = None;
        env.world.alive.extend([1, 3]);
        env.vars.set(
            VariableName::new("herd"),
            Value::ActorArray(Arc::new(vec![
                Actor::Handle(1),
                Actor::Handle(2),
                Actor::Handle(0),
                Actor::Handle(3),
            ])),
        );
        env.vars.set(VariableName::new("one"), Value::Float(1.0));
        let mut run = |source: &str| -> f32 {
            let expr = server_expr(source);
            let mut rng = Xorshift128::new();
            let mut sink = CollectSink::new();
            env.with_cx(&mut rng, &mut sink, |cx| expr.eval_f32(cx))
        };
        assert_eq!(
            run("t.n = 0; for_each(t.a, v.herd, {t.n = t.n + 1;}); return t.n;"),
            2.0
        );
        assert_eq!(
            run("t.n = 0; for_each(t.a, v.one, {t.n = t.n + 1;}); return t.n;"),
            0.0
        );
        assert_eq!(
            run("t.n = 0; for_each(v.a, v.herd, {t.n = t.n + 1; break;}); return t.n;"),
            1.0
        );
        assert_eq!(
            run("t.n = 0; for_each(v.a, v.herd, {continue; t.n = t.n + 1;}); return t.n;"),
            0.0
        );
        // The last live entry stays in the loop variable, as its id.
        run("for_each(v.last, v.herd, {t.x = 1;});");
        assert_eq!(
            env.vars.get(VariableName::new("last")),
            Some(&Value::Actor(Actor::Id(3)))
        );
        // The loop guard applies to `for_each` too.
        env.limits = EvalLimits {
            loop_iterations: Some(1),
            total_steps: None,
            ..EvalLimits::NONE
        };
        let expr = server_expr("t.n = 0; for_each(t.a, v.herd, {t.n = t.n + 1;}); return t.n;");
        let mut rng = Xorshift128::new();
        let mut sink = CollectSink::new();
        assert_eq!(
            env.with_cx(&mut rng, &mut sink, |cx| expr.eval_f32(cx)),
            1.0
        );
        assert_eq!(sink.messages.len(), 1);
    }

    #[test]
    fn version_2_loop_arithmetic_is_ignored() {
        let expr = server_expr_at(
            "v.count = 0; loop(3, {v.count = v.count + 1;}) + 1; return v.count;",
            2,
        );
        assert_eq!(expr.eval_f32(&mut NoHostEnv::new().cx()), 3.0);
    }

    #[test]
    fn expr_api() {
        let options = CompileOptions {
            keep_source: true,
            ..CompileOptions::server(MolangVersion::LATEST)
        };
        let expr = compile("v.x = 2; return v.x * 3;", &options)
            .expr()
            .cloned()
            .expect("an expression");
        assert_eq!(expr.source(), Some("v.x = 2; return v.x * 3;"));
        assert_eq!(expr.version(), MolangVersion::LATEST);
        assert_eq!(expr.as_constant(), None);
        assert!(
            expr.flags()
                .contains(molangx::compile::ProgramFlags::HAS_ASSIGNMENT)
        );
        let mut env = NoHostEnv::new();
        assert_eq!(expr.eval(&mut env.cx()), Value::Float(6.0));
        assert_eq!(expr.eval_f32(&mut env.cx()), 6.0);
        let constant = compile("1 + 2 * 3", &options)
            .expr()
            .cloned()
            .expect("an expression");
        assert_eq!(constant.as_constant(), Some(7.0));
        assert_eq!(constant.eval(&mut env.cx()), Value::Float(7.0));
        // A string constant is its hash.
        let string = compile("'abc'", &options)
            .expr_or_zero()
            .cloned()
            .expect("an expression");
        assert_eq!(string.eval(&mut env.cx()), Value::string("abc"));
        let rejected = compile("1 +", &options)
            .expr_or_zero()
            .cloned()
            .expect("the failed node");
        assert_eq!(rejected.eval(&mut env.cx()), Value::ZERO);
        assert_eq!(rejected.eval_f32(&mut env.cx()), 0.0);
    }

    #[test]
    fn rng_by_reference() {
        let expr = server_expr("math.random(0, 1) + math.random(0, 1)");
        let mut env = NoHostEnv::new();
        let mut reference = Xorshift128::new();
        let expected = sample(&mut reference) + sample(&mut reference);
        assert_eq!(expr.eval_f32(&mut env.cx()), expected);
    }

    #[test]
    fn failed_expressions_are_zero() {
        for (source, version) in [
            ("", 2),
            ("", 13),
            ("   ", 13),
            ("math.max(3)", 13),
            ("continue; return 5;", 13),
            ("return c.x = 1;", 13),
            ("1 ?? 2", 13),
        ] {
            let compiled = compile(source, &server_at(version));
            assert_eq!(
                compiled.failure(),
                Some(CompileFailure::Rejected),
                "{source:?}"
            );
            let expr = compiled.expr_or_zero().cloned().expect("the failed node");
            let mut env = NoHostEnv::new();
            assert_eq!(expr.eval(&mut env.cx()), Value::ZERO, "{source:?}");
            assert!(env.sink.is_empty());
        }
    }

    /// A `break` outside a loop compiles (logged) and, when it runs, ends the expression silently.
    #[test]
    fn break_outside_a_loop_ends_the_expression() {
        let mut env = NoHostEnv::new();
        env.variables.set(VariableName::new("x"), Value::Float(1.0));
        let expr = server_expr("v.x ? break : 1; v.after = 1;");
        assert_eq!(expr.eval(&mut env.cx()), Value::ZERO);
        assert_eq!(env.variables.get(VariableName::new("after")), None);
        assert!(env.sink.is_empty());
        env.variables.set(VariableName::new("x"), Value::Float(0.0));
        expr.eval(&mut env.cx());
        assert_eq!(
            env.variables.get(VariableName::new("after")),
            Some(&Value::Float(1.0))
        );
    }

    /// A constant with a post-op is worth `S·v + O` when moved into its parent (so an assignment
    /// stores a folded −0 as +0) and its raw value as a conditional branch (a folded −0 reaches the
    /// consumer).
    #[test]
    fn folded_constants_in_branches_and_assignments() {
        // The sum folds to the Float 1 carrying the post-op (1, +1).
        assert_eq!(
            eval("v.x = 2; v.y = 3; v.r = ((v.x + v.y + 1) + (-v.x - v.y)); return v.r;"),
            2.0
        );
        assert_eq!(
            eval("v.x = 2; v.y = 3; v.c = 1; return v.c ? ((v.x + v.y + 1) + (-v.x - v.y)) : 7;"),
            1.0
        );
        assert_eq!(
            eval("v.m = math.mod(-4, 2); return math.atan2(v.m, -1);"),
            180.0
        );
        assert_eq!(
            eval("v.c = 1; return math.atan2(v.c ? math.mod(-4, 2) : 1, -1);"),
            -180.0
        );
    }
}

mod variables {
    use crate::common::compile_support::server_expr;
    use crate::common::host::{Env, TestHost};
    use molangx::compile::ProgramFlags;
    use molangx::hash::HashedStr;
    use molangx::rng::Xorshift128;

    use molangx::vm::{
        CollectSink, NoHost, NoHostEnv, StructValue, TempMap, TempName, Temps, Value, VariableName,
    };

    fn run(env: &mut Env, source: &str) -> (Value<TestHost>, Vec<String>) {
        let expr = server_expr(source);
        let mut rng = Xorshift128::new();
        let mut sink = CollectSink::new();
        let value = env.with_cx(&mut rng, &mut sink, |cx| expr.eval(cx));
        (value, sink.messages)
    }

    /// Temps start empty in every evaluation unless the environment keeps them.
    #[test]
    fn temp_scope() {
        let set = server_expr("t.persist = 42;");
        let get = server_expr("return t.persist;");

        let mut ours = NoHostEnv::new();
        set.eval(&mut ours.cx());
        assert_eq!(get.eval(&mut ours.cx()), Value::ZERO);
        assert_eq!(
            ours.sink.take(),
            vec!["Error: unhandled request for unknown variable 'temp.persist'".to_owned()]
        );

        let mut persistent = NoHostEnv {
            temps: Temps::Kept(TempMap::new()),
            ..NoHostEnv::new()
        };
        set.eval(&mut persistent.cx());
        assert_eq!(get.eval(&mut persistent.cx()), Value::Float(42.0));
        assert!(persistent.sink.is_empty());
        assert_eq!(
            persistent
                .temps
                .kept()
                .and_then(|t| t.get(TempName::new("persist"))),
            Some(&Value::Float(42.0))
        );
    }

    /// Earlier assignments stay, later ones and the `return` never run; the message names the full
    /// canonical name.
    #[test]
    fn missing_read_aborts() {
        let mut env = NoHostEnv::new();
        let expr = server_expr("v.q = 1; v.r = v.missing; v.s = 2; return 3;");
        assert_eq!(expr.eval(&mut env.cx()), Value::ZERO);
        assert_eq!(
            env.variables.get(VariableName::new("q")),
            Some(&Value::Float(1.0))
        );
        assert_eq!(env.variables.get(VariableName::new("r")), None);
        assert_eq!(env.variables.get(VariableName::new("s")), None);
        assert_eq!(
            env.sink.take(),
            vec!["Error: unhandled request for unknown variable 'variable.missing'".to_owned()]
        );

        assert_eq!(
            server_expr("return c.Other;").eval(&mut env.cx()),
            Value::ZERO
        );
        assert_eq!(
            env.sink.take(),
            vec!["Error: unhandled request for unknown variable 'context.other'".to_owned()]
        );
        assert_eq!(
            server_expr("return t.X + 1;").eval(&mut env.cx()),
            Value::ZERO
        );
        assert_eq!(
            env.sink.take(),
            vec!["Error: unhandled request for unknown variable 'temp.x'".to_owned()]
        );
    }

    /// `??` reacts to a missing variable or member only, never to 0 or NaN; its right side may be a
    /// block.
    #[test]
    fn null_coalescing() {
        let mut env = NoHostEnv::new();
        let mut eval = |source: &str| server_expr(source).eval_f32(&mut env.cx());
        assert_eq!(eval("v.b = 0.2; return (v.a ?? 2) + (v.b ?? 3);"), 2.2);
        assert_eq!(eval("v.x = 0; return v.x ?? 5;"), 0.0);
        assert!(eval("v.n = math.sqrt(-1); return v.n ?? 5;").is_nan());
        assert_eq!(eval("v.m = 1; return v.m.b ?? 7;"), 7.0);
        assert_eq!(
            eval("v.dir ?? { v.dir.x = 0; v.dir.y = 1; }; return v.dir.y;"),
            1.0
        );
        assert_eq!(eval("return v.never ?? v.nor ?? 5;"), 5.0);
        assert_eq!(eval("return (v.never ?? 2) * 3 + 1;"), 7.0);
        // A missing read inside a loop under `??` abandons the loop for the right side.
        assert_eq!(
            eval(
                "t.n = 0; v.out = (loop(3, {t.n = t.n + 1; v.never;}) ?? 9); return t.n * 10 + v.out;"
            ),
            19.0
        );
        // Only the member miss is logged.
        assert_eq!(
            env.sink.take(),
            vec!["Error: unable to find member variable .b".to_owned()]
        );
    }

    /// Member writes create the structs on the way; a struct is copied by value.
    #[test]
    fn members() {
        let mut env = NoHostEnv::new();
        let mut eval = |source: &str| server_expr(source).eval_f32(&mut env.cx());
        assert_eq!(eval("v.a.b.c = 2; return v.a.b.c;"), 2.0);
        assert_eq!(
            eval("v.p.x = 1; v.p.y = 2; v.q = v.p; v.q.x = 5; return v.p.x * 10 + v.q.x;"),
            15.0
        );
        assert_eq!(eval("v.f = 3; v.f.g = 4; return v.f.g;"), 4.0);
        assert_eq!(eval("t.s.x = 7; return t.s.x;"), 7.0);
        let struct_value = env
            .variables
            .get(VariableName::new("a"))
            .cloned()
            .expect("v.a");
        assert!(matches!(struct_value, Value::Struct(_)));
        assert_eq!(
            struct_value.member_path(&[HashedStr::new("b"), HashedStr::new("c")]),
            Some(&Value::Float(2.0))
        );

        // A missing member, and a member of a float: the member message, then the abort.
        let expr = server_expr("v.g = 1; return v.g.h;");
        assert_eq!(expr.eval(&mut env.cx()), Value::ZERO);
        assert_eq!(
            env.sink.take(),
            vec![
                "Error: unable to find member variable .h".to_owned(),
                "Error: unhandled request for unknown variable '.h'".to_owned()
            ]
        );
    }

    /// Self-nesting copies (three levels, no cycle).
    #[test]
    fn missing_member_is_reported_by_the_last_member_of_the_path() {
        let mut env = NoHostEnv::new();
        let source =
            "v.s.c = 1; loop(3, { v.s.b = v.s; }); v.r = v.s.b.b.b.c; return v.s.b.b.b.b.c ?? 0;";
        assert_eq!(server_expr(source).eval_f32(&mut env.cx()), 0.0);
        assert_eq!(
            env.variables.get(VariableName::new("r")),
            Some(&Value::Float(1.0))
        );
        assert_eq!(
            env.sink.take(),
            vec!["Error: unable to find member variable .c".to_owned()]
        );
        // Without `??` the abort names the same member.
        assert_eq!(
            server_expr("v.s.c = 1; return v.s.x.y.c;").eval_f32(&mut env.cx()),
            0.0
        );
        assert_eq!(
            env.sink.take(),
            vec![
                "Error: unable to find member variable .c".to_owned(),
                "Error: unhandled request for unknown variable '.c'".to_owned()
            ]
        );
    }

    #[test]
    fn this() {
        let mut env = NoHostEnv {
            this: 2.34,
            ..NoHostEnv::new()
        };
        assert_eq!(server_expr("this * 2").eval_f32(&mut env.cx()), 4.68);
    }

    /// `==` of different kinds is false; a hash in arithmetic is its low 32 bits as a float.
    #[test]
    fn mixed_kinds() {
        let mut env = NoHostEnv::new();
        env.variables
            .set(VariableName::new("h"), Value::string("abc"));
        let low = f32::from_bits(HashedStr::new("abc").as_u64() as u32);
        assert_eq!(server_expr("v.h == 1").eval_f32(&mut env.cx()), 0.0);
        assert_eq!(server_expr("v.h == 'abc'").eval_f32(&mut env.cx()), 1.0);
        assert_eq!(server_expr("'abc' == v.h").eval_f32(&mut env.cx()), 1.0);
        assert_eq!(
            server_expr("v.h * 1").eval_f32(&mut env.cx()).to_bits(),
            low.to_bits()
        );
    }

    /// Covers `Eq`, `EqConst` and `EqHash`, with the constant on either side (it is moved to the
    /// right).
    #[test]
    fn mixed_kind_equality_follows_the_right_operand() {
        let eval = |source: &str| server_expr(source).eval_f32(&mut NoHostEnv::new().cx());
        let rows = [
            // '' against 0, both orders and forms.
            ("v.e = ''; return v.e == 0;", 1.0),
            ("v.e = ''; return v.e != 0;", 0.0),
            ("v.z = 0; return v.z == '';", 1.0),
            ("v.z = 0; return v.z != '';", 0.0),
            ("v.e = ''; v.z = 0; return v.e == v.z;", 1.0),
            ("v.e = ''; v.z = 0; return v.z == v.e;", 1.0),
            // A string against its own low bits as a float.
            (
                "v.one = 1; v.s = 'a'; v.f = v.s * v.one; return v.s == v.f;",
                1.0,
            ),
            (
                "v.one = 1; v.s = 'a'; v.f = v.s * v.one; return v.f == v.s;",
                0.0,
            ),
            (
                "v.one = 1; v.s = 'a'; v.f = v.s * v.one; return v.s != v.f;",
                0.0,
            ),
            (
                "v.one = 1; v.s = 'a'; v.f = v.s * v.one; v.g = v.f * v.one; return v.s == v.g;",
                1.0,
            ),
            ("v.s = 'a'; v.g = 1; return v.s == v.g;", 0.0),
            ("v.s = 'a'; return v.s == 1;", 0.0),
            ("v.a = 'a'; v.b = 'b'; return v.a == v.b;", 0.0),
            // A constant on the left is moved to the right; a run-time −0.
            ("v.e = ''; return 0 == v.e;", 1.0),
            (
                "v.one = 1; v.s = 'a'; v.f = v.s * v.one; return 'a' == v.f;",
                0.0,
            ),
            (
                "v.one = 1; v.s = 'a'; v.f = v.s * v.one; return v.f == 'a';",
                0.0,
            ),
            (
                "v.e = ''; v.h = -0.5; v.nz = math.ceil(v.h); return v.e == v.nz;",
                1.0,
            ),
            (
                "v.e = ''; v.h = -0.5; v.nz = math.ceil(v.h); return v.nz == v.e;",
                0.0,
            ),
            // Strings as conditions.
            ("v.s = 'a'; return v.s ? 1 : 0;", 1.0),
            ("v.s = ''; return v.s ? 1 : 0;", 0.0),
        ];
        for (source, expected) in rows {
            assert_eq!(eval(source), expected, "{source}");
        }
        // `'a' + 0` is the hash's low bits (negative); `v.s + 1` is 1.
        let low = f32::from_bits(HashedStr::new("a").as_u64() as u32);
        assert_eq!(
            eval("v.s = 'a'; v.z = 0; return v.s + v.z;").to_bits(),
            low.to_bits()
        );
        assert_eq!(eval("v.s = 'a'; return v.s + 1;"), 1.0);
    }

    /// A float against a string constant is zero-extended, so a string with upper bits set equals
    /// neither the constant nor the float.
    #[test]
    fn a_string_against_a_float_constant_is_read_by_its_low_32_bits() {
        let one_in_the_low_bits = Value::Hash(HashedStr::from_u64(0x1_3f80_0000));
        let mut env = NoHostEnv::new();
        env.variables
            .set(VariableName::new("s"), one_in_the_low_bits);
        let eval = |source: &str, env: &mut NoHostEnv| server_expr(source).eval_f32(&mut env.cx());
        assert_eq!(eval("v.s == 1", &mut env), 1.0);
        assert_eq!(eval("v.s != 1", &mut env), 0.0);
        assert_eq!(eval("1 == v.s", &mut env), 1.0);
        assert_eq!(eval("v.s == 2", &mut env), 0.0);
        // Both sides run-time values: the same rule through `Eq`.
        env.variables
            .set(VariableName::new("one"), Value::Float(1.0));
        assert_eq!(eval("v.s == v.one", &mut env), 1.0);
        assert_eq!(eval("v.one == v.s", &mut env), 0.0);
    }

    /// A float-only program that meets a non-float value continues on the general loop with the
    /// same state.
    #[test]
    fn float_only_programs_hand_over_non_float_values() {
        let mut env = NoHostEnv::new();
        env.variables
            .set(VariableName::new("h"), Value::string("abc"));
        env.variables.set(
            VariableName::new("s"),
            Value::structure(StructValue::xy(1.0, 2.0)),
        );
        let expr = server_expr("t.k = 3; v.copy = v.h; v.sc = v.s; return (v.copy == v.h) * t.k;");
        assert!(expr.flags().contains(ProgramFlags::FLOAT_ONLY));
        assert_eq!(expr.eval(&mut env.cx()), Value::Float(3.0));
        assert_eq!(
            env.variables.get(VariableName::new("copy")),
            Some(&Value::string("abc"))
        );
        assert_eq!(
            env.variables.get(VariableName::new("sc")),
            env.variables.get(VariableName::new("s"))
        );
        // The whole value comes back from `eval`, its float from `eval_f32`.
        assert_eq!(
            server_expr("v.s").eval(&mut env.cx()),
            Value::structure(StructValue::xy(1.0, 2.0))
        );
        assert_eq!(server_expr("v.s").eval_f32(&mut env.cx()), 0.0);
        assert_eq!(server_expr("v.h").eval(&mut env.cx()), Value::string("abc"));
    }

    /// A string works as an assigned value, a `?:` operand, a `??` right side and a query argument.
    #[test]
    fn strings_are_hashes() {
        let mut env = NoHostEnv::new();
        let abc = Value::<NoHost>::string("abc");
        assert_eq!(
            server_expr("v.s = 'abc'; return v.s;").eval(&mut env.cx()),
            abc
        );
        assert_eq!(env.variables.get(VariableName::new("s")), Some(&abc));
        assert_eq!(
            server_expr("v.c = 1; return v.c ? 'abc' : 'b';").eval(&mut env.cx()),
            abc
        );
        assert_eq!(
            server_expr("return v.never_set ?? 'abc';").eval(&mut env.cx()),
            abc
        );
        assert_eq!(
            server_expr("v.s == 'abc' && v.s != 'b'").eval(&mut env.cx()),
            Value::ONE
        );
        let mut host_env = Env::new();
        assert_eq!(run(&mut host_env, "q.any('abc', 'b', 'abc')").0, Value::ONE);
        assert!(env.sink.is_empty());
    }
}

mod evaluation_model {
    use crate::common::compile_support::{server_expr, server_expr_at};
    use crate::common::host::{Actor, Env};

    use molangx::hash::HashedStr;
    use molangx::rng::Xorshift128;
    use molangx::vm::{
        CollectSink, ContextName, EvalLimits, NoHostEnv, Subjects, Value, VariableName,
    };

    fn eval_in(env: &mut NoHostEnv, source: &str) -> f32 {
        server_expr(source).eval_f32(&mut env.cx())
    }

    fn var(env: &NoHostEnv, name: &str) -> Option<Value<molangx::vm::NoHost>> {
        env.variables.get(VariableName::new(name)).cloned()
    }

    /// Negated it gives 0, plus 1 gives 1; its statements still run.
    #[test]
    fn a_block_as_an_operand_is_zero() {
        let mut env = NoHostEnv::new();
        eval_in(&mut env, "v.y = 7; v.y = -{v.x = 1;};");
        assert_eq!(
            (var(&env, "x"), var(&env, "y")),
            (Some(Value::Float(1.0)), Some(Value::Float(0.0)))
        );
        eval_in(&mut env, "v.x = 0; v.y = {v.x = 1;} + 1;");
        assert_eq!(
            (var(&env, "x"), var(&env, "y")),
            (Some(Value::Float(1.0)), Some(Value::Float(1.0)))
        );
    }

    /// The assignment's value is the written value; at version 2 the arithmetic around an
    /// assignment does not reach the store.
    #[test]
    fn assignments_write_the_value() {
        let mut env = NoHostEnv::new();
        assert_eq!(eval_in(&mut env, "return v.a = 5;"), 5.0);
        assert_eq!(var(&env, "a"), Some(Value::Float(5.0)));
        assert_eq!(eval_in(&mut env, "v.b = 3; return v.c = v.b * 2 + 1;"), 7.0);
        assert_eq!(var(&env, "c"), Some(Value::Float(7.0)));
        let mut env = NoHostEnv::new();
        server_expr_at(
            "v.one = 1; v.two = 2; (v.foo = v.one) + 2; (v.bar = 1) + 2; (v.baz = v.two) * 3;",
            2,
        )
        .eval(&mut env.cx());
        assert_eq!(
            (var(&env, "foo"), var(&env, "bar"), var(&env, "baz")),
            (
                Some(Value::Float(1.0)),
                Some(Value::Float(1.0)),
                Some(Value::Float(2.0))
            )
        );
    }

    /// Here the step budget runs out inside the right side's query argument.
    #[test]
    fn arrow_subjects_are_restored_after_an_early_end() {
        let mut env = Env::new();
        env.world.alive.extend([1, 2]);
        env.context
            .set(ContextName::new("moo"), Value::Actor(Actor::Handle(2)));
        env.limits = EvalLimits {
            total_steps: Some(50),
            ..EvalLimits::NONE
        };
        let expr = server_expr("v.r = c.moo->q.log(math.die_roll(1000, 1, 2)); return 1;");
        let mut rng = Xorshift128::new();
        let mut sink = CollectSink::new();
        let (value, subjects) = env.with_cx(&mut rng, &mut sink, |cx| {
            cx.subjects = Subjects::actor(Actor::Handle(1));
            let value = expr.eval_f32(cx);
            (value, cx.subjects)
        });
        assert_eq!(value, 0.0);
        assert_eq!(subjects, Subjects::actor(Actor::Handle(1)));
        assert_eq!(
            sink.messages,
            ["molangx: evaluation stopped after its budget of 50 steps"]
        );
    }

    /// The loop counter is an `f32` counted down by 1: `+inf`, 2^25 and a multiple of 4 in
    /// [2^24, 2^25) never reach the end; only the step budget ends them.
    #[test]
    fn large_loop_counts_stick() {
        let mut env = NoHostEnv {
            limits: EvalLimits {
                loop_iterations: None,
                total_steps: Some(20_000),
                ..EvalLimits::NONE
            },
            ..NoHostEnv::new()
        };
        for count in ["33554432", "16777220", "16777228", "v.inf"] {
            env.variables
                .set(VariableName::new("inf"), Value::Float(f32::INFINITY));
            let source = format!("v.i = 0; loop({count}, {{v.i = v.i + 1;}}); return 1;");
            assert_eq!(eval_in(&mut env, &source), 0.0, "{count}");
            assert_eq!(
                env.sink.take(),
                ["molangx: evaluation stopped after its budget of 20000 steps"],
                "{count}"
            );
        }
        // The f32 arithmetic these counts rest on.
        for start in [16_777_220.0_f32, 33_554_432.0] {
            assert_eq!(start - 1.0, start);
        }
    }

    /// A query argument left by a `break` with no loop around it leaves the caller's pending
    /// operands as it found them, and the next evaluation starts clean.
    #[test]
    fn a_nested_evaluation_leaves_the_shared_state_as_it_found_it() {
        let mut env = Env::new();
        env.vars.set(VariableName::new("one"), Value::Float(1.0));
        env.vars.set(VariableName::new("k"), Value::Float(5.0));
        env.vars.set(VariableName::new("two"), Value::Float(2.0));
        // The caller has `v.two` pending, the argument `v.k`; were `v.k` left behind, `max` would
        // give 51.
        for jump in ["break", "continue"] {
            let expr = server_expr(&format!(
                "v.r = math.max(v.two, q.log(math.max(v.k, (v.one ? {jump} : 1)))); return v.r * 10 + 1;"
            ));
            let mut rng = Xorshift128::new();
            for _ in 0..2 {
                let mut sink = CollectSink::new();
                assert_eq!(
                    env.with_cx(&mut rng, &mut sink, |cx| expr.eval_f32(cx)),
                    21.0,
                    "{jump}"
                );
                assert!(sink.messages.is_empty(), "{:?}", sink.messages);
            }
            let mut sink = CollectSink::new();
            assert_eq!(
                env.with_cx(&mut rng, &mut sink, |cx| server_expr("return 2 * 3 + 1;")
                    .eval_f32(cx)),
                7.0
            );
        }
    }

    #[test]
    fn eval_returns_an_owned_value() {
        let mut env = NoHostEnv::new();
        let read = server_expr("v.s");
        eval_in(&mut env, "v.s.x = 1;");
        let before = read.eval(&mut env.cx());
        eval_in(&mut env, "v.s.x = 2;");
        let after = read.eval(&mut env.cx());
        assert_ne!(before, after);
        assert_eq!(before.member(HashedStr::new("x")), Some(&Value::Float(1.0)));
        assert_eq!(after.member(HashedStr::new("x")), Some(&Value::Float(2.0)));
    }
}

mod names {
    use crate::common::compile_support::server_expr;

    use molangx::rng::Xorshift128;
    use molangx::vm::{CollectSink, ContextName, NoHostEnv, TempMap, Temps, Value};

    use crate::common::host::{Actor, Env};

    fn eval_in(env: &mut NoHostEnv, source: &str) -> f32 {
        server_expr(source).eval_f32(&mut env.cx())
    }

    #[test]
    fn aliases_name_the_same_variable() {
        let mut env = NoHostEnv {
            temps: Temps::Kept(TempMap::new()),
            ..NoHostEnv::new()
        };
        env.context.set(ContextName::new("c"), Value::Float(4.0));
        assert_eq!(
            eval_in(
                &mut env,
                "variable.a = 1; temp.b = 2; return v.a + t.b * 10 + c.c * 100;"
            ),
            421.0
        );
        assert_eq!(
            eval_in(
                &mut env,
                "v.a = 5; t.b = 6; return variable.a + temp.b * 10 + context.c * 100;"
            ),
            465.0
        );
    }

    #[test]
    fn temp_member_assignment_runs() {
        assert_eq!(eval_in(&mut NoHostEnv::new(), "t.a.b = 1; return 1;"), 1.0);
    }

    #[test]
    fn this_reads_the_current_value() {
        let mut env = NoHostEnv {
            this: 2.34,
            ..NoHostEnv::new()
        };
        assert_eq!(eval_in(&mut env, "return this;"), 2.34);
    }

    #[test]
    fn query_without_parentheses_is_a_call() {
        let mut env = Env::new();
        env.world.alive.insert(1);
        env.world.baby.insert(1);
        env.context
            .set(ContextName::new("moo"), Value::Actor(Actor::Handle(1)));
        let expr = server_expr("c.moo->q.is_baby + c.moo->query.is_baby");
        let mut rng = Xorshift128::new();
        let mut sink = CollectSink::new();
        assert_eq!(
            env.with_cx(&mut rng, &mut sink, |cx| expr.eval_f32(cx)),
            2.0
        );
    }
}

mod statements {
    use crate::common::compile_support::{server_at, server_expr_at};
    use molangx::compile::compile;
    use molangx::vm::{NoHostEnv, Value, VariableName};

    fn eval_at(source: &str, version: i16) -> (f32, NoHostEnv) {
        let expr = server_expr_at(source, version);
        let mut env = NoHostEnv::new();
        let value = expr.eval_f32(&mut env.cx());
        (value, env)
    }

    #[test]
    fn simple_expression_value() {
        let compiled = compile("v.x+1", &server_at(13));
        let mut env = NoHostEnv::new();
        env.variables.set(VariableName::new("x"), Value::Float(2.0));
        assert_eq!(
            compiled
                .expr()
                .cloned()
                .expect("an expression")
                .eval_f32(&mut env.cx()),
            3.0
        );
    }

    /// A trailing value statement does not become the list's value.
    #[test]
    fn statement_list_value() {
        assert_eq!(eval_at("1 ? 2;", 13).0, 0.0);
        assert_eq!(eval_at("return 1 ? 2;", 13).0, 2.0);
        let (value, env) = eval_at("v.x = 1; 2;", 13);
        assert_eq!(value, 0.0);
        assert_eq!(
            env.variables.get(VariableName::new("x")),
            Some(&Value::Float(1.0))
        );
    }

    /// Also from a block, a branch or a nested loop body.
    #[test]
    fn return_ends_the_whole_expression() {
        let (value, env) = eval_at(
            "v.n = 0; loop(3, { v.n = v.n + 1; return 1; }); v.after = 1;",
            13,
        );
        assert_eq!(value, 1.0);
        assert_eq!(
            env.variables.get(VariableName::new("n")),
            Some(&Value::Float(1.0))
        );
        assert_eq!(env.variables.get(VariableName::new("after")), None);
        let (value, env) = eval_at(
            "v.n = 0; v.m = 0; loop(2, { loop(2, { v.n = v.n + 1; return 1; }); v.m = v.m + 1; }); v.after = 1;",
            13,
        );
        assert_eq!(value, 1.0);
        assert_eq!(
            env.variables.get(VariableName::new("m")),
            Some(&Value::Float(0.0))
        );
        let (value, env) = eval_at("v.c = 1; v.c ? { v.s = 3; return 2; } : 0; v.s = 2;", 13);
        assert_eq!(value, 2.0);
        assert_eq!(
            env.variables.get(VariableName::new("s")),
            Some(&Value::Float(3.0))
        );
        assert_eq!(
            eval_at("v.x = 1; v.y = 0; v.x ? (v.y ? {return 3;} : {return 1;}) : (v.y ? {return 2;} : {return 0;}); return 4;", 13).0,
            1.0
        );
    }

    #[test]
    fn version_2_operand_statements_run() {
        assert_eq!(
            eval_at(
                "v.count = 0; loop(3,{v.count = v.count + 1;}) + 1; return v.count;",
                2
            )
            .0,
            3.0
        );
        assert_eq!(eval_at("(v.foo = 1) + 2; return v.foo;", 2).0, 1.0);
    }

    /// The value is 0 and nothing is logged.
    #[test]
    fn jump_outside_a_loop_ends_the_expression() {
        for source in [
            "v.x = 5; v.c = 1; v.c ? {break;}; v.after = 1;",
            "v.x = 5; v.c = 1; v.c ? {continue;}; v.after = 1;",
        ] {
            let compiled = compile(source, &server_at(13));
            let expr = compiled.expr().cloned().expect("kept");
            let mut env = NoHostEnv::new();
            assert_eq!(expr.eval_f32(&mut env.cx()), 0.0, "{source}");
            assert_eq!(
                env.variables.get(VariableName::new("x")),
                Some(&Value::Float(5.0)),
                "{source}"
            );
            assert_eq!(
                env.variables.get(VariableName::new("after")),
                None,
                "{source}"
            );
            assert!(env.sink.is_empty(), "{source}");
        }
    }
}

mod result_kinds {
    //! `eval_f32` reads a string as the low 32 bits of its hash and any other non-float as +0.

    use crate::common::compile_support::{server_at, server_expr};
    use crate::common::host::{Actor, Env};
    use molangx::compile::{CompileFailure, compile};
    use molangx::hash::HashedStr;
    use molangx::rng::Xorshift128;
    use molangx::vm::{
        CollectSink, ContextName, NoHost, NoHostEnv, StructValue, Value, VariableName,
    };

    fn both(source: &str) -> (Value<NoHost>, f32) {
        let expr = server_expr(source);
        let full = expr.eval(&mut NoHostEnv::new().cx());
        let float = expr.eval_f32(&mut NoHostEnv::new().cx());
        (full, float)
    }

    #[test]
    fn a_number_is_the_same_through_both_reads() {
        assert_eq!(
            both("v.x = 3; return v.x * 2 + 1;"),
            (Value::Float(7.0), 7.0)
        );
        assert_eq!(both("1 + 2 * 3"), (Value::Float(7.0), 7.0));
        let (full, float) = both("v.n = math.sqrt(-1); return v.n;");
        assert!(matches!(full, Value::Float(x) if x.is_nan()));
        assert!(float.is_nan());
    }

    #[test]
    fn a_string_is_a_hash_and_its_low_bits() {
        let low = f32::from_bits(HashedStr::new("moo").as_u64() as u32);
        for source in ["return 'moo';", "v.s = 'moo'; return v.s;"] {
            let (full, float) = both(source);
            assert_eq!(full, Value::string("moo"), "{source}");
            assert_eq!(float.to_bits(), low.to_bits(), "{source}");
        }
        let (full, float) = both("return '';");
        assert_eq!(full, Value::string(""));
        assert_eq!(float.to_bits(), 0.0_f32.to_bits());
    }

    #[test]
    fn a_struct_comes_back_whole_and_is_zero_as_a_float() {
        let mut env = NoHostEnv::new();
        env.variables.set(
            VariableName::new("s"),
            Value::structure(StructValue::xy(1.0, 2.0)),
        );
        let read = server_expr("v.s");
        assert_eq!(
            read.eval(&mut env.cx()),
            Value::structure(StructValue::xy(1.0, 2.0))
        );
        assert_eq!(read.eval_f32(&mut env.cx()).to_bits(), 0.0_f32.to_bits());
        assert!(env.sink.is_empty());
    }

    #[test]
    fn an_actor_comes_back_as_the_actor_and_is_zero_as_a_float() {
        let mut env = Env::new();
        env.world.alive.insert(1);
        env.context
            .set(ContextName::new("moo"), Value::Actor(Actor::Handle(1)));
        let read = server_expr("c.moo");
        let mut rng = Xorshift128::new();
        let mut sink = CollectSink::new();
        assert_eq!(
            env.with_cx(&mut rng, &mut sink, |cx| read.eval(cx)),
            Value::Actor(Actor::Handle(1))
        );
        assert_eq!(
            env.with_cx(&mut rng, &mut sink, |cx| read.eval_f32(cx))
                .to_bits(),
            0.0_f32.to_bits()
        );
        assert!(sink.messages.is_empty());
    }

    /// Only the abort logs.
    #[test]
    fn nothing_to_return_is_zero_through_both_reads() {
        for (source, messages) in [
            ("v.x = 1;", 0),
            ("v.x = 1; return v.never_set;", 1),
            ("loop(2, {v.n = 1;});", 0),
        ] {
            let program = server_expr(source);
            let mut env = NoHostEnv::new();
            assert_eq!(program.eval(&mut env.cx()), Value::ZERO, "{source}");
            assert_eq!(env.sink.take().len(), messages, "{source}");
            assert_eq!(
                program.eval_f32(&mut env.cx()).to_bits(),
                0.0_f32.to_bits(),
                "{source}"
            );
            assert_eq!(env.sink.take().len(), messages, "{source}");
        }
        let rejected = compile("1 +", &server_at(13));
        assert_eq!(rejected.failure(), Some(CompileFailure::Rejected));
        let rejected = rejected.expr_or_zero().cloned().expect("the failed node");
        let mut env = NoHostEnv::new();
        assert_eq!(rejected.eval(&mut env.cx()), Value::ZERO);
        assert_eq!(rejected.eval_f32(&mut env.cx()), 0.0);
        assert!(env.sink.is_empty());
    }

    #[test]
    fn booleans_are_one_and_zero() {
        for (source, expected) in [
            ("1 < 2", 1.0),
            ("2 < 1", 0.0),
            ("1 && 0", 0.0),
            ("1 || 0", 1.0),
            ("!0", 1.0),
            ("v.a = 5; return v.a == 5;", 1.0),
        ] {
            assert_eq!(both(source), (Value::Float(expected), expected), "{source}");
        }
    }
}

mod threads {
    use crate::common::compile_support::server_expr;
    use molangx::compile::Expr;
    use molangx::vm::{NoHostEnv, Value, VariableName};

    fn assert_send_sync<T: Send + Sync>() {}
    fn assert_send<T: Send>() {}

    #[test]
    fn expressions_and_environments_cross_threads() {
        assert_send_sync::<Expr>();
        assert_send::<NoHostEnv>();
    }

    #[test]
    fn the_same_program_evaluated_from_several_threads() {
        let expr = server_expr("v.s = 0; loop(10, {v.s = v.s + v.n;}); return v.s * 2;");
        let results: Vec<(u32, f32)> = std::thread::scope(|scope| {
            let handles: Vec<_> = (0..8_u32)
                .map(|n| {
                    let expr = &expr;
                    scope.spawn(move || {
                        let mut env = NoHostEnv::new();
                        env.variables
                            .set(VariableName::new("n"), Value::Float(n as f32));
                        let mut total = 0.0;
                        for _ in 0..50 {
                            total = expr.eval_f32(&mut env.cx());
                        }
                        (n, total)
                    })
                })
                .collect();
            handles
                .into_iter()
                .map(|h| h.join().expect("a thread"))
                .collect()
        });
        assert_eq!(results.len(), 8);
        for (n, total) in results {
            assert_eq!(total, n as f32 * 20.0, "thread {n}");
        }
    }
}
