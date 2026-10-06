//! The embedding's side of an evaluation: its handle types, subjects, world access and context.

use std::collections::HashMap;
use std::fmt::{self, Debug};

use nohash_hasher::BuildNoHashHasher;

use super::name::ContextName;
use super::value::Value;

/// The embedding: the handle types its world uses and the type of its world-access object.
///
/// A marker type that lives as long as stored [`Value<H>`]s, so it carries no borrow; the
/// per-evaluation view of the world is [`Host::Access`].
pub trait Host: Sized + 'static {
    /// An actor: a direct reference, a unique id or either, as the host chooses (see
    /// [`HostAccess::resolve_actor`] and [`HostAccess::stored_actor`]).
    type ActorRef: Copy + Eq + Debug + Send + Sync + 'static;
    /// An item stack.
    type ItemRef: Copy + Eq + Debug + Send + Sync + 'static;
    /// A block in the world.
    type BlockRef: Copy + Eq + Debug + Send + Sync + 'static;
    /// The world-access object of one evaluation, as queries see it through
    /// [`QueryCx::host`](super::QueryCx::host).
    ///
    /// It may borrow the world and may be unsized (`type Access<'w> = dyn MyWorld + 'w`). The
    /// evaluator uses it only through [`HostAccess`].
    type Access<'w>: HostAccess<Self> + ?Sized + 'w;
}

/// What the evaluator needs from the world: resolving the left side of `->` and `for_each`
/// entries.
pub trait HostAccess<H: Host> {
    /// The live actor `actor` refers to, or `None` for a null reference, a removed entity or a
    /// stale id.
    ///
    /// `from` is the subject the lookup is made from. `->` with an unresolvable left side yields 0
    /// and skips its right side; `for_each` skips unresolvable entries.
    fn resolve_actor(&self, from: &Subjects<H>, actor: H::ActorRef) -> Option<H::ActorRef>;

    /// The subjects `->` switches to for its right side when the target is `actor`; by default
    /// the actor and nothing else.
    fn subjects_of(&self, actor: H::ActorRef) -> Subjects<H> {
        Subjects::actor(actor)
    }

    /// The subjects of `item` as the left side of `->`; by default the item and nothing else.
    fn subjects_of_item(&self, from: &Subjects<H>, item: H::ItemRef) -> Subjects<H> {
        let _ = from;
        Subjects::item(item)
    }

    /// The handle stored when an actor is written into a variable; by default the handle itself.
    ///
    /// Return an id here if direct references could dangle once stored.
    fn stored_actor(&self, actor: H::ActorRef) -> H::ActorRef {
        actor
    }
}

/// A feature / feature-rule placement position (the world-gen subject).
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub struct WorldGenPos {
    /// Block x.
    pub x: i32,
    /// Block y.
    pub y: i32,
    /// Block z.
    pub z: i32,
}

impl WorldGenPos {
    /// The position `(x, y, z)`.
    pub const fn new(x: i32, y: i32, z: i32) -> Self {
        Self { x, y, z }
    }
}

/// What an expression is evaluated for: the actor, item, block or placement it is about, and
/// the value `this` reads.
///
/// A query whose subject is absent returns its no-subject default
/// ([`DefaultReturn`](crate::catalog::DefaultReturn)). With `actor = None`, `variable.*` is the
/// store's detached map ([`VariableStore::get_local`](super::VariableStore::get_local)); without
/// one, writes are dropped and reads are missing.
pub struct Subjects<H: Host> {
    /// The actor the expression runs for; its variable map is `variable.*`.
    pub actor: Option<H::ActorRef>,
    /// The item stack.
    pub item: Option<H::ItemRef>,
    /// The block.
    pub block: Option<H::BlockRef>,
    /// The world-generation placement position.
    pub world_gen: Option<WorldGenPos>,
    /// `this`: the current value of the property being computed.
    pub this: f32,
}

impl<H: Host> Subjects<H> {
    /// No subject at all and `this` = 0.
    pub const fn none() -> Self {
        Self {
            actor: None,
            item: None,
            block: None,
            world_gen: None,
            this: 0.0,
        }
    }

    /// An actor subject.
    pub const fn actor(actor: H::ActorRef) -> Self {
        Self {
            actor: Some(actor),
            ..Self::none()
        }
    }

    /// An item subject.
    pub const fn item(item: H::ItemRef) -> Self {
        Self {
            item: Some(item),
            ..Self::none()
        }
    }

    /// A block subject.
    pub const fn block(block: H::BlockRef) -> Self {
        Self {
            block: Some(block),
            ..Self::none()
        }
    }

    /// A world-generation subject.
    pub const fn world_gen(pos: WorldGenPos) -> Self {
        Self {
            world_gen: Some(pos),
            ..Self::none()
        }
    }
}

// Derives would demand `H: Clone` / `H: PartialEq` / `H: Debug` of the marker type.
impl<H: Host> Clone for Subjects<H> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<H: Host> Copy for Subjects<H> {}

impl<H: Host> Default for Subjects<H> {
    fn default() -> Self {
        Self::none()
    }
}

impl<H: Host> PartialEq for Subjects<H> {
    fn eq(&self, other: &Self) -> bool {
        self.actor == other.actor
            && self.item == other.item
            && self.block == other.block
            && self.world_gen == other.world_gen
            && self.this == other.this
    }
}

impl<H: Host> Debug for Subjects<H> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Subjects")
            .field("actor", &self.actor)
            .field("item", &self.item)
            .field("block", &self.block)
            .field("world_gen", &self.world_gen)
            .field("this", &self.this)
            .finish()
    }
}

/// Read-only `context.*`: the variables the caller sets for one evaluation.
pub trait ContextProvider<H: Host> {
    /// The value of the context variable `name`, or `None` (read as a missing variable).
    fn context(&self, name: ContextName) -> Option<Value<H>>;
}

/// A [`ContextProvider`] with no context variable at all.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub struct NoContext;

impl<H: Host> ContextProvider<H> for NoContext {
    fn context(&self, _name: ContextName) -> Option<Value<H>> {
        None
    }
}

/// A ready-made [`ContextProvider`]: a map the caller fills before an evaluation.
pub struct ContextMap<H: Host> {
    values: HashMap<ContextName, Value<H>, BuildNoHashHasher<ContextName>>,
}

impl<H: Host> ContextMap<H> {
    /// An empty context.
    pub fn new() -> Self {
        Self {
            values: HashMap::default(),
        }
    }

    /// Sets a context variable, returning the previous value.
    pub fn set(&mut self, name: ContextName, value: impl Into<Value<H>>) -> Option<Value<H>> {
        self.values.insert(name, value.into())
    }

    /// The value of a context variable.
    pub fn get(&self, name: ContextName) -> Option<&Value<H>> {
        self.values.get(&name)
    }

    /// Removes one context variable.
    pub fn remove(&mut self, name: ContextName) -> Option<Value<H>> {
        self.values.remove(&name)
    }

    /// Removes every context variable.
    pub fn clear(&mut self) {
        self.values.clear();
    }

    /// Number of context variables set.
    pub fn len(&self) -> usize {
        self.values.len()
    }

    /// Whether no context variable is set.
    pub fn is_empty(&self) -> bool {
        self.values.is_empty()
    }
}

impl<H: Host> Default for ContextMap<H> {
    fn default() -> Self {
        Self::new()
    }
}

impl<H: Host> Clone for ContextMap<H> {
    fn clone(&self) -> Self {
        Self {
            values: self.values.clone(),
        }
    }
}

impl<H: Host> Debug for ContextMap<H> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_map().entries(self.values.iter()).finish()
    }
}

impl<H: Host, V: Into<Value<H>>> FromIterator<(ContextName, V)> for ContextMap<H> {
    /// A later value of a name replaces an earlier one.
    fn from_iter<I: IntoIterator<Item = (ContextName, V)>>(entries: I) -> Self {
        Self {
            values: entries
                .into_iter()
                .map(|(name, value)| (name, value.into()))
                .collect(),
        }
    }
}

impl<H: Host, V: Into<Value<H>>, const LEN: usize> From<[(ContextName, V); LEN]> for ContextMap<H> {
    fn from(entries: [(ContextName, V); LEN]) -> Self {
        entries.into_iter().collect()
    }
}

impl<H: Host> ContextProvider<H> for ContextMap<H> {
    fn context(&self, name: ContextName) -> Option<Value<H>> {
        self.values.get(&name).cloned()
    }
}

/// The host of an evaluation that needs no world: every handle is `()` and nothing resolves.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub struct NoHost;

impl Host for NoHost {
    type ActorRef = ();
    type ItemRef = ();
    type BlockRef = ();
    type Access<'w> = NoHost;
}

impl HostAccess<NoHost> for NoHost {
    fn resolve_actor(&self, _from: &Subjects<NoHost>, _actor: ()) -> Option<()> {
        None
    }

    fn subjects_of(&self, _actor: ()) -> Subjects<NoHost> {
        Subjects::none()
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::*;
    use crate::vm::StructValue;
    use crate::vm::test_support::TestHost;

    type V = Value<TestHost>;
    type S = Subjects<TestHost>;

    #[test]
    fn subjects() {
        let none = S::none();
        assert_eq!(
            (none.actor, none.item, none.block, none.world_gen, none.this),
            (None, None, None, None, 0.0)
        );
        assert_eq!(S::default(), none);
        assert_eq!(S::actor(1).actor, Some(1));
        assert_eq!(S::item(3).item, Some(3));
        assert_eq!(S::block((1, 2, 3)).block, Some((1, 2, 3)));
        assert_eq!(
            S::world_gen(WorldGenPos { x: 1, y: 2, z: 3 }).world_gen,
            Some(WorldGenPos { x: 1, y: 2, z: 3 })
        );
        let with_this = S { this: 2.34, ..none };
        assert_eq!(with_this.this, 2.34);
        let copy = with_this;
        assert_eq!(copy, with_this);
    }

    #[test]
    fn each_constructor_sets_exactly_one_subject() {
        let by_constructor = [
            S::actor(1),
            S::item(2),
            S::block((1, 2, 3)),
            S::world_gen(WorldGenPos { x: 4, y: 5, z: 6 }),
        ];
        for (i, subjects) in by_constructor.iter().enumerate() {
            let set = [
                subjects.actor.is_some(),
                subjects.item.is_some(),
                subjects.block.is_some(),
                subjects.world_gen.is_some(),
            ];
            for (j, is_set) in set.into_iter().enumerate() {
                assert_eq!(is_set, i == j, "constructor {i}, subject {j}");
            }
            assert_eq!(subjects.this, 0.0);
        }
    }

    #[test]
    fn subjects_compare_every_field_including_this() {
        assert_eq!(S::actor(1), S::actor(1));
        assert_ne!(S::actor(1), S::actor(2));
        assert_ne!(S::actor(1), S::item(1));
        assert_ne!(
            S::none(),
            S {
                this: 1.0,
                ..S::none()
            }
        );
        assert_eq!(
            S {
                this: 1.0,
                ..S::none()
            },
            S {
                this: 1.0,
                ..S::none()
            }
        );
        assert_ne!(S::block((1, 2, 3)), S::block((1, 2, 4)));
        assert_ne!(
            S::world_gen(WorldGenPos { x: 1, y: 2, z: 3 }),
            S::world_gen(WorldGenPos { x: 1, y: 2, z: 4 })
        );
        assert_ne!(
            S {
                this: f32::NAN,
                ..S::none()
            },
            S {
                this: f32::NAN,
                ..S::none()
            }
        );
        assert_eq!(
            S {
                this: 0.0,
                ..S::none()
            },
            S {
                this: -0.0,
                ..S::none()
            }
        );
    }

    #[test]
    fn subjects_debug_names_all_five_fields() {
        assert_eq!(
            format!(
                "{:?}",
                S {
                    this: 2.5,
                    ..S::actor(1)
                }
            ),
            "Subjects { actor: Some(1), item: None, block: None, world_gen: None, this: 2.5 }"
        );
        assert_eq!(
            format!("{:?}", S::world_gen(WorldGenPos { x: 1, y: 2, z: 3 })),
            "Subjects { actor: None, item: None, block: None, world_gen: Some(WorldGenPos { x: 1, y: 2, z: 3 }), this: 0.0 }"
        );
    }

    #[test]
    fn world_gen_positions_default_to_the_origin_and_hash_by_value() {
        assert_eq!(WorldGenPos::default(), WorldGenPos { x: 0, y: 0, z: 0 });
        let set: HashSet<WorldGenPos> = [
            WorldGenPos { x: 1, y: 2, z: 3 },
            WorldGenPos { x: 1, y: 2, z: 3 },
            WorldGenPos { x: 3, y: 2, z: 1 },
        ]
        .into_iter()
        .collect();
        assert_eq!(set.len(), 2);
    }

    struct Minimal;

    impl HostAccess<TestHost> for Minimal {
        fn resolve_actor(&self, _from: &S, actor: u32) -> Option<u32> {
            Some(actor)
        }
    }

    #[test]
    fn the_default_stored_actor_keeps_the_handle() {
        assert_eq!(Minimal.stored_actor(5), 5);
        assert_eq!(Minimal.stored_actor(0), 0);
    }

    #[test]
    fn the_default_actor_subjects_are_the_actor_and_nothing_else() {
        assert_eq!(Minimal.subjects_of(7), S::actor(7));
        assert_eq!(
            Minimal.subjects_of(7),
            S {
                actor: Some(7),
                ..S::none()
            }
        );
    }

    #[test]
    fn the_default_item_subjects_are_the_item_and_nothing_else() {
        let from = S {
            this: 9.0,
            ..S::actor(1)
        };
        assert_eq!(Minimal.subjects_of_item(&from, 4), S::item(4));
        assert_eq!(Minimal.subjects_of_item(&S::none(), 4), S::item(4));
        let subjects = Minimal.subjects_of_item(&from, 4);
        assert_eq!(
            (
                subjects.actor,
                subjects.block,
                subjects.world_gen,
                subjects.this
            ),
            (None, None, None, 0.0)
        );
    }

    #[test]
    fn no_host_resolves_nothing_and_has_no_subjects() {
        assert_eq!(NoHost.resolve_actor(&Subjects::none(), ()), None);
        assert_eq!(NoHost.resolve_actor(&Subjects::actor(()), ()), None);
        assert_eq!(NoHost.subjects_of(()), Subjects::none());
        assert_eq!(NoHost.stored_actor(()), ());
        assert_eq!(
            NoHost.subjects_of_item(&Subjects::none(), ()),
            Subjects::item(())
        );
        let copy = NoHost;
        assert_eq!(copy, NoHost);
    }

    #[test]
    fn a_context_map_sets_returns_the_previous_value_and_removes() {
        let (moo, baa) = (ContextName::new("moo"), ContextName::new("baa"));
        let mut map = ContextMap::<TestHost>::new();
        assert!(map.is_empty());
        assert_eq!(map.set(moo, V::Float(1.0)), None);
        assert_eq!(map.set(moo, V::Float(2.0)), Some(V::Float(1.0)));
        assert_eq!(map.len(), 1);
        assert_eq!(map.get(moo), Some(&V::Float(2.0)));
        assert_eq!(map.get(baa), None);
        assert_eq!(map.remove(baa), None);
        assert_eq!(map.remove(moo), Some(V::Float(2.0)));
        assert!(map.is_empty());
        assert_eq!(map.remove(moo), None);
    }

    #[test]
    fn a_context_map_from_entries_keeps_the_last_value_of_a_name_and_clear_empties() {
        let map = ContextMap::<TestHost>::from([
            (ContextName::new("a"), V::Float(1.0)),
            (ContextName::new("b"), V::string("b")),
            (ContextName::new("a"), V::Float(3.0)),
        ]);
        assert_eq!(map.len(), 2);
        assert_eq!(map.get(ContextName::new("a")), Some(&V::Float(3.0)));
        let collected: ContextMap<TestHost> = [
            (ContextName::new("n"), 1.0f32),
            (ContextName::new("n"), 2.0),
        ]
        .into_iter()
        .collect();
        assert_eq!(
            (collected.len(), collected.get(ContextName::new("n"))),
            (1, Some(&V::Float(2.0)))
        );
        let mut map = map;
        map.clear();
        assert!(map.is_empty());
        assert_eq!(map.len(), 0);
        assert!(ContextMap::<TestHost>::default().is_empty());
    }

    #[test]
    fn a_context_map_holding_a_variable_is_not_empty() {
        let moo = ContextName::new("moo");
        let mut map = ContextMap::<TestHost>::new();
        map.set(moo, V::Float(1.0));
        assert!(!map.is_empty());
        assert_eq!(map.len(), 1);
        assert!(!ContextMap::<TestHost>::from([(moo, V::Float(1.0))]).is_empty());
        map.remove(moo);
        assert!(map.is_empty());
    }

    #[test]
    fn cloning_subjects_keeps_every_field() {
        // Cloning a vector's elements calls `Clone::clone` even though `Subjects` is `Copy`.
        let pool = vec![
            S::none(),
            S {
                this: 2.5,
                ..S::actor(3)
            },
            S::item(4),
            S::block((1, 2, 3)),
            S::world_gen(WorldGenPos { x: 4, y: 5, z: 6 }),
        ];
        let cloned = pool.clone();
        assert_eq!(cloned, pool);
        assert_eq!(cloned[1].actor, Some(3));
        assert_eq!(cloned[1].this, 2.5);
        assert_ne!(cloned[1], S::default());
    }

    #[test]
    fn a_context_map_clone_is_independent() {
        let moo = ContextName::new("moo");
        let mut original = ContextMap::<TestHost>::from([(moo, V::Float(1.0))]);
        let copy = original.clone();
        original.set(moo, V::Float(2.0));
        original.set(ContextName::new("other"), V::Float(3.0));
        assert_eq!(copy.get(moo), Some(&V::Float(1.0)));
        assert_eq!(copy.len(), 1);
    }

    #[test]
    fn a_context_map_debug_is_a_map_by_key() {
        let moo = ContextName::new("moo");
        assert_eq!(format!("{:?}", ContextMap::<TestHost>::new()), "{}");
        let map = ContextMap::<TestHost>::from([(moo, V::Float(1.0))]);
        assert_eq!(format!("{map:?}"), format!("{{{moo:?}: Float(1.0)}}"));
    }

    #[test]
    fn the_context_provider_hands_out_an_owned_copy() {
        let moo = ContextName::new("moo");
        let map = ContextMap::<TestHost>::from([
            (moo, V::structure(StructValue::xy(1.0, 2.0))),
            (ContextName::new("n"), V::Float(1.0)),
        ]);
        assert_eq!(
            map.context(moo),
            Some(V::structure(StructValue::xy(1.0, 2.0)))
        );
        assert_eq!(map.context(ContextName::new("n")), Some(V::Float(1.0)));
        assert_eq!(map.context(ContextName::new("absent")), None);
    }

    #[test]
    fn no_context_has_no_variable_at_all() {
        let none = NoContext;
        assert_eq!(
            ContextProvider::<TestHost>::context(&none, ContextName::new("moo")),
            None
        );
        assert_eq!(
            ContextProvider::<NoHost>::context(&none, ContextName::new("")),
            None
        );
    }

    #[test]
    fn host_handle_bounds() {
        fn bounds<H: Host>()
        where
            H::ActorRef: Copy + Eq + Debug,
            H::ItemRef: Copy + Eq + Debug,
            H::BlockRef: Copy + Eq + Debug,
        {
        }
        bounds::<NoHost>();
        bounds::<TestHost>();
    }
}
