//! The hand-over from the float loop to the general loop, and the general entry.

use super::query;
use super::{Base, Deopt, Slot, Stop, Vm};
use crate::compile::program::{CallIdx, PostIdx, Program};
use crate::vm::{cx::EvalCx, host::Host, value::Value};

#[allow(clippy::inline_always)] // each instruction method has one call site, in the hot loop `Vm::run`, which must not call it
impl<H: Host, R: Slot<H>> Vm<H, R> {
    /// The float loop makes argument-less calls itself and hands over before any other; a result
    /// it cannot hold is handed over with the call done, to continue at `pc`.
    #[inline(always)]
    pub(super) fn call_query(
        &mut self,
        program: &Program,
        cx: &mut EvalCx<'_, '_, H>,
        q: CallIdx,
        p: PostIdx,
        at: usize,
        pc: usize,
    ) -> Result<(), Stop<H>> {
        let result = match R::general(self) {
            Some(general) => {
                let result = query::call(program, cx, program.call(q), Some(general));
                if self.stopped {
                    return Err(Stop::Abort);
                }
                result
            }
            None => match program.calls.get(usize::from(q.0)) {
                Some(call) if call.args.is_empty() => query::call(program, cx, call, None),
                _ => return Err(Deopt::At(at).into()),
            },
        };
        match Self::with_post(program, &result, p) {
            Some(value) => {
                self.acc = value;
                Ok(())
            }
            // Only the float loop cannot hold a value.
            None => Err(Deopt::After {
                next: pc,
                value: result,
            }
            .into()),
        }
    }
}

impl<H: Host> Vm<H, f32> {
    /// The general loop finishes the evaluation.
    pub(super) fn hand_over(
        self,
        program: &Program,
        cx: &mut EvalCx<'_, '_, H>,
        deopt: Deopt<H>,
    ) -> Value<H> {
        match deopt {
            Deopt::At(pc) => self.into_general(1).finish(program, cx, pc),
            Deopt::After { next, value } => {
                let mut general = self.into_general(0);
                general.acc = value;
                general.finish(program, cx, next)
            }
        }
    }

    /// `give_back` is 1 when handing over before an instruction the float loop charged but did not
    /// run, since the general loop charges it again; 0 after an instruction.
    pub(super) fn into_general(self, give_back: u64) -> Vm<H, Value<H>> {
        Vm {
            acc: Value::Float(self.acc),
            stack: self.stack.into_iter().map(Value::Float).collect(),
            loops: self.loops,
            handlers: self.handlers,
            arrows: self.arrows,
            temps: self
                .temps
                .into_iter()
                .map(|t| t.map(Value::Float))
                .collect(),
            steps_left: self.steps_left.saturating_add(give_back),
            stopped: self.stopped,
            loop_budget_logged: self.loop_budget_logged,
            arg_depth: self.arg_depth,
        }
    }
}

impl<H: Host> Vm<H, Value<H>> {
    pub(super) fn finish(
        &mut self,
        program: &Program,
        cx: &mut EvalCx<'_, '_, H>,
        pc: usize,
    ) -> Value<H> {
        let result = match self.run(program, cx, pc, Base::ROOT) {
            Ok(()) => std::mem::take(&mut self.acc),
            Err(_) => Value::ZERO,
        };
        self.unwind(cx, Base::ROOT);
        result
    }
}

#[cfg(test)]
mod tests {
    // Query functions have the fixed `Query` signature.
    #![allow(clippy::unnecessary_wraps)]

    use super::*;
    use crate::catalog::stdlib_index;
    use crate::compile::program::{HashIdx, Instr, QueryCall};
    use crate::hash::HashedStr;
    use crate::stdlib::query;
    use crate::vm::eval::test_support::*;
    use crate::vm::{
        NoHost, NoHostEnv, QueryError, QueryResult, host::Host, name::TempName, query::QueryCx,
        value::ResourceRef,
    };

    fn string_x<H: Host>(_cx: &mut QueryCx<'_, '_, H>) -> Result<Value<H>, QueryError> {
        count_call();
        Ok(Value::string("x"))
    }

    /// Asserts a hand-over before the instruction at `pc`.
    #[track_caller]
    fn expect_deopt_at(
        vm: &mut Vm<NoHost, f32>,
        program: &Program,
        cx: &mut EvalCx<'_, '_, NoHost>,
        pc: usize,
    ) {
        match vm.run(program, cx, 0, Base::ROOT) {
            Err(Stop::Deopt(Deopt::At(at))) => assert_eq!(at, pc),
            other => panic!("no hand-over before an instruction: {other:?}"),
        }
    }

    #[test]
    fn the_float_loop_hands_over_before_a_non_float_variable_read() {
        let expr = compile_ok("v.s");
        let program = expr.program().unwrap();
        assert!(program.float_loop);
        let mut env = NoHostEnv::new();
        strings(&mut env);
        let mut cx = env.cx();
        let mut vm = Vm::<NoHost, f32>::new(program, Some(10));
        expect_deopt_at(&mut vm, program, &mut cx, 0);
        // The instruction was charged but did not run.
        assert_eq!(vm.steps_left, 9);
        // The general loop charges it again, so one step is given back.
        let general = vm.into_general(1);
        assert_eq!(general.steps_left, 10);
    }

    #[test]
    fn the_hand_over_continues_correctly() {
        let r = ran("v.s", strings);
        assert_eq!(r.value, NV::string("moo"));
        let expr = compile_ok("v.s");
        let mut env = NoHostEnv::new();
        strings(&mut env);
        assert_eq!(
            expr.eval_f32(&mut env.cx()).to_bits(),
            NV::string("moo").as_f32().to_bits()
        );
    }

    #[test]
    fn the_handed_over_instruction_is_charged_once() {
        for src in [
            "v.s",
            "v.t == v.s",
            "v.t * 2 == v.s",
            "v.t < 5 ? v.s : 1",
            "v.t > 5 ? v.s : 1",
            "t.a = 5; t.b = v.s; return t.a;",
        ] {
            let expr = compile_ok(src);
            assert!(
                expr.program().unwrap().float_loop,
                "{src} starts on the float loop"
            );
            let float_start = steps_needed_of(&expr, strings, |e, env| e.eval(&mut env.cx()));
            let general = steps_needed_of(&expr, strings, eval_general);
            assert_eq!(float_start, general, "{src}");
        }
        assert_eq!(steps_needed("v.s", strings), 2);
        assert_eq!(steps_needed("v.t == v.s", strings), 5);
    }

    #[test]
    fn a_hand_over_in_the_middle_keeps_the_operand_stack() {
        let expr = compile_ok("v.t == v.s");
        let program = expr.program().unwrap();
        let mut env = NoHostEnv::new();
        strings(&mut env);
        let mut cx = env.cx();
        let mut vm = Vm::<NoHost, f32>::new(program, Some(100));
        // load t, push, then the string read.
        expect_deopt_at(&mut vm, program, &mut cx, 2);
        assert_eq!(vm.stack.as_slice(), [4.0]);
        let general = vm.into_general(1);
        assert_eq!(general.stack.as_slice(), [NV::Float(4.0)]);
        assert_eq!(general.acc, NV::Float(4.0));
    }

    #[test]
    fn a_hand_over_in_the_middle_keeps_the_temps() {
        let expr = compile_ok("t.a = 5; t.b = v.s; return t.a;");
        let program = expr.program().unwrap();
        let mut env = NoHostEnv::new();
        strings(&mut env);
        let mut cx = env.cx();
        let mut vm = Vm::<NoHost, f32>::new(program, Some(100));
        expect_deopt_at(&mut vm, program, &mut cx, 2);
        let general = vm.into_general(1);
        assert_eq!(general.temps.as_slice(), [Some(NV::Float(5.0)), None]);
        let r = ran("t.a = 5; t.b = v.s; return t.a;", strings);
        assert_eq!(r.value, NV::Float(5.0));
        assert!(r.msgs.is_empty());
    }

    #[test]
    fn the_hand_over_keeps_persistent_temps_in_the_environment() {
        let expr = compile_ok("t.a = 5; t.b = v.s; return t.a;");
        let mut env = kept_temps_env();
        strings(&mut env);
        assert_eq!(expr.eval(&mut env.cx()), NV::Float(5.0));
        let temps = env.temps.kept().unwrap();
        assert_eq!(temps.get(TempName::new("a")), Some(&NV::Float(5.0)));
        assert_eq!(temps.get(TempName::new("b")), Some(&NV::string("moo")));
    }

    #[test]
    fn the_hand_over_keeps_the_loop_frames_the_handlers_and_the_counters() {
        // The float loop is in a loop with a handler when it meets the string.
        let src = "loop(3, { v.n = v.n + (v.nope ?? 1); v.r = v.s; }); return v.n;";
        let expr = compile_ok(src);
        assert!(expr.program().unwrap().float_loop);
        let setup = |env: &mut NoHostEnv| {
            strings(env);
            set(env, &[("n", 0.0)]);
        };
        let r = ran(src, setup);
        assert_eq!(r.value, NV::Float(3.0));
        assert!(r.msgs.is_empty());
        assert_eq!(r.var("r"), Some(&NV::string("moo")));
        assert_eq!(
            steps_needed_of(&expr, setup, |e, env| e.eval(&mut env.cx())),
            steps_needed_of(&expr, setup, eval_general)
        );
    }

    #[test]
    fn strings_and_resources_need_the_general_loop() {
        let mut p = asm(vec![Instr::Hash { h: HashIdx(0) }, Instr::End]);
        p.hashes = vec![HashedStr::new("moo").as_u64()].into_boxed_slice();
        let mut env = NoHostEnv::new();
        let mut cx = env.cx();
        let mut vm = Vm::<NoHost, f32>::new(&p, Some(10));
        expect_deopt_at(&mut vm, &p, &mut cx, 0);
        let mut vm = Vm::<NoHost, NV>::new(&p, Some(10));
        vm.run(&p, &mut cx, 0, Base::ROOT).unwrap();
        assert_eq!(vm.acc, NV::string("moo"));

        let mut p = asm(vec![Instr::Resource { h: HashIdx(0) }, Instr::End]);
        p.hashes = vec![HashedStr::new("texture.default").as_u64()].into_boxed_slice();
        let mut vm = Vm::<NoHost, f32>::new(&p, Some(10));
        expect_deopt_at(&mut vm, &p, &mut cx, 0);
        let mut vm = Vm::<NoHost, NV>::new(&p, Some(10));
        vm.run(&p, &mut cx, 0, Base::ROOT).unwrap();
        assert_eq!(
            vm.acc,
            NV::Resource(ResourceRef::from_raw_hash(HashedStr::new(
                "texture.default"
            )))
        );
    }

    #[test]
    fn a_float_variable_never_hands_over() {
        let expr = compile_ok("v.x * 2 + v.y");
        let program = expr.program().unwrap();
        let mut env = NoHostEnv::new();
        set(&mut env, &[("x", 2.0), ("y", 3.0)]);
        let mut cx = env.cx();
        let mut vm = Vm::<NoHost, f32>::new(program, Some(100));
        vm.run(program, &mut cx, 0, Base::ROOT).unwrap();
        assert_eq!(vm.acc, 7.0);
    }

    #[test]
    fn a_post_op_on_a_string_variable_reads_its_bits_without_handing_over() {
        let expr = compile_ok("v.s * 2");
        let program = expr.program().unwrap();
        let mut env = NoHostEnv::new();
        strings(&mut env);
        let mut cx = env.cx();
        let mut vm = Vm::<NoHost, f32>::new(program, Some(100));
        vm.run(program, &mut cx, 0, Base::ROOT).unwrap();
        assert_eq!(
            vm.acc.to_bits(),
            (NV::string("moo").as_f32() * 2.0).to_bits()
        );
    }

    fn query_env(f: fn(&mut QueryCx<'_, '_, NoHost>) -> QueryResult<NoHost>) -> NoHostEnv {
        let mut env = server_env();
        table(&mut env).set(query::IS_BABY, f).unwrap();
        env
    }

    #[test]
    fn a_query_without_arguments_runs_on_the_float_loop() {
        let expr = compile_ok("q.is_baby * 2");
        assert!(expr.program().unwrap().float_loop);
        assert!(!compile_ok("q.position(0)").program().unwrap().float_loop);
        reset_calls();
        let mut env = query_env(five);
        assert_eq!(expr.eval(&mut env.cx()), NV::Float(10.0));
        assert_eq!(calls(), 1);
        assert_eq!(expr.eval_f32(&mut env.cx()), 10.0);
        assert_eq!(calls(), 2);
    }

    #[test]
    fn the_float_loop_hands_over_before_a_call_with_arguments() {
        // The compiler never gives such a call to the float loop; a hand-assembled program does.
        let mut p = asm(vec![
            Instr::Call {
                q: CallIdx(0),
                p: PLAIN,
            },
            Instr::End,
        ]);
        p.calls = vec![QueryCall {
            index: stdlib_index(query::IS_BABY),
            impl_idx: 0,
            args: vec![1].into_boxed_slice(),
        }]
        .into_boxed_slice();
        reset_calls();
        let mut env = query_env(five);
        let mut cx = env.cx();
        let mut vm = Vm::<NoHost, f32>::new(&p, Some(10));
        expect_deopt_at(&mut vm, &p, &mut cx, 0);
        // The float loop did not make the call.
        assert_eq!(calls(), 0);
        assert_eq!(vm.steps_left, 9);
        // A call that names no entry is handed over as well.
        let mut p = asm(vec![
            Instr::Call {
                q: CallIdx(3),
                p: PLAIN,
            },
            Instr::End,
        ]);
        p.calls = Box::new([]);
        let mut vm = Vm::<NoHost, f32>::new(&p, Some(10));
        expect_deopt_at(&mut vm, &p, &mut cx, 0);
        assert_eq!(calls(), 0);
    }

    #[test]
    fn a_non_float_query_result_hands_over_after_the_call() {
        let expr = compile_ok("q.is_baby");
        let program = expr.program().unwrap();
        assert!(program.float_loop);
        reset_calls();
        let mut env = query_env(string_x);
        let mut cx = env.cx();
        let mut vm = Vm::<NoHost, f32>::new(program, Some(10));
        match vm.run(program, &mut cx, 0, Base::ROOT) {
            Err(Stop::Deopt(Deopt::After { next, value })) => {
                assert_eq!(next, 1);
                assert_eq!(value, NV::string("x"));
            }
            other => panic!("expected a hand-over after the call: {other:?}"),
        }
        // The call ran: it is charged and nothing is given back.
        assert_eq!(vm.steps_left, 9);
        assert_eq!(vm.into_general(0).steps_left, 9);
        assert_eq!(calls(), 1);
    }

    #[test]
    fn the_query_runs_once_even_when_its_result_is_handed_over() {
        reset_calls();
        let mut env = query_env(string_x);
        assert_eq!(compile_ok("q.is_baby").eval(&mut env.cx()), NV::string("x"));
        assert_eq!(calls(), 1);
        reset_calls();
        let bits = compile_ok("q.is_baby").eval_f32(&mut env.cx()).to_bits();
        assert_eq!(bits, NV::string("x").as_f32().to_bits());
        assert_eq!(calls(), 1);
    }

    #[test]
    fn the_general_loop_continues_after_a_handed_over_query_result() {
        reset_calls();
        let mut env = query_env(string_x);
        let expr = compile_ok("v.z = q.is_baby; return v.z;");
        assert!(expr.program().unwrap().float_loop);
        assert_eq!(expr.eval(&mut env.cx()), NV::string("x"));
        assert_eq!(calls(), 1);
        assert_eq!(env.variables.get(key("z")), Some(&NV::string("x")));
    }

    #[test]
    fn a_post_op_on_a_query_string_is_arithmetic_not_a_hand_over() {
        reset_calls();
        let mut env = query_env(string_x);
        let expr = compile_ok("q.is_baby * 2");
        let value = expr.eval(&mut env.cx());
        assert_eq!(value, NV::Float(NV::string("x").as_f32() * 2.0));
        assert_eq!(calls(), 1);
    }

    #[test]
    fn a_hand_over_after_a_query_costs_what_the_general_loop_costs() {
        let expr = compile_ok("v.z = q.is_baby; return v.z;");
        let setup = |env: &mut NoHostEnv| {
            table(env).set(query::IS_BABY, string_x).unwrap();
        };
        let float_start = steps_needed_of(&expr, setup, |e, env| e.eval(&mut env.cx()));
        let general = steps_needed_of(&expr, setup, eval_general);
        assert_eq!(float_start, general);
        // call, store, load, return.
        assert_eq!(float_start, 4);
    }

    #[test]
    fn a_float_loop_program_with_kept_temps_hands_over_a_string_temp() {
        let mut env = kept_temps_env();
        env.temps
            .kept_mut()
            .unwrap()
            .set(TempName::new("s"), NV::string("moo"));
        assert_eq!(
            compile_ok("return t.s;").eval(&mut env.cx()),
            NV::string("moo")
        );
        // With a post-op the string is just a number.
        let value = compile_ok("return t.s * 2;").eval_f32(&mut env.cx());
        assert_eq!(
            value.to_bits(),
            (NV::string("moo").as_f32() * 2.0).to_bits()
        );
    }
}
