//! The machine's construction, step budget and member stores.

use smallvec::SmallVec;

use super::{Aborted, Slot, Stop, Vm};
use crate::compile::program::{NameIdx, PostIdx, Program, StoreIdx, StoreRoot, TempIdx};
use crate::hash::HashedStr;
use crate::vm::{cx::EvalCx, host::Host, sink::RuntimeMsg, value::Value};

#[allow(clippy::inline_always)] // each instruction method has one call site, in the hot loop `Vm::run`, which must not call it
impl<H: Host, R: Slot<H>> Vm<H, R> {
    /// `total_steps: None` counts down from `u64::MAX`, which one step per nanosecond spends in
    /// 584 years.
    pub(super) fn new(program: &Program, total_steps: Option<u64>) -> Self {
        let mut temps = SmallVec::new();
        // Most programs have no temps: skipping the resize keeps their start-up short.
        if !program.temps.is_empty() {
            temps.resize(program.temps.len(), None);
        }
        Self {
            acc: R::float(0.0),
            stack: SmallVec::new(),
            loops: SmallVec::new(),
            handlers: SmallVec::new(),
            arrows: SmallVec::new(),
            temps,
            steps_left: total_steps.unwrap_or(u64::MAX),
            stopped: false,
            loop_budget_logged: false,
            arg_depth: 0,
        }
    }

    #[inline]
    pub(super) fn pop(&mut self) -> R {
        self.stack.pop().unwrap_or_else(|| R::float(0.0))
    }

    /// The plain post-op keeps `value`; any other gives the float `value·S + O`. `None`:
    /// deoptimise.
    #[inline]
    pub(super) fn with_post(program: &Program, value: &Value<H>, p: PostIdx) -> Option<R> {
        if p == PostIdx::PLAIN {
            R::from_ref(value)
        } else {
            Some(R::float(program.post(p).apply(value.as_f32())))
        }
    }

    #[cold]
    pub(super) fn budget_spent(&mut self, cx: &mut EvalCx<'_, '_, H>, limit: u64) {
        self.stopped = true;
        cx.sink.runtime(RuntimeMsg::StepLimit { limit });
    }

    /// Charges `cost` steps beyond the instruction's own; when the budget cannot pay them the
    /// evaluation stops.
    #[inline]
    pub(super) fn charge(&mut self, cx: &mut EvalCx<'_, '_, H>, cost: u64) -> Result<(), Aborted> {
        if cost <= self.steps_left {
            self.steps_left -= cost;
            return Ok(());
        }
        self.steps_left = 0;
        self.budget_spent(cx, cx.limits.total_steps.unwrap_or(u64::MAX));
        Err(Aborted)
    }

    /// Pays the step of the instruction about to run (or of a fused constant's second half); when
    /// the budget is spent the evaluation stops.
    #[inline(always)]
    pub(super) fn step(&mut self, cx: &mut EvalCx<'_, '_, H>, limit: u64) -> Result<(), Aborted> {
        if self.steps_left == 0 {
            self.budget_spent(cx, limit);
            return Err(Aborted);
        }
        self.steps_left -= 1;
        Ok(())
    }

    /// Charges the store cost of the accumulator.
    #[inline(always)]
    fn charge_store(&mut self, cx: &mut EvalCx<'_, '_, H>) -> Result<(), Aborted> {
        match self.acc.store_cost() {
            0 => Ok(()),
            cost => self.charge(cx, cost),
        }
    }

    /// Stores the raw accumulator, then applies the post-op to it.
    #[inline(always)]
    pub(super) fn store_var(
        &mut self,
        program: &Program,
        cx: &mut EvalCx<'_, '_, H>,
        n: NameIdx,
        p: PostIdx,
    ) -> Result<(), Aborted> {
        self.charge_store(cx)?;
        cx.set_variable(program.variable_name(n), self.acc.to_value());
        self.post_in_place(program, p);
        Ok(())
    }

    /// [`Self::store_var`] for temp slot `t`.
    #[inline(always)]
    pub(super) fn store_temp_instr(
        &mut self,
        program: &Program,
        cx: &mut EvalCx<'_, '_, H>,
        t: TempIdx,
        p: PostIdx,
        at: usize,
    ) -> Result<(), Stop<H>> {
        self.charge_store(cx)?;
        self.store_temp(program, cx, t, at)?;
        self.post_in_place(program, p);
        Ok(())
    }

    /// [`Self::store_var`] for a member path.
    #[inline(always)]
    pub(super) fn store_member(
        &mut self,
        program: &Program,
        cx: &mut EvalCx<'_, '_, H>,
        s: StoreIdx,
        p: PostIdx,
        at: usize,
    ) -> Result<(), Stop<H>> {
        let store = program.store(s);
        let value = self.acc.to_value();
        match store.root {
            StoreRoot::Var(n) => {
                let key = program.variable_name(n);
                let whole = cx.variable(key).cloned().unwrap_or_default();
                let whole = self.member_store(cx, whole, &store.path, value)?;
                cx.set_variable(key, whole);
            }
            StoreRoot::Temp(t) => {
                let whole = self.temp_value(program, cx, t).unwrap_or_default();
                let whole = self.member_store(cx, whole, &store.path, value)?;
                self.set_temp(program, cx, t, whole, at)?;
            }
            StoreRoot::Other => {}
        }
        self.post_in_place(program, p);
        Ok(())
    }

    /// Stops the evaluation, writing nothing, when a budget is spent. In order: the cost is
    /// charged, widening a struct past
    /// [`EvalLimits::struct_members`](crate::vm::EvalLimits::struct_members) stops, the value is
    /// written, and a result nested past
    /// [`EvalLimits::struct_depth`](crate::vm::EvalLimits::struct_depth) stops.
    #[inline(never)]
    pub(super) fn member_store(
        &mut self,
        cx: &mut EvalCx<'_, '_, H>,
        mut whole: Value<H>,
        path: &[HashedStr],
        value: Value<H>,
    ) -> Result<Value<H>, Aborted> {
        let check = whole.member_store_check(path, cx.limits.struct_members);
        self.charge(cx, check.cost.saturating_add(value.store_cost()))?;
        if let Some(limit) = check.exceeded_width {
            self.stopped = true;
            cx.sink.runtime(RuntimeMsg::StructMemberLimit { limit });
            return Err(Aborted);
        }
        whole.set_member_path(path, cx.storable(value));
        if let Some(limit) = cx.limits.struct_depth
            && whole.struct_depth() > limit
        {
            self.stopped = true;
            cx.sink.runtime(RuntimeMsg::StructDepthLimit { limit });
            return Err(Aborted);
        }
        Ok(whole)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compile::program::{ConstIdx, Fn3, Instr, MemberStore};
    use crate::numeric::PostOp;
    use crate::stdlib::query;
    use crate::vm::eval::test_support::*;
    use crate::vm::eval::{Base, Stop};
    use crate::vm::name::ContextName;
    use crate::vm::{EvalLimits, NoHost, NoHostEnv, StructValue};

    const WIDTH_MSG: &str =
        "molangx: evaluation stopped: a struct would hold more than its budget of 2 members";

    #[test]
    fn a_store_that_nests_past_the_depth_budget_ends_the_evaluation() {
        let r = ran(
            "v.a = 1; v.s.x.y.z = 1; v.b = 2; return 5;",
            limited(Some(2), None),
        );
        assert_eq!(r.value, NV::Float(0.0));
        assert_eq!(r.msgs, [DEPTH_MSG]);
        assert_eq!(r.var("a"), Some(&NV::Float(1.0)));
        assert!(r.var("s").is_none());
        assert!(r.var("b").is_none());
    }

    #[test]
    fn a_store_within_the_depth_budget_goes_through() {
        let r = ran("v.s.x.y = 1; return 5;", limited(Some(2), None));
        assert_eq!(r.value, NV::Float(5.0));
        assert!(r.msgs.is_empty());
        assert_eq!(r.var("s").map(Value::struct_depth), Some(2));
    }

    #[test]
    fn the_depth_budget_counts_what_the_variable_already_holds() {
        let r = ran("v.s.x.y = 1; v.s.a.b = 2; return 5;", |env| {
            limited(Some(2), None)(env);
        });
        assert_eq!(r.value, NV::Float(5.0));
        // Deepening an existing branch is refused.
        let r = ran(
            "v.s.x.y = 1; v.s.x.y.z = 2; v.k = 1;",
            limited(Some(2), None),
        );
        assert_eq!(r.msgs, [DEPTH_MSG]);
        assert!(r.var("k").is_none());
        assert_eq!(
            r.var("s").map(Value::struct_depth),
            Some(2),
            "the first store is kept"
        );
    }

    #[test]
    fn a_temp_store_is_checked_too() {
        let r = ran("t.s.x.y.z = 1; v.k = 1;", limited(Some(2), None));
        assert_eq!(r.value, NV::Float(0.0));
        assert_eq!(r.msgs, [DEPTH_MSG]);
        assert!(r.var("k").is_none());
    }

    #[test]
    fn the_default_depth_budget_is_32() {
        let deep = |levels: usize| format!("v.s{} = 1; return 1;", ".m".repeat(levels));
        assert_eq!(ran(&deep(32), |_| {}).value, NV::Float(1.0));
        let r = ran(&deep(33), |_| {});
        assert_eq!(r.value, NV::Float(0.0));
        assert_eq!(
            r.msgs,
            [
                "molangx: evaluation stopped: a struct would nest deeper than its budget of 32 levels"
            ]
        );
    }

    #[test]
    fn no_limits_have_no_depth_budget() {
        let deep = format!("v.s{} = 1; return 1;", ".m".repeat(100));
        let r = ran(&deep, |env| env.limits = EvalLimits::NONE);
        assert_eq!(r.value, NV::Float(1.0));
        assert!(r.msgs.is_empty());
    }

    #[test]
    fn a_store_that_widens_a_full_struct_ends_the_evaluation() {
        let r = ran(
            "v.s.a = 1; v.s.b = 2; v.s.c = 3; v.d = 1; return 5;",
            limited(None, Some(2)),
        );
        assert_eq!(r.value, NV::Float(0.0));
        assert_eq!(r.msgs, [WIDTH_MSG]);
        let s = r.var("s").unwrap();
        assert_eq!(s.as_struct().unwrap().len(), 2);
        assert!(r.var("d").is_none());
    }

    #[test]
    fn updating_an_existing_member_is_never_refused() {
        let r = ran(
            "v.s.a = 1; v.s.b = 2; v.s.a = 9; v.d = 1; return 5;",
            limited(None, Some(2)),
        );
        assert_eq!(r.value, NV::Float(5.0));
        assert!(r.msgs.is_empty());
        assert_eq!(
            r.var("s").unwrap().member(HashedStr::new("a")),
            Some(&NV::Float(9.0))
        );
    }

    #[test]
    fn a_struct_the_host_made_wider_than_the_budget_stays_usable() {
        let r = ran("v.s.a = 7; return v.s.a;", |env| {
            env.limits.struct_members = Some(1);
            env.variables.set(
                key("s"),
                NV::structure(StructValue::from([
                    ("x", 1.0),
                    ("y", 2.0),
                    ("z", 3.0),
                    ("a", 0.0),
                ])),
            );
        });
        assert_eq!(r.value, NV::Float(7.0));
        assert!(r.msgs.is_empty());
    }

    #[test]
    fn the_width_budget_applies_to_inner_structs() {
        let r = ran(
            "v.s.a.p = 1; v.s.a.q = 2; v.s.a.r = 3; v.k = 1;",
            limited(None, Some(2)),
        );
        assert_eq!(r.msgs, [WIDTH_MSG]);
        assert!(r.var("k").is_none());
    }

    #[test]
    fn the_default_width_budget_is_256_members() {
        let fill = |n: usize| {
            (0..n)
                .map(|i| format!("v.s.m{i} = 1;"))
                .collect::<Vec<_>>()
                .concat()
        };
        let r = ran(&format!("{} return 1;", fill(256)), |env| {
            env.limits.total_steps = None;
        });
        assert_eq!(r.value, NV::Float(1.0));
        let r = ran(&format!("{} return 1;", fill(257)), |env| {
            env.limits.total_steps = None;
        });
        assert_eq!(r.value, NV::Float(0.0));
        assert_eq!(
            r.msgs,
            [
                "molangx: evaluation stopped: a struct would hold more than its budget of 256 members"
            ]
        );
    }

    #[test]
    fn a_new_vm_has_one_empty_slot_per_temp() {
        let none = asm(vec![Instr::End]);
        assert!(Vm::<NoHost, NV>::new(&none, Some(5)).temps.is_empty());
        let three = with_names(
            asm(vec![Instr::End]),
            &["temp.a", "temp.b", "temp.c"],
            &[0, 1, 2],
        );
        let vm = Vm::<NoHost, NV>::new(&three, Some(5));
        assert_eq!(vm.temps.as_slice(), [None, None, None]);
        assert_eq!(vm.steps_left, 5);
        assert!(!vm.stopped && !vm.loop_budget_logged);
        assert_eq!(vm.acc, NV::Float(0.0));
        let vm = Vm::<NoHost, f32>::new(&three, Some(5));
        assert_eq!(vm.temps.as_slice(), [None, None, None]);
    }

    #[test]
    fn constants_and_string_literals_run_nothing() {
        let mut env = NoHostEnv::new();
        env.limits.total_steps = Some(0);
        assert_eq!(compile_ok("1 + 2").eval(&mut env.cx()), NV::Float(3.0));
        assert_eq!(compile_ok("'moo'").eval(&mut env.cx()), NV::string("moo"));
        assert_eq!(
            compile_ok("'moo'").eval_f32(&mut env.cx()).to_bits(),
            NV::string("moo").as_f32().to_bits()
        );
        assert_eq!(compile_ok("2.5").eval_f32(&mut env.cx()), 2.5);
        assert!(env.sink.is_empty());
    }

    #[test]
    fn an_expression_with_a_zero_budget_runs_nothing_and_says_so() {
        let mut env = NoHostEnv::new();
        env.limits.total_steps = Some(0);
        env.variables.set(key("x"), Value::Float(7.0));
        assert_eq!(compile_ok("v.x").eval(&mut env.cx()), NV::ZERO);
        assert_eq!(env.sink.take(), [step_msg(0)]);
        assert_eq!(compile_ok("v.x").eval_f32(&mut env.cx()), 0.0);
        assert_eq!(env.sink.take(), [step_msg(0)]);
    }

    #[test]
    fn the_step_cost_of_each_instruction_class() {
        let table: &[(&str, u64)] = &[
            ("v.x", 2),
            ("this", 2),
            ("c.foo", 2),
            ("v.x * 2", 2),
            ("-v.x", 2),
            ("!v.x", 3),
            ("v.x * v.y", 5),
            ("v.x + v.y", 5),
            ("v.x + v.y + v.z", 7),
            ("v.x - v.y", 5),
            ("v.a / v.b", 5),
            ("math.abs(v.x)", 3),
            ("math.max(v.x, v.y)", 5),
            ("math.max(v.x, 2)", 3),
            ("math.mod(v.x, 3)", 3),
            ("math.mod(v.x, v.y)", 5),
            ("math.clamp(v.x, v.y, v.z)", 7),
            ("math.random(1, 3)", 2),
            ("math.random(v.x, v.y)", 5),
            ("v.x < v.y", 5),
            ("v.x < 3", 3),
            ("v.x == v.y", 5),
            ("v.x == 'abc'", 3),
            ("v.x && v.y", 5),
            ("v.zero && v.y", 3),
            ("v.x || v.y", 3),
            ("v.zero || v.y", 5),
            ("v.x ? 1 : 2", 5),
            ("v.zero ? 1 : 2", 4),
            ("v.x ?? 1", 4),
            ("v.nope ?? 1", 4),
            ("v.q = 1;", 4),
            ("t.a = 1; return t.a;", 4),
            ("v.q = 1; v.r = 2;", 6),
        ];
        for &(src, steps) in table {
            assert_eq!(steps_needed(src, sample_vars), steps, "{src}");
        }
    }

    #[test]
    fn straight_line_code_costs_one_step_per_instruction_and_one_more_per_fused_push() {
        for src in [
            "v.x",
            "v.x * v.y",
            "v.x + v.y + v.z",
            "math.clamp(v.x, 0, 1)",
            "math.lerp(v.x, 2, 3)",
            "math.die_roll(0, 1, 6)",
            "v.a / v.b",
            "math.clamp(v.x, v.y, v.z) * 2",
        ] {
            let expr = compile_ok(src);
            let code = &expr.program().unwrap().code;
            let fused = code
                .iter()
                .filter(|i| matches!(i, Instr::PushConst { .. }))
                .count() as u64;
            assert_eq!(
                steps_needed(src, sample_vars),
                code.len() as u64 + fused,
                "{src}"
            );
        }
    }

    #[test]
    fn a_fused_push_constant_costs_the_two_steps_of_the_pair() {
        assert_eq!(steps_needed("math.clamp(v.x, 0, 1)", sample_vars), 7);
        assert_eq!(steps_needed("math.clamp(v.x, v.y, v.z)", sample_vars), 7);
        assert_eq!(steps_needed("math.lerp(v.x, 2, 3)", sample_vars), 7);
    }

    /// A program with a fused push-constant and the same program with the pair spelled out.
    fn clamp_programs() -> (Program, Program) {
        let load = Instr::LoadVar {
            n: NameIdx(0),
            p: PLAIN,
        };
        let clamp = Instr::Math3 {
            f: Fn3::Clamp,
            p: PLAIN,
        };
        let fused = vec![
            load,
            Instr::PushConst { c: ConstIdx(0) },
            Instr::PushConst { c: ConstIdx(1) },
            clamp,
            Instr::End,
        ];
        let pair = vec![
            load,
            Instr::Push,
            Instr::Const { c: ConstIdx(0) },
            Instr::Push,
            Instr::Const { c: ConstIdx(1) },
            clamp,
            Instr::End,
        ];
        let make = |code| {
            let mut p = with_names(asm(code), &["variable.x"], &[]);
            p.consts = vec![0.0, 1.0].into_boxed_slice();
            p
        };
        (make(fused), make(pair))
    }

    #[test]
    fn a_budget_ends_a_fused_push_constant_where_the_pair_would() {
        let (fused, pair) = clamp_programs();
        for budget in 0..=9 {
            let (mut env_fused, mut env_pair) = (NoHostEnv::new(), NoHostEnv::new());
            env_fused.variables.set(key("x"), Value::Float(5.0));
            env_pair.variables.set(key("x"), Value::Float(5.0));
            let (value_fused, left_fused) = run_general(&fused, &mut env_fused, budget);
            let (value_pair, left_pair) = run_general(&pair, &mut env_pair, budget);
            assert_eq!(value_fused, value_pair, "budget {budget}");
            assert_eq!(left_fused, left_pair, "budget {budget}");
            assert_eq!(
                env_fused.sink.take(),
                env_pair.sink.take(),
                "budget {budget}"
            );
            assert_eq!(
                value_fused,
                if budget >= 7 {
                    NV::Float(1.0)
                } else {
                    NV::ZERO
                },
                "budget {budget}"
            );
            assert_eq!(left_fused, budget.saturating_sub(7), "budget {budget}");
        }
    }

    #[test]
    fn the_second_step_of_a_fused_push_is_charged_between_its_halves() {
        let (fused, _) = clamp_programs();
        let mut env = NoHostEnv::new();
        env.limits.total_steps = Some(3);
        env.variables.set(key("x"), Value::Float(5.0));
        // Budget 3 pays the load, the first push and its constant; the next instruction finds it
        // spent.
        let mut vm = Vm::<NoHost, NV>::new(&fused, Some(3));
        let mut cx = env.cx();
        assert!(matches!(
            vm.run(&fused, &mut cx, 0, Base::ROOT),
            Err(Stop::Abort)
        ));
        assert_eq!(vm.steps_left, 0);
        assert_eq!(vm.acc, NV::Float(0.0), "the first constant was loaded");
        assert_eq!(vm.stack.as_slice(), [NV::Float(5.0)], "the push half ran");
        assert!(vm.stopped);
        assert_eq!(env.sink.take(), [step_msg(3)]);
        // Budget 2: the push half runs, the constant's step is not paid.
        let mut env = NoHostEnv::new();
        env.limits.total_steps = Some(2);
        env.variables.set(key("x"), Value::Float(5.0));
        let mut vm = Vm::<NoHost, NV>::new(&fused, Some(2));
        let mut cx = env.cx();
        assert!(matches!(
            vm.run(&fused, &mut cx, 0, Base::ROOT),
            Err(Stop::Abort)
        ));
        assert!(vm.stopped);
        assert_eq!(vm.stack.len(), 1);
        assert_eq!(vm.acc, NV::Float(5.0), "the constant was not loaded");
        assert_eq!(env.sink.take(), [step_msg(2)]);
    }

    #[test]
    fn charge_pays_up_to_the_remaining_budget() {
        let program = asm(vec![Instr::End]);
        let mut env = NoHostEnv::new();
        env.limits.total_steps = Some(77);
        let mut cx = env.cx();
        let mut vm = Vm::<NoHost, NV>::new(&program, Some(10));
        assert!(vm.charge(&mut cx, 4).is_ok());
        assert_eq!(vm.steps_left, 6);
        assert!(vm.charge(&mut cx, 6).is_ok());
        assert_eq!(vm.steps_left, 0);
        assert!(vm.charge(&mut cx, 0).is_ok());
        assert!(!vm.stopped);
        assert!(env.sink.is_empty());
    }

    #[test]
    fn charge_past_the_budget_spends_it_stops_and_reports_the_whole_budget() {
        let program = asm(vec![Instr::End]);
        let mut env = NoHostEnv::new();
        env.limits.total_steps = Some(77);
        let mut cx = env.cx();
        let mut vm = Vm::<NoHost, NV>::new(&program, Some(10));
        assert!(vm.charge(&mut cx, 11).is_err());
        assert_eq!(vm.steps_left, 0);
        assert!(vm.stopped);
        assert_eq!(env.sink.take(), [step_msg(77)]);
    }

    fn path(names: &[&str]) -> Vec<HashedStr> {
        names.iter().map(|n| HashedStr::new(n)).collect()
    }

    #[test]
    fn a_member_store_charges_four_per_new_level_and_writes() {
        let program = asm(vec![Instr::End]);
        let mut env = NoHostEnv::new();
        let mut cx = env.cx();
        let mut vm = Vm::<NoHost, NV>::new(&program, Some(100));
        let whole = vm
            .member_store(&mut cx, NV::ZERO, &path(&["a", "b"]), NV::Float(1.0))
            .unwrap();
        assert_eq!(vm.steps_left, 100 - 2 * EvalLimits::STRUCT_COPY_STEPS);
        assert_eq!(whole.member_path(&path(&["a", "b"])), Some(&NV::Float(1.0)));
        let mut vm = Vm::<NoHost, NV>::new(&program, Some(100));
        let whole = vm
            .member_store(&mut cx, whole, &path(&["a", "c"]), NV::Float(2.0))
            .unwrap();
        // Each struct also pays its members: `whole` holds {a}, `a` holds {b}: (4 + 1) + (4 + 1).
        assert_eq!(vm.steps_left, 100 - 10);
        assert_eq!(whole.member_path(&path(&["a", "c"])), Some(&NV::Float(2.0)));
        assert_eq!(whole.member_path(&path(&["a", "b"])), Some(&NV::Float(1.0)));
    }

    #[test]
    fn a_member_store_of_an_actor_array_also_pays_one_step_per_entry() {
        let program = asm(vec![Instr::End]);
        let mut env = NoHostEnv::new();
        let mut cx = env.cx();
        let mut vm = Vm::<NoHost, NV>::new(&program, Some(100));
        let herd = NV::actor_array([(); 3]);
        assert_eq!(herd.store_cost(), 3);
        vm.member_store(&mut cx, NV::ZERO, &path(&["a"]), herd)
            .unwrap();
        assert_eq!(vm.steps_left, 100 - 4 - 3);
    }

    #[test]
    fn a_member_store_checks_its_cost_before_the_width_and_depth() {
        let program = asm(vec![Instr::End]);
        let mut env = NoHostEnv {
            limits: EvalLimits {
                struct_members: Some(0),
                struct_depth: Some(1),
                total_steps: Some(50),
                ..EvalLimits::DEFAULT
            },
            ..NoHostEnv::new()
        };
        let mut cx = env.cx();
        // The budget cannot pay the 4 steps: the step message wins over width and depth.
        let mut vm = Vm::<NoHost, NV>::new(&program, Some(3));
        assert!(
            vm.member_store(&mut cx, NV::ZERO, &path(&["a", "b"]), NV::Float(1.0))
                .is_err()
        );
        assert!(vm.stopped);
        assert_eq!(vm.steps_left, 0);
        assert_eq!(env.sink.take(), [step_msg(50)]);
    }

    #[test]
    fn a_member_store_that_widens_a_full_struct_stops_and_writes_nothing() {
        let program = asm(vec![Instr::End]);
        let mut env = NoHostEnv::new();
        env.limits.struct_members = Some(2);
        let mut cx = env.cx();
        let mut vm = Vm::<NoHost, NV>::new(&program, Some(100));
        let full = NV::structure(StructValue::from([("a", 1.0), ("b", 2.0)]));
        assert!(
            vm.member_store(&mut cx, full.clone(), &path(&["c"]), NV::Float(3.0))
                .is_err()
        );
        assert!(vm.stopped);
        // Charged before the refusal: 4 + 2 members.
        assert_eq!(vm.steps_left, 100 - 6);
        assert_eq!(
            env.sink.take(),
            ["molangx: evaluation stopped: a struct would hold more than its budget of 2 members"]
        );

        // An existing member is never refused.
        let mut env = NoHostEnv::new();
        env.limits.struct_members = Some(2);
        let mut cx = env.cx();
        let mut vm = Vm::<NoHost, NV>::new(&program, Some(100));
        let whole = vm
            .member_store(&mut cx, full, &path(&["a"]), NV::Float(9.0))
            .unwrap();
        assert_eq!(whole.member_path(&path(&["a"])), Some(&NV::Float(9.0)));
        assert!(!vm.stopped);
    }

    #[test]
    fn a_member_store_that_nests_too_deep_stops_with_the_depth_message() {
        let program = asm(vec![Instr::End]);
        let mut env = NoHostEnv::new();
        env.limits.struct_depth = Some(2);
        let mut cx = env.cx();
        let mut vm = Vm::<NoHost, NV>::new(&program, Some(100));
        assert!(
            vm.member_store(&mut cx, NV::ZERO, &path(&["a", "b", "c"]), NV::Float(1.0))
                .is_err()
        );
        assert!(vm.stopped);
        let mut vm = Vm::<NoHost, NV>::new(&program, Some(100));
        let ok = vm
            .member_store(&mut cx, NV::ZERO, &path(&["a", "b"]), NV::Float(1.0))
            .unwrap();
        assert_eq!(ok.struct_depth(), 2);
        assert_eq!(
            env.sink.take(),
            ["molangx: evaluation stopped: a struct would nest deeper than its budget of 2 levels"]
        );
    }

    #[test]
    fn storing_through_the_language_costs_four_plus_members_per_level() {
        // const, store-member (1 + cost), const 0, end.
        assert_eq!(steps_needed("v.s.a = 1;", |_| {}), 4 + 4);
        assert_eq!(steps_needed("v.s.a.b = 1;", |_| {}), 4 + 8);
        assert_eq!(steps_needed("v.s.a.b.c = 1;", |_| {}), 4 + 12);
        // A struct already holding three members pays them too.
        let three = |env: &mut NoHostEnv| {
            env.variables
                .set(key("s"), NV::structure(StructValue::xyz(1.0, 2.0, 3.0)));
        };
        assert_eq!(steps_needed("v.s.x = 5;", three), 4 + 4 + 3);
        assert_eq!(steps_needed("v.s.w = 5;", three), 4 + 4 + 3);
        assert_eq!(steps_needed("v.s.x.q = 5;", three), 4 + (4 + 3) + 4);
        // The second store pays for the member the first made.
        assert_eq!(
            steps_needed("v.s.a = 1; v.s.c = 1;", |_| {}),
            1 + (1 + 4) + 1 + (1 + 5) + 1 + 1
        );
    }

    #[test]
    fn a_temp_member_store_costs_the_same() {
        assert_eq!(steps_needed("t.s.a = 1;", |_| {}), 4 + 4);
        assert_eq!(steps_needed("t.s.a.b = 1;", |_| {}), 4 + 8);
    }

    #[test]
    fn a_member_store_into_a_context_is_a_free_no_op() {
        let r = ran("c.s.a = 1; return 7;", |_| {});
        assert_eq!(r.value, NV::Float(7.0));
        assert!(r.msgs.is_empty());
        // const, store-member (no cost: nothing is stored), const 0, return, ...
        assert_eq!(steps_needed("c.s.a = 1;", |_| {}), 4);
    }

    #[test]
    fn storing_an_actor_array_costs_one_step_per_entry() {
        let with = |len: usize| {
            move |env: &mut NoHostEnv| {
                env.context
                    .set(ContextName::new("arr"), NV::actor_array(vec![(); len]));
            }
        };
        // load context, store (1 + len), const 0, end.
        for len in [0_u64, 1, 5, 40] {
            assert_eq!(
                steps_needed("v.x = c.arr;", with(len as usize)),
                4 + len,
                "{len} entries"
            );
            assert_eq!(
                steps_needed("t.x = c.arr;", with(len as usize)),
                4 + len,
                "{len} entries (temp)"
            );
        }
    }

    #[test]
    fn a_store_that_the_budget_cannot_pay_stops_before_writing() {
        let expr = compile_ok("v.x = c.arr; v.y = 1;");
        let mut env = NoHostEnv::new();
        env.limits.total_steps = Some(6);
        env.context
            .set(ContextName::new("arr"), NV::actor_array(vec![(); 10]));
        assert_eq!(expr.eval(&mut env.cx()), NV::ZERO);
        assert_eq!(env.sink.take(), [step_msg(6)]);
        assert!(env.variables.get(key("x")).is_none());
        assert!(env.variables.get(key("y")).is_none());
    }

    #[test]
    fn the_step_limit_message_is_logged_once_per_evaluation() {
        // The budget ends inside the inner argument; nothing after it logs again.
        let expr = compile_ok("q.position(q.position(v.x)) + q.position(v.x)");
        let mut env = server_env();
        table(&mut env).set(query::POSITION, first_arg).unwrap();
        env.limits.total_steps = Some(3);
        set(&mut env, &[("x", 1.0)]);
        assert_eq!(expr.eval(&mut env.cx()), NV::ZERO);
        assert_eq!(env.sink.take(), [step_msg(3)]);
    }

    #[test]
    fn a_step_budget_applies_to_each_evaluation_separately() {
        let expr = compile_ok("v.x * v.y");
        let mut env = NoHostEnv::new();
        env.limits.total_steps = Some(5);
        set(&mut env, &[("x", 2.0), ("y", 3.0)]);
        for _ in 0..3 {
            assert_eq!(expr.eval(&mut env.cx()), NV::Float(6.0));
        }
        assert!(env.sink.is_empty());
    }

    /// A program that loads 5, runs `store` (whose post-op is 2·x + 3) and ends.
    fn store_program(store: Instr, names: &[&str], temps: &[u16]) -> Program {
        let mut p = with_names(
            asm(vec![Instr::Const { c: ConstIdx(0) }, store, Instr::End]),
            names,
            temps,
        );
        p.consts = vec![5.0].into_boxed_slice();
        p.posts = vec![PostOp::IDENTITY, PostOp::new(2.0, 3.0)].into_boxed_slice();
        p
    }

    // The compiler never puts a post-op on an assignment, so only an assembled program shows this.
    #[test]
    fn a_store_applies_its_post_op_to_the_accumulator_once_after_writing_the_raw_value() {
        let x = key("x");

        let p = store_program(
            Instr::StoreVar {
                n: NameIdx(0),
                p: PostIdx(1),
            },
            &["variable.x"],
            &[],
        );
        let mut env = NoHostEnv::new();
        assert_eq!(run_general(&p, &mut env, 100), (NV::Float(13.0), 97));
        assert_eq!(env.variables.get(x), Some(&NV::Float(5.0)));
        let mut env = NoHostEnv::new();
        let mut cx = env.cx();
        let mut vm = Vm::<NoHost, f32>::new(&p, Some(100));
        vm.run(&p, &mut cx, 0, Base::ROOT).unwrap();
        assert_eq!(vm.acc, 13.0);
        assert_eq!(env.variables.get(x), Some(&NV::Float(5.0)));

        let p = store_program(
            Instr::StoreTemp {
                t: TempIdx(0),
                p: PostIdx(1),
            },
            &["temp.t"],
            &[0],
        );
        let mut env = NoHostEnv::new();
        let mut cx = env.cx();
        let mut vm = Vm::<NoHost, NV>::new(&p, Some(100));
        vm.run(&p, &mut cx, 0, Base::ROOT).unwrap();
        assert_eq!(vm.acc, NV::Float(13.0));
        assert_eq!(vm.temps[0], Some(NV::Float(5.0)));

        let mut p = store_program(
            Instr::StoreMember {
                s: StoreIdx(0),
                p: PostIdx(1),
            },
            &["variable.x"],
            &[],
        );
        p.stores = vec![MemberStore {
            root: StoreRoot::Var(NameIdx(0)),
            path: vec![HashedStr::new("m")].into_boxed_slice(),
        }]
        .into_boxed_slice();
        let mut env = NoHostEnv::new();
        assert_eq!(run_general(&p, &mut env, 100).0, NV::Float(13.0));
        let stored = env
            .variables
            .get(x)
            .and_then(|v| v.member(HashedStr::new("m")));
        assert_eq!(stored, Some(&NV::Float(5.0)));
    }

    #[test]
    fn pop_on_an_empty_stack_reads_zero_and_add_acc_does_nothing() {
        let program = asm(vec![Instr::End]);
        let mut vm = Vm::<NoHost, NV>::new(&program, Some(100));
        assert_eq!(vm.pop(), NV::Float(0.0));
        vm.stack.push(NV::Float(4.0));
        assert_eq!(vm.pop(), NV::Float(4.0));
        assert!(vm.stack.is_empty());

        // `5 + pop` with nothing pushed is `5 + 0`; `add-acc` onto an empty stack leaves it empty.
        let mut p = asm(vec![
            Instr::Const { c: ConstIdx(0) },
            Instr::AddLast { p: PLAIN },
            Instr::End,
        ]);
        p.consts = vec![5.0].into_boxed_slice();
        let mut env = NoHostEnv::new();
        assert_eq!(run_general(&p, &mut env, 100).0, NV::Float(5.0));
        let mut p = asm(vec![
            Instr::Const { c: ConstIdx(0) },
            Instr::AddAcc,
            Instr::End,
        ]);
        p.consts = vec![5.0].into_boxed_slice();
        let mut cx = env.cx();
        let mut vm = Vm::<NoHost, NV>::new(&p, Some(100));
        vm.run(&p, &mut cx, 0, Base::ROOT).unwrap();
        assert!(vm.stack.is_empty());
        assert_eq!(vm.acc, NV::Float(5.0));
    }
}
