//! Arithmetic, the math functions, random values and die rolls.

use super::{Build, Builder, LinkError, first_child, two};
#[cfg(feature = "stdlib")]
use super::{payload_f32, three};
use crate::compile::{
    ast::{Node, Payload},
    program::{Instr, PostIdx, ProgramFlags},
};
use crate::ops::ExpressionOp as Op;
#[cfg(feature = "stdlib")]
use crate::{
    compile::program::{Fn1, Fn2, Fn3},
    stdlib::math,
};

impl Builder<'_, '_> {
    /// `-` or `!`.
    #[inline(never)]
    pub(super) fn unary(&mut self, node: &Node, instr: fn(PostIdx) -> Instr) -> Build {
        self.expr(first_child(node)?)?;
        let p = self.post(node)?;
        self.code.emit(instr(p));
        Ok(())
    }

    #[inline(never)]
    pub(super) fn mul(&mut self, node: &Node) -> Build {
        let [a, b] = two(node)?;
        self.binary(a, b)?;
        let p = self.post(node)?;
        self.code.emit(Instr::Mul { p });
        self.depth.pop_operands(1);
        Ok(())
    }

    #[inline(never)]
    pub(super) fn div(&mut self, node: &Node) -> Build {
        let [a, b] = two(node)?;
        let end = self.code.label();
        self.expr(b)?;
        let guard = Instr::DivGuard {
            end: 0,
            divisor: self.divisor,
        };
        // The guard pushes the divisor.
        self.code.emit_jump(guard, end);
        self.depth.push_operand();
        self.expr(a)?;
        let p = self.post(node)?;
        self.code.emit(Instr::Div { p });
        self.depth.pop_operands(1);
        self.code.place(end);
        Ok(())
    }

    /// The n-ary `+`: the first term pushed, every further term accumulated into the top of the
    /// stack, the last one added with the post-op.
    pub(super) fn add(&mut self, node: &Node) -> Build {
        let Some((last, rest)) = node.children.split_last() else {
            return Err(LinkError::Failed);
        };
        let Some((first, middle)) = rest.split_first() else {
            // A one-term `+` never survives optimisation; treat it as the term with the post-op.
            self.expr(last)?;
            return self.post_instr(node);
        };
        self.expr(first)?;
        self.push();
        for term in middle {
            self.expr(term)?;
            self.code.emit(Instr::AddAcc);
        }
        self.expr(last)?;
        let p = self.post(node)?;
        self.code.emit(Instr::AddLast { p });
        self.depth.pop_operands(1);
        Ok(())
    }

    /// A host math call: every argument but the last pushed, in order.
    #[inline(never)]
    pub(super) fn host_math(&mut self, node: &Node) -> Build {
        let Payload::HostMath(f) = node.value else {
            return Err(LinkError::Failed);
        };
        let Some((last, rest)) = node.children.split_last() else {
            return Err(LinkError::Failed);
        };
        let argc = u8::try_from(node.children.len()).map_err(|_| LinkError::Failed)?;
        for arg in rest {
            self.expr(arg)?;
            self.push();
        }
        self.expr(last)?;
        let p = self.post(node)?;
        self.code.emit(Instr::HostMath { f, argc, p });
        self.depth.pop_operands(u16::from(argc) - 1);
        if node.is(Op::HostMathVolatile) {
            self.flags |= ProgramFlags::USES_RANDOM.union(ProgramFlags::USES_RANDOM_OP);
        }
        Ok(())
    }
}

/// The `math.*` functions of the standard library.
#[cfg(feature = "stdlib")]
impl Builder<'_, '_> {
    /// A `math.*` function but `math.pi`.
    pub(super) fn math(&mut self, node: &Node, op: Op) -> Build {
        match op {
            Op::Mod => self.modulo(node),
            Op::Random => self.random(node),
            Op::RandomInt => self.random_integer(node),
            Op::DieRoll => self.die_roll(node, |p| Instr::DieRoll { p }),
            Op::DieRollInt => self.die_roll(node, |p| Instr::DieRollInt { p }),
            _ => {
                if let Some(f) = Fn1::of(op) {
                    self.math1(node, f)
                } else if let Some(f) = Fn2::of(op) {
                    self.math2(node, f)
                } else if let Some(f) = Fn3::of(op) {
                    self.math3(node, f)
                } else {
                    Err(LinkError::Failed)
                }
            }
        }
    }

    #[inline(never)]
    fn modulo(&mut self, node: &Node) -> Build {
        let p = self.post(node)?;
        if let [a] = node.children.as_slice() {
            self.expr(a)?;
            let c = self.konst(payload_f32(node))?;
            self.code.emit(Instr::ModConst { c, p });
        } else {
            let [a, b] = two(node)?;
            self.binary(a, b)?;
            self.code.emit(Instr::Mod { p });
            self.depth.pop_operands(1);
        }
        Ok(())
    }

    #[inline(never)]
    fn math1(&mut self, node: &Node, f: Fn1) -> Build {
        self.expr(first_child(node)?)?;
        let p = self.post(node)?;
        self.code.emit(Instr::Math1 { f, p });
        Ok(())
    }

    #[inline(never)]
    fn math2(&mut self, node: &Node, f: Fn2) -> Build {
        let p = self.post(node)?;
        if let [a] = node.children.as_slice() {
            // The constant operand moved into the node's value; the instruction takes it as its
            // second operand.
            self.expr(a)?;
            let c = self.konst(payload_f32(node))?;
            self.code.emit(Instr::Math2Const { f, c, p });
        } else {
            let [a, b] = two(node)?;
            self.binary(a, b)?;
            self.code.emit(Instr::Math2 { f, p });
            self.depth.pop_operands(1);
        }
        Ok(())
    }

    fn math3(&mut self, node: &Node, f: Fn3) -> Build {
        let [first, second, third] = three(node)?;
        self.expr(first)?;
        self.push();
        self.binary(second, third)?;
        let p = self.post(node)?;
        self.code.emit(Instr::Math3 { f, p });
        self.depth.pop_operands(2);
        Ok(())
    }

    /// `math.die_roll` or `math.die_roll_integer`.
    #[inline(never)]
    fn die_roll(&mut self, node: &Node, instr: fn(PostIdx) -> Instr) -> Build {
        let [n, a, b] = three(node)?;
        self.expr(n)?;
        self.push();
        self.binary(a, b)?;
        let p = self.post(node)?;
        self.code.emit(instr(p));
        self.depth.pop_operands(2);
        self.flags |= ProgramFlags::USES_RANDOM;
        Ok(())
    }

    /// `math.random`: literal bounds become the post-op `r·S + O` (an identity pair is the same
    /// arithmetic).
    fn random(&mut self, node: &Node) -> Build {
        let [a, b] = two(node)?;
        self.flags |= ProgramFlags::USES_RANDOM.union(ProgramFlags::USES_RANDOM_OP);
        if a.is(Op::Float) && b.is(Op::Float) {
            let p = self.post_of(math::random_const_bounds(a.float(), b.float(), node.post))?;
            self.code.emit(Instr::RandomConst { p });
            return Ok(());
        }
        self.random_with_bounds(node, a, b, |p| Instr::Random { p })
    }

    /// `math.random_integer`: literal bounds become two consecutive constants.
    fn random_integer(&mut self, node: &Node) -> Build {
        let [a, b] = two(node)?;
        self.flags |= ProgramFlags::USES_RANDOM.union(ProgramFlags::USES_RANDOM_OP);
        if a.is(Op::Float) && b.is(Op::Float) {
            let c = self.konst_pair(a.float(), b.float())?;
            let p = self.post(node)?;
            self.code.emit(Instr::RandomIntConst { c, p });
            return Ok(());
        }
        self.random_with_bounds(node, a, b, |p| Instr::RandomInt { p })
    }

    /// A random instruction that reads its bounds from the operand stack and `acc`.
    fn random_with_bounds(
        &mut self,
        node: &Node,
        a: &Node,
        b: &Node,
        instr: fn(PostIdx) -> Instr,
    ) -> Build {
        self.binary(a, b)?;
        let p = self.post(node)?;
        self.code.emit(instr(p));
        self.depth.pop_operands(1);
        Ok(())
    }
}

#[cfg(test)]
mod tests {

    use crate::compile::{
        ast::Payload,
        codegen::{LinkError, test_support::*},
        program::{ConstIdx, Divisor, Fn1, Fn3, Instr, PostIdx, ProgramFlags},
    };
    use crate::numeric::PostOp;
    use crate::ops::ExpressionOp as Op;

    /// The function of the `Math1` instruction a one-argument `op` node lowers to, if any.
    fn math1_of(op_: Op) -> Option<Fn1> {
        let program = built(&op(op_, vec![var("x")])).ok()?;
        program.code.iter().find_map(|instr| match instr {
            Instr::Math1 { f, .. } => Some(*f),
            _ => None,
        })
    }

    #[test]
    fn exactly_the_sixteen_one_argument_ops_lower_to_math1() {
        let table = [
            (Op::Abs, Fn1::Abs),
            (Op::Acos, Fn1::Acos),
            (Op::Asin, Fn1::Asin),
            (Op::Atan, Fn1::Atan),
            (Op::Ceil, Fn1::Ceil),
            (Op::Cos, Fn1::Cos),
            (Op::Exp, Fn1::Exp),
            (Op::Floor, Fn1::Floor),
            (Op::HermiteBlend, Fn1::HermiteBlend),
            (Op::Ln, Fn1::Ln),
            (Op::MinAngle, Fn1::MinAngle),
            (Op::Round, Fn1::Round),
            (Op::Sin, Fn1::Sin),
            (Op::Sign, Fn1::Sign),
            (Op::Sqrt, Fn1::Sqrt),
            (Op::Trunc, Fn1::Trunc),
        ];
        for (op_, f) in table {
            assert_eq!(math1_of(op_), Some(f), "{op_:?}");
        }
        let mapped = Op::all()
            .iter()
            .filter(|&&op_| math1_of(op_).is_some())
            .count();
        assert_eq!(mapped, 16);
        assert_eq!(math1_of(Op::Add), None);
        assert_eq!(math1_of(Op::Clamp), None);
        assert_eq!(math1_of(Op::Max), None);
    }

    /// Clamp, the three interpolations and the thirty easings.
    fn three_argument_functions() -> Vec<Op> {
        Op::all()
            .iter()
            .copied()
            .filter(|op_| {
                matches!(op_, Op::Clamp | Op::Lerp | Op::LerpRotate | Op::InverseLerp)
                    || format!("{op_:?}").starts_with("Ease")
            })
            .collect()
    }

    #[test]
    fn every_three_argument_function_lowers_to_the_math3_of_its_name() {
        let ops = three_argument_functions();
        assert_eq!(ops.len(), 34);
        for op_ in ops {
            let program = built(&op(op_, vec![var("a"), var("b"), var("c")])).unwrap();
            let Instr::Math3 { f, .. } = program.code[5] else {
                panic!("{op_:?}: {:?}", program.code[5]);
            };
            assert_eq!(format!("{f:?}"), format!("{op_:?}"));
        }
        for (op_, f) in [
            (Op::Clamp, Fn3::Clamp),
            (Op::Lerp, Fn3::Lerp),
            (Op::LerpRotate, Fn3::LerpRotate),
            (Op::InverseLerp, Fn3::InverseLerp),
            (Op::EaseInQuad, Fn3::EaseInQuad),
            (Op::EaseInOutBounce, Fn3::EaseInOutBounce),
            (Op::EaseInOutElastic, Fn3::EaseInOutElastic),
        ] {
            let program = built(&op(op_, vec![var("a"), var("b"), var("c")])).unwrap();
            assert_eq!(
                program.code[5],
                Instr::Math3 {
                    f,
                    p: PostIdx::PLAIN
                }
            );
        }
        // Max and Abs lower to the two- and one-argument instructions.
        assert_eq!(math1_of(Op::Abs), Some(Fn1::Abs));
        let max = built(&op(Op::Max, vec![var("a"), var("b")])).unwrap();
        assert!(matches!(max.code[3], Instr::Math2 { .. }));
    }

    #[test]
    fn unary_minus_folds_into_a_post_op_and_not_is_an_instruction() {
        assert_listing("-v.x", &["load variable.x *-1+0", "end"]);
        assert_listing("!v.x", &["load variable.x", "not", "end"]);
        assert_listing(
            "!v.x && !v.y",
            &[
                "load variable.x",
                "not",
                "and-step -> 6",
                "load variable.y",
                "not",
                "and-last",
                "end",
            ],
        );
    }

    #[test]
    fn negate_and_not_nodes_lower_to_their_instructions() {
        let neg = op(Op::Negate, vec![var("x")]);
        assert_built(&neg, &["load variable.x", "negate", "end"]);
        let not = with_post(op(Op::LogicalNot, vec![var("x")]), 2.0, 0.0);
        assert_built(&not, &["load variable.x", "not *2+0", "end"]);
        assert_eq!(
            built(&op(Op::Negate, vec![])).unwrap_err(),
            LinkError::Failed
        );
    }

    #[test]
    fn n_ary_addition_pushes_the_first_term_and_accumulates_the_rest() {
        assert_listing(
            "v.x + v.y + v.z",
            &[
                "load variable.x",
                "push",
                "load variable.y",
                "add-acc",
                "load variable.z",
                "add",
                "end",
            ],
        );
        assert_listing(
            "v.x + v.y + v.z + 1",
            &[
                "load variable.x",
                "push",
                "load variable.y",
                "add-acc",
                "load variable.z",
                "add *1+1",
                "end",
            ],
        );
        assert_listing(
            "v.x - v.y",
            &[
                "load variable.x",
                "push",
                "load variable.y *-1+0",
                "add",
                "end",
            ],
        );
    }

    #[test]
    fn a_sixteen_term_sum_keeps_one_stack_slot() {
        let src = (b'a'..=b'p')
            .map(|c| format!("v.{}", c as char))
            .collect::<Vec<_>>()
            .join(" + ");
        let expr = compiled(&src, 13);
        let p = expr.program().unwrap();
        assert_eq!(p.depths.stack, 1);
        // load, push, 14 x (load, add-acc), load, add, end
        assert_eq!(p.code.len(), 33);
        assert_eq!(p.code.iter().filter(|i| **i == Instr::AddAcc).count(), 14);
        assert!(matches!(p.code[p.code.len() - 2], Instr::AddLast { .. }));
    }

    #[test]
    fn a_single_term_addition_is_the_term_with_the_post_op() {
        let tree = with_post(op(Op::Add, vec![var("x")]), 2.0, 0.0);
        // A lone term loads, then applies the node's post-op as a separate instruction.
        assert_built(&tree, &["load variable.x", "post *2+0", "end"]);
        assert_eq!(built(&op(Op::Add, vec![])).unwrap_err(), LinkError::Failed);
    }

    #[test]
    fn multiplication_evaluates_the_first_operand_pushes_then_the_second() {
        assert_listing(
            "v.x * v.y",
            &["load variable.x", "push", "load variable.y", "mul", "end"],
        );
        assert_listing(
            "v.x * v.y * v.z",
            &[
                "load variable.x",
                "push",
                "load variable.y",
                "mul",
                "push",
                "load variable.z",
                "mul",
                "end",
            ],
        );
    }

    #[test]
    fn division_by_a_constant_is_a_multiplication_by_the_reciprocal() {
        // The constant is pushed with the fused form.
        assert_listing(
            "v.x / 2",
            &["load variable.x", "push; const 0.5", "mul", "end"],
        );
    }

    #[test]
    fn a_constant_factor_folds_into_the_multiplication_post_op() {
        assert_listing(
            "v.x * v.y * 2",
            &[
                "load variable.x",
                "push",
                "load variable.y",
                "mul *2+0",
                "end",
            ],
        );
    }

    #[test]
    fn division_evaluates_the_divisor_first_then_the_guard_then_the_numerator() {
        assert_listing(
            "v.a / v.b",
            &[
                "load variable.b",
                "div-guard -> 4",
                "load variable.a",
                "div",
                "end",
            ],
        );
        assert_listing(
            "3 / v.x",
            &["load variable.x", "div-guard -> 4", "const 3", "div", "end"],
        );
        assert_listing(
            "v.x / v.y * 2",
            &[
                "load variable.y",
                "div-guard -> 4",
                "load variable.x",
                "div *2+0",
                "end",
            ],
        );
        assert_listing(
            "v.a / v.b + v.c",
            &[
                "load variable.b",
                "div-guard -> 4",
                "load variable.a",
                "div",
                "push",
                "load variable.c",
                "add",
                "end",
            ],
        );
    }

    #[test]
    fn nested_divisions_guard_each_divisor_and_jump_past_their_own_div() {
        assert_listing(
            "v.x / v.y / v.z",
            &[
                "load variable.z",
                "div-guard -> 7",
                "load variable.y",
                "div-guard -> 6",
                "load variable.x",
                "div",
                "div",
                "end",
            ],
        );
        assert_listing(
            "v.a / (v.b / v.c)",
            &[
                "load variable.c",
                "div-guard -> 4",
                "load variable.b",
                "div",
                "div-guard -> 7",
                "load variable.a",
                "div",
                "end",
            ],
        );
        assert_listing(
            "(v.x + v.y) / v.z",
            &[
                "load variable.z",
                "div-guard -> 7",
                "load variable.x",
                "push",
                "load variable.y",
                "add",
                "div",
                "end",
            ],
        );
    }

    #[test]
    fn the_division_guard_form_follows_the_version() {
        for raw in -1..=13_i16 {
            let text = compiled("v.a / v.b", raw).program().unwrap().disassemble();
            let guard = if raw >= 7 {
                "div-guard -> 4"
            } else {
                "div-guard-abs -> 4"
            };
            assert_eq!(
                text,
                numbered(&["load variable.b", guard, "load variable.a", "div", "end"]),
                "version {raw}"
            );
        }
        // Versions above 13 gate like 13.
        for raw in [14, 100] {
            let text = compiled("v.a / v.b", raw).program().unwrap().disassemble();
            assert!(text.contains("div-guard -> 4"), "version {raw}: {text}");
        }
    }

    #[test]
    fn division_guard_flag_is_signed_from_version_seven() {
        for (raw, divisor) in [
            (-1, Divisor::Absolute),
            (0, Divisor::Absolute),
            (6, Divisor::Absolute),
            (7, Divisor::Signed),
            (13, Divisor::Signed),
        ] {
            let p = compiled("v.a / v.b", raw).program().unwrap().code[1];
            assert_eq!(p, Instr::DivGuard { end: 4, divisor }, "version {raw}");
        }
    }

    #[test]
    fn division_reserves_a_stack_slot_for_the_guarded_divisor() {
        assert_eq!(program_of("v.a / v.b").depths.stack, 1);
        assert_eq!(program_of("v.x / v.y / v.z").depths.stack, 2);
        assert_eq!(program_of("v.a / (v.b / v.c)").depths.stack, 1);
    }

    #[test]
    fn modulo_has_a_constant_form_and_a_run_time_form() {
        assert_listing(
            "math.mod(v.x, 3)",
            &["load variable.x", "mod-const 3", "end"],
        );
        assert_listing(
            "math.mod(v.x, -3)",
            &["load variable.x", "mod-const -3", "end"],
        );
        assert_listing(
            "math.mod(v.x, v.y)",
            &["load variable.x", "push", "load variable.y", "mod", "end"],
        );
        assert_listing(
            "math.mod(3, v.y)",
            &["const 3", "push", "load variable.y", "mod", "end"],
        );
        assert_listing(
            "math.mod(v.x, 3) * 2",
            &["load variable.x", "mod-const 3 *2+0", "end"],
        );
        assert_listing(
            "math.mod(v.x, 3) + 1",
            &["load variable.x", "mod-const 3 *1+1", "end"],
        );
    }

    #[test]
    fn modulo_node_forms_by_hand() {
        let mut constant = op(Op::Mod, vec![var("x")]);
        constant.value = Payload::Float(4.0);
        assert_built(&constant, &["load variable.x", "mod-const 4", "end"]);
        let general = op(Op::Mod, vec![var("x"), var("y")]);
        assert_built(
            &general,
            &["load variable.x", "push", "load variable.y", "mod", "end"],
        );
        assert_eq!(built(&op(Op::Mod, vec![])).unwrap_err(), LinkError::Failed);
    }

    #[test]
    fn one_argument_functions_evaluate_the_argument_then_apply() {
        assert_listing("math.abs(v.x)", &["load variable.x", "Abs", "end"]);
        assert_listing("math.sin(v.x) * 2", &["load variable.x", "Sin *2+0", "end"]);
        assert_listing(
            "math.abs(v.x) + math.abs(v.y)",
            &[
                "load variable.x",
                "Abs",
                "push",
                "load variable.y",
                "Abs",
                "add",
                "end",
            ],
        );
    }

    #[test]
    fn every_one_argument_op_lowers_to_its_function() {
        let ops = [
            (Op::Abs, "Abs"),
            (Op::Acos, "Acos"),
            (Op::Asin, "Asin"),
            (Op::Atan, "Atan"),
            (Op::Ceil, "Ceil"),
            (Op::Cos, "Cos"),
            (Op::Exp, "Exp"),
            (Op::Floor, "Floor"),
            (Op::HermiteBlend, "HermiteBlend"),
            (Op::Ln, "Ln"),
            (Op::MinAngle, "MinAngle"),
            (Op::Round, "Round"),
            (Op::Sin, "Sin"),
            (Op::Sign, "Sign"),
            (Op::Sqrt, "Sqrt"),
            (Op::Trunc, "Trunc"),
        ];
        for (op_, name) in ops {
            let tree = op(op_, vec![var("x")]);
            assert_built(&tree, &["load variable.x", name, "end"]);
            let with = with_post(op(op_, vec![var("x")]), 2.0, 3.0);
            assert_built(
                &with,
                &[
                    "load variable.x".to_owned(),
                    format!("{name} *2+3"),
                    "end".to_owned(),
                ],
            );
            assert_eq!(built(&op(op_, vec![])).unwrap_err(), LinkError::Failed);
        }
    }

    #[test]
    fn two_argument_functions_have_a_general_and_a_moved_constant_form() {
        assert_listing(
            "math.max(v.x, v.y)",
            &["load variable.x", "push", "load variable.y", "Max", "end"],
        );
        assert_listing(
            "math.atan2(v.x, v.y)",
            &["load variable.x", "push", "load variable.y", "Atan2", "end"],
        );
        assert_listing(
            "math.pow(v.x, v.y)",
            &["load variable.x", "push", "load variable.y", "Pow", "end"],
        );
        // The constant operand moved into the node: either order of the arguments.
        assert_listing(
            "math.max(v.x, 2)",
            &["load variable.x", "Max-const 2", "end"],
        );
        assert_listing(
            "math.max(2, v.x)",
            &["load variable.x", "Max-const 2", "end"],
        );
        assert_listing(
            "math.pow(v.x, 2)",
            &["load variable.x", "Pow-const 2", "end"],
        );
        assert_listing(
            "math.max(v.x, 1) + 1",
            &["load variable.x", "Max-const 1 *1+1", "end"],
        );
        assert_listing(
            "math.max(v.x, 1) * 2",
            &["load variable.x", "Max-const 1 *2+0", "end"],
        );
    }

    #[test]
    fn every_two_argument_op_lowers_to_its_function() {
        for (op_, name) in [
            (Op::Atan2, "Atan2"),
            (Op::CopySign, "CopySign"),
            (Op::Max, "Max"),
            (Op::Min, "Min"),
            (Op::Pow, "Pow"),
        ] {
            let general = op(op_, vec![var("x"), var("y")]);
            assert_built(
                &general,
                &["load variable.x", "push", "load variable.y", name, "end"],
            );
            let mut constant = op(op_, vec![var("x")]);
            constant.value = Payload::Float(1.5);
            assert_built(
                &constant,
                &[
                    "load variable.x".to_owned(),
                    format!("{name}-const 1.5"),
                    "end".to_owned(),
                ],
            );
        }
    }

    #[test]
    fn three_argument_functions_push_the_first_two_arguments() {
        assert_listing(
            "math.clamp(v.x, v.y, v.z)",
            &[
                "load variable.x",
                "push",
                "load variable.y",
                "push",
                "load variable.z",
                "Clamp",
                "end",
            ],
        );
        assert_listing(
            "math.lerp(v.a, v.b, v.c)",
            &[
                "load variable.a",
                "push",
                "load variable.b",
                "push",
                "load variable.c",
                "Lerp",
                "end",
            ],
        );
        assert_listing(
            "math.ease_in_quad(v.a, v.b, v.c)",
            &[
                "load variable.a",
                "push",
                "load variable.b",
                "push",
                "load variable.c",
                "EaseInQuad",
                "end",
            ],
        );
        assert_listing(
            "math.clamp(v.x, 0, 1) * 2",
            &[
                "load variable.x",
                "push; const 0",
                "push; const 1",
                "Clamp *2+0",
                "end",
            ],
        );
        assert_eq!(program_of("math.clamp(v.x, v.y, v.z)").depths.stack, 2);
    }

    #[test]
    fn a_push_followed_by_a_constant_is_fused() {
        assert_listing(
            "math.clamp(v.x, 0, 1)",
            &[
                "load variable.x",
                "push; const 0",
                "push; const 1",
                "Clamp",
                "end",
            ],
        );
        let p = program_of("math.clamp(v.x, 0, 1)");
        assert_eq!(p.code.len(), 5);
        assert_eq!(p.code[1], Instr::PushConst { c: ConstIdx(0) });
        assert_eq!(p.code[2], Instr::PushConst { c: ConstIdx(1) });
        // The first operand being a constant is not preceded by a push, so not fused.
        assert_listing(
            "math.lerp(1, 2, v.x)",
            &[
                "const 1",
                "push; const 2",
                "push",
                "load variable.x",
                "Lerp",
                "end",
            ],
        );
        assert_listing(
            "math.lerp(v.x, 2, 3)",
            &[
                "load variable.x",
                "push; const 2",
                "push; const 3",
                "Lerp",
                "end",
            ],
        );
    }

    #[test]
    fn every_easing_op_lowers_through_the_three_argument_arm() {
        for op_ in three_argument_functions() {
            let tree = op(op_, vec![var("a"), var("b"), var("c")]);
            assert_built(
                &tree,
                &[
                    "load variable.a".to_owned(),
                    "push".to_owned(),
                    "load variable.b".to_owned(),
                    "push".to_owned(),
                    "load variable.c".to_owned(),
                    format!("{op_:?}"),
                    "end".to_owned(),
                ],
            );
            assert_eq!(
                built(&op(op_, vec![var("a"), var("b")])).unwrap_err(),
                LinkError::Failed
            );
        }
    }

    #[test]
    fn random_with_literal_bounds_folds_them_into_a_post_op() {
        assert_listing("math.random(1, 3)", &["random-const *2+1", "end"]);
        let p = program_of("math.random(1, 3)");
        assert_eq!(*p.posts, [PostOp::IDENTITY, PostOp::new(2.0, 1.0)]);
        assert_listing("math.random(2, 5) * 3", &["random-const *9+6", "end"]);
        assert_listing("math.random(2, 5) + 1", &["random-const *3+3", "end"]);
        assert_listing(
            "math.random(1, 3) + math.random(1, 3)",
            &[
                "random-const *2+1",
                "push",
                "random-const *2+1",
                "add",
                "end",
            ],
        );
        // The two equal folded post-ops share one pool entry.
        assert_eq!(
            program_of("math.random(1, 3) + math.random(1, 3)")
                .posts
                .len(),
            2
        );
    }

    #[test]
    fn random_with_run_time_bounds_pushes_them() {
        assert_listing(
            "math.random(v.lo, v.hi)",
            &[
                "load variable.lo",
                "push",
                "load variable.hi",
                "random",
                "end",
            ],
        );
        assert_listing(
            "math.random(v.a, 3)",
            &["load variable.a", "push; const 3", "random", "end"],
        );
        assert_listing(
            "math.random(2, v.hi)",
            &["const 2", "push", "load variable.hi", "random", "end"],
        );
    }

    #[test]
    fn random_integer_with_literal_bounds_takes_two_consecutive_constants() {
        assert_listing(
            "math.random_integer(2, 5)",
            &["random-integer-const 2 5", "end"],
        );
        assert_listing(
            "math.random_integer(2, 5) * 3",
            &["random-integer-const 2 5 *3+0", "end"],
        );
        assert_listing(
            "math.random_integer(2, 5) + 1",
            &["random-integer-const 2 5 *1+1", "end"],
        );
        let p = program_of("math.random_integer(1, 6) + math.random_integer(1, 6)");
        // The pair is never pooled: two sites, four constants.
        assert_eq!(*p.consts, [1.0, 6.0, 1.0, 6.0]);
        assert_eq!(
            p.code[0],
            Instr::RandomIntConst {
                c: ConstIdx(0),
                p: PostIdx::PLAIN
            }
        );
        assert_eq!(
            p.code[2],
            Instr::RandomIntConst {
                c: ConstIdx(2),
                p: PostIdx::PLAIN
            }
        );
    }

    #[test]
    fn random_integer_with_run_time_bounds_pushes_them() {
        assert_listing(
            "math.random_integer(v.a, 6)",
            &["load variable.a", "push; const 6", "random-integer", "end"],
        );
        assert_listing(
            "math.random_integer(v.a, 5) + 1",
            &[
                "load variable.a",
                "push; const 5",
                "random-integer *1+1",
                "end",
            ],
        );
    }

    #[test]
    fn random_sets_the_random_flags() {
        let both = ProgramFlags::FLOAT_ONLY
            .union(ProgramFlags::USES_RANDOM)
            .union(ProgramFlags::USES_RANDOM_OP);
        for src in ["math.random(1, 3)", "math.random_integer(1, 6)"] {
            assert_eq!(program_of(src).flags, both, "{src}");
        }
        let with_var = both.union(ProgramFlags::READS_ACTOR_VARS);
        for src in ["math.random(v.a, 3)", "math.random_integer(v.a, 6)"] {
            assert_eq!(program_of(src).flags, with_var, "{src}");
        }
    }

    #[test]
    fn die_rolls_push_the_count_and_both_bounds() {
        assert_listing(
            "math.die_roll(3, 1, 6)",
            &[
                "const 3",
                "push; const 1",
                "push; const 6",
                "die-roll",
                "end",
            ],
        );
        assert_listing(
            "math.die_roll_integer(v.n, 1, 6)",
            &[
                "load variable.n",
                "push; const 1",
                "push; const 6",
                "die-roll-integer",
                "end",
            ],
        );
        assert_listing(
            "math.die_roll(2, v.lo, 6)",
            &[
                "const 2",
                "push",
                "load variable.lo",
                "push; const 6",
                "die-roll",
                "end",
            ],
        );
        assert_listing(
            "math.die_roll(v.n, 1, 6) + math.die_roll(v.n, 1, 6)",
            &[
                "load variable.n",
                "push; const 1",
                "push; const 6",
                "die-roll *2+0",
                "end",
            ],
        );
    }

    #[test]
    fn die_rolls_are_random_but_not_a_random_op() {
        let p = program_of("math.die_roll(3, 1, 6)");
        assert_eq!(
            p.flags,
            ProgramFlags::FLOAT_ONLY.union(ProgramFlags::USES_RANDOM)
        );
        assert!(!p.flags.contains(ProgramFlags::USES_RANDOM_OP));
        assert_eq!(p.depths.stack, 2);
        assert_eq!(program_of("math.die_roll_integer(3, 1, 6)").flags, p.flags);
    }
}
