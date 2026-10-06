//! Embedding the evaluator: what a host implements and what it sees.

#![cfg(all(feature = "vm", feature = "stdlib"))]

mod common;

use molangx::vm::{NoHost, NoHostEnv, QueryTable};

/// The query table of an environment built with one.
fn table(env: &mut NoHostEnv) -> &mut QueryTable<NoHost> {
    env.queries.as_mut().expect("a query table")
}

mod queries {
    use crate::common::compile_support::{client_expr, client_expr_at};
    use crate::table;
    use molangx::hash::HashedStr;
    use molangx::vm::QueryTable;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use molangx::catalog::Side;
    use molangx::compile::{CompileOptions, Deviations, compile};
    use molangx::diag::DiagCode;
    use molangx::stdlib::query;
    use molangx::version::MolangVersion;
    use molangx::vm::{NoHost, NoHostEnv, QueryCx, QueryError, Value, VariableName};

    type Result = std::result::Result<Value<NoHost>, QueryError>;

    static ARG_EVALUATIONS: AtomicUsize = AtomicUsize::new(0);

    /// Evaluates its first argument twice and returns the second result; never looks at the rest.
    fn twice_first(cx: &mut QueryCx<'_, '_, NoHost>) -> Result {
        ARG_EVALUATIONS.fetch_add(1, Ordering::SeqCst);
        let _ = cx.arg(0);
        Ok(cx.arg(0).unwrap_or_default())
    }

    /// Returns which implementation of its name resolved, and the version of the caller.
    fn which_implementation(cx: &mut QueryCx<'_, '_, NoHost>) -> Result {
        Ok(Value::Float(
            f32::from(cx.implementation()) * 100.0 + f32::from(cx.version().as_i16()),
        ))
    }

    fn failing(cx: &mut QueryCx<'_, '_, NoHost>) -> Result {
        Err(cx.error("Error: query.is_baby is not available here."))
    }

    /// Reads `variable.base` through the query context and adds its argument.
    fn reads_variable(cx: &mut QueryCx<'_, '_, NoHost>) -> Result {
        let base = cx
            .variable(VariableName::new("base"))
            .map_or(0.0, |v| v.as_f32());
        Ok(Value::Float(base + cx.arg_f32(0).unwrap_or(0.0)))
    }

    /// Each argument access runs the argument again, so a query decides which run and how often.
    #[test]
    fn arguments_are_evaluated_on_demand() {
        let mut env = NoHostEnv {
            queries: Some(QueryTable::new(molangx::stdlib::queries(Side::Server))),
            ..NoHostEnv::new()
        };
        table(&mut env).set(query::IS_BABY, twice_first).unwrap();
        // `t.n = t.n + 1` is not allowed as an operand, so the side effect is a random draw.
        let expr = client_expr("q.is_baby(math.random(0, 10), math.random(0, 10))");
        let before = ARG_EVALUATIONS.load(Ordering::SeqCst);
        let mut rng = crate::common::Samples::repeat(0.5);
        let value = {
            let mut cx = env.cx();
            cx.rng = &mut rng;
            expr.eval(&mut cx)
        };
        assert_eq!(value, Value::Float(5.0));
        // The first argument ran twice, the second never.
        assert_eq!(rng.draws, 2);
        assert_eq!(ARG_EVALUATIONS.load(Ordering::SeqCst) - before, 1);
    }

    #[test]
    fn arguments_share_the_evaluation() {
        let mut env = NoHostEnv {
            queries: Some(QueryTable::new(molangx::stdlib::queries(Side::Server))),
            ..NoHostEnv::new()
        };
        table(&mut env).set(query::IS_BABY, reads_variable).unwrap();
        env.variables
            .set(VariableName::new("base"), Value::Float(10.0));
        let expr =
            client_expr("t.k = 2; v.x = 3; return q.is_baby(t.k * v.x + q.is_baby(1)) * 2 + 1;");
        // 10 + (2·3 + (10 + 1)) = 27, then ·2 + 1.
        assert_eq!(expr.eval_f32(&mut env.cx()), 55.0);
    }

    /// The argument logs and reads 0; the call and the caller continue, and an outer `??` does not
    /// catch it.
    #[test]
    fn a_missing_read_in_an_argument_ends_only_the_argument() {
        let mut env = NoHostEnv {
            queries: Some(QueryTable::new(molangx::stdlib::queries(Side::Server))),
            ..NoHostEnv::new()
        };
        table(&mut env).set(query::IS_BABY, reads_variable).unwrap();
        env.variables
            .set(VariableName::new("base"), Value::Float(10.0));
        let expr = client_expr("v.r = q.is_baby(v.never_set + 1) ?? 99; return v.r + 1;");
        assert_eq!(expr.eval_f32(&mut env.cx()), 11.0);
        assert_eq!(
            env.sink.take(),
            vec!["Error: unhandled request for unknown variable 'variable.never_set'".to_owned()]
        );
    }

    /// A stub returns its declared default; one function serving a name with two version ranges
    /// learns which resolved; an `Err` is logged and the call is worth the default.
    #[test]
    fn stubs_version_ranges_and_failures() {
        let mut env = NoHostEnv {
            queries: Some(QueryTable::new(molangx::stdlib::queries(Side::Server))),
            ..NoHostEnv::new()
        };
        for (name, default) in [
            ("query.is_baby", 0.0),
            ("query.armor_color_slot", 1.0),
            ("query.time_since_last_vibration_detection", -1.0),
        ] {
            assert_eq!(table(&mut env).is_stub(name), Ok(true), "{name}");
            assert_eq!(
                client_expr(name).eval(&mut env.cx()),
                Value::Float(default),
                "{name}"
            );
        }
        assert_eq!(
            client_expr("q.get_equipped_item_name").eval(&mut env.cx()),
            Value::Hash(HashedStr::EMPTY)
        );

        table(&mut env)
            .set(query::CAPE_FLAP_AMOUNT, which_implementation)
            .unwrap();
        assert_eq!(
            client_expr_at("q.cape_flap_amount", 7).eval_f32(&mut env.cx()),
            7.0
        );
        assert_eq!(
            client_expr_at("q.cape_flap_amount", 8).eval_f32(&mut env.cx()),
            108.0
        );
        assert_eq!(
            client_expr_at("q.cape_flap_amount", 13).eval_f32(&mut env.cx()),
            113.0
        );

        table(&mut env).set(query::IS_BABY, failing).unwrap();
        assert_eq!(
            client_expr("v.x = q.is_baby + 2; return v.x;").eval_f32(&mut env.cx()),
            2.0
        );
        assert_eq!(
            env.sink.take(),
            vec!["Error: query.is_baby is not available here.".to_owned()]
        );
    }

    /// The query lints switch off with `Deviations::NONE`.
    #[test]
    fn query_lints_are_switchable() {
        let lint = |deviations: Deviations, side: Side, source: &str, code: DiagCode| {
            let options = CompileOptions {
                deviations,
                ..CompileOptions::new(
                    molangx::stdlib::queries(side).clone(),
                    MolangVersion::LATEST,
                )
            };
            compile(source, &options)
                .diagnostics()
                .iter()
                .any(|d| d.code() == code)
        };
        assert!(lint(
            Deviations::ALL,
            Side::Server,
            "q.in_range(1, 2)",
            DiagCode::QueryArity
        ));
        assert!(!lint(
            Deviations::NONE,
            Side::Server,
            "q.in_range(1, 2)",
            DiagCode::QueryArity
        ));
        assert!(lint(
            Deviations::ALL,
            Side::Server,
            "q.client_memory_tier",
            DiagCode::QueryClientOnly
        ));
        assert!(!lint(
            Deviations::NONE,
            Side::Server,
            "q.client_memory_tier",
            DiagCode::QueryClientOnly
        ));
        assert!(!lint(
            Deviations::ALL,
            Side::Client,
            "q.client_memory_tier",
            DiagCode::QueryClientOnly
        ));
    }

    fn double(cx: &mut QueryCx<'_, '_, NoHost>) -> Result {
        Ok(Value::Float(cx.arg_f32(0).unwrap_or(0.0) * 2.0))
    }

    fn a_string(_: &mut QueryCx<'_, '_, NoHost>) -> Result {
        Ok(Value::string("forty-two"))
    }

    /// The table's function runs only when every kind its declaration returns is one the
    /// expression's declaration returns; otherwise the call is worth the expression's default.
    #[test]
    fn a_table_of_another_catalogue_runs_only_what_the_expression_was_compiled_to_expect() {
        use molangx::catalog::{Arity, QueryDecl, QueryShape, ReturnType};
        let declaring = |returns: ReturnType| {
            let double = QueryDecl::new(
                "query.double",
                QueryShape {
                    args: Arity::exactly(1),
                    returns,
                    ..QueryShape::DEFAULT
                },
            )
            .unwrap();
            molangx::stdlib::queries(Side::Server)
                .extended([double])
                .unwrap()
        };
        let numbers = declaring(ReturnType::FLOAT);
        let strings = declaring(ReturnType::STRING);
        let (expr, _) = compile(
            "query.double(21) + 1",
            &CompileOptions::new(numbers, MolangVersion::LATEST),
        )
        .into_result()
        .unwrap();

        let mut env = NoHostEnv {
            queries: Some(QueryTable::new(&strings)),
            ..NoHostEnv::new()
        };
        table(&mut env).set("query.double", a_string).unwrap();
        assert_eq!(
            expr.eval(&mut env.cx()),
            Value::Float(1.0),
            "the default 0 of the number declaration, plus 1"
        );
        let mut env = NoHostEnv {
            queries: Some(QueryTable::new(&declaring(ReturnType::FLOAT))),
            ..NoHostEnv::new()
        };
        table(&mut env).set("query.double", double).unwrap();
        assert_eq!(
            expr.eval(&mut env.cx()),
            Value::Float(43.0),
            "an equal declaration in another catalogue runs"
        );
    }

    /// `query.life_time` redeclared with two arguments and a string result: the compiler checks
    /// calls against the new declaration and a table of the new catalogue runs it by name.
    #[test]
    fn an_overriding_declaration_replaces_a_built_in_one() {
        use molangx::catalog::{Arity, CatalogError, QueryDecl, QueryShape, ReturnType};
        let standard = molangx::stdlib::queries(Side::Client);
        let life_time = QueryDecl::new(
            query::LIFE_TIME,
            QueryShape {
                args: Arity::exactly(2),
                returns: ReturnType::STRING,
                ..QueryShape::DEFAULT
            },
        )
        .unwrap();
        let catalog = standard.overriding([life_time.clone()]).unwrap();
        assert!(catalog != *standard && catalog.len() == standard.len());
        assert_eq!(catalog.get(query::LIFE_TIME), Some(&life_time));
        assert_eq!(
            catalog.iter().position(|d| d.name() == query::LIFE_TIME),
            standard.iter().position(|d| d.name() == query::LIFE_TIME)
        );
        let unknown = QueryDecl::new("query.no_such", QueryShape::DEFAULT).unwrap();
        assert_eq!(
            standard.overriding([unknown]).err(),
            Some(CatalogError::Undeclared("query.no_such".into()))
        );

        let options = CompileOptions::new(catalog.clone(), MolangVersion::LATEST);
        let messages = |source: &str, options: &CompileOptions| {
            compile(source, options)
                .diagnostics()
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
        };
        assert_eq!(
            messages("q.life_time(1)", &options),
            ["query.life_time is registered with 2 arguments, 1 given (this crate's check)"]
        );
        assert_eq!(
            messages(
                "q.life_time",
                &CompileOptions::new(standard.clone(), MolangVersion::LATEST)
            ),
            Vec::<String>::new()
        );
        assert!(
            compile("q.life_time(1, 2) + 1", &options)
                .failure()
                .is_some(),
            "a string is no operand of arithmetic"
        );
        assert_eq!(
            compile(
                "q.life_time + 1",
                &CompileOptions::new(standard.clone(), MolangVersion::LATEST)
            )
            .failure(),
            None
        );

        let (expr, _) = compile("q.life_time('a', 'b') == 'ok'", &options)
            .into_result()
            .unwrap();
        let mut env = NoHostEnv {
            queries: Some(QueryTable::new(&catalog)),
            ..NoHostEnv::new()
        };
        table(&mut env)
            .set(query::LIFE_TIME, |cx| {
                Ok(Value::Hash(HashedStr::new(if cx.arg_count() == 2 {
                    "ok"
                } else {
                    "no"
                })))
            })
            .unwrap();
        assert_eq!(expr.eval(&mut env.cx()), Value::Float(1.0));
    }
}

mod runtime_messages {
    //! The run-time messages an evaluation emits, and what `->` switches.

    use molangx::catalog::Side;
    use molangx::compile::{CompileOptions, compile};
    use molangx::rng::{Xorshift128, sample};
    use molangx::stdlib::query;
    use molangx::version::MolangVersion;
    use molangx::vm::{
        Access, ContextMap, ContextName, Host, HostAccess, HostEnv, QueryCx, QueryError,
        QueryTable, Subjects, Value, VariableName, VariableStorage,
    };

    /// Every actor's subjects carry their own item, block and `this`, so a switch of any is
    /// visible.
    struct Farm;

    impl Host for Farm {
        type ActorRef = u32;
        type ItemRef = u32;
        type BlockRef = u32;
        type Access<'w> = World;
    }

    #[derive(Default)]
    struct World {
        alive: Vec<u32>,
        /// The subjects and a random sample of each `query.health` call, in order.
        seen: Vec<(Subjects<Farm>, f32)>,
    }

    impl HostAccess<Farm> for World {
        fn resolve_actor(&self, _from: &Subjects<Farm>, actor: u32) -> Option<u32> {
            self.alive.contains(&actor).then_some(actor)
        }

        fn subjects_of(&self, actor: u32) -> Subjects<Farm> {
            subjects_of(actor)
        }
    }

    fn subjects_of(actor: u32) -> Subjects<Farm> {
        Subjects {
            actor: Some(actor),
            item: Some(actor + 100),
            block: Some(actor + 200),
            world_gen: None,
            this: actor as f32 * 10.0,
        }
    }

    /// `query.health`: records its subjects and draws a sample from the evaluation's random source;
    /// its value is the subject actor's number.
    fn health(cx: &mut QueryCx<'_, '_, Farm>) -> Result<Value<Farm>, QueryError> {
        let subjects = cx.subjects();
        let sample = sample(cx.rng());
        cx.host().seen.push((subjects, sample));
        Ok(Value::Float(subjects.actor.map_or(0.0, |a| a as f32)))
    }

    /// `query.log`: its first argument.
    fn log(cx: &mut QueryCx<'_, '_, Farm>) -> Result<Value<Farm>, QueryError> {
        Ok(cx.arg(0).unwrap_or_default())
    }

    /// Actor 1 (the subject, `this` 1.5) and actor 2 (alive, `c.other`), each with a variable `x`:
    /// actor 1's private 1, actor 2's public 2 with its snapshot taken, and a public struct `pubs`.
    fn farm() -> (HostEnv<Farm, VariableStorage<Farm>>, World) {
        let mut variables = VariableStorage::new();
        variables
            .actor_mut(1)
            .set(VariableName::new("x"), Value::Float(1.0));
        let two = variables.actor_mut(2);
        two.set_public(VariableName::new("x"), Value::Float(2.0));
        two.set_public(
            VariableName::new("pubs"),
            Value::structure(molangx::vm::StructValue::from([("a", 1.0)])),
        );
        two.refresh_snapshots();
        assert_eq!(two.access(VariableName::new("x")), Some(Access::Public));
        let mut queries = QueryTable::new(molangx::stdlib::queries(Side::Server));
        queries.set(query::HEALTH, health).unwrap();
        queries.set(query::LOG, log).unwrap();
        let env = HostEnv::new(queries)
            .with_variables(variables)
            .with_context(ContextMap::from([(
                ContextName::new("other"),
                Value::Actor(2),
            )]));
        (
            env,
            World {
                alive: vec![1, 2],
                seen: Vec::new(),
            },
        )
    }

    fn eval(
        env: &mut HostEnv<Farm, VariableStorage<Farm>>,
        world: &mut World,
        source: &str,
    ) -> (Value<Farm>, Vec<String>) {
        let compiled = compile(source, &CompileOptions::server(MolangVersion::LATEST));
        assert_eq!(
            compiled.failure(),
            None,
            "{source:?}: {:?}",
            compiled.diagnostics()
        );
        let expr = compiled
            .expr()
            .cloned()
            .unwrap_or_else(|| panic!("{source:?} compiles"));
        let value = expr.eval(&mut env.cx(
            world,
            Subjects {
                this: 1.5,
                ..Subjects::actor(1)
            },
        ));
        (value, env.sink.take())
    }

    /// `->` switches the subjects (actor, item, block, `this`) for its right side and back
    /// afterwards. It does not switch what lives in the `EvalCx`: the random source, host access,
    /// temps, context, query table, sink and budgets.
    #[test]
    fn arrow_switches_the_subjects_only() {
        let (mut env, mut world) = farm();
        // The variable map (the right side of `->` is a `variable.` read or a query call).
        assert_eq!(eval(&mut env, &mut world, "v.x").0, Value::Float(1.0));
        assert_eq!(
            eval(&mut env, &mut world, "c.other->v.x").0,
            Value::Float(2.0)
        );
        assert_eq!(
            eval(&mut env, &mut world, "(c.other->v.x) * 10 + v.x").0,
            Value::Float(21.0)
        );

        // The subjects a query sees — actor, item, block and `this` — are the target's inside and
        // the caller's again after; one level (`world`) and one random source serve both sides.
        let (value, messages) = eval(
            &mut env,
            &mut world,
            "v.r = math.random(0, 1); return q.health * 100 + (c.other->q.health) * 10 + q.health;",
        );
        assert_eq!((value, messages), (Value::Float(121.0), Vec::new()));
        let caller = Subjects {
            this: 1.5,
            ..Subjects::actor(1)
        };
        let mut reference = Xorshift128::new();
        let first = sample(&mut reference);
        assert_eq!(
            env.variables
                .actor(1)
                .and_then(|m| m.get(VariableName::new("r"))),
            Some(&Value::Float(first))
        );
        let samples: Vec<f32> = (0..3).map(|_| sample(&mut reference)).collect();
        assert_eq!(
            world.seen,
            [
                (caller, samples[0]),
                (subjects_of(2), samples[1]),
                (caller, samples[2])
            ]
        );

        // Temps and context are the evaluation's: a temp written outside is read inside.
        assert_eq!(
            eval(
                &mut env,
                &mut world,
                "t.k = 5; return c.other->q.log(t.k * 10 + this);"
            )
            .0,
            Value::Float(70.0)
        );
        // A dead target switches nothing: the right side is skipped.
        world.alive.retain(|&a| a != 2);
        world.seen.clear();
        assert_eq!(
            eval(&mut env, &mut world, "(c.other->q.health) + 1").0,
            Value::Float(1.0)
        );
        assert!(world.seen.is_empty());
    }

    /// The end of the longer unknown-variable text logged inside `->`.
    const PUBLIC_SUFFIX: &str = " - are you trying to access a variable from a different mob that hasn't made its variable public in its resource definition?";

    /// The message carries the full canonical name, for each namespace and either spelling.
    #[test]
    fn unknown_variable_is_emitted() {
        let (mut env, mut world) = farm();
        for (source, name) in [
            ("v.missing_never_set_r16", "variable.missing_never_set_r16"),
            ("variable.Missing + 1", "variable.missing"),
            ("t.never", "temp.never"),
            ("c.never", "context.never"),
            ("v.x = 3; return v.never * v.x;", "variable.never"),
        ] {
            let (value, messages) = eval(&mut env, &mut world, source);
            assert_eq!(value, Value::ZERO, "{source}");
            assert_eq!(
                messages,
                [format!(
                    "Error: unhandled request for unknown variable '{name}'"
                )],
                "{source}"
            );
        }
        // Caught by `??`: nothing is emitted.
        assert_eq!(
            eval(&mut env, &mut world, "v.never ?? 4"),
            (Value::Float(4.0), Vec::new())
        );
    }

    /// A missing `temp.` / `context.` read or member inside `->` gets the longer text and the
    /// argument reads 0; an absent or private `variable.` there reads 0 without a message.
    #[test]
    fn unknown_variable_inside_an_arrow_is_emitted_with_the_public_text() {
        let (mut env, mut world) = farm();
        for (source, name) in [
            ("c.other->q.log(t.never)", "temp.never"),
            ("c.other->q.log(c.never)", "context.never"),
        ] {
            let (value, messages) = eval(&mut env, &mut world, source);
            assert_eq!(value, Value::ZERO, "{source}");
            assert_eq!(
                messages,
                [format!(
                    "Error: unhandled request for unknown variable '{name}'{PUBLIC_SUFFIX}"
                )],
                "{source}"
            );
        }
        let (value, messages) = eval(&mut env, &mut world, "c.other->q.log(v.pubs.missing)");
        assert_eq!(value, Value::ZERO);
        assert_eq!(
            messages,
            [
                "Error: unable to find member variable .missing".to_owned(),
                format!("Error: unhandled request for unknown variable '.missing'{PUBLIC_SUFFIX}")
            ]
        );
        assert_eq!(
            eval(&mut env, &mut world, "(c.other->v.absent) + 1"),
            (Value::Float(1.0), Vec::new())
        );
    }

    /// The member message names the read path's last member; without `??` the unknown-variable text
    /// and 0 follow, with `??` nothing more. A member read on a float is a missing member too.
    #[test]
    fn missing_member_is_emitted() {
        let (mut env, mut world) = farm();
        let (value, messages) = eval(&mut env, &mut world, "v.s.a = 1; return v.s.b;");
        assert_eq!(value, Value::ZERO);
        assert_eq!(
            messages,
            [
                "Error: unable to find member variable .b",
                "Error: unhandled request for unknown variable '.b'"
            ]
        );
        let (value, messages) = eval(&mut env, &mut world, "v.f = 1; return v.f.g ?? 3;");
        assert_eq!(
            (value, messages),
            (
                Value::Float(3.0),
                vec!["Error: unable to find member variable .g".to_owned()]
            )
        );
    }

    /// Covers dead, null, numeric and struct targets, nested arrows, and jumps or a missing read
    /// inside the right side.
    #[test]
    fn wrong_kinds_and_arrow_shapes_log_only_the_variable_texts() {
        let (mut env, mut world) = farm();
        env.context = ContextMap::from([
            (ContextName::new("other"), Value::Actor(2)),
            (ContextName::new("dead"), Value::Actor(9)),
            (ContextName::new("s"), Value::string("moo")),
        ]);
        let sources = [
            "v.f = 1; return v.f.g;",
            "c.s.x",
            "v.s = 'moo'; return v.s.x ?? 1;",
            "v.a = c.other; return v.a.x ?? 2;",
            "c.other + 1",
            "c.s * 2",
            "return (c.dead->v.x) + (c.s->v.x) + (c.other->v.pubs) + (v.q.z ?? 0);",
            "v.st.a = 1; return v.st->q.health;",
            "c.other->q.log(c.other->v.x)",
            "loop(3, { c.other->q.log(v.never ?? 1); t.i = (t.i ?? 0) + 1; t.i > 1 ? break; }); return t.i;",
            "loop(3, { t.r = c.other->q.log(t.never); continue; }); return t.r;",
            "return c.other->q.log(c.never, v.st.b);",
            "q.health + (c.other->q.health) + (c.dead->q.health)",
        ];
        let mut emitted = Vec::new();
        for source in sources {
            emitted.extend(eval(&mut env, &mut world, source).1);
        }
        assert!(
            !emitted.is_empty(),
            "the battery reaches the reachable texts"
        );
        for message in &emitted {
            assert!(
                message.starts_with("Error: unhandled request for unknown variable '")
                    || message.starts_with("Error: unable to find member variable "),
                "only the reachable evaluator texts: {message}"
            );
        }
    }
}

mod query_misuse {
    //! A host query's misuse text is logged once per session and the call yields the query's
    //! default. The texts are example host messages.

    use crate::common::compile_support::compile_expr;
    use crate::table;
    use molangx::catalog::Side;
    use molangx::compile::{CompileOptions, Deviations, Expr};
    use molangx::stdlib::query;
    use molangx::version::MolangVersion;
    use molangx::vm::{
        CollectSink, LogOnce, NoHost, NoHostEnv, QueryCx, QueryError, QueryTable, Value,
    };

    type Result = std::result::Result<Value<NoHost>, QueryError>;

    fn compiled(source: &str) -> Expr {
        let options = CompileOptions {
            deviations: Deviations::NONE,
            ..CompileOptions::client(MolangVersion::LATEST)
        };
        compile_expr(source, &options)
    }

    fn head_x_rotation(cx: &mut QueryCx<'_, '_, NoHost>) -> Result {
        if cx.arg_count() != 1 {
            return Err(cx.error("query.head_x_rotation takes one argument"));
        }
        Ok(Value::Float(cx.arg_f32(0).unwrap_or(0.0) + 100.0))
    }

    fn has_any_family(cx: &mut QueryCx<'_, '_, NoHost>) -> Result {
        if cx.arg_count() == 0 {
            return Err(cx.error("query.has_any_family takes one or more strings"));
        }
        for i in 0..cx.arg_count() {
            if cx.arg_hash(i).is_none() {
                return Err(cx.error(format_args!(
                    "argument {i} of query.has_any_family is not a string"
                )));
            }
        }
        Ok(Value::Float(1.0))
    }

    fn is_name_any(cx: &mut QueryCx<'_, '_, NoHost>) -> Result {
        for i in 0..cx.arg_count() {
            if cx.arg_hash(i).is_none() {
                return Err(cx.error(format_args!(
                    "argument {i} of query.is_name_any is not a string"
                )));
            }
        }
        Ok(Value::Float(1.0))
    }

    fn block_state(cx: &mut QueryCx<'_, '_, NoHost>) -> Result {
        if cx.arg_count() != 1 {
            return Err(cx.error(format_args!("{} takes one block state name", cx.name())));
        }
        Ok(Value::Float(1.0))
    }

    /// A text without the `Error:` prefix, from a query whose default is 1.0.
    fn armor_color_slot(cx: &mut QueryCx<'_, '_, NoHost>) -> Result {
        if cx.arg_count() != 2 {
            return Err(cx.error("query.armor_color_slot takes an armor slot and a colour channel"));
        }
        Ok(Value::Float(0.25))
    }

    fn in_range(cx: &mut QueryCx<'_, '_, NoHost>) -> Result {
        if cx.arg_count() != 3 {
            return Err(cx.error("query.in_range takes three numbers"));
        }
        let Some(x) = cx.arg_f32(0) else {
            return Err(cx.error("the first argument of query.in_range is not a number"));
        };
        let lo = cx.arg_f32(1).unwrap_or(0.0);
        let hi = cx.arg_f32(2).unwrap_or(0.0);
        Ok(Value::Float(if lo <= x && x <= hi { 1.0 } else { 0.0 }))
    }

    fn env() -> NoHostEnv {
        let mut env = NoHostEnv {
            queries: Some(QueryTable::new(molangx::stdlib::queries(Side::Server))),
            ..NoHostEnv::new()
        };
        table(&mut env)
            .set(query::HEAD_X_ROTATION, head_x_rotation)
            .unwrap();
        table(&mut env)
            .set(query::HAS_ANY_FAMILY, has_any_family)
            .unwrap();
        table(&mut env)
            .set(query::IS_NAME_ANY, is_name_any)
            .unwrap();
        table(&mut env)
            .set(query::BLOCK_STATE, block_state)
            .unwrap();
        table(&mut env)
            .set(query::ARMOR_COLOR_SLOT, armor_color_slot)
            .unwrap();
        table(&mut env).set(query::IN_RANGE, in_range).unwrap();
        env
    }

    fn eval(env: &mut NoHostEnv, log: &mut LogOnce<CollectSink>, source: &str) -> Value<NoHost> {
        let expr = compiled(source);
        let mut cx = env.cx();
        cx.sink = log;
        expr.eval(&mut cx)
    }

    fn default_of(name: &str) -> Value<NoHost> {
        Value::from(
            molangx::stdlib::queries(Side::Client)
                .get(name)
                .unwrap()
                .shape()
                .default_return,
        )
    }

    #[test]
    fn query_misuse_text_is_logged_once_and_the_call_yields_its_default() {
        let mut env = env();
        let mut log = LogOnce::new(CollectSink::new());

        // Correct use: real results, no log.
        assert_eq!(
            eval(&mut env, &mut log, "q.head_x_rotation(1)"),
            Value::Float(101.0)
        );
        assert_eq!(
            eval(&mut env, &mut log, "q.in_range(2, 1, 3)"),
            Value::Float(1.0)
        );
        assert_eq!(
            eval(&mut env, &mut log, "q.armor_color_slot(0, 1)"),
            Value::Float(0.25)
        );
        assert!(log.inner().is_empty());

        let misuse = [
            (
                query::HEAD_X_ROTATION,
                "q.head_x_rotation",
                "query.head_x_rotation takes one argument",
            ),
            (
                query::HAS_ANY_FAMILY,
                "q.has_any_family",
                "query.has_any_family takes one or more strings",
            ),
            (
                query::HAS_ANY_FAMILY,
                "q.has_any_family('monster', 2)",
                "argument 1 of query.has_any_family is not a string",
            ),
            (
                query::IS_NAME_ANY,
                "q.is_name_any(1)",
                "argument 0 of query.is_name_any is not a string",
            ),
            (
                query::BLOCK_STATE,
                "q.block_state",
                "query.block_state takes one block state name",
            ),
            (
                query::ARMOR_COLOR_SLOT,
                "q.armor_color_slot(0)",
                "query.armor_color_slot takes an armor slot and a colour channel",
            ),
            (
                query::IN_RANGE,
                "q.in_range(1, 2)",
                "query.in_range takes three numbers",
            ),
            (
                query::IN_RANGE,
                "q.in_range('a', 1, 2)",
                "the first argument of query.in_range is not a number",
            ),
        ];

        // Each misuse twice in one session: the value is the default both times, the text logged
        // once.
        for _ in 0..2 {
            for (id, source, _) in misuse {
                assert_eq!(eval(&mut env, &mut log, source), default_of(id), "{source}");
            }
        }
        let expected: Vec<String> = misuse
            .iter()
            .map(|(_, _, text)| (*text).to_owned())
            .collect();
        assert_eq!(log.inner().messages, expected);

        // The default is the query's own: `query.armor_color_slot` gives 1.0, not 0.
        assert_eq!(default_of(query::ARMOR_COLOR_SLOT), Value::Float(1.0));
        assert_eq!(default_of(query::IN_RANGE), Value::Float(0.0));

        // The failing call does not end the expression: the rest still runs.
        assert_eq!(
            eval(
                &mut env,
                &mut log,
                "v.x = q.armor_color_slot(0) + 2; return v.x;"
            ),
            Value::Float(3.0)
        );
        assert_eq!(log.inner().messages.len(), expected.len());

        // A new content-log session logs each text again.
        log.reset();
        assert_eq!(
            eval(&mut env, &mut log, "q.block_state"),
            default_of(query::BLOCK_STATE)
        );
        assert_eq!(
            log.inner().messages.last().map(String::as_str),
            Some(misuse[4].2)
        );
    }
}

mod names_a_host_makes {
    //! The names a host makes, read back by an evaluation.

    use molangx::hash::HashedStr;
    use molangx::vm::{NoHost, StructValue, Value};

    type V = Value<NoHost>;

    fn h(name: &str) -> HashedStr {
        HashedStr::new(name)
    }

    /// Member and variable keys are lowered and end at a NUL byte, so a member a host names `Speed`
    /// is the one Molang reads as `v.s.speed`.
    #[cfg(feature = "compiler")]
    #[test]
    fn names_a_host_makes_are_canonical() {
        use molangx::compile::{CompileOptions, compile};
        use molangx::version::MolangVersion;
        use molangx::vm::{ContextName, NoHostEnv, TempName, VariableName};

        assert_eq!(StructValue::<NoHost>::key("Speed"), h("speed"));
        assert_eq!(StructValue::<NoHost>::key("a\0b"), h("a"));
        assert_eq!(StructValue::<NoHost>::key(""), HashedStr::EMPTY);
        let speed = StructValue::<NoHost>::from([("Speed", 2.0)]);
        assert_eq!(speed.get(h("speed")), Some(&Value::Float(2.0)));
        let mut env = NoHostEnv::new();
        env.variables
            .set(VariableName::new("S"), Value::structure(speed));
        let read = |source: &str, env: &mut NoHostEnv| {
            compile(source, &CompileOptions::server(MolangVersion::LATEST))
                .expr()
                .cloned()
                .expect("compiles")
                .eval_f32(&mut env.cx())
        };
        assert_eq!(read("v.s.speed", &mut env), 2.0);
        assert_eq!(read("v.S.Speed", &mut env), 2.0);

        // Every name constructor stops at a NUL byte, as `HashedStr` does.
        assert_eq!(VariableName::new("x\0y"), VariableName::new("x"));
        assert_eq!(TempName::new("x\0").hashed(), h("temp.x\0z"));
        assert_eq!(ContextName::new("X\0Y"), ContextName::new("x"));
        assert_eq!(VariableName::parse("v.x\0y"), Some(VariableName::new("x")));

        // Conversions between the hash forms.
        let key = VariableName::new("x");
        assert_eq!(key.hashed(), h("variable.x"));
        assert_eq!(VariableName::from_raw_hash(key.hashed()), key);
        assert_eq!(VariableName::from_raw_hash(h("variable.x")), key);
        assert_eq!(V::Hash(h("moo")), V::string("moo"));
        assert_eq!(V::string("moo").as_hash(), Some(h("moo")));
        assert_eq!(V::from(h("moo")), V::string("moo"));
    }

    /// A name that is empty up to its NUL is no name, like `v.`.
    #[test]
    fn name_key_parse_reads_the_text_up_to_its_nul() {
        use molangx::vm::{AnyName, TempName, VariableName};

        assert_eq!(VariableName::parse("v.\0x"), None);
        assert_eq!(VariableName::parse("v."), None);
        assert_eq!(TempName::parse("temp.\0"), None);
        assert_eq!(AnyName::parse("v\0.x"), None);
        assert_eq!(
            VariableName::parse("V.Moo\0x"),
            Some(VariableName::new("moo"))
        );
    }
}

mod variable_store {
    use std::sync::Arc;

    use crate::common::compile_support::server_expr;
    use crate::common::host::{Actor, Env, TestHost};

    use molangx::rng::Xorshift128;

    use molangx::vm::{
        Access, CollectSink, ContextName, EvalCx, EvalLimits, NoContext, NoHost, NoHostEnv,
        NoVariables, NullSink, Subjects, Temps, Value, VariableMap, VariableName,
    };

    fn run(env: &mut Env, source: &str) -> (Value<TestHost>, Vec<String>) {
        let expr = server_expr(source);
        let mut rng = Xorshift128::new();
        let mut sink = CollectSink::new();
        let value = env.with_cx(&mut rng, &mut sink, |cx| expr.eval(cx));
        (value, sink.messages)
    }

    /// An actor array is stored without its unresolvable entries.
    #[test]
    fn stored_actors_are_ids() {
        let mut env = Env::new();
        env.world.alive.extend([1, 2]);
        env.context
            .set(ContextName::new("moo"), Value::Actor(Actor::Handle(1)));
        env.vars.set(
            VariableName::new("herd"),
            Value::ActorArray(Arc::new(vec![
                Actor::Handle(1),
                Actor::Handle(9),
                Actor::Handle(2),
            ])),
        );
        run(
            &mut env,
            "v.friend = c.moo; v.copy = v.herd; t.f = c.moo; v.tf = t.f;",
        );
        assert_eq!(
            env.vars.get(VariableName::new("friend")),
            Some(&Value::Actor(Actor::Id(1)))
        );
        assert_eq!(
            env.vars.get(VariableName::new("copy")),
            Some(&Value::ActorArray(Arc::new(vec![
                Actor::Id(1),
                Actor::Id(2)
            ])))
        );
        assert_eq!(
            env.vars.get(VariableName::new("tf")),
            Some(&Value::Actor(Actor::Id(1)))
        );
    }

    /// An absent, private or unsnapshotted variable reads 0 and evaluation continues; a null, dead
    /// or non-actor left side yields the post-op of 0.
    #[test]
    fn arrow() {
        let mut env = Env::new();
        env.world.alive.insert(1);
        env.context
            .set(ContextName::new("moo"), Value::Actor(Actor::Handle(1)));
        env.context
            .set(ContextName::new("dead"), Value::Actor(Actor::Handle(7)));
        env.context
            .set(ContextName::new("null"), Value::Actor(Actor::Handle(0)));
        env.context
            .set(ContextName::new("number"), Value::Float(1.0));
        env.vars
            .set_public(VariableName::new("pub"), Value::Float(1.5));
        env.vars.refresh_snapshots();
        env.vars
            .set_public(VariableName::new("pub"), Value::Float(2.5));
        env.vars
            .set(VariableName::new("private"), Value::Float(3.0));

        assert_eq!(run(&mut env, "c.moo->v.pub").0, Value::Float(1.5));
        assert_eq!(run(&mut env, "v.pub").0, Value::Float(2.5));
        let (value, messages) = run(&mut env, "(c.moo->v.private) + (c.moo->v.absent) + 1");
        assert_eq!((value, messages.is_empty()), (Value::Float(1.0), true));
        for target in ["dead", "null", "number"] {
            let (value, messages) = run(
                &mut env,
                &format!("v.r = c.{target}->v.pub + 1; return v.r * 10 + 2;"),
            );
            assert_eq!(value, Value::Float(12.0), "{target}");
            assert!(messages.is_empty(), "{target}");
        }
        // An actor id resolves through the current subject's actor only.
        env.vars
            .set(VariableName::new("friend"), Value::Actor(Actor::Id(1)));
        assert_eq!(run(&mut env, "v.friend->v.pub").0, Value::Float(0.0));
        let expr = server_expr("v.friend->v.pub");
        let mut rng = Xorshift128::new();
        let mut sink = NullSink;
        let (value, subjects) = env.with_cx(&mut rng, &mut sink, |cx| {
            cx.subjects = Subjects::actor(Actor::Handle(1));
            let value = expr.eval(cx);
            (value, cx.subjects)
        });
        assert_eq!(value, Value::Float(1.5));
        // Back in the caller's subjects.
        assert_eq!(subjects, Subjects::actor(Actor::Handle(1)));
    }

    #[test]
    fn missing_read_inside_arrow_has_the_public_text() {
        let mut env = Env::new();
        env.world.alive.insert(1);
        env.context
            .set(ContextName::new("moo"), Value::Actor(Actor::Handle(1)));
        let (value, messages) = run(&mut env, "c.moo->q.count(t.never_set)");
        assert_eq!(value, Value::Float(1.0));
        assert_eq!(
            messages,
            vec!["Error: unhandled request for unknown variable 'temp.never_set' - are you trying to access a variable from a different mob that hasn't made its variable public in its resource definition?".to_owned()]
        );
    }

    /// Without a map, writes are ignored and reads are missing.
    #[test]
    fn no_variable_map() {
        let expr = server_expr("v.x = 1; return v.x;");
        let mut host = NoHost;
        let mut rng = Xorshift128::new();
        let mut sink = CollectSink::new();
        let mut vars = NoVariables;
        let mut cx: EvalCx<'_, '_, NoHost> = EvalCx {
            subjects: Subjects::none(),
            host: &mut host,
            variables: &mut vars,
            context: &NoContext,
            queries: None,
            rng: &mut rng,
            sink: &mut sink,
            limits: EvalLimits::DEFAULT,
            temps: Temps::PerEvaluation,
        };
        assert_eq!(expr.eval(&mut cx), Value::ZERO);
        assert_eq!(
            sink.messages,
            vec!["Error: unhandled request for unknown variable 'variable.x'".to_owned()]
        );
    }

    #[test]
    fn assignment_keeps_access() {
        let mut env = NoHostEnv::new();
        env.variables
            .set_access(VariableName::new("x"), Access::Public);
        server_expr("v.x = 4;").eval(&mut env.cx());
        assert_eq!(
            env.variables.access(VariableName::new("x")),
            Some(Access::Public)
        );
        server_expr("v.y = 4;").eval(&mut env.cx());
        assert_eq!(
            env.variables.access(VariableName::new("y")),
            Some(Access::Private)
        );
    }

    #[test]
    fn per_actor_maps() {
        let mut map = VariableMap::<TestHost>::new();
        map.set(VariableName::new("x"), Value::Float(1.0));
        let mut env = Env::new();
        env.vars = map;
        assert_eq!(run(&mut env, "v.x + 1").0, Value::Float(2.0));
    }
}

mod process_rng {
    //! `ProcessRng`, one generator shared by every evaluation in the process.

    use std::sync::{Mutex, PoisonError};

    use crate::common::compile_support::{server_expr, server_expr_at};

    use molangx::rng::{Xorshift128, rand_core::Rng, sample};
    use molangx::vm::{NoHostEnv, ProcessRng};

    /// Both tests reset and draw from the one process-wide generator, so they take turns.
    static SERIAL: Mutex<()> = Mutex::new(());

    /// The default is a generator per evaluator.
    #[test]
    fn process_rng_is_one_sequence() {
        let _serial = SERIAL.lock().unwrap_or_else(PoisonError::into_inner);
        let expr = server_expr("math.random(0, 1)");
        ProcessRng::reset();
        let mut reference = Xorshift128::new();
        let mut env = NoHostEnv::new();
        for _ in 0..3 {
            let mut rng = ProcessRng;
            let mut cx = env.cx();
            cx.rng = &mut rng;
            assert_eq!(expr.eval_f32(&mut cx), sample(&mut reference));
        }
        // Two fresh per-evaluator generators repeat each other.
        assert_eq!(
            expr.eval_f32(&mut NoHostEnv::new().cx()),
            expr.eval_f32(&mut NoHostEnv::new().cx())
        );
    }

    /// One xorshift128 seeded 123456789, 362436069, 521288629, 88675123, sampled as
    /// `(w & 0x7fffffff) · 2^-31`.
    #[test]
    fn the_reference_random_source() {
        let _serial = SERIAL.lock().unwrap_or_else(PoisonError::into_inner);
        assert_eq!(
            Xorshift128::STANDARD_SEED,
            [123_456_789, 362_436_069, 521_288_629, 88_675_123]
        );
        let mut raw = Xorshift128::new();
        let first = raw.next_u32();
        assert_eq!(first, 3_701_687_786);
        assert_eq!(
            sample(&mut Xorshift128::new()),
            ((first & 0x7fff_ffff) as f32) * 2.0_f32.powi(-31)
        );
        // A sample can round to exactly 1.0.
        assert_eq!(
            sample(&mut Xorshift128::with_state([0, 1, 2, 0xffff_e000])),
            1.0
        );

        let expr = server_expr_at("math.random(0, 1)", 13);
        ProcessRng::reset();
        let mut reference = Xorshift128::new();
        for _ in 0..3 {
            let mut env = NoHostEnv::new();
            let mut rng = ProcessRng;
            let mut cx = env.cx();
            cx.rng = &mut rng;
            assert_eq!(expr.eval_f32(&mut cx), sample(&mut reference));
        }
    }
}

mod query_defaults {
    //! A failing query is logged and the call is worth the query's own default.

    use crate::common::compile_support::client_expr;
    use crate::table;
    use molangx::catalog::Side;

    use molangx::hash::HashedStr;
    use molangx::stdlib::query;

    use molangx::vm::{NoHost, NoHostEnv, QueryCx, QueryError, QueryTable, Value};

    fn failing(cx: &mut QueryCx<'_, '_, NoHost>) -> Result<Value<NoHost>, QueryError> {
        Err(cx.error("host failure"))
    }

    fn run(env: &mut NoHostEnv, source: &str) -> Value<NoHost> {
        client_expr(source).eval(&mut env.cx())
    }

    #[test]
    fn a_failing_query_falls_back_to_its_own_default() {
        let mut env = NoHostEnv {
            queries: Some(QueryTable::new(molangx::stdlib::queries(Side::Server))),
            ..NoHostEnv::new()
        };
        let cases: [(&str, &str, Value<NoHost>); 4] = [
            (query::IS_BABY, "q.is_baby", Value::Float(0.0)),
            (
                query::ARMOR_COLOR_SLOT,
                "q.armor_color_slot",
                Value::Float(1.0),
            ),
            (
                query::TIME_SINCE_LAST_VIBRATION_DETECTION,
                "q.time_since_last_vibration_detection",
                Value::Float(-1.0),
            ),
            (
                query::GET_EQUIPPED_ITEM_NAME,
                "q.get_equipped_item_name",
                Value::Hash(HashedStr::EMPTY),
            ),
        ];
        for (name, source, default) in cases {
            table(&mut env).set(name, failing).unwrap();
            assert_eq!(run(&mut env, source), default, "{source}");
            assert_eq!(env.sink.take(), ["host failure"], "{source}");
        }
    }

    #[test]
    fn the_expression_goes_on_after_a_failing_query() {
        let mut env = NoHostEnv {
            queries: Some(QueryTable::new(molangx::stdlib::queries(Side::Server))),
            ..NoHostEnv::new()
        };
        table(&mut env)
            .set(query::ARMOR_COLOR_SLOT, failing)
            .unwrap();
        table(&mut env).set(query::IS_BABY, failing).unwrap();
        assert_eq!(
            run(
                &mut env,
                "v.x = q.armor_color_slot * 10 + q.is_baby + 5; return v.x;"
            ),
            Value::Float(15.0)
        );
        assert_eq!(env.sink.take(), ["host failure", "host failure"]);
    }
}

mod public_variables {
    //! `->` reads the target's public snapshot, taken when the host calls `refresh_snapshots`, not
    //! the live value; a private variable is not visible.

    use crate::common::compile_support::server_expr;

    use molangx::vm::{
        ContextMap, ContextName, Host, HostAccess, HostEnv, Subjects, Value, VariableName,
        VariableStorage,
    };

    struct Farm;

    impl Host for Farm {
        type ActorRef = u32;
        type ItemRef = u16;
        type BlockRef = ();
        type Access<'w> = World;
    }

    struct World;

    impl HostAccess<Farm> for World {
        fn resolve_actor(&self, _from: &Subjects<Farm>, actor: u32) -> Option<u32> {
            Some(actor)
        }
    }

    type FarmEnv = HostEnv<Farm, VariableStorage<Farm>>;

    fn eval_as(
        env: &mut FarmEnv,
        actor: u32,
        other: u32,
        source: &str,
    ) -> (Value<Farm>, Vec<String>) {
        env.context
            .set(ContextName::new("other"), Value::Actor(other));
        let expr = server_expr(source);
        let value = expr.eval(&mut env.cx(&mut World, Subjects::actor(actor)));
        (value, env.sink.take())
    }

    fn hp(env: &mut FarmEnv, actor: u32) -> f32 {
        let (value, messages) = eval_as(env, actor, 0, "v.hp");
        assert!(messages.is_empty(), "{messages:?}");
        value.as_f32()
    }

    #[test]
    fn a_snapshot_is_what_the_other_actor_sees_until_the_next_tick() {
        let mut variables = VariableStorage::new();
        variables
            .actor_mut(1)
            .set_public(VariableName::new("hp"), Value::Float(10.0));
        variables
            .actor_mut(2)
            .set_public(VariableName::new("hp"), Value::Float(20.0));
        variables
            .actor_mut(1)
            .set(VariableName::new("secret"), Value::Float(7.0));
        let mut env: FarmEnv = HostEnv::default().with_variables(variables);

        // Before the first update nothing is published yet.
        assert_eq!(
            eval_as(&mut env, 1, 2, "c.other->v.hp"),
            (Value::ZERO, vec![])
        );

        env.variables.refresh_snapshots();
        assert_eq!(
            eval_as(&mut env, 1, 2, "c.other->v.hp"),
            (Value::Float(20.0), vec![])
        );
        assert_eq!(
            eval_as(&mut env, 2, 1, "c.other->v.hp"),
            (Value::Float(10.0), vec![])
        );
        // A private variable of the target reads 0, without a message.
        assert_eq!(
            eval_as(&mut env, 2, 1, "c.other->v.secret"),
            (Value::ZERO, vec![])
        );

        // Tick 2: actor 2 changes its own value; the snapshot lags behind.
        assert_eq!(
            eval_as(&mut env, 2, 1, "v.hp = 25;").1,
            Vec::<String>::new()
        );
        assert_eq!(hp(&mut env, 2), 25.0);
        assert_eq!(
            eval_as(&mut env, 1, 2, "c.other->v.hp"),
            (Value::Float(20.0), vec![])
        );
        assert_eq!(hp(&mut env, 1), 10.0);

        // The host takes the snapshots of tick 2.
        env.variables.refresh_snapshots();
        assert_eq!(
            eval_as(&mut env, 1, 2, "c.other->v.hp"),
            (Value::Float(25.0), vec![])
        );
        assert_eq!(
            eval_as(&mut env, 2, 1, "c.other->v.hp"),
            (Value::Float(10.0), vec![])
        );
    }

    #[test]
    fn every_actor_has_its_own_variables() {
        let mut env: FarmEnv = HostEnv::default()
            .with_variables(VariableStorage::new())
            .with_context(ContextMap::new());
        assert_eq!(
            eval_as(&mut env, 1, 0, "v.score = 3; return v.score;").0,
            Value::Float(3.0)
        );
        assert_eq!(
            eval_as(&mut env, 2, 0, "v.score = 9; return v.score;").0,
            Value::Float(9.0)
        );
        assert_eq!(
            eval_as(&mut env, 1, 0, "return v.score;").0,
            Value::Float(3.0)
        );
        assert_eq!(
            env.variables
                .actor(2)
                .and_then(|m| m.get(VariableName::new("score"))),
            Some(&Value::Float(9.0))
        );
        assert!(env.variables.actor(3).is_none());
    }
}

mod temps {
    //! `temp.*` with and without persistent temps (`HostEnv::temps`).

    use crate::common::compile_support::server_expr;

    use molangx::vm::{HostEnv, NoHost, Subjects, TempMap, TempName, Temps, Value};

    fn run(env: &mut HostEnv<NoHost>, source: &str) -> (Value<NoHost>, Vec<String>) {
        let expr = server_expr(source);
        let value = expr.eval(&mut env.cx(&mut NoHost, Subjects::none()));
        (value, env.sink.take())
    }

    #[test]
    fn a_temp_lives_within_one_evaluation() {
        for mut env in [
            HostEnv::<NoHost>::default(),
            HostEnv {
                temps: Temps::Kept(TempMap::new()),
                ..HostEnv::default()
            },
        ] {
            assert_eq!(
                run(&mut env, "t.k = 5; return t.k * 2;"),
                (Value::Float(10.0), vec![])
            );
        }
    }

    #[test]
    fn our_temps_start_empty_in_every_evaluation() {
        let mut env = HostEnv::<NoHost>::default();
        assert_eq!(run(&mut env, "t.k = 5;").1, Vec::<String>::new());
        assert_eq!(
            run(&mut env, "return t.k;"),
            (
                Value::ZERO,
                vec!["Error: unhandled request for unknown variable 'temp.k'".to_owned()]
            )
        );
        assert!(matches!(env.temps, Temps::PerEvaluation));
    }

    /// The host can read the one map, which is never cleared.
    #[test]
    fn persistent_temps_carry_over_across_evaluations() {
        let mut env = HostEnv::<NoHost> {
            temps: Temps::Kept(TempMap::new()),
            ..HostEnv::default()
        };
        assert_eq!(run(&mut env, "t.k = 5;").1, Vec::<String>::new());
        assert_eq!(
            run(&mut env, "t.k = t.k + 1; return t.k;"),
            (Value::Float(6.0), vec![])
        );
        assert_eq!(
            env.temps.kept().and_then(|t| t.get(TempName::new("k"))),
            Some(&Value::Float(6.0))
        );
    }
}

mod sinks {
    use crate::common::host::{Env, LevelSink};
    use molangx::compile::{CompileOptions, compile};
    use molangx::rng::Xorshift128;
    use molangx::version::MolangVersion;
    use molangx::vm::LogLevel;

    #[test]
    fn query_failures_and_evaluator_messages_arrive_in_order() {
        let mut env = Env::new();
        let compiled = compile(
            "q.client_memory_tier + v.never_set",
            &CompileOptions::server(MolangVersion::LATEST),
        );
        let expr = compiled.expr().cloned().expect("compiles");
        let mut sink = LevelSink::default();
        let mut rng = Xorshift128::new();
        assert_eq!(
            env.with_cx(&mut rng, &mut sink, |cx| expr.eval_f32(cx)),
            0.0
        );
        assert_eq!(
            sink.lines,
            [
                (
                    LogLevel::Error,
                    "Error: client_memory_tier isn't supported on the server (headless mode)."
                        .to_owned()
                ),
                (
                    LogLevel::Error,
                    "Error: unhandled request for unknown variable 'variable.never_set'".to_owned()
                ),
            ]
        );
    }

    /// A sink can tell the crate's budget messages from the language's.
    #[test]
    fn a_language_only_sink_drops_the_budget_messages() {
        let mut env = Env::new();
        env.limits.loop_iterations = Some(2);
        let compiled = compile(
            "t.n = 0; loop(5, {t.n = t.n + 1;}); return v.never_set;",
            &CompileOptions::server(MolangVersion::LATEST),
        );
        let expr = compiled.expr().cloned().expect("compiles");
        let mut everything = LevelSink::default();
        let mut language_only = LevelSink::language_only();
        let mut rng = Xorshift128::new();
        env.with_cx(&mut rng, &mut everything, |cx| expr.eval_f32(cx));
        env.with_cx(&mut rng, &mut language_only, |cx| expr.eval_f32(cx));
        assert_eq!(everything.lines.len(), 2, "{:?}", everything.lines);
        assert!(
            everything.lines[0]
                .1
                .starts_with("molangx: loop stopped after its budget of 2 iterations")
        );
        assert_eq!(
            language_only.lines,
            [(
                LogLevel::Error,
                "Error: unhandled request for unknown variable 'variable.never_set'".to_owned()
            )]
        );
    }
}

mod value_type {
    use std::sync::Arc;

    use molangx::hash::HashedStr;

    use molangx::catalog::DefaultReturn;

    use molangx::vm::{
        Host, HostAccess, NoHost, ResourceRef, StructValue, Subjects, Value, ValueKind,
    };

    /// Actors 100 and above are dead / null.
    #[derive(Debug)]
    struct TestHost;

    impl Host for TestHost {
        type ActorRef = u32;
        type ItemRef = u16;
        type BlockRef = (i32, i32, i32);
        type Access<'w> = TestHost;
    }

    impl HostAccess<TestHost> for TestHost {
        fn resolve_actor(&self, _from: &Subjects<TestHost>, actor: u32) -> Option<u32> {
            (actor < 100).then_some(actor)
        }
    }

    type V = Value<TestHost>;

    fn alive(actor: u32) -> Option<u32> {
        (actor < 100).then_some(actor)
    }

    fn h(name: &str) -> HashedStr {
        HashedStr::new(name)
    }

    #[test]
    fn value_is_sixteen_bytes_with_unit_handles() {
        assert_eq!(size_of::<Value<NoHost>>(), 16);
        assert_eq!(size_of::<V>(), 16);
        let kinds = [
            (V::Float(1.0).kind(), ValueKind::Float),
            (V::Hash(HashedStr::from_u64(1)).kind(), ValueKind::Hash),
            (V::Actor(1).kind(), ValueKind::Actor),
            (V::Item(1).kind(), ValueKind::Item),
            (V::actor_array([1, 2]).kind(), ValueKind::ActorArray),
            (V::structure(StructValue::new()).kind(), ValueKind::Struct),
            (V::identity_matrix().kind(), ValueKind::Matrix),
            (
                V::Resource(ResourceRef::new("texture.default")).kind(),
                ValueKind::Resource,
            ),
        ];
        for (got, want) in kinds {
            assert_eq!(got, want);
        }
    }

    #[test]
    fn value_is_send_and_sync() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<Value<NoHost>>();
        assert_send_sync::<V>();
        assert_send_sync::<StructValue<TestHost>>();
    }

    /// A string value is its FNV-1 hash; the text is not kept.
    #[test]
    fn string_values_are_hashes() {
        assert_eq!(
            V::string("a"),
            V::Hash(HashedStr::from_u64(12_638_153_115_695_167_422))
        );
        assert_eq!(V::string(""), V::Hash(HashedStr::from_u64(0)));
        assert_eq!(V::string("moo").as_hash(), Some(h("moo")));
        assert_ne!(V::string("abc"), V::string("ABC"));
        assert_eq!(V::from(h("moo")), V::string("moo"));
    }

    /// Every other non-float kind reads 0.0.
    #[test]
    fn as_f32_reinterprets_hash_low_bits() {
        assert_eq!(V::Float(2.5).as_f32(), 2.5);
        assert_eq!(
            V::Hash(HashedStr::from_u64(0xdead_beef_3fc0_0000)).as_f32(),
            1.5
        );
        let hash = h("a").as_u64();
        assert_eq!(V::string("a").as_f32().to_bits(), hash as u32);
        assert_eq!(V::Actor(3).as_f32(), 0.0);
        assert_eq!(V::Item(3).as_f32(), 0.0);
        assert_eq!(V::actor_array([1]).as_f32(), 0.0);
        assert_eq!(V::structure(StructValue::xy(1.0, 2.0)).as_f32(), 0.0);
        assert_eq!(V::identity_matrix().as_f32(), 0.0);
        assert_eq!(V::Resource(ResourceRef::new("geometry.x")).as_f32(), 0.0);
    }

    /// Truthy is `x != 0.0`: NaN is true, −0.0 false.
    #[test]
    fn truthiness() {
        assert!(V::Float(1.0).truthy());
        assert!(V::Float(0.000_000_1).truthy());
        assert!(V::Float(f32::NAN).truthy());
        assert!(!V::Float(0.0).truthy());
        assert!(!V::Float(-0.0).truthy());
        // The empty string hashes to 0: its float view is 0.0.
        assert!(!V::string("").truthy());
        assert!(!V::Actor(1).truthy());
        assert_eq!(V::bool(true), V::Float(1.0));
        assert_eq!(V::from(false), V::Float(0.0));
    }

    #[test]
    fn string_equality_compares_hashes() {
        assert!(V::string("moo").molang_eq(&V::string("moo"), alive));
        assert!(!V::string("moo").molang_eq(&V::string("rabbit"), alive));
        // Hashes that agree in the low 32 bits only are different strings.
        assert!(
            !V::Hash(HashedStr::from_u64(0x1_0000_0001))
                .molang_eq(&V::Hash(HashedStr::from_u64(0x2_0000_0001)), alive)
        );
    }

    /// The left payload is read as the right operand's kind; kinds without payload bits are
    /// unequal.
    #[test]
    fn equality_dispatches_on_the_right_operand() {
        assert!(V::Float(1.5).molang_eq(&V::Float(1.5), alive));
        assert!(V::Float(0.0).molang_eq(&V::Float(-0.0), alive));
        assert!(!V::Float(f32::NAN).molang_eq(&V::Float(f32::NAN), alive));
        // Right a float: the left hash's low 32 bits.
        assert!(V::Hash(HashedStr::from_u64(0x3fc0_0000)).molang_eq(&V::Float(1.5), alive));
        assert!(
            V::Hash(HashedStr::from_u64(0xffff_ffff_3fc0_0000)).molang_eq(&V::Float(1.5), alive)
        );
        assert!(V::string("").molang_eq(&V::Float(0.0), alive));
        assert!(V::string("").molang_eq(&V::Float(-0.0), alive));
        // Right a string: the left float's bits, zero-extended, against the 64-bit hash.
        assert!(V::Float(0.0).molang_eq(&V::string(""), alive));
        assert!(!V::Float(-0.0).molang_eq(&V::string(""), alive));
        assert!(V::Float(1.5).molang_eq(&V::Hash(HashedStr::from_u64(0x3fc0_0000)), alive));
        assert!(!V::Float(1.5).molang_eq(&V::Hash(HashedStr::from_u64(0x1_3fc0_0000)), alive));
        let a = V::string("a");
        assert!(a.molang_eq(&V::Float(a.as_f32()), alive));
        assert!(!V::Float(a.as_f32()).molang_eq(&a, alive));
        // No payload bits: unequal.
        assert!(!V::Float(0.0).molang_eq(&V::Actor(1), alive));
        assert!(!V::Actor(1).molang_eq(&V::Float(0.0), alive));
        // Any other kind on the right is unequal, even to itself.
        let item = V::Item(7);
        assert!(!item.molang_eq(&item, alive));
        let s = V::structure(StructValue::xy(1.0, 2.0));
        assert!(!s.molang_eq(&s, alive));
        let array = V::actor_array([1, 2]);
        assert!(!array.molang_eq(&array, alive));
        let m = V::identity_matrix();
        assert!(!m.molang_eq(&m, alive));
    }

    /// An unresolvable actor equals nothing.
    #[test]
    fn actor_equality_is_resolved_identity() {
        assert!(V::Actor(1).molang_eq(&V::Actor(1), alive));
        assert!(!V::Actor(1).molang_eq(&V::Actor(2), alive));
        assert!(!V::Actor(100).molang_eq(&V::Actor(100), alive));
        // A host that maps an id and a pointer to the same actor makes them equal.
        assert!(V::Actor(1).molang_eq(&V::Actor(1001), |a| Some(a % 1000)));
    }

    /// Structural (Rust) equality is not the Molang `==`: it compares kind and payload.
    #[test]
    fn structural_equality() {
        assert_eq!(V::Item(7), V::Item(7));
        assert_ne!(V::Float(1.0), V::Hash(HashedStr::from_u64(1)));
        assert_ne!(V::Float(f32::NAN), V::Float(f32::NAN));
        assert_eq!(V::actor_array([1, 2]), V::actor_array([1, 2]));
        assert_ne!(V::actor_array([1, 2]), V::actor_array([2, 1]));
        assert_eq!(V::default(), V::ZERO);
    }

    /// A clone shares the `Arc` and is equal without a walk, even with a NaN member; two structs
    /// built apart with the same NaN are not.
    #[test]
    fn a_shared_struct_is_equal_to_itself_without_a_walk() {
        let a = V::structure(StructValue::from([("x", f32::NAN)]));
        let shared = a.clone();
        assert_eq!(a, shared);
        // Nested: the same inner struct under two parents.
        let outer_a = V::structure(StructValue::from([("in", a.clone())]));
        let outer_b = V::structure(StructValue::from([("in", shared)]));
        assert_eq!(outer_a, outer_b);
        // Built apart, the NaN members are unequal to each other.
        assert_ne!(a, V::structure(StructValue::from([("x", f32::NAN)])));
        assert_ne!(
            outer_a,
            V::structure(StructValue::from([(
                "in",
                V::structure(StructValue::from([("x", f32::NAN)]))
            )]))
        );
    }

    #[test]
    fn default_return_values() {
        assert_eq!(V::from(DefaultReturn::Float0), V::Float(0.0));
        assert_eq!(V::from(DefaultReturn::Float1), V::Float(1.0));
        assert_eq!(V::from(DefaultReturn::FloatNeg1), V::Float(-1.0));
        assert_eq!(V::from(DefaultReturn::EmptyString), V::string(""));
        assert_eq!(
            V::from(DefaultReturn::EmptyActorArray).as_actor_array(),
            Some(&[][..])
        );
        let rgba = V::from(DefaultReturn::StructRgba0);
        for name in ["r", "g", "b", "a"] {
            assert_eq!(rgba.member(h(name)), Some(&V::Float(0.0)));
        }
        // The float view agrees with the metadata's own.
        for d in [
            DefaultReturn::Float0,
            DefaultReturn::Float1,
            DefaultReturn::FloatNeg1,
            DefaultReturn::EmptyString,
            DefaultReturn::EmptyActorArray,
            DefaultReturn::StructRgba0,
        ] {
            assert_eq!(V::from(d).as_f32(), d.as_f32());
        }
    }

    #[test]
    fn member_lookup() {
        let v = V::structure(StructValue::xyz(1.0, 2.0, 3.0));
        assert_eq!(v.member(h("y")), Some(&V::Float(2.0)));
        assert_eq!(v.member(h("w")), None);
        // A member read on a non-struct finds nothing.
        assert_eq!(V::Float(1.0).member(h("x")), None);
        let trs = V::structure(StructValue::trs(
            [1.0, 2.0, 3.0],
            [4.0, 5.0, 6.0],
            [7.0, 8.0, 9.0],
        ));
        assert_eq!(trs.member_path(&[h("r"), h("x")]), Some(&V::Float(4.0)));
        assert_eq!(trs.member_path(&[h("s"), h("z")]), Some(&V::Float(9.0)));
        assert_eq!(trs.member_path(&[h("r"), h("x"), h("deeper")]), None);
        assert_eq!(trs.member_path(&[]), Some(&trs));
        let aabb = StructValue::<TestHost>::min_and_max([0.0, 1.0, 2.0], [3.0, 4.0, 5.0]);
        assert_eq!(
            aabb.get(h("max")).and_then(|m| m.member(h("y"))),
            Some(&V::Float(4.0))
        );
    }

    #[test]
    fn duplicate_member_names_are_an_error() {
        let mut s = StructValue::<TestHost>::new();
        assert!(s.add(h("x"), V::Float(1.0)).is_ok());
        let err = s.add(h("x"), V::Float(2.0)).unwrap_err();
        assert_eq!(err.name, h("x"));
        assert!(
            err.to_string()
                .starts_with("molangx: a struct already has a member named '")
        );
        assert!(
            err.to_string()
                .ends_with(&format!("{:#018x}'", h("x").as_u64()))
        );
        assert_eq!(s.get(h("x")), Some(&V::Float(1.0)));
        assert_eq!(s.len(), 1);
        // An assignment replaces instead.
        s.set(h("x"), V::Float(3.0));
        assert_eq!(s.get(h("x")), Some(&V::Float(3.0)));
        assert_eq!(s.len(), 1);
    }

    #[test]
    fn struct_value_equality_is_order_independent() {
        let a = StructValue::<TestHost>::from([("x", 1.0), ("y", 2.0)]);
        let b = StructValue::<TestHost>::from([("y", 2.0), ("x", 1.0)]);
        assert_eq!(a, b);
        assert_ne!(a, StructValue::from([("x", 1.0)]));
        assert_ne!(a, StructValue::from([("x", 1.0), ("y", 3.0)]));
        assert_eq!(StructValue::<TestHost>::rgb(1.0, 2.0, 3.0).iter().len(), 3);
        assert!(StructValue::<TestHost>::default().is_empty());
    }

    #[test]
    fn member_writes_create_intermediate_structs() {
        let mut v = V::ZERO;
        v.set_member_path(&[h("a"), h("b"), h("c")], V::Float(1.0));
        assert_eq!(
            v.member_path(&[h("a"), h("b"), h("c")]),
            Some(&V::Float(1.0))
        );
        // A second member next to the first keeps both.
        v.set_member_path(&[h("a"), h("d")], V::Float(2.0));
        assert_eq!(
            v.member_path(&[h("a"), h("b"), h("c")]),
            Some(&V::Float(1.0))
        );
        assert_eq!(v.member_path(&[h("a"), h("d")]), Some(&V::Float(2.0)));
        // `v.x.x = 1; v.x.y = 2;`
        let mut x = V::ZERO;
        x.set_member_path(&[h("x")], V::Float(1.0));
        x.set_member_path(&[h("y")], V::Float(2.0));
        assert_eq!(x, V::structure(StructValue::xy(1.0, 2.0)));
        // A non-struct on the way is replaced by a struct.
        let mut y = V::structure(StructValue::from([("a", 5.0)]));
        y.set_member_path(&[h("a"), h("b")], V::Float(6.0));
        assert_eq!(y.member_path(&[h("a"), h("b")]), Some(&V::Float(6.0)));
        // An empty path replaces the value itself.
        y.set_member_path(&[], V::Float(7.0));
        assert_eq!(y, V::Float(7.0));
    }

    #[test]
    fn struct_copies_share_until_written() {
        let original = V::structure(StructValue::xy(1.0, 2.0));
        let mut copy = original.clone();
        let (V::Struct(a), V::Struct(b)) = (&original, &copy) else {
            panic!("structs")
        };
        assert!(Arc::ptr_eq(a, b));
        copy.set_member_path(&[h("x")], V::Float(9.0));
        assert_eq!(original.member(h("x")), Some(&V::Float(1.0)));
        assert_eq!(copy.member(h("x")), Some(&V::Float(9.0)));
        assert_eq!(copy.member(h("y")), Some(&V::Float(2.0)));
    }

    /// The conversion applied when a value is stored.
    #[test]
    fn map_actors_converts_actors_and_arrays() {
        let to_id = |a: u32| alive(a).map(|a| a + 1000);
        assert_eq!(V::Actor(1).map_actors(to_id), V::Actor(1001));
        // A single unresolvable actor is kept as it is.
        assert_eq!(V::Actor(100).map_actors(to_id), V::Actor(100));
        assert_eq!(
            V::actor_array([1, 100, 2, 200, 3]).map_actors(to_id),
            V::actor_array([1001, 1002, 1003])
        );
        assert_eq!(V::Float(1.0).map_actors(to_id), V::Float(1.0));
        assert_eq!(V::Item(5).map_actors(to_id), V::Item(5));
    }

    #[test]
    fn typed_accessors() {
        assert_eq!(V::Actor(4).as_actor(), Some(4));
        assert_eq!(V::Float(4.0).as_actor(), None);
        assert_eq!(V::Item(4).as_item(), Some(4));
        assert_eq!(V::Hash(HashedStr::from_u64(4)).as_item(), None);
        assert_eq!(V::actor_array([4, 5]).as_actor_array(), Some(&[4, 5][..]));
        assert_eq!(V::Float(4.0).as_actor_array(), None);
        assert_eq!(V::Float(4.0).as_hash(), None);
        assert_eq!(V::Float(4.0).as_float(), Some(4.0));
        assert_eq!(V::Hash(HashedStr::from_u64(4)).as_float(), None);
        assert_eq!(
            ResourceRef::new("texture.default").hashed(),
            h("texture.default")
        );
        assert_eq!(
            ResourceRef::from_raw_hash(h("geometry.x")),
            ResourceRef::new("geometry.x")
        );
        assert_eq!(ValueKind::ActorArray.to_string(), "actor array");
        assert!(format!("{:?}", V::string("a")).starts_with("Hash(0x"));
    }
}

mod variable_maps {
    use molangx::hash::HashedStr;

    use molangx::rng::{FixedRng, Xorshift128, sample};

    use molangx::vm::{
        Access, AnyName, ContextMap, ContextName, ContextProvider, EvalCx, EvalLimits, Host,
        HostAccess, NoContext, NoHost, NoHostEnv, NoVariables, NullSink, StructValue, Subjects,
        TempName, Temps, Value, VariableMap, VariableName, VariableStorage, VariableStore,
        WorldGenPos,
    };

    /// A host that tells a direct actor reference from an actor id.
    #[derive(Debug)]
    struct TestHost;

    #[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
    enum Actor {
        /// A direct handle of this test host; `Handle(0)` is the null handle.
        Handle(u32),
        Id(u32),
    }

    impl Host for TestHost {
        type ActorRef = Actor;
        type ItemRef = u16;
        type BlockRef = ();
        type Access<'w> = World<'w>;
    }

    struct World<'w> {
        alive: &'w [u32],
    }

    impl HostAccess<TestHost> for World<'_> {
        fn resolve_actor(&self, from: &Subjects<TestHost>, actor: Actor) -> Option<Actor> {
            match actor {
                // A pointer is used directly; only null fails.
                Actor::Handle(0) => None,
                Actor::Handle(n) => Some(Actor::Handle(n)),
                // An id is resolved through the current subject's actor: none, no resolution.
                Actor::Id(n) => {
                    (from.actor.is_some() && self.alive.contains(&n)).then_some(Actor::Handle(n))
                }
            }
        }

        fn subjects_of(&self, actor: Actor) -> Subjects<TestHost> {
            Subjects {
                this: 9.0,
                ..Subjects::actor(actor)
            }
        }

        fn stored_actor(&self, actor: Actor) -> Actor {
            match actor {
                Actor::Handle(n) | Actor::Id(n) => Actor::Id(n),
            }
        }
    }

    type V = Value<TestHost>;

    const BAA: VariableName = VariableName::new("baa");

    fn with_cx<R>(
        subjects: Subjects<TestHost>,
        vars: &mut dyn VariableStore<TestHost>,
        ctx: &dyn ContextProvider<TestHost>,
        f: impl FnOnce(&mut EvalCx<'_, '_, TestHost>) -> R,
    ) -> R {
        let mut world = World { alive: &[1, 2, 3] };
        let mut rng = FixedRng::HALF;
        let mut sink = NullSink;
        let mut cx = EvalCx {
            subjects,
            host: &mut world,
            variables: vars,
            context: ctx,
            queries: None,
            rng: &mut rng,
            sink: &mut sink,
            limits: EvalLimits::default(),
            temps: Temps::PerEvaluation,
        };
        f(&mut cx)
    }

    #[test]
    fn name_key_is_the_hash_of_the_canonical_name() {
        assert_eq!(
            VariableName::new("moo").hashed(),
            HashedStr::new("variable.moo")
        );
        assert_eq!(TempName::new("i").hashed(), HashedStr::new("temp.i"));
        assert_eq!(
            ContextName::new("other").hashed(),
            HashedStr::new("context.other")
        );
        assert_eq!(
            VariableName::new("moo").hashed(),
            HashedStr::new("variable.moo")
        );
        assert_eq!(
            TempName::from_raw_hash(HashedStr::new("temp.i")),
            TempName::new("i")
        );
        // One key scheme, three namespaces: the same short name is three different hashes.
        assert_ne!(VariableName::new("x").hashed(), TempName::new("x").hashed());
        assert_ne!(
            VariableName::new("x").hashed(),
            ContextName::new("x").hashed()
        );
        assert_ne!(TempName::new("x").hashed(), ContextName::new("x").hashed());
    }

    #[test]
    fn name_key_aliases_and_case() {
        assert_eq!(VariableName::parse("v.baa"), Some(VariableName::new("baa")));
        assert_eq!(
            VariableName::parse("variable.baa"),
            Some(VariableName::new("baa"))
        );
        assert_eq!(VariableName::parse("V.Baa"), Some(VariableName::new("baa")));
        assert_eq!(
            VariableName::parse("Variable.BAA"),
            Some(VariableName::new("baa"))
        );
        assert_eq!(TempName::parse("t.x"), Some(TempName::new("x")));
        assert_eq!(TempName::parse("temp.x"), Some(TempName::new("x")));
        assert_eq!(ContextName::parse("c.moo"), Some(ContextName::new("moo")));
        assert_eq!(
            ContextName::parse("context.moo"),
            Some(ContextName::new("moo"))
        );
        assert_eq!(VariableName::new("BAA"), VariableName::new("baa"));
        // Struct variables keep their dots after the namespace.
        assert_eq!(VariableName::parse("v.a.b"), Some(VariableName::new("a.b")));
        for bad in [
            "",
            "x",
            "v.",
            "variable.",
            "q.x",
            "query.x",
            "math.pi",
            ".x",
        ] {
            assert_eq!(AnyName::parse(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn variable_map_reads_and_writes() {
        let mut map = VariableMap::<TestHost>::new();
        assert!(map.is_empty());
        assert_eq!(map.get(BAA), None);
        map.set(BAA, V::Float(1.0));
        map.set(BAA, V::Float(2.0));
        assert_eq!(map.get(BAA), Some(&V::Float(2.0)));
        assert_eq!(map.len(), 1);
        assert_eq!(map.iter().collect::<Vec<_>>(), vec![(BAA, &V::Float(2.0))]);
        // Member writes go through the value.
        map.set(
            VariableName::new("s"),
            V::structure(StructValue::xy(1.0, 2.0)),
        );
        map.get_mut(VariableName::new("s"))
            .unwrap()
            .set_member_path(&[HashedStr::new("x")], V::Float(5.0));
        assert_eq!(
            map.get(VariableName::new("s"))
                .unwrap()
                .member(HashedStr::new("x")),
            Some(&V::Float(5.0))
        );
        assert_eq!(map.remove(BAA), Some(V::Float(2.0)));
        assert_eq!(map.get(BAA), None);
        map.clear();
        assert!(map.is_empty());
    }

    /// A new slot is private, an existing one keeps its setting.
    #[test]
    fn writes_never_change_access() {
        let mut map = VariableMap::<TestHost>::new();
        map.set(BAA, V::Float(1.0));
        assert_eq!(map.access(BAA), Some(Access::Private));
        assert!(!map.any_public());

        // The host declares the name public, even before the variable has a value.
        let moo = VariableName::new("moo");
        map.set_access(moo, Access::Public);
        assert_eq!(map.get(moo), None);
        assert_eq!(map.len(), 1);
        map.set(moo, V::Float(3.0));
        assert_eq!(map.access(moo), Some(Access::Public));
        assert!(map.any_public());

        // A write through the store interface keeps the access of an existing slot.
        VariableStore::set(&mut map, Actor::Handle(1), moo, V::Float(4.0));
        assert_eq!(map.access(moo), Some(Access::Public));
        assert_eq!(map.get(moo), Some(&V::Float(4.0)));
        assert_eq!(map.access(VariableName::new("unknown")), None);
    }

    /// The owner reads the latest value, another entity the snapshot of the last refresh.
    #[test]
    fn public_snapshot_needs_an_update() {
        let mut map = VariableMap::<TestHost>::new();
        map.set_public(BAA, V::Float(1.23));
        // Public but not yet snapshotted reads as nothing through `->`.
        assert_eq!(map.get_public(BAA), None);
        map.refresh_snapshots();
        map.set_public(BAA, V::Float(2.34));
        assert_eq!(map.get_public(BAA), Some(&V::Float(1.23)));
        assert_eq!(map.get(BAA), Some(&V::Float(2.34)));
        map.refresh_snapshots();
        assert_eq!(map.get_public(BAA), Some(&V::Float(2.34)));
        assert_eq!(map.get(BAA), Some(&V::Float(2.34)));
    }

    /// A private or absent variable reads as nothing through `->` (the evaluator then uses 0.0).
    #[test]
    fn private_and_absent_variables_have_no_public_value() {
        let mut map = VariableMap::<TestHost>::new();
        map.set(BAA, V::Float(1.0));
        map.refresh_snapshots();
        assert_eq!(map.get_public(BAA), None);
        assert_eq!(
            map.get_public(VariableName::new("this_var_does_not_exist_yet")),
            None
        );
        // Making a public variable private drops its snapshot.
        map.set_access(BAA, Access::Public);
        map.refresh_snapshots();
        assert_eq!(map.get_public(BAA), Some(&V::Float(1.0)));
        map.set_access(BAA, Access::Private);
        assert_eq!(map.get_public(BAA), None);
        map.set_access(BAA, Access::Public);
        assert_eq!(map.get_public(BAA), None);
    }

    #[test]
    fn variable_map_equality_is_order_independent() {
        let names = [
            VariableName::new("a"),
            VariableName::new("b"),
            VariableName::new("c"),
        ];
        let mut forward = VariableMap::<TestHost>::new();
        let mut backward = VariableMap::<TestHost>::new();
        for (i, name) in names.iter().enumerate() {
            forward.set(*name, V::Float(i as f32));
        }
        for (i, name) in names.iter().enumerate().rev() {
            backward.set(*name, V::Float(i as f32));
        }
        assert_eq!(forward, backward);
        backward.set(names[0], V::Float(9.0));
        assert_ne!(forward, backward);
        assert_eq!(forward.clone(), forward);
        // Access is part of a variable.
        let mut public = forward.clone();
        public.set_access(names[0], Access::Public);
        assert_ne!(forward, public);
    }

    /// `get` is the owner's view, `get_public` the `->` view.
    #[test]
    fn variable_storage_keeps_one_map_per_actor() {
        let mut store = VariableStorage::<TestHost>::new();
        let (cow, pig) = (Actor::Id(1), Actor::Id(2));
        store.set(cow, BAA, V::Float(1.0));
        store.set(pig, BAA, V::Float(2.0));
        assert_eq!(store.get(cow, BAA), Some(&V::Float(1.0)));
        assert_eq!(store.get(pig, BAA), Some(&V::Float(2.0)));
        assert_eq!(store.get(Actor::Id(3), BAA), None);
        assert_eq!(store.get_public(cow, BAA), None);

        store.actor_mut(cow).set_access(BAA, Access::Public);
        store.refresh_snapshots();
        store.set(cow, BAA, V::Float(5.0));
        assert_eq!(store.get_public(cow, BAA), Some(&V::Float(1.0)));
        assert_eq!(store.get(cow, BAA), Some(&V::Float(5.0)));
        assert_eq!(store.get_public(pig, BAA), None);

        assert!(store.remove_actor(pig).is_some());
        assert_eq!(store.get(pig, BAA), None);
        assert!(store.actor(pig).is_none());
    }

    #[test]
    fn no_variable_map_means_writes_are_ignored_and_reads_are_missing() {
        let mut store = VariableStorage::<TestHost>::new();
        with_cx(Subjects::none(), &mut store, &NoContext, |cx| {
            cx.set_variable(BAA, V::Float(1.0));
            assert_eq!(cx.variable(BAA), None);
        });
        assert!(store.local().is_none());

        let mut none = NoVariables;
        with_cx(
            Subjects::actor(Actor::Handle(1)),
            &mut none,
            &NoContext,
            |cx| {
                cx.set_variable(BAA, V::Float(1.0));
                assert_eq!(cx.variable(BAA), None);
            },
        );
    }

    #[test]
    fn detached_map_serves_actorless_subjects() {
        let mut store = VariableStorage::<TestHost>::with_local();
        store
            .local_mut()
            .set(VariableName::new("worldx"), V::Float(16.0));
        with_cx(
            Subjects::world_gen(WorldGenPos::new(16, 64, -32)),
            &mut store,
            &NoContext,
            |cx| {
                assert_eq!(
                    cx.variable(VariableName::new("worldx")),
                    Some(&V::Float(16.0))
                );
                cx.set_variable(VariableName::new("count"), V::Float(1.0));
                assert_eq!(
                    cx.variable(VariableName::new("count")),
                    Some(&V::Float(1.0))
                );
            },
        );
        // The actor maps are untouched.
        assert!(store.actor(Actor::Id(1)).is_none());
        assert_eq!(store.local().unwrap().len(), 2);
        assert!(store.remove_local().is_some());
        assert!(store.local().is_none());
    }

    #[test]
    fn eval_cx_reads_and_writes_the_subject_actors_map() {
        let mut store = VariableStorage::<TestHost>::with_local();
        let cow = Actor::Id(1);
        store.actor_mut(cow).set_public(BAA, V::Float(1.23));
        store.refresh_snapshots();
        with_cx(Subjects::actor(cow), &mut store, &NoContext, |cx| {
            cx.set_variable(BAA, V::Float(2.34));
            assert_eq!(cx.variable(BAA), Some(&V::Float(2.34)));
        });
        assert_eq!(store.actor(cow).unwrap().access(BAA), Some(Access::Public));
        assert!(store.local().unwrap().is_empty());
    }

    /// Unresolvable array entries are dropped, so stored references never dangle.
    #[test]
    fn assignments_store_actor_ids() {
        let mut map = VariableMap::<TestHost>::new();
        with_cx(
            Subjects::actor(Actor::Handle(1)),
            &mut map,
            &NoContext,
            |cx| {
                cx.set_variable(VariableName::new("friend"), V::Actor(Actor::Handle(2)));
                cx.set_variable(
                    VariableName::new("herd"),
                    V::actor_array([
                        Actor::Handle(1),
                        Actor::Handle(0),
                        Actor::Id(3),
                        Actor::Id(77),
                    ]),
                );
                cx.set_variable(VariableName::new("n"), V::Float(1.0));
            },
        );
        assert_eq!(
            map.get(VariableName::new("friend")),
            Some(&V::Actor(Actor::Id(2)))
        );
        // The null pointer and the dead id are dropped.
        assert_eq!(
            map.get(VariableName::new("herd")),
            Some(&V::actor_array([Actor::Id(1), Actor::Id(3)]))
        );
        assert_eq!(map.get(VariableName::new("n")), Some(&V::Float(1.0)));
    }

    #[test]
    fn context_provider() {
        let moo = ContextName::new("moo");
        let mut context = ContextMap::<TestHost>::from([(moo, V::Actor(Actor::Handle(1)))]);
        assert_eq!(context.context(moo), Some(V::Actor(Actor::Handle(1))));
        assert_eq!(context.context(ContextName::new("other")), None);
        assert_eq!(context.len(), 1);
        assert_eq!(
            context.set(ContextName::new("null_actor"), V::Actor(Actor::Handle(0))),
            None
        );
        assert_eq!(
            context.get(ContextName::new("null_actor")),
            Some(&V::Actor(Actor::Handle(0)))
        );
        assert_eq!(context.remove(moo), Some(V::Actor(Actor::Handle(1))));
        let mut map = VariableMap::<TestHost>::new();
        with_cx(Subjects::none(), &mut map, &context, |cx| {
            assert_eq!(
                cx.context(ContextName::parse("c.null_actor").unwrap()),
                Some(V::Actor(Actor::Handle(0)))
            );
            assert_eq!(cx.context(moo), None);
        });
        // A host clears the context after the evaluation.
        context.clear();
        assert!(context.is_empty());
        assert_eq!(ContextProvider::<TestHost>::context(&NoContext, moo), None);
    }

    #[test]
    fn subjects() {
        let none = Subjects::<TestHost>::none();
        assert_eq!(
            (none.actor, none.item, none.block, none.world_gen, none.this),
            (None, None, None, None, 0.0)
        );
        assert_eq!(Subjects::<TestHost>::default(), none);
        assert_eq!(
            Subjects::<TestHost>::actor(Actor::Id(1)).actor,
            Some(Actor::Id(1))
        );
        assert_eq!(Subjects::<TestHost>::item(3).item, Some(3));
        assert_eq!(Subjects::<TestHost>::block(()).block, Some(()));
        assert_eq!(
            Subjects::<TestHost>::world_gen(WorldGenPos::new(1, 2, 3)).world_gen,
            Some(WorldGenPos::new(1, 2, 3))
        );
        // `this` is the caller-supplied current value.
        let with_this = Subjects { this: 2.34, ..none };
        assert_eq!(with_this.this, 2.34);
        let copy = with_this;
        assert_eq!(copy, with_this);
    }

    #[test]
    fn eval_limits() {
        let limits = EvalLimits::default();
        assert_eq!(
            limits.loop_iterations,
            Some(molangx::vm::EvalLimits::DEFAULT_LOOP_ITERATIONS)
        );
        assert_eq!(limits.loop_iterations, Some(1024));
        assert_eq!(limits.total_steps, Some(1_048_576));
        assert_eq!(limits.loop_iterations, Some(1024));
        assert_eq!(limits.total_steps, Some(1_048_576));
        assert!(!limits.is_unlimited());
        // `EvalLimits::NONE` sets no loop, step or struct-depth budget.
        assert_eq!(EvalLimits::NONE.loop_iterations, None);
        assert_eq!(EvalLimits::NONE.total_steps, None);
        assert_eq!(limits.struct_depth, Some(32));
        assert_eq!(EvalLimits::NONE.struct_depth, None);
        assert!(EvalLimits::NONE.is_unlimited());
    }

    #[test]
    fn no_host_environment() {
        let mut env = NoHostEnv {
            this: 2.34,
            ..NoHostEnv::new()
        };
        env.variables.set(VariableName::new("x"), Value::Float(3.0));
        env.context
            .set(ContextName::new("moo"), Value::string("moo"));
        {
            let mut cx = env.cx();
            assert_eq!(
                cx.subjects,
                Subjects::<NoHost> {
                    this: 2.34,
                    ..Subjects::<NoHost>::none()
                }
            );
            assert_eq!(
                cx.variable(VariableName::new("x")),
                Some(&Value::Float(3.0))
            );
            cx.set_variable(VariableName::new("y"), Value::Float(4.0));
            assert_eq!(
                cx.context(ContextName::new("moo")),
                Some(Value::string("moo"))
            );
            assert_eq!(cx.limits, EvalLimits::DEFAULT);
            // The first sample of the standard xorshift128 sequence.
            let first = sample(cx.rng);
            assert_eq!(first, sample(&mut Xorshift128::new()));
            assert!(cx.queries.is_none());
        }
        assert_eq!(
            env.variables.get(VariableName::new("y")),
            Some(&Value::Float(4.0))
        );
        assert!(env.sink.is_empty());
    }
}

mod query_tables {
    use molangx::version::MolangVersion;

    use molangx::catalog::{DefaultReturn, QueryCatalog, Side};
    use molangx::stdlib::query;

    use molangx::rng::{FixedRng, rand_core::Rng, sample};

    use molangx::vm::{
        CollectSink, ContextName, Host, HostAccess, QueryBackend, QueryCx, QueryResult, QueryTable,
        RuntimeMsg, RuntimeSink, StructValue, Subjects, Value, VariableName,
    };

    #[derive(Debug)]
    struct TestHost;

    impl Host for TestHost {
        type ActorRef = u32;
        type ItemRef = ();
        type BlockRef = ();
        type Access<'w> = World<'w>;
    }

    struct World<'w> {
        babies: &'w [u32],
    }

    impl HostAccess<TestHost> for World<'_> {
        fn resolve_actor(&self, _from: &Subjects<TestHost>, actor: u32) -> Option<u32> {
            Some(actor)
        }
    }

    type V = Value<TestHost>;

    /// The client's standard catalogue, which the stub tables are for.
    fn client() -> &'static QueryCatalog {
        molangx::stdlib::queries(Side::Client)
    }

    /// A stand-in for the evaluator: arguments are precomputed values, and it counts evaluations.
    struct MockVm<'a, 'w> {
        subjects: Subjects<TestHost>,
        world: &'a mut World<'w>,
        rng: FixedRng,
        sink: CollectSink,
        args: Vec<V>,
        evaluated: Vec<usize>,
    }

    impl<'a, 'w> MockVm<'a, 'w> {
        fn new(world: &'a mut World<'w>, subjects: Subjects<TestHost>, args: Vec<V>) -> Self {
            Self {
                subjects,
                world,
                rng: FixedRng::HALF,
                sink: CollectSink::new(),
                args,
                evaluated: Vec::new(),
            }
        }

        fn call(&mut self, table: &QueryTable<TestHost>, name: &str, version: MolangVersion) -> V {
            let mut cx = QueryCx::new(client(), name, version, self)
                .expect("query available at this version");
            table.call(&mut cx)
        }
    }

    impl<'w> QueryBackend<'w, TestHost> for MockVm<'_, 'w> {
        fn subjects(&self) -> Subjects<TestHost> {
            self.subjects
        }

        fn host(&mut self) -> &mut World<'w> {
            self.world
        }

        fn rng(&mut self) -> &mut dyn Rng {
            &mut self.rng
        }

        fn sink(&mut self) -> &mut dyn RuntimeSink {
            &mut self.sink
        }

        fn variable(&self, _name: VariableName) -> Option<V> {
            None
        }

        fn context(&self, name: ContextName) -> Option<V> {
            (name == ContextName::new("moo")).then_some(V::Actor(1))
        }

        fn arg_count(&self) -> usize {
            self.args.len()
        }

        fn eval_arg(&mut self, index: usize) -> Option<V> {
            let value = self.args.get(index)?.clone();
            self.evaluated.push(index);
            Some(value)
        }
    }

    fn is_baby(cx: &mut QueryCx<'_, '_, TestHost>) -> QueryResult<TestHost> {
        let Some(actor) = cx.subjects().actor else {
            return Ok(V::ZERO);
        };
        Ok(V::bool(cx.host().babies.contains(&actor)))
    }

    fn sum(cx: &mut QueryCx<'_, '_, TestHost>) -> QueryResult<TestHost> {
        let mut total = 0.0;
        for i in 0..cx.arg_count() {
            total += cx.arg_f32(i).ok_or_else(|| {
                cx.error(format_args!(
                    "argument {i} of {} is not a number",
                    cx.name()
                ))
            })?;
        }
        Ok(V::Float(total))
    }

    fn second_only(cx: &mut QueryCx<'_, '_, TestHost>) -> QueryResult<TestHost> {
        Ok(cx.arg(1).unwrap_or(V::ZERO))
    }

    fn remaining_use_duration(cx: &mut QueryCx<'_, '_, TestHost>) -> QueryResult<TestHost> {
        Ok(V::Float(if cx.implementation() == 0 { 10.0 } else { 20.0 }))
    }

    fn needs_item(cx: &mut QueryCx<'_, '_, TestHost>) -> QueryResult<TestHost> {
        Err(cx.error(format_args!("{} has no item", cx.name())))
    }

    fn logs_and_draws(cx: &mut QueryCx<'_, '_, TestHost>) -> QueryResult<TestHost> {
        cx.sink().runtime(RuntimeMsg::PublicAccessUnderflow);
        let sample = sample(cx.rng());
        let moo = cx
            .context(ContextName::new("moo"))
            .and_then(|v| v.as_actor())
            .map_or(0.0, |a| a as f32);
        assert_eq!(cx.variable(VariableName::new("x")), None);
        assert_eq!(cx.name(), "query.life_time");
        Ok(V::Float(sample + moo))
    }

    #[test]
    fn stubs_return_the_default_of_each_query() {
        let table = QueryTable::<TestHost>::new(client());
        assert_eq!(table.implemented(), 0);
        let mut world = World { babies: &[] };
        let mut vm = MockVm::new(&mut world, Subjects::none(), vec![V::Float(1.0)]);
        for decl in client() {
            assert!(table.is_stub(decl.name()).unwrap());
            assert!(table.get(decl.name()).unwrap().is_none());
            let version = decl.shape().ranges.as_slice()[0].first();
            let mut cx = QueryCx::new(client(), decl.name(), version, &mut vm).unwrap();
            assert_eq!(
                table.call(&mut cx),
                V::from(decl.shape().default_return),
                "{}",
                decl.name()
            );
        }
        // A stub evaluates no argument and logs nothing.
        assert!(vm.evaluated.is_empty());
        assert!(vm.sink.is_empty());
        // Spot checks of the non-float defaults.
        assert_eq!(
            client()
                .get(query::SPELLCOLOR)
                .unwrap()
                .shape()
                .default_return,
            DefaultReturn::StructRgba0
        );
        assert_eq!(
            vm.call(&table, query::SPELLCOLOR, MolangVersion::LATEST),
            V::structure(StructValue::rgba(0.0, 0.0, 0.0, 0.0))
        );
        assert_eq!(
            vm.call(&table, query::COMBINE_ENTITIES, MolangVersion::LATEST),
            V::actor_array([])
        );
        assert_eq!(
            vm.call(&table, query::IS_BABY, MolangVersion::LATEST),
            V::Float(0.0)
        );
    }

    /// `q.is_baby` is 1 for a baby subject and 0 for another or for none.
    #[test]
    fn installed_queries_see_subjects_and_the_world() {
        let mut table = QueryTable::<TestHost>::new(client());
        table.set(query::IS_BABY, is_baby).unwrap();
        table.set(query::LIFE_TIME, logs_and_draws).unwrap();
        assert_eq!(table.implemented(), 2);
        assert!(!table.is_stub(query::IS_BABY).unwrap());

        let babies = [1];
        let mut world = World { babies: &babies };
        let mut vm = MockVm::new(&mut world, Subjects::actor(1), vec![]);
        assert_eq!(
            vm.call(&table, query::IS_BABY, MolangVersion::LATEST),
            V::Float(1.0)
        );
        vm.subjects = Subjects::actor(2);
        assert_eq!(
            vm.call(&table, query::IS_BABY, MolangVersion::LATEST),
            V::Float(0.0)
        );
        vm.subjects = Subjects::none();
        assert_eq!(
            vm.call(&table, query::IS_BABY, MolangVersion::LATEST),
            V::Float(0.0)
        );

        // Random source, context, and a non-fatal log are reachable from a query.
        assert_eq!(
            vm.call(&table, query::LIFE_TIME, MolangVersion::LATEST),
            V::Float(1.5)
        );
        assert_eq!(vm.sink.messages.len(), 1);

        table.unset(query::IS_BABY).unwrap();
        assert!(table.is_stub(query::IS_BABY).unwrap());
        assert_eq!(table.clone().implemented(), 1);
    }

    /// In the order the query asks.
    #[test]
    fn arguments_are_evaluated_lazily() {
        let mut table = QueryTable::<TestHost>::new(client());
        table.set(query::LIFE_TIME, sum).unwrap();
        table.set(query::ANGER_LEVEL, second_only).unwrap();
        let mut world = World { babies: &[] };
        let mut vm = MockVm::new(
            &mut world,
            Subjects::none(),
            vec![V::Float(1.0), V::Float(2.0), V::Float(3.0), V::Float(1.0)],
        );
        assert_eq!(
            vm.call(&table, query::LIFE_TIME, MolangVersion::LATEST),
            V::Float(7.0)
        );
        assert_eq!(vm.evaluated, [0, 1, 2, 3]);

        vm.evaluated.clear();
        assert_eq!(
            vm.call(&table, query::ANGER_LEVEL, MolangVersion::LATEST),
            V::Float(2.0)
        );
        assert_eq!(vm.evaluated, [1]);

        // Out-of-range and wrong-kind accesses are `None`, not panics.
        vm.args = vec![V::string("moo"), V::Actor(5)];
        let mut cx =
            QueryCx::new(client(), query::LIFE_TIME, MolangVersion::LATEST, &mut vm).unwrap();
        assert_eq!(cx.arg_count(), 2);
        assert_eq!(cx.arg_f32(0), None);
        assert_eq!(cx.arg_hash(0), V::string("moo").as_hash());
        assert_eq!(cx.arg_actor(1), Some(5));
        assert_eq!(cx.arg_hash(1), None);
        assert_eq!(cx.arg(2), None);
        assert_eq!(cx.arg_f32(9), None);
    }

    #[test]
    fn query_errors_are_logged_and_yield_the_default() {
        let mut table = QueryTable::<TestHost>::new(client());
        table.set(query::MAX_DURABILITY, needs_item).unwrap();
        table.set(query::LIFE_TIME, sum).unwrap();
        let mut world = World { babies: &[] };
        let mut vm = MockVm::new(
            &mut world,
            Subjects::none(),
            vec![V::Float(1.0), V::string("two")],
        );
        assert_eq!(
            vm.call(&table, query::MAX_DURABILITY, MolangVersion::LATEST),
            V::from(
                client()
                    .get(query::MAX_DURABILITY)
                    .unwrap()
                    .shape()
                    .default_return
            )
        );
        assert_eq!(
            vm.call(&table, query::LIFE_TIME, MolangVersion::LATEST),
            V::Float(0.0)
        );
        assert_eq!(
            vm.sink.messages,
            [
                "query.max_durability has no item",
                "argument 1 of query.life_time is not a number"
            ]
        );
    }

    #[test]
    fn names_with_two_ranges_branch_on_the_implementation() {
        let mut table = QueryTable::<TestHost>::new(client());
        table
            .set(query::ITEM_REMAINING_USE_DURATION, remaining_use_duration)
            .unwrap();
        let mut world = World { babies: &[] };
        let mut vm = MockVm::new(&mut world, Subjects::none(), vec![]);
        assert_eq!(
            vm.call(
                &table,
                query::ITEM_REMAINING_USE_DURATION,
                MolangVersion::V1
            ),
            V::Float(10.0)
        );
        assert_eq!(
            vm.call(
                &table,
                query::ITEM_REMAINING_USE_DURATION,
                MolangVersion::V2
            ),
            V::Float(20.0)
        );
        assert_eq!(
            vm.call(
                &table,
                query::ITEM_REMAINING_USE_DURATION,
                MolangVersion::LATEST
            ),
            V::Float(20.0)
        );
        // The table is indexed by name: one slot for both version ranges.
        assert_eq!(table.implemented(), 1);
    }
}

mod message_sinks {
    use molangx::catalog::Side;
    use molangx::rng::{FixedRng, rand_core::Rng};
    use molangx::stdlib::query;
    use molangx::version::MolangVersion;
    use molangx::vm::{
        CollectSink, ContextName, LogLevel, LogOnce, NoHost, NullSink, QueryBackend, QueryCx,
        QueryError, RuntimeMsg, RuntimeSink, Subjects, Value, VariableName,
    };

    /// A backend with nothing behind it: enough to make a query's context.
    struct Nothing(NoHost, NullSink, FixedRng);

    impl QueryBackend<'static, NoHost> for Nothing {
        fn subjects(&self) -> Subjects<NoHost> {
            Subjects::none()
        }
        fn host(&mut self) -> &mut NoHost {
            &mut self.0
        }
        fn rng(&mut self) -> &mut dyn Rng {
            &mut self.2
        }
        fn sink(&mut self) -> &mut dyn RuntimeSink {
            &mut self.1
        }
        fn variable(&self, _name: VariableName) -> Option<Value<NoHost>> {
            None
        }
        fn context(&self, _name: ContextName) -> Option<Value<NoHost>> {
            None
        }
        fn arg_count(&self) -> usize {
            0
        }
        fn eval_arg(&mut self, _index: usize) -> Option<Value<NoHost>> {
            None
        }
    }

    /// An error of the query `name`, made the only way a query makes one: through its context.
    fn query_error(name: &str, message: impl std::fmt::Display) -> QueryError {
        let mut backend = Nothing(NoHost, NullSink, FixedRng::HALF);
        let cx = QueryCx::new(
            molangx::stdlib::queries(Side::Server),
            name,
            MolangVersion::LATEST,
            &mut backend,
        )
        .expect("a standard query");
        cx.error(message)
    }

    /// Both language messages keep their texts at Error level, with the longer unknown-variable
    /// wording inside `->`.
    #[test]
    fn the_two_recorded_messages_keep_their_texts() {
        let cases = [
            (
                RuntimeMsg::UnknownVariable {
                    name: "variable.missing_never_set_r16",
                    public_access: false,
                },
                "Error: unhandled request for unknown variable 'variable.missing_never_set_r16'",
            ),
            (
                RuntimeMsg::UnknownVariable {
                    name: "temp.x",
                    public_access: true,
                },
                "Error: unhandled request for unknown variable 'temp.x' - are you trying to access a variable from a different mob that hasn't made its variable public in its resource definition?",
            ),
            (
                RuntimeMsg::MissingMember { name: "b" },
                "Error: unable to find member variable b",
            ),
        ];
        for (msg, text) in cases {
            assert_eq!(msg.to_string(), text);
            assert_eq!(msg.level(), LogLevel::Error);
        }
    }

    /// The budget messages are the crate's own, not language messages.
    #[test]
    fn budget_messages_are_ours() {
        for msg in [
            RuntimeMsg::LoopLimit { limit: 1024 },
            RuntimeMsg::StepLimit { limit: 1_048_576 },
            RuntimeMsg::StructDepthLimit { limit: 32 },
            RuntimeMsg::OperandStackLimit { limit: 65_536 },
            RuntimeMsg::StructMemberLimit { limit: 256 },
            RuntimeMsg::QueryDepthLimit { limit: 8 },
        ] {
            assert_eq!(msg.level(), LogLevel::Warn);
            assert!(msg.to_string().starts_with("molangx: "));
        }
        assert!(
            RuntimeMsg::LoopLimit { limit: 1024 }
                .to_string()
                .contains("1024")
        );
        assert!(LogLevel::Error > LogLevel::Warn);
    }

    /// Once per content-log session, however many evaluations hit it.
    #[test]
    fn log_once_drops_repeats_by_formatted_text() {
        let mut sink = LogOnce::new(CollectSink::new());
        for _ in 0..3 {
            sink.runtime(RuntimeMsg::UnknownVariable {
                name: "variable.x",
                public_access: false,
            });
        }
        // A different name is a different text; so is the public-access variant of the same name.
        sink.runtime(RuntimeMsg::UnknownVariable {
            name: "variable.y",
            public_access: false,
        });
        sink.runtime(RuntimeMsg::UnknownVariable {
            name: "variable.x",
            public_access: true,
        });
        sink.runtime(RuntimeMsg::UnknownVariable {
            name: "variable.y",
            public_access: false,
        });
        assert_eq!(sink.inner().messages.len(), 3);
        assert_eq!(sink.distinct(), 3);
        assert_eq!(
            sink.inner().messages[0],
            "Error: unhandled request for unknown variable 'variable.x'"
        );
        assert_eq!(
            sink.inner().messages[1],
            "Error: unhandled request for unknown variable 'variable.y'"
        );

        // Query errors go through the same filter, keyed by their formatted text.
        let error = |n: i32| {
            query_error(
                query::HAS_ANY_FAMILY,
                format_args!("argument {n} of query.has_any_family is not a string"),
            )
        };
        sink.query_error(error(1));
        sink.query_error(error(1));
        sink.query_error(error(2));
        assert_eq!(sink.inner().messages.len(), 5);

        // A new session logs everything once again.
        sink.reset();
        sink.runtime(RuntimeMsg::UnknownVariable {
            name: "variable.x",
            public_access: false,
        });
        assert_eq!(sink.inner_mut().take().len(), 6);
        assert!(sink.into_inner().is_empty());
    }

    #[test]
    fn sinks() {
        let mut collect = CollectSink::new();
        {
            let mut by_ref: &mut dyn RuntimeSink = &mut collect;
            by_ref.runtime(RuntimeMsg::PublicAccessUnderflow);
            (&mut by_ref).query_error(query_error(
                query::MAX_DURABILITY,
                "query.max_durability has no item",
            ));
        }
        assert_eq!(
            collect.messages,
            [
                "molangx: a public-access scope was closed while none was open",
                "query.max_durability has no item"
            ]
        );
        let mut null = NullSink;
        null.runtime(RuntimeMsg::PublicAccessUnderflow);
        null.query_error(query_error(query::MAX_DURABILITY, "x"));
    }

    /// A `%` or a brace in a query error's text is just a character.
    #[test]
    fn a_query_error_is_its_text() {
        let slot = "slot.hotbar";
        let cases: [(QueryError, &str); 5] = [
            (
                query_error(
                    query::HAS_ANY_FAMILY,
                    format_args!("argument {} of query.has_any_family is not a string", 3),
                ),
                "argument 3 of query.has_any_family is not a string",
            ),
            (
                query_error(
                    query::COOLDOWN_TIME,
                    format_args!(
                        "query.cooldown_time with slot '{slot}' needs a slot index as its second argument"
                    ),
                ),
                "query.cooldown_time with slot 'slot.hotbar' needs a slot index as its second argument",
            ),
            (
                query_error(
                    query::BLOCK_STATE,
                    format_args!("{} takes one block state name", query::BLOCK_STATE),
                ),
                "query.block_state takes one block state name",
            ),
            (
                query_error(query::LIFE_TIME, "Error: %s failed at 100% {x}"),
                "Error: %s failed at 100% {x}",
            ),
            (
                query_error(query::LIFE_TIME, String::from("plain text")),
                "plain text",
            ),
        ];
        for (error, text) in cases {
            assert_eq!(error.to_string(), text);
            assert_eq!(error.message(), text);
        }
        let e = query_error(query::IS_BABY, "text");
        assert_eq!(e.query(), query::IS_BABY);
        // It is a std error, usable with `?`.
        let _: &dyn std::error::Error = &e;
    }
}

mod message_texts {
    #[cfg(feature = "vm")]
    #[test]
    fn log_levels_are_ordered_from_warn_to_error() {
        use molangx::vm::LogLevel;
        let levels = [LogLevel::Warn, LogLevel::Error];
        assert!(
            levels.windows(2).all(|pair| pair[0] < pair[1]),
            "{levels:?}"
        );
    }

    #[cfg(feature = "vm")]
    #[test]
    fn public_access_underflow_and_host_type_texts() {
        use molangx::vm::{LogLevel, RuntimeMsg};
        let cases = [
            (
                RuntimeMsg::PublicAccessUnderflow,
                "molangx: a public-access scope was closed while none was open",
            ),
            (
                RuntimeMsg::IncompatibleType { name: None },
                "molangx: a host value has an incompatible type",
            ),
            (
                RuntimeMsg::IncompatibleType {
                    name: Some("texture.default"),
                },
                "molangx: a host value has an incompatible type: texture.default",
            ),
        ];
        for (msg, text) in cases {
            assert_eq!(msg.to_string(), text);
            assert_eq!(msg.level(), LogLevel::Warn);
        }
    }

    #[cfg(feature = "vm")]
    #[test]
    fn duplicate_member() {
        use molangx::hash::HashedStr;
        use molangx::vm::{LogLevel, NoHost, RuntimeMsg, StructValue, Value};

        let mut members = StructValue::<NoHost>::new();
        members
            .add(HashedStr::new("x"), Value::Float(1.0))
            .expect("first add");
        let error = members
            .add(HashedStr::new("x"), Value::Float(2.0))
            .expect_err("second add");
        assert_eq!(error.name, HashedStr::new("x"));
        assert_eq!(members.get(HashedStr::new("x")), Some(&Value::Float(1.0)));
        assert!(
            error
                .to_string()
                .starts_with("molangx: a struct already has a member named '")
        );
        let msg = RuntimeMsg::DuplicateMember { name: "x" };
        assert_eq!(
            msg.to_string(),
            "molangx: a struct already has a member named 'x'"
        );
        assert_eq!(msg.level(), LogLevel::Warn);
    }
}
