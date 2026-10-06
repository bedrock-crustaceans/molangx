//! [`Arity`]: the argument counts a function or query is declared with.

use core::fmt;
use core::ops::{RangeFrom, RangeInclusive};

use thiserror::Error;

/// A non-empty range of argument counts: `min..=max`, or `min..` when `max` is `None`.
///
/// ```
/// use molangx::catalog::Arity;
///
/// const PAIR_OR_TRIPLE: Arity = Arity::between(2, 3);
/// assert_eq!(Arity::try_from(2..=3), Ok(PAIR_OR_TRIPLE));
/// assert_eq!(Arity::from(2), Arity::exactly(2));
/// assert_eq!(Arity::from(2..), Arity::at_least(2));
/// assert!(Arity::at_least(2).contains(9) && !Arity::exactly(1).contains(2));
/// assert_eq!(Arity::checked_between(3, 1), None);
/// assert_eq!(PAIR_OR_TRIPLE.to_string(), "2 to 3");
/// ```
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub struct Arity {
    min: u8,
    max: Option<u8>,
}

/// An argument-count range with no count in it.
#[derive(Error, Copy, Clone, Debug, PartialEq, Eq)]
#[error("the argument-count range is empty")]
pub struct EmptyArity;

impl Arity {
    /// Any number of arguments.
    pub const ANY: Self = Self { min: 0, max: None };

    /// Exactly `n` arguments.
    pub const fn exactly(n: u8) -> Self {
        Self {
            min: n,
            max: Some(n),
        }
    }

    /// `n` or more arguments.
    pub const fn at_least(n: u8) -> Self {
        Self { min: n, max: None }
    }

    /// `min..=max` arguments.
    ///
    /// # Panics
    ///
    /// When `min` is above `max` (in a `const`, a compile error); [`Arity::checked_between`]
    /// returns `None` instead.
    pub const fn between(min: u8, max: u8) -> Self {
        match Self::checked_between(min, max) {
            Some(arity) => arity,
            None => panic!("an empty argument-count range: min is above max"),
        }
    }

    /// `min..=max` arguments; `None` when `min` is above `max`.
    pub const fn checked_between(min: u8, max: u8) -> Option<Self> {
        if min <= max {
            Some(Self {
                min,
                max: Some(max),
            })
        } else {
            None
        }
    }

    /// The smallest count.
    pub const fn min(self) -> u8 {
        self.min
    }

    /// The largest count; `None` for no upper bound.
    pub const fn max(self) -> Option<u8> {
        self.max
    }

    /// Whether `count` arguments are in the range.
    pub const fn contains(self, count: usize) -> bool {
        count >= self.min as usize
            && match self.max {
                Some(max) => count <= max as usize,
                None => true,
            }
    }

    /// The counts followed by `argument` or `arguments` (`1 argument`, `1 to 3 arguments`).
    #[cfg(feature = "compiler")]
    pub(crate) fn arguments(self) -> String {
        let one = self.min == 1 && matches!(self.max, Some(1) | None);
        format!("{self} argument{}", if one { "" } else { "s" })
    }
}

impl Default for Arity {
    /// [`Arity::ANY`].
    fn default() -> Self {
        Self::ANY
    }
}

impl From<u8> for Arity {
    /// [`Arity::exactly`].
    fn from(n: u8) -> Self {
        Self::exactly(n)
    }
}

impl From<RangeFrom<u8>> for Arity {
    /// [`Arity::at_least`].
    fn from(range: RangeFrom<u8>) -> Self {
        Self::at_least(range.start)
    }
}

impl TryFrom<RangeInclusive<u8>> for Arity {
    type Error = EmptyArity;

    /// `start..=end`; an error for an empty or exhausted range.
    fn try_from(range: RangeInclusive<u8>) -> Result<Self, EmptyArity> {
        if range.is_empty() {
            return Err(EmptyArity);
        }
        Self::checked_between(*range.start(), *range.end()).ok_or(EmptyArity)
    }
}

impl fmt::Display for Arity {
    /// `1`, `1 to 3`, `at least 2`, `any number of`: the count in front of "arguments".
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match (self.min, self.max) {
            (0, None) => f.write_str("any number of"),
            (min, None) => write!(f, "at least {min}"),
            (min, Some(max)) if min == max => write!(f, "{min}"),
            (min, Some(max)) => write!(f, "{min} to {max}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_constructors_and_the_getters() {
        assert_eq!((Arity::ANY.min(), Arity::ANY.max()), (0, None));
        assert_eq!(
            (Arity::exactly(2).min(), Arity::exactly(2).max()),
            (2, Some(2))
        );
        assert_eq!(
            (Arity::at_least(3).min(), Arity::at_least(3).max()),
            (3, None)
        );
        assert_eq!(
            Arity::checked_between(1, 4).map(|a| (a.min(), a.max())),
            Some((1, Some(4)))
        );
        assert_eq!(Arity::checked_between(0, 0), Some(Arity::exactly(0)));
        assert_eq!(Arity::checked_between(255, 255), Some(Arity::exactly(255)));
        assert_eq!(Arity::checked_between(2, 1), None);
        assert_eq!(Arity::between(1, 4), Arity::checked_between(1, 4).unwrap());
        assert_eq!(Arity::default(), Arity::ANY);
        assert_eq!(Arity::from(3), Arity::exactly(3));
        assert_eq!(Arity::from(3..), Arity::at_least(3));
        assert_eq!(Arity::from(0..), Arity::ANY);
    }

    #[test]
    #[should_panic(expected = "an empty argument-count range")]
    fn between_panics_on_an_empty_range() {
        let (min, max) = (std::hint::black_box(2), 1);
        let _ = Arity::between(min, max);
    }

    const SPELLED: [(Arity, &str, &str); 8] = [
        (Arity::exactly(0), "0", "0 arguments"),
        (Arity::exactly(1), "1", "1 argument"),
        (Arity::exactly(2), "2", "2 arguments"),
        (Arity::between(1, 3), "1 to 3", "1 to 3 arguments"),
        (Arity::between(0, 1), "0 to 1", "0 to 1 arguments"),
        (Arity::at_least(1), "at least 1", "at least 1 argument"),
        (Arity::at_least(2), "at least 2", "at least 2 arguments"),
        (Arity::ANY, "any number of", "any number of arguments"),
    ];

    #[test]
    fn display_spells_the_counts() {
        for (arity, text, _) in SPELLED {
            assert_eq!(arity.to_string(), text);
        }
    }

    #[cfg(feature = "compiler")]
    #[test]
    fn arguments_spells_the_counts_and_the_noun() {
        for (arity, _, arguments) in SPELLED {
            assert_eq!(arity.arguments(), arguments);
        }
    }

    #[test]
    fn an_inclusive_range_converts_unless_it_is_empty() {
        assert_eq!(Arity::try_from(1..=3), Ok(Arity::between(1, 3)));
        assert_eq!(Arity::try_from(4..=4), Ok(Arity::exactly(4)));
        assert_eq!(Arity::try_from(RangeInclusive::new(3, 1)), Err(EmptyArity));
        let mut exhausted = 4..=4;
        assert_eq!(exhausted.next(), Some(4));
        assert_eq!(
            Arity::try_from(exhausted),
            Err(EmptyArity),
            "an exhausted range is empty"
        );
        assert_eq!(EmptyArity.to_string(), "the argument-count range is empty");
    }

    #[test]
    fn contains_checks_both_bounds() {
        let a = Arity::between(1, 2);
        assert!(!a.contains(0) && a.contains(1) && a.contains(2) && !a.contains(3));
        assert!(Arity::ANY.contains(0) && Arity::ANY.contains(usize::MAX));
        assert!(!Arity::at_least(2).contains(1) && Arity::at_least(2).contains(300));
        assert!(Arity::exactly(255).contains(255) && !Arity::exactly(255).contains(256));
    }
}
