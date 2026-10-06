//! Query calls, whose arguments are evaluated on demand.

use super::walker::with_post;
use super::{Eval, Unwind, Walker};
use molangx::internals::{Node, Payload, query_cx};
use molangx::rng::rand_core::Rng;
use molangx::vm::{
    ContextName, EvalCx, Host, QueryBackend, RuntimeMsg, RuntimeSink, Subjects, Value, VariableName,
};

impl<H: Host> Walker<H> {
    /// A query call (`QueryFunction`): the host's function with the arguments unevaluated;
    /// each argument it asks for runs as a separate expression.
    #[inline(never)]
    pub(super) fn query(&mut self, n: &Node, cx: &mut EvalCx<'_, '_, H>) -> Eval<H> {
        let Payload::Query(query) = n.value() else {
            return Ok(Value::ZERO);
        };
        self.step(cx)?;
        let queries = cx.queries;
        let version = self.version;
        let catalog = self.catalog.clone();
        let result = {
            let mut backend = ArgEval {
                walker: self,
                cx,
                args: n.children(),
            };
            let mut call = query_cx(&catalog, *query, version, &mut backend);
            match queries {
                Some(queries) => queries.call(&mut call),
                None => call.default_value(),
            }
        };
        if self.stopped {
            return Err(Unwind::Stop);
        }
        Ok(with_post(result, n.post()))
    }
}

/// The evaluator's side of a running query: arguments evaluated on demand by the walker.
struct ArgEval<'r, 'a, 'w, H: Host> {
    walker: &'r mut Walker<H>,
    cx: &'r mut EvalCx<'a, 'w, H>,
    args: &'r [Node],
}

impl<'w, H: Host> QueryBackend<'w, H> for ArgEval<'_, '_, 'w, H> {
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
        self.args.len()
    }

    fn eval_arg(&mut self, index: usize) -> Option<Value<H>> {
        let arg = self.args.get(index)?;
        Some(self.walker.run_arg(arg, self.cx))
    }
}

impl<H: Host> Walker<H> {
    /// Runs a query argument one level deeper, under the query-argument budget
    /// (`EvalLimits::query_depth`): an argument past it ends the evaluation. The budget is checked
    /// after a stopped evaluation returns 0, before anything runs.
    fn run_arg(&mut self, arg: &Node, cx: &mut EvalCx<'_, '_, H>) -> Value<H> {
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
        self.arg_depth += 1;
        let value = self.run(arg, cx);
        self.arg_depth -= 1;
        value
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tree_walker::test_support::*;
    use molangx::rng::{Xorshift128, sample};
    use molangx::vm::QueryCx;

    #[test]
    fn an_arrow_inside_a_query_argument_sees_the_target_and_leaves_the_caller_untouched() {
        // The argument of `sum_test` runs as its own program; its `->` is entered and left inside
        // it.
        let run = both("q.sum_test(1) + q.sum_test(this)");
        assert_eq!(run.f(), 3.5);
        let run = both("q.sum_test(this) + (c.other -> q.sum_test(this)) + this");
        assert_eq!(run.f(), 2.5 + 12.0 + 2.5);
    }

    /// Asks for arguments the call does not have: there are none, and nothing runs.
    #[allow(
        clippy::unnecessary_wraps,
        reason = "a query returns a `QueryResult` even when it cannot fail"
    )]
    fn beyond(cx: &mut QueryCx<'_, '_, World>) -> molangx::vm::QueryResult<World> {
        let count = cx.arg_count();
        let missing = cx.arg(count).is_none() && cx.arg(usize::MAX).is_none();
        Ok(Value::Float(if missing { 1.0 } else { 2.0 }))
    }

    #[test]
    fn an_argument_the_call_does_not_have_is_none_and_evaluates_nothing() {
        let mut start = Env::new();
        start
            .queries
            .set(molangx::internals::reference_catalog::SUM_TEST, beyond)
            .unwrap();
        for src in ["q.sum_test", "q.sum_test(math.random(0, 1))"] {
            let run = both_on(src, &start);
            assert_eq!(run.f(), 1.0, "{src}");
            assert_eq!(
                run.env.rng,
                FuzzRng::Xorshift(Xorshift128::new()),
                "{src}: no argument ran"
            );
        }
    }

    #[test]
    fn query_arguments_are_evaluated_only_when_the_query_asks() {
        // `get_name_test` evaluates its first argument only.
        let run = both("q.get_name_test(1, math.random(0, 1))");
        assert_eq!(run.f(), 101.0);
        assert_eq!(
            run.env.rng,
            FuzzRng::Xorshift(Xorshift128::new()),
            "the second argument was not evaluated"
        );
        // `log` evaluates all of them, in order.
        let run = both("q.log(math.random(0, 1), math.random(0, 1), math.random(0, 1))");
        let mut rng = Xorshift128::new();
        let first = sample(&mut rng);
        sample(&mut rng);
        sample(&mut rng);
        assert_eq!(bits(run.f()), bits(first));
        assert_eq!(
            run.env.rng,
            FuzzRng::Xorshift(rng),
            "all three arguments were evaluated"
        );
        assert_eq!(float("q.sum_test(1, v.x, v.y)"), 2.0);
        assert_eq!(float("q.sum_test"), 0.0);
        assert_eq!(float("q.sum_test(1, 2, 3) + 1"), 7.0);
    }

    #[test]
    fn a_query_argument_is_its_own_program() {
        // A missing read in an argument ends only the argument (with 0): the call goes on.
        let run = both("q.sum_test(v.nothing, 5)");
        assert_eq!(run.f(), 5.0);
        assert_eq!(run.env.messages().len(), 1);
        // An argument that reads `this` sees the subjects of the call.
        assert_eq!(both("q.sum_test(this, 1)").f(), 3.5);
    }

    #[test]
    fn a_query_draws_from_the_shared_random_source() {
        let run = both_on(
            "q.experimental_test + q.experimental_test",
            &Env::new().with_fixed_random(0.25),
        );
        assert_eq!(run.f(), 0.5);
        let run = both("q.experimental_test");
        let mut expected = Xorshift128::new();
        assert_eq!(bits(run.f()), bits(sample(&mut expected)));
        assert_eq!(run.env.rng, FuzzRng::Xorshift(expected));
    }

    #[test]
    fn query_arguments_nest_only_as_deep_as_the_budget() {
        let mut start = Env::new();
        start.limits.query_depth = Some(1);
        let run = both_on("q.sum_test(1, 2)", &start);
        assert_eq!(run.f(), 3.0, "one level of arguments is allowed");
        let run = both_on("q.sum_test(q.sum_test(1), 2)", &start);
        assert_eq!(run.f(), 0.0);
        assert_eq!(
            run.env.messages(),
            [
                "molangx: evaluation stopped: query arguments would nest deeper than their budget of 1 levels"
            ]
        );
        start.limits.query_depth = Some(0);
        let run = both_on("q.sum_test(1)", &start);
        assert_eq!(run.f(), 0.0);
        assert_eq!(run.env.messages().len(), 1);
    }

    #[test]
    fn run_arg_enforces_the_depth_budget_before_anything_runs() {
        let expr = build("v.x");
        let tree = tree(&expr).expect("a tree");
        let mut walker = Walker::<World>::new(&expr);
        let mut env = Env::new();
        env.limits.query_depth = Some(1);
        let mut cx = env.cx();
        assert_eq!(walker.run_arg(tree, &mut cx), Value::Float(3.0));
        assert_eq!(walker.arg_depth, 0);
        assert_eq!(walker.steps, 2);
        let steps_before = walker.steps;
        walker.arg_depth = 1;
        assert_eq!(walker.run_arg(tree, &mut cx), Value::ZERO);
        assert!(walker.stopped);
        assert_eq!(walker.steps, steps_before, "nothing ran");
        // Once stopped, a further argument is zero without a second message.
        assert_eq!(walker.run_arg(tree, &mut cx), Value::ZERO);
        assert_eq!(env.messages().len(), 1);
    }
}
