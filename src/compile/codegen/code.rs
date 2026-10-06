//! The instruction stream: labels, jump fix-ups, operand-depth tracking and the relocation of a
//! finished sub-program.

use super::{Build, Builder, LinkError};
use crate::compile::{
    ast::Node,
    program::{Depths, Instr, PostIdx, ProgramFlags},
};

/// A position in the code that may not be known yet.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub(super) struct Label(usize);

/// One sub-program under construction: its code, its pending jump fix-ups and its labels.
#[derive(Default)]
pub(super) struct Code {
    instrs: Vec<Instr>,
    /// `labels[i]` = the code index label `i` was placed at.
    labels: Vec<Option<usize>>,
    /// Instructions whose jump operand is still to be resolved, with the label it jumps to.
    fixups: Vec<(usize, Label)>,
    /// The code index the last label was placed at. Labels are only placed at the end of the
    /// code, so no label points past it.
    last_label: Option<usize>,
}

impl Code {
    /// Appends `instr` and returns its index.
    pub(super) fn emit(&mut self, instr: Instr) -> usize {
        let at = self.instrs.len();
        // A push followed by a constant is one instruction, unless a jump lands on the
        // constant.
        if let Instr::Const { c } = instr
            && self.last_label != Some(at)
            && let Some(last) = self.instrs.last_mut()
            && *last == Instr::Push
        {
            *last = Instr::PushConst { c };
            return at - 1;
        }
        self.instrs.push(instr);
        at
    }

    pub(super) fn label(&mut self) -> Label {
        self.labels.push(None);
        Label(self.labels.len() - 1)
    }

    /// Places `label` at the next instruction.
    pub(super) fn place(&mut self, label: Label) {
        let at = self.instrs.len();
        self.labels[label.0] = Some(at);
        self.last_label = Some(at);
    }

    /// Emits a jump-carrying instruction whose target becomes `label` in [`Code::finish`].
    pub(super) fn emit_jump(&mut self, instr: Instr, label: Label) {
        let at = self.emit(instr);
        self.fixups.push((at, label));
    }

    /// Resolves the labels and relocates the code to start at `start`.
    pub(super) fn finish(mut self, start: u32) -> Result<Vec<Instr>, LinkError> {
        for &(at, label) in &self.fixups {
            if let Some(target) = self.instrs[at].target_mut() {
                let placed = self.labels[label.0].ok_or(LinkError::Failed)?;
                *target = u32::try_from(placed).map_err(|_| LinkError::Failed)?;
            }
        }
        for instr in &mut self.instrs {
            if let Some(target) = instr.target_mut() {
                *target = target.checked_add(start).ok_or(LinkError::Failed)?;
            }
        }
        Ok(self.instrs)
    }
}

/// The current and the deepest nesting of the evaluator's dynamic structures.
#[derive(Default)]
pub(super) struct Depth {
    now: Depths,
    pub(super) max: Depths,
}

impl Depth {
    /// One more operand on the stack.
    pub(super) fn push_operand(&mut self) {
        self.now.stack += 1;
        self.max.stack = self.max.stack.max(self.now.stack);
    }

    /// `n` operands fewer on the stack.
    pub(super) fn pop_operands(&mut self, n: u16) {
        self.now.stack = self.now.stack.saturating_sub(n);
    }

    pub(super) fn enter_loop(&mut self) {
        self.now.loops += 1;
        self.max.loops = self.max.loops.max(self.now.loops);
    }

    pub(super) fn leave_loop(&mut self) {
        self.now.loops -= 1;
    }

    pub(super) fn enter_handler(&mut self) {
        self.now.handlers += 1;
        self.max.handlers = self.max.handlers.max(self.now.handlers);
    }

    pub(super) fn leave_handler(&mut self) {
        self.now.handlers -= 1;
    }
}

impl Builder<'_, '_> {
    /// Leaves the float-only loop.
    pub(super) fn not_float_only(&mut self) {
        self.flags.remove(ProgramFlags::FLOAT_ONLY);
        self.float_loop = false;
    }

    pub(super) fn push(&mut self) {
        self.code.emit(Instr::Push);
        self.depth.push_operand();
    }

    /// `a`, push, `b`: the operand order of every binary instruction but the division.
    pub(super) fn binary(&mut self, a: &Node, b: &Node) -> Build {
        self.expr(a)?;
        self.push();
        self.expr(b)
    }

    /// A post-op applied after a branch (`?:`, `??`); nothing for the identity.
    pub(super) fn post_instr(&mut self, node: &Node) -> Build {
        let p = self.post(node)?;
        if p != PostIdx::PLAIN {
            self.code.emit(Instr::Post { p });
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use crate::compile::{
        codegen::{
            LinkError,
            code::{Code, Depth, Label},
            test_support::*,
        },
        program::{ConstIdx, HashIdx, Instr, ProgramFlags},
    };

    /// `fixups` are (instruction, label) pairs.
    fn code_of(instrs: Vec<Instr>, labels: Vec<Option<usize>>, fixups: &[(usize, usize)]) -> Code {
        Code {
            instrs,
            labels,
            fixups: fixups.iter().map(|&(at, l)| (at, Label(l))).collect(),
            last_label: None,
        }
    }

    #[test]
    fn finish_resolves_labels_and_relocates_by_the_start() {
        let code = || {
            code_of(
                vec![Instr::Jump { to: 0 }, Instr::End],
                vec![Some(1)],
                &[(0, 0)],
            )
        };
        assert_eq!(
            code().finish(0).unwrap(),
            [Instr::Jump { to: 1 }, Instr::End]
        );
        assert_eq!(
            code().finish(10).unwrap(),
            [Instr::Jump { to: 11 }, Instr::End]
        );
    }

    #[test]
    fn finish_rejects_an_unplaced_label() {
        let code = code_of(
            vec![Instr::Jump { to: 0 }, Instr::End],
            vec![None],
            &[(0, 0)],
        );
        assert_eq!(code.finish(0).unwrap_err(), LinkError::Failed);
    }

    #[test]
    fn finish_rejects_a_relocation_overflow() {
        let max = u32::MAX as usize;
        let code = || {
            code_of(
                vec![Instr::Jump { to: 0 }, Instr::End],
                vec![Some(max)],
                &[(0, 0)],
            )
        };
        assert_eq!(code().finish(1).unwrap_err(), LinkError::Failed);
        // Without relocation the same label is fine.
        assert_eq!(code().finish(0).unwrap()[0], Instr::Jump { to: u32::MAX });
    }

    #[test]
    fn finish_relocates_every_jump_even_without_a_fixup() {
        let code = code_of(
            vec![
                Instr::LoopCheck { body: 3 },
                Instr::Const { c: ConstIdx(0) },
                Instr::End,
            ],
            vec![],
            &[],
        );
        assert_eq!(
            code.finish(5).unwrap(),
            [
                Instr::LoopCheck { body: 8 },
                Instr::Const { c: ConstIdx(0) },
                Instr::End
            ]
        );
    }

    #[test]
    fn finish_resolves_each_fixup_through_its_own_label() {
        let code = code_of(
            vec![
                Instr::JumpIfFalsy { to: 0 },
                Instr::Jump { to: 0 },
                Instr::Const { c: ConstIdx(0) },
                Instr::End,
            ],
            vec![Some(2), Some(3)],
            &[(0, 1), (1, 0)],
        );
        assert_eq!(
            code.finish(100).unwrap(),
            [
                Instr::JumpIfFalsy { to: 103 },
                Instr::Jump { to: 102 },
                Instr::Const { c: ConstIdx(0) },
                Instr::End
            ]
        );
    }

    #[test]
    fn finish_leaves_instructions_without_a_target_alone() {
        let code = code_of(vec![Instr::Push, Instr::Halt, Instr::End], vec![], &[]);
        assert_eq!(
            code.finish(9).unwrap(),
            [Instr::Push, Instr::Halt, Instr::End]
        );
    }

    #[test]
    fn emit_fuses_a_push_with_the_constant_that_follows() {
        let mut code = Code::default();
        code.emit(Instr::Push);
        let at = code.emit(Instr::Const { c: ConstIdx(3) });
        assert_eq!(at, 0);
        assert_eq!(code.instrs, [Instr::PushConst { c: ConstIdx(3) }]);
        // A constant after anything else is not fused.
        code.emit(Instr::Const { c: ConstIdx(4) });
        code.emit(Instr::Const { c: ConstIdx(5) });
        assert_eq!(
            code.instrs,
            [
                Instr::PushConst { c: ConstIdx(3) },
                Instr::Const { c: ConstIdx(4) },
                Instr::Const { c: ConstIdx(5) }
            ]
        );
    }

    #[test]
    fn emit_does_not_fuse_when_a_label_sits_on_the_constant() {
        let mut code = Code::default();
        code.emit(Instr::Push);
        let l = code.label();
        code.place(l);
        assert_eq!(code.emit(Instr::Const { c: ConstIdx(0) }), 1);
        assert_eq!(code.instrs, [Instr::Push, Instr::Const { c: ConstIdx(0) }]);
        assert_eq!(code.labels, [Some(1)]);
    }

    #[test]
    fn emit_fuses_again_when_the_label_is_elsewhere() {
        let mut code = Code::default();
        let l = code.label();
        code.place(l);
        code.emit(Instr::Push);
        code.emit(Instr::Const { c: ConstIdx(2) });
        assert_eq!(code.instrs, [Instr::PushConst { c: ConstIdx(2) }]);
        assert_eq!(code.labels, [Some(0)]);
    }

    #[test]
    fn emit_only_fuses_a_constant_not_other_loads() {
        let mut code = Code::default();
        code.emit(Instr::Push);
        code.emit(Instr::Hash { h: HashIdx(0) });
        assert_eq!(code.instrs, [Instr::Push, Instr::Hash { h: HashIdx(0) }]);
    }

    #[test]
    fn labels_are_numbered_and_placed_at_the_next_instruction() {
        let mut code = Code::default();
        let (l0, l1) = (code.label(), code.label());
        assert_eq!((l0, l1), (Label(0), Label(1)));
        assert_eq!(code.labels, [None, None]);
        code.emit(Instr::Push);
        code.place(l1);
        assert_eq!(code.labels, [None, Some(1)]);
        assert_eq!(code.last_label, Some(1));
    }

    #[test]
    fn emit_jump_records_the_instruction_and_its_label() {
        let mut code = Code::default();
        let _skip = code.label();
        let target = code.label();
        code.emit_jump(Instr::Jump { to: 0 }, target);
        assert_eq!(code.instrs, [Instr::Jump { to: 0 }]);
        assert_eq!(code.fixups, [(0, Label(1))]);
        // An instruction without a target is emitted unchanged.
        code.emit_jump(Instr::Push, target);
        assert_eq!(code.instrs[1], Instr::Push);
        code.place(target);
        assert_eq!(
            code.finish(0).unwrap(),
            [Instr::Jump { to: 2 }, Instr::Push]
        );
    }

    #[test]
    fn not_float_only_clears_the_flag_and_the_float_loop() {
        builder!(b);
        assert!(b.flags.contains(ProgramFlags::FLOAT_ONLY) && b.float_loop);
        b.not_float_only();
        assert!(!b.flags.contains(ProgramFlags::FLOAT_ONLY));
        assert!(!b.float_loop);
    }

    #[test]
    fn push_and_pop_operands_track_the_operand_depth() {
        builder!(b);
        b.push();
        b.push();
        b.push();
        assert_eq!((b.depth.now.stack, b.depth.max.stack), (3, 3));
        b.depth.pop_operands(2);
        assert_eq!((b.depth.now.stack, b.depth.max.stack), (1, 3));
        b.push();
        assert_eq!((b.depth.now.stack, b.depth.max.stack), (2, 3));
        // Popping more than is there stops at zero.
        b.depth.pop_operands(9);
        assert_eq!(b.depth.now.stack, 0);
        assert_eq!(b.code.instrs, [Instr::Push; 4]);
    }

    #[test]
    fn loops_and_handlers_keep_their_deepest_nesting() {
        let mut depth = Depth::default();
        depth.enter_loop();
        depth.enter_loop();
        depth.leave_loop();
        depth.enter_handler();
        depth.leave_handler();
        depth.enter_loop();
        assert_eq!((depth.now.loops, depth.max.loops), (2, 2));
        assert_eq!((depth.now.handlers, depth.max.handlers), (0, 1));
    }
}
