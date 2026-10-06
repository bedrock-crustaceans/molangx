//! The bytecode VM against the tree walker on inputs outside the main crate's data files.

mod common;

mod differential {
    //! The pack corpus runs through `common::differential` locally (ignored test; the corpus is
    //! never committed):
    //!
    //! ```text
    //! MOLANG_CORPUS_DIR=/path/to/corpus cargo test --release --manifest-path fuzz/Cargo.toml --test tree_walker -- --ignored --nocapture
    //! ```

    use std::sync::Arc;

    use crate::common::{
        differential::{Tally, differential},
        host::{Actor, Env, LIVE_ACTOR, SECOND_ACTOR},
    };
    use molangx::compile::{CompileOptions, compile};
    use molangx::rng::Xorshift128;
    use molangx::version::MolangVersion;
    use molangx::vm::{ContextName, EvalLimits, Value, VariableName};
    use molangx_fuzz::generator::env::FuzzRng;
    use molangx_fuzz::tree_walker;

    /// Every corpus pair, compiled for the client and evaluated with stub queries and a persistent
    /// temp map.
    #[test]
    #[ignore = "needs MOLANG_CORPUS_DIR (Mojang content, never committed)"]
    fn vm_equals_tree_walker_on_the_corpus() {
        let Some(dir) = std::env::var_os("MOLANG_CORPUS_DIR") else {
            println!("MOLANG_CORPUS_DIR is not set; nothing to compare");
            return;
        };
        let path = std::path::PathBuf::from(dir).join("corpus_pairs.json");
        let text = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()));
        let json: serde_json::Value = serde_json::from_str(&text).expect("corpus json");
        let mut tally = Tally::default();
        for pair in json["pairs"].as_array().expect("pairs") {
            let source = pair["expr"].as_str().expect("expr");
            let version = i16::try_from(pair["version"].as_i64().expect("version")).expect("i16");
            let options = crate::common::client_at(version);
            let mut env = Env::reference();
            env.limits = EvalLimits::DEFAULT;
            tally.check(
                "corpus",
                source,
                &compile(source, &options),
                &env,
                &FuzzRng::Xorshift(Xorshift128::new()),
            );
        }
        println!(
            "corpus: {} expressions, {} evaluations, {} disagreements",
            tally.expressions,
            tally.evaluations,
            tally.failures.len()
        );
        // Report counts and the first few only: corpus strings are not ours to print in full.
        for failure in tally.failures.iter().take(10) {
            println!("  {failure}");
        }
        assert!(
            tally.failures.is_empty(),
            "{} corpus disagreements",
            tally.failures.len()
        );
    }

    /// After a `break` out of a `??` left side, a later missing read continues with the abandoned
    /// right side in the VM; the tree walker does not model that and says so.
    #[test]
    fn a_jump_out_of_a_coalescing_left_side_is_not_modelled() {
        let options = CompileOptions::server(MolangVersion::LATEST);
        let stale = compile(
            "loop(2, { v.a = (v.c ?? {break;}) ?? 2; }); return v.never;",
            &options,
        )
        .expr()
        .cloned()
        .expect("compiles");
        assert!(!tree_walker::models(&stale));
        let mut env = molangx::vm::NoHostEnv::new();
        assert_eq!(stale.eval(&mut env.cx()), Value::ZERO);
        // The missing `v.never` jumped to the outer handler: `v.a = 2` ran after the loop.
        assert_eq!(
            env.variables.get(VariableName::new("a")),
            Some(&Value::Float(2.0))
        );
        assert_eq!(
            env.sink.take(),
            ["Error: unhandled request for unknown variable 'variable.never'"]
        );
        // Without a loop the jump ends the expression, which the tree walker models.
        for source in [
            "v.a = (v.c ?? {break;}) ?? 2; return v.never;",
            "loop(2, { v.a = v.c ?? 2; break; }); return 1;",
        ] {
            let expr = compile(source, &options).expr().cloned().expect("compiles");
            assert!(tree_walker::models(&expr), "{source}");
        }
    }

    /// The host-protection costs and budgets (member and actor-array store copies, struct width,
    /// query arguments) end the evaluation at the same step, with the same message and writes, on
    /// both. Every step budget from 0 to 300 is tried, so each cost boundary is crossed.
    #[test]
    fn the_host_protection_budgets_agree() {
        let options = CompileOptions::server(MolangVersion::LATEST);
        let mut env = Env::new();
        env.world.alive.extend([LIVE_ACTOR, SECOND_ACTOR]);
        env.context.set(
            ContextName::new("arr"),
            Value::ActorArray(Arc::new(vec![
                Actor::Handle(LIVE_ACTOR),
                Actor::Handle(9),
                Actor::Handle(SECOND_ACTOR),
            ])),
        );
        let rng = FuzzRng::Xorshift(Xorshift128::new());
        let sources = [
            "v.s.a = 1; v.s.b = 2; v.s.c.d = 3; v.t = v.s; v.s.e = v.t; v.s.e.f = 1; return v.s.c.d;",
            "t.s.x = 1; t.s.y.z = 2; loop(4, { t.c = t.s; t.s.n = t.c; }); return t.s.y.z;",
            "v.a = c.arr; t.b = c.arr; v.s.arr = c.arr; v.s.arr2 = v.a; return 1;",
            "v.r = q.log(q.log(q.log(1))); return v.r + q.log(2, q.log(3));",
            "v.w.a = 1; v.w.b = 1; v.w.c = 1; v.w.a = 2; v.w.d = 1; return v.w.a;",
        ];
        let mut evaluations = 0;
        for source in sources {
            let expr = compile(source, &options).expr().cloned().expect("compiles");
            assert!(tree_walker::models(&expr), "{source}");
            for steps in 0..=300 {
                for struct_members in [0, 1, 2, 3, u32::MAX] {
                    for query_depth in [0, 1, 2, u32::MAX] {
                        let limits = EvalLimits {
                            total_steps: Some(steps),
                            struct_members: Some(struct_members),
                            query_depth: Some(query_depth),
                            ..EvalLimits::DEFAULT
                        };
                        if let (_, Some(why)) = differential(&expr, &env, &rng, limits) {
                            panic!("{source:?} under {limits:?}: {why}");
                        }
                        evaluations += 1;
                    }
                }
            }
        }
        assert_eq!(evaluations, 5 * 301 * 5 * 4);
    }
}

mod float_loop {
    //! A program float-only but for argument-less queries starts on the float-only loop, which
    //! hands a non-float query result over to the general loop. Either way the value, variables,
    //! messages and step accounting match the tree walker.

    use molangx::catalog::Side;
    use molangx::compile::{CompileOptions, ProgramFlags, compile};
    use molangx::hash::HashedStr;
    use molangx::stdlib::query;
    use molangx::version::MolangVersion;
    use molangx::vm::{
        EvalLimits, NoHost, NoHostEnv, QueryCx, QueryError, QueryTable, Value, VariableName,
    };
    use molangx_fuzz::tree_walker;

    /// A string, which the float loop cannot hold.
    #[allow(
        clippy::unnecessary_wraps,
        reason = "a query returns a `QueryResult` even when it cannot fail"
    )]
    fn hash_query(_cx: &mut QueryCx<'_, '_, NoHost>) -> Result<Value<NoHost>, QueryError> {
        Ok(Value::Hash(HashedStr::new("moo")))
    }

    #[allow(
        clippy::unnecessary_wraps,
        reason = "a query returns a `QueryResult` even when it cannot fail"
    )]
    fn float_query(_cx: &mut QueryCx<'_, '_, NoHost>) -> Result<Value<NoHost>, QueryError> {
        Ok(Value::Float(7.5))
    }

    /// Fails, so the call count shows in the messages.
    fn failing_query(cx: &mut QueryCx<'_, '_, NoHost>) -> Result<Value<NoHost>, QueryError> {
        Err(cx.error("query failed on purpose"))
    }

    fn env(limits: EvalLimits) -> NoHostEnv {
        let mut queries = QueryTable::new(molangx::stdlib::queries(Side::Server));
        queries.set(query::ANGER_LEVEL, hash_query).unwrap();
        queries.set(query::HEALTH, float_query).unwrap();
        queries.set(query::LIFE_TIME, failing_query).unwrap();
        let mut env = NoHostEnv {
            queries: Some(queries),
            limits,
            ..NoHostEnv::new()
        };
        env.variables
            .set(VariableName::new("one"), Value::Float(1.0));
        env
    }

    const CASES: &[&str] = &[
        "query.anger_level",
        "query.anger_level * 2 + 1",
        "query.health",
        "math.clamp(query.health / 80 * 1.5, 0, 1.5)",
        "t.x = query.anger_level; v.copy = t.x; return v.copy;",
        "v.a = 1 + query.health; v.b = query.anger_level; v.c = v.a + 2; return v.c;",
        "v.n = 0; loop(3, { v.n = v.n + query.health; v.s = query.anger_level; }); return v.n;",
        "query.life_time + query.life_time",
        "v.one ? query.anger_level : query.health",
    ];

    #[test]
    fn queries_without_arguments_start_on_the_float_loop_and_agree_with_the_tree_walker() {
        let options = CompileOptions::server(MolangVersion::LATEST);
        for source in CASES {
            let expr = compile(source, &options)
                .expr()
                .cloned()
                .unwrap_or_else(|| panic!("`{source}` compiles"));
            assert!(
                !expr.flags().contains(ProgramFlags::FLOAT_ONLY),
                "{source}: a query is never FLOAT_ONLY"
            );
            let mut budgets = vec![EvalLimits::DEFAULT];
            budgets.extend((0..12).map(|steps| EvalLimits {
                total_steps: Some(steps),
                ..EvalLimits::DEFAULT
            }));
            for limits in budgets {
                let mut vm_env = env(limits);
                let mut walker_env = env(limits);
                let vm = expr.eval(&mut vm_env.cx());
                let reference = tree_walker::eval(&expr, &mut walker_env.cx());
                assert_eq!(vm, reference, "{source} under {limits:?}: value");
                assert_eq!(
                    vm_env.variables, walker_env.variables,
                    "{source} under {limits:?}: variables"
                );
                assert_eq!(
                    vm_env.sink.take(),
                    walker_env.sink.take(),
                    "{source} under {limits:?}: messages"
                );

                let mut f32_env = env(limits);
                let float = expr.eval_f32(&mut f32_env.cx());
                assert_eq!(
                    float.to_bits(),
                    reference.as_f32().to_bits(),
                    "{source} under {limits:?}: eval_f32"
                );
                assert_eq!(
                    f32_env.variables, walker_env.variables,
                    "{source} under {limits:?}: eval_f32 variables"
                );
            }
        }
    }

    #[test]
    fn a_string_result_is_kept_whole() {
        let expr = compile(
            "t.x = query.anger_level; v.copy = t.x; return v.copy;",
            &CompileOptions::server(MolangVersion::LATEST),
        )
        .expr()
        .cloned()
        .unwrap();
        let mut env = env(EvalLimits::DEFAULT);
        let value = expr.eval(&mut env.cx());
        assert_eq!(value, Value::Hash(HashedStr::new("moo")));
        assert_eq!(env.variables.get(VariableName::new("copy")), Some(&value));
    }
}

mod modelled_jumps {
    use molangx::compile::{CompileOptions, compile};
    use molangx::version::MolangVersion;
    use molangx_fuzz::tree_walker;

    /// A `break` / `continue` taken inside an operand ends the loop early, and inside an inner loop
    /// makes the enclosing loop run past its count; the VM gets that from its operand stack, which
    /// the walker lacks, so `tree_walker::models` declines exactly those shapes.
    #[test]
    fn a_jump_with_an_operand_pending_is_not_modelled() {
        let options = CompileOptions::server(MolangVersion::LATEST);
        let models = |source: &str| {
            tree_walker::models(
                &compile(source, &options)
                    .expr_or_zero()
                    .cloned()
                    .expect("compiles"),
            )
        };
        for source in [
            "loop(3, { v.t = v.k * (v.c ? {continue;} : 0); });",
            "loop(3, { v.t = v.k + (v.c ? {break;} : 0); });",
            "loop(3, { v.t = math.max(v.k, (v.c ? {continue;} : 0)); });",
            "loop(2, { loop(3, { v.t = v.z * (v.c ? {continue;} : 0); }); });",
        ] {
            assert!(!models(source), "{source}");
        }
        for source in [
            "loop(3, { v.c ? {continue;} : 0; });",
            "loop(3, { v.t = (v.c ? {break;} : 0); });",
            "loop(3, { v.c && {break;}; });",
            "loop(3, { v.r = q.log(v.k * (v.c ? {break;} : 0)); });",
        ] {
            assert!(models(source), "{source}");
        }
        // Not for the jump: a `for_each` is never modelled.
        assert!(!models(
            "for_each(t.x, v.a, { v.t = v.k * (v.c ? {continue;} : 0); });"
        ));
    }
}

mod public_api {
    use molangx::compile::{CompileOptions, compile};
    use molangx::version::MolangVersion;
    use molangx::vm::NoHostEnv;
    use molangx_fuzz::tree_walker;

    #[test]
    fn tree_walker_smoke() {
        let expr = compile(
            "t.a = 2; return t.a * math.clamp(7, 0, 5) + q.is_baby;",
            &CompileOptions::client(MolangVersion::LATEST),
        )
        .expr()
        .cloned()
        .expect("expr");
        let mut env = NoHostEnv::new();
        assert_eq!(tree_walker::eval(&expr, &mut env.cx()).as_f32(), 10.0);
        assert_eq!(expr.eval(&mut env.cx()).as_f32(), 10.0);
    }
}
