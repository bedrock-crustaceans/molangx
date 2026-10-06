//! Control flow: `?:`, `??`, statement lists, `return`, `break` / `continue`, `loop` and
//! `for_each`.

use super::code::Label;
use super::{Build, Builder, LinkError, three, two};
use crate::compile::{
    ast::{Node, Payload},
    program::{Instr, ProgramFlags},
};
use crate::ops::ExpressionOp as Op;

/// The loop a `break` / `continue` jumps out of / back to.
#[derive(Copy, Clone)]
pub(super) struct LoopTargets {
    /// Where `continue` jumps to.
    pub(super) check: Label,
    /// Where `break` jumps to.
    pub(super) cleanup: Label,
}

impl Builder<'_, '_> {
    #[inline(never)]
    pub(super) fn coalesce(&mut self, node: &Node) -> Build {
        let [x, y] = two(node)?;
        let handler = self.code.label();
        let end = self.code.label();
        self.code.emit_jump(Instr::HandlerPush { to: 0 }, handler);
        self.depth.enter_handler();
        self.expr(x)?;
        self.depth.leave_handler();
        self.code.emit_jump(Instr::HandlerPop { to: 0 }, end);
        self.code.place(handler);
        self.expr(y)?;
        self.code.place(end);
        self.post_instr(node)
    }

    #[inline(never)]
    pub(super) fn conditional(&mut self, node: &Node) -> Build {
        let [condition, then, rest @ ..] = node.children.as_slice() else {
            return Err(LinkError::Failed);
        };
        let otherwise = self.code.label();
        let end = self.code.label();
        self.expr(condition)?;
        self.code.emit_jump(Instr::JumpIfFalsy { to: 0 }, otherwise);
        self.expr(then)?;
        self.code.emit_jump(Instr::Jump { to: 0 }, end);
        self.code.place(otherwise);
        if let Some(else_branch) = rest.first() {
            self.expr(else_branch)?;
        } else {
            // `c ? a` without `: b`: the false branch is 0.
            let c = self.konst(0.0)?;
            self.code.emit(Instr::Const { c });
        }
        self.code.place(end);
        self.post_instr(node)
    }

    #[inline(never)]
    pub(super) fn statements(&mut self, node: &Node) -> Build {
        for statement in &node.children {
            if statement.is(Op::Return) {
                self.ret(statement)?;
            } else {
                self.expr(statement)?;
            }
        }
        // A statement list leaves its constant (`O`, 0 unless a parent folded into it).
        let c = self.konst(node.post.offset)?;
        self.code.emit(Instr::Const { c });
        Ok(())
    }

    /// `break` or `continue`: a jump to `target` of the innermost loop.
    #[inline(never)]
    pub(super) fn jump_out(&mut self, target: fn(LoopTargets) -> Label) {
        match self.loops.last() {
            Some(&targets) => self.code.emit_jump(Instr::Jump { to: 0 }, target(targets)),
            // No loop around it (`break;` at the root loads and logs): the program ends with 0.
            None => {
                self.code.emit(Instr::Halt);
            }
        }
    }

    fn ret(&mut self, node: &Node) -> Build {
        if let Some(value) = node.children.first() {
            self.expr(value)?;
        } else {
            let c = self.konst(0.0)?;
            self.code.emit(Instr::Const { c });
        }
        let p = self.post(node)?;
        self.code.emit(Instr::Return { p });
        Ok(())
    }

    pub(super) fn counted_loop(&mut self, node: &Node) -> Build {
        let [count, body] = two(node)?;
        let exit = self.code.label();
        let body_start = self.code.label();
        let check = self.code.label();
        let cleanup = self.code.label();
        self.flags |= ProgramFlags::HAS_LOOPS;
        self.expr(count)?;
        self.code.emit_jump(Instr::LoopBegin { exit: 0 }, exit);
        // The counter is an operand-stack slot for the whole loop.
        self.depth.push_operand();
        self.depth.enter_loop();
        self.code.place(body_start);
        self.loops.push(LoopTargets { check, cleanup });
        self.expr(body)?;
        self.loops.pop();
        self.code.place(check);
        self.code
            .emit_jump(Instr::LoopCheck { body: 0 }, body_start);
        self.code.place(cleanup);
        self.code.emit(Instr::LoopEnd);
        self.depth.pop_operands(1);
        self.depth.leave_loop();
        self.code.place(exit);
        let c = self.konst(node.post.apply(0.0))?;
        self.code.emit(Instr::Const { c });
        Ok(())
    }

    pub(super) fn for_each(&mut self, node: &Node) -> Build {
        let [variable, array, body] = three(node)?;
        let each_next = match &variable.value {
            Payload::Entity(name) => Instr::EachNextVar {
                n: self.name(name)?,
                exit: 0,
            },
            Payload::Temp(name) => Instr::EachNextTemp {
                t: self.temp(name)?,
                exit: 0,
            },
            _ => return Err(LinkError::Failed),
        };
        let exit = self.code.label();
        let next = self.code.label();
        let cleanup = self.code.label();
        self.flags |= ProgramFlags::HAS_LOOPS;
        self.not_float_only();
        self.expr(array)?;
        self.code.emit_jump(Instr::EachBegin { exit: 0 }, exit);
        self.depth.enter_loop();
        self.code.place(next);
        self.code.emit_jump(each_next, cleanup);
        self.loops.push(LoopTargets {
            check: next,
            cleanup,
        });
        self.expr(body)?;
        self.loops.pop();
        self.code.emit_jump(Instr::Jump { to: 0 }, next);
        self.code.place(cleanup);
        self.code.emit(Instr::LoopEnd);
        self.depth.leave_loop();
        self.code.place(exit);
        let c = self.konst(node.post.apply(0.0))?;
        self.code.emit(Instr::Const { c });
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use crate::compile::{
        ast::Payload,
        codegen::{LinkError, test_support::*},
        program::{ConstIdx, Depths, Instr, ProgramFlags, TempIdx},
    };
    use crate::ops::ExpressionOp as Op;

    #[test]
    fn conditional_with_an_else_branch() {
        assert_listing(
            "v.x ? 1 : 2",
            &[
                "load variable.x",
                "jump-if-falsy -> 4",
                "const 1",
                "jump -> 5",
                "const 2",
                "end",
            ],
        );
        assert_listing(
            "v.x ? v.a : v.b",
            &[
                "load variable.x",
                "jump-if-falsy -> 4",
                "load variable.a",
                "jump -> 5",
                "load variable.b",
                "end",
            ],
        );
        assert_listing(
            "v.x ? 1 : v.y ? 2 : 3",
            &[
                "load variable.x",
                "jump-if-falsy -> 4",
                "const 1",
                "jump -> 9",
                "load variable.y",
                "jump-if-falsy -> 8",
                "const 2",
                "jump -> 9",
                "const 3",
                "end",
            ],
        );
        assert_listing(
            "v.x ? v.y ? 1 : 2 : 3",
            &[
                "load variable.x",
                "jump-if-falsy -> 8",
                "load variable.y",
                "jump-if-falsy -> 6",
                "const 1",
                "jump -> 7",
                "const 2",
                "jump -> 9",
                "const 3",
                "end",
            ],
        );
        assert_listing(
            "v.x && v.y ? 1 : 2",
            &[
                "load variable.x",
                "and-step -> 4",
                "load variable.y",
                "and-last",
                "jump-if-falsy -> 7",
                "const 1",
                "jump -> 8",
                "const 2",
                "end",
            ],
        );
    }

    #[test]
    fn conditional_without_else_loads_zero_on_the_false_branch() {
        assert_listing(
            "v.x ? 1",
            &[
                "load variable.x",
                "jump-if-falsy -> 4",
                "const 1",
                "jump -> 5",
                "const 0",
                "end",
            ],
        );
    }

    #[test]
    fn the_conditional_post_op_sits_after_the_end_label() {
        assert_listing(
            "(v.x ? 1 : 2) * 3",
            &[
                "load variable.x",
                "jump-if-falsy -> 4",
                "const 1",
                "jump -> 5",
                "const 2",
                "post *3+0",
                "end",
            ],
        );
        // Both the then-branch jump and the else fall-through reach the post instruction.
        let p = program_of("(v.x ? 1 : 2) * 3");
        assert_eq!(p.code[3], Instr::Jump { to: 5 });
        assert!(matches!(p.code[5], Instr::Post { .. }));
    }

    #[test]
    fn conditional_nodes_need_a_condition_and_a_then_branch() {
        assert_eq!(
            built(&op(Op::Conditional, vec![])).unwrap_err(),
            LinkError::Failed
        );
        assert_eq!(
            built(&op(Op::Conditional, vec![var("x")])).unwrap_err(),
            LinkError::Failed
        );
    }

    #[test]
    fn null_coalescing_registers_a_handler_around_the_left_side() {
        assert_listing(
            "v.a ?? 1",
            &[
                "handler-push -> 3",
                "load variable.a",
                "handler-pop -> 4",
                "const 1",
                "end",
            ],
        );
        assert_listing(
            "v.a ?? v.b",
            &[
                "handler-push -> 3",
                "load variable.a",
                "handler-pop -> 4",
                "load variable.b",
                "end",
            ],
        );
        let p = program_of("v.a ?? 1");
        assert_eq!(p.depths.handlers, 1);
    }

    #[test]
    fn the_coalescing_post_op_sits_after_the_end_label() {
        assert_listing(
            "(v.a ?? 1) * 2",
            &[
                "handler-push -> 3",
                "load variable.a",
                "handler-pop -> 4",
                "const 1",
                "post *2+0",
                "end",
            ],
        );
        assert_listing(
            "v.a ?? (v.b ?? 1) + 1",
            &[
                "handler-push -> 3",
                "load variable.a",
                "handler-pop -> 8",
                "handler-push -> 6",
                "load variable.b",
                "handler-pop -> 7",
                "const 1",
                "post *1+1",
                "end",
            ],
        );
    }

    #[test]
    fn nested_right_hand_coalescing_needs_one_handler_at_a_time() {
        assert_listing(
            "v.a ?? (v.b ?? 1)",
            &[
                "handler-push -> 3",
                "load variable.a",
                "handler-pop -> 7",
                "handler-push -> 6",
                "load variable.b",
                "handler-pop -> 7",
                "const 1",
                "end",
            ],
        );
        assert_eq!(
            program_of("v.a ?? (v.b ?? (v.c ?? (v.d ?? (v.e ?? 1))))")
                .depths
                .handlers,
            1
        );
    }

    #[test]
    fn coalescing_in_two_operands_reuses_the_handler_depth() {
        assert_listing(
            "(v.a ?? 1) + (v.b ?? 2)",
            &[
                "handler-push -> 3",
                "load variable.a",
                "handler-pop -> 4",
                "const 1",
                "push",
                "handler-push -> 8",
                "load variable.b",
                "handler-pop -> 9",
                "const 2",
                "add",
                "end",
            ],
        );
        assert_eq!(program_of("(v.a ?? 1) + (v.b ?? 2)").depths.handlers, 1);
    }

    #[test]
    fn hand_built_left_nested_coalescing_counts_handler_depth() {
        let mut tree = var("a");
        for _ in 0..3 {
            tree = op(Op::NullCoalescing, vec![tree, float(1.0)]);
        }
        let p = built(&tree).unwrap();
        assert_eq!(p.depths.handlers, 3);
        // Each level's handler is its own right-hand constant; the pops jump to the end of their
        // level.
        assert_eq!(
            p.disassemble(),
            numbered(&[
                "handler-push -> 9",
                "handler-push -> 7",
                "handler-push -> 5",
                "load variable.a",
                "handler-pop -> 6",
                "const 1",
                "handler-pop -> 8",
                "const 1",
                "handler-pop -> 10",
                "const 1",
                "end"
            ])
        );
        assert_eq!(
            built(&op(Op::NullCoalescing, vec![var("a")])).unwrap_err(),
            LinkError::Failed
        );
    }

    #[test]
    fn a_statement_list_ends_with_its_constant() {
        assert_listing(
            "v.a = 1; v.b = 2;",
            &[
                "const 1",
                "store variable.a",
                "const 2",
                "store variable.b",
                "const 0",
                "end",
            ],
        );
        assert_listing(
            "v.a = 1; v.a = 1;",
            &[
                "const 1",
                "store variable.a",
                "const 1",
                "store variable.a",
                "const 0",
                "end",
            ],
        );
    }

    #[test]
    fn return_ends_the_program_after_its_value() {
        assert_listing("return 5;", &["const 5", "return", "const 0", "end"]);
        assert_listing(
            "v.a = 1; return v.a + 1;",
            &[
                "const 1",
                "store variable.a",
                "load variable.a *1+1",
                "return",
                "const 0",
                "end",
            ],
        );
    }

    #[test]
    fn a_statement_list_constant_comes_from_the_node_offset() {
        let tree = with_post(
            op(
                Op::Semicolon,
                vec![op(Op::Assignment, vec![var("a"), float(1.0)])],
            ),
            1.0,
            5.0,
        );
        assert_built(&tree, &["const 1", "store variable.a", "const 5", "end"]);
    }

    #[test]
    fn a_statement_list_constant_keeps_the_sign_of_a_zero_offset() {
        let list = |offset: f32| {
            with_post(
                op(
                    Op::Semicolon,
                    vec![op(Op::Assignment, vec![var("a"), float(1.0)])],
                ),
                1.0,
                offset,
            )
        };
        // `0.0` and `-0.0` are two pool entries after the assigned `1`.
        let p = built(&list(-0.0)).unwrap();
        assert_eq!(p.consts.len(), 2);
        assert!(p.consts[1] == 0.0 && p.consts[1].is_sign_negative());
        let p = built(&list(0.0)).unwrap();
        assert!(p.consts[1] == 0.0 && p.consts[1].is_sign_positive());
        // Both signs in one program stay apart: the second list reads the pool entry of its own
        // sign.
        let both = op(Op::Add, vec![list(0.0), list(-0.0)]);
        let p = built(&both).unwrap();
        assert_eq!(p.consts.len(), 3);
        assert!(p.consts[1].is_sign_positive() && p.consts[2].is_sign_negative());
    }

    #[test]
    fn return_forms_by_hand() {
        // `return;` without a value returns 0.
        let bare = op(Op::Semicolon, vec![op(Op::Return, vec![])]);
        assert_built(&bare, &["const 0", "return", "const 0", "end"]);
        let post = op(
            Op::Semicolon,
            vec![with_post(op(Op::Return, vec![var("x")]), 2.0, 1.0)],
        );
        assert_built(&post, &["load variable.x", "return *2+1", "const 0", "end"]);
        // A return outside a statement list builds as a generic constant.
        let lone = with_post(node(Op::Return, Payload::Float(4.0), vec![]), 2.0, 1.0);
        assert_built(&lone, &["const 9", "end"]);
    }

    #[test]
    fn a_loop_counts_on_the_operand_stack_and_cleans_up() {
        assert_listing(
            "loop(3, { v.x = v.x + 1; });",
            &[
                "const 3",
                "loop-begin -> 7",
                "load variable.x *1+1",
                "store variable.x",
                "const 0",
                "loop-check -> 2",
                "loop-end",
                "const 0",
                "const 0",
                "end",
            ],
        );
        let p = program_of("loop(3, { v.x = v.x + 1; });");
        assert_eq!(
            p.depths,
            Depths {
                stack: 1,
                loops: 1,
                handlers: 0
            }
        );
        assert_eq!(
            p.flags,
            ProgramFlags::FLOAT_ONLY
                .union(ProgramFlags::HAS_ASSIGNMENT)
                .union(ProgramFlags::READS_ACTOR_VARS)
                .union(ProgramFlags::HAS_LOOPS)
        );
        assert!(p.float_loop);
    }

    #[test]
    fn the_loop_check_jumps_to_the_body_and_begin_to_the_exit_constant() {
        let p = program_of("loop(v.n, { v.x = 1; });");
        // 0 load n; 1 loop-begin; 2 const 1; 3 store; 4 const 0; 5 loop-check; 6 loop-end; 7 const
        // 0; ...
        assert_eq!(p.code[1], Instr::LoopBegin { exit: 7 });
        assert_eq!(p.code[5], Instr::LoopCheck { body: 2 });
        assert_eq!(p.code[6], Instr::LoopEnd);
        assert_eq!(p.code[7], Instr::Const { c: ConstIdx(1) });
    }

    #[test]
    fn nested_loops_nest_their_frames_and_their_stack_slots() {
        assert_listing(
            "loop(2, { loop(2, { v.x = 1; }); });",
            &[
                "const 2",
                "loop-begin -> 13",
                "const 2",
                "loop-begin -> 9",
                "const 1",
                "store variable.x",
                "const 0",
                "loop-check -> 4",
                "loop-end",
                "const 0",
                "const 0",
                "loop-check -> 2",
                "loop-end",
                "const 0",
                "const 0",
                "end",
            ],
        );
        let p = program_of("loop(2, { loop(2, { v.x = 1; }); });");
        assert_eq!((p.depths.stack, p.depths.loops), (2, 2));
    }

    #[test]
    fn break_and_continue_become_jumps_to_the_loop_targets() {
        assert_listing(
            "loop(1, { break; });",
            &[
                "const 1",
                "loop-begin -> 6",
                "jump -> 5",
                "const 0",
                "loop-check -> 2",
                "loop-end",
                "const 0",
                "const 0",
                "end",
            ],
        );
        assert_listing(
            "loop(1, { continue; });",
            &[
                "const 1",
                "loop-begin -> 6",
                "jump -> 4",
                "const 0",
                "loop-check -> 2",
                "loop-end",
                "const 0",
                "const 0",
                "end",
            ],
        );
    }

    #[test]
    fn conditional_break_and_continue_in_a_loop() {
        assert_listing(
            "loop(v.n, { v.x ? break; v.y ? continue; });",
            &[
                "load variable.n",
                "loop-begin -> 15",
                "load variable.x",
                "jump-if-falsy -> 6",
                "jump -> 14",
                "jump -> 7",
                "const 0",
                "load variable.y",
                "jump-if-falsy -> 11",
                "jump -> 13",
                "jump -> 12",
                "const 0",
                "const 0",
                "loop-check -> 2",
                "loop-end",
                "const 0",
                "const 0",
                "end",
            ],
        );
    }

    #[test]
    fn break_and_continue_without_a_loop_halt() {
        assert_listing("break;", &["halt", "const 0", "end"]);
        assert_listing("continue;", &["halt", "const 0", "end"]);
    }

    #[test]
    fn a_break_inside_a_query_argument_halts_the_argument() {
        // An argument sub-program has no loop around it, so `break` ends the argument.
        let l = lines("loop(v.n, { q.position(v.x ? break : 1); });");
        assert_eq!(l[2], "call query.position args [9]");
        assert!(l[9..].contains(&"halt".to_owned()), "{l:?}");
    }

    #[test]
    fn a_loop_value_takes_the_loops_post_op() {
        let body = op(
            Op::Semicolon,
            vec![op(Op::Assignment, vec![var("x"), float(1.0)])],
        );
        let tree = with_post(op(Op::Loop, vec![float(2.0), body]), 2.0, 3.0);
        // The exit constant is the post-op applied to 0.
        assert_built(
            &tree,
            &[
                "const 2",
                "loop-begin -> 7",
                "const 1",
                "store variable.x",
                "const 0",
                "loop-check -> 2",
                "loop-end",
                "const 3",
                "end",
            ],
        );
    }

    #[test]
    fn a_for_each_value_takes_the_post_op_too() {
        let body = op(Op::Semicolon, vec![]);
        let tree = with_post(op(Op::ForEach, vec![temp("e"), var("arr"), body]), 2.0, 4.0);
        assert_built(
            &tree,
            &[
                "load variable.arr",
                "each-begin -> 6",
                "each-next temp.e -> 5",
                "const 0",
                "jump -> 2",
                "loop-end",
                "const 4",
                "end",
            ],
        );
    }

    #[test]
    fn a_loop_assigned_to_a_variable_stores_its_zero() {
        assert_listing(
            "v.z = loop(1, { v.x = 1; });",
            &[
                "const 1",
                "loop-begin -> 7",
                "const 1",
                "store variable.x",
                "const 0",
                "loop-check -> 2",
                "loop-end",
                "const 0",
                "store variable.z",
                "const 0",
                "end",
            ],
        );
    }

    #[test]
    fn for_each_iterates_an_array_into_a_temp_or_a_variable() {
        assert_listing(
            "for_each(t.e, v.arr, { t.n = 1; });",
            &[
                "load variable.arr",
                "each-begin -> 8",
                "each-next temp.e -> 7",
                "const 1",
                "store temp.n",
                "const 0",
                "jump -> 2",
                "loop-end",
                "const 0",
                "const 0",
                "end",
            ],
        );
        assert_listing(
            "for_each(v.e, v.arr, { v.n = 1; break; });",
            &[
                "load variable.arr",
                "each-begin -> 9",
                "each-next variable.e -> 8",
                "const 1",
                "store variable.n",
                "jump -> 8",
                "const 0",
                "jump -> 2",
                "loop-end",
                "const 0",
                "const 0",
                "end",
            ],
        );
    }

    #[test]
    fn for_each_continue_jumps_to_the_next_step_and_break_to_the_cleanup() {
        let p = program_of("for_each(t.e, v.arr, { v.x ? continue; v.y ? break; });");
        // 2 each-next; the continue jumps back to it, the break to the loop-end at 15.
        assert_eq!(
            p.code[2],
            Instr::EachNextTemp {
                t: TempIdx(0),
                exit: 15
            }
        );
        assert_eq!(p.code[5], Instr::Jump { to: 2 });
        assert_eq!(p.code[10], Instr::Jump { to: 15 });
        assert_eq!(p.code[14], Instr::Jump { to: 2 });
        assert_eq!(p.code[15], Instr::LoopEnd);
        assert_eq!(p.code[1], Instr::EachBegin { exit: 16 });
    }

    #[test]
    fn for_each_is_never_float_only_and_keeps_the_counter_off_the_stack() {
        let p = program_of("for_each(t.e, v.arr, { t.n = 1; });");
        assert!(!p.flags.contains(ProgramFlags::FLOAT_ONLY));
        assert!(p.flags.contains(ProgramFlags::HAS_LOOPS));
        assert!(p.flags.contains(ProgramFlags::USES_TEMPS));
        assert!(!p.float_loop);
        // No counter is pushed: the frame lives in the loop stack only.
        assert_eq!(
            p.depths,
            Depths {
                stack: 0,
                loops: 1,
                handlers: 0
            }
        );
        assert_eq!(p.temps.len(), 2);
    }

    #[test]
    fn for_each_needs_a_variable_or_temp_as_its_loop_variable() {
        let body = op(Op::Semicolon, vec![]);
        let tree = op(Op::ForEach, vec![float(1.0), var("arr"), body]);
        assert_eq!(built(&tree).unwrap_err(), LinkError::Failed);
    }

    #[test]
    fn loop_nodes_need_their_children() {
        assert_eq!(
            built(&op(Op::Loop, vec![float(1.0)])).unwrap_err(),
            LinkError::Failed
        );
        assert_eq!(
            built(&op(Op::ForEach, vec![temp("e"), var("a")])).unwrap_err(),
            LinkError::Failed
        );
    }

    #[test]
    fn a_for_each_exit_restores_the_loop_depth() {
        // Sequential `for_each` loops reuse one loop frame: the maximum is not the sum.
        let p = program_of("for_each(t.a, v.x, { t.n = 1; }); for_each(t.b, v.y, { t.n = 2; });");
        assert_eq!(
            p.depths,
            Depths {
                stack: 0,
                loops: 1,
                handlers: 0
            }
        );
        // A `for_each` after a counted loop and the other way round.
        let p = program_of(
            "for_each(t.a, v.x, { t.n = 1; }); loop(1, { t.n = 2; }); for_each(t.b, v.y, { t.n = 3; });",
        );
        assert_eq!((p.depths.stack, p.depths.loops), (1, 1));
        // A nested `for_each` inside a counted loop nests the frames.
        let p = program_of("loop(1, { for_each(t.a, v.x, { t.n = 1; }); });");
        assert_eq!((p.depths.stack, p.depths.loops), (1, 2));
    }

    #[test]
    fn a_loop_exit_restores_the_loop_and_stack_depth() {
        // Two sequential loops use the same slot: the maximum is not the sum.
        let p = program_of("loop(1, { v.x = 1; }); loop(1, { v.y = 1; });");
        assert_eq!((p.depths.stack, p.depths.loops), (1, 1));
    }
}
