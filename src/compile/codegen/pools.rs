//! The interned tables of a program: constants, post-ops, hashes, names, temps and member entries.
//! Each lookup is by key, so interning stays linear in the program's size.

use super::{Builder, LinkError};
use crate::compile::{
    ast::{Name, Node},
    program::{
        ConstIdx, HashIdx, MemberEntry, MemberIdx, NameEntry, NameIdx, PostIdx, ProgramFlags,
        TempIdx,
    },
};
use crate::numeric::PostOp;
use std::collections::HashMap;
use std::hash::Hash;

/// The most keys an [`Index`] keeps in its inline list.
const SCAN: usize = 16;

/// The interned keys of a table: a search of an inline list up to [`SCAN`] keys (no allocation or
/// hashing for the usual handful), a hash map beyond, so a long program stays linear.
struct Index<K> {
    few: [(K, u32); SCAN],
    len: usize,
    many: Option<HashMap<K, u32>>,
}

impl<K: Copy + Default> Default for Index<K> {
    fn default() -> Self {
        Self {
            few: [(K::default(), 0); SCAN],
            len: 0,
            many: None,
        }
    }
}

impl<K: Copy + Eq + Hash> Index<K> {
    fn get(&self, key: K) -> Option<u32> {
        match &self.many {
            None => self.few[..self.len]
                .iter()
                .find(|&&(k, _)| k == key)
                .map(|&(_, at)| at),
            Some(many) => many.get(&key).copied(),
        }
    }

    /// Records where `key`, which is not in the table yet, is.
    fn insert(&mut self, key: K, at: u32) {
        match &mut self.many {
            None if self.len < SCAN => {
                self.few[self.len] = (key, at);
                self.len += 1;
            }
            None => {
                let mut many: HashMap<K, u32> = self.few.iter().copied().collect();
                many.insert(key, at);
                self.many = Some(many);
            }
            Some(many) => {
                many.insert(key, at);
            }
        }
    }
}

/// An interned table: its entries, and where the entry of each key is.
pub(super) struct Pool<K, T> {
    items: Vec<T>,
    index: Index<K>,
}

impl<K: Copy + Default, T> Default for Pool<K, T> {
    fn default() -> Self {
        Self {
            items: Vec::new(),
            index: Index::default(),
        }
    }
}

impl<K: Copy + Default, T> Pool<K, T> {
    /// A pool whose entry 0 is `first`, without a key.
    pub(super) fn starting_with(first: T) -> Self {
        Self {
            items: vec![first],
            index: Index::default(),
        }
    }
}

impl<K: Copy + Eq + Hash, T> Pool<K, T> {
    /// The position of the entry of `key`, appending `make()` as that entry if there is none.
    fn intern(&mut self, key: K, make: impl FnOnce() -> T) -> Result<u32, LinkError> {
        if let Some(at) = self.index.get(key) {
            return Ok(at);
        }
        let at = self.push(make())?;
        self.index.insert(key, at);
        Ok(at)
    }

    /// Appends `item` without a key: no lookup finds it.
    pub(super) fn push(&mut self, item: T) -> Result<u32, LinkError> {
        let at = u32::try_from(self.items.len()).map_err(|_| LinkError::Failed)?;
        self.items.push(item);
        Ok(at)
    }

    pub(super) fn len(&self) -> usize {
        self.items.len()
    }

    pub(super) fn into_boxed_slice(self) -> Box<[T]> {
        self.items.into_boxed_slice()
    }
}

/// A position in a table that `u16` operands index.
fn narrow(at: u32) -> Result<u16, LinkError> {
    u16::try_from(at).map_err(|_| LinkError::Failed)
}

impl Builder<'_, '_> {
    pub(super) fn konst(&mut self, value: f32) -> Result<ConstIdx, LinkError> {
        self.consts.intern(value.to_bits(), || value).map(ConstIdx)
    }

    /// Two consecutive constants (the literal bounds of `math.random_integer`).
    #[cfg(feature = "stdlib")]
    pub(super) fn konst_pair(&mut self, a: f32, b: f32) -> Result<ConstIdx, LinkError> {
        let c = self.consts.push(a)?;
        self.consts.push(b)?;
        Ok(ConstIdx(c))
    }

    pub(super) fn post_of(&mut self, post: PostOp) -> Result<PostIdx, LinkError> {
        if post.is_identity() {
            return Ok(PostIdx::PLAIN);
        }
        let key = (post.scale.to_bits(), post.offset.to_bits());
        self.posts
            .intern(key, || post)
            .and_then(narrow)
            .map(PostIdx)
    }

    pub(super) fn post(&mut self, node: &Node) -> Result<PostIdx, LinkError> {
        self.post_of(node.post)
    }

    pub(super) fn hash(&mut self, hash: u64) -> Result<HashIdx, LinkError> {
        self.hashes.intern(hash, || hash).map(HashIdx)
    }

    pub(super) fn name(&mut self, name: &Name) -> Result<NameIdx, LinkError> {
        let hash = name.hash();
        let entry = || NameEntry {
            hash,
            text: name.as_str().into(),
        };
        self.names
            .intern(hash.as_u64(), entry)
            .and_then(narrow)
            .map(NameIdx)
    }

    /// The temp slot of a `temp.` name.
    pub(super) fn temp(&mut self, name: &Name) -> Result<TempIdx, LinkError> {
        let n = self.name(name)?;
        self.flags |= ProgramFlags::USES_TEMPS;
        self.temps.intern(n.0, || n).and_then(narrow).map(TempIdx)
    }

    /// The member entry of `name` in a read path whose last member is `last`.
    pub(super) fn member(&mut self, name: &Name, last: &Name) -> Result<MemberIdx, LinkError> {
        let key = (name.hash().as_u64(), last.hash().as_u64());
        let entry = || MemberEntry {
            hash: name.hash(),
            text: format!(".{}", name.as_str()).into(),
            report: format!(".{}", last.as_str()).into(),
        };
        self.members
            .intern(key, entry)
            .and_then(narrow)
            .map(MemberIdx)
    }
}

#[cfg(test)]
mod tests {
    use crate::compile::{
        ast::Name,
        codegen::test_support::*,
        program::{Instr, NameIdx, PostIdx, ProgramFlags},
    };
    use crate::hash::HashedStr;
    use crate::numeric::PostOp;

    #[test]
    fn an_index_finds_what_was_inserted_before_and_after_it_grows_a_map() {
        let mut index = super::Index::<u64>::default();
        assert_eq!(index.get(7), None);
        for key in 0..100u64 {
            index.insert(key * 3, u32::try_from(key).unwrap() + 1000);
            assert_eq!(
                index.many.is_none(),
                key < 16,
                "the list holds sixteen keys"
            );
            for earlier in 0..=key {
                assert_eq!(
                    index.get(earlier * 3),
                    Some(u32::try_from(earlier).unwrap() + 1000),
                    "{earlier} after {key}"
                );
            }
            assert_eq!(index.get(key * 3 + 1), None);
        }
        assert_eq!(
            index.many.as_ref().map(std::collections::HashMap::len),
            Some(100)
        );
    }

    #[test]
    fn an_empty_index_finds_not_even_the_default_key() {
        let index = super::Index::<u32>::default();
        assert_eq!(index.get(0), None);
    }

    #[test]
    fn an_index_compares_whole_keys() {
        let mut index = super::Index::<(u32, u32)>::default();
        index.insert((1, 2), 0);
        index.insert((2, 1), 1);
        assert_eq!(index.get((1, 2)), Some(0));
        assert_eq!(index.get((2, 1)), Some(1));
        assert_eq!(index.get((1, 1)), None);
    }

    #[test]
    fn konst_pools_by_bit_pattern() {
        builder!(b);
        assert_eq!(b.konst(1.0).unwrap().0, 0);
        assert_eq!(b.konst(2.0).unwrap().0, 1);
        assert_eq!(b.konst(1.0).unwrap().0, 0);
        // 0.0 and -0.0 differ in their bits.
        assert_eq!(b.konst(0.0).unwrap().0, 2);
        assert_eq!(b.konst(-0.0).unwrap().0, 3);
        assert_eq!(b.konst(0.0).unwrap().0, 2);
        // The same NaN pattern is one entry.
        assert_eq!(b.konst(f32::NAN).unwrap().0, 4);
        assert_eq!(b.konst(f32::NAN).unwrap().0, 4);
        assert_eq!(b.consts.len(), 5);
        assert_eq!(b.consts.items[1], 2.0);
        assert!(b.consts.items[3].is_sign_negative());
    }

    #[test]
    fn konst_pair_never_pools_and_is_consecutive() {
        builder!(b);
        assert_eq!(b.konst(1.0).unwrap().0, 0);
        let c = b.konst_pair(1.0, 6.0).unwrap();
        assert_eq!(c.0, 1);
        assert_eq!(b.konst_pair(1.0, 6.0).unwrap().0, 3);
        assert_eq!(b.consts.items, [1.0, 1.0, 6.0, 1.0, 6.0]);
        // A pair does not enter the pool.
        assert_eq!(b.konst(6.0).unwrap().0, 5);
    }

    #[test]
    fn post_of_maps_the_identity_to_plain_and_pools_the_rest() {
        builder!(b);
        assert_eq!(b.post_of(PostOp::IDENTITY).unwrap(), PostIdx::PLAIN);
        // `-0.0 == 0.0`: the identity test is by value.
        assert_eq!(b.post_of(PostOp::new(1.0, -0.0)).unwrap(), PostIdx::PLAIN);
        assert_eq!(b.post_of(PostOp::new(2.0, 0.0)).unwrap(), PostIdx(1));
        assert_eq!(b.post_of(PostOp::new(2.0, 1.0)).unwrap(), PostIdx(2));
        assert_eq!(b.post_of(PostOp::new(2.0, 0.0)).unwrap(), PostIdx(1));
        assert_eq!(
            b.posts.items,
            [
                PostOp::IDENTITY,
                PostOp::new(2.0, 0.0),
                PostOp::new(2.0, 1.0)
            ]
        );
    }

    #[test]
    fn post_of_keys_on_bits_so_signed_zeros_stay_apart() {
        builder!(b);
        assert_eq!(b.post_of(PostOp::new(0.0, 5.0)).unwrap(), PostIdx(1));
        assert_eq!(b.post_of(PostOp::new(-0.0, 5.0)).unwrap(), PostIdx(2));
    }

    #[test]
    fn hashes_names_and_members_are_pooled() {
        builder!(b);
        assert_eq!(b.hash(7).unwrap().0, 0);
        assert_eq!(b.hash(9).unwrap().0, 1);
        assert_eq!(b.hash(7).unwrap().0, 0);
        assert_eq!(b.hashes.items, [7, 9]);

        let x = Name::new("variable.x");
        let y = Name::new("variable.y");
        assert_eq!(b.name(&x).unwrap().0, 0);
        assert_eq!(b.name(&y).unwrap().0, 1);
        assert_eq!(b.name(&x).unwrap().0, 0);
        assert_eq!(b.names.len(), 2);
        assert_eq!(&*b.names.items[1].text, "variable.y");
        assert_eq!(b.names.items[1].hash, HashedStr::new("variable.y"));
    }

    #[test]
    fn temps_get_one_slot_per_distinct_name_and_set_the_flag() {
        builder!(b);
        assert!(!b.flags.contains(ProgramFlags::USES_TEMPS));
        let (a, c) = (Name::new("temp.a"), Name::new("temp.c"));
        assert_eq!(b.temp(&a).unwrap().0, 0);
        assert!(b.flags.contains(ProgramFlags::USES_TEMPS));
        assert_eq!(b.temp(&c).unwrap().0, 1);
        assert_eq!(b.temp(&a).unwrap().0, 0);
        // The slot table maps to the name table.
        assert_eq!(b.temps.items, [NameIdx(0), NameIdx(1)]);
        assert_eq!(
            &*b.names.items[usize::from(b.temps.items[1].0)].text,
            "temp.c"
        );
        // A variable in between shifts the name index, not the slot.
        b.name(&Name::new("variable.v")).unwrap();
        assert_eq!(b.temp(&Name::new("temp.d")).unwrap().0, 2);
        assert_eq!(b.temps.items, [NameIdx(0), NameIdx(1), NameIdx(3)]);
    }

    #[test]
    fn members_are_pooled_by_name_and_reported_member() {
        builder!(b);
        let (bb, c) = (Name::new("b"), Name::new("c"));
        let first = b.member(&bb, &c).unwrap();
        assert_eq!(first.0, 0);
        assert_eq!(&*b.members.items[0].text, ".b");
        assert_eq!(&*b.members.items[0].report, ".c");
        assert_eq!(b.members.items[0].hash, HashedStr::new("b"));
        // Same member, same reported member: shared.
        assert_eq!(b.member(&bb, &c).unwrap().0, 0);
        // Same member, another reported member: its own entry.
        assert_eq!(b.member(&bb, &bb).unwrap().0, 1);
        assert_eq!(&*b.members.items[1].report, ".b");
        // Another member with the first's reported member.
        assert_eq!(b.member(&c, &c).unwrap().0, 2);
        assert_eq!(b.members.len(), 3);
    }

    #[test]
    fn temps_and_members_pool_past_the_inline_list() {
        builder!(b);
        let last = Name::new("z");
        for round in 0..2 {
            for i in 0..40 {
                assert_eq!(
                    b.temp(&Name::new(format!("temp.t{i}"))).unwrap().0,
                    i,
                    "{round}"
                );
                let member = Name::new(format!("m{i}"));
                assert_eq!(b.member(&member, &last).unwrap().0, i, "{round}");
            }
        }
        assert_eq!((b.temps.len(), b.members.len()), (40, 40));
    }

    #[test]
    fn the_identity_post_op_is_index_zero_and_equal_post_ops_share_an_entry() {
        let p = program_of("v.x * v.y");
        assert_eq!(*p.posts, [PostOp::IDENTITY]);
        let p = program_of("-v.x + -v.y");
        assert_eq!(*p.posts, [PostOp::IDENTITY, PostOp::new(-1.0, 0.0)]);
        assert_eq!(
            p.code[0],
            Instr::LoadVar {
                n: NameIdx(0),
                p: PostIdx(1)
            }
        );
        assert_eq!(
            p.code[2],
            Instr::LoadVar {
                n: NameIdx(1),
                p: PostIdx(1)
            }
        );
        let p = program_of("v.a * 0.5 + v.b * 0.5");
        assert_eq!(p.posts.len(), 2);
        let p = program_of("v.a * 2 + v.b * 3");
        assert_eq!(
            *p.posts,
            [
                PostOp::IDENTITY,
                PostOp::new(2.0, 0.0),
                PostOp::new(3.0, 0.0)
            ]
        );
    }

    #[test]
    fn constants_are_pooled_by_value() {
        let p = program_of("math.clamp(v.a, 0, 1) + math.clamp(v.b, 0, 1)");
        assert_eq!(*p.consts, [0.0, 1.0]);
        assert_eq!(
            p.disassemble(),
            numbered(&[
                "load variable.a",
                "push; const 0",
                "push; const 1",
                "Clamp",
                "push",
                "load variable.b",
                "push; const 0",
                "push; const 1",
                "Clamp",
                "add",
                "end"
            ])
        );
    }

    #[test]
    fn a_constant_zero_in_statements_and_conditionals_is_one_pool_entry() {
        let p = program_of("v.a = 0; v.b = v.c ? 1; return 0;");
        assert_eq!(p.consts.iter().filter(|&&c| c == 0.0).count(), 1);
    }

    #[test]
    fn names_are_interned_across_loads_and_stores() {
        let p = program_of("v.a = v.a + v.b; v.b = v.a;");
        assert_eq!(
            p.names.iter().map(|n| &*n.text).collect::<Vec<_>>(),
            ["variable.a", "variable.b"]
        );
    }
}
