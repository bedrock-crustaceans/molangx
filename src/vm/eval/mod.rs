//! The evaluator: [`Expr::eval`] and [`Expr::eval_f32`].
//!
//! The interpreter is generic over its slot type ([`Slot`]): `f32` or [`Value<H>`]. A program
//! that is float-only (but for argument-less queries) starts on the `f32` loop, which never
//! allocates. When that loop meets a value it cannot hold it stops *before* the instruction
//! changes anything, converts its state and the general loop continues from the same instruction
//! (deoptimisation); a query's non-float result is handed over with the call done. Both loops give
//! the same bits.
//!
//! A missing variable or struct member jumps to the innermost `??` handler; without one it logs
//! `Error: unhandled request for unknown variable '{}'` and the expression ends with 0.0. Inside
//! `->` an entity-variable read never aborts: an absent, private or unsnapshotted variable reads
//! 0.0.
//!
//! A query argument runs as a sub-program in the same evaluation
//! ([`QueryCx::arg`](crate::vm::QueryCx::arg)), the evaluator's only native recursion. A missing
//! variable inside it ends that argument with 0 and the call continues; a `??` outside the call
//! does not catch it.

// Instruction arms match on their two outcomes.
#![allow(clippy::single_match_else)]

use std::sync::Arc;

use smallvec::SmallVec;

use super::cx::EvalCx;
use super::host::{Host, Subjects};
use super::value::Value;
use crate::compile::{
    Evaluation, Expr,
    program::{Depths, FLOAT_LOOP_TEMPS, Instr, Program},
};
use crate::hash::HashedStr;

mod handover;
mod interp;
mod loops;
mod machine;
mod missing;
mod query;
mod slot;

use slot::Slot;

impl Expr {
    /// Evaluates the expression; `0.0` when it was rejected or a missing variable or a budget of
    /// [`EvalCx::limits`] ends it.
    ///
    /// `cx.subjects` is restored on return, also when a `->` was left early.
    ///
    /// # Stack
    ///
    /// A query evaluating an argument recurses natively. At the compiler's bound of 255 nested
    /// calls an evaluation needs about **320 KiB** of stack optimised and **1.9 MiB** unoptimised
    /// (about 1.2 KiB and 7.5 KiB per level). Evaluate on threads with at least **512 KiB**
    /// (optimised) or **4 MiB** (unoptimised), or cap the nesting with
    /// [`EvalLimits::query_depth`](crate::vm::EvalLimits::query_depth); a stack overflow aborts the
    /// process. Everything else runs in constant stack.
    pub fn eval<H: Host>(&self, cx: &mut EvalCx<'_, '_, H>) -> Value<H> {
        let program = match self.evaluation() {
            Evaluation::Constant(constant) => return Value::Float(constant),
            Evaluation::Program(program) => program,
        };
        if let Some(hash) = constant_hash(program) {
            return Value::Hash(HashedStr::from_u64(hash));
        }
        let subjects = cx.subjects;
        let result = if program.float_loop {
            let mut vm = Vm::<H, f32>::new(program, cx.limits.total_steps);
            match vm.run(program, cx, 0, Base::ROOT) {
                Ok(()) => Value::Float(vm.acc),
                Err(Stop::Abort) => Value::ZERO,
                Err(Stop::Deopt(deopt)) => vm.hand_over(program, cx, deopt),
            }
        } else {
            Vm::<H, Value<H>>::new(program, cx.limits.total_steps).finish(program, cx, 0)
        };
        cx.subjects = subjects;
        result
    }

    /// The [`Value::as_f32`] of [`Expr::eval`]; a
    /// [`ProgramFlags::FLOAT_ONLY`](crate::compile::ProgramFlags::FLOAT_ONLY) program does not
    /// allocate.
    ///
    /// A string result becomes the **low 32 bits of its hash as an `f32`** (often a NaN or a
    /// denormal, not 0); any other handle becomes 0.0. Needs the stack [`Expr::eval`] needs.
    pub fn eval_f32<H: Host>(&self, cx: &mut EvalCx<'_, '_, H>) -> f32 {
        self.eval(cx).as_f32()
    }
}

/// The hash of a program that is a lone string constant.
fn constant_hash(program: &Program) -> Option<u64> {
    match program.code.as_ref() {
        [Instr::Hash { h }, Instr::End] => Some(program.hash(*h)),
        _ => None,
    }
}

/// A running loop; `iterations` is counted only under a loop budget.
enum Frame<H: Host> {
    /// `loop`. The counter lives on the operand stack, where operands a `break` / `continue`
    /// leaves behind can displace it.
    Count { iterations: u32 },
    /// `for_each` over `array`, of which `index` entries are visited; `stack` is the operand-stack
    /// depth it started at.
    Each {
        array: Arc<Vec<H::ActorRef>>,
        index: usize,
        iterations: u32,
        stack: usize,
    },
}

/// The depths of the operand stack, the loop frames and the entered `->`s at one point of a run.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
struct Mark {
    stack: usize,
    loops: usize,
    arrows: usize,
}

/// A `??` handler: the right side to jump to and the depths to unwind to when it catches.
struct Handler {
    to: u32,
    mark: Mark,
}

/// Where a (sub-)program run started; the handlers below `handlers` are not its own.
#[derive(Copy, Clone)]
struct Base {
    mark: Mark,
    handlers: usize,
}

impl Base {
    const ROOT: Self = Self {
        mark: Mark {
            stack: 0,
            loops: 0,
            arrows: 0,
        },
        handlers: 0,
    };
}

/// Why a run ended before its program did; a run that reaches the end leaves its result in the
/// accumulator.
#[derive(Debug)]
enum Stop<H: Host> {
    /// The result is 0.0: a budget or a missing variable without a handler ended the run.
    Abort,
    Deopt(Deopt<H>),
}

impl<H: Host> From<Deopt<H>> for Stop<H> {
    fn from(deopt: Deopt<H>) -> Self {
        Self::Deopt(deopt)
    }
}

/// [`Stop::Abort`], as the error of a step that cannot hand over: one byte, so the hot loop
/// tests it like a flag.
#[derive(Debug)]
struct Aborted;

impl<H: Host> From<Aborted> for Stop<H> {
    #[inline]
    fn from(Aborted: Aborted) -> Self {
        Self::Abort
    }
}

/// Why the float loop hands over to the general loop.
#[derive(Debug)]
enum Deopt<H: Host> {
    /// Before this instruction changed anything: the general loop runs it from the start.
    At(usize),
    /// After an argument-less query returned a non-float: `value` becomes the accumulator and
    /// the general loop continues at `next`.
    After { next: usize, value: Value<H> },
}

/// The machine of one evaluation, with slots of type `R`.
struct Vm<H: Host, R> {
    acc: R,
    stack: SmallVec<[R; Depths::FLOAT_LOOP.stack as usize]>,
    loops: SmallVec<[Frame<H>; Depths::FLOAT_LOOP.loops as usize]>,
    handlers: SmallVec<[Handler; Depths::FLOAT_LOOP.handlers as usize]>,
    /// The caller's subjects of every `->` entered; their number is the public-access depth.
    arrows: SmallVec<[Subjects<H>; 2]>,
    /// `temp.*` under [`Temps::PerEvaluation`](crate::vm::Temps::PerEvaluation), one slot per
    /// name.
    temps: SmallVec<[Option<R>; FLOAT_LOOP_TEMPS]>,
    /// One step per instruction executed and per roll of a die roll.
    steps_left: u64,
    /// A budget ended the evaluation: every run returns at once.
    stopped: bool,
    /// The loop guard reports once per evaluation.
    loop_budget_logged: bool,
    /// Nesting of running query arguments
    /// ([`EvalLimits::query_depth`](super::EvalLimits::query_depth)).
    arg_depth: u32,
}

#[cfg(test)]
pub(crate) mod test_support {
    // Query functions have the fixed `Query` signature.
    #![allow(clippy::unnecessary_wraps)]

    use std::cell::Cell;

    use super::Vm;
    use crate::catalog::Side;
    use crate::compile::program::{
        Depths, Instr, NameEntry, NameIdx, PostIdx, Program, ProgramFlags,
    };
    use crate::compile::{CompileOptions, Expr, compile};
    use crate::hash::HashedStr;
    use crate::numeric::PostOp;
    use crate::version::MolangVersion;
    use crate::vm::name::{ContextName, VariableName};
    use crate::vm::test_support::TestHost;
    use crate::vm::{
        HostEnv, NoHost, NoHostEnv, QueryError, QueryTable, TempMap, Temps, VariableMap,
        VariableStore,
        host::{Host, Subjects},
        query::QueryCx,
        value::Value,
    };

    pub(crate) type NV = Value<NoHost>;

    pub(crate) type HV = Value<TestHost>;

    pub(crate) fn step_msg(steps: u64) -> String {
        format!("molangx: evaluation stopped after its budget of {steps} steps")
    }

    pub(crate) fn unknown_msg(name: &str) -> String {
        format!("Error: unhandled request for unknown variable '{name}'")
    }

    pub(crate) fn compile_ok(src: &str) -> Expr {
        compile_at(src, MolangVersion::LATEST)
    }

    pub(crate) fn compile_at(src: &str, version: MolangVersion) -> Expr {
        let c = compile(src, &CompileOptions::server(version));
        assert_eq!(
            c.failure(),
            None,
            "{src}: {:?}",
            c.diagnostics()
                .iter()
                .map(|d| d.message().into_owned())
                .collect::<Vec<_>>()
        );
        c.expr().cloned().unwrap()
    }

    pub(crate) fn key(name: &str) -> VariableName {
        VariableName::new(name)
    }

    pub(crate) struct Ran {
        pub(crate) value: NV,
        pub(crate) msgs: Vec<String>,
        pub(crate) env: NoHostEnv,
    }

    impl Ran {
        pub(crate) fn var(&self, name: &str) -> Option<&NV> {
            self.env.variables.get(key(name))
        }

        pub(crate) fn num(&self, name: &str) -> f32 {
            self.var(name).map_or(f32::NAN, Value::as_f32)
        }
    }

    /// Every query is a stub until a test installs a function.
    #[track_caller]
    pub(crate) fn server_env() -> NoHostEnv {
        NoHostEnv {
            queries: Some(QueryTable::new(crate::stdlib::queries(Side::Server))),
            ..NoHostEnv::new()
        }
    }

    /// The query table of a [`server_env`].
    pub(crate) fn table(env: &mut NoHostEnv) -> &mut QueryTable<NoHost> {
        env.queries.as_mut().expect("a server environment")
    }

    /// An environment that keeps `temp.*` across evaluations.
    pub(crate) fn kept_temps_env() -> NoHostEnv {
        NoHostEnv {
            temps: Temps::Kept(TempMap::new()),
            ..NoHostEnv::new()
        }
    }

    pub(crate) fn ran(src: &str, setup: impl FnOnce(&mut NoHostEnv)) -> Ran {
        let expr = compile_ok(src);
        let mut env = server_env();
        setup(&mut env);
        let value = expr.eval(&mut env.cx());
        let msgs = env.sink.take();
        Ran { value, msgs, env }
    }

    pub(crate) fn set(env: &mut NoHostEnv, vars: &[(&str, f32)]) {
        for &(name, value) in vars {
            env.variables.set(key(name), Value::Float(value));
        }
    }

    /// `v.x` 2, `v.y` 3, `v.z` 4, `v.a` 6, `v.b` 2, `v.n` 3, `v.m` 0, `v.zero` 0, `c.foo` 1 and
    /// `this` 1.5.
    pub(crate) fn sample_vars(env: &mut NoHostEnv) {
        set(
            env,
            &[
                ("x", 2.0),
                ("y", 3.0),
                ("z", 4.0),
                ("a", 6.0),
                ("b", 2.0),
                ("n", 3.0),
                ("m", 0.0),
                ("zero", 0.0),
            ],
        );
        env.context.set(ContextName::new("foo"), NV::Float(1.0));
        env.this = 1.5;
    }

    pub(crate) fn step_limit_hit(msgs: &[String]) -> bool {
        msgs.iter()
            .any(|m| m.starts_with("molangx: evaluation stopped after its budget"))
    }

    /// The smallest `total_steps` with which `src` completes.
    #[track_caller]
    pub(crate) fn steps_needed(src: &str, setup: impl Fn(&mut NoHostEnv)) -> u64 {
        let expr = compile_ok(src);
        steps_needed_of(&expr, setup, |expr, env| expr.eval(&mut env.cx()))
    }

    #[track_caller]
    pub(crate) fn steps_needed_of(
        expr: &Expr,
        setup: impl Fn(&mut NoHostEnv),
        run: impl Fn(&Expr, &mut NoHostEnv) -> NV,
    ) -> u64 {
        let completes = |steps: u64| {
            let mut env = server_env();
            env.limits.total_steps = Some(steps);
            setup(&mut env);
            let _ = run(expr, &mut env);
            !step_limit_hit(&env.sink.take())
        };
        assert!(
            !completes(0),
            "an expression that completes with no steps has no step count"
        );
        let mut hi = 1;
        while !completes(hi) {
            hi *= 2;
            assert!(hi <= 1 << 24, "does not complete");
        }
        let mut lo = hi / 2;
        while lo + 1 < hi {
            let mid = lo + (hi - lo) / 2;
            if completes(mid) {
                hi = mid;
            } else {
                lo = mid;
            }
        }
        hi
    }

    pub(crate) fn eval_general(expr: &Expr, env: &mut NoHostEnv) -> NV {
        let program = expr.program().unwrap();
        let mut cx = env.cx();
        Vm::<NoHost, NV>::new(program, cx.limits.total_steps).finish(program, &mut cx, 0)
    }

    /// A program of `code` over empty pools.
    pub(crate) fn asm(code: Vec<Instr>) -> Program {
        Program {
            code: code.into_boxed_slice(),
            consts: Box::new([]),
            posts: vec![PostOp::IDENTITY].into_boxed_slice(),
            hashes: Box::new([]),
            names: Box::new([]),
            temps: Box::new([]),
            members: Box::new([]),
            stores: Box::new([]),
            calls: Box::new([]),
            catalog: crate::stdlib::queries(Side::Client).clone(),
            math: None,
            depths: Depths::default(),
            flags: ProgramFlags::empty(),
            float_loop: false,
            version: MolangVersion::LATEST,
        }
    }

    pub(crate) fn name_entry(text: &str) -> NameEntry {
        NameEntry {
            hash: HashedStr::new(text),
            text: text.into(),
        }
    }

    /// `temps` are the indices of the temp names in `names`.
    pub(crate) fn with_names(mut program: Program, names: &[&str], temps: &[u16]) -> Program {
        program.names = names.iter().map(|n| name_entry(n)).collect();
        program.temps = temps.iter().copied().map(NameIdx).collect();
        program
    }

    pub(crate) const PLAIN: PostIdx = PostIdx::PLAIN;

    /// The value and the steps left.
    pub(crate) fn run_general(program: &Program, env: &mut NoHostEnv, steps: u64) -> (NV, u64) {
        let mut vm = Vm::<NoHost, NV>::new(program, Some(steps));
        let mut cx = env.cx();
        let value = vm.finish(program, &mut cx, 0);
        (value, vm.steps_left)
    }

    pub(crate) struct HostRan<V = VariableMap<TestHost>> {
        pub(crate) value: HV,
        pub(crate) msgs: Vec<String>,
        pub(crate) env: HostEnv<TestHost, V>,
    }

    /// Evaluates `src` in `env` for actor 1 of the test host.
    #[track_caller]
    pub(crate) fn host_ran<V: VariableStore<TestHost>>(
        src: &str,
        mut env: HostEnv<TestHost, V>,
    ) -> HostRan<V> {
        let expr = compile_ok(src);
        let mut world = TestHost;
        let value = expr.eval(&mut env.cx(&mut world, Subjects::actor(1)));
        let msgs = env.sink.take();
        HostRan { value, msgs, env }
    }

    thread_local! {
        static CALLS: Cell<u32> = const { Cell::new(0) };
    }

    /// The calls of [`first_arg`] and [`five`] and the ones [`count_call`] counts on this thread
    /// since the last [`reset_calls`].
    pub(crate) fn calls() -> u32 {
        CALLS.with(Cell::get)
    }

    pub(crate) fn reset_calls() {
        CALLS.with(|c| c.set(0));
    }

    pub(crate) fn count_call() {
        CALLS.with(|c| c.set(c.get() + 1));
    }

    pub(crate) fn first_arg<H: Host>(cx: &mut QueryCx<'_, '_, H>) -> Result<Value<H>, QueryError> {
        count_call();
        Ok(cx.arg(0).unwrap_or_default())
    }

    pub(crate) fn five<H: Host>(_cx: &mut QueryCx<'_, '_, H>) -> Result<Value<H>, QueryError> {
        count_call();
        Ok(Value::Float(5.0))
    }

    pub(crate) fn strings(env: &mut NoHostEnv) {
        env.variables.set(key("s"), NV::string("moo"));
        set(env, &[("t", 4.0)]);
    }

    pub(crate) const DEPTH_MSG: &str =
        "molangx: evaluation stopped: a struct would nest deeper than its budget of 2 levels";

    pub(crate) fn limited(depth: Option<u32>, members: Option<u32>) -> impl Fn(&mut NoHostEnv) {
        move |env| {
            env.limits.struct_depth = depth;
            env.limits.struct_members = members;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::test_support::*;
    use super::*;
    use crate::compile::program::{ConstIdx, HashIdx};
    use crate::compile::{CompileFailure, CompileOptions, compile};
    use crate::diag::LanguageMessage;
    use crate::rng::Xorshift128;
    use crate::stdlib::query;
    use crate::version::MolangVersion;
    use crate::vm::name::ContextName;
    use crate::vm::{NoHostEnv, StructValue, value::ResourceRef};

    #[test]
    fn eval_f32_reads_each_kind_of_value_as_arithmetic_does() {
        let kinds: Vec<(&str, NV, f32)> = vec![
            ("float", NV::Float(2.5), 2.5),
            (
                "string",
                NV::string("moo"),
                f32::from_bits(HashedStr::new("moo").as_u64() as u32),
            ),
            ("actor", NV::Actor(()), 0.0),
            ("item", NV::Item(()), 0.0),
            ("actor array", NV::actor_array([(), ()]), 0.0),
            ("struct", NV::structure(StructValue::xy(1.0, 2.0)), 0.0),
            ("matrix", NV::identity_matrix(), 0.0),
            (
                "resource",
                NV::Resource(ResourceRef::new("texture.default")),
                0.0,
            ),
        ];
        let plain = compile_ok("v.k");
        assert!(plain.program().unwrap().float_loop);
        let via_query = compile_ok("q.position(v.k)");
        assert!(!via_query.program().unwrap().float_loop);
        for (what, value, expected) in kinds {
            for (how, expr) in [("float loop", &plain), ("general loop", &via_query)] {
                let mut env = server_env();
                table(&mut env).set(query::POSITION, first_arg).unwrap();
                env.variables.set(key("k"), value.clone());
                assert_eq!(expr.eval(&mut env.cx()), value, "{what}, {how}");
                assert_eq!(
                    expr.eval_f32(&mut env.cx()).to_bits(),
                    expected.to_bits(),
                    "{what}, {how}"
                );
            }
        }
    }

    #[test]
    fn a_string_result_is_the_low_bits_of_its_hash_as_a_float() {
        let hash = HashedStr::new("walk").as_u64();
        let mut env = NoHostEnv::new();
        env.variables.set(key("k"), NV::string("walk"));
        let f = compile_ok("v.k").eval_f32(&mut env.cx());
        assert_eq!(f.to_bits(), hash as u32);
    }

    #[test]
    fn a_rejected_expression_evaluates_to_zero() {
        let c = compile(
            "v.p->v.x = 1; return 1;",
            &CompileOptions::server(MolangVersion::LATEST),
        );
        assert_eq!(c.failure(), Some(CompileFailure::Rejected));
        let expr = c.expr_or_zero().cloned().unwrap();
        assert!(expr.program().is_none());
        let mut env = NoHostEnv::new();
        assert_eq!(expr.eval(&mut env.cx()), NV::Float(0.0));
        assert_eq!(expr.eval_f32(&mut env.cx()), 0.0);
        assert!(env.sink.is_empty());
    }

    const FLOAT_SOURCES: &[&str] = &[
        "v.x * v.y + v.z",
        "v.a / v.b",
        "v.a / v.zero",
        "v.a / -v.b",
        "math.clamp(v.x, 0, 1)",
        "math.sin(v.x) + math.cos(v.y)",
        "v.x < v.y ? v.x : v.y",
        "v.x && v.y || v.zero",
        "v.nope ?? 4",
        "v.x ?? 4",
        "v.nope",
        "v.x + v.nope",
        "v.r = v.x * 2; v.s = v.r + 1; return v.s;",
        "t.a = v.x; t.b = t.a * 3; return t.b;",
        "t.a = v.nope; return 1;",
        "loop(5, { v.n = v.n + v.x; }); return v.n;",
        "loop(5000, { v.n = v.n + 1; }); return v.n;",
        "loop(10, { v.n >= 5 ? break; v.n = v.n + 1; }); return v.n;",
        "loop(4, { v.m = v.m + 1; v.m < 3 ? continue; v.n = v.n + 10; }); return v.n + v.m;",
        "loop(3, { loop(3, { v.n = v.n + 1; }); }); return v.n;",
        "math.random(1, 3) + math.random(v.x, v.y)",
        "math.random_integer(1, 10) * 2 + math.random_integer(v.x, v.a)",
        "math.die_roll(4, 1, 6)",
        "math.die_roll_integer(v.n, 1, 6)",
        "math.mod(v.a, v.b) + math.mod(v.a, 4)",
        "math.pow(v.x, v.y) - math.atan2(v.x, v.y)",
        "math.lerp(v.x, v.y, 0.5) + math.ease_in_quad(0, 1, v.zero)",
        "this * 2 + v.x",
        "c.foo + 1",
        "!v.x + !v.zero",
        "-v.x * v.y",
        "v.x == v.y",
        "v.x != 2",
        "math.min(v.x, v.y) - math.max(v.x, v.y)",
        "math.abs(-3) + math.floor(v.x * 1.5) + math.ceil(v.x * 0.3)",
        "v.x * 0.1 + 0.3",
    ];

    #[derive(Debug, PartialEq)]
    struct Outcome {
        value: u32,
        msgs: Vec<String>,
        vars: Vec<Option<u32>>,
        rng: Xorshift128,
    }

    fn outcome(expr: &Expr, general: bool, steps: u64) -> Outcome {
        let mut env = NoHostEnv::new();
        sample_vars(&mut env);
        env.limits.total_steps = Some(steps);
        let value = if general {
            eval_general(expr, &mut env)
        } else {
            expr.eval(&mut env.cx())
        };
        let vars = ["a", "b", "n", "m", "r", "s", "t", "x", "y", "z"]
            .iter()
            .map(|n| env.variables.get(key(n)).map(|v| v.as_f32().to_bits()))
            .collect();
        Outcome {
            value: value.as_f32().to_bits(),
            msgs: env.sink.take(),
            vars,
            rng: env.rng,
        }
    }

    #[test]
    fn the_float_loop_and_the_general_loop_give_identical_results() {
        for src in FLOAT_SOURCES {
            let expr = compile_ok(src);
            assert!(
                expr.program().unwrap().float_loop,
                "{src} starts on the float loop"
            );
            let full = outcome(&expr, true, u64::MAX);
            assert_eq!(outcome(&expr, false, u64::MAX), full, "{src}");
        }
    }

    #[test]
    fn eval_f32_on_the_float_loop_matches_the_general_loop() {
        for src in FLOAT_SOURCES {
            let expr = compile_ok(src);
            let mut float_env = NoHostEnv::new();
            sample_vars(&mut float_env);
            let float = expr.eval_f32(&mut float_env.cx());
            let general = outcome(&expr, true, u64::MAX);
            assert_eq!(float.to_bits(), general.value, "{src}");
            assert_eq!(float_env.sink.take(), general.msgs, "{src}");
            assert_eq!(float_env.rng, general.rng, "{src}");
        }
    }

    #[test]
    fn both_loops_stop_at_the_same_step_of_every_budget() {
        for src in FLOAT_SOURCES
            .iter()
            .filter(|s| !s.contains("5000") && !s.contains("loop(10"))
        {
            let expr = compile_ok(src);
            let needed = steps_needed_of(&expr, sample_vars, |e, env| e.eval(&mut env.cx()));
            assert_eq!(
                needed,
                steps_needed_of(&expr, sample_vars, eval_general),
                "{src}"
            );
            for budget in 0..=needed + 1 {
                assert_eq!(
                    outcome(&expr, false, budget),
                    outcome(&expr, true, budget),
                    "{src}, budget {budget}"
                );
            }
        }
    }

    #[test]
    fn both_loops_agree_when_the_loop_guard_ends_a_loop() {
        let expr = compile_ok("loop(5000, { v.n = v.n + 1; }); return v.n;");
        let (float, general) = (
            outcome(&expr, false, u64::MAX),
            outcome(&expr, true, u64::MAX),
        );
        assert_eq!(float, general);
        assert_eq!(
            float.msgs,
            ["molangx: loop stopped after its budget of 1024 iterations"]
        );
    }

    #[test]
    fn the_loop_is_picked_by_the_program_not_by_the_values() {
        let expr = compile_ok("q.position(v.x) + 1");
        assert!(!expr.program().unwrap().float_loop);
        let mut env = server_env();
        table(&mut env).set(query::POSITION, first_arg).unwrap();
        set(&mut env, &[("x", 4.0)]);
        assert_eq!(expr.eval(&mut env.cx()), NV::Float(5.0));
    }

    #[test]
    fn constant_hash_recognises_a_lone_string_program() {
        let mut p = asm(vec![Instr::Hash { h: HashIdx(1) }, Instr::End]);
        p.hashes = vec![5, 9].into_boxed_slice();
        assert_eq!(constant_hash(&p), Some(9));
        for code in [
            vec![
                Instr::Hash { h: HashIdx(0) },
                Instr::Hash { h: HashIdx(0) },
                Instr::End,
            ],
            vec![Instr::Const { c: ConstIdx(0) }, Instr::End],
            vec![Instr::Hash { h: HashIdx(0) }],
            vec![],
        ] {
            let mut p = asm(code);
            p.hashes = vec![5].into_boxed_slice();
            assert_eq!(constant_hash(&p), None);
        }
    }

    #[test]
    fn this_reads_the_subjects_value() {
        let r = ran("this * 2 + 1", |env| env.this = 3.0);
        assert_eq!(r.value, NV::Float(7.0));
    }

    #[test]
    fn a_context_string_is_read_by_the_general_loop() {
        let setup = |env: &mut NoHostEnv| {
            env.context.set(ContextName::new("k"), NV::string("moo"));
        };
        let r = ran("c.k", setup);
        assert_eq!(r.value, NV::string("moo"));
        let expr = compile_ok("c.k");
        let mut env = NoHostEnv::new();
        setup(&mut env);
        assert_eq!(
            expr.eval_f32(&mut env.cx()).to_bits(),
            NV::string("moo").as_f32().to_bits()
        );
    }

    #[test]
    fn eval_does_not_touch_the_subjects_of_the_caller() {
        let expr = compile_ok("v.x * 2");
        let mut env = NoHostEnv {
            this: 2.0,
            ..NoHostEnv::new()
        };
        set(&mut env, &[("x", 1.0)]);
        let mut cx = env.cx();
        let before = cx.subjects;
        expr.eval(&mut cx);
        expr.eval_f32(&mut cx);
        assert_eq!(cx.subjects, before);
    }

    #[test]
    fn a_context_assignment_that_parses_fails_the_link_with_only_the_compile_failed_message() {
        let c = compile(
            "return c.x = 1;\0 trailing bytes",
            &CompileOptions::server(MolangVersion::LATEST),
        );
        assert_eq!(c.failure(), Some(CompileFailure::Rejected));
        let last = c.diagnostics().last().unwrap();
        assert_eq!(
            last.message(),
            "expression 'return c.x = 1;' compile failed"
        );
        assert!(
            c.diagnostics()
                .iter()
                .all(|d| d.language_message() != Some(LanguageMessage::WriteToOtherMob))
        );
        let expr = c.expr_or_zero().cloned().unwrap();
        assert_eq!(expr.as_constant(), Some(0.0));
        assert_eq!(expr.eval(&mut NoHostEnv::new().cx()), NV::Float(0.0));
    }
}
