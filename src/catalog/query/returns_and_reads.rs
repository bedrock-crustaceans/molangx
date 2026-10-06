//! The return-type and read-set bitmasks of a query declaration.

use core::fmt;

/// What a query returns, as a non-empty set of kinds: a query may return one of several.
#[derive(Copy, Clone, PartialEq, Eq, Hash)]
pub struct ReturnType(u8);

impl ReturnType {
    /// A number.
    pub const FLOAT: Self = Self(1 << 0);
    /// A boolean.
    pub const BOOL: Self = Self(1 << 1);
    /// A string.
    pub const STRING: Self = Self(1 << 2);
    /// An actor.
    pub const ACTOR: Self = Self(1 << 3);
    /// An array of actors.
    pub const ACTOR_ARRAY: Self = Self(1 << 4);
    /// A struct.
    pub const STRUCT: Self = Self(1 << 5);
    /// A matrix.
    pub const MATRIX: Self = Self(1 << 6);
    /// A number or a boolean (`FLOAT | BOOL`).
    pub const NUMBER: Self = Self(Self::FLOAT.0 | Self::BOOL.0);

    const NAMES: [&str; 7] = [
        "Float",
        "Bool",
        "String",
        "Actor",
        "ActorArray",
        "Struct",
        "Matrix",
    ];

    /// Every kind.
    pub const fn all() -> Self {
        Self((1 << Self::NAMES.len()) - 1)
    }

    /// Both sets of kinds.
    #[must_use]
    pub const fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }

    /// Adds the kinds of `other`.
    pub const fn insert(&mut self, other: Self) {
        *self = self.union(other);
    }

    /// The raw bits.
    pub const fn bits(self) -> u8 {
        self.0
    }

    /// Whether every kind of `other` is in the set.
    pub const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }

    /// Whether the sets share a kind.
    pub const fn intersects(self, other: Self) -> bool {
        self.0 & other.0 != 0
    }

    /// Whether the set holds only `Float` and `Bool`, the kinds arithmetic accepts.
    pub const fn is_number(self) -> bool {
        Self::NUMBER.contains(self)
    }
}

impl core::ops::BitOr for ReturnType {
    type Output = Self;

    fn bitor(self, rhs: Self) -> Self {
        self.union(rhs)
    }
}

impl core::ops::BitOrAssign for ReturnType {
    fn bitor_assign(&mut self, rhs: Self) {
        *self = self.union(rhs);
    }
}

impl fmt::Debug for ReturnType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        crate::bitmask::fmt_bit_names(f, self.0.into(), &Self::NAMES)
    }
}

bitflags::bitflags! {
    /// The inputs a query implementation reads, as a set; empty for a query that reads none
    /// (`query.approx_eq`, `query.log`, …).
    #[derive(Copy, Clone, Default, PartialEq, Eq, Hash)]
    pub struct Reads: u16 {
        /// The actor, its target and the other actors the context names.
        const ACTOR = 1 << 0;
        /// The item stack.
        const ITEM = 1 << 1;
        /// The block, its position or the blocks around it.
        const BLOCK = 1 << 2;
        /// The world as a whole.
        const LEVEL = 1 << 3;
        /// The world-generation placement target and the blocks world generation sees.
        const WORLD_GEN = 1 << 4;
        /// The distance to the camera.
        const CAMERA = 1 << 5;
        /// Animation and render-context values of the rendered frame; client only.
        const RENDER = 1 << 6;
        /// Client state: frame times, particle and actor counts, input and graphics settings,
        /// camera and bone values.
        const CLIENT_STATE = 1 << 7;
        /// The context's variables (e.g. `context.block_face`).
        const VARIABLES = 1 << 8;
    }
}

impl fmt::Debug for Reads {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        const NAMES: [&str; 9] = [
            "Actor",
            "Item",
            "Block",
            "Level",
            "WorldGen",
            "Camera",
            "Render",
            "ClientState",
            "Variables",
        ];
        crate::bitmask::fmt_bit_names(f, self.bits().into(), &NAMES)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The empty set, which no public constructor builds.
    const NOTHING: ReturnType = ReturnType(0);

    #[test]
    fn return_type_constants_cover_each_bit_once() {
        let kinds = [
            ReturnType::FLOAT,
            ReturnType::BOOL,
            ReturnType::STRING,
            ReturnType::ACTOR,
            ReturnType::ACTOR_ARRAY,
            ReturnType::STRUCT,
            ReturnType::MATRIX,
        ];
        for (bit, ty) in kinds.into_iter().enumerate() {
            assert_eq!(ty.bits(), 1 << bit, "{ty:?}");
        }
        assert_eq!(
            kinds.into_iter().fold(NOTHING, ReturnType::union).bits(),
            (1 << 7) - 1
        );
        assert_eq!(
            ReturnType::NUMBER,
            ReturnType::FLOAT.union(ReturnType::BOOL)
        );
        assert_eq!(ReturnType::NUMBER, ReturnType::FLOAT | ReturnType::BOOL);
    }

    #[test]
    fn return_type_contains_and_intersects() {
        assert!(ReturnType::NUMBER.contains(ReturnType::FLOAT));
        assert!(ReturnType::NUMBER.contains(ReturnType::BOOL));
        assert!(!ReturnType::FLOAT.contains(ReturnType::NUMBER));
        assert!(ReturnType::FLOAT.contains(NOTHING));
        assert!(NOTHING.contains(NOTHING));
        assert!(ReturnType::FLOAT.intersects(ReturnType::NUMBER));
        assert!(ReturnType::NUMBER.intersects(ReturnType::FLOAT));
        assert!(!ReturnType::ACTOR.intersects(ReturnType::ACTOR_ARRAY));
        assert!(!NOTHING.intersects(ReturnType::NUMBER));
        assert!(!ReturnType::NUMBER.intersects(NOTHING));
    }

    #[test]
    fn return_type_is_number_only_for_float_and_bool() {
        assert!(ReturnType::FLOAT.is_number());
        assert!(ReturnType::BOOL.is_number());
        assert!(ReturnType::NUMBER.is_number());
        assert!(!ReturnType::STRING.is_number());
        assert!(!ReturnType::FLOAT.union(ReturnType::ACTOR).is_number());
    }

    #[test]
    fn return_type_debug_lists_the_kinds_in_bit_order() {
        assert_eq!(format!("{:?}", ReturnType::FLOAT), "{Float}");
        assert_eq!(format!("{:?}", ReturnType::ACTOR), "{Actor}");
        assert_eq!(format!("{:?}", ReturnType::NUMBER), "{Float, Bool}");
        assert_eq!(format!("{NOTHING:?}"), "{}");
        assert_eq!(
            format!("{:?}", ReturnType::MATRIX.union(ReturnType::STRING)),
            "{String, Matrix}"
        );
    }

    #[test]
    fn reads_constants_are_consecutive_bits() {
        let singles = [
            Reads::ACTOR,
            Reads::ITEM,
            Reads::BLOCK,
            Reads::LEVEL,
            Reads::WORLD_GEN,
            Reads::CAMERA,
            Reads::RENDER,
            Reads::CLIENT_STATE,
            Reads::VARIABLES,
        ];
        for (bit, reads) in singles.into_iter().enumerate() {
            assert_eq!(reads.bits(), 1 << bit);
            assert!(!reads.is_empty());
        }
        assert_eq!(Reads::empty().bits(), 0);
        assert_eq!(Reads::default(), Reads::empty());
    }

    #[test]
    fn reads_union_and_contains() {
        let both = Reads::ACTOR.union(Reads::ITEM);
        assert!(both.contains(Reads::ITEM));
        assert!(both.contains(Reads::ACTOR));
        assert!(both.contains(both));
        assert!(!Reads::ACTOR.contains(Reads::ITEM));
        assert!(!Reads::ACTOR.contains(both));
        assert!(Reads::ACTOR.contains(Reads::empty()));
        assert!(Reads::empty().contains(Reads::empty()));
        assert_eq!(both, Reads::ITEM.union(Reads::ACTOR));
        assert_eq!(both.union(both), both);
        assert_eq!(both.union(Reads::empty()), both);
        assert_eq!(Reads::ACTOR | Reads::ITEM, both);
        assert!(Reads::empty().is_empty());
        assert!(!both.is_empty());
    }

    #[test]
    fn return_type_all_and_insert() {
        let each = [
            ReturnType::FLOAT,
            ReturnType::BOOL,
            ReturnType::STRING,
            ReturnType::ACTOR,
            ReturnType::ACTOR_ARRAY,
            ReturnType::STRUCT,
            ReturnType::MATRIX,
        ];
        assert_eq!(
            each.into_iter().reduce(ReturnType::union),
            Some(ReturnType::all())
        );
        let mut ty = ReturnType::FLOAT;
        ty.insert(ReturnType::BOOL);
        assert_eq!(ty, ReturnType::NUMBER);
        ty.insert(ReturnType::BOOL);
        assert_eq!(ty, ReturnType::NUMBER);
    }

    #[test]
    fn reads_debug_shows_undefined_bits_in_hex() {
        assert_eq!(
            format!("{:?}", Reads::from_bits_retain(1 << 12)),
            "{0x1000}"
        );
        assert_eq!(
            format!("{:?}", Reads::ACTOR | Reads::from_bits_retain(0xf000)),
            "{Actor, 0xf000}"
        );
    }

    #[test]
    fn reads_debug_lists_the_names_in_bit_order() {
        assert_eq!(format!("{:?}", Reads::empty()), "{}");
        assert_eq!(
            format!("{:?}", Reads::BLOCK.union(Reads::ACTOR)),
            "{Actor, Block}"
        );
        let all = [
            Reads::ACTOR,
            Reads::ITEM,
            Reads::BLOCK,
            Reads::LEVEL,
            Reads::WORLD_GEN,
            Reads::CAMERA,
            Reads::RENDER,
            Reads::CLIENT_STATE,
            Reads::VARIABLES,
        ]
        .into_iter()
        .fold(Reads::empty(), Reads::union);
        assert_eq!(
            format!("{all:?}"),
            "{Actor, Item, Block, Level, WorldGen, Camera, Render, ClientState, Variables}"
        );
    }
}
