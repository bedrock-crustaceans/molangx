//! Query calls: the argument sub-programs and the backend a query runs against.

use super::Vm;
use crate::compile::program::{Program, QueryCall};
use crate::rng::rand_core::Rng;
use crate::vm::{
    cx::EvalCx,
    host::{Host, Subjects},
    name::{ContextName, VariableName},
    query::{QueryBackend, QueryCx},
    sink::{RuntimeMsg, RuntimeSink},
    value::Value,
};

impl<H: Host> Vm<H, Value<H>> {
    /// Runs an argument sub-program in this evaluation; 0.0 when it aborted.
    ///
    /// The evaluator's one native recursion, which
    /// [`EvalLimits::query_depth`](crate::vm::EvalLimits::query_depth) may bound below the
    /// compiler's 255 levels.
    pub(super) fn run_sub(
        &mut self,
        program: &Program,
        cx: &mut EvalCx<'_, '_, H>,
        start: u32,
    ) -> Value<H> {
        if self.stopped {
            return Value::ZERO;
        }
        if let Some(limit) = cx.limits.query_depth
            && self.arg_depth >= limit
        {
            self.stopped = true;
            cx.sink.runtime(RuntimeMsg::QueryDepthLimit { limit });
            return Value::ZERO;
        }
        let base = self.base();
        let saved = std::mem::take(&mut self.acc);
        self.arg_depth += 1;
        let run = self.run(program, cx, start as usize, base);
        self.arg_depth -= 1;
        let result = match run {
            Ok(()) => std::mem::replace(&mut self.acc, saved),
            Err(_) => {
                self.acc = saved;
                Value::ZERO
            }
        };
        self.unwind(cx, base);
        result
    }
}

/// Calls query `call`; without a query table it returns its declared default. Only the general
/// loop passes its machine, which runs the arguments; the float loop calls only queries without
/// arguments.
pub(super) fn call<H: Host>(
    program: &Program,
    cx: &mut EvalCx<'_, '_, H>,
    call: &QueryCall,
    vm: Option<&mut Vm<H, Value<H>>>,
) -> Value<H> {
    let Some(queries) = cx.queries else {
        return Value::from(program.catalog.decl(call.index).shape().default_return);
    };
    let mut backend = Backend {
        cx,
        args: vm.map(|vm| Args {
            vm,
            program,
            starts: &call.args,
        }),
    };
    let mut query = QueryCx::resolved(
        &program.catalog,
        call.index,
        program.version,
        call.impl_idx,
        &mut backend,
    );
    queries.call(&mut query)
}

struct Backend<'r, 'a, 'w, H: Host> {
    cx: &'r mut EvalCx<'a, 'w, H>,
    args: Option<Args<'r, H>>,
}

/// The argument sub-programs of a call and the machine that runs them.
struct Args<'r, H: Host> {
    vm: &'r mut Vm<H, Value<H>>,
    program: &'r Program,
    starts: &'r [u32],
}

impl<'w, H: Host> QueryBackend<'w, H> for Backend<'_, '_, 'w, H> {
    fn subjects(&self) -> Subjects<H> {
        self.cx.subjects
    }

    fn host(&mut self) -> &mut H::Access<'w> {
        self.cx.host
    }

    fn rng(&mut self) -> &mut dyn Rng {
        self.cx.rng
    }

    fn sink(&mut self) -> &mut dyn RuntimeSink {
        self.cx.sink
    }

    fn variable(&self, name: VariableName) -> Option<Value<H>> {
        self.cx.variable(name).cloned()
    }

    fn context(&self, name: ContextName) -> Option<Value<H>> {
        self.cx.context(name)
    }

    fn arg_count(&self) -> usize {
        self.args.as_ref().map_or(0, |args| args.starts.len())
    }

    fn eval_arg(&mut self, index: usize) -> Option<Value<H>> {
        let args = self.args.as_mut()?;
        let start = *args.starts.get(index)?;
        Some(args.vm.run_sub(args.program, self.cx, start))
    }
}

#[cfg(test)]
mod tests {
    // Query functions have the fixed `Query` signature.
    #![allow(clippy::unnecessary_wraps)]

    use super::*;
    use crate::catalog::{DefaultReturn, QueryCatalog, QueryDecl, QueryShape, Side};
    use crate::compile::{CompileOptions, compile};
    use crate::rng::{Xorshift128, sample};
    use crate::stdlib::query;
    use crate::version::MolangVersion;
    use crate::vm::eval::test_support::*;
    use crate::vm::eval::{Base, Stop};
    use crate::vm::{NoHost, NoHostEnv, QueryError};

    fn nested_call(limit: u32) -> Ran {
        ran(
            "v.a = 1; v.b = q.position(q.position(1)); v.c = 1; return 5;",
            |env| {
                table(env).set(query::POSITION, first_arg).unwrap();
                env.limits.query_depth = Some(limit);
            },
        )
    }

    const QUERY_DEPTH_MSG: &str =
        "molangx: evaluation stopped: query arguments would nest deeper than their budget of";

    #[test]
    fn an_argument_deeper_than_the_query_budget_ends_the_evaluation() {
        let r = nested_call(1);
        assert_eq!(r.value, NV::Float(0.0));
        assert_eq!(r.msgs, [format!("{QUERY_DEPTH_MSG} 1 levels")]);
        assert_eq!(r.var("a"), Some(&NV::Float(1.0)));
        assert!(r.var("b").is_none());
        assert!(r.var("c").is_none());
    }

    #[test]
    fn a_query_budget_of_zero_lets_no_query_evaluate_an_argument() {
        let r = ran("v.a = q.position(1); v.c = 1;", |env| {
            table(env).set(query::POSITION, first_arg).unwrap();
            env.limits.query_depth = Some(0);
        });
        assert_eq!(r.msgs, [format!("{QUERY_DEPTH_MSG} 0 levels")]);
        assert!(r.var("a").is_none() && r.var("c").is_none());
        // A query with no argument is not an argument: it still runs.
        let r = ran("q.is_baby", |env| {
            table(env).set(query::IS_BABY, five).unwrap();
            env.limits.query_depth = Some(0);
        });
        assert_eq!(r.value, NV::Float(5.0));
        assert!(r.msgs.is_empty());
    }

    #[test]
    fn nesting_within_the_query_budget_runs() {
        let r = nested_call(2);
        assert_eq!(r.value, NV::Float(5.0));
        assert!(r.msgs.is_empty());
        assert_eq!(r.var("b"), Some(&NV::Float(1.0)));
        let r = nested_call(u32::MAX);
        assert_eq!(r.value, NV::Float(5.0));
    }

    #[test]
    fn run_sub_respects_the_query_depth_budget_and_restores_the_accumulator() {
        let expr = compile_ok("q.position(1)");
        let program = expr.program().unwrap();
        let start = program.calls[0].args[0];
        let mut env = NoHostEnv::new();
        env.limits.query_depth = Some(1);
        let mut cx = env.cx();
        let mut vm = Vm::<NoHost, NV>::new(program, Some(100));
        vm.acc = NV::Float(42.0);
        // One level is allowed, and the caller's accumulator comes back.
        assert_eq!(vm.run_sub(program, &mut cx, start), NV::Float(1.0));
        assert_eq!(vm.acc, NV::Float(42.0));
        assert_eq!(vm.arg_depth, 0);
        assert!(!vm.stopped);
        // From one level down, the next is refused.
        vm.arg_depth = 1;
        assert_eq!(vm.run_sub(program, &mut cx, start), NV::ZERO);
        assert!(vm.stopped);
        assert_eq!(vm.arg_depth, 1, "a refused argument does not count");
        // A stopped evaluation returns at once and says nothing more.
        assert_eq!(vm.run_sub(program, &mut cx, start), NV::ZERO);
        assert_eq!(env.sink.take(), [format!("{QUERY_DEPTH_MSG} 1 levels")]);
    }

    #[test]
    fn run_sub_gives_zero_for_an_argument_that_aborts() {
        let expr = compile_ok("q.position(v.nope)");
        let program = expr.program().unwrap();
        let mut env = NoHostEnv::new();
        let mut cx = env.cx();
        let mut vm = Vm::<NoHost, NV>::new(program, Some(100));
        vm.acc = NV::Float(8.0);
        assert_eq!(
            vm.run_sub(program, &mut cx, program.calls[0].args[0]),
            NV::ZERO
        );
        assert_eq!(vm.acc, NV::Float(8.0));
        assert!(!vm.stopped, "a miss ends the argument, not the evaluation");
        assert_eq!(env.sink.take(), [unknown_msg("variable.nope")]);
    }

    #[test]
    fn a_stopped_evaluation_runs_nothing_more() {
        let expr = compile_ok("v.x * 2");
        let program = expr.program().unwrap();
        let mut env = NoHostEnv::new();
        set(&mut env, &[("x", 1.0)]);
        let mut cx = env.cx();
        let mut vm = Vm::<NoHost, NV>::new(program, Some(100));
        vm.stopped = true;
        assert!(matches!(
            vm.run(program, &mut cx, 0, Base::ROOT),
            Err(Stop::Abort)
        ));
        assert_eq!(vm.steps_left, 100, "no step was taken");
    }

    #[test]
    fn a_limit_that_stops_an_evaluation_stops_it_for_good_within_that_evaluation() {
        // After the depth budget stops the store, the loop around it does not run another pass.
        let r = ran(
            "loop(3, { v.n = v.n + 1; v.s.x.y.z = 1; }); return v.n;",
            |env| {
                env.variables.set(key("n"), NV::Float(0.0));
                limited(Some(2), None)(env);
            },
        );
        assert_eq!(r.value, NV::Float(0.0));
        assert_eq!(r.msgs, [DEPTH_MSG]);
        assert_eq!(r.var("n"), Some(&NV::Float(1.0)));
    }

    fn twice<H: Host>(cx: &mut QueryCx<'_, '_, H>) -> Result<Value<H>, QueryError> {
        let a = cx.arg_f32(0).unwrap_or(0.0);
        let b = cx.arg_f32(0).unwrap_or(0.0);
        Ok(Value::Float(a + b))
    }

    /// `QueryCx::arg` forwards any index to the backend.
    fn beyond<H: Host>(cx: &mut QueryCx<'_, '_, H>) -> Result<Value<H>, QueryError> {
        let count = cx.arg_count();
        let missing =
            cx.arg(count).is_none() && cx.arg_f32(3).is_none() && cx.arg(usize::MAX).is_none();
        Ok(Value::Float(if missing { 1.0 } else { 2.0 }))
    }

    fn probe<H: Host>(cx: &mut QueryCx<'_, '_, H>) -> Result<Value<H>, QueryError> {
        let x = cx
            .variable(VariableName::new("x"))
            .map_or(0.0, |v| v.as_f32());
        let k = cx
            .context(ContextName::new("k"))
            .map_or(0.0, |v| v.as_f32());
        let this = cx.subjects().this;
        let sample = sample(cx.rng());
        cx.sink().runtime(RuntimeMsg::LoopLimit { limit: 7 });
        Ok(Value::Float(x + k * 10.0 + this * 100.0 + sample))
    }

    fn which_implementation<H: Host>(cx: &mut QueryCx<'_, '_, H>) -> Result<Value<H>, QueryError> {
        let latest = if cx.version() == MolangVersion::LATEST {
            100.0
        } else {
            0.0
        };
        Ok(Value::Float(f32::from(cx.implementation()) + latest))
    }

    fn fails<H: Host>(cx: &mut QueryCx<'_, '_, H>) -> Result<Value<H>, QueryError> {
        Err(cx.error(format_args!("Error: {}", cx.name())))
    }

    fn first_sample() -> f32 {
        sample(&mut Xorshift128::new())
    }

    #[test]
    fn an_argument_the_query_never_asks_for_is_never_evaluated() {
        let src = "q.position(math.random(0, 1) + v.x)";
        let mut env = server_env();
        table(&mut env).set(query::POSITION, five).unwrap();
        set(&mut env, &[("x", 1.0)]);
        let before = env.rng.clone();
        assert_eq!(compile_ok(src).eval(&mut env.cx()), NV::Float(5.0));
        assert_eq!(
            env.rng, before,
            "the random draw in the argument did not run"
        );
        // call, end: the argument's instructions cost nothing.
        assert_eq!(
            steps_needed(src, |env| {
                table(env).set(query::POSITION, five).unwrap();
            }),
            2
        );
    }

    #[test]
    fn an_argument_the_query_asks_for_runs_when_it_asks() {
        let src = "q.position(math.random(0, 1) + v.x)";
        let mut env = server_env();
        table(&mut env).set(query::POSITION, first_arg).unwrap();
        set(&mut env, &[("x", 1.0)]);
        let mut expected = env.rng.clone();
        let sample = sample(&mut expected);
        assert_eq!(compile_ok(src).eval(&mut env.cx()), NV::Float(sample + 1.0));
        assert_eq!(env.rng, expected, "exactly one draw");
    }

    #[test]
    fn an_argument_is_evaluated_again_every_time_it_is_asked_for() {
        let mut env = server_env();
        table(&mut env).set(query::POSITION, twice).unwrap();
        let mut expected = env.rng.clone();
        let (a, b) = (sample(&mut expected), sample(&mut expected));
        assert_eq!(
            compile_ok("q.position(math.random(0, 1))").eval(&mut env.cx()),
            NV::Float(a + b)
        );
        assert_eq!(env.rng, expected);
    }

    #[test]
    fn an_argument_costs_its_own_steps_each_time() {
        let once = steps_needed("q.position(v.x * 2)", |env| {
            table(env).set(query::POSITION, first_arg).unwrap();
            set(env, &[("x", 1.0)]);
        });
        let doubled = steps_needed("q.position(v.x * 2)", |env| {
            table(env).set(query::POSITION, twice).unwrap();
            set(env, &[("x", 1.0)]);
        });
        // call, end of the caller, and the argument: load, end.
        assert_eq!(once, 2 + 2);
        assert_eq!(doubled, 2 + 2 * 2);
    }

    #[test]
    fn an_argument_reads_the_callers_variables_and_values_flow_back() {
        let r = ran("v.y * 10 + q.position(v.x * 2)", |env| {
            table(env).set(query::POSITION, first_arg).unwrap();
            set(env, &[("x", 3.0), ("y", 4.0)]);
        });
        // The caller's accumulator is restored after the argument ran.
        assert_eq!(r.value, NV::Float(46.0));
    }

    #[test]
    fn nested_calls_evaluate_innermost_first() {
        let r = ran("q.position(q.position(v.x) + 1)", |env| {
            table(env).set(query::POSITION, first_arg).unwrap();
            set(env, &[("x", 3.0)]);
        });
        assert_eq!(r.value, NV::Float(4.0));
    }

    #[test]
    fn an_argument_of_any_kind_comes_back_as_it_is() {
        let r = ran("q.position('moo')", |env| {
            table(env).set(query::POSITION, first_arg).unwrap();
        });
        assert_eq!(r.value, NV::string("moo"));
    }

    #[test]
    fn asking_for_an_argument_the_call_does_not_have_gives_none_from_either_backend() {
        // `q.is_baby` has no arguments (the float loop's backend); `q.position(…)` has one, whose
        // random draw shows that asking past the end evaluates nothing.
        for src in ["q.is_baby", "q.position(math.random(0, 1))"] {
            let mut env = server_env();
            table(&mut env).set(query::IS_BABY, beyond).unwrap();
            table(&mut env).set(query::POSITION, beyond).unwrap();
            let before = env.rng.clone();
            assert_eq!(compile_ok(src).eval(&mut env.cx()), NV::Float(1.0), "{src}");
            assert_eq!(env.rng, before, "{src}: no argument ran");
        }
    }

    fn probe_env() -> NoHostEnv {
        let mut env = server_env();
        table(&mut env).set(query::IS_BABY, probe).unwrap();
        table(&mut env).set(query::POSITION, probe).unwrap();
        set(&mut env, &[("x", 2.0)]);
        env.context.set(ContextName::new("k"), NV::Float(3.0));
        env.this = 4.0;
        env
    }

    #[test]
    fn both_backends_offer_variables_context_subjects_rng_and_sink() {
        let expected = 2.0 + 3.0 * 10.0 + 4.0 * 100.0 + first_sample();
        for src in ["q.is_baby", "q.position(0)"] {
            let mut env = probe_env();
            let value = compile_ok(src).eval(&mut env.cx());
            assert_eq!(value, NV::Float(expected), "{src}");
            assert_eq!(
                env.sink.take(),
                ["molangx: loop stopped after its budget of 7 iterations"],
                "{src}"
            );
        }
    }

    #[test]
    fn the_call_context_carries_the_version_and_the_implementation() {
        for src in ["q.is_baby", "q.position(0)"] {
            let mut env = server_env();
            table(&mut env)
                .set(query::IS_BABY, which_implementation)
                .unwrap();
            table(&mut env)
                .set(query::POSITION, which_implementation)
                .unwrap();
            let expr = compile_ok(src);
            let idx = expr.program().unwrap().calls[0].impl_idx;
            assert_eq!(
                expr.eval(&mut env.cx()),
                NV::Float(100.0 + f32::from(idx)),
                "{src}"
            );
        }
    }

    fn is_baby_default() -> NV {
        NV::from(
            crate::stdlib::queries(Side::Client)
                .get(query::IS_BABY)
                .unwrap()
                .shape()
                .default_return,
        )
    }

    #[test]
    fn a_stub_gives_the_default_of_its_return_type_on_either_loop() {
        let expr = compile_ok("q.is_baby");
        let default = is_baby_default();
        let mut env = NoHostEnv::new();
        assert_eq!(expr.eval(&mut env.cx()), default);
        assert_eq!(eval_general(&expr, &mut env), default);
        assert!(env.sink.is_empty());
    }

    #[test]
    fn without_a_table_every_call_returns_its_declared_default_and_evaluates_no_argument() {
        let options = CompileOptions::client(MolangVersion::LATEST);
        let mut env = NoHostEnv::new();
        for (src, want) in [
            ("q.is_baby(math.random(0, 1))", NV::Float(0.0)),
            ("q.armor_color_slot(math.random(0, 1))", NV::Float(1.0)),
            ("q.time_since_last_vibration_detection", NV::Float(-1.0)),
            (
                "q.get_equipped_item_name(math.random(0, 1))",
                NV::string(""),
            ),
            ("q.combine_entities(math.random(0, 1))", NV::actor_array([])),
            ("q.spellcolor", NV::from(DefaultReturn::StructRgba0)),
        ] {
            let (expr, _) = compile(src, &options).into_result().unwrap();
            assert_eq!(expr.eval(&mut env.cx()), want, "{src}");
        }
        assert_eq!(env.rng, Xorshift128::new());
        assert!(env.sink.is_empty());
        let own = QueryCatalog::new(
            Side::Client,
            [QueryDecl::new(
                "query.own",
                QueryShape {
                    default_return: DefaultReturn::FloatNeg1,
                    ..QueryShape::DEFAULT
                },
            )
            .unwrap()],
        )
        .unwrap();
        let (expr, _) = compile("q.own", &CompileOptions::new(own, MolangVersion::LATEST))
            .into_result()
            .unwrap();
        assert_eq!(expr.eval(&mut env.cx()), NV::Float(-1.0));
    }

    #[test]
    fn a_failing_query_logs_its_error_and_gives_the_default_on_either_loop() {
        let expr = compile_ok("q.is_baby");
        let default = is_baby_default();
        let mut env = server_env();
        table(&mut env).set(query::IS_BABY, fails).unwrap();
        assert_eq!(expr.eval(&mut env.cx()), default);
        assert_eq!(env.sink.take(), ["Error: query.is_baby"]);
        assert_eq!(eval_general(&expr, &mut env), default);
        assert_eq!(env.sink.take(), ["Error: query.is_baby"]);
    }

    #[test]
    fn a_query_result_takes_the_post_op_of_its_call() {
        let mut env = server_env();
        table(&mut env).set(query::IS_BABY, five).unwrap();
        assert_eq!(
            compile_ok("q.is_baby * 2 + 1").eval(&mut env.cx()),
            NV::Float(11.0)
        );
        table(&mut env).set(query::POSITION, five).unwrap();
        assert_eq!(
            compile_ok("q.position(0) * 2 + 1").eval(&mut env.cx()),
            NV::Float(11.0)
        );
    }

    #[test]
    fn a_query_call_in_the_float_loop_does_not_need_a_vm_of_the_general_kind() {
        let expr = compile_ok("q.is_baby + q.is_baby");
        let program = expr.program().unwrap();
        assert!(program.float_loop);
        let mut env = server_env();
        table(&mut env).set(query::IS_BABY, five).unwrap();
        let mut cx = env.cx();
        let mut vm = Vm::<NoHost, f32>::new(program, Some(100));
        vm.run(program, &mut cx, 0, Base::ROOT).unwrap();
        assert_eq!(vm.acc, 10.0);
    }
}
