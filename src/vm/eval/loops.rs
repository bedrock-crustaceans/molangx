//! The loop guard: the iteration budget and the operand-stack cap, with their messages.

use std::ops::ControlFlow;

use super::{Aborted, Deopt, Frame, Slot, Stop, Vm};
use crate::compile::program::{Instr, NameIdx, Program, TempIdx};
use crate::numeric;
use crate::vm::{cx::EvalCx, host::Host, sink::RuntimeMsg, value::Value};

/// The variable a `for_each` binds each actor to.
#[derive(Copy, Clone)]
pub(super) enum LoopVar {
    Var(NameIdx),
    Temp(TempIdx),
}

// `ControlFlow::Continue`: the loop makes a pass; `ControlFlow::Break`: it is left.
#[allow(clippy::inline_always)] // each instruction method has one call site, in the hot loop `Vm::run`, which must not call it
impl<H: Host, R: Slot<H>> Vm<H, R> {
    /// Leaves the loop when the count is at most zero by the architecture's `<=`. On `Arm64` a
    /// NaN count is, so the loop makes no pass; on `X86_64` it is not and never runs out
    /// (`NaN − 1` is NaN): `loop(math.sqrt(-1), …)` runs until something else ends it.
    #[inline(always)]
    pub(super) fn loop_begin(&mut self) -> ControlFlow<()> {
        let count = self.acc.f32();
        if spent(count, numeric::le) {
            return ControlFlow::Break(());
        }
        // The counter lives on the operand stack, where the check reads it.
        self.stack.push(R::float(numeric::arith::add(count, -1.0)));
        self.loops.push(Frame::Count { iterations: 1 });
        ControlFlow::Continue(())
    }

    /// Whether the loop makes another pass; the operand-stack cap stops the evaluation.
    #[inline(always)]
    pub(super) fn loop_check(
        &mut self,
        cx: &mut EvalCx<'_, '_, H>,
        loop_limit: Option<u32>,
    ) -> Result<ControlFlow<()>, Aborted> {
        // Counts down the top of the operand stack: the counter, unless a `continue` left an
        // operand pending above it. A NaN goes round again on `X86_64`.
        if self.stack.len() > OPERAND_STACK_CAP {
            self.operand_stack_spent(cx);
            return Err(Aborted);
        }
        let top = self.stack.last().map_or(0.0, R::f32);
        if !spent(top, numeric::le)
            && let Some(Frame::Count { iterations }) = self.loops.last_mut()
        {
            if next_iteration(iterations, loop_limit) {
                if let Some(slot) = self.stack.last_mut() {
                    *slot = R::float(numeric::arith::add(top, -1.0));
                }
                return Ok(ControlFlow::Continue(()));
            }
            self.loop_budget_spent(cx, loop_limit);
        }
        Ok(ControlFlow::Break(()))
    }

    #[inline(always)]
    pub(super) fn loop_end(&mut self) {
        match self.loops.pop() {
            // Pops one slot: the counter, or an operand a `break` / `continue` left pending, in
            // which case the counter stays behind.
            Some(Frame::Count { .. }) => {
                self.stack.pop();
            }
            Some(Frame::Each { stack, .. }) => self.stack.truncate(stack),
            None => {}
        }
    }

    /// Only a non-empty actor array is iterated. Over anything else the `for_each` gives its value
    /// and breaks to past the one instruction after the loop, so the next statement does not
    /// take effect (a read is skipped, a constant store is lost) while the ones after it run.
    #[inline(always)]
    pub(super) fn each_begin(&mut self, program: &Program, exit: u32) -> ControlFlow<usize> {
        match self.acc.to_value() {
            Value::ActorArray(array) if !array.is_empty() => {
                self.loops.push(Frame::Each {
                    array,
                    index: 0,
                    iterations: 0,
                    stack: self.stack.len(),
                });
                ControlFlow::Continue(())
            }
            _ => {
                let exit = exit as usize;
                // The `for_each`'s own value: the constant `codegen` places at `exit`.
                if let Some(&Instr::Const { c }) = program.code.get(exit) {
                    self.acc = R::float(program.konst(c));
                }
                ControlFlow::Break(skip_one(&program.code, exit + 1))
            }
        }
    }

    /// Binds the next live actor to `var` for another pass; the operand-stack cap stops the
    /// evaluation.
    #[inline(always)]
    pub(super) fn each_next(
        &mut self,
        program: &Program,
        cx: &mut EvalCx<'_, '_, H>,
        var: LoopVar,
        loop_limit: Option<u32>,
        at: usize,
    ) -> Result<ControlFlow<()>, Stop<H>> {
        // The float loop cannot hold an actor: hand over before the frame advances. Unreachable
        // while `for_each` is never `FLOAT_ONLY`.
        if R::general(self).is_none() {
            return Err(Deopt::At(at).into());
        }
        if self.stack.len() > OPERAND_STACK_CAP {
            self.operand_stack_spent(cx);
            return Err(Stop::Abort);
        }
        let Some(Frame::Each {
            array,
            index,
            iterations,
            ..
        }) = self.loops.last_mut()
        else {
            return Ok(ControlFlow::Break(()));
        };
        // Entries that resolve to no actor are skipped.
        let actor = loop {
            let Some(&entry) = array.get(*index) else {
                return Ok(ControlFlow::Break(()));
            };
            *index += 1;
            if let Some(live) = cx.resolve_actor(entry) {
                break live;
            }
        };
        if !next_iteration(iterations, loop_limit) {
            self.loop_budget_spent(cx, loop_limit);
            return Ok(ControlFlow::Break(()));
        }
        match var {
            LoopVar::Var(n) => cx.set_variable(program.variable_name(n), Value::Actor(actor)),
            LoopVar::Temp(t) => self.set_temp(program, cx, t, Value::Actor(actor), at)?,
        }
        Ok(ControlFlow::Continue(()))
    }

    /// Only operands left behind by `break` / `continue` pass after pass can reach the cap
    /// ([`EvalLimits::OPERAND_STACK_CAP`](crate::vm::EvalLimits::OPERAND_STACK_CAP)).
    #[cold]
    pub(super) fn operand_stack_spent(&mut self, cx: &mut EvalCx<'_, '_, H>) {
        self.stopped = true;
        cx.sink.runtime(RuntimeMsg::OperandStackLimit {
            limit: OPERAND_STACK_CAP as u32,
        });
    }

    /// Logs once per evaluation, however many loops the loop guard leaves.
    #[cold]
    pub(super) fn loop_budget_spent(&mut self, cx: &mut EvalCx<'_, '_, H>, limit: Option<u32>) {
        if !self.loop_budget_logged {
            self.loop_budget_logged = true;
            cx.sink.runtime(RuntimeMsg::LoopLimit {
                limit: limit.unwrap_or(u32::MAX),
            });
        }
    }
}

/// Whether a loop count makes no further pass: `count <= 0` by `le`, the build's comparison.
#[inline]
fn spent(count: f32, le: impl Fn(f32, f32) -> bool) -> bool {
    le(count, 0.0)
}

/// `false` when the loop guard's budget is spent. Without a budget nothing is counted, so an
/// endless loop cannot overflow the count.
#[inline]
fn next_iteration(iterations: &mut u32, limit: Option<u32>) -> bool {
    match limit {
        Some(limit) if *iterations >= limit => false,
        // `iterations < limit`: cannot overflow.
        Some(_) => {
            *iterations += 1;
            true
        }
        None => true,
    }
}

/// A constant followed by its store counts as one instruction; the end of the program is never
/// skipped.
fn skip_one(code: &[Instr], at: usize) -> usize {
    match (code.get(at), code.get(at + 1)) {
        (None | Some(Instr::End | Instr::Halt), _) => at,
        (Some(Instr::Const { .. }), Some(Instr::StoreVar { .. } | Instr::StoreTemp { .. })) => {
            at + 2
        }
        _ => at + 1,
    }
}

const OPERAND_STACK_CAP: usize = crate::vm::EvalLimits::OPERAND_STACK_CAP as usize;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compile::program::{ConstIdx, PostIdx};
    use crate::compile::{CompileOptions, compile};
    use crate::numeric::PostOp;
    use crate::numeric::arch::{arm64, x86_64};
    use crate::numeric::test_support::per_arch;
    use crate::version::MolangVersion;
    use crate::vm::eval::test_support::*;
    use crate::vm::eval::{Base, Stop};
    use crate::vm::host::Subjects;
    use crate::vm::test_support::TestHost;
    use crate::vm::{EvalLimits, HostEnv, NoHost, NoHostEnv, StructValue};

    #[test]
    fn each_architecture_decides_whether_a_count_is_spent() {
        for count in [0.0, -0.0, -1.0, f32::NEG_INFINITY] {
            assert!(
                spent(count, x86_64::le) && spent(count, arm64::le),
                "{count}"
            );
        }
        for count in [f32::from_bits(1), 1.0, f32::INFINITY] {
            assert!(
                !spent(count, x86_64::le) && !spent(count, arm64::le),
                "{count}"
            );
        }
        assert!(!spent(f32::NAN, x86_64::le));
        assert!(spent(f32::NAN, arm64::le));
        assert_eq!(spent(f32::NAN, numeric::le), per_arch(false, true));
    }

    #[test]
    fn iteration_count_cannot_overflow() {
        let mut iterations = u32::MAX;
        assert!(next_iteration(&mut iterations, None));
        assert_eq!(iterations, u32::MAX);
        let limit = u32::MAX - 1;
        let mut iterations = limit - 1;
        assert!(next_iteration(&mut iterations, Some(limit)));
        assert_eq!(iterations, limit);
        assert!(!next_iteration(&mut iterations, Some(limit)));
        assert_eq!(iterations, limit);
        let mut iterations = 0;
        assert!(!next_iteration(&mut iterations, Some(0)));
    }

    #[test]
    fn next_iteration_counts_up_to_the_budget() {
        let mut iterations = 0;
        let allowed: Vec<bool> = (0..5)
            .map(|_| next_iteration(&mut iterations, Some(3)))
            .collect();
        assert_eq!(allowed, [true, true, true, false, false]);
        assert_eq!(iterations, 3);
        let mut iterations = 7;
        assert!(next_iteration(&mut iterations, None));
        assert_eq!(iterations, 7);
    }

    const COUNT_LOOP: &str = "loop(v.c, { v.n = v.n + 1; }); return v.n;";

    fn iterations(count: f32) -> f32 {
        let r = ran(COUNT_LOOP, |env| set(env, &[("c", count), ("n", 0.0)]));
        assert!(r.msgs.is_empty(), "{:?}", r.msgs);
        r.value.as_f32()
    }

    #[test]
    fn a_loop_runs_the_ceiling_of_its_count() {
        for (count, expected) in [
            (1.0, 1.0),
            (2.0, 2.0),
            (3.0, 3.0),
            (2.5, 3.0),
            (0.5, 1.0),
            (0.001, 1.0),
            (7.01, 8.0),
            (10.0, 10.0),
        ] {
            assert_eq!(iterations(count), expected, "count {count}");
        }
    }

    #[test]
    fn a_loop_runs_no_iteration_for_zero_or_negative_counts() {
        for count in [0.0, -0.0, -1.0, -0.5, -1e9, f32::NEG_INFINITY] {
            assert_eq!(iterations(count), 0.0, "count {count}");
        }
    }

    #[test]
    fn a_nan_count_never_runs_out_on_x86_64_and_makes_no_pass_on_arm64() {
        let escaping = "loop(v.c, { v.n = v.n + 1; v.n > 8 ? {break;} : 0; }); return v.n;";
        let (escaped, guarded) = per_arch((9.0, 1024.0), (0.0, 0.0));
        let options = CompileOptions::server(MolangVersion::LATEST);
        let run = |src: &str| {
            let expr = compile(src, &options).expr().cloned().expect("compiles");
            let mut env = server_env();
            set(&mut env, &[("c", f32::NAN), ("n", 0.0)]);
            let value = expr.eval(&mut env.cx());
            (value, env.sink.take())
        };
        assert_eq!(run(escaping), (NV::Float(escaped), vec![]));
        // Without an escape the loop guard ends it on `X86_64`.
        let guard = if guarded > 0.0 {
            vec!["molangx: loop stopped after its budget of 1024 iterations".to_owned()]
        } else {
            vec![]
        };
        assert_eq!(run(COUNT_LOOP), (NV::Float(guarded), guard));
    }

    /// A `continue` leaves a NaN operand above the counter, so the re-check reads a NaN.
    #[test]
    fn the_re_check_of_a_nan_counter_follows_the_architectures_less_equal() {
        let src = "v.k = math.sqrt(-1); v.n = 0; loop(5, { v.n = v.n + 1; v.t = v.k * (v.n > 0 ? {continue;} : 0); }); return v.n;";
        let budget = vec!["molangx: loop stopped after its budget of 1024 iterations".to_owned()];
        let expected = per_arch((NV::Float(1024.0), budget), (NV::Float(1.0), vec![]));
        let expr = compile(src, &CompileOptions::server(MolangVersion::LATEST))
            .expr()
            .cloned()
            .expect("compiles");
        let mut env = server_env();
        let value = expr.eval(&mut env.cx());
        assert_eq!((value, env.sink.take()), expected);
    }

    #[test]
    fn for_each_over_a_non_array_skips_the_next_instruction() {
        for (collection, value) in [("v.a = 0;", 7.0), ("v.a = 4;", 7.0), ("v.a = 'x';", 7.0)] {
            let src = format!("{collection} v.r = 7; for_each(v.i, v.a, 1); v.r = 1; return v.r;");
            assert_eq!(ran(&src, |_| ()).value, NV::Float(value), "{src}");
        }
        // Only the first store after it is lost.
        assert_eq!(
            ran(
                "v.a = 0; v.r = 0; for_each(t.i, v.a, {v.z = 1;}); v.r = 1; v.r = 2; return v.r;",
                |_| ()
            )
            .value,
            NV::Float(2.0)
        );
        // A skipped load: the store that follows stores the `for_each`'s value, 0.
        assert_eq!(
            ran(
                "v.a = 0; v.r = 5; for_each(v.i, v.a, 1); v.r = v.r + 5; return v.r;",
                |_| ()
            )
            .value,
            NV::Float(0.0)
        );
        // As the last statement of a block, the block's own value is what is skipped.
        assert_eq!(
            ran(
                "v.a = 0; v.r = 0; {for_each(v.i, v.a, 1);}; v.r = 3; return v.r;",
                |_| ()
            )
            .value,
            NV::Float(3.0)
        );
        // Nothing is skipped past the end.
        assert_eq!(
            ran("v.a = 0; for_each(v.i, v.a, 1);", |_| ()).value,
            NV::Float(0.0)
        );
    }

    #[test]
    fn skip_one_takes_a_constant_store_as_one_instruction() {
        let code = [
            Instr::Const { c: ConstIdx(0) },
            Instr::StoreVar {
                n: NameIdx(0),
                p: PostIdx::PLAIN,
            },
            Instr::Const { c: ConstIdx(0) },
            Instr::StoreTemp {
                t: TempIdx(0),
                p: PostIdx::PLAIN,
            },
            Instr::Const { c: ConstIdx(0) },
            Instr::End,
        ];
        assert_eq!(skip_one(&code, 0), 2);
        assert_eq!(skip_one(&code, 2), 4);
        assert_eq!(skip_one(&code, 4), 5);
        assert_eq!(skip_one(&code, 5), 5);
        assert_eq!(skip_one(&code, 6), 6);
    }

    #[test]
    fn a_loop_with_a_literal_count() {
        let r = ran("loop(4, { v.n = v.n + 1; }); return v.n;", |env| {
            set(env, &[("n", 0.0)]);
        });
        assert_eq!(r.value, NV::Float(4.0));
        let r = ran("loop(0, { v.n = v.n + 1; }); return v.n;", |env| {
            set(env, &[("n", 0.0)]);
        });
        assert_eq!(r.value, NV::Float(0.0));
    }

    #[test]
    fn the_loop_guard_stops_a_loop_after_1024_iterations_by_default() {
        let r = ran("loop(100000, { v.n = v.n + 1; }); return v.n;", |env| {
            set(env, &[("n", 0.0)]);
        });
        assert_eq!(r.value, NV::Float(1024.0));
        assert_eq!(
            r.msgs,
            ["molangx: loop stopped after its budget of 1024 iterations"]
        );
    }

    #[test]
    fn the_guard_allows_exactly_its_budget_of_iterations() {
        let with = |count: f32| {
            ran(COUNT_LOOP, |env| {
                env.limits.loop_iterations = Some(4);
                set(env, &[("c", count), ("n", 0.0)]);
            })
        };
        let at = with(4.0);
        assert_eq!((at.value, at.msgs.len()), (NV::Float(4.0), 0));
        let past = with(5.0);
        assert_eq!(past.value, NV::Float(4.0));
        assert_eq!(
            past.msgs,
            ["molangx: loop stopped after its budget of 4 iterations"]
        );
        // The first iteration is always allowed.
        let r = ran(COUNT_LOOP, |env| {
            env.limits.loop_iterations = Some(1);
            set(env, &[("c", 9.0), ("n", 0.0)]);
        });
        assert_eq!(r.value, NV::Float(1.0));
        assert_eq!(
            r.msgs,
            ["molangx: loop stopped after its budget of 1 iterations"]
        );
    }

    #[test]
    fn a_loop_budget_of_zero_still_runs_the_first_iteration() {
        let r = ran(COUNT_LOOP, |env| {
            env.limits.loop_iterations = Some(0);
            set(env, &[("c", 3.0), ("n", 0.0)]);
        });
        assert_eq!(r.value, NV::Float(1.0));
        assert_eq!(
            r.msgs,
            ["molangx: loop stopped after its budget of 0 iterations"]
        );
    }

    #[test]
    fn no_limits_have_no_loop_guard() {
        let r = ran("loop(5000, { v.n = v.n + 1; }); return v.n;", |env| {
            env.limits = EvalLimits::NONE;
            set(env, &[("n", 0.0)]);
        });
        assert_eq!(r.value, NV::Float(5000.0));
        assert!(r.msgs.is_empty());
    }

    #[test]
    fn the_loop_message_is_logged_once_per_evaluation() {
        let r = ran(
            "loop(10, { v.a = v.a + 1; }); loop(10, { v.b = v.b + 1; }); return v.a + v.b;",
            |env| {
                env.limits.loop_iterations = Some(3);
                set(env, &[("a", 0.0), ("b", 0.0)]);
            },
        );
        assert_eq!(r.value, NV::Float(6.0));
        assert_eq!(
            r.msgs,
            ["molangx: loop stopped after its budget of 3 iterations"]
        );
        // Nested: the inner loop is left on every pass of the outer one.
        let r = ran(
            "loop(2, { loop(10, { v.n = v.n + 1; }); }); return v.n;",
            |env| {
                env.limits.loop_iterations = Some(3);
                set(env, &[("n", 0.0)]);
            },
        );
        assert_eq!(r.value, NV::Float(6.0));
        assert_eq!(
            r.msgs,
            ["molangx: loop stopped after its budget of 3 iterations"]
        );
        // Another evaluation logs it again.
        let expr = compile_ok("loop(10, { v.n = v.n + 1; });");
        let mut env = NoHostEnv::new();
        env.limits.loop_iterations = Some(3);
        set(&mut env, &[("n", 0.0)]);
        expr.eval(&mut env.cx());
        expr.eval(&mut env.cx());
        assert_eq!(env.sink.take().len(), 2);
    }

    #[test]
    fn nested_loops_multiply() {
        let r = ran(
            "loop(2, { loop(3, { v.n = v.n + 1; }); }); return v.n;",
            |env| set(env, &[("n", 0.0)]),
        );
        assert_eq!(r.value, NV::Float(6.0));
        let r = ran(
            "loop(2, { loop(3, { loop(4, { v.n = v.n + 1; }); }); }); return v.n;",
            |env| set(env, &[("n", 0.0)]),
        );
        assert_eq!(r.value, NV::Float(24.0));
    }

    #[test]
    fn break_leaves_the_loop_and_continue_goes_to_the_next_pass() {
        let r = ran(
            "loop(10, { v.n = v.n + 1; v.n >= 3 ? break; }); return v.n;",
            |env| set(env, &[("n", 0.0)]),
        );
        assert_eq!(r.value, NV::Float(3.0));
        let r = ran(
            "loop(5, { v.n = v.n + 1; v.n < 3 ? continue; v.m = v.m + 1; }); return v.m * 10 + v.n;",
            |env| {
                set(env, &[("n", 0.0), ("m", 0.0)]);
            },
        );
        assert_eq!(r.value, NV::Float(35.0));
        let r = ran(
            "loop(3, { v.o = v.o + 1; loop(5, { v.n = v.n + 1; break; }); }); return v.o * 10 + v.n;",
            |env| {
                set(env, &[("n", 0.0), ("o", 0.0)]);
            },
        );
        assert_eq!(r.value, NV::Float(33.0));
        assert!(r.msgs.is_empty());
    }

    #[test]
    fn break_and_continue_outside_a_loop_end_the_program_with_zero() {
        let r = ran("v.n = 1; break;", |_| {});
        assert_eq!(r.value, NV::Float(0.0));
        assert_eq!(r.num("n"), 1.0);
        let r = ran("v.n = 1; continue;", |_| {});
        assert_eq!(r.value, NV::Float(0.0));
        assert_eq!(r.num("n"), 1.0);
    }

    #[test]
    fn the_loop_value_is_zero() {
        let r = ran("v.r = loop(2, { v.n = v.n + 1; }); return v.r;", |env| {
            set(env, &[("n", 0.0), ("r", 9.0)]);
        });
        assert_eq!(r.value, NV::Float(0.0));
    }

    const N: NameIdx = NameIdx(0);

    fn counter_posts(mut p: Program) -> Program {
        p.posts = vec![PostOp::IDENTITY, PostOp::new(1.0, 1.0)].into_boxed_slice();
        p.consts = vec![3.0, 0.0, 1.0].into_boxed_slice();
        with_names(p, &["variable.n"], &[])
    }

    /// `loop(3, { n = n + 1; })`, then `n`.
    fn plain_loop() -> Program {
        counter_posts(asm(vec![
            Instr::Const { c: ConstIdx(0) },
            Instr::LoopBegin { exit: 6 },
            Instr::LoadVar {
                n: N,
                p: PostIdx(1),
            },
            Instr::StoreVar { n: N, p: PLAIN },
            Instr::LoopCheck { body: 2 },
            Instr::LoopEnd,
            Instr::LoadVar { n: N, p: PLAIN },
            Instr::End,
        ]))
    }

    /// [`plain_loop`] with an operand left pending above the counter at the back edge, as a
    /// `continue` leaves it.
    fn loop_with_pending(pending_const: ConstIdx) -> Program {
        counter_posts(asm(vec![
            Instr::Const { c: ConstIdx(0) },
            Instr::LoopBegin { exit: 8 },
            Instr::LoadVar {
                n: N,
                p: PostIdx(1),
            },
            Instr::StoreVar { n: N, p: PLAIN },
            Instr::Const { c: pending_const },
            Instr::Push,
            Instr::LoopCheck { body: 2 },
            Instr::LoopEnd,
            Instr::LoadVar { n: N, p: PLAIN },
            Instr::End,
        ]))
    }

    fn counted_env() -> NoHostEnv {
        let mut env = NoHostEnv::new();
        env.variables.set(key("n"), Value::Float(0.0));
        env
    }

    #[test]
    fn a_plain_loop_leaves_nothing_on_the_operand_stack() {
        let program = plain_loop();
        let mut env = counted_env();
        let mut cx = env.cx();
        let mut vm = Vm::<NoHost, NV>::new(&program, Some(1000));
        vm.run(&program, &mut cx, 0, Base::ROOT).unwrap();
        assert_eq!(vm.acc, NV::Float(3.0));
        assert!(vm.stack.is_empty());
        assert!(vm.loops.is_empty());
        // const, begin, 3 x (load, store, check), end of loop, load, end.
        assert_eq!(1000 - vm.steps_left, 14);
    }

    #[test]
    fn the_loop_counter_is_an_operand_stack_slot() {
        let program = plain_loop();
        let mut env = counted_env();
        let mut cx = env.cx();
        let mut vm = Vm::<NoHost, NV>::new(&program, Some(1000));
        // Run up to the first back edge: the counter is `count - 1`.
        vm.steps_left = 4;
        assert!(matches!(
            vm.run(&program, &mut cx, 0, Base::ROOT),
            Err(Stop::Abort)
        ));
        assert_eq!(vm.stack.as_slice(), [NV::Float(2.0)]);
        assert_eq!(vm.loops.len(), 1);
    }

    #[test]
    fn a_continue_with_a_zero_operand_pending_ends_the_loop_after_that_iteration() {
        let program = loop_with_pending(ConstIdx(1));
        let mut env = counted_env();
        let mut cx = env.cx();
        let mut vm = Vm::<NoHost, NV>::new(&program, Some(1000));
        vm.run(&program, &mut cx, 0, Base::ROOT).unwrap();
        // One iteration only: the check read the pending 0, not the counter.
        assert_eq!(vm.acc, NV::Float(1.0));
        // The cleanup popped the pending operand; the counter stays behind.
        assert_eq!(vm.stack.as_slice(), [NV::Float(2.0)]);
    }

    #[test]
    fn a_pending_positive_operand_is_counted_down_instead_of_the_counter() {
        let program = loop_with_pending(ConstIdx(2));
        let mut env = counted_env();
        env.limits.loop_iterations = Some(5);
        let mut cx = env.cx();
        let mut vm = Vm::<NoHost, NV>::new(&program, Some(1000));
        vm.run(&program, &mut cx, 0, Base::ROOT).unwrap();
        // Each pass leaves its own pending operand: only the guard ends the loop, after 5.
        assert_eq!(vm.acc, NV::Float(5.0));
        // The counter, and one operand left by each pass but the last (whose cleanup popped it).
        assert_eq!(vm.stack.len(), 5);
        assert_eq!(vm.stack[0], NV::Float(2.0));
        assert_eq!(
            env.sink.take(),
            ["molangx: loop stopped after its budget of 5 iterations"]
        );
    }

    #[test]
    fn the_pending_operand_comes_from_the_language_too() {
        // `v.zero * ({continue;})` leaves the 0 pending when it continues.
        let source = "v.zero = 0; v.n = 0; loop(5, { v.n = v.n + 1; v.t = v.zero * (v.n > 0 ? {continue;} : 0); }); return v.n;";
        let r = ran(source, |_| {});
        assert_eq!(r.value, NV::Float(1.0));
        // The same body with a plain continue runs all five passes.
        let r = ran(
            "v.n = 0; loop(5, { v.n = v.n + 1; v.n > 0 ? {continue;}; }); return v.n;",
            |_| {},
        );
        assert_eq!(r.value, NV::Float(5.0));
    }

    #[test]
    fn the_operand_stack_cap_ends_a_runaway_loop() {
        let source = "v.zero = 0; v.i = 0; v.j = 0; loop(2, { v.i = v.i + 1; loop(3, { v.j = v.j + 1; v.t = v.zero * (v.j > 0 ? {continue;} : 0); }); }); return 1;";
        let expr = compile_ok(source);
        let mut env = NoHostEnv {
            limits: EvalLimits::NONE,
            ..NoHostEnv::new()
        };
        assert_eq!(expr.eval_f32(&mut env.cx()), 0.0);
        assert_eq!(
            env.sink.take(),
            [
                "molangx: evaluation stopped: operands left behind by break / continue passed the cap of 65536"
            ]
        );
        // Under the default guard the inner loop is left after 1024 passes instead.
        let mut env = NoHostEnv::new();
        assert_eq!(expr.eval_f32(&mut env.cx()), 1.0);
        assert_eq!(
            env.sink.take(),
            ["molangx: loop stopped after its budget of 1024 iterations"]
        );
    }

    fn cap_program(check: Instr) -> Program {
        asm(vec![check, Instr::End])
    }

    #[test]
    fn the_cap_is_checked_at_the_loop_back_edge() {
        let cap = EvalLimits::OPERAND_STACK_CAP as usize;
        let program = cap_program(Instr::LoopCheck { body: 0 });
        // Exactly the cap: no trip (the top is 0, so the loop is left).
        let mut env = NoHostEnv {
            limits: EvalLimits::NONE,
            ..NoHostEnv::new()
        };
        let mut cx = env.cx();
        let mut vm = Vm::<NoHost, NV>::new(&program, None);
        vm.stack.extend((0..cap).map(|_| NV::Float(0.0)));
        vm.run(&program, &mut cx, 0, Base::ROOT).unwrap();
        assert!(!vm.stopped);
        // One more: the evaluation ends.
        let mut vm = Vm::<NoHost, NV>::new(&program, None);
        vm.stack.extend((0..=cap).map(|_| NV::Float(0.0)));
        assert!(matches!(
            vm.run(&program, &mut cx, 0, Base::ROOT),
            Err(Stop::Abort)
        ));
        assert!(vm.stopped);
        assert_eq!(
            env.sink.take(),
            [
                "molangx: evaluation stopped: operands left behind by break / continue passed the cap of 65536"
            ]
        );
    }

    #[test]
    fn the_cap_is_checked_at_the_for_each_step_too() {
        let cap = EvalLimits::OPERAND_STACK_CAP as usize;
        let program = cap_program(Instr::EachNextVar {
            n: NameIdx(0),
            exit: 1,
        });
        let mut env = NoHostEnv::new();
        let mut cx = env.cx();
        // Exactly the cap: no trip (there is no frame, so the step leaves the loop).
        let mut vm = Vm::<NoHost, NV>::new(&program, None);
        vm.stack.extend((0..cap).map(|_| NV::Float(0.0)));
        vm.run(&program, &mut cx, 0, Base::ROOT).unwrap();
        assert!(!vm.stopped);
        // One more: the evaluation ends (and only this run sends the message).
        let mut vm = Vm::<NoHost, NV>::new(&program, None);
        vm.stack.extend((0..=cap).map(|_| NV::Float(0.0)));
        assert!(matches!(
            vm.run(&program, &mut cx, 0, Base::ROOT),
            Err(Stop::Abort)
        ));
        assert!(vm.stopped);
        assert_eq!(
            env.sink.take(),
            [
                "molangx: evaluation stopped: operands left behind by break / continue passed the cap of 65536"
            ]
        );
    }

    #[test]
    fn the_operand_stack_cap_matches_the_documented_constant() {
        assert_eq!(OPERAND_STACK_CAP, 65_536);
    }

    fn herd(actors: &[u32]) -> HV {
        HV::actor_array(actors.iter().copied())
    }

    fn each(src: &str, arr: HV, extra: impl FnOnce(&mut HostEnv<TestHost>)) -> HostRan {
        let mut env = HostEnv::default();
        env.variables.set(key("arr"), arr);
        env.variables.set(key("n"), HV::Float(0.0));
        extra(&mut env);
        host_ran(src, env)
    }

    const COUNT_EACH: &str = "for_each(t.e, v.arr, { v.n = v.n + 1; }); return v.n;";

    #[test]
    fn for_each_visits_every_live_actor_in_order() {
        let r = each(COUNT_EACH, herd(&[1, 2, 3]), |_| {});
        assert_eq!(r.value, HV::Float(3.0));
        let r = each(
            "for_each(v.e, v.arr, { v.n = v.n * 10 + v.e; }); return v.n;",
            herd(&[1, 2, 3]),
            |_| {},
        );
        // An actor reads as 0.0 in arithmetic.
        assert_eq!(r.value, HV::Float(0.0));
        assert_eq!(r.env.variables.get(key("e")), Some(&HV::Actor(3)));
    }

    #[test]
    fn for_each_skips_entries_that_resolve_to_no_actor() {
        // 100 and above are dead in the test host.
        let r = each(COUNT_EACH, herd(&[1, 150, 2, 100, 3, 999]), |_| {});
        assert_eq!(r.value, HV::Float(3.0));
        let r = each(COUNT_EACH, herd(&[100, 101]), |_| {});
        assert_eq!(r.value, HV::Float(0.0));
    }

    #[test]
    fn the_loop_variable_holds_the_last_live_actor_afterwards() {
        let r = each(
            "for_each(v.e, v.arr, { v.n = v.n + 1; }); return v.n;",
            herd(&[4, 7, 120]),
            |_| {},
        );
        assert_eq!(r.value, HV::Float(2.0));
        assert_eq!(r.env.variables.get(key("e")), Some(&HV::Actor(7)));
    }

    #[test]
    fn for_each_into_a_temp_leaves_the_variable_alone() {
        let r = each(
            "for_each(t.e, v.arr, { v.last = t.e; }); return v.n;",
            herd(&[4, 7]),
            |_| {},
        );
        assert_eq!(r.env.variables.get(key("last")), Some(&HV::Actor(7)));
        assert!(r.env.variables.get(key("e")).is_none());
    }

    #[test]
    fn for_each_over_anything_but_a_non_empty_actor_array_runs_zero_times() {
        for arr in [
            HV::Float(3.0),
            HV::ZERO,
            herd(&[]),
            HV::string("moo"),
            HV::Actor(1),
            HV::structure(StructValue::xy(1.0, 2.0)),
            HV::identity_matrix(),
        ] {
            let r = each(COUNT_EACH, arr.clone(), |_| {});
            assert_eq!(r.value, HV::Float(0.0), "{arr:?}");
            assert!(r.msgs.is_empty(), "{arr:?}: {:?}", r.msgs);
        }
    }

    #[test]
    fn a_missing_array_ends_the_evaluation() {
        let mut env = HostEnv::default();
        env.variables.set(key("n"), HV::Float(0.0));
        let r = host_ran(COUNT_EACH, env);
        assert_eq!(r.value, HV::ZERO);
        assert_eq!(r.msgs, [unknown_msg("variable.arr")]);
    }

    #[test]
    fn break_and_continue_in_for_each() {
        let r = each(
            "for_each(t.e, v.arr, { v.n = v.n + 1; break; }); return v.n;",
            herd(&[1, 2, 3]),
            |_| {},
        );
        assert_eq!(r.value, HV::Float(1.0));
        let r = each(
            "for_each(t.e, v.arr, { v.n = v.n + 1; v.n < 3 ? continue; v.m = v.m + 1; }); return v.m;",
            herd(&[1, 2, 3, 4, 5]),
            |env| {
                env.variables.set(key("m"), HV::Float(0.0));
            },
        );
        assert_eq!(r.value, HV::Float(3.0));
    }

    #[test]
    fn the_loop_guard_applies_to_for_each() {
        let r = each(COUNT_EACH, herd(&[1, 2, 3, 4, 5]), |env| {
            env.limits.loop_iterations = Some(2);
        });
        assert_eq!(r.value, HV::Float(2.0));
        assert_eq!(
            r.msgs,
            ["molangx: loop stopped after its budget of 2 iterations"]
        );
        // Skipped entries do not count as iterations.
        let r = each(COUNT_EACH, herd(&[100, 1, 101, 2]), |env| {
            env.limits.loop_iterations = Some(2);
        });
        assert_eq!(r.value, HV::Float(2.0));
        assert!(r.msgs.is_empty());
    }

    #[test]
    fn for_each_iterates_a_copy_of_the_array() {
        let r = each(
            "for_each(t.e, v.arr, { v.n = v.n + 1; v.arr = 0; }); return v.n;",
            herd(&[1, 2, 3]),
            |_| {},
        );
        assert_eq!(r.value, HV::Float(3.0));
    }

    #[test]
    fn for_each_inside_loop_and_loop_inside_for_each() {
        let r = each(
            "loop(2, { for_each(t.e, v.arr, { v.n = v.n + 1; }); }); return v.n;",
            herd(&[1, 2, 3]),
            |_| {},
        );
        assert_eq!(r.value, HV::Float(6.0));
        let r = each(
            "for_each(t.e, v.arr, { loop(3, { v.n = v.n + 1; }); }); return v.n;",
            herd(&[1, 2]),
            |_| {},
        );
        assert_eq!(r.value, HV::Float(6.0));
    }

    #[test]
    fn for_each_costs_the_steps_of_its_instructions() {
        let expr = compile_ok("for_each(t.e, v.arr, { v.n = v.n + 1; });");
        let steps_for = |len: u32| {
            let actors: Vec<u32> = (1..=len).collect();
            let completes = |steps: u64| {
                let mut env = HostEnv::<TestHost>::default();
                env.limits.total_steps = Some(steps);
                env.variables.set(key("arr"), herd(&actors));
                env.variables.set(key("n"), HV::Float(0.0));
                let mut world = TestHost;
                let _ = expr.eval(&mut env.cx(&mut world, Subjects::actor(1)));
                !env.sink
                    .take()
                    .iter()
                    .any(|m| m.starts_with("molangx: evaluation stopped after"))
            };
            (1..2000).find(|&steps| completes(steps)).unwrap()
        };
        let (one, two, four) = (steps_for(1), steps_for(2), steps_for(4));
        // Each further actor costs: each-next, load n *1+1, store n, const 0, jump.
        assert_eq!(two - one, 5);
        assert_eq!(four - two, 10);
        // Entering with an empty array: load, each-begin, end; the loop's value is set without a
        // step and the statement list's constant after it is skipped.
        assert_eq!(steps_for(0), 3);
    }

    #[test]
    fn for_each_leaves_the_operand_stack_as_it_found_it() {
        for body in [
            "v.n = v.n + 1;",
            "v.n = v.n + 1; break;",
            "v.n = v.n + 1; continue;",
            "v.n = v.n + 1; v.n > 1 ? break;",
        ] {
            let src = format!("for_each(t.e, v.arr, {{ {body} }}); return v.n;");
            let expr = compile_ok(&src);
            let program = expr.program().unwrap();
            let mut env = HostEnv::<TestHost>::default();
            env.variables
                .set(key("arr"), HV::actor_array([1_u32, 2, 3]));
            env.variables.set(key("n"), HV::Float(0.0));
            let mut world = TestHost;
            let mut cx = env.cx(&mut world, Subjects::actor(1));
            let mut vm = Vm::<TestHost, HV>::new(program, Some(1000));
            assert!(
                matches!(vm.run(program, &mut cx, 0, Base::ROOT), Ok(())),
                "{src}"
            );
            assert!(vm.stack.is_empty(), "{src}");
            assert!(vm.loops.is_empty(), "{src}");
        }
    }
}
