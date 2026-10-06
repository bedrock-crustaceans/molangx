//! The instruction loop and the temp accessors.
//!
//! The temp accessors stay in this file: moved out, temp-heavy evaluation gets slower.

use std::ops::ControlFlow;

#[cfg(feature = "stdlib")]
use super::Aborted;
use super::loops::LoopVar;
use super::{Base, Deopt, Slot, Stop, Vm};
use crate::catalog::{MAX_MATH_ARGS, MathImpl, MathRef};
use crate::compile::program::{Divisor, Instr, PostIdx, Program, TempIdx};
use crate::hash::HashedStr;
use crate::numeric;
use crate::numeric::arith;
use crate::vm::{
    cx::{EvalCx, Temps},
    host::Host,
    sink::RuntimeMsg,
    value::{ResourceRef, Value},
};
#[cfg(feature = "stdlib")]
use crate::{rng, stdlib::math};

impl<H: Host, R: Slot<H>> Vm<H, R> {
    /// Runs from `pc` until the (sub-)program ends.
    #[allow(clippy::too_many_lines)] // one arm per instruction
    pub(super) fn run(
        &mut self,
        program: &Program,
        cx: &mut EvalCx<'_, '_, H>,
        mut pc: usize,
        base: Base,
    ) -> Result<(), Stop<H>> {
        let step_budget = cx.limits.total_steps.unwrap_or(u64::MAX);
        let loop_limit = cx.limits.loop_iterations;
        if self.stopped {
            return Err(Stop::Abort);
        }
        loop {
            self.step(cx, step_budget)?;
            let at = pc;
            let Some(&instr) = program.code.get(pc) else {
                return Ok(());
            };
            pc += 1;
            match instr {
                Instr::Const { c } => self.acc = R::float(program.konst(c)),
                Instr::Hash { h } => {
                    self.acc = R::from_ref(&Value::Hash(HashedStr::from_u64(program.hash(h))))
                        .ok_or(Deopt::At(at))?;
                }
                Instr::Resource { h } => {
                    let resource = Value::Resource(ResourceRef::from_raw_hash(
                        HashedStr::from_u64(program.hash(h)),
                    ));
                    self.acc = R::from_ref(&resource).ok_or(Deopt::At(at))?;
                }
                Instr::This { p } => self.acc = R::float(program.post(p).apply(cx.subjects.this)),
                Instr::LoadVar { n, p } => {
                    let key = program.variable_name(n);
                    if self.arrows.is_empty() {
                        match cx.variable(key) {
                            Some(value) => {
                                self.acc =
                                    Self::with_post(program, value, p).ok_or(Deopt::At(at))?;
                            }
                            None => pc = self.missing(cx, base, program.name_text(n))?,
                        }
                    } else {
                        // Public-access mode: the snapshot, or 0 without aborting.
                        let value = cx.public_variable(key).unwrap_or(&Value::ZERO);
                        self.acc = Self::with_post(program, value, p).ok_or(Deopt::At(at))?;
                    }
                }
                Instr::LoadTemp { t, p } => match self.load_temp(program, cx, t, p, at)? {
                    Some(value) => self.acc = value,
                    None => pc = self.missing(cx, base, program.temp_text(t))?,
                },
                Instr::LoadCtx { n, p } => match cx.context(program.context_name(n)) {
                    Some(value) => {
                        self.acc = Self::with_post(program, &value, p).ok_or(Deopt::At(at))?;
                    }
                    None => pc = self.missing(cx, base, program.name_text(n))?,
                },
                Instr::Member { m, p } => {
                    let entry = program.member(m);
                    let base_value = self.acc.to_value();
                    match base_value.member(entry.hash) {
                        Some(value) => {
                            self.acc = Self::with_post(program, value, p).ok_or(Deopt::At(at))?;
                        }
                        None => {
                            // Named by the read path's last member.
                            cx.sink.runtime(RuntimeMsg::MissingMember {
                                name: &entry.report,
                            });
                            pc = self.missing(cx, base, &entry.report)?;
                        }
                    }
                }
                Instr::StoreVar { n, p } => self.store_var(program, cx, n, p)?,
                Instr::StoreTemp { t, p } => self.store_temp_instr(program, cx, t, p, at)?,
                Instr::StoreMember { s, p } => self.store_member(program, cx, s, p, at)?,
                Instr::Push => {
                    let value = self.acc.clone();
                    self.stack.push(value);
                }
                Instr::PushConst { c } => {
                    let value = self.acc.clone();
                    self.stack.push(value);
                    // The constant's own step.
                    self.step(cx, step_budget)?;
                    self.acc = R::float(program.konst(c));
                }
                Instr::Negate { p } => {
                    self.acc = R::float(numeric::negate(self.acc.f32(), program.post(p)));
                }
                Instr::Not { p } => {
                    self.acc = R::float(numeric::not(self.acc.f32(), program.post(p)));
                }
                Instr::AddAcc => {
                    let acc = self.acc.f32();
                    if let Some(top) = self.stack.last_mut() {
                        *top = R::float(arith::add(top.f32(), acc));
                    }
                }
                Instr::AddLast { p } => {
                    let top = self.pop().f32();
                    self.acc = R::float(numeric::add(self.acc.f32(), top, program.post(p)));
                }
                Instr::Mul { p } => {
                    let top = self.pop().f32();
                    self.acc = R::float(numeric::mul(self.acc.f32(), top, program.post(p)));
                }
                Instr::DivGuard { end, divisor } => {
                    match numeric::div_guard(divisor == Divisor::Signed, self.acc.f32()) {
                        Some(divisor) => self.stack.push(R::float(divisor)),
                        None => {
                            self.acc = R::float(0.0);
                            pc = end as usize;
                        }
                    }
                }
                Instr::Div { p } => {
                    let divisor = self.pop().f32();
                    self.acc = R::float(numeric::div(self.acc.f32(), divisor, program.post(p)));
                }
                #[cfg(feature = "stdlib")]
                Instr::Mod { p } => {
                    let a = self.pop().f32();
                    self.acc = R::float(math::mod_runtime(a, self.acc.f32(), program.post(p)));
                }
                #[cfg(feature = "stdlib")]
                Instr::ModConst { c, p } => {
                    self.acc = R::float(math::mod_const(
                        self.acc.f32(),
                        program.konst(c),
                        program.post(p),
                    ));
                }
                #[cfg(feature = "stdlib")]
                Instr::Math1 { f, p } => {
                    self.acc = R::float(f.apply(self.acc.f32(), program.post(p)));
                }
                #[cfg(feature = "stdlib")]
                Instr::Math2 { f, p } => {
                    let a = self.pop().f32();
                    self.acc = R::float(f.apply(a, self.acc.f32(), program.post(p)));
                }
                #[cfg(feature = "stdlib")]
                Instr::Math2Const { f, c, p } => {
                    self.acc = R::float(f.apply(self.acc.f32(), program.konst(c), program.post(p)));
                }
                #[cfg(feature = "stdlib")]
                Instr::Math3 { f, p } => {
                    let b = self.pop().f32();
                    let a = self.pop().f32();
                    self.acc = R::float(f.apply(a, b, self.acc.f32(), program.post(p)));
                }
                #[cfg(feature = "stdlib")]
                Instr::Random { p } => {
                    // The sample is drawn before the bounds are read.
                    let sample = rng::sample(cx.rng);
                    let a = self.pop().f32();
                    self.acc = R::float(math::random(a, self.acc.f32(), sample, program.post(p)));
                }
                #[cfg(feature = "stdlib")]
                Instr::RandomConst { p } => {
                    let sample = rng::sample(cx.rng);
                    self.acc = R::float(math::random_folded(sample, program.post(p)));
                }
                #[cfg(feature = "stdlib")]
                Instr::RandomInt { p } => {
                    let sample = rng::sample(cx.rng);
                    let a = self.pop().f32();
                    self.acc = R::float(math::random_integer(
                        a,
                        self.acc.f32(),
                        sample,
                        program.post(p),
                    ));
                }
                #[cfg(feature = "stdlib")]
                Instr::RandomIntConst { c, p } => {
                    let sample = rng::sample(cx.rng);
                    let [lo, hi] = program.konst_pair(c);
                    let value = math::random_integer_const_bounds(lo, hi, sample, program.post(p));
                    self.acc = R::float(value);
                }
                #[cfg(feature = "stdlib")]
                Instr::DieRoll { p } => {
                    let sum = self.die_roll(cx, step_budget, math::DieRoll::new)?;
                    self.acc = R::float(sum.finish(program.post(p)));
                }
                #[cfg(feature = "stdlib")]
                Instr::DieRollInt { p } => {
                    let sum = self.die_roll(cx, step_budget, math::DieRoll::new_integer)?;
                    self.acc = R::float(sum.finish(program.post(p)));
                }
                Instr::HostMath { f, argc, p } => {
                    self.acc = R::float(self.host_math(program, cx, f, argc, p));
                }
                Instr::Cmp { op, p } => {
                    let a = self.pop().f32();
                    self.acc = R::float(program.post(p).select(op.holds(a, self.acc.f32())));
                }
                Instr::CmpConst { op, c, p } => {
                    let holds = op.holds(self.acc.f32(), program.konst(c));
                    self.acc = R::float(program.post(p).select(holds));
                }
                Instr::Eq { op, p } => {
                    let a = self.pop();
                    let equal = R::equals(&a, &self.acc, cx);
                    self.acc = R::float(program.post(p).select(op.holds(equal)));
                }
                Instr::EqConst { op, c, p } => {
                    let equal = R::equals(&self.acc, &R::float(program.konst(c)), cx);
                    self.acc = R::float(program.post(p).select(op.holds(equal)));
                }
                Instr::EqHash { op, h, p } => {
                    let equal = self.acc.is_hash(program.hash(h));
                    self.acc = R::float(program.post(p).select(op.holds(equal)));
                }
                Instr::AndStep { to, p } => {
                    if !numeric::truthy(self.acc.f32()) {
                        self.acc = R::float(program.post(p).falsy_value());
                        pc = to as usize;
                    }
                }
                Instr::OrStep { to, p } => {
                    if numeric::truthy(self.acc.f32()) {
                        self.acc = R::float(program.post(p).truthy_value());
                        pc = to as usize;
                    }
                }
                Instr::AndLast { p } | Instr::OrLast { p } => {
                    self.acc = R::float(program.post(p).select(numeric::truthy(self.acc.f32())));
                }
                Instr::Jump { to } => pc = to as usize,
                Instr::JumpIfFalsy { to } => {
                    if !numeric::truthy(self.acc.f32()) {
                        pc = to as usize;
                    }
                }
                Instr::Post { p } => self.post_in_place(program, p),
                Instr::HandlerPush { to } => self.handler_push(to),
                Instr::HandlerPop { to } => {
                    // The left side of the `??` succeeded: the right side is skipped.
                    self.handlers.pop();
                    pc = to as usize;
                }
                Instr::PointerEnter { to, p } => {
                    if self.pointer_enter(program, cx, p).is_break() {
                        pc = to as usize;
                    }
                }
                Instr::PointerLeave { p } => self.pointer_leave(program, cx, p),
                Instr::Call { q, p } => self.call_query(program, cx, q, p, at, pc)?,
                Instr::LoopBegin { exit } => {
                    if self.loop_begin().is_break() {
                        pc = exit as usize;
                    }
                }
                Instr::LoopCheck { body } => {
                    if self.loop_check(cx, loop_limit)?.is_continue() {
                        pc = body as usize;
                    }
                }
                Instr::LoopEnd => self.loop_end(),
                Instr::EachBegin { exit } => {
                    if let ControlFlow::Break(to) = self.each_begin(program, exit) {
                        pc = to;
                    }
                }
                Instr::EachNextVar { n, exit } => {
                    let var = LoopVar::Var(n);
                    if self.each_next(program, cx, var, loop_limit, at)?.is_break() {
                        pc = exit as usize;
                    }
                }
                Instr::EachNextTemp { t, exit } => {
                    let var = LoopVar::Temp(t);
                    if self.each_next(program, cx, var, loop_limit, at)?.is_break() {
                        pc = exit as usize;
                    }
                }
                Instr::Return { p } => {
                    self.post_in_place(program, p);
                    return Ok(());
                }
                Instr::Halt => {
                    self.acc = R::float(0.0);
                    return Ok(());
                }
                Instr::End => return Ok(()),
            }
        }
    }

    /// The dice of a die roll with the count and bounds that end in `acc`, rolled one step each.
    #[cfg(feature = "stdlib")]
    #[inline]
    fn die_roll(
        &mut self,
        cx: &mut EvalCx<'_, '_, H>,
        step_budget: u64,
        dice: fn(f32, f32, f32) -> math::DieRoll,
    ) -> Result<math::DieRoll, Aborted> {
        let b = self.acc.f32();
        let a = self.pop().f32();
        let n = self.pop().f32();
        let mut roll = dice(n, a, b);
        while roll.remaining() > 0 {
            self.step(cx, step_budget)?;
            roll.roll(rng::sample(cx.rng));
        }
        Ok(roll)
    }

    /// A host math call on the `argc` arguments that end in `acc`, its post-op applied.
    #[inline(never)]
    fn host_math(
        &mut self,
        program: &Program,
        cx: &mut EvalCx<'_, '_, H>,
        f: MathRef,
        argc: u8,
        p: PostIdx,
    ) -> f32 {
        let mut values = [0.0; MAX_MATH_ARGS as usize];
        let count = usize::from(argc).min(values.len());
        if let Some((last, rest)) = values[..count].split_last_mut() {
            *last = self.acc.f32();
            for arg in rest.iter_mut().rev() {
                *arg = self.pop().f32();
            }
        }
        let Some(math) = &program.math else {
            return 0.0;
        };
        let value = match math.decl(f).implementation() {
            MathImpl::Pure(call) => call(&values[..count]),
            MathImpl::Volatile(call) => call(&mut *cx.rng, &values[..count]),
        };
        program.post(p).apply(value)
    }

    #[inline]
    pub(super) fn post_in_place(&mut self, program: &Program, p: PostIdx) {
        if p != PostIdx::PLAIN {
            self.acc = R::float(program.post(p).apply(self.acc.f32()));
        }
    }

    pub(super) fn temp_value(
        &self,
        program: &Program,
        cx: &EvalCx<'_, '_, H>,
        t: TempIdx,
    ) -> Option<Value<H>> {
        match &cx.temps {
            Temps::Kept(map) => map.get(program.temp_name(t)).cloned(),
            Temps::PerEvaluation => self.temps.get(usize::from(t.0))?.as_ref().map(R::to_value),
        }
    }

    /// `None` when the temp is unset.
    #[inline]
    fn load_temp(
        &self,
        program: &Program,
        cx: &EvalCx<'_, '_, H>,
        t: TempIdx,
        p: PostIdx,
        at: usize,
    ) -> Result<Option<R>, Deopt<H>> {
        match &cx.temps {
            Temps::Kept(map) => match map.get(program.temp_name(t)) {
                Some(value) => Self::with_post(program, value, p)
                    .map(Some)
                    .ok_or(Deopt::At(at)),
                None => Ok(None),
            },
            Temps::PerEvaluation => {
                let slot = self.temps.get(usize::from(t.0)).ok_or(Deopt::At(at))?;
                Ok(slot.as_ref().map(|slot| match p {
                    PostIdx::PLAIN => slot.clone(),
                    _ => R::float(program.post(p).apply(slot.f32())),
                }))
            }
        }
    }

    /// Stores the accumulator in temp `t`; hands over when the slot cannot be written.
    #[inline]
    pub(super) fn store_temp(
        &mut self,
        program: &Program,
        cx: &mut EvalCx<'_, '_, H>,
        t: TempIdx,
        at: usize,
    ) -> Result<(), Deopt<H>> {
        if let Temps::Kept(_) = cx.temps {
            return self.set_temp(program, cx, t, self.acc.to_value(), at);
        }
        let value = R::storable(self.acc.clone(), cx);
        *self.temps.get_mut(usize::from(t.0)).ok_or(Deopt::At(at))? = Some(value);
        Ok(())
    }

    /// Hands over when the slot type cannot hold `value`.
    pub(super) fn set_temp(
        &mut self,
        program: &Program,
        cx: &mut EvalCx<'_, '_, H>,
        t: TempIdx,
        value: Value<H>,
        at: usize,
    ) -> Result<(), Deopt<H>> {
        let value = cx.storable(value);
        match &mut cx.temps {
            Temps::Kept(map) => {
                map.set(program.temp_name(t), value);
            }
            Temps::PerEvaluation => {
                let slot = self.temps.get_mut(usize::from(t.0)).ok_or(Deopt::At(at))?;
                *slot = Some(R::from_ref(&value).ok_or(Deopt::At(at))?);
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compile::program::ConstIdx;
    use crate::compile::{CompileOptions, compile};
    use crate::numeric::PostOp;
    use crate::rng::{Xorshift128, sample};
    use crate::version::{MolangVersion, RawVersion};
    use crate::vm::eval::test_support::*;
    use crate::vm::{NoHost, NoHostEnv, TempName};

    #[test]
    fn a_program_runs_with_the_arithmetic_of_the_numeric_functions() {
        let c = compile(
            "math.sin(v.x) + math.max(v.x, v.y) * 0.3",
            &CompileOptions::server(MolangVersion::LATEST),
        );
        let expr = c.expr().cloned().unwrap();
        let mut env = NoHostEnv::new();
        set(&mut env, &[("x", 30.0), ("y", 7.0)]);
        let value = expr.eval_f32(&mut env.cx());
        let sin = math::sin(30.0, PostOp::IDENTITY);
        let max = math::max(30.0, 7.0, PostOp::new(0.3, 0.0));
        let sum = numeric::add(max, sin, PostOp::IDENTITY);
        assert_eq!(value.to_bits(), sum.to_bits());
    }

    #[test]
    fn halt_end_and_running_off_the_code() {
        let mut p = asm(vec![
            Instr::Const { c: ConstIdx(0) },
            Instr::Halt,
            Instr::End,
        ]);
        p.consts = vec![5.0].into_boxed_slice();
        let mut env = NoHostEnv::new();
        let mut cx = env.cx();
        let mut vm = Vm::<NoHost, NV>::new(&p, Some(100));
        vm.run(&p, &mut cx, 0, Base::ROOT).unwrap();
        assert_eq!(vm.acc, NV::Float(0.0), "halt zeroes the accumulator");

        // `end` keeps it.
        let mut p = asm(vec![Instr::Const { c: ConstIdx(0) }, Instr::End]);
        p.consts = vec![5.0].into_boxed_slice();
        let mut vm = Vm::<NoHost, NV>::new(&p, Some(100));
        vm.run(&p, &mut cx, 0, Base::ROOT).unwrap();
        assert_eq!(vm.acc, NV::Float(5.0));
        assert_eq!(vm.steps_left, 98);

        // Running off the code finishes; looking costs the one step.
        let mut p = asm(vec![Instr::Const { c: ConstIdx(0) }]);
        p.consts = vec![5.0].into_boxed_slice();
        let mut vm = Vm::<NoHost, NV>::new(&p, Some(100));
        vm.run(&p, &mut cx, 0, Base::ROOT).unwrap();
        assert_eq!(vm.acc, NV::Float(5.0));
        assert_eq!(vm.steps_left, 98);
    }

    #[test]
    fn return_applies_its_post_op_and_ends_the_program() {
        let mut p = asm(vec![
            Instr::Const { c: ConstIdx(0) },
            Instr::Return { p: PostIdx(1) },
            Instr::Const { c: ConstIdx(1) },
            Instr::End,
        ]);
        p.consts = vec![5.0, 9.0].into_boxed_slice();
        p.posts = vec![PostOp::IDENTITY, PostOp::new(2.0, 3.0)].into_boxed_slice();
        let mut env = NoHostEnv::new();
        let (value, left) = run_general(&p, &mut env, 100);
        assert_eq!(value, NV::Float(13.0));
        assert_eq!(left, 98);
    }

    #[test]
    fn and_or_steps_load_the_post_of_the_short_circuit_value() {
        let setup = |env: &mut NoHostEnv| set(env, &[("x", 2.0), ("y", 3.0), ("zero", 0.0)]);
        for (src, expected) in [
            ("(v.x && v.y) * 3 + 1", 4.0),
            ("(v.zero && v.y) * 3 + 1", 1.0),
            ("(v.x || v.y) * 3 + 1", 4.0),
            ("(v.zero || v.zero) * 3 + 1", 1.0),
            ("(v.zero || v.y) * 3 + 1", 4.0),
            ("(v.x && v.zero) * 3 + 1", 1.0),
        ] {
            assert_eq!(ran(src, setup).value, NV::Float(expected), "{src}");
        }
    }

    #[test]
    fn string_equality_compares_hashes_and_floats_by_their_bits() {
        let r = ran("v.s == 'moo'", |env| {
            env.variables.set(key("s"), NV::string("moo"));
        });
        assert_eq!(r.value, NV::Float(1.0));
        let r = ran("v.s != 'moo'", |env| {
            env.variables.set(key("s"), NV::string("cow"));
        });
        assert_eq!(r.value, NV::Float(1.0));
        let r = ran("v.s == 'moo'", |env| {
            env.variables.set(key("s"), NV::Float(1.0));
        });
        assert_eq!(r.value, NV::Float(0.0));
        let r = ran("v.s == v.t", |env| {
            env.variables.set(key("s"), NV::string("moo"));
            env.variables.set(key("t"), NV::string("moo"));
        });
        assert_eq!(r.value, NV::Float(1.0));
    }

    #[test]
    fn arithmetic_and_logic_give_the_values_the_instructions_define() {
        let vars = [
            ("x", 2.0),
            ("y", 3.0),
            ("z", 4.0),
            ("a", 7.0),
            ("b", 3.0),
            ("zero", 0.0),
            ("neg", -2.0),
        ];
        let table: &[(&str, f32)] = &[
            ("v.x - v.y", -1.0),
            ("v.y - v.x", 1.0),
            ("-v.x", -2.0),
            ("-v.neg", 2.0),
            ("v.x * v.y", 6.0),
            ("v.x + v.y + v.z", 9.0),
            ("v.x + v.y + v.z + 1", 10.0),
            ("v.a / v.b * 3", 7.0),
            ("v.z / v.x", 2.0),
            ("v.x / v.neg", -1.0),
            ("v.a / v.zero", 0.0),
            ("math.mod(v.a, v.b)", 1.0),
            ("math.mod(v.a, 4)", 3.0),
            ("math.mod(v.neg, 3)", -2.0),
            ("math.pow(v.x, v.y)", 8.0),
            ("math.pow(v.y, v.x)", 9.0),
            ("math.min(v.x, v.y)", 2.0),
            ("math.max(v.x, v.y)", 3.0),
            ("math.max(v.x, 5)", 5.0),
            ("math.clamp(v.a, v.x, v.y)", 3.0),
            ("math.clamp(v.neg, v.x, v.y)", 2.0),
            ("math.clamp(v.a, 0, 10)", 7.0),
            ("math.lerp(v.x, v.z, 0.5)", 3.0),
            ("math.abs(v.neg)", 2.0),
            ("!v.zero", 1.0),
            ("!v.x", 0.0),
            ("!!v.x", 1.0),
            ("v.x < v.y", 1.0),
            ("v.y < v.x", 0.0),
            ("v.x <= v.x", 1.0),
            ("v.x >= v.y", 0.0),
            ("v.y > v.x", 1.0),
            ("v.x < 5", 1.0),
            ("v.x > 5", 0.0),
            ("v.x == 2", 1.0),
            ("v.x != 2", 0.0),
            ("v.x == v.y", 0.0),
            ("v.x != v.y", 1.0),
            ("v.x && v.y", 1.0),
            ("v.x && v.zero", 0.0),
            ("v.zero || v.zero", 0.0),
            ("v.zero || v.y", 1.0),
            ("v.x ? 5 : 6", 5.0),
            ("v.zero ? 5 : 6", 6.0),
            ("v.x ? 5", 5.0),
            ("v.zero ? 5", 0.0),
            ("(v.x ? 1 : 2) * 3", 3.0),
            ("(v.zero ? 1 : 2) * 3", 6.0),
            ("(v.zero ? 1) * 3 + 1", 1.0),
            ("v.zero ? 1 : v.x ? 2 : 3", 2.0),
            ("v.x ? v.y ? 7 : 8 : 9", 7.0),
            ("(v.nope ?? 4) + 1", 5.0),
        ];
        for &(src, expected) in table {
            let r = ran(src, |env| set(env, &vars));
            assert_eq!(r.value, NV::Float(expected), "{src}");
            assert!(r.msgs.is_empty(), "{src}: {:?}", r.msgs);
        }
    }

    #[test]
    fn each_random_instruction_draws_the_samples_it_is_defined_to() {
        let setup = |env: &mut NoHostEnv| set(env, &[("n", 3.0), ("lo", 1.0), ("hi", 4.0)]);
        for (src, draws) in [
            ("math.random(1, 3)", 1),
            ("math.random(v.lo, v.hi)", 1),
            ("math.random_integer(1, 6)", 1),
            ("math.random_integer(v.lo, v.hi)", 1),
            ("math.die_roll(v.n, 1, 6)", 3),
            ("math.die_roll_integer(v.n, v.lo, v.hi)", 3),
            ("math.random(1, 3) + math.random(1, 3)", 2),
            ("1 + 2", 0),
        ] {
            let mut env = NoHostEnv::new();
            setup(&mut env);
            let mut expected = env.rng.clone();
            for _ in 0..draws {
                sample(&mut expected);
            }
            compile_ok(src).eval(&mut env.cx());
            assert_eq!(env.rng, expected, "{src}");
        }
    }

    #[test]
    fn random_values_come_from_the_samples() {
        let mut first = Xorshift128::new();
        let sample = sample(&mut first);
        let r = ran("math.random(10, 20)", |_| {});
        assert_eq!(
            r.value,
            NV::Float(math::random(10.0, 20.0, sample, PostOp::IDENTITY))
        );
        let r = ran("math.random(v.lo, v.hi)", |env| {
            set(env, &[("lo", 10.0), ("hi", 20.0)]);
        });
        assert_eq!(
            r.value,
            NV::Float(math::random(10.0, 20.0, sample, PostOp::IDENTITY))
        );
        let r = ran("math.random_integer(v.lo, v.hi)", |env| {
            set(env, &[("lo", 10.0), ("hi", 20.0)]);
        });
        assert_eq!(
            r.value,
            NV::Float(math::random_integer(10.0, 20.0, sample, PostOp::IDENTITY))
        );
    }

    #[test]
    fn temps_are_cleared_at_the_start_of_every_evaluation_by_default() {
        let write = compile_ok("t.a = 5;");
        let read = compile_ok("return t.a;");
        let mut env = NoHostEnv::new();
        write.eval(&mut env.cx());
        assert_eq!(read.eval(&mut env.cx()), NV::Float(0.0));
        assert_eq!(env.sink.take(), [unknown_msg("temp.a")]);
        assert!(matches!(env.temps, Temps::PerEvaluation));
    }

    #[test]
    fn temps_carry_over_across_evaluations_when_kept() {
        let write = compile_ok("t.a = 5;");
        let read = compile_ok("return t.a * 2;");
        let mut env = kept_temps_env();
        write.eval(&mut env.cx());
        assert_eq!(read.eval(&mut env.cx()), NV::Float(10.0));
        assert!(env.sink.is_empty());
        assert_eq!(
            env.temps.kept().unwrap().get(TempName::new("a")),
            Some(&NV::Float(5.0))
        );
        assert_eq!(eval_general(&read, &mut env), NV::Float(10.0));
    }

    #[test]
    fn a_temp_holds_any_value_within_one_evaluation() {
        let r = ran("t.s = v.s; v.r = t.s; return 1;", strings);
        assert_eq!(r.value, NV::Float(1.0));
        assert_eq!(r.var("r"), Some(&NV::string("moo")));
        let expr = compile_ok("t.s = v.s; v.r = t.s; return 1;");
        let mut env = kept_temps_env();
        strings(&mut env);
        expr.eval(&mut env.cx());
        assert_eq!(
            env.temps.kept().unwrap().get(TempName::new("s")),
            Some(&NV::string("moo"))
        );
    }

    #[test]
    fn a_temp_miss_reads_the_slot_or_the_environment_alike() {
        let missing = compile_ok("return t.nope;");
        let mut default_env = NoHostEnv::new();
        assert_eq!(missing.eval(&mut default_env.cx()), NV::Float(0.0));
        let mut persistent_env = kept_temps_env();
        assert_eq!(missing.eval(&mut persistent_env.cx()), NV::Float(0.0));
        assert_eq!(default_env.sink.take(), persistent_env.sink.take());
    }

    #[test]
    fn a_guarded_division_whose_guard_fires_skips_the_numerator() {
        let with = |b: f32| move |env: &mut NoHostEnv| set(env, &[("a", 6.0), ("b", b)]);
        assert_eq!(steps_needed("v.a / v.b", with(2.0)), 5);
        assert_eq!(steps_needed("v.a / v.b", with(0.0)), 3);
        assert_eq!(steps_needed("v.a / v.b", with(1e-9)), 3);
        assert_eq!(steps_needed("v.a / v.b", with(-1e-9)), 3);
        let long = "math.max(v.a, v.a * v.b) / v.b";
        let len = compile_ok(long).program().unwrap().code.len() as u64;
        assert_eq!(steps_needed(long, with(0.0)), 3);
        assert_eq!(steps_needed(long, with(2.0)), len);
    }

    #[test]
    fn a_division_by_zero_is_zero_and_the_numerator_does_not_run() {
        let r = ran("v.a / v.b", |env| set(env, &[("a", 6.0), ("b", 0.0)]));
        assert_eq!(r.value, NV::Float(0.0));
        assert!(r.msgs.is_empty());
        // A missing numerator is never read, so it is never reported.
        let r = ran("v.nope / v.b", |env| set(env, &[("b", 0.0)]));
        assert_eq!(r.value, NV::Float(0.0));
        assert!(r.msgs.is_empty());
        let r = ran("v.nope / v.b", |env| set(env, &[("b", 2.0)]));
        assert_eq!(r.value, NV::Float(0.0));
        assert_eq!(r.msgs, [unknown_msg("variable.nope")]);
    }

    #[test]
    fn the_division_guard_pushes_the_signed_or_the_absolute_divisor_by_version() {
        let setup = |env: &mut NoHostEnv| set(env, &[("a", 6.0), ("b", -2.0)]);
        for (raw, expected) in [(6, 3.0), (7, -3.0)] {
            let expr = compile_at("v.a / v.b", RawVersion(raw).effective());
            let mut env = NoHostEnv::new();
            setup(&mut env);
            assert_eq!(
                expr.eval(&mut env.cx()),
                NV::Float(expected),
                "version {raw}"
            );
            assert_eq!(
                steps_needed_of(&expr, setup, |e, env| e.eval(&mut env.cx())),
                5,
                "version {raw}"
            );
        }
    }

    #[test]
    fn each_die_roll_costs_one_more_step() {
        let with = |n: f32| move |env: &mut NoHostEnv| set(env, &[("n", n)]);
        let base_cost = steps_needed("math.die_roll(v.n, 1, 6)", with(0.0));
        assert_eq!(base_cost, 7, "load, 2 pushed constants, die-roll, end");
        for rolls in [1_u64, 3, 5, 20] {
            assert_eq!(
                steps_needed("math.die_roll(v.n, 1, 6)", with(rolls as f32)),
                base_cost + rolls,
                "{rolls} rolls"
            );
            assert_eq!(
                steps_needed("math.die_roll_integer(v.n, 1, 6)", with(rolls as f32)),
                base_cost + rolls,
                "{rolls} rolls, integer"
            );
        }
    }

    #[test]
    fn a_budget_that_ends_between_two_rolls_stops_the_evaluation() {
        let expr = compile_ok("math.die_roll(v.n, 1, 6)");
        for budget in [7, 8, 9] {
            let mut env = NoHostEnv::new();
            env.limits.total_steps = Some(budget);
            set(&mut env, &[("n", 5.0)]);
            assert_eq!(expr.eval(&mut env.cx()), NV::ZERO, "budget {budget}");
            assert_eq!(env.sink.take(), [step_msg(budget)], "budget {budget}");
        }
        let mut env = NoHostEnv::new();
        env.limits.total_steps = Some(12);
        set(&mut env, &[("n", 5.0)]);
        assert!(expr.eval(&mut env.cx()).as_f32() >= 5.0);
        assert!(env.sink.is_empty());
    }

    #[test]
    fn each_die_roll_draws_one_random_number() {
        let expr = compile_ok("math.die_roll(v.n, 1, 6)");
        let mut env = NoHostEnv::new();
        set(&mut env, &[("n", 4.0)]);
        let before = env.rng.clone();
        expr.eval(&mut env.cx());
        let mut expected = before;
        for _ in 0..4 {
            sample(&mut expected);
        }
        assert_eq!(env.rng, expected);
    }
}
