//! [`Value`], what the evaluator computes with, and [`StructValue`].

use std::fmt::{self, Debug};
use std::sync::Arc;

use super::host::Host;
use super::name::hash_lowered;
use crate::catalog::DefaultReturn;
use crate::hash::HashedStr;

mod members;
mod walks;

#[cfg(feature = "fuzz")]
pub(crate) use members::MemberStoreCheck;
pub use members::StructValue;
pub use walks::DistinctStructs;

/// A Molang value.
///
/// A string is kept only as its 64-bit FNV-1 hash ([`Value::Hash`]); the text is gone.
///
/// With unit host handles (`Value<NoHost>`) a value is 16 bytes; floats and hashes never allocate,
/// and shared payloads sit behind [`Arc`], so a `Value` is `Send + Sync` and cheap to clone.
#[non_exhaustive]
pub enum Value<H: Host> {
    /// A number; booleans are `1.0` / `0.0`.
    Float(f32),
    /// A string, as its FNV-1 hash.
    Hash(HashedStr),
    /// An actor handle of the host's choosing.
    Actor(H::ActorRef),
    /// An item stack (a valid left side of `->`).
    Item(H::ItemRef),
    /// An array of actors: the only value `for_each` iterates.
    ActorArray(Arc<Vec<H::ActorRef>>),
    /// A struct (`v.a.b`, `query.spellcolor`, …).
    Struct(Arc<StructValue<H>>),
    /// A row-major 4×4 matrix (bone queries).
    Matrix(Arc<[f32; 16]>),
    /// A texture / geometry / material / array resource; reads as 0 in arithmetic.
    Resource(ResourceRef),
}

/// The kind of a [`Value`], for messages and dispatch.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ValueKind {
    /// [`Value::Float`].
    Float,
    /// [`Value::Hash`].
    Hash,
    /// [`Value::Actor`].
    Actor,
    /// [`Value::Item`].
    Item,
    /// [`Value::ActorArray`].
    ActorArray,
    /// [`Value::Struct`].
    Struct,
    /// [`Value::Matrix`].
    Matrix,
    /// [`Value::Resource`].
    Resource,
}

impl ValueKind {
    /// A lower-case name of the kind.
    pub const fn name(self) -> &'static str {
        match self {
            Self::Float => "float",
            Self::Hash => "hash",
            Self::Actor => "actor",
            Self::Item => "item",
            Self::ActorArray => "actor array",
            Self::Struct => "struct",
            Self::Matrix => "matrix",
            Self::Resource => "resource",
        }
    }
}

impl fmt::Display for ValueKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

/// A handle to a resource (`geometry.x`, `material.x`, `texture.x`, an `array.x` element): the hash
/// of its full canonical name.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub struct ResourceRef(HashedStr);

impl ResourceRef {
    /// The handle of the resource with this full name (`"texture.default"`), canonicalised as the
    /// lexer does: ASCII `A`–`Z` are lowered and the text ends at its first NUL byte.
    pub const fn new(name: &str) -> Self {
        Self(hash_lowered("", name.as_bytes()))
    }

    /// The handle with this hash of the canonical name, **unchecked**: a hash of a name that is not
    /// canonical is a handle no expression reaches. Prefer [`new`](Self::new) when the text is at
    /// hand.
    pub const fn from_raw_hash(hash: HashedStr) -> Self {
        Self(hash)
    }

    /// The hash of the resource's name.
    pub const fn hashed(self) -> HashedStr {
        self.0
    }
}

const IDENTITY_MATRIX: [f32; 16] = [
    1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0,
];

impl<H: Host> Value<H> {
    /// Float 0.0, the result of every failure path.
    pub const ZERO: Self = Self::Float(0.0);

    /// Float 1.0 (boolean true).
    pub const ONE: Self = Self::Float(1.0);

    /// A boolean: `1.0` or `0.0`.
    pub const fn bool(b: bool) -> Self {
        Self::Float(if b { 1.0 } else { 0.0 })
    }

    /// The value of the string `text`: its FNV-1 hash, hashed as written (no case folding).
    pub const fn string(text: &str) -> Self {
        Self::Hash(HashedStr::new(text))
    }

    /// An actor array.
    pub fn actor_array(actors: impl IntoIterator<Item = H::ActorRef>) -> Self {
        Self::ActorArray(Arc::new(actors.into_iter().collect()))
    }

    /// A struct value.
    pub fn structure(members: StructValue<H>) -> Self {
        Self::Struct(Arc::new(members))
    }

    /// The identity matrix, the value of a matrix the host could not read.
    pub fn identity_matrix() -> Self {
        Self::Matrix(Arc::new(IDENTITY_MATRIX))
    }

    /// The kind of this value.
    pub const fn kind(&self) -> ValueKind {
        match self {
            Self::Float(_) => ValueKind::Float,
            Self::Hash(_) => ValueKind::Hash,
            Self::Actor(_) => ValueKind::Actor,
            Self::Item(_) => ValueKind::Item,
            Self::ActorArray(_) => ValueKind::ActorArray,
            Self::Struct(_) => ValueKind::Struct,
            Self::Matrix(_) => ValueKind::Matrix,
            Self::Resource(_) => ValueKind::Resource,
        }
    }

    /// The value as arithmetic reads it, never `None`: a float is itself, a string is the low 32
    /// bits of its hash reinterpreted as an `f32`, and every other kind is 0.0.
    #[inline]
    pub const fn as_f32(&self) -> f32 {
        match self {
            Self::Float(x) => *x,
            Self::Hash(h) => f32::from_bits(h.as_u64() as u32),
            _ => 0.0,
        }
    }

    /// `as_f32() != 0.0`: NaN is true and `-0.0` false.
    #[inline]
    pub fn truthy(&self) -> bool {
        self.as_f32() != 0.0
    }

    /// The number of a float value; `None` for every other kind (see [`as_f32`](Self::as_f32)).
    pub const fn as_float(&self) -> Option<f32> {
        match self {
            Self::Float(x) => Some(*x),
            _ => None,
        }
    }

    /// The hash of a string value.
    pub const fn as_hash(&self) -> Option<HashedStr> {
        match self {
            Self::Hash(h) => Some(*h),
            _ => None,
        }
    }

    /// The actor handle of an actor value.
    pub const fn as_actor(&self) -> Option<H::ActorRef> {
        match self {
            Self::Actor(a) => Some(*a),
            _ => None,
        }
    }

    /// The item handle of an item value.
    pub const fn as_item(&self) -> Option<H::ItemRef> {
        match self {
            Self::Item(i) => Some(*i),
            _ => None,
        }
    }

    /// The entries of an actor array.
    pub fn as_actor_array(&self) -> Option<&[H::ActorRef]> {
        match self {
            Self::ActorArray(a) => Some(a.as_slice()),
            _ => None,
        }
    }

    /// The members of a struct value.
    pub fn as_struct(&self) -> Option<&StructValue<H>> {
        match self {
            Self::Struct(s) => Some(s),
            _ => None,
        }
    }

    /// The elements of a matrix value, row-major.
    pub fn as_matrix(&self) -> Option<&[f32; 16]> {
        match self {
            Self::Matrix(m) => Some(m),
            _ => None,
        }
    }

    /// The handle of a resource value.
    pub const fn as_resource(&self) -> Option<ResourceRef> {
        match self {
            Self::Resource(r) => Some(*r),
            _ => None,
        }
    }

    /// The steps a store costs beyond its instruction: one per actor-array entry, each converted by
    /// [`EvalCx::storable`](super::EvalCx::storable).
    #[inline]
    pub(crate) fn store_cost(&self) -> u64 {
        match self {
            Self::ActorArray(actors) => actors.len() as u64,
            _ => 0,
        }
    }

    /// Molang `==`: dispatches on the **right** operand's kind and reads the left operand's payload
    /// as that kind.
    /// - Right a float: the left payload's low 32 bits as an `f32`, compared as floats (NaN is
    ///   unequal to everything), so `'' == 0`, `'' == -0` and `'a' == ('a' * 1)` are true.
    /// - Right a string: the left payload's 64 bits against the hash; a float's payload is its 32
    ///   bits zero-extended, so `0 == ''` is true (the hash of `''` is 0) and `-0 == ''` is false.
    /// - Right an actor: the identity of the resolved actors; unresolvable handles are unequal.
    ///
    /// Every other pairing is unequal; an item, array, struct, matrix or resource is unequal even
    /// to itself.
    ///
    /// `resolve` maps an actor handle to the live actor
    /// ([`HostAccess::resolve_actor`](super::HostAccess::resolve_actor)).
    pub fn molang_eq(
        &self,
        rhs: &Self,
        mut resolve: impl FnMut(H::ActorRef) -> Option<H::ActorRef>,
    ) -> bool {
        match (self, rhs) {
            (Self::Float(a), Self::Float(b)) => a == b,
            (Self::Hash(a), Self::Float(b)) => f32::from_bits(a.as_u64() as u32) == *b,
            (Self::Hash(a), Self::Hash(b)) => a == b,
            (Self::Float(a), Self::Hash(b)) => u64::from(a.to_bits()) == b.as_u64(),
            (Self::Actor(a), Self::Actor(b)) => match (resolve(*a), resolve(*b)) {
                (Some(a), Some(b)) => a == b,
                _ => false,
            },
            _ => false,
        }
    }

    /// Replaces every top-level actor handle (an actor, or an actor array's entries) by
    /// `f(handle)`. Array entries for which `f` returns `None` are dropped; a single actor for
    /// which it returns `None` is kept.
    #[must_use]
    pub fn map_actors(self, mut f: impl FnMut(H::ActorRef) -> Option<H::ActorRef>) -> Self {
        match self {
            Self::Actor(actor) => Self::Actor(f(actor).unwrap_or(actor)),
            Self::ActorArray(actors) => {
                Self::ActorArray(Arc::new(actors.iter().filter_map(|a| f(*a)).collect()))
            }
            other => other,
        }
    }
}

// Manual impls: derives would demand `H: Clone` / `H: PartialEq` / `H: Debug` of the marker type.
impl<H: Host> Clone for Value<H> {
    #[inline]
    fn clone(&self) -> Self {
        match self {
            Self::Float(x) => Self::Float(*x),
            Self::Hash(h) => Self::Hash(*h),
            Self::Actor(a) => Self::Actor(*a),
            Self::Item(i) => Self::Item(*i),
            Self::ActorArray(a) => Self::ActorArray(Arc::clone(a)),
            Self::Struct(s) => Self::Struct(Arc::clone(s)),
            Self::Matrix(m) => Self::Matrix(Arc::clone(m)),
            Self::Resource(r) => Self::Resource(*r),
        }
    }
}

impl<H: Host> Default for Value<H> {
    fn default() -> Self {
        Self::ZERO
    }
}

impl<H: Host> From<f32> for Value<H> {
    fn from(x: f32) -> Self {
        Self::Float(x)
    }
}

impl<H: Host> From<bool> for Value<H> {
    fn from(b: bool) -> Self {
        Self::bool(b)
    }
}

impl<H: Host> From<HashedStr> for Value<H> {
    fn from(hash: HashedStr) -> Self {
        Self::Hash(hash)
    }
}

impl<H: Host> From<StructValue<H>> for Value<H> {
    fn from(members: StructValue<H>) -> Self {
        Self::structure(members)
    }
}

/// The "no subject" result of a query.
impl<H: Host> From<DefaultReturn> for Value<H> {
    fn from(default: DefaultReturn) -> Self {
        match default {
            DefaultReturn::Float0 => Self::Float(0.0),
            DefaultReturn::Float1 => Self::Float(1.0),
            DefaultReturn::FloatNeg1 => Self::Float(-1.0),
            DefaultReturn::EmptyString => Self::Hash(HashedStr::EMPTY),
            DefaultReturn::EmptyActorArray => Self::ActorArray(Arc::new(Vec::new())),
            DefaultReturn::StructRgba0 => Self::structure(StructValue::rgba(0.0, 0.0, 0.0, 0.0)),
        }
    }
}

#[cfg(test)]
pub(crate) mod test_support {
    use super::*;
    use crate::vm::test_support::TestHost;

    pub(crate) type V = Value<TestHost>;

    pub(crate) type M = StructValue<TestHost>;

    pub(crate) fn h(name: &str) -> HashedStr {
        HashedStr::new(name)
    }

    pub(crate) fn arc_of(value: &V) -> &Arc<M> {
        match value {
            Value::Struct(members) => members,
            other => panic!("not a struct: {other:?}"),
        }
    }

    /// Runs `f` on a thread with a 512 KiB stack: a recursion over 100,000 levels would overflow
    /// it.
    pub(crate) fn on_a_small_stack(f: impl FnOnce() + Send + 'static) {
        std::thread::Builder::new()
            .stack_size(512 * 1024)
            .spawn(f)
            .expect("spawns")
            .join()
            .expect("did not overflow");
    }

    /// A chain of `levels` structs, each holding the next as member `a`; the innermost holds 1.0.
    pub(crate) fn chain(levels: usize) -> V {
        let mut value = V::Float(1.0);
        for _ in 0..levels {
            value = V::structure(M::from([("a", value)]));
        }
        value
    }

    /// `v.a.z = 1; loop(rounds, { v.b = v.a; v.a.x = v.b; v.a.y = v.b; })`: every round stores the
    /// struct into two members of itself, so the paths to the innermost struct double.
    pub(crate) fn doubling(rounds: usize) -> V {
        let mut a = V::ZERO;
        a.set_member_path(&[h("z")], V::Float(1.0));
        for _ in 0..rounds {
            let b = a.clone();
            a.set_member_path(&[h("x")], b.clone());
            a.set_member_path(&[h("y")], b);
        }
        a
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use super::*;
    use crate::vm::{
        NoHost,
        test_support::{TestHost, alive},
        value::test_support::*,
    };

    #[test]
    fn value_is_sixteen_bytes_with_unit_handles() {
        assert_eq!(size_of::<Value<NoHost>>(), 16);
        assert_eq!(size_of::<V>(), 16);
        let kinds = [
            (V::Float(1.0).kind(), ValueKind::Float),
            (V::Hash(HashedStr::from_u64(1)).kind(), ValueKind::Hash),
            (V::Actor(1).kind(), ValueKind::Actor),
            (V::Item(1).kind(), ValueKind::Item),
            (V::actor_array([1, 2]).kind(), ValueKind::ActorArray),
            (V::structure(StructValue::new()).kind(), ValueKind::Struct),
            (V::identity_matrix().kind(), ValueKind::Matrix),
            (
                V::Resource(ResourceRef::new("texture.default")).kind(),
                ValueKind::Resource,
            ),
        ];
        for (got, want) in kinds {
            assert_eq!(got, want);
        }
    }

    #[test]
    fn value_kind_is_one_byte() {
        assert_eq!(size_of::<ValueKind>(), 1);
    }

    #[test]
    fn kind_names_and_display() {
        let names = [
            (ValueKind::Float, "float"),
            (ValueKind::Hash, "hash"),
            (ValueKind::Actor, "actor"),
            (ValueKind::Item, "item"),
            (ValueKind::ActorArray, "actor array"),
            (ValueKind::Struct, "struct"),
            (ValueKind::Matrix, "matrix"),
            (ValueKind::Resource, "resource"),
        ];
        for (kind, name) in names {
            assert_eq!(kind.name(), name);
            assert_eq!(kind.to_string(), name);
        }
    }

    #[test]
    fn value_is_send_and_sync() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<Value<NoHost>>();
        assert_send_sync::<V>();
        assert_send_sync::<StructValue<TestHost>>();
    }

    #[test]
    fn string_values_are_hashes() {
        assert_eq!(
            V::string("a"),
            V::Hash(HashedStr::from_u64(12_638_153_115_695_167_422))
        );
        assert_eq!(V::string(""), V::Hash(HashedStr::from_u64(0)));
        assert_eq!(V::string("moo").as_hash(), Some(h("moo")));
        assert_ne!(V::string("abc"), V::string("ABC"));
        assert_eq!(V::from(h("moo")), V::string("moo"));
    }

    #[test]
    fn as_f32_reinterprets_hash_low_bits() {
        assert_eq!(V::Float(2.5).as_f32(), 2.5);
        assert_eq!(
            V::Hash(HashedStr::from_u64(0xdead_beef_3fc0_0000)).as_f32(),
            1.5
        );
        let hash = h("a").as_u64();
        assert_eq!(V::string("a").as_f32().to_bits(), hash as u32);
        assert_eq!(V::Actor(3).as_f32(), 0.0);
        assert_eq!(V::Item(3).as_f32(), 0.0);
        assert_eq!(V::actor_array([1]).as_f32(), 0.0);
        assert_eq!(V::structure(StructValue::xy(1.0, 2.0)).as_f32(), 0.0);
        assert_eq!(V::identity_matrix().as_f32(), 0.0);
        assert_eq!(V::Resource(ResourceRef::new("geometry.x")).as_f32(), 0.0);
    }

    #[test]
    fn as_f32_of_a_non_float_is_positive_zero_not_negative_zero() {
        let others = [
            V::Actor(3),
            V::Item(3),
            V::actor_array([]),
            V::structure(M::new()),
            V::identity_matrix(),
            V::Resource(ResourceRef::new("texture.default")),
        ];
        for value in others {
            assert_eq!(value.as_f32().to_bits(), 0, "{value:?}");
        }
        // The empty string hashes to 0.
        assert_eq!(V::string("").as_f32().to_bits(), 0);
    }

    #[test]
    fn as_f32_of_a_float_keeps_every_bit_pattern() {
        for bits in [0u32, 0x8000_0000, 1, 0x7f80_0000, 0xff80_0000, 0x7fc0_0001] {
            assert_eq!(V::Float(f32::from_bits(bits)).as_f32().to_bits(), bits);
        }
    }

    #[test]
    fn truthiness() {
        assert!(V::Float(1.0).truthy());
        assert!(V::Float(0.000_000_1).truthy());
        assert!(V::Float(f32::NAN).truthy());
        assert!(!V::Float(0.0).truthy());
        assert!(!V::Float(-0.0).truthy());
        assert!(!V::string("").truthy());
        assert!(!V::Actor(1).truthy());
        assert_eq!(V::bool(true), V::Float(1.0));
        assert_eq!(V::from(false), V::Float(0.0));
    }

    #[test]
    fn truthiness_of_a_hash_follows_its_low_32_bits() {
        // The low 32 bits are -0.0, the high bits are ignored.
        assert!(!V::Hash(HashedStr::from_u64(0x8000_0000)).truthy());
        assert!(!V::Hash(HashedStr::from_u64(0x1_0000_0000)).truthy());
        // The smallest denormal is not zero.
        assert!(V::Hash(HashedStr::from_u64(1)).truthy());
        // A NaN pattern in the low bits is true.
        assert!(V::Hash(HashedStr::from_u64(0x7fc0_0000)).truthy());
        // A real string reads the low bits of its own hash.
        let low = f32::from_bits(h("moo").as_u64() as u32);
        assert_eq!(V::string("moo").truthy(), low != 0.0);
    }

    #[test]
    fn every_other_kind_is_falsy() {
        let others = [
            V::Actor(1),
            V::Item(1),
            V::actor_array([1, 2]),
            V::structure(M::xy(1.0, 1.0)),
            V::identity_matrix(),
            V::Resource(ResourceRef::new("texture.default")),
        ];
        for value in others {
            assert!(!value.truthy(), "{value:?}");
        }
    }

    #[test]
    fn constants_and_constructors() {
        assert_eq!(V::ZERO, V::Float(0.0));
        assert_eq!(V::ONE, V::Float(1.0));
        assert_eq!(V::default(), V::ZERO);
        assert_eq!(V::from(2.5_f32), V::Float(2.5));
        assert_eq!(V::from(true), V::ONE);
        assert_eq!(V::bool(false), V::ZERO);
        assert_eq!(V::from(M::xy(1.0, 2.0)), V::structure(M::xy(1.0, 2.0)));
        assert_eq!(V::string(""), V::Hash(HashedStr::EMPTY));
        assert_eq!(V::actor_array([]).as_actor_array(), Some(&[][..]));
        assert_eq!(
            V::actor_array([3, 1, 2]).as_actor_array(),
            Some(&[3, 1, 2][..])
        );
    }

    #[test]
    fn identity_matrix_has_ones_on_the_diagonal_only() {
        let Value::Matrix(m) = V::identity_matrix() else {
            panic!("not a matrix")
        };
        for (i, x) in m.iter().enumerate() {
            assert_eq!(*x, if i % 5 == 0 { 1.0 } else { 0.0 }, "element {i}");
        }
    }

    #[test]
    fn string_hashes_as_written_with_no_case_folding() {
        assert_eq!(V::string("ABC").as_hash(), Some(h("ABC")));
        assert_ne!(V::string("ABC").as_hash(), Some(h("abc")));
    }

    #[test]
    fn default_return_values() {
        assert_eq!(V::from(DefaultReturn::Float0), V::Float(0.0));
        assert_eq!(V::from(DefaultReturn::Float1), V::Float(1.0));
        assert_eq!(V::from(DefaultReturn::FloatNeg1), V::Float(-1.0));
        assert_eq!(V::from(DefaultReturn::EmptyString), V::string(""));
        assert_eq!(
            V::from(DefaultReturn::EmptyActorArray).as_actor_array(),
            Some(&[][..])
        );
        let rgba = V::from(DefaultReturn::StructRgba0);
        for name in ["r", "g", "b", "a"] {
            assert_eq!(rgba.member(h(name)), Some(&V::Float(0.0)));
        }
        // The float view agrees with the metadata's own.
        for d in [
            DefaultReturn::Float0,
            DefaultReturn::Float1,
            DefaultReturn::FloatNeg1,
            DefaultReturn::EmptyString,
            DefaultReturn::EmptyActorArray,
            DefaultReturn::StructRgba0,
        ] {
            assert_eq!(V::from(d).as_f32(), d.as_f32());
        }
    }

    #[test]
    fn the_struct_default_builds_the_struct_members_of_the_query_defaults() {
        assert_eq!(
            V::from(DefaultReturn::StructRgba0),
            V::structure(M::rgba(0.0, 0.0, 0.0, 0.0))
        );
        assert_eq!(
            V::from(DefaultReturn::StructRgba0)
                .as_struct()
                .map(StructValue::len),
            Some(4)
        );
        assert_eq!(
            V::from(DefaultReturn::EmptyString).as_hash(),
            Some(HashedStr::EMPTY)
        );
    }

    #[test]
    fn typed_accessors() {
        assert_eq!(V::Actor(4).as_actor(), Some(4));
        assert_eq!(V::Float(4.0).as_actor(), None);
        assert_eq!(V::Item(4).as_item(), Some(4));
        assert_eq!(V::Hash(HashedStr::from_u64(4)).as_item(), None);
        assert_eq!(V::actor_array([4, 5]).as_actor_array(), Some(&[4, 5][..]));
        assert_eq!(V::Float(4.0).as_actor_array(), None);
        assert_eq!(V::Float(4.0).as_hash(), None);
        assert_eq!(V::Float(4.0).as_float(), Some(4.0));
        assert_eq!(V::Hash(HashedStr::from_u64(4)).as_float(), None);
        assert_eq!(V::identity_matrix().as_matrix().map(|m| m[0]), Some(1.0));
        assert_eq!(V::Float(4.0).as_matrix(), None);
        let texture = ResourceRef::new("texture.default");
        assert_eq!(V::Resource(texture).as_resource(), Some(texture));
        assert_eq!(V::Float(0.0).as_resource(), None);
        assert_eq!(
            ResourceRef::new("texture.default").hashed(),
            h("texture.default")
        );
        assert_eq!(
            ResourceRef::from_raw_hash(h("geometry.x")),
            ResourceRef::new("geometry.x")
        );
        assert_eq!(ValueKind::ActorArray.to_string(), "actor array");
        assert!(format!("{:?}", V::string("a")).starts_with("Hash(0x"));
    }

    #[test]
    fn every_accessor_answers_for_exactly_one_kind() {
        let sample = [
            V::Float(1.0),
            V::string("a"),
            V::Actor(1),
            V::Item(1),
            V::actor_array([1]),
            V::structure(M::new()),
            V::identity_matrix(),
            V::Resource(ResourceRef::new("texture.default")),
        ];
        for (i, value) in sample.iter().enumerate() {
            assert_eq!(value.as_float().is_some(), i == 0, "as_float {value:?}");
            assert_eq!(value.as_hash().is_some(), i == 1, "as_hash {value:?}");
            assert_eq!(value.as_actor().is_some(), i == 2, "as_actor {value:?}");
            assert_eq!(value.as_item().is_some(), i == 3, "as_item {value:?}");
            assert_eq!(
                value.as_actor_array().is_some(),
                i == 4,
                "as_actor_array {value:?}"
            );
            assert_eq!(value.as_struct().is_some(), i == 5, "as_struct {value:?}");
            assert_eq!(value.as_matrix().is_some(), i == 6, "as_matrix {value:?}");
            assert_eq!(
                value.as_resource().is_some(),
                i == 7,
                "as_resource {value:?}"
            );
        }
    }

    #[test]
    fn resource_refs_are_hashes_of_their_names() {
        let texture = ResourceRef::new("texture.default");
        assert_eq!(texture.hashed(), h("texture.default"));
        assert_eq!(texture, ResourceRef::from_raw_hash(h("texture.default")));
        assert_ne!(texture, ResourceRef::new("texture.other"));
        let copy = texture;
        assert_eq!(copy, texture);
        assert!(format!("{texture:?}").starts_with("ResourceRef("));
        assert_eq!(V::Resource(texture), V::Resource(copy));
    }

    #[test]
    fn a_resource_ref_from_text_is_canonical() {
        let texture = ResourceRef::new("texture.default");
        for spelling in [
            "Texture.Default",
            "TEXTURE.DEFAULT",
            "texture.default\0ignored",
            "TeXtUrE.dEfAuLt\0",
        ] {
            assert_eq!(ResourceRef::new(spelling), texture, "{spelling:?}");
        }
        assert_ne!(
            ResourceRef::from_raw_hash(h("Texture.Default")),
            texture,
            "the raw hash is not canonicalised"
        );
        // Only ASCII letters are lowered.
        assert_ne!(ResourceRef::new("texture.Ä"), ResourceRef::new("texture.ä"));
        assert_eq!(ResourceRef::new("").hashed(), HashedStr::new(""));
    }

    #[test]
    fn clone_shares_the_payload_of_every_shared_arm() {
        let array = V::actor_array([1, 2]);
        let (Value::ActorArray(a), Value::ActorArray(b)) = (&array, &array.clone()) else {
            panic!("arrays")
        };
        assert!(Arc::ptr_eq(a, b));
        let matrix = V::identity_matrix();
        let (Value::Matrix(a), Value::Matrix(b)) = (&matrix, &matrix.clone()) else {
            panic!("matrices")
        };
        assert!(Arc::ptr_eq(a, b));
        let structure = V::structure(M::xy(1.0, 2.0));
        assert!(Arc::ptr_eq(arc_of(&structure), arc_of(&structure.clone())));
        for value in [
            V::Float(1.5),
            V::string("a"),
            V::Actor(2),
            V::Item(3),
            V::Resource(ResourceRef::new("texture.default")),
        ] {
            assert_eq!(value.clone(), value);
        }
    }

    #[test]
    fn string_equality_compares_hashes() {
        assert!(V::string("moo").molang_eq(&V::string("moo"), alive));
        assert!(!V::string("moo").molang_eq(&V::string("rabbit"), alive));
        // Hashes that agree in the low 32 bits only are different strings.
        assert!(
            !V::Hash(HashedStr::from_u64(0x1_0000_0001))
                .molang_eq(&V::Hash(HashedStr::from_u64(0x2_0000_0001)), alive)
        );
    }

    #[test]
    fn equality_dispatches_on_the_right_operand() {
        assert!(V::Float(1.5).molang_eq(&V::Float(1.5), alive));
        assert!(V::Float(0.0).molang_eq(&V::Float(-0.0), alive));
        assert!(!V::Float(f32::NAN).molang_eq(&V::Float(f32::NAN), alive));
        // Right a float: the left hash's low 32 bits.
        assert!(V::Hash(HashedStr::from_u64(0x3fc0_0000)).molang_eq(&V::Float(1.5), alive));
        assert!(
            V::Hash(HashedStr::from_u64(0xffff_ffff_3fc0_0000)).molang_eq(&V::Float(1.5), alive)
        );
        assert!(V::string("").molang_eq(&V::Float(0.0), alive));
        assert!(V::string("").molang_eq(&V::Float(-0.0), alive));
        // Right a string: the left float's bits, zero-extended, against the 64-bit hash.
        assert!(V::Float(0.0).molang_eq(&V::string(""), alive));
        assert!(!V::Float(-0.0).molang_eq(&V::string(""), alive));
        assert!(V::Float(1.5).molang_eq(&V::Hash(HashedStr::from_u64(0x3fc0_0000)), alive));
        assert!(!V::Float(1.5).molang_eq(&V::Hash(HashedStr::from_u64(0x1_3fc0_0000)), alive));
        let a = V::string("a");
        assert!(a.molang_eq(&V::Float(a.as_f32()), alive));
        assert!(!V::Float(a.as_f32()).molang_eq(&a, alive));
        // No payload bits: unequal.
        assert!(!V::Float(0.0).molang_eq(&V::Actor(1), alive));
        assert!(!V::Actor(1).molang_eq(&V::Float(0.0), alive));
        // Any other kind on the right is unequal, even to itself.
        let item = V::Item(7);
        assert!(!item.molang_eq(&item, alive));
        let s = V::structure(StructValue::xy(1.0, 2.0));
        assert!(!s.molang_eq(&s, alive));
        let array = V::actor_array([1, 2]);
        assert!(!array.molang_eq(&array, alive));
        let m = V::identity_matrix();
        assert!(!m.molang_eq(&m, alive));
    }

    #[test]
    fn a_float_on_the_right_reads_the_low_32_bits_of_a_hash_on_the_left() {
        // The high 32 bits of the hash do not matter.
        assert!(
            V::Hash(HashedStr::from_u64(0xffff_ffff_3fc0_0000)).molang_eq(&V::Float(1.5), alive)
        );
        assert!(
            V::Hash(HashedStr::from_u64(0x0000_0001_3fc0_0000)).molang_eq(&V::Float(1.5), alive)
        );
        assert!(!V::Hash(HashedStr::from_u64(0x3fc0_0001)).molang_eq(&V::Float(1.5), alive));
        // The empty string reads 0.0 and so equals both zeros.
        assert!(V::string("").molang_eq(&V::Float(0.0), alive));
        assert!(V::string("").molang_eq(&V::Float(-0.0), alive));
        // NaN is unequal to everything, itself included.
        assert!(!V::Hash(HashedStr::from_u64(0x7fc0_0000)).molang_eq(&V::Float(f32::NAN), alive));
    }

    #[test]
    fn a_hash_on_the_right_compares_all_64_bits_with_the_zero_extended_float() {
        assert!(V::Float(0.0).molang_eq(&V::string(""), alive));
        assert!(!V::Float(-0.0).molang_eq(&V::string(""), alive));
        assert!(V::Float(1.5).molang_eq(&V::Hash(HashedStr::from_u64(0x3fc0_0000)), alive));
        assert!(!V::Float(1.5).molang_eq(&V::Hash(HashedStr::from_u64(0x1_3fc0_0000)), alive));
        // Asymmetric: the string equals the float built from its low bits, not the other way round.
        let a = V::string("a");
        assert!(a.molang_eq(&V::Float(a.as_f32()), alive));
        assert!(!V::Float(a.as_f32()).molang_eq(&a, alive));
        // NaN bits are compared as bits on this side, so a NaN equals the hash of its own bits.
        assert!(
            V::Float(f32::from_bits(0x7fc0_0000))
                .molang_eq(&V::Hash(HashedStr::from_u64(0x7fc0_0000)), alive)
        );
    }

    #[test]
    fn two_hashes_compare_all_64_bits() {
        assert!(
            V::Hash(HashedStr::from_u64(0xaaaa_0000_0000_0001))
                .molang_eq(&V::Hash(HashedStr::from_u64(0xaaaa_0000_0000_0001)), alive)
        );
        assert!(
            !V::Hash(HashedStr::from_u64(0xaaaa_0000_0000_0001))
                .molang_eq(&V::Hash(HashedStr::from_u64(0xbbbb_0000_0000_0001)), alive)
        );
        assert!(
            !V::Hash(HashedStr::from_u64(0x0000_0001_0000_0001))
                .molang_eq(&V::Hash(HashedStr::from_u64(0x0000_0002_0000_0001)), alive)
        );
    }

    #[test]
    fn actor_equality_is_resolved_identity() {
        assert!(V::Actor(1).molang_eq(&V::Actor(1), alive));
        assert!(!V::Actor(1).molang_eq(&V::Actor(2), alive));
        assert!(!V::Actor(100).molang_eq(&V::Actor(100), alive));
        // A host that maps an id and a pointer to the same actor makes them equal.
        assert!(V::Actor(1).molang_eq(&V::Actor(1001), |a| Some(a % 1000)));
    }

    #[test]
    fn actor_equality_resolves_both_sides_and_stops_when_one_is_dead() {
        let calls = Cell::new(0);
        let counting = |a: u32| {
            calls.set(calls.get() + 1);
            alive(a)
        };
        assert!(V::Actor(1).molang_eq(&V::Actor(1), counting));
        assert_eq!(calls.get(), 2);
        // Either side unresolvable: unequal even for identical handles.
        assert!(!V::Actor(100).molang_eq(&V::Actor(100), alive));
        assert!(!V::Actor(1).molang_eq(&V::Actor(100), alive));
        assert!(!V::Actor(100).molang_eq(&V::Actor(1), alive));
        // A host mapping two handles to one actor makes them equal; a different actor does not.
        assert!(V::Actor(1).molang_eq(&V::Actor(1001), |a| Some(a % 1000)));
        assert!(!V::Actor(1).molang_eq(&V::Actor(1002), |a| Some(a % 1000)));
    }

    #[test]
    fn actor_against_another_kind_is_unequal_in_both_directions() {
        for other in [
            V::Float(1.0),
            V::string("a"),
            V::Item(1),
            V::actor_array([1]),
            V::identity_matrix(),
        ] {
            assert!(!V::Actor(1).molang_eq(&other, alive), "actor == {other:?}");
            assert!(!other.molang_eq(&V::Actor(1), alive), "{other:?} == actor");
        }
    }

    #[test]
    fn kinds_without_payload_bits_are_unequal_even_to_themselves() {
        let values = [
            V::Item(7),
            V::actor_array([1, 2]),
            V::structure(M::xy(1.0, 2.0)),
            V::identity_matrix(),
            V::Resource(ResourceRef::new("texture.default")),
        ];
        for value in &values {
            assert!(!value.molang_eq(value, alive), "{value:?} == itself");
            assert!(!value.molang_eq(&V::Float(0.0), alive), "{value:?} == 0");
            assert!(!V::Float(0.0).molang_eq(value, alive), "0 == {value:?}");
            assert!(!value.molang_eq(&V::string(""), alive), "{value:?} == ''");
            assert!(!V::string("").molang_eq(value, alive), "'' == {value:?}");
        }
    }

    #[test]
    fn store_cost_is_one_step_per_actor_array_entry() {
        assert_eq!(V::actor_array([1, 2, 3]).store_cost(), 3);
        assert_eq!(V::actor_array([]).store_cost(), 0);
        for value in [
            V::Float(1.0),
            V::string("a"),
            V::Actor(1),
            V::Item(1),
            V::structure(M::xy(1.0, 2.0)),
            V::identity_matrix(),
        ] {
            assert_eq!(value.store_cost(), 0, "{value:?}");
        }
    }

    #[test]
    fn map_actors_converts_actors_and_arrays() {
        let to_id = |a: u32| alive(a).map(|a| a + 1000);
        assert_eq!(V::Actor(1).map_actors(to_id), V::Actor(1001));
        // A single unresolvable actor is kept as it is.
        assert_eq!(V::Actor(100).map_actors(to_id), V::Actor(100));
        assert_eq!(
            V::actor_array([1, 100, 2, 200, 3]).map_actors(to_id),
            V::actor_array([1001, 1002, 1003])
        );
        assert_eq!(V::Float(1.0).map_actors(to_id), V::Float(1.0));
        assert_eq!(V::Item(5).map_actors(to_id), V::Item(5));
    }

    #[test]
    fn map_actors_leaves_other_kinds_and_nested_actors_alone() {
        let plus_one = |a: u32| Some(a + 1);
        assert_eq!(V::string("a").map_actors(plus_one), V::string("a"));
        assert_eq!(
            V::identity_matrix().map_actors(plus_one),
            V::identity_matrix()
        );
        // Only a top-level actor or array is converted, not an actor inside a struct.
        let in_struct = V::structure(M::from([("who", V::Actor(1))]));
        assert_eq!(in_struct.clone().map_actors(plus_one), in_struct);
    }

    #[test]
    fn map_actors_drops_every_entry_that_maps_to_nothing() {
        assert_eq!(
            V::actor_array([1, 2]).map_actors(|_| None),
            V::actor_array([])
        );
        assert_eq!(
            V::actor_array([]).map_actors(|a| Some(a + 1)),
            V::actor_array([])
        );
        // The function runs once per entry, in order.
        let seen = std::cell::RefCell::new(Vec::new());
        let _ = V::actor_array([5, 6, 7]).map_actors(|a| {
            seen.borrow_mut().push(a);
            Some(a)
        });
        assert_eq!(*seen.borrow(), [5, 6, 7]);
    }

    // Fails to compile when an arm is added.
    #[test]
    fn value_arms() {
        fn kind(value: &Value<NoHost>) -> ValueKind {
            match value {
                Value::Float(_) => ValueKind::Float,
                Value::Hash(_) => ValueKind::Hash,
                Value::Actor(()) => ValueKind::Actor,
                Value::Item(()) => ValueKind::Item,
                Value::ActorArray(_) => ValueKind::ActorArray,
                Value::Struct(_) => ValueKind::Struct,
                Value::Matrix(_) => ValueKind::Matrix,
                Value::Resource(_) => ValueKind::Resource,
            }
        }
        assert_eq!(kind(&Value::Float(1.0)), Value::<NoHost>::Float(1.0).kind());
        assert_eq!(kind(&Value::string("a")), ValueKind::Hash);
    }
}
