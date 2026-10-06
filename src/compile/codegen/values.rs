//! Constants, strings, resources, `this`, loads, member reads, assignments and member stores.

use super::{Build, Builder, LinkError, first_child, payload_f32};
use crate::compile::{
    ast::{Node, Payload},
    program::{Instr, MemberStore, ProgramFlags, StoreIdx, StoreRoot},
};
use crate::diag::LanguageMessage as Msg;
use crate::hash::HashedStr;
use crate::ops::ExpressionOp as Op;

impl Builder<'_, '_> {
    #[inline(never)]
    pub(super) fn float(&mut self, node: &Node) -> Build {
        // The payload is loaded without the post-op.
        let c = self.konst(node.float())?;
        self.code.emit(Instr::Const { c });
        Ok(())
    }

    #[inline(never)]
    pub(super) fn string_literal(&mut self, node: &Node) -> Build {
        let hash = match node.value {
            Payload::Hash(h) => h,
            _ => 0,
        };
        let h = self.hash(hash)?;
        self.code.emit(Instr::Hash { h });
        self.not_float_only();
        Ok(())
    }

    #[inline(never)]
    pub(super) fn resource(&mut self, node: &Node) -> Build {
        let hash = match &node.value {
            Payload::Geometry(n) | Payload::Material(n) | Payload::Texture(n) => n.hash().as_u64(),
            _ => 0,
        };
        let h = self.hash(hash)?;
        self.code.emit(Instr::Resource { h });
        self.not_float_only();
        Ok(())
    }

    #[inline(never)]
    pub(super) fn this(&mut self, node: &Node) -> Build {
        let p = self.post(node)?;
        self.code.emit(Instr::This { p });
        Ok(())
    }

    /// A member read path (`v.a.b.c`): the base, then one `Member` per member, innermost first. A
    /// missing member is reported by the path's last member, so the whole chain is built here.
    #[inline(never)]
    pub(super) fn member_read(&mut self, node: &Node) -> Build {
        let Payload::Member(last) = &node.value else {
            return Err(LinkError::Failed);
        };
        let mut chain = vec![node];
        let mut base = first_child(node)?;
        while base.is(Op::MemberAccessor) {
            chain.push(base);
            base = first_child(base)?;
        }
        self.expr(base)?;
        for accessor in chain.into_iter().rev() {
            let Payload::Member(name) = &accessor.value else {
                return Err(LinkError::Failed);
            };
            let m = self.member(name, last)?;
            let p = self.post(accessor)?;
            self.code.emit(Instr::Member { m, p });
        }
        self.not_float_only();
        self.flags |= ProgramFlags::USES_MEMBERS;
        Ok(())
    }

    pub(super) fn load(&mut self, node: &Node) -> Build {
        let p = self.post(node)?;
        match &node.value {
            Payload::Entity(name) => {
                let n = self.name(name)?;
                self.code.emit(Instr::LoadVar { n, p });
                self.flags |= ProgramFlags::READS_ACTOR_VARS;
            }
            Payload::Temp(name) => {
                let t = self.temp(name)?;
                self.code.emit(Instr::LoadTemp { t, p });
            }
            Payload::Context(name) => {
                let n = self.name(name)?;
                self.code.emit(Instr::LoadCtx { n, p });
                self.flags |= ProgramFlags::READS_CONTEXT;
            }
            _ => return Err(LinkError::Failed),
        }
        Ok(())
    }

    #[inline(never)]
    pub(super) fn generic_constant(&mut self, node: &Node) -> Build {
        let c = self.konst(node.post.apply(payload_f32(node)))?;
        self.code.emit(Instr::Const { c });
        Ok(())
    }

    pub(super) fn assignment(&mut self, node: &Node) -> Build {
        let target = first_child(node)?;
        // The link-stage check of the target, before anything below is built.
        match target.op {
            Op::EntityVariable | Op::MemberAccessor => {}
            Op::Pointer => {
                self.cx
                    .language(Msg::WriteToOtherMob, target.full_span(), &[]);
                return Err(LinkError::Failed);
            }
            _ if matches!(target.value, Payload::Temp(_)) => {}
            // Any other target fails the link; the validator has already logged it.
            _ => return Err(LinkError::Failed),
        }
        self.flags |= ProgramFlags::HAS_ASSIGNMENT;
        if let Some(value) = node.children.get(1) {
            self.expr(value)?;
        } else {
            let c = self.konst(payload_f32(node))?;
            self.code.emit(Instr::Const { c });
        }
        let p = self.post(node)?;
        match (&target.value, target.op) {
            (Payload::Entity(name), _) => {
                let n = self.name(name)?;
                self.code.emit(Instr::StoreVar { n, p });
            }
            (Payload::Temp(name), _) => {
                let t = self.temp(name)?;
                self.code.emit(Instr::StoreTemp { t, p });
            }
            (_, Op::MemberAccessor) => {
                let store = self.member_store(target)?;
                let s = u16::try_from(self.stores.len()).map_err(|_| LinkError::Failed)?;
                let s = StoreIdx(s);
                self.stores.push(store);
                self.code.emit(Instr::StoreMember { s, p });
                self.not_float_only();
                self.flags |= ProgramFlags::USES_MEMBERS;
            }
            _ => return Err(LinkError::Failed),
        }
        Ok(())
    }

    /// The root variable and member path of `v.a.b.c` (`MemberAccessor(c, MemberAccessor(b,
    /// v.a))`).
    fn member_store(&mut self, target: &Node) -> Result<MemberStore, LinkError> {
        let mut path: Vec<HashedStr> = Vec::new();
        let mut node = target;
        while node.is(Op::MemberAccessor) {
            let Payload::Member(name) = &node.value else {
                return Err(LinkError::Failed);
            };
            path.push(name.hash());
            node = first_child(node)?;
        }
        path.reverse();
        let root = match &node.value {
            Payload::Entity(name) => StoreRoot::Var(self.name(name)?),
            Payload::Temp(name) => StoreRoot::Temp(self.temp(name)?),
            _ => StoreRoot::Other,
        };
        Ok(MemberStore {
            root,
            path: path.into_boxed_slice(),
        })
    }
}

#[cfg(test)]
mod tests {
    use crate::compile::{
        CompileFailure, Cx,
        ast::{Name, Payload, Span},
        codegen::{LinkError, build, test_support::*},
        compile,
        program::{HashIdx, Instr, MemberIdx, NameIdx, PostIdx, ProgramFlags, StoreRoot, TempIdx},
    };
    use crate::diag::LanguageMessage as Msg;
    use crate::hash::HashedStr;
    use crate::ops::ExpressionOp as Op;

    #[test]
    fn a_folded_constant_is_one_const_and_the_end() {
        assert_listing("1 + 2", &["const 3", "end"]);
        assert_listing("0", &["const 0", "end"]);
        assert_listing("-2.5", &["const -2.5", "end"]);
    }

    #[test]
    fn a_string_is_a_hash_instruction() {
        assert_listing("'moo'", &[format!("hash {}", hex("moo")), "end".to_owned()]);
        let p = program_of("'moo'");
        assert_eq!(*p.hashes, [HashedStr::new("moo").as_u64()]);
        assert!(!p.flags.contains(ProgramFlags::FLOAT_ONLY));
        assert!(!p.float_loop);
    }

    #[test]
    fn string_literals_keep_their_case_when_hashed() {
        assert_eq!(
            lines("'MOO'"),
            [format!("hash {}", hex("MOO")), "end".to_owned()]
        );
        assert_ne!(lines("'MOO'"), lines("'moo'"));
    }

    #[test]
    fn this_loads_with_its_post_op() {
        assert_listing("this", &["this", "end"]);
        assert_listing("this * 2", &["this *2+0", "end"]);
        assert_listing("this + 1", &["this *1+1", "end"]);
    }

    #[test]
    fn variable_temp_and_context_loads() {
        assert_listing("v.x", &["load variable.x", "end"]);
        assert_listing("variable.x", &["load variable.x", "end"]);
        assert_listing("c.foo", &["load context.foo", "end"]);
        assert_listing(
            "t.a = 1; return t.a;",
            &[
                "const 1",
                "store temp.a",
                "load temp.a",
                "return",
                "const 0",
                "end",
            ],
        );
    }

    #[test]
    fn loads_carry_a_folded_post_op() {
        assert_listing("v.x * 2", &["load variable.x *2+0", "end"]);
        assert_listing("v.x + 1", &["load variable.x *1+1", "end"]);
        assert_listing("v.x * 2 + 1", &["load variable.x *2+1", "end"]);
        assert_listing("-v.x", &["load variable.x *-1+0", "end"]);
        assert_listing("c.foo * 2", &["load context.foo *2+0", "end"]);
    }

    #[test]
    fn names_are_lower_cased_and_shared_between_reads_and_writes() {
        assert_eq!(lines("V.X"), lines("v.x"));
        let p = program_of("v.a = v.b; v.a = v.a;");
        assert_eq!(p.names.len(), 2);
        assert_eq!(&*p.names[0].text, "variable.b");
        assert_eq!(&*p.names[1].text, "variable.a");
    }

    #[test]
    fn member_reads_emit_the_base_then_one_member_per_step() {
        assert_listing("v.s.b", &["load variable.s", "member .b", "end"]);
        assert_listing(
            "v.s.b.c",
            &["load variable.s", "member .b", "member .c", "end"],
        );
        assert_listing("v.s.b * 2", &["load variable.s", "member .b *2+0", "end"]);
        assert_listing(
            "v.s.b + v.t.b",
            &[
                "load variable.s",
                "member .b",
                "push",
                "load variable.t",
                "member .b",
                "add",
                "end",
            ],
        );
    }

    #[test]
    fn a_missing_member_is_reported_by_the_last_member_of_the_path() {
        let p = program_of("v.s.b.b.c");
        // Three accessors of the one path: `.b` twice and `.c`, all reporting `.c`.
        let texts: Vec<_> = p.members.iter().map(|m| (&*m.text, &*m.report)).collect();
        assert_eq!(texts, [(".b", ".c"), (".c", ".c")]);
        assert_eq!(p.code.len(), 5);
        assert_eq!(
            p.code[1],
            Instr::Member {
                m: MemberIdx(0),
                p: PostIdx::PLAIN
            }
        );
        assert_eq!(
            p.code[2],
            Instr::Member {
                m: MemberIdx(0),
                p: PostIdx::PLAIN
            }
        );
        assert_eq!(
            p.code[3],
            Instr::Member {
                m: MemberIdx(1),
                p: PostIdx::PLAIN
            }
        );
    }

    #[test]
    fn the_same_member_in_paths_with_different_last_members_has_its_own_entry() {
        let p = program_of("v.s.b + v.s.b.c");
        let texts: Vec<_> = p.members.iter().map(|m| (&*m.text, &*m.report)).collect();
        assert_eq!(texts, [(".b", ".b"), (".b", ".c"), (".c", ".c")]);
    }

    #[test]
    fn member_reads_set_the_member_flag_and_leave_the_float_loop() {
        let p = program_of("v.s.b");
        assert_eq!(
            p.flags,
            ProgramFlags::READS_ACTOR_VARS.union(ProgramFlags::USES_MEMBERS)
        );
        assert!(!p.float_loop);
    }

    #[test]
    fn assignment_to_a_variable_loads_the_value_then_stores_it() {
        assert_listing(
            "v.a = 1;",
            &["const 1", "store variable.a", "const 0", "end"],
        );
        assert_listing(
            "v.a = v.b + 1;",
            &["load variable.b *1+1", "store variable.a", "const 0", "end"],
        );
        assert_listing(
            "v.x = 5; v.y = v.x;",
            &[
                "const 5",
                "store variable.x",
                "load variable.x",
                "store variable.y",
                "const 0",
                "end",
            ],
        );
    }

    #[test]
    fn assignment_to_a_temp_uses_a_slot() {
        assert_listing("t.a = 1;", &["const 1", "store temp.a", "const 0", "end"]);
        let p = program_of("t.a = 1; t.b = 2; t.a = 3;");
        assert_eq!(p.temps.len(), 2);
        assert_eq!(
            p.code[1],
            Instr::StoreTemp {
                t: TempIdx(0),
                p: PostIdx::PLAIN
            }
        );
        assert_eq!(
            p.code[3],
            Instr::StoreTemp {
                t: TempIdx(1),
                p: PostIdx::PLAIN
            }
        );
        assert_eq!(
            p.code[5],
            Instr::StoreTemp {
                t: TempIdx(0),
                p: PostIdx::PLAIN
            }
        );
    }

    #[test]
    fn assignments_set_the_assignment_flag_and_keep_the_float_loop() {
        let p = program_of("v.a = 1;");
        assert_eq!(
            p.flags,
            ProgramFlags::FLOAT_ONLY.union(ProgramFlags::HAS_ASSIGNMENT)
        );
        assert!(p.float_loop);
    }

    #[test]
    fn member_assignment_stores_through_a_path() {
        assert_listing(
            "v.s.a.b = 1;",
            &["const 1", "store-member #0", "const 0", "end"],
        );
        let p = program_of("v.s.a.b = 1;");
        assert_eq!(p.stores.len(), 1);
        assert_eq!(p.stores[0].root, StoreRoot::Var(NameIdx(0)));
        assert_eq!(&*p.names[0].text, "variable.s");
        assert_eq!(
            *p.stores[0].path,
            [HashedStr::new("a"), HashedStr::new("b")]
        );
        assert_eq!(
            p.flags,
            ProgramFlags::HAS_ASSIGNMENT.union(ProgramFlags::USES_MEMBERS)
        );
        assert!(!p.float_loop);
    }

    #[test]
    fn member_assignment_roots() {
        let var = program_of("v.s.a = 1;");
        assert_eq!(var.stores[0].root, StoreRoot::Var(NameIdx(0)));
        let temp = program_of("t.s.a = 1;");
        assert_eq!(temp.stores[0].root, StoreRoot::Temp(TempIdx(0)));
        assert!(temp.flags.contains(ProgramFlags::USES_TEMPS));
        assert_eq!(&*temp.names[0].text, "temp.s");
        // A context base is kept by the validator and builds as a no-op target.
        let other = program_of("c.s.a = 1;");
        assert_eq!(other.stores[0].root, StoreRoot::Other);
        assert_eq!(*other.stores[0].path, [HashedStr::new("a")]);
    }

    #[test]
    fn every_member_assignment_has_its_own_store_entry() {
        let p = program_of("v.s.a = 1; v.s.a = 2; v.s.b = v.x;");
        assert_eq!(p.stores.len(), 3);
        let slots: Vec<u16> = p
            .code
            .iter()
            .filter_map(|i| {
                if let Instr::StoreMember { s, .. } = i {
                    Some(s.0)
                } else {
                    None
                }
            })
            .collect();
        assert_eq!(slots, [0, 1, 2]);
    }

    #[test]
    fn assignment_of_a_handler_value() {
        assert_listing(
            "v.a = (v.b ?? 1);",
            &[
                "handler-push -> 3",
                "load variable.b",
                "handler-pop -> 4",
                "const 1",
                "store variable.a",
                "const 0",
                "end",
            ],
        );
    }

    #[test]
    fn assignment_with_a_constant_moved_into_the_node() {
        // The optimiser can leave the assigned constant in the assignment's own payload.
        let mut assign = op(Op::Assignment, vec![var("a")]);
        assign.value = Payload::Float(5.0);
        let tree = op(Op::Semicolon, vec![assign]);
        assert_built(&tree, &["const 5", "store variable.a", "const 0", "end"]);
    }

    #[test]
    fn assignment_post_op_applies_after_the_store_value() {
        let tree = op(
            Op::Semicolon,
            vec![with_post(
                op(Op::Assignment, vec![var("a"), float(1.0)]),
                2.0,
                3.0,
            )],
        );
        assert_built(
            &tree,
            &["const 1", "store variable.a *2+3", "const 0", "end"],
        );
    }

    #[test]
    fn ops_without_an_instruction_take_the_generic_constant_path() {
        for op_ in [
            Op::Pi,
            Op::RightBrace,
            Op::RightBracket,
            Op::RightParenthesis,
            Op::ConditionalElse,
            Op::Comma,
            Op::LeftBrace,
            Op::Return,
        ] {
            // The payload float with the post-op applied at build time.
            let n = with_post(node(op_, Payload::Float(3.0), vec![]), 2.0, 1.0);
            assert_built(&n, &["const 7", "end"]);
            // A hash payload reads as its low 32 bits.
            let h = node(op_, Payload::Hash(0x0000_0001_4000_0000), vec![]);
            assert_built(&h, &["const 2", "end"]);
            // Anything else is 0.
            let none = with_post(op(op_, vec![]), 2.0, 1.0);
            assert_built(&none, &["const 1", "end"]);
        }
    }

    #[test]
    fn resolved_resource_nodes_lower_to_a_resource_instruction() {
        let name = Name::new("geometry.default");
        let tree = node(Op::Geometry, Payload::Geometry(name.clone()), vec![]);
        let p = built(&tree).unwrap();
        assert_eq!(p.code[0], Instr::Resource { h: HashIdx(0) });
        assert_eq!(*p.hashes, [name.hash().as_u64()]);
        assert!(!p.float_loop && !p.flags.contains(ProgramFlags::FLOAT_ONLY));
        for (op_, payload) in [
            (Op::Material, Payload::Material(Name::new("material.m"))),
            (Op::Texture, Payload::Texture(Name::new("texture.t"))),
        ] {
            let p = built(&node(op_, payload, vec![])).unwrap();
            assert_eq!(
                p.disassemble(),
                numbered(&[
                    format!(
                        "resource {}",
                        hex(match op_ {
                            Op::Material => "material.m",
                            _ => "texture.t",
                        })
                    ),
                    "end".to_owned()
                ])
            );
        }
        // A resource node without its payload hashes 0.
        assert_eq!(*built(&op(Op::Geometry, vec![])).unwrap().hashes, [0]);
    }

    #[test]
    fn a_hash_node_without_a_hash_payload_loads_hash_zero() {
        let p = built(&op(Op::StringLiteral, vec![])).unwrap();
        assert_eq!(*p.hashes, [0]);
    }

    #[test]
    fn a_write_to_another_mob_logs_e49_and_fails_the_link() {
        let c = compile("v.p->v.x = 1; return 1;", &opts(13));
        assert_eq!(c.failure(), Some(CompileFailure::Rejected));
        let messages: Vec<String> = c
            .diagnostics()
            .iter()
            .map(|d| d.message().into_owned())
            .collect();
        assert_eq!(
            messages,
            [
                "Error: Assignment attempted on Pointer result. Writing to another entity's public variables is not supported, only reading them.",
                "Error: You cannot write to a variable on another mob.",
                "expression 'v.p->v.x = 1; return 1;' compile failed",
            ]
        );
        let language: Vec<_> = c
            .diagnostics()
            .iter()
            .map(crate::diag::Diagnostic::language_message)
            .collect();
        assert_eq!(language[1], Some(Msg::WriteToOtherMob));
        assert_eq!(language[2], Some(Msg::CompileFailed));
        assert_eq!(c.expr_or_zero().cloned().unwrap().as_constant(), Some(0.0));
    }

    #[test]
    fn a_pointer_target_assignment_by_hand_logs_the_mob_message_at_the_target() {
        let mut target = op(Op::Pointer, vec![var("p"), var("x")]);
        target.span = Span::new(4, 6);
        target.children[0].span = Span::new(3, 4);
        target.children[1].span = Span::new(6, 8);
        let tree = op(Op::Assignment, vec![target, float(1.0)]);
        let options = opts(13);
        let mut cx = Cx::for_test("", &options);
        assert_eq!(build(&mut cx, &tree).unwrap_err(), LinkError::Failed);
        let logged = cx.logged_diagnostics();
        assert_eq!(logged.len(), 1);
        assert_eq!(logged[0].language_message(), Some(Msg::WriteToOtherMob));
        assert_eq!(
            logged[0].message(),
            "Error: You cannot write to a variable on another mob."
        );
        assert_eq!(logged[0].span(), 3..8);
    }

    #[test]
    fn assignment_targets_are_checked_before_anything_is_built() {
        // The value would fail with UsesArrays, but the target check comes first.
        let tree = op(
            Op::Assignment,
            vec![float(1.0), op(Op::ArrayVariable, vec![])],
        );
        assert_eq!(built(&tree).unwrap_err(), LinkError::Failed);
        // A valid target lets the value's failure through.
        let tree = op(
            Op::Assignment,
            vec![var("a"), op(Op::ArrayVariable, vec![])],
        );
        assert_eq!(built(&tree).unwrap_err(), LinkError::UsesArrays);
        assert_eq!(
            built(&op(Op::Assignment, vec![])).unwrap_err(),
            LinkError::Failed
        );
    }

    #[test]
    fn a_member_node_without_its_payload_fails() {
        let access = op(Op::MemberAccessor, vec![var("s")]);
        assert_eq!(built(&access).unwrap_err(), LinkError::Failed);
        let childless = node(Op::MemberAccessor, Payload::Member(Name::new("b")), vec![]);
        assert_eq!(built(&childless).unwrap_err(), LinkError::Failed);
        let ok = node(
            Op::MemberAccessor,
            Payload::Member(Name::new("b")),
            vec![var("s")],
        );
        assert_built(&ok, &["load variable.s", "member .b", "end"]);
    }

    #[test]
    fn a_load_node_with_the_wrong_payload_fails() {
        assert_eq!(
            built(&op(Op::EntityVariable, vec![])).unwrap_err(),
            LinkError::Failed
        );
        assert_eq!(
            built(&op(Op::TempVariable, vec![])).unwrap_err(),
            LinkError::Failed
        );
        assert_eq!(
            built(&op(Op::ContextVariable, vec![])).unwrap_err(),
            LinkError::Failed
        );
        assert_eq!(
            built(&op(Op::QueryFunction, vec![])).unwrap_err(),
            LinkError::Failed
        );
    }

    #[test]
    fn a_hand_built_context_load_and_temp_load() {
        let ctx = node(
            Op::ContextVariable,
            Payload::Context(Name::new("context.c")),
            vec![],
        );
        assert_built(&ctx, &["load context.c", "end"]);
        assert_built(&temp("t"), &["load temp.t", "end"]);
        let p = built(&temp("t")).unwrap();
        assert!(p.flags.contains(ProgramFlags::USES_TEMPS));
    }

    #[test]
    fn a_context_assignment_target_fails_the_link_without_the_mob_message() {
        let target = node(
            Op::ContextVariable,
            Payload::Context(Name::new("context.c")),
            vec![],
        );
        let tree = op(Op::Assignment, vec![target, float(1.0)]);
        let options = opts(13);
        let mut cx = Cx::for_test("", &options);
        assert_eq!(build(&mut cx, &tree).unwrap_err(), LinkError::Failed);
        assert!(cx.logged_diagnostics().is_empty());
    }

    #[test]
    fn a_parsed_context_assignment_logs_only_the_compile_failed_message_up_to_the_first_nul() {
        let c = compile("return c.x = 1;\0 trailing bytes", &opts(13));
        assert_eq!(c.failure(), Some(CompileFailure::Rejected));
        let last = c.diagnostics().last().unwrap();
        assert_eq!(last.language_message(), Some(Msg::CompileFailed));
        assert_eq!(
            last.message(),
            "expression 'return c.x = 1;' compile failed"
        );
        assert!(
            c.diagnostics()
                .iter()
                .all(|d| d.language_message() != Some(Msg::WriteToOtherMob))
        );
        assert_eq!(c.expr_or_zero().cloned().unwrap().as_constant(), Some(0.0));
    }
}
