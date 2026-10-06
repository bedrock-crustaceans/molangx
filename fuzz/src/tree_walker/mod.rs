//! A second evaluator that walks the optimised expression tree, for differential tests against the
//! bytecode VM. Trees are at most 256 levels deep, so the recursion is bounded.
//!
//! The step budget is the one place it follows the bytecode: each node is charged the steps its
//! lowering executes, when that instruction would run.
//!
//! A jump out of a `??` left side followed by a missing read resumes the abandoned right side in
//! the VM; a tree cannot express that, so [`models`] excludes such expressions.
//!
//! These rules are the VM's by construction, so the differential tests cannot judge them:
//! - which operand of `*` is scaled first under `Arm64`;
//! - `==` between kinds without payload bits is false;
//! - a constant operand counts as the second operand whichever side it was written on;
//! - a `Float` that carries a post-op is worth its bare value wherever it stays a node.

mod arith;
mod flow;
mod predicate;
mod query;
mod values;
mod walker;

pub use predicate::models;

use molangx::catalog::{MathCatalog, QueryCatalog};
use molangx::compile::Expr;
use molangx::hash::HashedStr;
use molangx::internals::{Payload, tree};
use molangx::ops::ExpressionOp as Op;
use molangx::version::MolangVersion;
use molangx::vm::{EvalCx, Host, Subjects, TempMap, Value};

/// Evaluates `expr` by walking its optimised tree. [`Expr::eval`] must give the same value, writes,
/// random draws, messages and budget outcome.
///
/// Like [`Expr::eval`], it leaves `cx.subjects` as it found them.
///
/// ```
/// use molangx::compile::{CompileOptions, compile};
/// use molangx::version::MolangVersion;
/// use molangx::vm::NoHostEnv;
/// use molangx_fuzz::tree_walker;
///
/// let (expr, _) = compile(
///     "t.a = 2; return t.a * math.clamp(7, 0, 5);",
///     &CompileOptions::server(MolangVersion::LATEST),
/// )
/// .into_result()
/// .unwrap();
/// let mut env = NoHostEnv::new();
/// assert_eq!(tree_walker::eval(&expr, &mut env.cx()).as_f32(), 10.0);
/// assert_eq!(expr.eval(&mut env.cx()).as_f32(), 10.0);
/// ```
pub fn eval<H: Host>(expr: &Expr, cx: &mut EvalCx<'_, '_, H>) -> Value<H> {
    // A folded or rejected expression runs nothing.
    if let Some(constant) = expr.as_constant() {
        return Value::Float(constant);
    }
    let Some(tree) = tree(expr) else {
        return Value::ZERO;
    };
    // A string-literal tree is constant: its hash, nothing runs.
    if let (Op::StringLiteral, Payload::Hash(hash)) = (tree.op(), tree.value()) {
        return Value::Hash(HashedStr::from_u64(*hash));
    }
    let subjects = cx.subjects;
    let mut walker = Walker::new(expr);
    let result = walker.run(tree, cx);
    cx.subjects = subjects;
    result
}

/// Why the walk of a node stopped before producing a value.
enum Unwind<H: Host> {
    /// A missing variable or member. Caught by the innermost `??` of the same (sub-)program;
    /// without one it was logged and ends the (sub-)program with 0.
    Missing,
    /// `return`: ends the (sub-)program with this value (its post-op applied).
    Return(Value<H>),
    /// `break` inside a loop of the same (sub-)program.
    Break,
    /// `continue` inside a loop of the same (sub-)program.
    Continue,
    /// `break` / `continue` with no loop around it: the (sub-)program ends with 0.
    Halt,
    /// The step budget is spent: the whole evaluation ends with 0, arguments included.
    Stop,
}

type Eval<H> = Result<Value<H>, Unwind<H>>;

/// What one (sub-)program sees of its surroundings. A query argument is a separate expression: it
/// starts with no `??` and no loop of its own.
#[derive(Copy, Clone, Default)]
struct Scope {
    /// The current node is inside a `??` left side.
    in_handler: bool,
    /// The current node is inside a loop body.
    in_loop: bool,
}

struct Walker<H: Host> {
    version: MolangVersion,
    /// The catalogue the expression was compiled against, which its query leaves index into.
    catalog: QueryCatalog,
    /// The host math functions the expression was compiled with, which its host math nodes index
    /// into.
    math: Option<MathCatalog>,
    /// The signed division guard (version ≥ 7), or the guard on the divisor's magnitude (≤ 6).
    signed_division: bool,
    steps: u64,
    /// The budget ended the evaluation.
    stopped: bool,
    /// The loop guard has reported: once per evaluation.
    loop_guard_logged: bool,
    /// How many query arguments are running inside each other (`EvalLimits::query_depth`).
    arg_depth: u32,
    /// The caller's subjects of every `->` entered and not yet left; its length is the `->`
    /// depth.
    arrows: Vec<Subjects<H>>,
    /// `temp.*` when the host keeps no temp map: empty at the start of the evaluation, shared with
    /// its query arguments.
    temps: TempMap<H>,
    scope: Scope,
}

#[cfg(test)]
pub(crate) mod test_support {
    //! Test world shared by the unit tests of the evaluator's files.

    // The query functions return a `QueryResult` even when they cannot fail.
    #![allow(clippy::unnecessary_wraps)]

    pub(crate) use crate::generator::env::FuzzRng;
    pub(crate) use crate::generator::env::{same_entries, same_temps, same_value};
    use crate::tree_walker::{Unwind, Walker, eval};
    use molangx::compile::{Expr, compile};
    use molangx::internals::reference_catalog;
    pub(crate) use molangx::internals::reference_catalog::options;
    pub(crate) use molangx::internals::tree;
    use molangx::internals::{Node, Payload};
    use molangx::numeric::PostOp;
    use molangx::ops::ExpressionOp as Op;
    use molangx::rng::{FixedRng, Xorshift128};
    use molangx::stdlib::query;
    use molangx::vm::{
        CollectSink, ContextMap, ContextName, EvalCx, EvalLimits, Host, HostAccess, QueryCx,
        QueryError, QueryTable, Subjects, TempMap, TempName, Temps, Value, VariableMap,
        VariableName,
    };

    /// A host whose actors are numbers: actors below 100 are alive, `->` to actor `n` sets `this`
    /// to `10 + n`.
    #[derive(Clone, Debug)]
    pub(crate) struct World;

    impl Host for World {
        type ActorRef = u32;
        type ItemRef = u16;
        type BlockRef = ();
        type Access<'w> = World;
    }

    impl HostAccess<World> for World {
        fn resolve_actor(&self, _from: &Subjects<World>, actor: u32) -> Option<u32> {
            (actor < 100).then_some(actor)
        }

        fn subjects_of(&self, actor: u32) -> Subjects<World> {
            Subjects {
                this: 10.0 + actor as f32,
                ..Subjects::actor(actor)
            }
        }
    }

    pub(crate) type V = Value<World>;

    #[derive(Clone)]
    pub(crate) struct Env {
        pub(crate) host: World,
        pub(crate) vars: VariableMap<World>,
        pub(crate) context: ContextMap<World>,
        pub(crate) queries: QueryTable<World>,
        pub(crate) rng: FuzzRng,
        pub(crate) sink: CollectSink,
        pub(crate) limits: EvalLimits,
        pub(crate) temps: Option<TempMap<World>>,
        pub(crate) subject: Option<u32>,
    }

    pub(crate) fn var(name: &str) -> VariableName {
        VariableName::new(name)
    }

    pub(crate) fn temp_key(name: &str) -> TempName {
        TempName::new(name)
    }

    /// `query.log(a, b, …)`: evaluates every argument, in order, and returns the first.
    pub(crate) fn log(cx: &mut QueryCx<'_, '_, World>) -> Result<V, QueryError> {
        let first = cx.arg(0).unwrap_or_default();
        for i in 1..cx.arg_count() {
            let _ = cx.arg(i);
        }
        Ok(first)
    }

    /// Evaluates only the first argument: the others stay unevaluated.
    pub(crate) fn first_only(cx: &mut QueryCx<'_, '_, World>) -> Result<V, QueryError> {
        Ok(Value::Float(cx.arg_f32(0).unwrap_or(0.0) + 100.0))
    }

    pub(crate) fn sum(cx: &mut QueryCx<'_, '_, World>) -> Result<V, QueryError> {
        let mut total = 0.0f32;
        for i in 0..cx.arg_count() {
            total += cx.arg_f32(i).unwrap_or(0.0);
        }
        Ok(Value::Float(total))
    }

    pub(crate) fn draw(cx: &mut QueryCx<'_, '_, World>) -> Result<V, QueryError> {
        Ok(Value::Float(molangx::rng::sample(cx.rng())))
    }

    impl Env {
        /// A few variables, a context, the test queries and the xorshift sequence.
        pub(crate) fn new() -> Self {
            let mut vars = VariableMap::new();
            vars.set(var("x"), Value::Float(3.0));
            vars.set(var("y"), Value::Float(-2.0));
            vars.set(var("s"), Value::string("moo"));
            vars.set(var("zero"), Value::Float(0.0));
            let context = ContextMap::from([
                (ContextName::new("other"), Value::Actor(2)),
                (ContextName::new("n"), Value::Float(4.0)),
                (ContextName::new("arr"), Value::actor_array([1, 2, 150])),
            ]);
            let mut queries = QueryTable::new(reference_catalog::catalog());
            queries.set(query::LOG, log).unwrap();
            queries.set(reference_catalog::SUM_TEST, sum).unwrap();
            queries
                .set(reference_catalog::GET_NAME_TEST, first_only)
                .unwrap();
            queries
                .set(reference_catalog::EXPERIMENTAL_TEST, draw)
                .unwrap();
            Self {
                host: World,
                vars,
                context,
                queries,
                rng: FuzzRng::Xorshift(Xorshift128::new()),
                sink: CollectSink::new(),
                limits: EvalLimits::DEFAULT,
                temps: None,
                subject: None,
            }
        }

        pub(crate) fn with_var(mut self, name: &str, value: V) -> Self {
            self.vars.set(var(name), value);
            self
        }

        pub(crate) fn with_steps(mut self, steps: u64) -> Self {
            self.limits.total_steps = Some(steps);
            self
        }

        pub(crate) fn with_fixed_random(mut self, sample: f32) -> Self {
            self.rng = FuzzRng::Fixed(FixedRng::from_sample(sample).expect("a word's sample"));
            self
        }

        pub(crate) fn cx(&mut self) -> EvalCx<'_, 'static, World> {
            let subjects = match self.subject {
                Some(actor) => Subjects::actor(actor),
                None => Subjects::none(),
            };
            EvalCx {
                subjects: Subjects {
                    this: 2.5,
                    ..subjects
                },
                host: &mut self.host,
                variables: &mut self.vars,
                context: &self.context,
                queries: Some(&self.queries),
                rng: &mut self.rng,
                sink: &mut self.sink,
                limits: self.limits,
                temps: match &mut self.temps {
                    Some(temps) => Temps::Kept(temps),
                    None => Temps::PerEvaluation,
                },
            }
        }

        pub(crate) fn float(&self, name: &str) -> f32 {
            self.vars.get(var(name)).map_or(f32::NAN, V::as_f32)
        }

        pub(crate) fn messages(&self) -> &[String] {
            &self.sink.messages
        }
    }

    pub(crate) fn build_at(src: &str, version: i16) -> Expr {
        let compiled = compile(src, &options(version));
        let expr = compiled
            .expr()
            .cloned()
            .unwrap_or_else(|| panic!("{src:?} does not compile: {:?}", compiled.diagnostics()));
        assert!(
            tree(&expr).is_some(),
            "{src:?} was rejected: {:?}",
            compiled.diagnostics()
        );
        expr
    }

    pub(crate) fn build(src: &str) -> Expr {
        build_at(src, 13)
    }

    pub(crate) struct Run {
        pub(crate) value: V,
        pub(crate) env: Env,
    }

    impl Run {
        pub(crate) fn f(&self) -> f32 {
            self.value.as_f32()
        }
    }

    pub(crate) fn vm_run(expr: &Expr, start: &Env) -> Run {
        let mut env = start.clone();
        let value = expr.eval(&mut env.cx());
        Run { value, env }
    }

    pub(crate) fn walker_run(expr: &Expr, start: &Env) -> Run {
        let mut env = start.clone();
        let value = eval(expr, &mut env.cx());
        Run { value, env }
    }

    pub(crate) fn assert_agree(what: &str, vm: &Run, walker: &Run) {
        assert!(
            same_value(&vm.value, &walker.value),
            "{what}: vm {:?}, walker {:?}",
            vm.value,
            walker.value
        );
        assert!(
            same_entries(&vm.env.vars, &walker.env.vars),
            "{what}: variables differ: {:?} vs {:?}",
            vm.env.vars,
            walker.env.vars
        );
        assert!(
            same_temps(vm.env.temps.as_ref(), walker.env.temps.as_ref()),
            "{what}: temps differ"
        );
        assert_eq!(
            vm.env.sink.messages, walker.env.sink.messages,
            "{what}: messages differ"
        );
        assert_eq!(vm.env.rng, walker.env.rng, "{what}: random state differs");
    }

    /// Runs `src` on both evaluators, asserts they agree and returns the tree walker's run.
    pub(crate) fn both_on(src: &str, start: &Env) -> Run {
        let expr = build(src);
        let vm = vm_run(&expr, start);
        let walker = walker_run(&expr, start);
        assert_agree(src, &vm, &walker);
        walker
    }

    pub(crate) fn both(src: &str) -> Run {
        both_on(src, &Env::new())
    }

    pub(crate) fn float(src: &str) -> f32 {
        both(src).f()
    }

    pub(crate) fn bits(x: f32) -> u32 {
        x.to_bits()
    }

    pub(crate) const STEP_MESSAGE: &str = "molangx: evaluation stopped after its budget of ";

    /// The smallest `total_steps` with which `src` completes (no step-limit message) on `start`,
    /// asserted to be the same for the VM and the tree walker.
    pub(crate) fn smallest_budget(src: &str, start: &Env) -> u64 {
        let expr = build(src);
        let smallest = |run: fn(&Expr, &Env) -> Run| {
            (0..2_000u64).find(|&budget| {
                !run(&expr, &start.clone().with_steps(budget))
                    .env
                    .messages()
                    .iter()
                    .any(|m| m.starts_with(STEP_MESSAGE))
            })
        };
        let vm = smallest(vm_run);
        assert_eq!(
            vm,
            smallest(walker_run),
            "{src}: the VM and the tree walker need different budgets"
        );
        vm.unwrap_or_else(|| panic!("{src}: no budget below 2000 completes"))
    }

    /// Pins the step count of `src` and checks the budget boundary: the smallest completing budget
    /// is `expected` and gives the unlimited result; every smaller budget stops both evaluators in
    /// the same state with the step-limit message and the value 0.
    pub(crate) fn assert_steps(src: &str, start: &Env, expected: u64) {
        let needed = smallest_budget(src, start);
        assert_eq!(needed, expected, "{src}: step count");
        let expr = build(src);
        let unlimited = walker_run(&expr, start);
        let exact = start.clone().with_steps(needed);
        let (vm, walker) = (vm_run(&expr, &exact), walker_run(&expr, &exact));
        assert_agree(src, &vm, &walker);
        assert!(
            same_value(&walker.value, &unlimited.value),
            "{src}: the exact budget changes the value"
        );
        assert_eq!(walker.env.sink.messages, unlimited.env.sink.messages);
        for budget in 0..needed {
            let limited = start.clone().with_steps(budget);
            let (vm, walker) = (vm_run(&expr, &limited), walker_run(&expr, &limited));
            assert_agree(&format!("{src} with {budget} steps"), &vm, &walker);
            assert_eq!(
                walker.f(),
                0.0,
                "{src} with {budget} steps: the value of a stopped evaluation"
            );
            let last = walker
                .env
                .messages()
                .last()
                .expect("a stopped evaluation says so");
            assert_eq!(
                last,
                &format!("{STEP_MESSAGE}{budget} steps"),
                "{src} with {budget} steps"
            );
        }
    }

    pub(crate) fn steps(src: &str, expected: u64) {
        assert_steps(src, &Env::new(), expected);
    }

    pub(crate) fn node(op: Op, children: Vec<Node>) -> Node {
        Node::new(op, Payload::None, PostOp::IDENTITY, children)
    }

    pub(crate) fn leaf(op: Op, value: Payload) -> Node {
        Node::new(op, value, PostOp::IDENTITY, Vec::new())
    }

    pub(crate) fn number(x: f32) -> Node {
        leaf(Op::Float, Payload::Float(x))
    }

    pub(super) fn walker_for(src: &str) -> Walker<World> {
        Walker::new(&build(src))
    }

    pub(super) fn is_stop<T>(r: &Result<T, Unwind<World>>) -> bool {
        matches!(r, Err(Unwind::Stop))
    }

    pub(crate) fn read(op: Op, canonical: &str) -> Node {
        let name = molangx::internals::Name::new(canonical);
        let payload = match op {
            Op::EntityVariable => Payload::Entity(name),
            Op::TempVariable => Payload::Temp(name),
            _ => Payload::Context(name),
        };
        leaf(op, payload)
    }

    /// `c.other -> t.nothing`: a pointer whose right side reads a temp that does not exist. (The
    /// compiler refuses a temp on the right of `->`, so the tree is built by hand.)
    pub(crate) fn arrow_to_a_missing_temp() -> Node {
        node(
            Op::Pointer,
            vec![
                read(Op::ContextVariable, "context.other"),
                read(Op::TempVariable, "temp.nothing"),
            ],
        )
    }

    pub(super) fn walk(
        walker: &mut Walker<World>,
        tree: &Node,
        cx: &mut EvalCx<'_, '_, World>,
    ) -> Option<V> {
        walker.node(tree, cx).ok()
    }
}

#[cfg(test)]
mod tests {
    //! The tree walker against the VM construct by construct. The step model is pinned as the
    //! smallest `total_steps` with which an expression completes.

    use super::*;
    use crate::tree_walker::test_support::*;
    use molangx::compile::compile;

    #[test]
    fn a_folded_constant_runs_nothing_and_needs_no_budget() {
        let expr = build("1 + 2");
        let start = Env::new().with_steps(0);
        let run = walker_run(&expr, &start);
        assert_eq!(run.value, Value::Float(3.0));
        assert!(run.env.messages().is_empty());
        assert_agree("1 + 2", &vm_run(&expr, &start), &run);
    }

    #[test]
    fn a_string_literal_is_its_hash_and_runs_nothing() {
        let start = Env::new().with_steps(0);
        let expr = build("'moo'");
        let run = walker_run(&expr, &start);
        assert_eq!(run.value, Value::string("moo"));
        assert!(run.env.messages().is_empty());
        assert_agree("'moo'", &vm_run(&expr, &start), &run);
    }

    #[test]
    fn a_rejected_expression_is_the_constant_zero() {
        let expr = compile("v.x +", &options(13))
            .expr_or_zero()
            .cloned()
            .expect("a rejected source still gives an expression");
        assert!(tree(&expr).is_none(), "no tree for a rejected expression");
        let start = Env::new().with_steps(0);
        let (vm, walker) = (vm_run(&expr, &start), walker_run(&expr, &start));
        assert_eq!(walker.value, Value::ZERO);
        assert!(walker.env.messages().is_empty());
        assert_agree("rejected", &vm, &walker);
        assert!(models(&expr), "a rejected expression is trivially modelled");
    }

    #[test]
    fn eval_restores_the_subjects_a_pointer_switched() {
        let expr = build("c.other -> v.x");
        let mut env = Env::new();
        env.subject = Some(7);
        let mut cx = env.cx();
        let before = cx.subjects;
        let value = eval(&expr, &mut cx);
        assert_eq!(value, Value::Float(0.0), "no public snapshot of v.x yet");
        assert_eq!(cx.subjects, before);
    }

    #[test]
    fn eval_restores_the_subjects_after_a_break_left_a_pointer() {
        // `break` inside `->` leaves the subjects switched until the end of the program; `eval` and
        // the VM still hand the caller's subjects back.
        let expr = build("loop(1, { c.other -> { break; }; }); return 1;");
        for walker in [false, true] {
            let mut env = Env::new();
            env.subject = Some(7);
            let mut cx = env.cx();
            let before = cx.subjects;
            let value = if walker {
                eval(&expr, &mut cx)
            } else {
                expr.eval(&mut cx)
            };
            assert_eq!(value.as_f32(), 1.0, "walker: {walker}");
            assert_eq!(cx.subjects, before, "walker: {walker}");
        }
    }

    #[test]
    fn a_nan_result_is_the_same_on_both_evaluators() {
        let run = both("math.sqrt(v.y)");
        assert!(run.f().is_nan());
        let run = both("v.n = math.sqrt(v.y); return v.n;");
        assert!(run.f().is_nan());
        assert!(run.env.float("n").is_nan());
    }

    #[test]
    fn the_step_model_of_simple_expressions() {
        // Each read, each push, the operation and the `End` is one step; a negation is folded into
        // the read's post-op.
        steps("v.x", 2);
        steps("v.x + v.y", 5);
        steps("v.x * v.y", 5);
        steps("v.x - v.y", 5);
        steps("-v.x", 2);
        steps("!v.x", 3);
        steps("math.abs(v.x)", 3);
        steps("math.max(v.x, v.y)", 5);
        steps("math.clamp(v.x, 0, 1)", 7);
        steps("this", 2);
        steps("c.n", 2);
        steps("return 5;", 2);
        steps("return v.x;", 2);
    }

    #[test]
    fn the_step_model_of_division_skips_the_numerator_for_a_zero_divisor() {
        steps("v.x / v.y", 5);
        steps("v.x / v.zero", 3);
    }

    #[test]
    fn the_step_model_of_comparisons_logic_and_branches() {
        steps("v.x < v.y", 5);
        steps("v.x == v.y", 5);
        steps("v.x == 3", 3);
        steps("v.x && v.y", 5);
        steps("v.zero && v.y", 3);
        steps("v.x || v.y", 3);
        steps("v.zero || v.y", 5);
        steps("v.x ? 1 : 2", 5);
        steps("v.zero ? 1 : 2", 4);
        steps("v.x ? 1", 5);
        steps("v.zero ? 1", 4);
        steps("v.x ?? 1", 4);
        steps("v.nothing ?? 1", 4);
    }

    #[test]
    fn the_step_model_of_assignments_and_statements() {
        steps("v.a = 1;", 4);
        steps("v.a = v.x;", 4);
        steps("t.a = 1; return t.a;", 4);
        steps("v.a = 1; v.b = 2; v.c = 3;", 8);
        steps("v.a = 1; v.a = 2;", 6);
        // A member store on a variable that holds nothing yet pays the struct copy: four steps.
        steps("v.s2.m = 1;", 8);
    }

    #[test]
    fn the_step_model_of_loops_counts_the_back_edges_and_the_exit() {
        steps("loop(0, { v.x; });", 5);
        steps("loop(1, { v.x; });", 9);
        steps("loop(3, { v.x; });", 15);
        steps("loop(3, { break; });", 7);
        steps("loop(3, { continue; });", 12);
    }

    #[test]
    fn each_loop_iteration_costs_the_same_number_of_steps() {
        let per_iteration = smallest_budget("loop(2, { v.x; });", &Env::new())
            - smallest_budget("loop(1, { v.x; });", &Env::new());
        assert_eq!(per_iteration, 3);
        assert_eq!(
            smallest_budget("loop(5, { v.x; });", &Env::new())
                - smallest_budget("loop(4, { v.x; });", &Env::new()),
            per_iteration
        );
    }

    #[test]
    fn the_step_model_of_for_each_and_queries_and_random() {
        // Two of the three entries of `c.arr` are alive.
        steps("for_each(t.e, c.arr, { v.x; });", 15);
        steps("q.sum_test(1, v.x)", 6);
        steps("q.get_name_test(1)", 4);
        steps("math.random(v.zero, v.x)", 5);
        steps("math.random(1, 2)", 2);
        steps("c.other -> q.sum_test(this)", 7);
    }

    #[test]
    fn a_die_roll_costs_one_step_per_roll() {
        // Seven steps of operands, the instruction and the `End`, and one per roll (`v.x` is 3).
        steps("math.die_roll(v.x, 1, 6)", 10);
        let one = Env::new().with_var("x", Value::Float(1.0));
        let none = Env::new().with_var("x", Value::Float(0.0));
        assert_steps("math.die_roll(v.x, 1, 6)", &one, 8);
        assert_steps("math.die_roll(v.x, 1, 6)", &none, 7);
        assert_steps("math.die_roll_integer(v.x, 1, 6)", &Env::new(), 10);
    }

    #[test]
    fn the_step_model_with_a_post_op_on_the_joins() {
        // A `?:` with a post-op pays one more step where its paths meet.
        steps("(v.x ? 1 : 2) * 2", 6);
        steps("(v.x ?? 1) * 2", 5);
    }

    #[test]
    fn the_step_budget_stops_the_tree_walker_where_the_vm_stops() {
        for src in [
            "v.a = 1; loop(3, { v.a = v.a + 1; v.a > 2 ? { break; }; }); return v.a;",
            "for_each(t.e, c.arr, { v.last = t.e; }); return v.last;",
            "v.st.a.b = 5; v.n = q.sum_test(v.st.a.b, 1); return v.n;",
            "c.other -> v.x",
            "(v.nothing ?? v.x) + math.random(0, 1)",
            "math.die_roll(v.x, 1, 6)",
        ] {
            let needed = smallest_budget(src, &Env::new());
            assert!(needed > 3, "{src}: {needed}");
            assert_steps(src, &Env::new(), needed);
        }
    }
}
