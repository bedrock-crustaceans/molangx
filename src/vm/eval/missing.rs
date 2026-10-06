//! Missing variables: the base of a call, the handlers of `??` and the unwinding of `->`.

use std::ops::ControlFlow;

use super::{Aborted, Base, Handler, Mark, Slot, Vm};
use crate::compile::program::{PostIdx, Program};
use crate::vm::{cx::EvalCx, host::Host, sink::RuntimeMsg};

#[allow(clippy::inline_always)] // each instruction method has one call site, in the hot loop `Vm::run`, which must not call it
impl<H: Host, R: Slot<H>> Vm<H, R> {
    #[inline(always)]
    pub(super) fn handler_push(&mut self, to: u32) {
        let mark = self.mark();
        self.handlers.push(Handler { to, mark });
    }

    /// A null, dead or non-actor left side loads the post-op of 0 and breaks: the right side is
    /// skipped.
    #[inline(always)]
    pub(super) fn pointer_enter(
        &mut self,
        program: &Program,
        cx: &mut EvalCx<'_, '_, H>,
        p: PostIdx,
    ) -> ControlFlow<()> {
        match cx.arrow_target(&self.acc.to_value()) {
            Some(target) => {
                let caller = std::mem::replace(&mut cx.subjects, target);
                self.arrows.push(caller);
                ControlFlow::Continue(())
            }
            None => {
                self.acc = R::float(program.post(p).apply(0.0));
                ControlFlow::Break(())
            }
        }
    }

    #[inline(always)]
    pub(super) fn pointer_leave(
        &mut self,
        program: &Program,
        cx: &mut EvalCx<'_, '_, H>,
        p: PostIdx,
    ) {
        match self.arrows.pop() {
            Some(caller) => cx.subjects = caller,
            None => cx.sink.runtime(RuntimeMsg::PublicAccessUnderflow),
        }
        self.post_in_place(program, p);
    }

    fn mark(&self) -> Mark {
        Mark {
            stack: self.stack.len(),
            loops: self.loops.len(),
            arrows: self.arrows.len(),
        }
    }

    pub(super) fn base(&self) -> Base {
        Base {
            mark: self.mark(),
            handlers: self.handlers.len(),
        }
    }

    /// Drops what was pushed after `mark` and restores the subjects of the `->`s it leaves.
    fn unwind_to(&mut self, cx: &mut EvalCx<'_, '_, H>, mark: Mark) {
        self.stack.truncate(mark.stack);
        self.loops.truncate(mark.loops);
        if let Some(&outermost) = self.arrows.get(mark.arrows) {
            cx.subjects = outermost;
            self.arrows.truncate(mark.arrows);
        }
    }

    /// Drops everything a run left above `base`, its handlers included.
    pub(super) fn unwind(&mut self, cx: &mut EvalCx<'_, '_, H>, base: Base) {
        self.handlers.truncate(base.handlers);
        self.unwind_to(cx, base.mark);
    }

    /// The innermost handler of this run catches and gives the pc to continue at; without one the
    /// message is logged and the run ends.
    #[cold]
    pub(super) fn missing(
        &mut self,
        cx: &mut EvalCx<'_, '_, H>,
        base: Base,
        name: &str,
    ) -> Result<usize, Aborted> {
        if self.handlers.len() > base.handlers
            && let Some(handler) = self.handlers.pop()
        {
            self.unwind_to(cx, handler.mark);
            self.acc = R::float(0.0);
            return Ok(handler.to as usize);
        }
        cx.sink.runtime(RuntimeMsg::UnknownVariable {
            name,
            public_access: !self.arrows.is_empty(),
        });
        Err(Aborted)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::{Side, stdlib_index};
    use crate::compile::program::{
        CallIdx, ConstIdx, Instr, MemberEntry, MemberIdx, NameIdx, QueryCall,
    };
    use crate::hash::HashedStr;
    use crate::numeric::PostOp;
    use crate::stdlib::query;
    use crate::vm::eval::test_support::*;
    use crate::vm::eval::{Frame, Stop};
    use crate::vm::host::Subjects;
    use crate::vm::test_support::TestHost;
    use crate::vm::{HostEnv, NoHost, NoHostEnv, QueryTable, StructValue, VariableStorage};

    fn member_entry(name: &str, report: &str) -> MemberEntry {
        MemberEntry {
            hash: HashedStr::new(name),
            text: format!(".{name}").into(),
            report: format!(".{report}").into(),
        }
    }

    const PUBLIC_SUFFIX: &str = " - are you trying to access a variable from a different mob that hasn't made its variable public in its resource definition?";

    fn unknown_public_msg(name: &str) -> String {
        format!("{}{PUBLIC_SUFFIX}", unknown_msg(name))
    }

    #[test]
    fn a_missing_variable_ends_the_expression_with_zero() {
        let r = ran("v.a = 1; v.b = v.nope; v.c = 1; return 5;", |_| {});
        assert_eq!(r.value, NV::Float(0.0));
        assert_eq!(r.msgs, [unknown_msg("variable.nope")]);
        assert_eq!(r.var("a"), Some(&NV::Float(1.0)));
        assert!(r.var("b").is_none());
        assert!(r.var("c").is_none());
    }

    #[test]
    fn a_miss_in_the_middle_of_an_expression_ends_it_whatever_it_would_give() {
        let r = ran("v.x * 10 + v.nope + 1", |env| set(env, &[("x", 2.0)]));
        assert_eq!(r.value, NV::Float(0.0));
        assert_eq!(r.msgs, [unknown_msg("variable.nope")]);
    }

    #[test]
    fn the_message_names_the_variable_temp_or_context() {
        let r = ran("v.nope", |_| {});
        assert_eq!(r.msgs, [unknown_msg("variable.nope")]);
        let r = ran("t.nope = t.nope; return 1;", |_| {});
        assert_eq!(r.msgs, [unknown_msg("temp.nope")]);
        assert_eq!(r.value, NV::Float(0.0));
        let r = ran("c.nope", |_| {});
        assert_eq!(r.msgs, [unknown_msg("context.nope")]);
        // Names are lower case.
        let r = ran("V.NoPe", |_| {});
        assert_eq!(r.msgs, [unknown_msg("variable.nope")]);
    }

    #[test]
    fn a_present_variable_of_any_kind_does_not_miss() {
        let r = ran("v.a", |env| {
            env.variables.set(key("a"), NV::string("moo"));
        });
        assert_eq!(r.value, NV::string("moo"));
        assert!(r.msgs.is_empty());
    }

    #[test]
    fn a_missing_variable_is_logged_once_per_evaluation() {
        let expr = compile_ok("v.nope + v.nope");
        let mut env = NoHostEnv::new();
        expr.eval(&mut env.cx());
        assert_eq!(env.sink.take().len(), 1);
        expr.eval(&mut env.cx());
        assert_eq!(env.sink.take().len(), 1);
    }

    #[test]
    fn a_miss_leaves_the_evaluation_unwound() {
        // A miss inside a loop with an operand pushed leaves nothing for the next evaluation.
        let expr = compile_ok("loop(3, { v.n = v.n + (v.x + v.nope); });");
        let mut env = NoHostEnv::new();
        set(&mut env, &[("n", 0.0), ("x", 1.0)]);
        assert_eq!(expr.eval(&mut env.cx()), NV::ZERO);
        assert_eq!(env.sink.take(), [unknown_msg("variable.nope")]);
        assert_eq!(env.variables.get(key("n")), Some(&NV::Float(0.0)));
        assert_eq!(expr.eval(&mut env.cx()), NV::ZERO);
    }

    #[test]
    fn a_handler_catches_a_miss_and_continues_with_its_right_side() {
        let r = ran("v.nope ?? 7", |_| {});
        assert_eq!(r.value, NV::Float(7.0));
        assert!(r.msgs.is_empty());
        let r = ran("v.x ?? v.nope", |env| set(env, &[("x", 3.0)]));
        assert_eq!(r.value, NV::Float(3.0));
        assert!(r.msgs.is_empty());
    }

    #[test]
    fn the_coalescing_post_op_applies_to_either_side() {
        let r = ran("(v.nope ?? 2) * 3", |_| {});
        assert_eq!(r.value, NV::Float(6.0));
        let r = ran("(v.x ?? 2) * 3", |env| set(env, &[("x", 5.0)]));
        assert_eq!(r.value, NV::Float(15.0));
    }

    #[test]
    fn handlers_nest_on_the_right_hand_side() {
        let src = "v.a ?? (v.b ?? 3)";
        assert_eq!(ran(src, |_| {}).value, NV::Float(3.0));
        assert_eq!(
            ran(src, |env| set(env, &[("b", 5.0)])).value,
            NV::Float(5.0)
        );
        assert_eq!(
            ran(src, |env| set(env, &[("a", 9.0), ("b", 5.0)])).value,
            NV::Float(9.0)
        );
        assert!(ran(src, |_| {}).msgs.is_empty());
    }

    #[test]
    fn a_handler_protects_only_its_left_side() {
        let r = ran("v.a ?? v.b", |_| {});
        assert_eq!(r.value, NV::Float(0.0));
        assert_eq!(r.msgs, [unknown_msg("variable.b")]);
    }

    #[test]
    fn a_handler_is_dropped_when_its_left_side_succeeds() {
        let r = ran("(v.x ?? 1) + v.nope", |env| set(env, &[("x", 4.0)]));
        assert_eq!(r.value, NV::Float(0.0));
        assert_eq!(r.msgs, [unknown_msg("variable.nope")]);
    }

    #[test]
    fn a_handler_unwinds_the_operand_stack_to_where_it_was_registered() {
        let r = ran("v.x + (v.nope ?? 1)", |env| set(env, &[("x", 2.0)]));
        assert_eq!(r.value, NV::Float(3.0));
        assert!(r.msgs.is_empty());
        let r = ran("v.x * v.y + (v.nope ?? v.y) * (v.nope ?? 2)", |env| {
            set(env, &[("x", 2.0), ("y", 5.0)]);
        });
        assert_eq!(r.value, NV::Float(10.0 + 5.0 * 2.0));
    }

    #[test]
    fn a_handler_inside_a_loop_catches_the_miss_of_every_pass() {
        let r = ran(
            "loop(3, { v.n = v.n + (v.nope ?? 1); }); return v.n;",
            |env| set(env, &[("n", 0.0)]),
        );
        assert_eq!(r.value, NV::Float(3.0));
        assert!(r.msgs.is_empty());
        let r = ran(
            "loop(4, { v.n = v.n + (v.m ?? 1); v.m = 10; }); return v.n;",
            |env| set(env, &[("n", 0.0)]),
        );
        assert_eq!(r.value, NV::Float(1.0 + 10.0 + 10.0 + 10.0));
    }

    #[test]
    fn a_handler_catches_a_missing_temp_and_a_missing_context_value() {
        let r = ran("t.nope ?? 4", |_| {});
        assert_eq!(r.value, NV::Float(4.0));
        assert!(r.msgs.is_empty());
        let r = ran("c.nope ?? 5", |_| {});
        assert_eq!(r.value, NV::Float(5.0));
        assert!(r.msgs.is_empty());
    }

    #[test]
    fn a_handler_costs_the_same_steps_whether_it_catches_or_not() {
        assert_eq!(steps_needed("v.x ?? 1", |env| set(env, &[("x", 1.0)])), 4);
        assert_eq!(steps_needed("v.x ?? 1", |_| {}), 4);
    }

    fn with_struct(env: &mut NoHostEnv) {
        env.variables
            .set(key("s"), NV::structure(StructValue::from([("a", 1.0)])));
    }

    #[test]
    fn a_missing_member_logs_two_messages_and_ends_the_expression() {
        let r = ran("v.s.b + 1", with_struct);
        assert_eq!(r.value, NV::Float(0.0));
        assert_eq!(
            r.msgs,
            [
                "Error: unable to find member variable .b".to_owned(),
                unknown_msg(".b")
            ]
        );
    }

    #[test]
    fn a_present_member_reads() {
        let r = ran("v.s.a * 4", with_struct);
        assert_eq!(r.value, NV::Float(4.0));
        assert!(r.msgs.is_empty());
    }

    #[test]
    fn a_missing_member_is_named_by_the_last_member_of_the_path() {
        // `.b` is the one missing, but the path ends in `.c`.
        let r = ran("v.s.b.c", with_struct);
        assert_eq!(
            r.msgs,
            [
                "Error: unable to find member variable .c".to_owned(),
                unknown_msg(".c")
            ]
        );
        // `.a` is a float: it has no `.z`.
        let r = ran("v.s.a.z", with_struct);
        assert_eq!(
            r.msgs,
            [
                "Error: unable to find member variable .z".to_owned(),
                unknown_msg(".z")
            ]
        );
    }

    #[test]
    fn reading_a_member_of_a_non_struct_misses_it() {
        let r = ran("v.f.x", |env| set(env, &[("f", 3.0)]));
        assert_eq!(r.value, NV::Float(0.0));
        assert_eq!(
            r.msgs,
            [
                "Error: unable to find member variable .x".to_owned(),
                unknown_msg(".x")
            ]
        );
    }

    #[test]
    fn a_missing_member_with_a_handler_logs_only_the_member_message() {
        // A handler cannot be written around a member read, so assemble one.
        let mut program = with_names(
            asm(vec![
                Instr::HandlerPush { to: 4 },
                Instr::LoadVar {
                    n: NameIdx(0),
                    p: PLAIN,
                },
                Instr::Member {
                    m: MemberIdx(0),
                    p: PLAIN,
                },
                Instr::HandlerPop { to: 5 },
                Instr::Const { c: ConstIdx(0) },
                Instr::End,
            ]),
            &["variable.s"],
            &[],
        );
        program.consts = vec![7.0].into_boxed_slice();
        program.members = vec![member_entry("b", "b")].into_boxed_slice();
        let mut env = NoHostEnv::new();
        with_struct(&mut env);
        let (value, _) = run_general(&program, &mut env, 100);
        assert_eq!(value, NV::Float(7.0));
        assert_eq!(
            env.sink.take(),
            ["Error: unable to find member variable .b"]
        );
    }

    /// A handler jumping to `to` that unwinds to `stack` operands and `loops` frames.
    fn handler(to: u32, stack: usize, loops: usize) -> Handler {
        Handler {
            to,
            mark: Mark {
                stack,
                loops,
                arrows: 0,
            },
        }
    }

    fn caller() -> Subjects<NoHost> {
        Subjects {
            this: 1.0,
            ..Subjects::none()
        }
    }

    #[test]
    fn missing_jumps_to_the_innermost_handler_and_unwinds_to_it() {
        let program = asm(vec![Instr::End]);
        let mut env = NoHostEnv::new();
        let mut cx = env.cx();
        let mut vm = Vm::<NoHost, NV>::new(&program, Some(100));
        vm.stack
            .extend([NV::Float(1.0), NV::Float(2.0), NV::Float(3.0)]);
        vm.acc = NV::Float(9.0);
        vm.handlers.push(handler(7, 1, 0));
        vm.arrows.push(caller());
        cx.subjects = Subjects {
            this: 2.0,
            ..Subjects::none()
        };
        assert_eq!(vm.missing(&mut cx, Base::ROOT, "variable.x").ok(), Some(7));
        assert_eq!(vm.stack.as_slice(), [NV::Float(1.0)]);
        assert!(vm.handlers.is_empty());
        assert!(vm.arrows.is_empty());
        assert_eq!(vm.acc, NV::Float(0.0));
        assert_eq!(cx.subjects, caller());
        assert!(env.sink.is_empty());
    }

    #[test]
    fn the_innermost_of_several_handlers_catches() {
        let program = asm(vec![Instr::End]);
        let mut env = NoHostEnv::new();
        let mut cx = env.cx();
        let mut vm = Vm::<NoHost, NV>::new(&program, Some(100));
        for to in [3, 5] {
            vm.handlers.push(handler(to, 0, 0));
        }
        assert_eq!(vm.missing(&mut cx, Base::ROOT, "variable.x").ok(), Some(5));
        assert_eq!(vm.handlers.len(), 1);
        assert_eq!(vm.missing(&mut cx, Base::ROOT, "variable.x").ok(), Some(3));
        assert_eq!(vm.missing(&mut cx, Base::ROOT, "variable.x").ok(), None);
    }

    #[test]
    fn a_handler_below_the_base_belongs_to_the_caller_and_does_not_catch() {
        let program = asm(vec![Instr::End]);
        let mut env = NoHostEnv::new();
        let mut cx = env.cx();
        let mut vm = Vm::<NoHost, NV>::new(&program, Some(100));
        vm.handlers.push(handler(7, 0, 0));
        let base = vm.base();
        assert_eq!(base.handlers, 1);
        assert_eq!(vm.missing(&mut cx, base, "variable.x").ok(), None);
        assert_eq!(
            vm.handlers.len(),
            1,
            "the caller's handler is still registered"
        );
        assert_eq!(env.sink.take(), [unknown_msg("variable.x")]);
    }

    #[test]
    fn inside_an_arrow_the_message_explains_public_variables() {
        let program = asm(vec![Instr::End]);
        let mut env = NoHostEnv::new();
        let mut cx = env.cx();
        let mut vm = Vm::<NoHost, NV>::new(&program, Some(100));
        vm.arrows.push(caller());
        assert_eq!(vm.missing(&mut cx, Base::ROOT, "temp.t").ok(), None);
        assert_eq!(env.sink.take(), [unknown_public_msg("temp.t")]);
    }

    #[test]
    fn base_records_the_four_depths_and_unwind_restores_them() {
        let program = asm(vec![Instr::End]);
        let mut env = NoHostEnv::new();
        let mut cx = env.cx();
        let mut vm = Vm::<NoHost, NV>::new(&program, Some(100));
        vm.stack.push(NV::Float(1.0));
        vm.loops.push(Frame::Count { iterations: 1 });
        vm.handlers.push(handler(0, 0, 0));
        vm.arrows.push(caller());
        let base = vm.base();
        let mark = Mark {
            stack: 1,
            loops: 1,
            arrows: 1,
        };
        assert_eq!((base.mark, base.handlers), (mark, 1));
        vm.stack.extend([NV::Float(2.0), NV::Float(3.0)]);
        vm.loops.push(Frame::Count { iterations: 1 });
        vm.handlers.push(handler(0, 0, 0));
        vm.arrows.push(Subjects {
            this: 5.0,
            ..Subjects::none()
        });
        cx.subjects = Subjects {
            this: 6.0,
            ..Subjects::none()
        };
        vm.unwind(&mut cx, base);
        assert_eq!(
            (
                vm.stack.len(),
                vm.loops.len(),
                vm.handlers.len(),
                vm.arrows.len()
            ),
            (1, 1, 1, 1)
        );
        assert_eq!(
            cx.subjects,
            Subjects {
                this: 5.0,
                ..Subjects::none()
            }
        );
        vm.unwind(&mut cx, Base::ROOT);
        assert_eq!(
            (
                vm.stack.len(),
                vm.loops.len(),
                vm.handlers.len(),
                vm.arrows.len()
            ),
            (0, 0, 0, 0)
        );
        assert_eq!(cx.subjects, caller());
    }

    /// Actor 1 holds `p` = actor 2, who has a public `x` = 5 and a private `y` = 6.
    fn arrow_world() -> HostEnv<TestHost, VariableStorage<TestHost>> {
        let mut store = VariableStorage::<TestHost>::new();
        store.actor_mut(1).set(key("p"), HV::Actor(2));
        store.actor_mut(2).set_public(key("x"), HV::Float(5.0));
        store.actor_mut(2).set(key("y"), HV::Float(6.0));
        store.refresh_snapshots();
        HostEnv::<TestHost>::default().with_variables(store)
    }

    #[test]
    fn an_arrow_reads_the_public_snapshot_of_the_other_actor() {
        let r = host_ran("v.p->v.x", arrow_world());
        assert_eq!(r.value, HV::Float(5.0));
        assert!(r.msgs.is_empty());
        let r = host_ran("v.p->v.x * 2 + 1", arrow_world());
        assert_eq!(r.value, HV::Float(11.0));
    }

    #[test]
    fn an_absent_or_private_variable_reads_zero_inside_an_arrow_and_the_expression_continues() {
        for src in ["v.p->v.y", "v.p->v.zzz"] {
            let r = host_ran(src, arrow_world());
            assert_eq!(r.value, HV::Float(0.0), "{src}");
            assert!(r.msgs.is_empty(), "{src}: {:?}", r.msgs);
        }
        let r = host_ran("v.r = v.p->v.zzz; return 9;", arrow_world());
        assert_eq!(r.value, HV::Float(9.0));
        assert!(r.msgs.is_empty());
        assert_eq!(
            r.env.variables.actor(1).unwrap().get(key("r")),
            Some(&HV::Float(0.0))
        );
    }

    #[test]
    fn an_arrow_to_nothing_is_the_post_op_of_zero_and_skips_the_right_side() {
        // An absent left side misses and aborts.
        let r = host_ran("v.zz->v.x", arrow_world());
        assert_eq!(r.value, HV::Float(0.0));
        assert_eq!(r.msgs, [unknown_msg("variable.zz")]);
        // A float where an actor belongs.
        let expr = compile_ok("v.f->v.x + 1");
        let mut env = arrow_world();
        env.variables.actor_mut(1).set(key("f"), HV::Float(3.0));
        let mut world = TestHost;
        let value = expr.eval(&mut env.cx(&mut world, Subjects::actor(1)));
        assert_eq!(value, HV::Float(1.0));
        assert!(env.sink.is_empty());
    }

    #[test]
    fn a_temp_miss_inside_an_arrow_uses_the_longer_message() {
        let mut queries = QueryTable::new(crate::stdlib::queries(Side::Server));
        queries.set(query::POSITION, first_arg).unwrap();
        let expr = compile_ok("v.p->q.position(t.nope)");
        let mut env = HostEnv::<TestHost>::new(queries).with_variables(arrow_world().variables);
        let mut world = TestHost;
        let value = expr.eval(&mut env.cx(&mut world, Subjects::actor(1)));
        // The argument ends with 0 and the call goes on.
        assert_eq!(value, HV::Float(0.0));
        assert_eq!(env.sink.take(), [unknown_public_msg("temp.nope")]);
    }

    #[test]
    fn a_context_miss_inside_an_arrow_uses_the_longer_message() {
        let mut queries = QueryTable::new(crate::stdlib::queries(Side::Server));
        queries.set(query::POSITION, first_arg).unwrap();
        let expr = compile_ok("v.p->q.position(c.nope)");
        let mut env = HostEnv::<TestHost>::new(queries).with_variables(arrow_world().variables);
        let mut world = TestHost;
        expr.eval(&mut env.cx(&mut world, Subjects::actor(1)));
        assert_eq!(env.sink.take(), [unknown_public_msg("context.nope")]);
    }

    #[test]
    fn the_subjects_are_restored_after_every_way_out_of_an_arrow() {
        let expr = compile_ok("v.p->v.x");
        for steps in 1..=8 {
            let mut env = arrow_world();
            env.limits.total_steps = Some(steps);
            let mut world = TestHost;
            let mut cx = env.cx(
                &mut world,
                Subjects {
                    this: 4.0,
                    ..Subjects::actor(1)
                },
            );
            let before = cx.subjects;
            expr.eval(&mut cx);
            assert_eq!(cx.subjects, before, "budget {steps}");
        }
    }

    #[test]
    fn a_budget_inside_an_arrow_leaves_the_arrow_open_until_the_evaluation_unwinds() {
        let program = compile_ok("v.p->v.x");
        let program = program.program().unwrap();
        let mut env = arrow_world();
        let mut world = TestHost;
        let mut cx = env.cx(&mut world, Subjects::actor(1));
        let before = cx.subjects;
        let mut vm = Vm::<TestHost, HV>::new(program, Some(3));
        // load p, pointer-enter, load x: the budget ends the third.
        assert!(matches!(
            vm.run(program, &mut cx, 0, Base::ROOT),
            Err(Stop::Abort)
        ));
        assert_eq!(vm.arrows.len(), 1);
        assert_ne!(cx.subjects, before);
        vm.unwind(&mut cx, Base::ROOT);
        assert_eq!(cx.subjects, before);
        assert!(vm.arrows.is_empty());
    }

    #[test]
    fn pointer_leave_without_enter_reports_the_underflow() {
        let p = asm(vec![Instr::PointerLeave { p: PLAIN }, Instr::End]);
        let mut env = NoHostEnv::new();
        assert_eq!(run_general(&p, &mut env, 100).0, NV::Float(0.0));
        assert_eq!(
            env.sink.take(),
            ["molangx: a public-access scope was closed while none was open"]
        );
    }

    #[test]
    fn pointer_enter_on_a_non_actor_skips_the_right_side() {
        let mut p = asm(vec![
            Instr::Const { c: ConstIdx(0) },
            Instr::PointerEnter {
                to: 4,
                p: PostIdx(1),
            },
            Instr::Const { c: ConstIdx(1) },
            Instr::PointerLeave { p: PostIdx(1) },
            Instr::End,
        ]);
        p.consts = vec![1.0, 9.0].into_boxed_slice();
        p.posts = vec![PostOp::IDENTITY, PostOp::new(2.0, 3.0)].into_boxed_slice();
        let mut env = NoHostEnv::new();
        let mut cx = env.cx();
        let before = cx.subjects;
        let mut vm = Vm::<NoHost, NV>::new(&p, Some(100));
        vm.run(&p, &mut cx, 0, Base::ROOT).unwrap();
        // The failed enter loads the post-op of 0: 2·0 + 3.
        assert_eq!(vm.acc, NV::Float(3.0));
        assert!(vm.arrows.is_empty());
        assert_eq!(cx.subjects, before);
    }

    fn call_program() -> Program {
        // handler-push -> 4; call q0; handler-pop -> 5; (unused); const 99; end.
        let mut p = with_names(
            asm(vec![
                Instr::HandlerPush { to: 4 },
                Instr::Call {
                    q: CallIdx(0),
                    p: PLAIN,
                },
                Instr::HandlerPop { to: 5 },
                Instr::End,
                Instr::Const { c: ConstIdx(0) },
                Instr::End,
                // the argument: variable.nope
                Instr::LoadVar {
                    n: NameIdx(0),
                    p: PLAIN,
                },
                Instr::End,
            ]),
            &["variable.nope"],
            &[],
        );
        p.consts = vec![99.0].into_boxed_slice();
        p.calls = vec![QueryCall {
            index: stdlib_index(query::POSITION),
            impl_idx: 0,
            args: vec![6].into_boxed_slice(),
        }]
        .into_boxed_slice();
        p
    }

    #[test]
    fn a_handler_outside_a_query_does_not_catch_a_miss_in_its_argument() {
        let p = call_program();
        let mut env = server_env();
        table(&mut env).set(query::POSITION, first_arg).unwrap();
        let (value, _) = run_general(&p, &mut env, 100);
        // The argument ended with 0 and the call went on: the handler never ran.
        assert_eq!(value, NV::Float(0.0));
        assert_eq!(env.sink.take(), [unknown_msg("variable.nope")]);
    }

    #[test]
    fn an_argument_that_misses_ends_alone_and_the_caller_goes_on() {
        let r = ran("v.a + q.position(v.nope)", |env| {
            table(env).set(query::POSITION, first_arg).unwrap();
            set(env, &[("a", 4.0)]);
        });
        assert_eq!(r.value, NV::Float(4.0));
        assert_eq!(r.msgs, [unknown_msg("variable.nope")]);
    }

    #[test]
    fn an_argument_may_use_its_own_handler() {
        let r = ran("v.a + q.position(v.nope ?? 3)", |env| {
            table(env).set(query::POSITION, first_arg).unwrap();
            set(env, &[("a", 4.0)]);
        });
        assert_eq!(r.value, NV::Float(7.0));
        assert!(r.msgs.is_empty());
    }

    #[test]
    fn a_catching_handler_drops_the_loop_frames_opened_after_it() {
        let program = asm(vec![Instr::End]);
        let mut env = NoHostEnv::new();
        let mut cx = env.cx();
        let mut vm = Vm::<NoHost, NV>::new(&program, Some(100));
        vm.loops.push(Frame::Count { iterations: 1 });
        vm.handlers.push(handler(3, 0, 1));
        vm.loops.push(Frame::Count { iterations: 1 });
        vm.loops.push(Frame::Count { iterations: 1 });
        assert_eq!(vm.missing(&mut cx, Base::ROOT, "variable.x").ok(), Some(3));
        assert_eq!(vm.loops.len(), 1);
    }
}
