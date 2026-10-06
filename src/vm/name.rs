//! [`Name`]: the key of a `variable.` / `temp.` / `context.` name, typed by its namespace.

use std::cmp::Ordering;
use std::fmt;
use std::hash::{Hash, Hasher};
use std::marker::PhantomData;

use crate::hash::{FNV1_OFFSET_BASIS, HashedStr, fnv1_step};

/// The three namespaces of Molang names, as uninhabited marker types for [`Name`].
pub mod namespace {
    use super::sealed::Sealed;

    /// A namespace of Molang names: [`Variable`], [`Temp`] or [`Context`]. Sealed.
    pub trait Namespace: Sealed + 'static {}

    /// `variable.` (`v.`): an entity variable.
    #[derive(Debug)]
    pub enum Variable {}

    /// `temp.` (`t.`): a temporary.
    #[derive(Debug)]
    pub enum Temp {}

    /// `context.` (`c.`): a context variable.
    #[derive(Debug)]
    pub enum Context {}

    impl Namespace for Variable {}
    impl Namespace for Temp {}
    impl Namespace for Context {}

    impl Sealed for Variable {
        const LONG: &'static str = "variable.";
        const SHORT: &'static str = "v.";
        const KEY_NAME: &'static str = "VariableName";
    }

    impl Sealed for Temp {
        const LONG: &'static str = "temp.";
        const SHORT: &'static str = "t.";
        const KEY_NAME: &'static str = "TempName";
    }

    impl Sealed for Context {
        const LONG: &'static str = "context.";
        const SHORT: &'static str = "c.";
        const KEY_NAME: &'static str = "ContextName";
    }
}

mod sealed {
    pub trait Sealed {
        /// The long spelling with its dot, lower case (`"variable."`); the canonical prefix.
        const LONG: &'static str;
        /// The short spelling with its dot, lower case (`"v."`).
        const SHORT: &'static str;
        /// The name of the key type, for `Debug` (`"VariableName"`).
        const KEY_NAME: &'static str;
    }
}

use namespace::{Context, Namespace, Temp, Variable};

/// The key of a name in namespace `N`: the [`HashedStr`] of its full canonical name with the
/// long namespace (`variable.x`), so separately compiled programs agree on names.
///
/// Every constructor from text lowers ASCII `A`–`Z` and ends the text at its first NUL byte, as
/// [`StructValue::key`](super::StructValue::key) does.
///
/// ```
/// use molangx::hash::HashedStr;
/// use molangx::vm::{AnyName, ContextName, TempName, VariableName};
///
/// const MOO: VariableName = VariableName::new("moo");
/// assert_eq!(VariableName::new("Moo"), MOO);
/// assert_eq!(VariableName::parse("v.Moo"), Some(MOO));
/// assert_eq!(VariableName::parse("Variable.MOO"), Some(MOO));
/// assert_eq!(TempName::parse("v.moo"), None);
/// assert_eq!(AnyName::parse("c.other"), Some(AnyName::Context(ContextName::new("other"))));
/// // Text after a NUL byte is not part of the name, in every constructor.
/// assert_eq!(VariableName::new("moo\0x"), MOO);
/// // The key is the `HashedStr` of the canonical name.
/// assert_eq!(MOO.hashed(), HashedStr::new("variable.moo"));
/// ```
///
/// A key of another namespace is another type, so this does not compile:
///
/// ```compile_fail,E0308
/// use molangx::vm::{ContextName, NoHostEnv, Value};
///
/// let mut env = NoHostEnv::new();
/// env.variables.set(ContextName::new("x"), Value::Float(1.0));
/// ```
///
/// ```compile_fail,E0308
/// use molangx::vm::{NoHostEnv, Value, VariableName};
///
/// let mut env = NoHostEnv::new();
/// env.context.set(VariableName::new("z"), Value::Float(1.0));
/// ```
pub struct Name<N: Namespace> {
    hash: HashedStr,
    marker: PhantomData<fn() -> N>,
}

/// The key of an entity variable, `variable.<name>`.
pub type VariableName = Name<Variable>;

/// The key of a temporary, `temp.<name>`.
pub type TempName = Name<Temp>;

/// The key of a context variable, `context.<name>`.
pub type ContextName = Name<Context>;

/// FNV-1 over `prefix` then `name`, ASCII-lowered, as one string ending at its first NUL byte.
pub(super) const fn hash_lowered(prefix: &str, name: &[u8]) -> HashedStr {
    let prefix = prefix.as_bytes();
    let mut h = FNV1_OFFSET_BASIS;
    let mut any = false;
    let mut i = 0;
    while i < prefix.len() + name.len() {
        let byte = if i < prefix.len() {
            prefix[i]
        } else {
            name[i - prefix.len()]
        };
        if byte == 0 {
            break;
        }
        h = fnv1_step(h, byte.to_ascii_lowercase());
        any = true;
        i += 1;
    }
    HashedStr::from_u64(if any { h } else { 0 })
}

/// `text` up to its first NUL byte, split after its first dot into the namespace with its dot and
/// the name; `None` without a dot or with nothing after it.
const fn split_spelled(text: &str) -> Option<(&[u8], &[u8])> {
    let bytes = text.as_bytes();
    let mut end = 0;
    while end < bytes.len() && bytes[end] != 0 {
        end += 1;
    }
    let (bytes, _) = bytes.split_at(end);
    let mut dot = 0;
    while dot < bytes.len() && bytes[dot] != b'.' {
        dot += 1;
    }
    if dot + 1 >= bytes.len() {
        return None;
    }
    Some(bytes.split_at(dot + 1))
}

impl<N: Namespace> Name<N> {
    /// The key of `name`, given without its namespace.
    pub const fn new(name: &str) -> Self {
        Self::from_raw_hash(hash_lowered(N::LONG, name.as_bytes()))
    }

    /// The key of a name spelled with either spelling of this namespace, in any case (`v.x`,
    /// `Variable.X`); `None` for another namespace or no name after it.
    ///
    /// Only the namespace is checked, not the name's characters. The text ends at its first NUL,
    /// so `"v.\0x"` has no name.
    pub const fn parse(text: &str) -> Option<Self> {
        let Some((namespace, name)) = split_spelled(text) else {
            return None;
        };
        if namespace.eq_ignore_ascii_case(N::LONG.as_bytes())
            || namespace.eq_ignore_ascii_case(N::SHORT.as_bytes())
        {
            Some(Self::from_raw_hash(hash_lowered(N::LONG, name)))
        } else {
            None
        }
    }

    /// The key with this hash of the canonical name, **unchecked**: a wrong hash is a key no
    /// expression reaches. Prefer [`new`](Self::new) when the text is at hand.
    pub const fn from_raw_hash(hash: HashedStr) -> Self {
        Self {
            hash,
            marker: PhantomData,
        }
    }

    /// The key as the [`HashedStr`] of the canonical name.
    pub const fn hashed(self) -> HashedStr {
        self.hash
    }
}

impl<N: Namespace> From<Name<N>> for HashedStr {
    fn from(name: Name<N>) -> Self {
        name.hashed()
    }
}

impl<N: Namespace> core::str::FromStr for Name<N> {
    type Err = ParseNameError;

    /// [`Name::parse`].
    fn from_str(text: &str) -> Result<Self, ParseNameError> {
        Self::parse(text).ok_or(ParseNameError)
    }
}

/// Text that is not a name of the namespace(s) asked for ([`Name::parse`], [`AnyName::parse`]).
#[derive(thiserror::Error, Copy, Clone, Debug, PartialEq, Eq)]
#[error("not a name of the expected namespace")]
pub struct ParseNameError;

// Manual impls: derives would bound the marker type `N`.
impl<N: Namespace> Clone for Name<N> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<N: Namespace> Copy for Name<N> {}

impl<N: Namespace> PartialEq for Name<N> {
    fn eq(&self, other: &Self) -> bool {
        self.hash == other.hash
    }
}

impl<N: Namespace> Eq for Name<N> {}

impl<N: Namespace> PartialOrd for Name<N> {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl<N: Namespace> Ord for Name<N> {
    fn cmp(&self, other: &Self) -> Ordering {
        self.hash.cmp(&other.hash)
    }
}

/// The key hashes as its `u64` alone, so maps use it as is ([`nohash_hasher::IsEnabled`]).
impl<N: Namespace> Hash for Name<N> {
    fn hash<S: Hasher>(&self, state: &mut S) {
        self.hash.hash(state);
    }
}

impl<N: Namespace> nohash_hasher::IsEnabled for Name<N> {}

impl<N: Namespace> fmt::Debug for Name<N> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}({:#018x})", N::KEY_NAME, self.hash.as_u64())
    }
}

/// A name of any of the three namespaces, as [`AnyName::parse`] reads it from text.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum AnyName {
    /// `variable.x` / `v.x`.
    Variable(VariableName),
    /// `temp.x` / `t.x`.
    Temp(TempName),
    /// `context.x` / `c.x`.
    Context(ContextName),
}

impl AnyName {
    /// The key of a name spelled with any namespace, following [`Name::parse`]'s rules.
    pub const fn parse(text: &str) -> Option<Self> {
        if let Some(name) = VariableName::parse(text) {
            Some(Self::Variable(name))
        } else if let Some(name) = TempName::parse(text) {
            Some(Self::Temp(name))
        } else if let Some(name) = ContextName::parse(text) {
            Some(Self::Context(name))
        } else {
            None
        }
    }

    /// The hash of the canonical name.
    pub const fn hashed(self) -> HashedStr {
        match self {
            Self::Variable(name) => name.hashed(),
            Self::Temp(name) => name.hashed(),
            Self::Context(name) => name.hashed(),
        }
    }
}

impl From<AnyName> for HashedStr {
    fn from(name: AnyName) -> Self {
        name.hashed()
    }
}

impl core::str::FromStr for AnyName {
    type Err = ParseNameError;

    /// [`AnyName::parse`].
    fn from_str(text: &str) -> Result<Self, ParseNameError> {
        Self::parse(text).ok_or(ParseNameError)
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::hash::BuildHasher;

    use nohash_hasher::BuildNoHashHasher;

    use super::*;

    #[test]
    fn a_key_is_the_hash_of_the_canonical_name() {
        assert_eq!(
            VariableName::new("moo").hashed(),
            HashedStr::new("variable.moo")
        );
        assert_eq!(TempName::new("i").hashed(), HashedStr::new("temp.i"));
        assert_eq!(
            ContextName::new("other").hashed(),
            HashedStr::new("context.other")
        );
        assert_eq!(
            VariableName::new("moo").hashed(),
            HashedStr::new("variable.moo")
        );
        assert_eq!(
            TempName::from_raw_hash(HashedStr::new("temp.i")),
            TempName::new("i")
        );
        assert_ne!(VariableName::new("x").hashed(), TempName::new("x").hashed());
        assert_ne!(
            VariableName::new("x").hashed(),
            ContextName::new("x").hashed()
        );
        assert_ne!(TempName::new("x").hashed(), ContextName::new("x").hashed());
    }

    #[test]
    fn parse_accepts_both_spellings_in_any_case() {
        let baa = VariableName::new("baa");
        for text in [
            "v.baa",
            "variable.baa",
            "V.Baa",
            "Variable.BAA",
            "VARIABLE.baa",
        ] {
            assert_eq!(VariableName::parse(text), Some(baa), "{text:?}");
            assert_eq!(
                AnyName::parse(text),
                Some(AnyName::Variable(baa)),
                "{text:?}"
            );
        }
        for text in ["t.x", "temp.x", "T.X", "Temp.X"] {
            assert_eq!(TempName::parse(text), Some(TempName::new("x")), "{text:?}");
            assert_eq!(
                AnyName::parse(text),
                Some(AnyName::Temp(TempName::new("x"))),
                "{text:?}"
            );
        }
        for text in ["c.moo", "context.moo", "C.MOO", "CONTEXT.moo"] {
            assert_eq!(
                ContextName::parse(text),
                Some(ContextName::new("moo")),
                "{text:?}"
            );
            assert_eq!(
                AnyName::parse(text),
                Some(AnyName::Context(ContextName::new("moo"))),
                "{text:?}"
            );
        }
        assert_eq!(
            VariableName::parse("v.a.b").map(VariableName::hashed),
            Some(HashedStr::new("variable.a.b"))
        );
        assert_eq!(
            TempName::parse("t.a.b.c").map(TempName::hashed),
            Some(HashedStr::new("temp.a.b.c"))
        );
    }

    #[test]
    fn parse_per_namespace_rejects_the_other_namespaces() {
        assert_eq!(VariableName::parse("t.x"), None);
        assert_eq!(VariableName::parse("context.x"), None);
        assert_eq!(TempName::parse("v.x"), None);
        assert_eq!(TempName::parse("c.x"), None);
        assert_eq!(ContextName::parse("variable.x"), None);
        assert_eq!(ContextName::parse("temp.x"), None);
    }

    #[test]
    fn parse_rejects_other_namespaces_and_missing_names() {
        for bad in [
            "",
            "x",
            "v",
            "variable",
            ".",
            ".x",
            "v.",
            "variable.",
            "temp.",
            "context.",
            "vv.x",
            "var.x",
            "variables.x",
            "q.x",
            "query.x",
            "math.pi",
            "geometry.x",
            "é.x",
            " v.x",
        ] {
            assert_eq!(AnyName::parse(bad), None, "{bad:?}");
            assert_eq!(VariableName::parse(bad), None, "{bad:?}");
            assert_eq!(TempName::parse(bad), None, "{bad:?}");
            assert_eq!(ContextName::parse(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn every_constructor_lowers_ascii_and_stops_at_a_nul() {
        assert_eq!(VariableName::new("Moo"), VariableName::new("moo"));
        assert_eq!(TempName::new("MOO"), TempName::new("moo"));
        assert_eq!(ContextName::new("MoO"), ContextName::new("moo"));
        assert_eq!(
            VariableName::new("x\0y").hashed(),
            HashedStr::new("variable.x")
        );
        assert_eq!(VariableName::new("x\0y"), VariableName::new("x\0z"));
        assert_eq!(TempName::new("x\0"), TempName::new("x"));
        assert_eq!(ContextName::new("X\0Y"), ContextName::new("x"));
        assert_eq!(VariableName::parse("v.x\0y"), Some(VariableName::new("x")));
    }

    #[test]
    fn only_ascii_letters_are_lowered() {
        assert_ne!(VariableName::new("É"), VariableName::new("é"));
        assert_eq!(
            VariableName::new("É").hashed(),
            HashedStr::new("variable.É")
        );
        assert_eq!(
            VariableName::new("ÉA").hashed(),
            HashedStr::new("variable.Éa")
        );
    }

    #[test]
    fn empty_names() {
        assert_eq!(VariableName::new("").hashed(), HashedStr::new("variable."));
        assert_eq!(TempName::new("").hashed(), HashedStr::new("temp."));
        assert_eq!(ContextName::new("").hashed(), HashedStr::new("context."));
        assert_ne!(
            VariableName::new(""),
            VariableName::from_raw_hash(HashedStr::from_u64(0))
        );
        assert_eq!(VariableName::new("\0abc"), VariableName::new(""));
        assert_eq!(hash_lowered("", b""), HashedStr::EMPTY);
        assert_eq!(hash_lowered("", b"\0abc"), HashedStr::EMPTY);
        assert_eq!(hash_lowered("", b"A"), HashedStr::new("a"));
    }

    #[test]
    fn hash_lowered_hashes_the_prefix_and_name_as_one_string() {
        assert_eq!(
            hash_lowered("variable.", b"Moo"),
            HashedStr::new("variable.moo")
        );
        assert_eq!(
            hash_lowered("va", b"riable.moo"),
            hash_lowered("variable.", b"moo")
        );
        assert_eq!(hash_lowered("ab\0cd", b"ef"), HashedStr::new("ab"));
    }

    #[test]
    fn parse_looks_for_the_first_dot_only() {
        assert_eq!(
            VariableName::parse("v.t.x").map(VariableName::hashed),
            Some(HashedStr::new("variable.t.x"))
        );
        assert_eq!(
            TempName::parse("t.v.x").map(TempName::hashed),
            Some(HashedStr::new("temp.v.x"))
        );
        assert_eq!(
            VariableName::parse("v..").map(VariableName::hashed),
            Some(HashedStr::new("variable.."))
        );
        assert_eq!(
            AnyName::parse("t.v.x"),
            Some(AnyName::Temp(TempName::new("v.x")))
        );
    }

    #[test]
    fn parse_rejects_a_name_that_is_empty_up_to_a_nul_as_it_rejects_no_name() {
        for text in ["v.\0x", "v.\0", "variable.\0moo", "t.\0", "context.\0x"] {
            assert_eq!(AnyName::parse(text), None, "{text:?}");
        }
        assert_eq!(AnyName::parse("v\0.x"), None);
        assert_eq!(AnyName::parse("\0v.x"), None);
    }

    #[test]
    fn parse_keeps_the_name_up_to_a_nul() {
        assert_eq!(VariableName::parse("v.x\0y"), Some(VariableName::new("x")));
        assert_eq!(TempName::parse("T.Moo\0"), Some(TempName::new("moo")));
        assert_eq!(
            ContextName::parse("c.a.b\0.c"),
            Some(ContextName::new("a.b"))
        );
    }

    #[test]
    fn conversions_round_trip() {
        let key = VariableName::new("x");
        assert_eq!(key.hashed(), HashedStr::new("variable.x"));
        assert_eq!(key.hashed(), HashedStr::new("variable.x"));
        assert_eq!(VariableName::from_raw_hash(key.hashed()), key);
        assert_eq!(
            VariableName::from_raw_hash(HashedStr::new("variable.x")),
            key
        );
        assert_eq!(
            ContextName::from_raw_hash(HashedStr::from_u64(7))
                .hashed()
                .as_u64(),
            7
        );
        assert_eq!(AnyName::Variable(key).hashed(), key.hashed());
        assert_eq!(
            AnyName::Temp(TempName::new("x")).hashed(),
            HashedStr::new("temp.x")
        );
        assert_eq!(
            AnyName::Context(ContextName::new("x")).hashed(),
            HashedStr::new("context.x")
        );
    }

    #[test]
    fn keys_order_by_their_hash() {
        assert!(
            TempName::from_raw_hash(HashedStr::from_u64(1))
                < TempName::from_raw_hash(HashedStr::from_u64(2))
        );
        assert!(
            TempName::from_raw_hash(HashedStr::from_u64(u64::MAX))
                > TempName::from_raw_hash(HashedStr::from_u64(0))
        );
        assert_eq!(
            TempName::from_raw_hash(HashedStr::from_u64(5))
                .cmp(&TempName::from_raw_hash(HashedStr::from_u64(5))),
            Ordering::Equal
        );
    }

    #[test]
    fn debug_names_the_namespace_and_prints_the_hash() {
        assert_eq!(
            format!(
                "{:?}",
                VariableName::from_raw_hash(HashedStr::from_u64(0x2a))
            ),
            "VariableName(0x000000000000002a)"
        );
        assert_eq!(
            format!("{:?}", TempName::from_raw_hash(HashedStr::from_u64(1))),
            "TempName(0x0000000000000001)"
        );
        assert_eq!(
            format!(
                "{:?}",
                ContextName::from_raw_hash(HashedStr::from_u64(u64::MAX))
            ),
            "ContextName(0xffffffffffffffff)"
        );
    }

    #[test]
    fn a_key_hashes_to_itself() {
        let build = BuildNoHashHasher::<VariableName>::default();
        assert_eq!(
            build.hash_one(VariableName::from_raw_hash(HashedStr::from_u64(42))),
            42
        );
        assert_eq!(
            build.hash_one(VariableName::new("x")),
            VariableName::new("x").hashed().as_u64()
        );
    }

    #[test]
    fn a_map_keyed_by_name_keys_finds_every_entry() {
        let mut map: HashMap<VariableName, usize, BuildNoHashHasher<VariableName>> =
            HashMap::default();
        for i in 0..1000 {
            map.insert(VariableName::new(&format!("v{i}")), i);
        }
        assert_eq!(map.len(), 1000);
        for i in 0..1000 {
            assert_eq!(map.get(&VariableName::new(&format!("v{i}"))), Some(&i));
        }
        assert_eq!(map.get(&VariableName::new("missing")), None);
    }
}
