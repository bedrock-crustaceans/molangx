//! The bytecode VM against the tree walker: both start from copies of one test state and must agree
//! on the result (floats by bits, NaN sign and payload included), the variable and temp maps, the
//! run-time messages in order, and the random source afterwards.
//!
//! Every input runs with its own budgets, then under a sweep of small step budgets with a per-loop
//! budget of 3, so the step accounting and the state an aborted evaluation leaves behind are
//! compared too.

use crate::common::host::{Env, TestHost, World};
use molangx::compile::{Compiled, Expr};
use molangx::vm::{CollectSink, EvalLimits, Value};
use molangx_fuzz::generator::env::{FuzzRng, same_entries, same_temps, same_value};
use molangx_fuzz::tree_walker;

pub const BUDGET_SWEEP: &[u64] = &[0, 1, 2, 3, 5, 8, 13, 21, 34, 55];

fn fork(env: &Env) -> Env {
    Env {
        vars: env.vars.clone(),
        context: env.context.clone(),
        temps: env.temps.clone(),
        world: World {
            alive: env.world.alive.clone(),
            baby: env.world.baby.clone(),
        },
        queries: env.queries.clone(),
        this: env.this,
        limits: env.limits,
    }
}

pub struct Outcome {
    pub value: Value<TestHost>,
    pub env: Env,
    pub rng: FuzzRng,
    pub messages: Vec<String>,
}

/// Which evaluator [`run`] uses.
#[derive(Copy, Clone)]
pub enum Evaluator {
    Vm,
    TreeWalker,
}

/// Evaluates `expr` on copies of `env` and `rng`.
pub fn run(
    expr: &Expr,
    env: &Env,
    rng: &FuzzRng,
    limits: EvalLimits,
    evaluator: Evaluator,
) -> Outcome {
    let mut env = fork(env);
    env.limits = limits;
    let mut rng = rng.clone();
    let mut sink = CollectSink::new();
    let value = env.with_cx(&mut rng, &mut sink, |cx| match evaluator {
        Evaluator::Vm => expr.eval(cx),
        Evaluator::TreeWalker => tree_walker::eval(expr, cx),
    });
    Outcome {
        value,
        env,
        rng,
        messages: sink.take(),
    }
}

/// Runs `expr` on both evaluators: the VM's outcome, and why the tree walker's differs, or `None`.
pub fn differential(
    expr: &Expr,
    env: &Env,
    rng: &FuzzRng,
    limits: EvalLimits,
) -> (Outcome, Option<String>) {
    let vm = run(expr, env, rng, limits, Evaluator::Vm);
    let walker = run(expr, env, rng, limits, Evaluator::TreeWalker);
    let why = compare(&vm, &walker);
    (vm, why)
}

/// Why two outcomes differ, or `None`.
fn compare(vm: &Outcome, walker: &Outcome) -> Option<String> {
    let mut why = Vec::new();
    if !same_value(&vm.value, &walker.value) {
        why.push(format!(
            "value: vm {:?}, walker {:?}",
            vm.value, walker.value
        ));
    }
    if !same_entries(&vm.env.vars, &walker.env.vars) {
        why.push(format!(
            "variables: vm {:?}, walker {:?}",
            vm.env.vars, walker.env.vars
        ));
    }
    if !same_temps(vm.env.temps.as_ref(), walker.env.temps.as_ref()) {
        why.push(format!(
            "temps: vm {:?}, walker {:?}",
            vm.env.temps, walker.env.temps
        ));
    }
    if vm.messages != walker.messages {
        why.push(format!(
            "messages: vm {:?}, walker {:?}",
            vm.messages, walker.messages
        ));
    }
    if vm.rng != walker.rng {
        why.push(format!(
            "random source: vm {:?}, walker {:?}",
            vm.rng, walker.rng
        ));
    }
    (!why.is_empty()).then(|| why.join("; "))
}

#[derive(Default)]
pub struct Tally {
    pub expressions: usize,
    /// Each expression once under its own budgets and once per sweep budget.
    pub evaluations: usize,
    /// Expressions outside `tree_walker::models`: run on the VM only.
    pub not_modelled: usize,
    pub failures: Vec<String>,
}

impl Tally {
    /// Compares one expression under the input's limits and the sweep. Returns the VM's outcome
    /// under the input's limits, for the replay to carry on from, or `None` when nothing compiled.
    pub fn check(
        &mut self,
        what: &str,
        source: &str,
        compiled: &Compiled,
        env: &Env,
        rng: &FuzzRng,
    ) -> Option<Outcome> {
        let expr = compiled.expr()?;
        if !tree_walker::models(expr) {
            self.not_modelled += 1;
            return Some(run(expr, env, rng, env.limits, Evaluator::Vm));
        }
        self.expressions += 1;
        let mut first = None;
        let mut disagreement = None;
        let sweep = BUDGET_SWEEP.iter().map(|&steps| EvalLimits {
            loop_iterations: Some(3),
            total_steps: Some(steps),
            ..EvalLimits::DEFAULT
        });
        for limits in std::iter::once(env.limits).chain(sweep) {
            self.evaluations += 1;
            let (vm, why) = differential(expr, env, rng, limits);
            if let Some(why) = why {
                disagreement.get_or_insert_with(|| {
                    format!(
                        "{what} {source:?} (v{}) under {limits:?}: {why}",
                        expr.version().as_i16()
                    )
                });
            }
            first.get_or_insert(vm);
        }
        self.failures.extend(disagreement);
        first
    }

    pub fn assert_clean(&self, what: &str) {
        assert!(
            self.failures.is_empty(),
            "{what}: {} VM / tree walker disagreements:\n{}",
            self.failures.len(),
            self.failures.join("\n")
        );
    }
}
