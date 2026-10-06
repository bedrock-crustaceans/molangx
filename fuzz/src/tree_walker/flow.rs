//! Control flow: `&&` / `||`, `??`, the conditional, statements, jumps, `->`, `loop` and
//! `for_each`.

use super::walker::{Slot, child, with_post};
use super::{Eval, Scope, Unwind, Walker};
use molangx::internals::{Node, arrow_target};
use molangx::numeric;
use molangx::ops::ExpressionOp as Op;
use molangx::vm::{EvalCx, Host, RuntimeMsg, Value};

/// `&&` or `||`.
#[derive(Copy, Clone)]
pub(super) enum Logic {
    And,
    Or,
}

/// `break` or `continue`.
#[derive(Copy, Clone)]
pub(super) enum Jump {
    Break,
    Continue,
}

impl<H: Host> Walker<H> {
    /// `&&` / `||`: every operand but the last can decide; the last is normalised.
    #[inline(never)]
    pub(super) fn logic(&mut self, logic: Logic, n: &Node, cx: &mut EvalCx<'_, '_, H>) -> Eval<H> {
        // The truthiness of an operand that decides the result.
        let deciding = matches!(logic, Logic::Or);
        let post = n.post();
        let Some((last, rest)) = n.children().split_last() else {
            return Ok(Value::ZERO);
        };
        for operand in rest {
            let truthy = self.node(operand, cx)?.truthy();
            self.step(cx)?;
            if truthy == deciding {
                return Ok(Value::Float(post.select(truthy)));
            }
        }
        let truthy = self.node(last, cx)?.truthy();
        self.step(cx)?;
        Ok(Value::Float(post.select(truthy)))
    }

    /// `x ?? y`: a handler around the left side; a missing read in it continues at the
    /// right side. The post-op applies to either.
    #[inline(never)]
    pub(super) fn coalesce(&mut self, n: &Node, cx: &mut EvalCx<'_, '_, H>) -> Eval<H> {
        self.step(cx)?;
        let depth = self.arrows.len();
        let handler = Scope {
            in_handler: true,
            ..self.scope
        };
        let left = self.within(handler, |walker| walker.node(child(n, 0), cx));
        let value = match left {
            Ok(value) => {
                self.step(cx)?;
                value
            }
            Err(Unwind::Missing) => {
                // The handler unwinds what the left side entered.
                self.leave_arrows(cx, depth);
                self.node(child(n, 1), cx)?
            }
            Err(other) => return Err(other),
        };
        self.post_step(value, n.post(), cx)
    }

    /// `c ? a : b` and `c ? a`: without `: b` the false branch is 0; the post-op applies to
    /// either branch.
    #[inline(never)]
    pub(super) fn conditional(&mut self, n: &Node, cx: &mut EvalCx<'_, '_, H>) -> Eval<H> {
        let condition = self.node(child(n, 0), cx)?.truthy();
        self.step(cx)?;
        let value = if condition {
            let value = self.node(child(n, 1), cx)?;
            self.step(cx)?;
            value
        } else if let Some(otherwise) = n.children().get(2) {
            self.node(otherwise, cx)?
        } else {
            self.step(cx)?;
            Value::ZERO
        };
        self.post_step(value, n.post(), cx)
    }

    /// A statement list (`;`, a `{…}` block): `return x;` ends the (sub-)program with `x` and the
    /// return's post-op; otherwise the list is worth its `O`.
    #[inline(never)]
    pub(super) fn statements(&mut self, n: &Node, cx: &mut EvalCx<'_, '_, H>) -> Eval<H> {
        for statement in n.children() {
            if !statement.is(Op::Return) {
                self.node(statement, cx)?;
                continue;
            }
            let value = if let Some(value) = statement.children().first() {
                self.node(value, cx)?
            } else {
                self.step(cx)?;
                Value::ZERO
            };
            self.step(cx)?;
            return Err(Unwind::Return(with_post(value, statement.post())));
        }
        self.step(cx)?;
        Ok(Value::Float(n.post().offset))
    }

    /// `break` / `continue`: a jump inside a loop of the same (sub-)program, otherwise the end of
    /// it with 0.
    #[inline(never)]
    pub(super) fn jump(&mut self, jump: Jump, cx: &mut EvalCx<'_, '_, H>) -> Eval<H> {
        // A jump with an operand pending is not modelled (`models`): here nothing is pending, so
        // the loop's counter is what its check and cleanup see.
        self.step(cx)?;
        Err(match (self.scope.in_loop, jump) {
            (false, _) => Unwind::Halt,
            (true, Jump::Break) => Unwind::Break,
            (true, Jump::Continue) => Unwind::Continue,
        })
    }

    /// `a->b`: the left side picks the subjects of the right side; a left side that is not a
    /// live actor or an item gives `post(0)` and the right side does not run.
    #[inline(never)]
    pub(super) fn pointer(&mut self, n: &Node, cx: &mut EvalCx<'_, '_, H>) -> Eval<H> {
        let target = self.node(child(n, 0), cx)?;
        self.step(cx)?;
        let Some(subjects) = arrow_target(cx, &target) else {
            return Ok(Value::Float(n.post().apply(0.0)));
        };
        let caller = std::mem::replace(&mut cx.subjects, subjects);
        self.arrows.push(caller);
        // An unwind out of the right side leaves the subjects switched: whoever catches it
        // restores them (`??`, the end of the (sub-)program); a loop does not.
        let value = self.node(child(n, 1), cx)?;
        self.step(cx)?;
        match self.arrows.pop() {
            Some(caller) => cx.subjects = caller,
            None => cx.sink.runtime(RuntimeMsg::PublicAccessUnderflow),
        }
        Ok(with_post(value, n.post()))
    }

    /// `loop(count, body)` under the loop guard: `ceil(count)` iterations for a
    /// finite positive count, the first one unconditional, each further one only while the loop
    /// has run fewer than the per-loop budget. The count and the counter are compared with the
    /// architecture's `<=`: a NaN count makes no pass on `Arm64` and never runs out on `X86_64`.
    #[inline(never)]
    pub(super) fn counted_loop(&mut self, n: &Node, cx: &mut EvalCx<'_, '_, H>) -> Eval<H> {
        let count = self.node(child(n, 0), cx)?.as_f32();
        self.step(cx)?;
        if !numeric::le(count, 0.0) {
            self.in_loop(|walker| walker.count_down(count, child(n, 1), cx))?;
            // Leaving the loop.
            self.step(cx)?;
        }
        self.step(cx)?;
        Ok(Value::Float(n.post().apply(0.0)))
    }

    /// The passes of a `loop` whose count is above zero.
    fn count_down(
        &mut self,
        count: f32,
        body: &Node,
        cx: &mut EvalCx<'_, '_, H>,
    ) -> Result<(), Unwind<H>> {
        let mut counter = count + -1.0;
        let mut iterations: u32 = 1;
        loop {
            match self.node(body, cx) {
                Ok(_) | Err(Unwind::Continue) => {}
                Err(Unwind::Break) => return Ok(()),
                Err(other) => return Err(other),
            }
            // The back edge.
            self.step(cx)?;
            // A counter at or below zero ends the loop; anything else runs the body again.
            if numeric::le(counter, 0.0) {
                return Ok(());
            }
            if let Some(limit) = cx.limits.loop_iterations
                && iterations >= limit
            {
                self.loop_guard(cx, limit);
                return Ok(());
            }
            counter += -1.0;
            iterations = iterations.saturating_add(1);
        }
    }

    /// `for_each(var, array, body)`: only a non-empty actor array is iterated; entries that
    /// resolve to no actor are skipped; the loop variable is written like an assignment writes an
    /// actor.
    #[inline(never)]
    pub(super) fn for_each(&mut self, n: &Node, cx: &mut EvalCx<'_, '_, H>) -> Eval<H> {
        let array = self.node(child(n, 1), cx)?;
        self.step(cx)?;
        if let Value::ActorArray(entries) = array
            && !entries.is_empty()
        {
            let variable = Slot::of(child(n, 0).value());
            self.in_loop(|walker| walker.each_live(&entries, variable, child(n, 2), cx))?;
            self.step(cx)?;
        }
        self.step(cx)?;
        Ok(Value::Float(n.post().apply(0.0)))
    }

    /// The passes of a `for_each` over the live actors among `entries`.
    fn each_live(
        &mut self,
        entries: &[H::ActorRef],
        variable: Option<Slot>,
        body: &Node,
        cx: &mut EvalCx<'_, '_, H>,
    ) -> Result<(), Unwind<H>> {
        let mut entries = entries.iter();
        let mut iterations: u32 = 0;
        loop {
            // The step to the next entry.
            self.step(cx)?;
            let Some(actor) = entries.find_map(|&entry| cx.resolve_actor(entry)) else {
                return Ok(());
            };
            // The budget is tested once a live entry is found, so an array whose remaining
            // entries are all dead ends without the message.
            if let Some(limit) = cx.limits.loop_iterations
                && iterations >= limit
            {
                self.loop_guard(cx, limit);
                return Ok(());
            }
            iterations = iterations.saturating_add(1);
            if let Some(slot) = variable {
                self.store(cx, slot, Value::Actor(actor));
            }
            match self.node(body, cx) {
                // The jump back to the next entry.
                Ok(_) => self.step(cx)?,
                Err(Unwind::Continue) => {}
                Err(Unwind::Break) => return Ok(()),
                Err(other) => return Err(other),
            }
        }
    }

    /// Runs the passes of a loop with the body's scope: a `break` / `continue` inside is the
    /// loop's.
    fn in_loop<T>(&mut self, passes: impl FnOnce(&mut Self) -> T) -> T {
        let body = Scope {
            in_loop: true,
            ..self.scope
        };
        self.within(body, passes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tree_walker::test_support::*;

    #[test]
    fn and_or_short_circuit_and_normalise_the_last_operand() {
        assert_eq!(float("v.x && v.y"), 1.0);
        assert_eq!(float("v.x && v.zero"), 0.0);
        assert_eq!(float("v.zero || v.y"), 1.0);
        assert_eq!(float("v.zero || v.zero"), 0.0);
        // The right operand is a block with a side effect: it runs only when it can decide.
        let hit = |src: &str| both(src).env.vars.get(var("hit")).map(V::as_f32);
        assert_eq!(hit("v.zero && { v.hit = 1; };"), None);
        assert_eq!(hit("v.x || { v.hit = 1; };"), None);
        assert_eq!(hit("v.x && { v.hit = 5; };"), Some(5.0));
        assert_eq!(hit("v.zero || { v.hit = 6; };"), Some(6.0));
        assert_eq!(hit("v.x && v.y && v.zero && { v.hit = 1; };"), None);
        assert_eq!(hit("v.x && v.y && v.x && { v.hit = 2; };"), Some(2.0));
        assert_eq!(float("v.zero || v.zero || 7"), 1.0);
    }

    #[test]
    fn a_conditional_with_and_without_an_else_branch() {
        assert_eq!(float("v.x ? 10 : 20"), 10.0);
        assert_eq!(float("v.zero ? 10 : 20"), 20.0);
        assert_eq!(float("v.x ? v.y"), -2.0);
        assert_eq!(float("v.zero ? v.y"), 0.0);
        assert_eq!(float("(v.zero ? 1 : 2) * 3"), 6.0);
        // Only the chosen branch runs.
        let run = both("v.x ? (v.a = 1) : (v.b = 1);");
        assert_eq!(run.env.float("a"), 1.0);
        assert!(run.env.vars.get(var("b")).is_none());
        // Nested, right-associative.
        assert_eq!(float("v.zero ? 1 : v.zero ? 2 : 3"), 3.0);
    }

    #[test]
    fn coalescing_uses_the_left_side_when_set_and_the_right_when_missing() {
        assert_eq!(float("v.x ?? 9"), 3.0);
        assert_eq!(float("v.nothing ?? 9"), 9.0);
        assert_eq!(float("v.nothing ?? (v.alsonot ?? 4)"), 4.0);
        assert_eq!(float("(v.nothing ?? v.x) + 1"), 4.0);
        assert_eq!(float("(v.x ?? v.nothing) + 1"), 4.0);
        // Nested handlers: a missing read in the right side of the inner one is the outer one's.
        assert_eq!(float("v.never ?? (v.nothing ?? (v.alsonot ?? 8))"), 8.0);
        // A missing read on the left of `??` logs nothing.
        assert!(both("v.nothing ?? 9").env.messages().is_empty());
    }

    #[test]
    fn a_nan_loop_count_follows_the_architectures_less_equal() {
        use molangx::compile::compile;
        use molangx::numeric::{ARCH, Arch};
        let src =
            "v.n = 0; loop(math.sqrt(-1), { v.n = v.n + 1; v.n > 8 ? {break;} : 0; }); return v.n;";
        let expr = compile(src, &options(13))
            .expr()
            .cloned()
            .expect("compiles");
        let (vm, walker) = (vm_run(&expr, &Env::new()), walker_run(&expr, &Env::new()));
        assert_agree(src, &vm, &walker);
        assert_eq!(walker.f(), if ARCH == Arch::X86_64 { 9.0 } else { 0.0 });
    }

    #[test]
    fn loop_counts_round_up_and_the_first_pass_is_unconditional() {
        for (count, expected) in [
            ("0", 0.0),
            ("-1", 0.0),
            ("1", 1.0),
            ("3", 3.0),
            ("2.2", 3.0),
            ("0.5", 1.0),
            ("v.x", 3.0),
            ("v.y", 0.0),
        ] {
            let run = both(&format!(
                "v.n = 0; loop({count}, {{ v.n = v.n + 1; }}); return v.n;"
            ));
            assert_eq!(run.f(), expected, "loop({count})");
        }
    }

    #[test]
    fn break_and_continue_in_a_loop() {
        assert_eq!(
            float("v.n = 0; loop(10, { v.n = v.n + 1; v.n >= 4 ? { break; }; }); return v.n;"),
            4.0
        );
        assert_eq!(
            float(
                "v.n = 0; v.m = 0; loop(5, { v.n = v.n + 1; v.n < 3 ? { continue; }; v.m = v.m + 1; }); return v.m;"
            ),
            3.0
        );
        assert_eq!(
            float("v.n = 0; loop(3, { loop(3, { v.n = v.n + 1; break; }); }); return v.n;"),
            3.0
        );
        assert_eq!(
            float("v.n = 0; loop(3, { v.n = v.n + 1; continue; v.n = 100; }); return v.n;"),
            3.0
        );
    }

    #[test]
    fn break_and_continue_outside_a_loop_end_the_program_with_zero() {
        for src in [
            "v.a = 1; v.x ? { break; }; v.a = 2; return 5;",
            "v.a = 1; v.x ? { continue; }; v.a = 2; return 5;",
        ] {
            let run = both(src);
            assert_eq!(run.f(), 0.0, "{src}");
            assert_eq!(run.env.float("a"), 1.0, "{src}");
        }
        // Not taken: the program runs on.
        assert_eq!(
            both("v.a = 1; v.zero ? { break; }; v.a = 2; return 5;").f(),
            5.0
        );
    }

    #[test]
    fn the_loop_guard_leaves_a_long_loop_with_one_message() {
        let mut start = Env::new();
        start.limits.loop_iterations = Some(4);
        let run = both_on(
            "v.n = 0; loop(100, { v.n = v.n + 1; }); loop(100, { v.n = v.n + 1; }); return v.n;",
            &start,
        );
        assert_eq!(run.f(), 8.0);
        assert_eq!(
            run.env.messages(),
            ["molangx: loop stopped after its budget of 4 iterations"]
        );
        // A loop that ends within its budget says nothing.
        let run = both_on("v.n = 0; loop(4, { v.n = v.n + 1; }); return v.n;", &start);
        assert_eq!(run.f(), 4.0);
        assert!(run.env.messages().is_empty());
    }

    #[test]
    fn for_each_visits_the_live_actors_of_an_array() {
        // The array is [1, 2, 150]: 150 is dead and skipped.
        let run = both("v.n = 0; for_each(t.e, c.arr, { v.n = v.n + 1; }); return v.n;");
        assert_eq!(run.f(), 2.0);
        let run =
            both("v.n = 0; for_each(v.e, c.arr, { v.n = v.n + 1; v.last = v.e; }); return v.n;");
        assert_eq!(run.f(), 2.0);
        assert_eq!(run.env.vars.get(var("last")), Some(&Value::Actor(2)));
        assert_eq!(
            float("v.n = 0; for_each(t.e, c.arr, { v.n = v.n + 1; break; }); return v.n;"),
            1.0
        );
        assert_eq!(
            float("v.n = 0; for_each(t.e, c.arr, { continue; v.n = 100; }); return v.n;"),
            0.0
        );
        // Not an actor array, or an empty one: the body never runs.
        assert_eq!(
            float("v.n = 0; for_each(t.e, v.x, { v.n = 1; }); return v.n;"),
            0.0
        );
        assert_eq!(
            float("v.n = 0; for_each(t.e, c.n, { v.n = 1; }); return v.n;"),
            0.0
        );
    }

    #[test]
    fn for_each_is_bounded_by_the_loop_guard() {
        let mut start = Env::new();
        start.limits.loop_iterations = Some(1);
        let run = both_on(
            "v.n = 0; for_each(t.e, c.arr, { v.n = v.n + 1; }); return v.n;",
            &start,
        );
        assert_eq!(run.f(), 1.0);
        assert_eq!(
            run.env.messages(),
            ["molangx: loop stopped after its budget of 1 iterations"]
        );
    }

    #[test]
    fn an_arrow_reads_the_public_snapshot_of_the_target() {
        let mut env = Env::new();
        env.vars.set_public(var("pub"), Value::Float(9.0));
        env.vars.refresh_snapshots();
        env.vars.set_public(var("pub"), Value::Float(10.0));
        // Inside `->` an entity variable is the public snapshot (9), and an unknown one reads as 0
        // without a message.
        let run = both_on("c.other -> v.pub", &env);
        assert_eq!(run.f(), 9.0);
        let run = both_on("c.other -> v.nothing", &env);
        assert_eq!(run.f(), 0.0);
        assert!(run.env.messages().is_empty());
        // `this` is the target's.
        assert_eq!(both("c.other -> q.sum_test(this)").f(), 12.0);
        assert_eq!(float("this"), 2.5);
    }

    #[test]
    fn an_arrow_to_a_dead_or_non_actor_target_is_zero_and_skips_the_right_side() {
        let start = Env::new().with_var("dead", Value::Actor(500));
        for src in [
            "v.dead -> q.sum_test(v.hit = 1);",
            "v.x -> q.sum_test(v.hit = 1);",
            "(v.nothing ?? 0) -> q.sum_test(v.hit = 1);",
        ] {
            let run = both_on(src, &start);
            assert_eq!(run.f(), 0.0, "{src}");
            assert!(run.env.vars.get(var("hit")).is_none(), "{src}");
        }
    }

    #[test]
    fn a_missing_read_inside_an_arrow_names_the_public_access() {
        // A temp read inside `->` is not a public read, but the message still uses the longer
        // wording: it follows the `->` depth.
        let run = both("c.other -> q.sum_test(t.nothing)");
        assert_eq!(run.f(), 0.0);
        assert_eq!(run.env.messages().len(), 1);
        assert!(
            run.env.messages()[0].starts_with("Error: unhandled request for unknown variable 'temp.nothing' - are you trying to access a variable from a different mob"),
            "{:?}",
            run.env.messages()
        );
    }

    #[test]
    fn jump_is_a_halt_without_a_loop_and_break_or_continue_inside_one() {
        let mut walker = walker_for("v.x");
        let mut env = Env::new();
        let mut cx = env.cx();
        assert!(matches!(
            walker.jump(Jump::Break, &mut cx),
            Err(Unwind::Halt)
        ));
        assert!(matches!(
            walker.jump(Jump::Continue, &mut cx),
            Err(Unwind::Halt)
        ));
        walker.scope.in_loop = true;
        assert!(matches!(
            walker.jump(Jump::Break, &mut cx),
            Err(Unwind::Break)
        ));
        assert!(matches!(
            walker.jump(Jump::Continue, &mut cx),
            Err(Unwind::Continue)
        ));
        assert_eq!(walker.steps, 4, "a jump is one step");
    }

    #[test]
    fn counted_loop_runs_ceil_of_the_count_iterations_and_restores_the_scope() {
        let expr = build("v.n = 0; loop(2.5, { v.n = v.n + 1; });");
        let mut walker = Walker::<World>::new(&expr);
        let mut env = Env::new();
        let _ = walker.run(tree(&expr).expect("a tree"), &mut env.cx());
        assert_eq!(env.float("n"), 3.0);
        assert!(!walker.scope.in_loop);
        assert!(!walker.scope.in_handler);
    }

    #[test]
    fn a_loop_left_by_an_unwind_restores_the_scope() {
        // A `return` inside a loop inside a `??` left side: the scope is restored after the run.
        let expr = build("loop(3, { (v.x ?? 0) > 1 ? { return 5; }; });");
        let mut walker = Walker::<World>::new(&expr);
        let mut env = Env::new();
        let value = walker.run(tree(&expr).expect("a tree"), &mut env.cx());
        assert_eq!(value.as_f32(), 5.0);
        assert!(!walker.scope.in_loop && !walker.scope.in_handler);
    }

    #[test]
    fn a_coalesce_with_an_arrow_in_its_left_side_unwinds_the_arrows() {
        let tree = node(
            Op::NullCoalescing,
            vec![arrow_to_a_missing_temp(), number(1.0)],
        );
        let mut walker = walker_for("v.x");
        let mut env = Env::new();
        env.subject = Some(7);
        let mut cx = env.cx();
        let before = cx.subjects;
        assert_eq!(walk(&mut walker, &tree, &mut cx), Some(Value::Float(1.0)));
        assert!(walker.arrows.is_empty());
        assert_eq!(cx.subjects, before);
        assert!(!walker.scope.in_handler && !walker.scope.in_loop);
        assert!(
            env.messages().is_empty(),
            "inside a handler the miss is silent"
        );
    }

    #[test]
    fn a_coalescing_handler_restores_the_subjects_of_an_arrow() {
        // The right side runs on the caller's subjects again: `this` is 2.5, not the target's 12.
        let tree = node(
            Op::NullCoalescing,
            vec![arrow_to_a_missing_temp(), node(Op::This, vec![])],
        );
        let mut walker = walker_for("v.x");
        let mut env = Env::new();
        env.subject = Some(7);
        let mut cx = env.cx();
        assert_eq!(walk(&mut walker, &tree, &mut cx), Some(Value::Float(2.5)));
        // A left side that is fine leaves the arrow itself, normally.
        let fine = node(
            Op::NullCoalescing,
            vec![
                node(
                    Op::Pointer,
                    vec![
                        read(Op::ContextVariable, "context.other"),
                        node(Op::This, vec![]),
                    ],
                ),
                number(1.0),
            ],
        );
        assert_eq!(walk(&mut walker, &fine, &mut cx), Some(Value::Float(12.0)));
        assert!(walker.arrows.is_empty());
        assert_eq!(cx.subjects.actor, Some(7));
    }

    #[test]
    fn a_return_unwinds_through_nested_loops() {
        let src = "v.n = 0; loop(3, { for_each(t.e, c.arr, { v.n = v.n + 1; v.n > 1 ? { return v.n * 10; }; }); }); return 99;";
        let run = both(src);
        assert_eq!(run.f(), 20.0);
        assert_eq!(run.env.float("n"), 2.0);
    }

    #[test]
    fn a_missing_read_inside_a_loop_ends_the_whole_evaluation() {
        let run =
            both("v.n = 0; loop(3, { v.n = v.n + 1; v.nothing + 1; }); v.after = 1; return 7;");
        assert_eq!(run.f(), 0.0);
        assert_eq!(run.env.float("n"), 1.0);
        assert!(run.env.vars.get(var("after")).is_none());
        assert_eq!(
            run.env.messages(),
            ["Error: unhandled request for unknown variable 'variable.nothing'"]
        );
    }

    #[test]
    fn a_handler_inside_a_loop_catches_each_iteration_separately() {
        let run = both("v.n = 0; loop(3, { v.n = v.n + (v.nothing ?? 2); }); return v.n;");
        assert_eq!(run.f(), 6.0);
        assert!(run.env.messages().is_empty());
    }

    #[test]
    fn the_loop_guard_speaks_once_for_all_the_loops_of_an_evaluation() {
        let mut start = Env::new();
        start.limits.loop_iterations = Some(2);
        let src = "v.n = 0; loop(9, { v.n = v.n + 1; }); for_each(t.e, c.arr, { v.n = v.n + 10; }); loop(9, { v.n = v.n + 100; }); return v.n;";
        let run = both_on(src, &start);
        assert_eq!(run.f(), 2.0 + 20.0 + 200.0);
        assert_eq!(
            run.env.messages(),
            ["molangx: loop stopped after its budget of 2 iterations"]
        );
        // A for_each whose array ends exactly at the budget does not speak.
        let run = both_on(
            "v.n = 0; for_each(t.e, c.arr, { v.n = v.n + 1; }); return v.n;",
            &start,
        );
        assert_eq!(run.f(), 2.0);
        assert!(run.env.messages().is_empty());
    }

    #[test]
    fn an_unlimited_loop_budget_runs_to_the_count() {
        let mut start = Env::new();
        start.limits.loop_iterations = None;
        let run = both_on(
            "v.n = 0; loop(300, { v.n = v.n + 1; }); return v.n;",
            &start,
        );
        assert_eq!(run.f(), 300.0);
        assert!(run.env.messages().is_empty());
        start.limits.loop_iterations = Some(0);
        let run = both_on(
            "v.n = 0; loop(300, { v.n = v.n + 1; }); return v.n;",
            &start,
        );
        assert_eq!(run.f(), 1.0, "a zero budget still runs the first pass");
        assert_eq!(
            run.env.messages(),
            ["molangx: loop stopped after its budget of 0 iterations"]
        );
    }
}
