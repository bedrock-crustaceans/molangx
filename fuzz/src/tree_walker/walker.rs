//! The walk: the step budget, the handlers, the temps and the dispatch on a node's op.

use super::flow::{Jump, Logic};
use super::{Eval, Scope, Unwind, Walker};
use molangx::compile::Expr;
use molangx::internals::{Node, Payload, storable};
use molangx::numeric::PostOp;
use molangx::ops::ExpressionOp as Op;
use molangx::vm::{EvalCx, Host, RuntimeMsg, TempMap, TempName, Temps, Value, VariableName};

impl<H: Host> Walker<H> {
    pub(super) fn new(expr: &Expr) -> Self {
        Self {
            version: expr.version(),
            catalog: expr.catalog().clone(),
            math: expr.math().cloned(),
            signed_division: expr.version().signed_division_fix(),
            steps: 0,
            stopped: false,
            loop_guard_logged: false,
            arg_depth: 0,
            arrows: Vec::new(),
            temps: TempMap::new(),
            scope: Scope::default(),
        }
    }

    /// Runs one (sub-)program — the whole expression, or one query argument — to its value: its
    /// root's value after the `End`, or a `return`'s, or 0 when it ended otherwise. Whatever `->`
    /// it left entered is left on the way out.
    pub(super) fn run(&mut self, root: &Node, cx: &mut EvalCx<'_, '_, H>) -> Value<H> {
        if self.stopped {
            return Value::ZERO;
        }
        let depth = self.arrows.len();
        let walked: Eval<H> = self.within(Scope::default(), |walker| {
            let value = walker.node(root, cx)?;
            walker.step(cx)?;
            Ok(value)
        });
        let value = match walked {
            Ok(value) | Err(Unwind::Return(value)) => value,
            Err(
                Unwind::Missing | Unwind::Halt | Unwind::Stop | Unwind::Break | Unwind::Continue,
            ) => Value::ZERO,
        };
        self.leave_arrows(cx, depth);
        value
    }

    /// Runs `f` in `scope`, then restores the scope around it.
    pub(super) fn within<T>(&mut self, scope: Scope, f: impl FnOnce(&mut Self) -> T) -> T {
        let outer = std::mem::replace(&mut self.scope, scope);
        let result = f(self);
        self.scope = outer;
        result
    }

    /// Charges one step (one instruction of the lowering).
    pub(super) fn step(&mut self, cx: &mut EvalCx<'_, '_, H>) -> Result<(), Unwind<H>> {
        self.charge(cx, 1)
    }

    /// Charges `cost` steps: one per instruction, or a size-proportional cost on top of one. The
    /// evaluation ends when they pass the budget.
    pub(super) fn charge(
        &mut self,
        cx: &mut EvalCx<'_, '_, H>,
        cost: u64,
    ) -> Result<(), Unwind<H>> {
        if cost == 0 {
            return Ok(());
        }
        self.steps = self.steps.saturating_add(cost);
        if let Some(limit) = cx.limits.total_steps
            && self.steps > limit
        {
            self.stopped = true;
            cx.sink.runtime(RuntimeMsg::StepLimit { limit });
            return Err(Unwind::Stop);
        }
        Ok(())
    }

    /// The loop guard left a loop: one message per evaluation, for the first loop left.
    pub(super) fn loop_guard(&mut self, cx: &mut EvalCx<'_, '_, H>, limit: u32) {
        if !self.loop_guard_logged {
            self.loop_guard_logged = true;
            cx.sink.runtime(RuntimeMsg::LoopLimit { limit });
        }
    }

    /// Restores the subjects of the `->`s entered beyond `depth`.
    pub(super) fn leave_arrows(&mut self, cx: &mut EvalCx<'_, '_, H>, depth: usize) {
        if let Some(&caller) = self.arrows.get(depth) {
            cx.subjects = caller;
            self.arrows.truncate(depth);
        }
    }

    /// The missing-variable path: a `??` of this (sub-)program catches it
    /// silently; otherwise the read is logged and the (sub-)program ends.
    pub(super) fn missing(&self, cx: &mut EvalCx<'_, '_, H>, name: &str) -> Unwind<H> {
        if !self.scope.in_handler {
            cx.sink.runtime(RuntimeMsg::UnknownVariable {
                name,
                public_access: !self.arrows.is_empty(),
            });
        }
        Unwind::Missing
    }

    /// The value in `slot` (a `variable.` name: the subject actor's latest).
    pub(super) fn load(&self, cx: &EvalCx<'_, '_, H>, slot: Slot) -> Option<Value<H>> {
        match slot {
            Slot::Variable(key) => cx.variable(key).cloned(),
            Slot::Temp(key) => match &cx.temps {
                Temps::Kept(map) => map.get(key).cloned(),
                Temps::PerEvaluation => self.temps.get(key).cloned(),
            },
        }
    }

    /// Writes `slot` as an assignment does: an actor made storable.
    pub(super) fn store(&mut self, cx: &mut EvalCx<'_, '_, H>, slot: Slot, value: Value<H>) {
        match slot {
            Slot::Variable(key) => cx.set_variable(key, value),
            Slot::Temp(key) => {
                let value = storable(cx, value);
                match &mut cx.temps {
                    Temps::Kept(map) => map.set(key, value),
                    Temps::PerEvaluation => self.temps.set(key, value),
                };
            }
        }
    }

    /// The value of a node. On return the node's own post-op has been applied.
    ///
    /// A thin dispatcher: each kind of node has its own small method, so the recursion through a
    /// 256-level tree keeps small stack frames even in a debug build.
    pub(super) fn node(&mut self, n: &Node, cx: &mut EvalCx<'_, '_, H>) -> Eval<H> {
        let op = n.op();
        match op {
            Op::Float
            | Op::StringLiteral
            | Op::Geometry
            | Op::Material
            | Op::Texture
            | Op::This => self.leaf(op, n, cx),
            Op::EntityVariable | Op::TempVariable | Op::ContextVariable => self.read(n, cx),
            Op::MemberAccessor => self.member(n, cx),
            Op::QueryFunction => self.query(n, cx),
            Op::LogicalAnd => self.logic(Logic::And, n, cx),
            Op::LogicalOr => self.logic(Logic::Or, n, cx),
            Op::NullCoalescing => self.coalesce(n, cx),
            Op::Conditional => self.conditional(n, cx),
            Op::Semicolon => self.statements(n, cx),
            Op::Loop => self.counted_loop(n, cx),
            Op::ForEach => self.for_each(n, cx),
            Op::Break => self.jump(Jump::Break, cx),
            Op::Continue => self.jump(Jump::Continue, cx),
            Op::Assignment => self.assignment(n, cx),
            Op::Pointer => self.pointer(n, cx),
            Op::LeftBracket | Op::LeftParenthesis => self.node(child(n, 0), cx),
            // Never in a tree that linked (the link stops at them).
            Op::ArrayVariable
            | Op::Array
            | Op::ExpressionArray
            | Op::GeometryVariable
            | Op::MaterialVariable
            | Op::TextureVariable => Ok(Value::ZERO),
            // The operators, the math functions and the generic constant path.
            op => self.arithmetic(op, n, cx),
        }
    }
}

/// A variable or temp: what a read, an assignment, a member store or `for_each` names.
#[derive(Copy, Clone)]
pub(super) enum Slot {
    Variable(VariableName),
    Temp(TempName),
}

impl Slot {
    /// The slot a read or write node names; `None` for any other payload.
    pub(super) fn of(payload: &Payload) -> Option<Self> {
        match payload {
            Payload::Entity(name) => Some(Self::Variable(VariableName::from_raw_hash(name.hash()))),
            Payload::Temp(name) => Some(Self::Temp(TempName::from_raw_hash(name.hash()))),
            _ => None,
        }
    }
}

/// `value` with a node's post-op: the value itself for the plain form, else the float `x·S + O`
/// (a non-float is read as its float).
pub(super) fn with_post<H: Host>(value: Value<H>, post: PostOp) -> Value<H> {
    if post.is_identity() {
        value
    } else {
        Value::Float(post.apply(value.as_f32()))
    }
}

/// Child `i` of a node; a compiled tree always has the children its op needs, so a missing one
/// reads as a constant 0 node.
pub(super) fn child(n: &Node, i: usize) -> &Node {
    static ZERO: std::sync::OnceLock<Node> = std::sync::OnceLock::new();
    n.children().get(i).unwrap_or_else(|| {
        ZERO.get_or_init(|| Node::new(Op::Float, Payload::Float(0.0), PostOp::IDENTITY, vec![]))
    })
}

/// The constant operand moved into a node's value: a float, or a string hash read as the float
/// whose bits are its low 32 bits.
pub(super) fn moved_constant(n: &Node) -> f32 {
    match *n.value() {
        Payload::Float(v) => v,
        Payload::Hash(h) => f32::from_bits(h as u32),
        _ => 0.0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tree_walker::test_support::*;
    use molangx::vm::Subjects;

    #[test]
    fn walker_new_takes_the_version_of_the_expression() {
        let old = Walker::<World>::new(&build_at("v.x", 6));
        let new = Walker::<World>::new(&build_at("v.x", 7));
        assert!(!old.signed_division);
        assert!(new.signed_division);
        assert_eq!(
            (old.steps, old.stopped, old.loop_guard_logged, old.arg_depth),
            (0, false, false, 0)
        );
        assert!(new.arrows.is_empty() && new.temps.is_empty());
        assert!(!new.scope.in_handler && !new.scope.in_loop);
    }

    #[test]
    fn step_stops_after_the_budget_and_says_so_once() {
        let mut walker = walker_for("v.x");
        let mut env = Env::new().with_steps(2);
        let mut cx = env.cx();
        assert!(walker.step(&mut cx).is_ok());
        assert!(walker.step(&mut cx).is_ok());
        assert!(!walker.stopped);
        assert!(is_stop(&walker.step(&mut cx)));
        assert!(walker.stopped);
        assert_eq!(walker.steps, 3);
        assert_eq!(
            env.messages(),
            ["molangx: evaluation stopped after its budget of 2 steps"]
        );
    }

    #[test]
    fn run_on_a_stopped_walker_returns_zero_without_charging() {
        let expr = build("v.x");
        let mut walker = Walker::<World>::new(&expr);
        walker.stopped = true;
        let mut env = Env::new();
        let tree = tree(&expr).expect("a tree");
        assert_eq!(walker.run(tree, &mut env.cx()), Value::ZERO);
        assert_eq!(walker.steps, 0);
    }

    #[test]
    fn run_restores_the_scope_and_the_subjects_it_found() {
        let expr = build("v.x");
        let mut walker = Walker::<World>::new(&expr);
        walker.scope = Scope {
            in_handler: true,
            in_loop: true,
        };
        let mut env = Env::new();
        let tree = tree(&expr).expect("a tree");
        assert_eq!(walker.run(tree, &mut env.cx()), Value::Float(3.0));
        assert!(walker.scope.in_handler && walker.scope.in_loop);
    }

    #[test]
    fn run_of_a_return_gives_its_value_and_of_a_jump_without_a_loop_gives_zero() {
        for (src, expected) in [
            ("v.a = 1; return 4;", 4.0),
            ("v.a = 1; v.x ? { break; }; return 4;", 0.0),
            ("v.a = 1; v.x ? { continue; }; return 4;", 0.0),
            ("v.nothing + 1", 0.0),
        ] {
            let expr = build(src);
            let mut walker = Walker::<World>::new(&expr);
            let mut env = Env::new();
            assert_eq!(
                walker
                    .run(tree(&expr).expect("a tree"), &mut env.cx())
                    .as_f32(),
                expected,
                "{src}"
            );
        }
    }

    #[test]
    fn charge_adds_a_size_proportional_cost() {
        let mut walker = walker_for("v.x");
        let mut env = Env::new().with_steps(10);
        let mut cx = env.cx();
        assert!(walker.charge(&mut cx, 0).is_ok());
        assert_eq!(walker.steps, 0);
        assert!(walker.charge(&mut cx, 10).is_ok());
        assert_eq!(walker.steps, 10, "exactly the budget is allowed");
        assert!(!walker.stopped);
        assert!(is_stop(&walker.charge(&mut cx, 1)));
        assert!(walker.stopped);
        assert_eq!(
            env.messages(),
            ["molangx: evaluation stopped after its budget of 10 steps"]
        );
    }

    #[test]
    fn charge_saturates_instead_of_wrapping() {
        let mut walker = walker_for("v.x");
        walker.steps = 5;
        let mut env = Env::new().with_steps(10);
        assert!(is_stop(&walker.charge(&mut env.cx(), u64::MAX)));
        assert_eq!(walker.steps, u64::MAX);
    }

    #[test]
    fn a_zero_cost_charge_never_stops_even_past_the_budget() {
        let mut walker = walker_for("v.x");
        walker.steps = 50;
        let mut env = Env::new().with_steps(10);
        assert!(walker.charge(&mut env.cx(), 0).is_ok());
        assert!(!walker.stopped);
        assert!(env.messages().is_empty());
    }

    #[test]
    fn loop_guard_logs_once_per_evaluation() {
        let mut walker = walker_for("v.x");
        let mut env = Env::new();
        let mut cx = env.cx();
        walker.loop_guard(&mut cx, 7);
        walker.loop_guard(&mut cx, 7);
        walker.loop_guard(&mut cx, 9);
        assert!(walker.loop_guard_logged);
        assert_eq!(
            env.messages(),
            ["molangx: loop stopped after its budget of 7 iterations"]
        );
    }

    #[test]
    fn leave_arrows_restores_the_outermost_caller() {
        let mut walker = walker_for("v.x");
        let mut env = Env::new();
        let mut cx = env.cx();
        let outer = Subjects {
            this: 1.0,
            ..Subjects::actor(1)
        };
        let middle = Subjects {
            this: 2.0,
            ..Subjects::actor(2)
        };
        let inner = Subjects {
            this: 3.0,
            ..Subjects::actor(3)
        };
        walker.arrows = vec![outer, middle];
        cx.subjects = inner;
        walker.leave_arrows(&mut cx, 0);
        assert!(walker.arrows.is_empty());
        assert_eq!(cx.subjects, outer);
        // Down to a depth: one entry stays, the subjects are the caller of the popped one.
        walker.arrows = vec![outer, middle];
        cx.subjects = inner;
        walker.leave_arrows(&mut cx, 1);
        assert_eq!(walker.arrows.len(), 1);
        assert_eq!(cx.subjects, middle);
        // Already at or below the depth: nothing changes.
        walker.leave_arrows(&mut cx, 1);
        walker.leave_arrows(&mut cx, 5);
        assert_eq!(walker.arrows.len(), 1);
        assert_eq!(cx.subjects, middle);
    }

    #[test]
    fn missing_is_silent_only_inside_a_handler() {
        let mut walker = walker_for("v.x");
        let mut env = Env::new();
        let mut cx = env.cx();
        walker.scope.in_handler = true;
        assert!(matches!(walker.missing(&mut cx, "gone"), Unwind::Missing));
        walker.scope.in_handler = true;
        walker.arrows.push(Subjects::none());
        assert!(matches!(walker.missing(&mut cx, "gone"), Unwind::Missing));
        assert!(env.messages().is_empty());
    }

    #[test]
    fn missing_outside_a_handler_logs_with_the_access_flavour() {
        let mut walker = walker_for("v.x");
        let mut env = Env::new();
        let mut cx = env.cx();
        assert!(matches!(walker.missing(&mut cx, "gone"), Unwind::Missing));
        walker.arrows.push(Subjects::none());
        assert!(matches!(walker.missing(&mut cx, "gone2"), Unwind::Missing));
        walker.scope.in_handler = true;
        assert!(matches!(walker.missing(&mut cx, "quiet"), Unwind::Missing));
        let messages = env.messages();
        assert_eq!(messages.len(), 2);
        assert_eq!(
            messages[0],
            "Error: unhandled request for unknown variable 'gone'"
        );
        assert!(
            messages[1].starts_with(
                "Error: unhandled request for unknown variable 'gone2' - are you trying"
            )
        );
    }

    #[test]
    fn post_applies_only_a_non_identity_post_op() {
        let post = with_post::<World>;
        assert_eq!(
            post(Value::string("a"), PostOp::IDENTITY),
            Value::string("a")
        );
        assert_eq!(post(Value::Actor(3), PostOp::IDENTITY), Value::Actor(3));
        assert_eq!(
            post(Value::Float(4.0), PostOp::new(2.0, 1.0)),
            Value::Float(9.0)
        );
        // A non-float is read as its float first.
        let hash = Value::<World>::string("a");
        let expected = PostOp::new(2.0, 1.0).apply(hash.as_f32());
        assert_eq!(
            bits(post(hash, PostOp::new(2.0, 1.0)).as_f32()),
            bits(expected)
        );
    }

    #[test]
    fn temps_live_in_the_walker_unless_the_host_keeps_a_map() {
        let mut walker = walker_for("v.x");
        let mut env = Env::new();
        let key = temp_key("t");
        {
            let mut cx = env.cx();
            assert_eq!(walker.load(&cx, Slot::Temp(key)), None);
            walker.store(&mut cx, Slot::Temp(key), Value::Float(5.0));
            assert_eq!(walker.load(&cx, Slot::Temp(key)), Some(Value::Float(5.0)));
        }
        assert_eq!(walker.temps.get(key), Some(&Value::Float(5.0)));
        let mut hosted = Env::new();
        hosted.temps = Some(TempMap::new());
        let mut other = walker_for("v.x");
        {
            let mut cx = hosted.cx();
            other.store(&mut cx, Slot::Temp(key), Value::Float(6.0));
            assert_eq!(other.load(&cx, Slot::Temp(key)), Some(Value::Float(6.0)));
        }
        assert!(other.temps.is_empty(), "the host map is used instead");
        assert_eq!(
            hosted.temps.as_ref().and_then(|t| t.get(key)),
            Some(&Value::Float(6.0))
        );
    }

    #[test]
    fn set_temp_makes_an_actor_storable() {
        let mut walker = walker_for("v.x");
        let mut env = Env::new();
        let mut cx = env.cx();
        let key = temp_key("who");
        walker.store(&mut cx, Slot::Temp(key), Value::actor_array([1, 150]));
        assert_eq!(
            walker.load(&cx, Slot::Temp(key)),
            Some(Value::actor_array([1]))
        );
    }

    #[test]
    fn child_falls_back_to_a_zero_constant() {
        let leaf = number(7.0);
        let missing = child(&leaf, 0);
        assert!(missing.is(Op::Float));
        assert_eq!(missing.float(), 0.0);
        let two = node(Op::Add, vec![number(1.0), number(2.0)]);
        assert_eq!(child(&two, 1).float(), 2.0);
        assert_eq!(child(&two, 0).float(), 1.0);
        assert_eq!(child(&two, 2).float(), 0.0);
    }

    #[test]
    fn moved_constant_reads_a_float_or_the_low_bits_of_a_hash() {
        assert_eq!(moved_constant(&number(1.5)), 1.5);
        let hash = leaf(Op::StringLiteral, Payload::Hash(0xdead_beef_3fc0_0000));
        assert_eq!(moved_constant(&hash), 1.5);
        assert_eq!(moved_constant(&node(Op::Add, vec![])), 0.0);
    }

    #[test]
    fn an_op_node_without_children_still_evaluates_through_the_generic_constant_path() {
        // `Pi` takes the generic path of the dispatcher: a step and the node's value with its
        // post-op.
        let expr = build("math.pi * 2");
        let run = walker_run(&expr, &Env::new());
        assert_eq!(run.f(), std::f32::consts::PI * 2.0);
    }

    #[test]
    fn an_unwind_out_of_an_arrow_leaves_the_subjects_switched_for_whoever_catches_it() {
        let tree = arrow_to_a_missing_temp();
        let mut walker = walker_for("v.x");
        let mut env = Env::new();
        env.subject = Some(7);
        let mut cx = env.cx();
        assert_eq!(walk(&mut walker, &tree, &mut cx), None);
        assert_eq!(
            walker.arrows.len(),
            1,
            "the caller's subjects are still saved"
        );
        assert_eq!(
            cx.subjects.actor,
            Some(2),
            "and the target's are still switched in"
        );
        walker.leave_arrows(&mut cx, 0);
        assert_eq!(cx.subjects.actor, Some(7));
        assert_eq!(env.messages().len(), 1);
        assert!(
            env.messages()[0].contains("'temp.nothing' - are you trying to access"),
            "{:?}",
            env.messages()
        );
    }

    #[test]
    fn run_leaves_the_arrows_an_unwind_left_entered() {
        let tree = arrow_to_a_missing_temp();
        let mut walker = walker_for("v.x");
        let mut env = Env::new();
        env.subject = Some(7);
        let mut cx = env.cx();
        let before = cx.subjects;
        assert_eq!(walker.run(&tree, &mut cx), Value::ZERO);
        assert!(walker.arrows.is_empty());
        assert_eq!(cx.subjects, before);
    }

    #[test]
    fn array_and_resource_variable_nodes_are_zero_and_free() {
        let mut walker = walker_for("v.x");
        let mut env = Env::new();
        for op in [
            Op::ArrayVariable,
            Op::Array,
            Op::ExpressionArray,
            Op::GeometryVariable,
            Op::MaterialVariable,
            Op::TextureVariable,
        ] {
            assert_eq!(
                walk(&mut walker, &node(op, vec![]), &mut env.cx()),
                Some(Value::ZERO),
                "{op:?}"
            );
        }
        assert_eq!(walker.steps, 0);
    }

    #[test]
    fn a_generic_op_takes_the_constant_path_with_its_post_op() {
        // `Pi` is no math function of the tables: one step and the node's value with its post-op.
        let mut walker = walker_for("v.x");
        let mut env = Env::new();
        let pi = Node::new(
            Op::Pi,
            Payload::Float(std::f32::consts::PI),
            PostOp::new(2.0, 1.0),
            vec![],
        );
        let got = walk(&mut walker, &pi, &mut env.cx());
        assert_eq!(
            got.map(|v| v.as_f32()),
            Some(std::f32::consts::PI * 2.0 + 1.0)
        );
        assert_eq!(walker.steps, 1);
    }
}
