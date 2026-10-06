//! Calls: `->` and queries with their argument sub-programs.

use super::{Argument, Build, Builder, LinkError, two};
use crate::compile::{
    ast::{Node, Payload},
    program::{CallIdx, Instr, ProgramFlags, QueryCall},
};

impl Builder<'_, '_> {
    #[inline(never)]
    pub(super) fn pointer(&mut self, node: &Node) -> Build {
        let [lhs, rhs] = two(node)?;
        let after = self.code.label();
        let p = self.post(node)?;
        self.expr(lhs)?;
        self.code.emit_jump(Instr::PointerEnter { to: 0, p }, after);
        self.expr(rhs)?;
        self.code.emit(Instr::PointerLeave { p });
        self.code.place(after);
        self.not_float_only();
        self.flags |= ProgramFlags::USES_ARROW;
        Ok(())
    }

    pub(super) fn query(&mut self, node: &Node) -> Build {
        let Payload::Query(query) = &node.value else {
            return Err(LinkError::Failed);
        };
        let call = self.calls.len();
        let q = CallIdx(u16::try_from(call).map_err(|_| LinkError::Failed)?);
        self.calls.push(QueryCall {
            index: query.index,
            impl_idx: query.impl_idx,
            args: vec![0; node.children.len()].into_boxed_slice(),
        });
        // A query without arguments can run on the float-only loop (its result is handed over
        // when the loop cannot hold it); its arguments are sub-programs only the general loop runs.
        self.flags.remove(ProgramFlags::FLOAT_ONLY);
        self.flags |= ProgramFlags::USES_QUERIES;
        if !node.children.is_empty() {
            self.float_loop = false;
        }
        // Each argument is its own sub-program: no loop around it, its operand depth on top of the
        // caller's.
        let loops = std::mem::take(&mut self.loops);
        for (index, arg) in node.children.iter().enumerate() {
            let caller = std::mem::take(&mut self.code);
            self.expr(arg)?;
            self.code.emit(Instr::End);
            let code = std::mem::replace(&mut self.code, caller);
            self.args.push(Argument { call, index, code });
        }
        self.loops = loops;
        let p = self.post(node)?;
        self.code.emit(Instr::Call { q, p });
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use crate::catalog::Side;
    use crate::compile::{
        ast::{Node, Payload, QueryRef},
        codegen::{LinkError, test_support::*},
        program::ProgramFlags,
    };
    use crate::ops::ExpressionOp as Op;
    use crate::stdlib::query;
    use crate::version::MolangVersion;

    #[test]
    fn pointer_enters_the_target_and_leaves_after_the_right_side() {
        assert_listing(
            "c.o->v.x",
            &[
                "load context.o",
                "pointer-enter -> 4",
                "load variable.x",
                "pointer-leave",
                "end",
            ],
        );
        assert_listing(
            "v.p->v.x",
            &[
                "load variable.p",
                "pointer-enter -> 4",
                "load variable.x",
                "pointer-leave",
                "end",
            ],
        );
        assert_listing(
            "(v.p->v.x) + 1",
            &[
                "load variable.p",
                "pointer-enter -> 4 *1+1",
                "load variable.x",
                "pointer-leave *1+1",
                "end",
            ],
        );
    }

    #[test]
    fn pointer_sets_the_arrow_flag_and_leaves_the_float_loop() {
        let p = program_of("c.o->v.x");
        assert_eq!(
            p.flags,
            ProgramFlags::USES_ARROW
                .union(ProgramFlags::READS_ACTOR_VARS)
                .union(ProgramFlags::READS_CONTEXT)
        );
        assert!(!p.float_loop);
    }

    #[test]
    fn pointer_needs_both_sides() {
        assert_eq!(
            built(&op(Op::Pointer, vec![var("p")])).unwrap_err(),
            LinkError::Failed
        );
    }

    #[test]
    fn a_query_without_arguments_is_one_call() {
        assert_listing("q.is_baby", &["call query.is_baby args []", "end"]);
        assert_listing("q.is_baby * 2", &["call query.is_baby args [] *2+0", "end"]);
        assert_listing(
            "q.is_baby + q.is_baby",
            &[
                "call query.is_baby args []",
                "push",
                "call query.is_baby args []",
                "add",
                "end",
            ],
        );
        let p = program_of("q.is_baby");
        assert_eq!(p.calls.len(), 1);
        assert_eq!(p.catalog.decl(p.calls[0].index).name(), query::IS_BABY);
        assert!(p.calls[0].args.is_empty());
    }

    #[test]
    fn query_arguments_are_sub_programs_after_the_main_code() {
        assert_listing(
            "q.position(0)",
            &["call query.position args [2]", "end", "const 0", "end"],
        );
        assert_listing(
            "q.in_range(1, 2, 3)",
            &[
                "call query.in_range args [2, 4, 6]",
                "end",
                "const 1",
                "end",
                "const 2",
                "end",
                "const 3",
                "end",
            ],
        );
        let p = program_of("q.in_range(1, 2, 3)");
        assert_eq!(*p.calls[0].args, [2, 4, 6]);
    }

    #[test]
    fn query_arguments_start_after_the_main_code_in_the_order_of_their_calls() {
        assert_listing(
            "q.position(0) + q.position(1)",
            &[
                "call query.position args [5]",
                "push",
                "call query.position args [7]",
                "add",
                "end",
                "const 0",
                "end",
                "const 1",
                "end",
            ],
        );
        let p = program_of("q.position(0) + q.position(1)");
        assert_eq!(
            (&*p.calls[0].args, &*p.calls[1].args),
            ([5].as_slice(), [7].as_slice())
        );
    }

    #[test]
    fn nested_query_arguments_finish_innermost_first() {
        assert_listing(
            "q.position(q.position(1))",
            &[
                "call query.position args [4]",
                "end",
                "const 1",
                "end",
                "call query.position args [2]",
                "end",
            ],
        );
    }

    #[test]
    fn jumps_inside_an_argument_are_relocated_to_its_start() {
        assert_listing(
            "q.in_range(v.a, v.b * v.c, v.d ? 1 : 2)",
            &[
                "call query.in_range args [2, 4, 9]",
                "end",
                "load variable.a",
                "end",
                "load variable.b",
                "push",
                "load variable.c",
                "mul",
                "end",
                "load variable.d",
                "jump-if-falsy -> 13",
                "const 1",
                "jump -> 14",
                "const 2",
                "end",
            ],
        );
    }

    #[test]
    fn every_jump_lands_inside_the_program() {
        for src in [
            "v.x ? 1 : 2",
            "v.a ?? 1",
            "v.x && v.y",
            "v.a / v.b",
            "loop(2, { v.x ? break; v.y ? continue; });",
            "for_each(t.e, v.arr, { v.x ? continue; });",
            "c.o->v.x",
            "q.in_range(v.a, v.b / v.c, v.d ? 1 : 2)",
            "v.a = 1 / v.b; return v.a ? 1 : 2;",
        ] {
            let mut p = program_of(src);
            let len = p.code.len() as u32;
            for instr in &mut p.code {
                let shown = format!("{instr:?}");
                if let Some(target) = instr.target_mut() {
                    assert!(
                        *target < len,
                        "{src}: {shown} jumps to {target} in {len} instructions"
                    );
                }
            }
        }
    }

    #[test]
    fn query_arguments_use_the_callers_stack_depth() {
        // The outer multiplication has pushed one operand when the argument is built.
        let p = program_of("v.a * q.position(v.b * v.c)");
        assert_eq!(p.depths.stack, 2);
        assert_eq!(program_of("q.position(v.b * v.c)").depths.stack, 1);
        // Each argument starts from the same depth again.
        assert_eq!(
            program_of("q.in_range(v.a * v.b, v.c * v.d, v.e)")
                .depths
                .stack,
            1
        );
    }

    #[test]
    fn query_flags_and_the_float_loop_rule() {
        let none = program_of("q.is_baby");
        assert_eq!(none.flags, ProgramFlags::USES_QUERIES);
        assert!(
            none.float_loop,
            "a query without arguments can run on the float loop"
        );
        let with = program_of("q.position(0)");
        assert_eq!(with.flags, ProgramFlags::USES_QUERIES);
        assert!(!with.float_loop);
        let mixed = program_of("v.x * q.is_baby");
        assert_eq!(
            mixed.flags,
            ProgramFlags::USES_QUERIES.union(ProgramFlags::READS_ACTOR_VARS)
        );
        assert!(mixed.float_loop);
    }

    fn query_node(name: &str, children: Vec<Node>) -> Node {
        // `built` compiles against the standard library's server catalogue.
        let catalog = crate::stdlib::queries(Side::Server);
        let index = catalog.index_of(name).unwrap();
        let impl_idx = catalog
            .decl(index)
            .implementation_at(MolangVersion::LATEST)
            .unwrap();
        node(
            Op::QueryFunction,
            Payload::Query(QueryRef { index, impl_idx }),
            children,
        )
    }

    #[test]
    fn a_failed_argument_fails_the_whole_call() {
        // The argument fails to link: the call is not built.
        let bad = query_node("query.position", vec![op(Op::LeftParenthesis, vec![])]);
        assert_eq!(built(&bad).unwrap_err(), LinkError::Failed);
        // A link result inside an argument surfaces unchanged.
        let arrays = query_node("query.position", vec![op(Op::ArrayVariable, vec![])]);
        assert_eq!(built(&arrays).unwrap_err(), LinkError::UsesArrays);
        // Later queries still build with their own arguments after a good one.
        let good = op(
            Op::Add,
            vec![
                query_node("query.position", vec![float(0.0)]),
                query_node("query.position", vec![float(1.0)]),
            ],
        );
        assert_eq!(built(&good).unwrap().calls.len(), 2);
    }

    #[test]
    fn a_call_records_the_implementation_the_version_selected() {
        let p = built(&query_node(query::CAPE_FLAP_AMOUNT, vec![])).unwrap();
        assert_eq!(
            p.catalog.decl(p.calls[0].index).name(),
            query::CAPE_FLAP_AMOUNT
        );
        assert_eq!(
            p.calls[0].impl_idx, 1,
            "the second range serves the latest version"
        );
    }

    #[test]
    fn a_query_node_needs_its_payload() {
        assert_eq!(
            built(&op(Op::QueryFunction, vec![])).unwrap_err(),
            LinkError::Failed
        );
    }
}
