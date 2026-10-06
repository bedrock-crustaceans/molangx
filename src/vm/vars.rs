//! `variable.*` storage, and [`TempMap`] for `temp.*` kept across evaluations.

use std::collections::HashMap;
use std::fmt::{self, Debug};
use std::hash::Hash;

use nohash_hasher::BuildNoHashHasher;
use rustc_hash::FxBuildHasher;

use super::host::Host;
use super::name::namespace::{Namespace, Temp, Variable};
use super::name::{Name, VariableName};
use super::value::Value;

/// Who may read an entity variable.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Hash)]
pub enum Access {
    /// Only the owning entity reads the variable; through `->` it reads 0.
    #[default]
    Private,
    /// Other entities read the variable's public snapshot through `->`.
    Public,
}

/// Entity (`variable.*`) storage: one variable map per actor, plus an optional detached map for
/// evaluations without an actor.
///
/// Inside `->` the evaluator reads the target through [`get_public`](Self::get_public). Actors in
/// written values have passed through
/// [`HostAccess::stored_actor`](super::HostAccess::stored_actor).
pub trait VariableStore<H: Host> {
    /// The latest value of `name` in `actor`'s map, as the owner reads it.
    fn get(&self, actor: H::ActorRef, name: VariableName) -> Option<&Value<H>>;

    /// Writes `name` in `actor`'s map; a new slot is [`Access::Private`], an existing one keeps
    /// its access. A store that has no map for `actor` ignores the write.
    fn set(&mut self, actor: H::ActorRef, name: VariableName, value: Value<H>);

    /// The public snapshot of `name` in `actor`'s map, as another entity reads it through `->`.
    ///
    /// `None` for an absent or private variable, or a public one with no snapshot yet; the
    /// evaluator then reads 0.0.
    fn get_public(&self, actor: H::ActorRef, name: VariableName) -> Option<&Value<H>>;

    /// The value of `name` in the detached map; by default there is none and every read misses.
    fn get_local(&self, name: VariableName) -> Option<&Value<H>> {
        let _ = name;
        None
    }

    /// Writes `name` in the detached map; by default there is none and the write is dropped.
    fn set_local(&mut self, name: VariableName, value: Value<H>) {
        let _ = (name, value);
    }
}

struct Slot<H: Host> {
    /// `None` while only the access has been declared.
    value: Option<Value<H>>,
    exposure: Exposure<H>,
}

/// A slot's [`Access`], with the snapshot other entities read when it is public.
enum Exposure<H: Host> {
    Private,
    /// `None` until the first refresh after the slot became public.
    Public(Option<Value<H>>),
}

impl<H: Host> Exposure<H> {
    /// No snapshot yet.
    const fn new(access: Access) -> Self {
        match access {
            Access::Private => Self::Private,
            Access::Public => Self::Public(None),
        }
    }

    const fn access(&self) -> Access {
        match self {
            Self::Private => Access::Private,
            Self::Public(_) => Access::Public,
        }
    }
}

impl<H: Host> Slot<H> {
    const fn new(access: Access) -> Self {
        Self {
            value: None,
            exposure: Exposure::new(access),
        }
    }

    const fn access(&self) -> Access {
        self.exposure.access()
    }

    fn snapshot(&self) -> Option<&Value<H>> {
        match &self.exposure {
            Exposure::Private => None,
            Exposure::Public(snapshot) => snapshot.as_ref(),
        }
    }
}

// Manual impls: derives would bound the marker type `H`.
impl<H: Host> Clone for Slot<H> {
    fn clone(&self) -> Self {
        Self {
            value: self.value.clone(),
            exposure: match &self.exposure {
                Exposure::Private => Exposure::Private,
                Exposure::Public(snapshot) => Exposure::Public(snapshot.clone()),
            },
        }
    }
}

impl<H: Host> PartialEq for Slot<H> {
    fn eq(&self, other: &Self) -> bool {
        self.value == other.value
            && self.access() == other.access()
            && self.snapshot() == other.snapshot()
    }
}

/// A map of names to values in namespace `N`: an entity's variables or a [`TempMap`].
///
/// A public variable's snapshot, which other entities read through `->`, changes only in
/// [`refresh_snapshots`](Self::refresh_snapshots). As a [`VariableStore`], one map serves every
/// actor and the detached map alike.
pub struct VariableMap<H: Host, N: Namespace = Variable> {
    slots: HashMap<Name<N>, Slot<H>, BuildNoHashHasher<Name<N>>>,
}

/// The `temp.*` values kept across evaluations ([`EvalCx::temps`](super::EvalCx::temps)).
pub type TempMap<H> = VariableMap<H, Temp>;

impl<H: Host, N: Namespace> VariableMap<H, N> {
    /// An empty map.
    pub fn new() -> Self {
        Self {
            slots: HashMap::default(),
        }
    }

    /// The latest value of a name (`None`: never written).
    pub fn get(&self, name: Name<N>) -> Option<&Value<H>> {
        self.slots.get(&name)?.value.as_ref()
    }

    /// The latest value of a name, mutably.
    pub fn get_mut(&mut self, name: Name<N>) -> Option<&mut Value<H>> {
        self.slots.get_mut(&name)?.value.as_mut()
    }

    /// Writes a name, returning its previous latest value. A new variable is
    /// [`Access::Private`]; an existing one keeps its access and snapshot.
    pub fn set(&mut self, name: Name<N>, value: impl Into<Value<H>>) -> Option<Value<H>> {
        self.insert(name, value.into(), Access::Private)
    }

    fn insert(&mut self, name: Name<N>, value: Value<H>, access: Access) -> Option<Value<H>> {
        self.slots
            .entry(name)
            .or_insert_with(|| Slot::new(access))
            .value
            .replace(value)
    }

    /// Removes a name, returning its latest value.
    pub fn remove(&mut self, name: Name<N>) -> Option<Value<H>> {
        self.slots.remove(&name)?.value
    }

    /// Removes every name.
    pub fn clear(&mut self) {
        self.slots.clear();
    }

    /// Number of names that have a value; walks every slot.
    pub fn len(&self) -> usize {
        self.slots
            .values()
            .filter(|slot| slot.value.is_some())
            .count()
    }

    /// Whether no name has a value; walks the slots up to the first with a value.
    pub fn is_empty(&self) -> bool {
        self.iter().next().is_none()
    }

    /// The names that have a value, in no particular order.
    pub fn iter(&self) -> impl Iterator<Item = (Name<N>, &Value<H>)> {
        self.slots
            .iter()
            .filter_map(|(name, slot)| slot.value.as_ref().map(|value| (*name, value)))
    }
}

impl<H: Host> VariableMap<H> {
    /// Writes a variable, returning its previous latest value; `access` applies only when the
    /// slot is new.
    pub fn set_with_access(
        &mut self,
        name: VariableName,
        value: impl Into<Value<H>>,
        access: Access,
    ) -> Option<Value<H>> {
        self.insert(name, value.into(), access)
    }

    /// Writes a variable and marks it public, without refreshing its snapshot; returns its
    /// previous latest value.
    pub fn set_public(
        &mut self,
        name: VariableName,
        value: impl Into<Value<H>>,
    ) -> Option<Value<H>> {
        let slot = self
            .slots
            .entry(name)
            .or_insert_with(|| Slot::new(Access::Public));
        if slot.access() == Access::Private {
            slot.exposure = Exposure::new(Access::Public);
        }
        slot.value.replace(value.into())
    }

    /// Declares the access of a variable, whether or not it has a value. Making it private drops
    /// its snapshot.
    pub fn set_access(&mut self, name: VariableName, access: Access) {
        let slot = self.slots.entry(name).or_insert_with(|| Slot::new(access));
        if slot.access() != access {
            slot.exposure = Exposure::new(access);
        }
    }

    /// The access of a variable (`None`: unknown name).
    pub fn access(&self, name: VariableName) -> Option<Access> {
        self.slots.get(&name).map(Slot::access)
    }

    /// The public snapshot of a variable: `None` when absent, private or not yet refreshed.
    pub fn get_public(&self, name: VariableName) -> Option<&Value<H>> {
        self.slots.get(&name)?.snapshot()
    }

    /// Copies the latest value of every public variable into its snapshot.
    pub fn refresh_snapshots(&mut self) {
        for slot in self.slots.values_mut() {
            if let Exposure::Public(snapshot) = &mut slot.exposure {
                snapshot.clone_from(&slot.value);
            }
        }
    }

    /// Whether any variable is public.
    pub fn any_public(&self) -> bool {
        self.slots
            .values()
            .any(|slot| slot.access() == Access::Public)
    }
}

impl<H: Host, N: Namespace> Default for VariableMap<H, N> {
    fn default() -> Self {
        Self::new()
    }
}

impl<H: Host, N: Namespace, V: Into<Value<H>>> FromIterator<(Name<N>, V)> for VariableMap<H, N> {
    /// Private names; a later value of a name replaces an earlier one.
    fn from_iter<I: IntoIterator<Item = (Name<N>, V)>>(entries: I) -> Self {
        let mut map = Self::new();
        for (name, value) in entries {
            map.set(name, value);
        }
        map
    }
}

impl<H: Host, N: Namespace, V: Into<Value<H>>, const LEN: usize> From<[(Name<N>, V); LEN]>
    for VariableMap<H, N>
{
    fn from(entries: [(Name<N>, V); LEN]) -> Self {
        entries.into_iter().collect()
    }
}

impl<H: Host, N: Namespace> Clone for VariableMap<H, N> {
    fn clone(&self) -> Self {
        Self {
            slots: self.slots.clone(),
        }
    }
}

impl<H: Host, N: Namespace> PartialEq for VariableMap<H, N> {
    fn eq(&self, other: &Self) -> bool {
        self.slots == other.slots
    }
}

impl<H: Host, N: Namespace> Debug for VariableMap<H, N> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_map()
            .entries(
                self.slots
                    .iter()
                    .map(|(name, slot)| (name, (&slot.value, slot.access(), slot.snapshot()))),
            )
            .finish()
    }
}

impl<H: Host> VariableStore<H> for VariableMap<H> {
    fn get(&self, _actor: H::ActorRef, name: VariableName) -> Option<&Value<H>> {
        VariableMap::get(self, name)
    }

    fn set(&mut self, _actor: H::ActorRef, name: VariableName, value: Value<H>) {
        VariableMap::set(self, name, value);
    }

    fn get_public(&self, _actor: H::ActorRef, name: VariableName) -> Option<&Value<H>> {
        VariableMap::get_public(self, name)
    }

    fn get_local(&self, name: VariableName) -> Option<&Value<H>> {
        VariableMap::get(self, name)
    }

    fn set_local(&mut self, name: VariableName, value: Value<H>) {
        VariableMap::set(self, name, value);
    }
}

/// A store without any map: every read misses and every write is dropped.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub struct NoVariables;

impl<H: Host> VariableStore<H> for NoVariables {
    fn get(&self, _actor: H::ActorRef, _name: VariableName) -> Option<&Value<H>> {
        None
    }

    fn set(&mut self, _actor: H::ActorRef, _name: VariableName, _value: Value<H>) {}

    fn get_public(&self, _actor: H::ActorRef, _name: VariableName) -> Option<&Value<H>> {
        None
    }
}

/// A ready-made [`VariableStore`]: one [`VariableMap`] per actor, created on first write, plus an
/// optional detached map for evaluations without an actor.
pub struct VariableStorage<H: Host>
where
    H::ActorRef: Hash,
{
    // Actor handles come from the host, not from scripts: a hasher without flooding resistance
    // is enough.
    actors: HashMap<H::ActorRef, VariableMap<H>, FxBuildHasher>,
    local: Option<VariableMap<H>>,
}

impl<H: Host> VariableStorage<H>
where
    H::ActorRef: Hash,
{
    /// A store with no actor map and no detached map.
    pub fn new() -> Self {
        Self {
            actors: HashMap::default(),
            local: None,
        }
    }

    /// A store whose detached map exists (empty), so actor-less evaluations can use `variable.*`.
    pub fn with_local() -> Self {
        Self {
            actors: HashMap::default(),
            local: Some(VariableMap::new()),
        }
    }

    /// The map of `actor`, if it has one.
    pub fn actor(&self, actor: H::ActorRef) -> Option<&VariableMap<H>> {
        self.actors.get(&actor)
    }

    /// The map of `actor`, created empty when it has none.
    pub fn actor_mut(&mut self, actor: H::ActorRef) -> &mut VariableMap<H> {
        self.actors.entry(actor).or_default()
    }

    /// Drops the map of `actor` (the entity was removed).
    pub fn remove_actor(&mut self, actor: H::ActorRef) -> Option<VariableMap<H>> {
        self.actors.remove(&actor)
    }

    /// The detached map, if the store has one.
    pub fn local(&self) -> Option<&VariableMap<H>> {
        self.local.as_ref()
    }

    /// The detached map, created empty when the store has none.
    pub fn local_mut(&mut self) -> &mut VariableMap<H> {
        self.local.get_or_insert_with(VariableMap::new)
    }

    /// Removes the detached map: actor-less evaluations then have no `variable.*` at all.
    pub fn remove_local(&mut self) -> Option<VariableMap<H>> {
        self.local.take()
    }

    /// Refreshes the public snapshots of every actor.
    pub fn refresh_snapshots(&mut self) {
        for map in self.actors.values_mut() {
            map.refresh_snapshots();
        }
    }
}

impl<H: Host> Default for VariableStorage<H>
where
    H::ActorRef: Hash,
{
    fn default() -> Self {
        Self::new()
    }
}

impl<H: Host> Debug for VariableStorage<H>
where
    H::ActorRef: Hash,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("VariableStorage")
            .field("actors", &self.actors)
            .field("local", &self.local)
            .finish()
    }
}

impl<H: Host> VariableStore<H> for VariableStorage<H>
where
    H::ActorRef: Hash,
{
    fn get(&self, actor: H::ActorRef, name: VariableName) -> Option<&Value<H>> {
        self.actors.get(&actor)?.get(name)
    }

    fn set(&mut self, actor: H::ActorRef, name: VariableName, value: Value<H>) {
        self.actor_mut(actor).set(name, value);
    }

    fn get_public(&self, actor: H::ActorRef, name: VariableName) -> Option<&Value<H>> {
        self.actors.get(&actor)?.get_public(name)
    }

    fn get_local(&self, name: VariableName) -> Option<&Value<H>> {
        self.local.as_ref()?.get(name)
    }

    fn set_local(&mut self, name: VariableName, value: Value<H>) {
        if let Some(local) = &mut self.local {
            local.set(name, value);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hash::HashedStr;
    use crate::vm::{StructValue, test_support::TestHost};

    type V = Value<TestHost>;
    type Map = VariableMap<TestHost>;

    const BAA: VariableName = VariableName::new("baa");
    const MOO: VariableName = VariableName::new("moo");

    #[test]
    fn variable_map_reads_and_writes() {
        let mut map = Map::new();
        assert!(map.is_empty());
        assert_eq!(map.get(BAA), None);
        map.set(BAA, V::Float(1.0));
        map.set(BAA, V::Float(2.0));
        assert_eq!(map.get(BAA), Some(&V::Float(2.0)));
        assert_eq!(map.len(), 1);
        assert_eq!(map.iter().collect::<Vec<_>>(), vec![(BAA, &V::Float(2.0))]);
        map.set(
            VariableName::new("s"),
            V::structure(StructValue::xy(1.0, 2.0)),
        );
        map.get_mut(VariableName::new("s"))
            .unwrap()
            .set_member_path(&[HashedStr::new("x")], V::Float(5.0));
        assert_eq!(
            map.get(VariableName::new("s"))
                .unwrap()
                .member(HashedStr::new("x")),
            Some(&V::Float(5.0))
        );
        assert_eq!(map.remove(BAA), Some(V::Float(2.0)));
        assert_eq!(map.get(BAA), None);
        map.clear();
        assert!(map.is_empty());
    }

    #[test]
    fn writes_never_change_access() {
        let mut map = Map::new();
        map.set(BAA, V::Float(1.0));
        assert_eq!(map.access(BAA), Some(Access::Private));
        assert!(!map.any_public());

        let moo = VariableName::new("moo");
        map.set_access(moo, Access::Public);
        assert_eq!(map.get(moo), None);
        assert_eq!(map.len(), 1);
        map.set(moo, V::Float(3.0));
        assert_eq!(map.access(moo), Some(Access::Public));
        assert!(map.any_public());

        VariableStore::set(&mut map, 1, moo, V::Float(4.0));
        assert_eq!(map.access(moo), Some(Access::Public));
        assert_eq!(map.get(moo), Some(&V::Float(4.0)));
        assert_eq!(map.access(VariableName::new("unknown")), None);
    }

    #[test]
    fn public_snapshot_needs_an_update() {
        let mut map = Map::new();
        map.set_public(BAA, V::Float(1.23));
        assert_eq!(map.get_public(BAA), None);
        map.refresh_snapshots();
        map.set_public(BAA, V::Float(2.34));
        assert_eq!(map.get_public(BAA), Some(&V::Float(1.23)));
        assert_eq!(map.get(BAA), Some(&V::Float(2.34)));
        map.refresh_snapshots();
        assert_eq!(map.get_public(BAA), Some(&V::Float(2.34)));
        assert_eq!(map.get(BAA), Some(&V::Float(2.34)));
    }

    #[test]
    fn private_and_absent_variables_have_no_public_value() {
        let mut map = Map::new();
        map.set(BAA, V::Float(1.0));
        map.refresh_snapshots();
        assert_eq!(map.get_public(BAA), None);
        assert_eq!(
            map.get_public(VariableName::new("this_var_does_not_exist_yet")),
            None
        );
        map.set_access(BAA, Access::Public);
        map.refresh_snapshots();
        assert_eq!(map.get_public(BAA), Some(&V::Float(1.0)));
        map.set_access(BAA, Access::Private);
        assert_eq!(map.get_public(BAA), None);
        map.set_access(BAA, Access::Public);
        assert_eq!(map.get_public(BAA), None);
    }

    #[test]
    fn declaring_the_same_access_again_keeps_the_snapshot() {
        let mut map = Map::new();
        map.set_public(BAA, V::Float(1.0));
        map.refresh_snapshots();
        map.set(BAA, V::Float(2.0));
        map.set_access(BAA, Access::Public);
        assert_eq!(map.get_public(BAA), Some(&V::Float(1.0)));
        assert_eq!(map.get(BAA), Some(&V::Float(2.0)));
        map.set_access(BAA, Access::Private);
        map.set_access(BAA, Access::Private);
        map.set_access(BAA, Access::Public);
        assert_eq!(map.get_public(BAA), None);
        assert_eq!(map.get(BAA), Some(&V::Float(2.0)));
    }

    #[test]
    fn variable_map_equality_is_order_independent() {
        let names = [
            VariableName::new("a"),
            VariableName::new("b"),
            VariableName::new("c"),
        ];
        let mut forward = Map::new();
        let mut backward = Map::new();
        for (i, name) in names.iter().enumerate() {
            forward.set(*name, V::Float(i as f32));
        }
        for (i, name) in names.iter().enumerate().rev() {
            backward.set(*name, V::Float(i as f32));
        }
        assert_eq!(forward, backward);
        backward.set(names[0], V::Float(9.0));
        assert_ne!(forward, backward);
        assert_eq!(forward.clone(), forward);
        let mut public = forward.clone();
        public.set_access(names[0], Access::Public);
        assert_ne!(forward, public);
    }

    #[test]
    fn variable_storage_keeps_one_map_per_actor() {
        let mut store = VariableStorage::<TestHost>::new();
        let (cow, pig) = (1, 2);
        store.set(cow, BAA, V::Float(1.0));
        store.set(pig, BAA, V::Float(2.0));
        assert_eq!(store.get(cow, BAA), Some(&V::Float(1.0)));
        assert_eq!(store.get(pig, BAA), Some(&V::Float(2.0)));
        assert_eq!(store.get(3, BAA), None);
        assert_eq!(store.get_public(cow, BAA), None);

        store.actor_mut(cow).set_access(BAA, Access::Public);
        store.refresh_snapshots();
        store.set(cow, BAA, V::Float(5.0));
        assert_eq!(store.get_public(cow, BAA), Some(&V::Float(1.0)));
        assert_eq!(store.get(cow, BAA), Some(&V::Float(5.0)));
        assert_eq!(store.get_public(pig, BAA), None);

        assert!(store.remove_actor(pig).is_some());
        assert_eq!(store.get(pig, BAA), None);
        assert!(store.actor(pig).is_none());
    }

    #[test]
    fn access_is_private_by_default() {
        assert_eq!(Access::default(), Access::Private);
        assert_ne!(Access::Private, Access::Public);
    }

    #[test]
    fn a_new_slot_is_private_and_a_declared_access_is_kept_by_set() {
        let mut map = Map::new();
        map.set(BAA, V::Float(1.0));
        assert_eq!(map.access(BAA), Some(Access::Private));
        map.set_access(MOO, Access::Public);
        map.set(MOO, V::Float(1.0));
        map.set(MOO, V::Float(2.0));
        assert_eq!(map.access(MOO), Some(Access::Public));
    }

    #[test]
    fn set_with_access_applies_the_access_to_a_new_slot_only() {
        let mut map = Map::new();
        map.set_with_access(BAA, V::Float(1.0), Access::Public);
        assert_eq!(map.access(BAA), Some(Access::Public));
        map.set_with_access(BAA, V::Float(2.0), Access::Private);
        assert_eq!(map.access(BAA), Some(Access::Public));
        assert_eq!(map.get(BAA), Some(&V::Float(2.0)));
        map.set_with_access(MOO, V::Float(3.0), Access::Private);
        assert_eq!(map.access(MOO), Some(Access::Private));
    }

    #[test]
    fn set_public_makes_a_private_slot_public_and_keeps_its_value_until_replaced() {
        let mut map = Map::new();
        map.set(BAA, V::Float(1.0));
        assert!(!map.any_public());
        map.set_public(BAA, V::Float(2.0));
        assert_eq!(map.access(BAA), Some(Access::Public));
        assert_eq!(map.get(BAA), Some(&V::Float(2.0)));
        assert!(map.any_public());
        map.set_public(MOO, V::Float(3.0));
        assert_eq!(map.access(MOO), Some(Access::Public));
        assert_eq!(map.len(), 2);
        assert_eq!(map.get_public(BAA), None);
        map.refresh_snapshots();
        assert_eq!(map.get_public(BAA), Some(&V::Float(2.0)));
        assert_eq!(map.get_public(MOO), Some(&V::Float(3.0)));
    }

    #[test]
    fn a_public_variable_reads_its_old_value_through_get_public_until_the_next_update() {
        let mut map = Map::new();
        map.set_public(BAA, V::Float(1.0));
        map.refresh_snapshots();
        for round in 2u8..5 {
            map.set(BAA, V::Float(f32::from(round)));
            assert_eq!(map.get(BAA), Some(&V::Float(f32::from(round))));
            assert_eq!(map.get_public(BAA), Some(&V::Float(f32::from(round - 1))));
            map.refresh_snapshots();
            assert_eq!(map.get_public(BAA), Some(&V::Float(f32::from(round))));
        }
    }

    #[test]
    fn a_private_variable_never_has_a_public_value() {
        let mut map = Map::new();
        map.set(BAA, V::Float(1.0));
        for _ in 0..3 {
            map.refresh_snapshots();
            assert_eq!(map.get_public(BAA), None);
        }
    }

    #[test]
    fn updating_snapshots_copies_none_for_a_declared_but_unset_public_slot() {
        let mut map = Map::new();
        map.set_access(BAA, Access::Public);
        map.refresh_snapshots();
        assert_eq!(map.get_public(BAA), None);
        map.set(BAA, V::Float(4.0));
        assert_eq!(map.get_public(BAA), None);
        map.refresh_snapshots();
        assert_eq!(map.get_public(BAA), Some(&V::Float(4.0)));
        assert_eq!(map.remove(BAA), Some(V::Float(4.0)));
        map.refresh_snapshots();
        assert_eq!(map.get_public(BAA), None);
    }

    #[test]
    fn making_a_variable_private_drops_the_snapshot_and_public_again_does_not_restore_it() {
        let mut map = Map::new();
        map.set_public(BAA, V::Float(1.0));
        map.refresh_snapshots();
        assert_eq!(map.get_public(BAA), Some(&V::Float(1.0)));
        map.set_access(BAA, Access::Private);
        map.set_access(BAA, Access::Public);
        assert_eq!(map.get_public(BAA), None);
        assert!(map.any_public());
        map.set(BAA, V::Float(2.0));
        map.refresh_snapshots();
        assert_eq!(map.get_public(BAA), Some(&V::Float(2.0)));
    }

    #[test]
    fn len_and_iter_count_only_slots_with_a_value() {
        let mut map = Map::new();
        map.set_access(BAA, Access::Public);
        assert_eq!(map.len(), 0);
        assert!(map.is_empty());
        assert_eq!(map.iter().count(), 0);
        assert_eq!(map.access(BAA), Some(Access::Public));
        assert!(map.any_public());
        assert_eq!(map.get(BAA), None);
        assert!(map.get_mut(BAA).is_none());
        map.set(BAA, V::Float(1.0));
        assert_eq!(map.len(), 1);
        assert!(!map.is_empty());
        assert_eq!(map.iter().count(), 1);
    }

    #[test]
    fn iter_lists_every_variable_with_a_value_in_some_order() {
        let mut map = Map::new();
        let names = [
            VariableName::new("a"),
            VariableName::new("b"),
            VariableName::new("c"),
        ];
        for (i, name) in names.iter().enumerate() {
            map.set(*name, V::Float(i as f32));
        }
        let mut seen: Vec<(HashedStr, f32)> = map
            .iter()
            .map(|(name, value)| (name.hashed(), value.as_f32()))
            .collect();
        seen.sort_by_key(|(name, _)| *name);
        let mut want: Vec<(HashedStr, f32)> = names
            .iter()
            .enumerate()
            .map(|(i, name)| (name.hashed(), i as f32))
            .collect();
        want.sort_by_key(|(name, _)| *name);
        assert_eq!(seen, want);
    }

    #[test]
    fn get_mut_changes_the_latest_value_not_the_snapshot() {
        let mut map = Map::new();
        map.set_public(BAA, V::structure(StructValue::xy(1.0, 2.0)));
        map.refresh_snapshots();
        map.get_mut(BAA)
            .unwrap()
            .set_member_path(&[HashedStr::new("x")], V::Float(9.0));
        assert_eq!(
            map.get(BAA).unwrap().member(HashedStr::new("x")),
            Some(&V::Float(9.0))
        );
        // The snapshot shared the struct; copy-on-write left it unchanged.
        assert_eq!(
            map.get_public(BAA).unwrap().member(HashedStr::new("x")),
            Some(&V::Float(1.0))
        );
    }

    #[test]
    fn remove_returns_the_latest_value_not_the_snapshot_and_forgets_the_slot() {
        let mut map = Map::new();
        map.set_public(BAA, V::Float(1.0));
        map.refresh_snapshots();
        map.set(BAA, V::Float(2.0));
        assert_eq!(map.remove(BAA), Some(V::Float(2.0)));
        assert_eq!(map.get_public(BAA), None);
        assert_eq!(map.access(BAA), None);
        assert!(!map.any_public());
        assert_eq!(map.remove(MOO), None);
    }

    #[test]
    fn removing_a_declared_but_valueless_slot_returns_nothing_and_forgets_its_access() {
        let mut map = Map::new();
        map.set_access(BAA, Access::Public);
        assert_eq!(map.remove(BAA), None);
        assert_eq!(map.access(BAA), None);
    }

    #[test]
    fn clear_forgets_values_and_access() {
        let mut map = Map::new();
        map.set_public(BAA, V::Float(1.0));
        map.set_access(MOO, Access::Public);
        map.refresh_snapshots();
        map.clear();
        assert!(map.is_empty());
        assert_eq!(map.access(BAA), None);
        assert_eq!(map.access(MOO), None);
        assert_eq!(map.get_public(BAA), None);
        assert!(!map.any_public());
    }

    #[test]
    fn any_public_follows_the_access_of_every_slot() {
        let mut map = Map::new();
        assert!(!map.any_public());
        map.set(BAA, V::Float(1.0));
        assert!(!map.any_public());
        map.set_access(MOO, Access::Public);
        assert!(map.any_public());
        map.set_access(MOO, Access::Private);
        assert!(!map.any_public());
    }

    #[test]
    fn equality_includes_values_access_and_snapshots() {
        let mut a = Map::new();
        let mut b = Map::new();
        a.set_public(BAA, V::Float(1.0));
        b.set_public(BAA, V::Float(1.0));
        assert_eq!(a, b);
        a.refresh_snapshots();
        assert_ne!(a, b);
        b.refresh_snapshots();
        assert_eq!(a, b);
        b.set_access(MOO, Access::Private);
        assert_ne!(a, b);
        assert_eq!(Map::new(), Map::default());
    }

    #[test]
    fn a_clone_is_independent() {
        let mut original = Map::new();
        original.set_public(BAA, V::Float(1.0));
        original.refresh_snapshots();
        let mut copy = original.clone();
        assert_eq!(copy, original);
        copy.set(BAA, V::Float(2.0));
        copy.set(MOO, V::Float(3.0));
        copy.refresh_snapshots();
        assert_eq!(original.get(BAA), Some(&V::Float(1.0)));
        assert_eq!(original.get_public(BAA), Some(&V::Float(1.0)));
        assert_eq!(original.get(MOO), None);
    }

    #[test]
    fn debug_shows_value_access_and_snapshot_by_key() {
        let mut map = Map::new();
        map.set(BAA, V::Float(1.0));
        assert_eq!(
            format!("{map:?}"),
            format!("{{{:?}: (Some(Float(1.0)), Private, None)}}", BAA)
        );
        map.set_public(MOO, V::Float(2.0));
        map.refresh_snapshots();
        let text = format!("{map:?}");
        assert!(
            text.contains(&format!(
                "{MOO:?}: (Some(Float(2.0)), Public, Some(Float(2.0)))"
            )),
            "{text}"
        );
        assert_eq!(format!("{:?}", Map::new()), "{}");
    }

    #[test]
    fn a_lone_map_as_a_store_serves_every_actor_and_the_detached_map() {
        let mut map = Map::new();
        VariableStore::set(&mut map, 7, BAA, V::Float(1.0));
        assert_eq!(map.access(BAA), Some(Access::Private));
        map.set_access(BAA, Access::Public);
        VariableStore::set(&mut map, 8, BAA, V::Float(2.0));
        assert_eq!(map.access(BAA), Some(Access::Public));
        for actor in [7, 8, 99] {
            assert_eq!(VariableStore::get(&map, actor, BAA), Some(&V::Float(2.0)));
        }
        assert_eq!(map.get_local(BAA), Some(&V::Float(2.0)));
        assert_eq!(VariableStore::<TestHost>::get_local(&map, MOO), None);
        map.set_local(MOO, V::Float(5.0));
        assert_eq!(map.access(MOO), Some(Access::Private));
        assert_eq!(VariableStore::get(&map, 1, MOO), Some(&V::Float(5.0)));
        assert_eq!(VariableStore::get_public(&map, 3, BAA), None);
        map.refresh_snapshots();
        assert_eq!(
            VariableStore::get_public(&map, 3, BAA),
            Some(&V::Float(2.0))
        );
        assert_eq!(VariableStore::get_public(&map, 3, MOO), None);
    }

    #[test]
    fn no_variables_ignores_writes_and_reads_nothing() {
        let mut store = NoVariables;
        VariableStore::<TestHost>::set(&mut store, 1, BAA, V::Float(1.0));
        store.set_local(BAA, V::Float(1.0));
        assert_eq!(VariableStore::<TestHost>::get(&store, 1, BAA), None);
        assert_eq!(VariableStore::<TestHost>::get_public(&store, 1, BAA), None);
        assert_eq!(VariableStore::<TestHost>::get_local(&store, BAA), None);
    }

    #[test]
    fn storage_creates_a_map_on_first_write_with_a_private_slot() {
        let mut store = VariableStorage::<TestHost>::new();
        assert!(store.actor(1).is_none());
        store.set(1, BAA, V::Float(1.0));
        assert_eq!(
            store.actor(1).and_then(|map| map.access(BAA)),
            Some(Access::Private)
        );
        assert!(store.actor(2).is_none());
        assert!(store.actor_mut(2).is_empty());
        assert!(store.actor(2).is_some());
    }

    #[test]
    fn storage_snapshots_are_refreshed_for_every_actor_but_not_the_detached_map() {
        let mut store = VariableStorage::<TestHost>::with_local();
        for actor in [1, 2] {
            store
                .actor_mut(actor)
                .set_public(BAA, V::Float(f32::from(actor as u8)));
        }
        store.local_mut().set_public(BAA, V::Float(9.0));
        assert_eq!(store.get_public(1, BAA), None);
        store.refresh_snapshots();
        assert_eq!(store.get_public(1, BAA), Some(&V::Float(1.0)));
        assert_eq!(store.get_public(2, BAA), Some(&V::Float(2.0)));
        assert_eq!(store.local().unwrap().get_public(BAA), None);
        assert_eq!(store.local().unwrap().get(BAA), Some(&V::Float(9.0)));
    }

    #[test]
    fn storage_without_a_detached_map_ignores_local_writes() {
        let mut store = VariableStorage::<TestHost>::new();
        assert!(store.local().is_none());
        store.set_local(BAA, V::Float(1.0));
        assert_eq!(store.get_local(BAA), None);
        assert!(store.local().is_none());
        store.local_mut().set(MOO, V::Float(2.0));
        assert_eq!(store.get_local(MOO), Some(&V::Float(2.0)));
        store.set_local(BAA, V::Float(3.0));
        assert_eq!(store.get_local(BAA), Some(&V::Float(3.0)));
        assert_eq!(store.local().unwrap().access(BAA), Some(Access::Private));
        assert_eq!(store.remove_local().map(|map| map.len()), Some(2));
        assert!(store.local().is_none());
        assert_eq!(store.get_local(BAA), None);
        assert!(store.remove_local().is_none());
    }

    #[test]
    fn storage_with_local_starts_with_an_empty_detached_map() {
        let store = VariableStorage::<TestHost>::with_local();
        assert!(store.local().is_some_and(VariableMap::is_empty));
        assert!(VariableStorage::<TestHost>::new().local().is_none());
    }

    #[test]
    fn storage_default_is_new() {
        let store = VariableStorage::<TestHost>::default();
        assert!(store.local().is_none());
        assert!(store.actor(0).is_none());
    }

    #[test]
    fn storage_actors_and_the_detached_map_do_not_share_variables() {
        let mut store = VariableStorage::<TestHost>::with_local();
        store.set(1, BAA, V::Float(1.0));
        store.set_local(BAA, V::Float(2.0));
        assert_eq!(store.get(1, BAA), Some(&V::Float(1.0)));
        assert_eq!(store.get_local(BAA), Some(&V::Float(2.0)));
        assert_eq!(store.get(2, BAA), None);
    }

    #[test]
    fn storage_remove_actor_returns_its_map() {
        let mut store = VariableStorage::<TestHost>::new();
        store.set(1, BAA, V::Float(1.0));
        let map = store.remove_actor(1).expect("had a map");
        assert_eq!(map.get(BAA), Some(&V::Float(1.0)));
        assert!(store.remove_actor(1).is_none());
    }

    #[test]
    fn storage_debug_lists_the_actors_and_the_detached_map() {
        let mut store = VariableStorage::<TestHost>::new();
        assert_eq!(
            format!("{store:?}"),
            "VariableStorage { actors: {}, local: None }"
        );
        store.set(1, BAA, V::Float(1.0));
        let text = format!("{store:?}");
        assert!(
            text.starts_with("VariableStorage { actors: {1: {"),
            "{text}"
        );
        assert!(text.ends_with("local: None }"), "{text}");
    }

    #[test]
    fn storage_serves_many_actors() {
        let mut store = VariableStorage::<TestHost>::new();
        for actor in 0..1000 {
            store.set(actor, BAA, V::Float(actor as f32));
        }
        for actor in 0..1000 {
            assert_eq!(store.get(actor, BAA), Some(&V::Float(actor as f32)));
        }
    }
}
