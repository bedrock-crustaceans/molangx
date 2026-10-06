//! Leaves, member paths, variable reads and assignments.

use super::walker::{Slot, child, moved_constant, with_post};
use super::{Eval, Unwind, Walker};
use molangx::hash::HashedStr;
use molangx::internals::{
    Name, Node, Payload, member_store_check, public_variable, storable, store_cost,
};
use molangx::ops::ExpressionOp as Op;
use molangx::vm::{ContextName, EvalCx, Host, ResourceRef, RuntimeMsg, Value};

impl<H: Host> Walker<H> {
    /// A literal, a resource or `this`.
    #[inline(never)]
    pub(super) fn leaf(&mut self, op: Op, n: &Node, cx: &mut EvalCx<'_, '_, H>) -> Eval<H> {
        self.step(cx)?;
        Ok(match op {
            // A literal is loaded without a post-op (shared with the VM).
            Op::Float => Value::Float(n.float()),
            Op::StringLiteral => Value::Hash(HashedStr::from_u64(match *n.value() {
                Payload::Hash(hash) => hash,
                _ => 0,
            })),
            Op::This => Value::Float(n.post().apply(cx.subjects.this)),
            _ => {
                let hash = match n.value() {
                    Payload::Geometry(name) | Payload::Material(name) | Payload::Texture(name) => {
                        name.hash()
                    }
                    _ => HashedStr::from_u64(0),
                };
                Value::Resource(ResourceRef::from_raw_hash(hash))
            }
        })
    }

    /// `base.name`: the outermost accessor of a read path.
    #[inline(never)]
    pub(super) fn member(&mut self, n: &Node, cx: &mut EvalCx<'_, '_, H>) -> Eval<H> {
        let Payload::Member(last) = n.value() else {
            return Ok(Value::ZERO);
        };
        self.member_of_path(n, last, cx)
    }

    /// One accessor of a read path whose last member is `last`. A missing member logs the member
    /// message naming the path's **last** member, then takes the missing-variable path with that
    /// same name.
    fn member_of_path(&mut self, n: &Node, last: &Name, cx: &mut EvalCx<'_, '_, H>) -> Eval<H> {
        let inner = child(n, 0);
        let base = if inner.is(Op::MemberAccessor) {
            self.member_of_path(inner, last, cx)?
        } else {
            self.node(inner, cx)?
        };
        self.step(cx)?;
        let Payload::Member(name) = n.value() else {
            return Ok(Value::ZERO);
        };
        let Some(value) = base.member(name.hash()) else {
            let text = format!(".{}", last.as_str());
            cx.sink.runtime(RuntimeMsg::MissingMember { name: &text });
            return Err(self.missing(cx, &text));
        };
        Ok(with_post(value.clone(), n.post()))
    }

    #[inline(never)]
    pub(super) fn read(&mut self, n: &Node, cx: &mut EvalCx<'_, '_, H>) -> Eval<H> {
        self.step(cx)?;
        let (Payload::Entity(name) | Payload::Temp(name) | Payload::Context(name)) = n.value()
        else {
            return Ok(Value::ZERO);
        };
        let found = match Slot::of(n.value()) {
            // Inside `->` an entity variable is the public snapshot, and never aborts.
            Some(Slot::Variable(key)) if !self.arrows.is_empty() => {
                Some(public_variable(cx, key).cloned().unwrap_or(Value::ZERO))
            }
            Some(slot) => self.load(cx, slot),
            None => cx.context(ContextName::from_raw_hash(name.hash())),
        };
        match found {
            Some(value) => Ok(with_post(value, n.post())),
            None => Err(self.missing(cx, name.as_str())),
        }
    }

    /// An assignment: the value is written as it is (an actor made storable), and the
    /// expression's value is that value with the node's post-op.
    #[inline(never)]
    pub(super) fn assignment(&mut self, n: &Node, cx: &mut EvalCx<'_, '_, H>) -> Eval<H> {
        let target = child(n, 0);
        let value = if let Some(value) = n.children().get(1) {
            self.node(value, cx)?
        } else {
            // A constant right side moved into the node.
            self.step(cx)?;
            Value::Float(moved_constant(n))
        };
        self.step(cx)?;
        match Slot::of(target.value()) {
            Some(slot) => {
                self.charge(cx, store_cost(&value))?;
                self.store(cx, slot, value.clone());
            }
            None if target.is(Op::MemberAccessor) => {
                self.store_member(target, value.clone(), cx)?;
            }
            // Any other target fails the link; nothing reaches here.
            None => {}
        }
        Ok(with_post(value, n.post()))
    }

    /// `v.a.b.c = x` / `t.a.b = x`: the root variable's value (0 when unset)
    /// with the member path written, intermediate structs created; any other root writes nothing
    /// and costs nothing. See [`Walker::write_member`] for the budgets.
    #[inline(never)]
    fn store_member(
        &mut self,
        target: &Node,
        value: Value<H>,
        cx: &mut EvalCx<'_, '_, H>,
    ) -> Result<(), Unwind<H>> {
        let mut path = Vec::new();
        let mut node = target;
        while node.is(Op::MemberAccessor) {
            if let Payload::Member(name) = node.value() {
                path.push(name.hash());
            }
            node = child(node, 0);
        }
        path.reverse();
        if let Some(slot) = Slot::of(node.value()) {
            let whole = self.load(cx, slot).unwrap_or_default();
            let whole = self.write_member(whole, &path, value, cx)?;
            self.store(cx, slot, whole);
        }
        Ok(())
    }

    /// `whole` with `value` written at `path`, under the budgets, each of which ends the evaluation
    /// like the step budget: first the cost of every struct on the path (its members plus a
    /// constant, whether or not it is shared) and of an actor array stored, then the width budget
    /// for a struct that gains a member, then, after the write, the depth budget on the result.
    fn write_member(
        &mut self,
        mut whole: Value<H>,
        path: &[HashedStr],
        value: Value<H>,
        cx: &mut EvalCx<'_, '_, H>,
    ) -> Result<Value<H>, Unwind<H>> {
        let check = member_store_check(&whole, path, cx.limits.struct_members);
        self.charge(cx, check.cost.saturating_add(store_cost(&value)))?;
        if let Some(limit) = check.exceeded_width {
            self.stopped = true;
            cx.sink.runtime(RuntimeMsg::StructMemberLimit { limit });
            return Err(Unwind::Stop);
        }
        whole.set_member_path(path, storable(cx, value));
        if let Some(limit) = cx.limits.struct_depth
            && whole.struct_depth() > limit
        {
            self.stopped = true;
            cx.sink.runtime(RuntimeMsg::StructDepthLimit { limit });
            return Err(Unwind::Stop);
        }
        Ok(whole)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tree_walker::test_support::*;
    use molangx::numeric::PostOp;
    use molangx::vm::{StructValue, TempMap};

    #[test]
    fn a_missing_read_outside_a_handler_logs_and_ends_the_evaluation_with_zero() {
        let run = both("v.x = 8; v.nothing + 1; v.after = 1; return 5;");
        assert_eq!(run.f(), 0.0);
        assert_eq!(run.env.float("x"), 8.0);
        assert!(run.env.vars.get(var("after")).is_none());
        assert_eq!(
            run.env.messages(),
            ["Error: unhandled request for unknown variable 'variable.nothing'"]
        );
    }

    #[test]
    fn a_missing_member_logs_the_member_then_the_variable_path() {
        let run = both("v.x.y.z + 1");
        assert_eq!(run.f(), 0.0);
        assert_eq!(run.env.messages().len(), 2);
        assert_eq!(
            run.env.messages()[0],
            "Error: unable to find member variable .z"
        );
        assert!(
            run.env.messages()[1].contains("'.z'"),
            "{:?}",
            run.env.messages()
        );
    }

    #[test]
    fn assignments_write_variables_and_temps_and_are_worth_their_value() {
        let run = both("v.a = 4; t.b = v.a * 2; v.c = t.b + 1;");
        assert_eq!(run.env.float("a"), 4.0);
        assert_eq!(run.env.float("c"), 9.0);
        assert_eq!(run.f(), 0.0, "a statement list without return is worth 0");
        assert_eq!(float("return v.a = 4;"), 4.0);
        assert_eq!(float("t.a = 3; t.a + 1;"), 0.0);
        assert_eq!(float("t.a = 3; return t.a + 1;"), 4.0);
    }

    #[test]
    fn temps_follow_the_hosts_choice() {
        // Without a temp map every evaluation starts with no temps and none reach the host; with
        // one they persist.
        let ours = both("t.a = 3; return t.a;");
        assert!(ours.env.temps.is_none());
        assert_eq!(ours.f(), 3.0);
        let mut persistent = Env::new();
        persistent.temps = Some(TempMap::new());
        let first = both_on("t.keep = 6; return 1;", &persistent);
        assert_eq!(
            first
                .env
                .temps
                .as_ref()
                .and_then(|t| t.get(temp_key("keep")))
                .map(V::as_f32),
            Some(6.0)
        );
        let second = both_on("return t.keep + 1;", &first.env);
        assert_eq!(second.f(), 7.0);
    }

    #[test]
    fn member_assignments_create_intermediate_structs() {
        let run = both("v.st.a.b = 5; return v.st.a.b;");
        assert_eq!(run.f(), 5.0);
        let st = run.env.vars.get(var("st")).expect("st is stored");
        assert_eq!(
            st.member_path(&[HashedStr::new("a"), HashedStr::new("b")]),
            Some(&Value::Float(5.0))
        );
        // A member write keeps the other members of the struct.
        let start = Env::new().with_var("st", Value::structure(StructValue::from([("k", 1.0)])));
        let run = both_on("v.st.m = 2; return v.st.k + v.st.m;", &start);
        assert_eq!(run.f(), 3.0);
        // A temp root works the same way and a member write over a number replaces it.
        assert_eq!(both("t.s.m = 7; return t.s.m;").f(), 7.0);
        assert_eq!(both("v.x.m = 7; return v.x.m;").f(), 7.0);
    }

    #[test]
    fn return_ends_the_program_with_its_value_and_post_op() {
        assert_eq!(float("return 5;"), 5.0);
        assert_eq!(float("v.a = 1; return v.x * 2;"), 6.0);
        let run = both("v.a = 1; v.x ? { return v.x * 2; }; v.b = 2;");
        assert_eq!(run.f(), 6.0);
        assert!(run.env.vars.get(var("b")).is_none());
        assert_eq!(float("v.x ? { return 1; }; return 2;"), 1.0);
        assert_eq!(float("v.zero ? { return 1; }; return 2;"), 2.0);
        assert_eq!(float("{ return 3; }; return 4;"), 3.0);
        assert_eq!(float("return v.x + 1;"), 4.0);
    }

    #[test]
    fn a_block_value_is_its_return() {
        assert_eq!(
            float("v.a = 1; { v.a = 2; return v.a + 1; }; v.a = 3;"),
            3.0
        );
        assert_eq!(
            both("v.a = 1; { v.a = 2; return v.a + 1; }; v.a = 3;")
                .env
                .float("a"),
            2.0
        );
    }

    #[test]
    fn strings_are_hashes_that_read_as_their_low_bits() {
        let run = both("v.s");
        assert_eq!(run.value, Value::string("moo"));
        assert_eq!(both("v.t = 'cow'; return v.t;").value, Value::string("cow"));
        assert_eq!(
            both("v.t = 'cow'; return v.t;").env.vars.get(var("t")),
            Some(&Value::string("cow"))
        );
        // In arithmetic a string is the float of its low 32 bits.
        let low = f32::from_bits(HashedStr::new("moo").as_u64() as u32);
        assert_eq!(bits(float("v.s + 0")), bits(low + 0.0));
        assert_eq!(bits(float("v.s * 1")), bits(low));
        assert_eq!(float("'cow' == v.s"), 0.0);
        assert_eq!(float("'moo' == v.s"), 1.0);
        // A string in a branch survives.
        assert_eq!(both("v.x ? 'cow' : 'bull'").value, Value::string("cow"));
        assert_eq!(both("v.zero ? 'cow' : 'bull'").value, Value::string("bull"));
        assert_eq!(both("v.nothing ?? 'moo'").value, Value::string("moo"));
    }

    #[test]
    fn this_is_a_float_with_its_post_op() {
        assert_eq!(float("this"), 2.5);
        assert_eq!(float("this * 2 + 1"), 6.0);
    }

    #[test]
    fn context_reads_and_missing_context() {
        assert_eq!(float("c.n"), 4.0);
        assert_eq!(float("c.n * c.n"), 16.0);
        assert_eq!(float("c.missing ?? 3"), 3.0);
        let run = both("c.missing");
        assert_eq!(run.f(), 0.0);
        assert_eq!(
            run.env.messages(),
            ["Error: unhandled request for unknown variable 'context.missing'"]
        );
    }

    #[test]
    fn an_actor_stored_by_an_assignment_is_made_storable() {
        let run = both("v.who = c.other; v.arr = c.arr;");
        assert_eq!(run.env.vars.get(var("who")), Some(&Value::Actor(2)));
        // The dead entry of the array is dropped by storing it.
        assert_eq!(
            run.env.vars.get(var("arr")),
            Some(&Value::actor_array([1, 2]))
        );
    }

    #[test]
    fn a_member_store_costs_four_plus_the_members_of_each_struct_on_its_path() {
        // `v.st.m = 1` on a struct with two members: the base cost of the store plus the struct
        // copy.
        let start = Env::new().with_var(
            "st",
            Value::structure(StructValue::from([("a", 1.0), ("b", 2.0)])),
        );
        let plain = smallest_budget("v.k = 1;", &start);
        let nested = smallest_budget("v.st.m = 1;", &start);
        assert_eq!(
            nested - plain,
            4 + 2,
            "STRUCT_COPY_STEPS plus the two members"
        );
        let deeper = Env::new().with_var(
            "st",
            Value::structure(StructValue::from([(
                "a",
                Value::structure(StructValue::from([("z", 1.0)])),
            )])),
        );
        let one = smallest_budget("v.st.m = 1;", &deeper);
        let two = smallest_budget("v.st.a.m = 1;", &deeper);
        assert_eq!(
            two - one,
            4 + 1,
            "one more struct on the path: the copy and its one member"
        );
    }

    #[test]
    fn storing_an_actor_array_costs_its_length() {
        let start = Env::new();
        let one = smallest_budget("v.a = c.n;", &start);
        let array = smallest_budget("v.a = c.arr;", &start);
        assert_eq!(array - one, 3, "an actor array of three entries");
    }

    #[test]
    fn a_struct_deeper_than_the_budget_ends_the_evaluation() {
        let mut start = Env::new();
        start.limits.struct_depth = Some(2);
        let run = both_on("v.st.a.b = 1; v.after = 1; return 7;", &start);
        assert_eq!(run.f(), 7.0, "depth 2 is allowed");
        let run = both_on("v.q.a.b.c = 1; v.after = 1; return 7;", &start);
        assert_eq!(run.f(), 0.0);
        assert!(run.env.vars.get(var("after")).is_none());
        assert!(run.env.vars.get(var("q")).is_none(), "nothing is written");
        assert_eq!(
            run.env.messages(),
            ["molangx: evaluation stopped: a struct would nest deeper than its budget of 2 levels"]
        );
    }

    #[test]
    fn a_struct_wider_than_the_budget_ends_the_evaluation() {
        let mut start = Env::new();
        start.limits.struct_members = Some(2);
        let run = both_on("v.q.a = 1; v.q.b = 2; return 7;", &start);
        assert_eq!(run.f(), 7.0);
        let run = both_on(
            "v.q.a = 1; v.q.b = 2; v.q.c = 3; v.after = 1; return 7;",
            &start,
        );
        assert_eq!(run.f(), 0.0);
        assert!(run.env.vars.get(var("after")).is_none());
        assert_eq!(
            run.env.messages(),
            ["molangx: evaluation stopped: a struct would hold more than its budget of 2 members"]
        );
        // Rewriting an existing member is never refused.
        let run = both_on("v.q.a = 1; v.q.b = 2; v.q.a = 9; return v.q.a;", &start);
        assert_eq!(run.f(), 9.0);
    }

    #[test]
    fn write_member_charges_creates_and_checks_the_limits_in_order() {
        let expr = build("v.x");
        let path = [HashedStr::new("a"), HashedStr::new("b")];
        // Creating a two-level path in nothing: the cost is positive and the struct is built.
        let mut walker = Walker::<World>::new(&expr);
        let mut env = Env::new();
        let whole = walker.write_member(Value::ZERO, &path, Value::Float(1.0), &mut env.cx());
        let whole = whole.unwrap_or_else(|_| panic!("the write is within the budgets"));
        assert_eq!(whole.member_path(&path), Some(&Value::Float(1.0)));
        assert!(
            walker.steps >= 4,
            "at least one struct copy: {}",
            walker.steps
        );
        let spent = walker.steps;
        // The same write with a budget one short of the cost stops before writing.
        let mut walker = Walker::<World>::new(&expr);
        let mut env = Env::new().with_steps(spent - 1);
        assert!(is_stop(&walker.write_member(
            Value::ZERO,
            &path,
            Value::Float(1.0),
            &mut env.cx()
        )));
        assert_eq!(
            env.messages(),
            [format!("{STEP_MESSAGE}{} steps", spent - 1)]
        );
        // The width check comes after the cost: with both exceeded the step message wins.
        let mut walker = Walker::<World>::new(&expr);
        let mut env = Env::new().with_steps(0);
        env.limits.struct_members = Some(0);
        assert!(is_stop(&walker.write_member(
            Value::ZERO,
            &path,
            Value::Float(1.0),
            &mut env.cx()
        )));
        assert_eq!(env.messages(), [format!("{STEP_MESSAGE}0 steps")]);
        // And with enough steps the width message is the one.
        let mut walker = Walker::<World>::new(&expr);
        let mut env = Env::new();
        env.limits.struct_members = Some(0);
        assert!(is_stop(&walker.write_member(
            Value::ZERO,
            &path,
            Value::Float(1.0),
            &mut env.cx()
        )));
        assert_eq!(
            env.messages(),
            ["molangx: evaluation stopped: a struct would hold more than its budget of 0 members"]
        );
    }

    #[test]
    fn a_resource_literal_is_a_resource_value_worth_one_step() {
        let mut walker = walker_for("v.x");
        let mut env = Env::new();
        for (op, payload, name) in [
            (
                Op::Geometry,
                Payload::Geometry(molangx::internals::Name::new("geometry.cow")),
                "geometry.cow",
            ),
            (
                Op::Material,
                Payload::Material(molangx::internals::Name::new("material.cow")),
                "material.cow",
            ),
            (
                Op::Texture,
                Payload::Texture(molangx::internals::Name::new("texture.cow")),
                "texture.cow",
            ),
        ] {
            let before = walker.steps;
            let got = walk(&mut walker, &leaf(op, payload), &mut env.cx());
            assert_eq!(
                got,
                Some(Value::Resource(ResourceRef::from_raw_hash(HashedStr::new(
                    name
                )))),
                "{op:?}"
            );
            // A host builds the same handle from any spelling of the name.
            assert_eq!(
                got,
                Some(Value::Resource(ResourceRef::new(
                    &name.to_ascii_uppercase()
                ))),
                "{op:?}"
            );
            assert_eq!(walker.steps, before + 1);
        }
        // Without a name the resource is the empty hash.
        let got = walk(&mut walker, &node(Op::Texture, vec![]), &mut env.cx());
        assert_eq!(
            got,
            Some(Value::Resource(ResourceRef::from_raw_hash(
                HashedStr::from_u64(0)
            )))
        );
    }

    #[test]
    fn nodes_with_the_wrong_payload_are_zero() {
        let mut walker = walker_for("v.x");
        let mut env = Env::new();
        // A read with no name, a member with no member name, a query with no query.
        for op in [Op::EntityVariable, Op::MemberAccessor, Op::QueryFunction] {
            assert_eq!(
                walk(&mut walker, &node(op, vec![number(1.0)]), &mut env.cx()),
                Some(Value::ZERO),
                "{op:?}"
            );
        }
    }

    #[test]
    fn a_leaf_float_is_loaded_without_its_post_op() {
        let mut walker = walker_for("v.x");
        let mut env = Env::new();
        let five = Node::new(
            Op::Float,
            Payload::Float(5.0),
            PostOp::new(3.0, 7.0),
            vec![],
        );
        assert_eq!(
            walk(&mut walker, &five, &mut env.cx()),
            Some(Value::Float(5.0))
        );
        // `this` takes its post-op.
        let this = Node::new(Op::This, Payload::None, PostOp::new(2.0, 1.0), vec![]);
        assert_eq!(
            walk(&mut walker, &this, &mut env.cx()),
            Some(Value::Float(6.0))
        );
    }
}
