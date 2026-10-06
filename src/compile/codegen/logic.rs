//! Comparisons, equality, `&&` and `||`.

use super::{Build, Builder, LinkError, payload_f32, two};
use crate::compile::{
    ast::{Node, Payload},
    program::{CmpOp, EqOp, Instr, PostIdx},
};

/// `&&` or `||`.
#[derive(Copy, Clone)]
pub(super) enum Junction {
    And,
    Or,
}

impl Junction {
    /// The instruction after every operand but the last; its jump target is resolved later.
    fn step(self, p: PostIdx) -> Instr {
        match self {
            Self::And => Instr::AndStep { to: 0, p },
            Self::Or => Instr::OrStep { to: 0, p },
        }
    }

    /// The instruction after the last operand.
    fn last(self, p: PostIdx) -> Instr {
        match self {
            Self::And => Instr::AndLast { p },
            Self::Or => Instr::OrLast { p },
        }
    }
}

impl Builder<'_, '_> {
    #[inline(never)]
    pub(super) fn compare(&mut self, node: &Node, op: CmpOp) -> Build {
        let p = self.post(node)?;
        if let [a] = node.children.as_slice() {
            self.expr(a)?;
            let c = self.konst(payload_f32(node))?;
            self.code.emit(Instr::CmpConst { op, c, p });
        } else {
            let [a, b] = two(node)?;
            self.binary(a, b)?;
            self.code.emit(Instr::Cmp { op, p });
            self.depth.pop_operands(1);
        }
        Ok(())
    }

    #[inline(never)]
    pub(super) fn equal(&mut self, node: &Node, op: EqOp) -> Build {
        let p = self.post(node)?;
        if let [a] = node.children.as_slice() {
            self.expr(a)?;
            if let Payload::Hash(hash) = node.value {
                let h = self.hash(hash)?;
                self.code.emit(Instr::EqHash { op, h, p });
                self.not_float_only();
            } else {
                let c = self.konst(node.float())?;
                self.code.emit(Instr::EqConst { op, c, p });
            }
        } else {
            let [a, b] = two(node)?;
            self.binary(a, b)?;
            self.code.emit(Instr::Eq { op, p });
            self.depth.pop_operands(1);
        }
        Ok(())
    }

    pub(super) fn logic(&mut self, node: &Node, junction: Junction) -> Build {
        let Some((last, rest)) = node.children.split_last() else {
            return Err(LinkError::Failed);
        };
        let p = self.post(node)?;
        let end = self.code.label();
        for operand in rest {
            self.expr(operand)?;
            self.code.emit_jump(junction.step(p), end);
        }
        self.expr(last)?;
        self.code.emit(junction.last(p));
        self.code.place(end);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use crate::compile::{
        ast::Payload,
        codegen::{LinkError, test_support::*},
        program::{CmpOp, ConstIdx, EqOp, HashIdx, Instr, PostIdx, ProgramFlags},
    };
    use crate::hash::HashedStr;
    use crate::ops::ExpressionOp as Op;

    #[test]
    fn comparisons_have_a_moved_constant_form() {
        assert_listing("v.x < 1", &["load variable.x", "Lt-const 1", "end"]);
        assert_listing("v.x <= 1", &["load variable.x", "Le-const 1", "end"]);
        assert_listing("v.x >= 1", &["load variable.x", "Ge-const 1", "end"]);
        assert_listing("v.x > 1", &["load variable.x", "Gt-const 1", "end"]);
        assert_listing(
            "(v.x < 1) + 1",
            &["load variable.x", "Lt-const 1 *1+1", "end"],
        );
    }

    #[test]
    fn comparisons_between_values_push_the_left_operand() {
        assert_listing(
            "v.x < v.y",
            &["load variable.x", "push", "load variable.y", "Lt", "end"],
        );
        // A constant on the left cannot move into the node.
        assert_listing(
            "1 < v.x",
            &["const 1", "push", "load variable.x", "Lt", "end"],
        );
        assert_listing(
            "(v.x < v.y) * 2",
            &[
                "load variable.x",
                "push",
                "load variable.y",
                "Lt *2+0",
                "end",
            ],
        );
        assert_listing(
            "1 + (v.x < v.y)",
            &[
                "load variable.x",
                "push",
                "load variable.y",
                "Lt *1+1",
                "end",
            ],
        );
    }

    #[test]
    fn every_comparison_op_lowers_to_its_cmp() {
        for (op_, cmp) in [
            (Op::LessThan, CmpOp::Lt),
            (Op::LessEqual, CmpOp::Le),
            (Op::GreaterEqual, CmpOp::Ge),
            (Op::GreaterThan, CmpOp::Gt),
        ] {
            let general = built(&op(op_, vec![var("x"), var("y")])).unwrap();
            assert_eq!(
                general.code[3],
                Instr::Cmp {
                    op: cmp,
                    p: PostIdx::PLAIN
                },
                "{op_:?}"
            );
            let mut constant = op(op_, vec![var("x")]);
            constant.value = Payload::Float(2.0);
            let constant = built(&constant).unwrap();
            assert_eq!(
                constant.code[1],
                Instr::CmpConst {
                    op: cmp,
                    c: ConstIdx(0),
                    p: PostIdx::PLAIN
                },
                "{op_:?}"
            );
        }
    }

    #[test]
    fn equality_has_a_value_a_constant_and_a_string_form() {
        assert_listing(
            "v.x == v.y",
            &["load variable.x", "push", "load variable.y", "eq", "end"],
        );
        assert_listing(
            "v.x != v.y",
            &["load variable.x", "push", "load variable.y", "ne", "end"],
        );
        assert_listing("v.x == 1", &["load variable.x", "eq-const 1", "end"]);
        assert_listing("v.x != 1", &["load variable.x", "ne-const 1", "end"]);
        assert_listing(
            "(v.x == 1) * 2",
            &["load variable.x", "eq-const 1 *2+0", "end"],
        );
        assert_eq!(
            lines("v.x == 'abc'"),
            [
                "load variable.x".to_owned(),
                format!("eq-hash {}", hex("abc")),
                "end".to_owned()
            ]
        );
        assert_eq!(
            lines("v.x != 'abc'"),
            [
                "load variable.x".to_owned(),
                format!("ne-hash {}", hex("abc")),
                "end".to_owned()
            ]
        );
        // The string may be on either side.
        assert_eq!(lines("'abc' == v.x"), lines("v.x == 'abc'"));
    }

    #[test]
    fn equality_chains_compare_the_boolean_result() {
        assert_listing(
            "v.x == v.y == v.z",
            &[
                "load variable.x",
                "push",
                "load variable.y",
                "eq",
                "push",
                "load variable.z",
                "eq",
                "end",
            ],
        );
    }

    #[test]
    fn string_equality_leaves_the_float_loop() {
        let p = program_of("v.x == 'abc'");
        assert_eq!(p.flags, ProgramFlags::READS_ACTOR_VARS);
        assert!(!p.float_loop);
        assert_eq!(*p.hashes, [HashedStr::new("abc").as_u64()]);
        // The same string twice shares one hash.
        assert_eq!(
            program_of("(v.x == 'abc') + (v.y == 'abc')").hashes.len(),
            1
        );
        // A float equality keeps both.
        let q = program_of("v.x == 1");
        assert!(q.flags.contains(ProgramFlags::FLOAT_ONLY) && q.float_loop);
    }

    #[test]
    fn equality_nodes_with_a_hash_payload_by_hand() {
        let mut eq = op(Op::LogicalEqual, vec![var("x")]);
        eq.value = Payload::Hash(0x1234);
        let p = built(&eq).unwrap();
        assert_eq!(
            p.code[1],
            Instr::EqHash {
                op: EqOp::Eq,
                h: HashIdx(0),
                p: PostIdx::PLAIN
            }
        );
        assert_eq!(*p.hashes, [0x1234]);
        assert!(!p.float_loop);
        let mut ne = op(Op::LogicalNotEqual, vec![var("x")]);
        ne.value = Payload::Float(3.0);
        let p = built(&ne).unwrap();
        assert_eq!(
            p.code[1],
            Instr::EqConst {
                op: EqOp::Ne,
                c: ConstIdx(0),
                p: PostIdx::PLAIN
            }
        );
    }

    #[test]
    fn and_and_or_step_through_the_operands_to_a_shared_end() {
        assert_listing(
            "v.x && v.y",
            &[
                "load variable.x",
                "and-step -> 4",
                "load variable.y",
                "and-last",
                "end",
            ],
        );
        assert_listing(
            "v.x || v.y",
            &[
                "load variable.x",
                "or-step -> 4",
                "load variable.y",
                "or-last",
                "end",
            ],
        );
        assert_listing(
            "v.x && v.y && v.z",
            &[
                "load variable.x",
                "and-step -> 6",
                "load variable.y",
                "and-step -> 6",
                "load variable.z",
                "and-last",
                "end",
            ],
        );
        assert_listing(
            "v.x || v.y || v.z",
            &[
                "load variable.x",
                "or-step -> 6",
                "load variable.y",
                "or-step -> 6",
                "load variable.z",
                "or-last",
                "end",
            ],
        );
    }

    #[test]
    fn and_or_carry_the_post_op_on_every_instruction() {
        assert_listing(
            "(v.x && v.y) * 2",
            &[
                "load variable.x",
                "and-step -> 4 *2+0",
                "load variable.y",
                "and-last *2+0",
                "end",
            ],
        );
        assert_listing(
            "(v.x || v.y) * 2",
            &[
                "load variable.x",
                "or-step -> 4 *2+0",
                "load variable.y",
                "or-last *2+0",
                "end",
            ],
        );
    }

    #[test]
    fn logic_with_an_equality_string_operand() {
        let l = lines("v.x == 'abc' && v.y");
        assert_eq!(l[2], "and-step -> 5");
        assert_eq!(l.len(), 6);
        assert_eq!(l[4], "and-last");
    }

    #[test]
    fn logic_without_operands_fails() {
        assert_eq!(
            built(&op(Op::LogicalAnd, vec![])).unwrap_err(),
            LinkError::Failed
        );
        // A single operand is the last operand.
        let one = op(Op::LogicalOr, vec![var("x")]);
        assert_built(&one, &["load variable.x", "or-last", "end"]);
    }
}
