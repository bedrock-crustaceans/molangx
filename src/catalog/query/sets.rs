//! [`QuerySetMask`]: the query sets a compilation names; a query resolves only in one of them.

use core::fmt;
use core::str::FromStr;

use thiserror::Error;

use crate::bitmask::set_operators;

/// A set of query sets: the three built-in sets and up to 13 host sets. The empty mask admits no
/// query; the default is [`QuerySetMask::DEFAULT`].
#[derive(Copy, Clone, PartialEq, Eq, Hash)]
pub struct QuerySetMask(u16);

impl QuerySetMask {
    /// No set: resolves no query.
    pub const fn empty() -> Self {
        Self(0)
    }

    /// The built-in set `default`: every standard query except the `tags` and `world_gen` members.
    pub const DEFAULT: Self = Self(1 << 0);
    /// The built-in set `tags`: `query.any_tag` and `query.all_tags`, for block and item descriptor
    /// `tags` expressions.
    pub const TAGS: Self = Self(1 << 1);
    /// The built-in set `world_gen`: the world-generation queries (scatter, feature and biome
    /// fields).
    pub const WORLD_GEN: Self = Self(1 << 2);
    /// The three built-in sets, without any host set.
    pub const BUILTIN: Self = Self(0b111);
    /// The number of sets a host can define.
    pub const HOST_SETS: u8 = 13;

    /// The host-defined set number `n` (`0..13`), for queries a host declares; `None` for a larger
    /// number. Host sets have no name and never hold a standard query.
    pub const fn host(n: u8) -> Option<Self> {
        if n < Self::HOST_SETS {
            Some(Self(1 << (3 + n)))
        } else {
            None
        }
    }

    /// The name of a single built-in set; `None` for an empty, multi-set or host mask.
    pub const fn name(self) -> Option<&'static str> {
        match self.0 {
            0b001 => Some("default"),
            0b010 => Some("tags"),
            0b100 => Some("world_gen"),
            _ => None,
        }
    }

    /// The raw bits: bits 0 to 2 the built-in sets, bit `3 + n` host set `n`.
    pub const fn bits(self) -> u16 {
        self.0
    }

    /// The three built-in sets and every host set.
    pub const fn all() -> Self {
        Self(u16::MAX)
    }

    /// Both masks' sets.
    #[must_use]
    pub const fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }

    /// The sets in both masks.
    #[must_use]
    pub const fn intersection(self, other: Self) -> Self {
        Self(self.0 & other.0)
    }

    /// The sets of `self` not in `other`.
    #[must_use]
    pub const fn difference(self, other: Self) -> Self {
        Self(self.0 & !other.0)
    }

    /// Adds the sets of `other`.
    pub const fn insert(&mut self, other: Self) {
        *self = self.union(other);
    }

    /// Removes the sets of `other`.
    pub const fn remove(&mut self, other: Self) {
        *self = self.difference(other);
    }

    /// Whether every set of `other` is in `self`.
    pub fn contains(self, other: impl Into<Self>) -> bool {
        let other = other.into();
        self.0 & other.0 == other.0
    }

    /// Whether the masks share a set.
    pub fn intersects(self, other: impl Into<Self>) -> bool {
        self.0 & other.into().0 != 0
    }

    /// Whether the mask holds no set.
    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }
}

impl Default for QuerySetMask {
    /// [`QuerySetMask::DEFAULT`].
    fn default() -> Self {
        Self::DEFAULT
    }
}

set_operators!(QuerySetMask);

/// Text that is not the name of a built-in query set ([`QuerySetMask`]'s `FromStr`).
#[derive(Error, Copy, Clone, Debug, PartialEq, Eq)]
#[error("not the name of a built-in query set")]
pub struct ParseQuerySetError;

impl FromStr for QuerySetMask {
    type Err = ParseQuerySetError;

    /// The built-in set named `name` (`"default"`, `"tags"`, `"world_gen"`), exactly.
    fn from_str(name: &str) -> Result<Self, ParseQuerySetError> {
        match name {
            "default" => Ok(Self::DEFAULT),
            "tags" => Ok(Self::TAGS),
            "world_gen" => Ok(Self::WORLD_GEN),
            _ => Err(ParseQuerySetError),
        }
    }
}

impl fmt::Debug for QuerySetMask {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut set = f.debug_set();
        for single in [Self::DEFAULT, Self::TAGS, Self::WORLD_GEN] {
            if self.contains(single) {
                set.entry(&format_args!("{}", single.name().unwrap_or_default()));
            }
        }
        for n in 0..Self::HOST_SETS {
            if self.0 & (1 << (3 + n)) != 0 {
                set.entry(&format_args!("host({n})"));
            }
        }
        set.finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn query_set_mask() {
        assert_eq!("default".parse::<QuerySetMask>(), Ok(QuerySetMask::DEFAULT));
        assert_eq!("tags".parse::<QuerySetMask>(), Ok(QuerySetMask::TAGS));
        assert_eq!(
            "world_gen".parse::<QuerySetMask>(),
            Ok(QuerySetMask::WORLD_GEN)
        );
        assert_eq!("worldgen".parse::<QuerySetMask>(), Err(ParseQuerySetError));
        assert_eq!("test".parse::<QuerySetMask>(), Err(ParseQuerySetError));
        assert!(QuerySetMask::empty().is_empty());
        let both = QuerySetMask::DEFAULT | QuerySetMask::TAGS;
        assert!(both.contains(QuerySetMask::TAGS) && !both.contains(QuerySetMask::WORLD_GEN));
        assert_eq!(both.name(), None);
        assert_eq!(QuerySetMask::WORLD_GEN.name(), Some("world_gen"));
        assert_eq!(format!("{both:?}"), "{default, tags}");
    }

    #[test]
    fn single_set_bits() {
        assert_eq!(QuerySetMask::empty().bits(), 0);
        assert_eq!(QuerySetMask::DEFAULT.bits(), 1);
        assert_eq!(QuerySetMask::TAGS.bits(), 2);
        assert_eq!(QuerySetMask::WORLD_GEN.bits(), 4);
        assert_eq!(QuerySetMask::host(0).map(QuerySetMask::bits), Some(8));
        assert_eq!(QuerySetMask::BUILTIN.bits(), 7);
        assert_eq!(QuerySetMask::default(), QuerySetMask::DEFAULT);
    }

    #[test]
    fn parse_and_name_round_trip_for_the_built_in_sets() {
        for (mask, name) in [
            (QuerySetMask::DEFAULT, "default"),
            (QuerySetMask::TAGS, "tags"),
            (QuerySetMask::WORLD_GEN, "world_gen"),
        ] {
            assert_eq!(mask.name(), Some(name));
            assert_eq!(name.parse::<QuerySetMask>(), Ok(mask));
        }
    }

    #[test]
    fn name_is_none_for_empty_and_multi_set_masks() {
        assert_eq!(QuerySetMask::empty().name(), None);
        assert_eq!((QuerySetMask::DEFAULT | QuerySetMask::TAGS).name(), None);
        assert_eq!(QuerySetMask::BUILTIN.name(), None);
    }

    #[test]
    fn parsing_is_exact() {
        for bad in [
            "",
            "Default",
            "DEFAULT",
            "default ",
            " default",
            "world-gen",
            "worldgen",
            "tag",
            "all",
        ] {
            assert_eq!(
                bad.parse::<QuerySetMask>(),
                Err(ParseQuerySetError),
                "{bad:?}"
            );
        }
        assert_eq!(
            ParseQuerySetError.to_string(),
            "not the name of a built-in query set"
        );
    }

    #[test]
    fn host_sets_are_the_bits_above_the_built_in_sets() {
        for n in 0..QuerySetMask::HOST_SETS {
            let set = QuerySetMask::host(n).unwrap();
            assert_eq!(set.bits(), 1 << (3 + n));
            assert!(!set.intersects(QuerySetMask::BUILTIN));
            assert_eq!(set.name(), None, "host sets have no name");
        }
        assert_eq!(QuerySetMask::host(13), None);
        assert_eq!(QuerySetMask::host(u8::MAX), None);
        assert_eq!(
            QuerySetMask::host(12).map(QuerySetMask::bits),
            Some(1 << 15)
        );
    }

    #[test]
    fn union_is_commutative_associative_and_idempotent_over_all_masks() {
        let masks: Vec<QuerySetMask> = (0..32).map(QuerySetMask).collect();
        for &a in &masks {
            assert_eq!(a.union(a), a);
            assert_eq!(a.union(QuerySetMask::empty()), a);
            for &b in &masks {
                assert_eq!(a.union(b), b.union(a));
                assert_eq!(a.union(b).bits(), a.bits() | b.bits());
                for &c in &masks {
                    assert_eq!(a.union(b).union(c), a.union(b.union(c)));
                }
            }
        }
    }

    #[test]
    fn contains_and_intersects_follow_the_bits() {
        for a in 0..32u16 {
            for b in 0..32u16 {
                let (ma, mb) = (QuerySetMask(a), QuerySetMask(b));
                assert_eq!(ma.contains(mb), a & b == b, "{a} {b}");
                assert_eq!(ma.intersects(mb), a & b != 0, "{a} {b}");
                assert_eq!(ma.intersects(mb), mb.intersects(ma));
            }
        }
        assert!(QuerySetMask::empty().contains(QuerySetMask::empty()));
        assert!(!QuerySetMask::empty().contains(QuerySetMask::DEFAULT));
        assert!(!QuerySetMask::BUILTIN.intersects(QuerySetMask::empty()));
        assert!((QuerySetMask::DEFAULT | QuerySetMask::TAGS).intersects(QuerySetMask::TAGS));
        assert!(!(QuerySetMask::DEFAULT | QuerySetMask::TAGS).intersects(QuerySetMask::WORLD_GEN));
    }

    #[test]
    fn bit_or_operators_agree_with_union() {
        assert_eq!(
            QuerySetMask::DEFAULT | QuerySetMask::TAGS,
            QuerySetMask::DEFAULT.union(QuerySetMask::TAGS)
        );
        let mut mask = QuerySetMask::DEFAULT;
        mask |= QuerySetMask::TAGS;
        assert_eq!(mask, QuerySetMask::DEFAULT | QuerySetMask::TAGS);
        mask |= QuerySetMask::TAGS;
        assert_eq!(mask.bits(), 3);
        assert_eq!(
            QuerySetMask::DEFAULT | QuerySetMask::TAGS | QuerySetMask::WORLD_GEN,
            QuerySetMask::BUILTIN
        );
    }

    #[test]
    fn intersection_difference_insert_and_remove_follow_the_bits() {
        let masks: Vec<QuerySetMask> = (0..32).map(QuerySetMask).collect();
        for &a in &masks {
            for &b in &masks {
                assert_eq!((a & b).bits(), a.bits() & b.bits());
                assert_eq!((a - b).bits(), a.bits() & !b.bits());
                let (mut and, mut sub, mut inserted, mut removed) = (a, a, a, a);
                and &= b;
                sub -= b;
                inserted.insert(b);
                removed.remove(b);
                assert_eq!(
                    (and, sub, inserted, removed),
                    (a.intersection(b), a.difference(b), a | b, a - b)
                );
            }
        }
        assert_eq!(QuerySetMask::all().bits(), u16::MAX);
        let every_host = (0..QuerySetMask::HOST_SETS)
            .filter_map(QuerySetMask::host)
            .fold(QuerySetMask::empty(), QuerySetMask::union);
        assert_eq!(QuerySetMask::all() - QuerySetMask::BUILTIN, every_host);
    }

    #[test]
    fn is_empty_only_for_the_empty_mask() {
        assert!(QuerySetMask::empty().is_empty());
        for mask in [
            QuerySetMask::DEFAULT,
            QuerySetMask::TAGS,
            QuerySetMask::WORLD_GEN,
            QuerySetMask::host(0).unwrap(),
            QuerySetMask::BUILTIN,
        ] {
            assert!(!mask.is_empty());
        }
    }

    #[test]
    fn query_set_debug_lists_the_names() {
        assert_eq!(format!("{:?}", QuerySetMask::empty()), "{}");
        assert_eq!(format!("{:?}", QuerySetMask::DEFAULT), "{default}");
        assert_eq!(
            format!("{:?}", QuerySetMask::DEFAULT | QuerySetMask::WORLD_GEN),
            "{default, world_gen}"
        );
        assert_eq!(
            format!("{:?}", QuerySetMask::BUILTIN),
            "{default, tags, world_gen}"
        );
        let host = QuerySetMask::host(0).unwrap() | QuerySetMask::host(12).unwrap();
        assert_eq!(format!("{host:?}"), "{host(0), host(12)}");
        assert_eq!(
            format!(
                "{:?}",
                QuerySetMask::BUILTIN | QuerySetMask::host(1).unwrap()
            ),
            "{default, tags, world_gen, host(1)}"
        );
    }
}
