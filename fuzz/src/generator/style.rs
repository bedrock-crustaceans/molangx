//! The style of a printed program and the cyclic byte streams its choices are drawn from.

use arbitrary::{Arbitrary, Result, Unstructured};

/// How a [`Program`](super::Program) is printed: the bytes the printer draws its choices from,
/// cyclically. An empty stream is the canonical form (no redundant parentheses, short namespaces,
/// lower case, one space between tokens).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Style {
    /// Choices that can change the parse: redundant parentheses, number spelling, namespace
    /// spelling.
    pub structure: Vec<u8>,
    /// Choices that must not change the compiled program: letter case outside strings and the
    /// whitespace between tokens.
    pub cosmetic: Vec<u8>,
}

impl<'a> Arbitrary<'a> for Style {
    fn arbitrary(u: &mut Unstructured<'a>) -> Result<Self> {
        let structure_len = u.int_in_range(0..=32usize)?;
        let structure = u.bytes(structure_len.min(u.len()))?.to_vec();
        let cosmetic_len = u.int_in_range(0..=32usize)?;
        let cosmetic = u.bytes(cosmetic_len.min(u.len()))?.to_vec();
        Ok(Self {
            structure,
            cosmetic,
        })
    }
}

impl Style {
    /// The same structure with other cosmetics.
    #[must_use]
    pub fn with_cosmetic(&self, cosmetic: Vec<u8>) -> Self {
        Self {
            structure: self.structure.clone(),
            cosmetic,
        }
    }
}

/// A cyclic reader over a choice stream.
pub(super) struct Choices<'s> {
    bytes: &'s [u8],
    /// The number of draws so far.
    pub(super) at: usize,
}

impl<'s> Choices<'s> {
    pub(super) fn new(bytes: &'s [u8]) -> Self {
        Self { bytes, at: 0 }
    }

    pub(super) fn next(&mut self) -> u8 {
        if self.bytes.is_empty() {
            return 0;
        }
        let byte = self.bytes[self.at % self.bytes.len()];
        // Mix in the position so a short stream does not repeat one pattern exactly.
        let mixed = byte
            .wrapping_add((self.at / self.bytes.len()) as u8)
            .rotate_left((self.at % 7) as u32);
        self.at += 1;
        mixed
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::generator::test_support::*;

    #[test]
    fn an_empty_choice_stream_is_all_zeros_and_never_advances() {
        let mut choices = Choices::new(&[]);
        for _ in 0..10 {
            assert_eq!(choices.next(), 0);
        }
        assert_eq!(choices.at, 0);
    }

    #[test]
    fn a_choice_stream_mixes_the_position_into_each_byte() {
        // Draw k of the one-byte stream [1] is (1 + k).rotate_left(k % 7).
        let mut choices = Choices::new(&[1]);
        let drawn: Vec<u8> = (0..4).map(|_| choices.next()).collect();
        assert_eq!(drawn, [1, 4, 12, 32]);
        let mut choices = Choices::new(&[1]);
        for k in 0u32..40 {
            let expected = 1u8.wrapping_add(k as u8).rotate_left(k % 7);
            assert_eq!(choices.next(), expected, "draw {k}");
        }
        assert_eq!(choices.at, 40);
    }

    #[test]
    fn a_choice_stream_is_cyclic_and_changes_on_each_lap() {
        let mut choices = Choices::new(&[10, 20]);
        // Lap 0: the bytes as they are (no rotation at position 0, 1 rotation at position 1).
        assert_eq!(choices.next(), 10);
        assert_eq!(choices.next(), 20u8.rotate_left(1));
        // Lap 1: one added to each byte before the rotation.
        assert_eq!(choices.next(), 11u8.rotate_left(2));
        assert_eq!(choices.next(), 21u8.rotate_left(3));
        // Lap 2.
        assert_eq!(choices.next(), 12u8.rotate_left(4));
    }

    #[test]
    fn a_choice_stream_is_deterministic() {
        let draw = |bytes: &[u8]| {
            let mut choices = Choices::new(bytes);
            (0..100).map(|_| choices.next()).collect::<Vec<u8>>()
        };
        let data = buffer(7, 5);
        assert_eq!(draw(&data), draw(&data));
        assert_ne!(draw(&data), draw(&buffer(8, 5)));
    }

    #[test]
    fn the_default_style_has_empty_streams() {
        let default = Style::default();
        assert!(default.structure.is_empty() && default.cosmetic.is_empty());
    }

    #[test]
    fn with_cosmetic_keeps_the_structure_and_leaves_the_original_alone() {
        let original = style(&[9, 8, 7], &[1, 2]);
        let changed = original.with_cosmetic(vec![5]);
        assert_eq!(changed.structure, [9, 8, 7]);
        assert_eq!(changed.cosmetic, [5]);
        assert_eq!(original.cosmetic, [1, 2]);
        assert_eq!(original.with_cosmetic(vec![]), style(&[9, 8, 7], &[]));
    }

    #[test]
    fn an_arbitrary_style_has_two_streams_of_at_most_thirty_two_bytes() {
        assert_eq!(
            Style::arbitrary(&mut Unstructured::new(&[])).expect("total"),
            Style::default()
        );
        for seed in 0..200 {
            let data = buffer(seed, 100);
            let style = Style::arbitrary(&mut Unstructured::new(&data)).expect("total");
            assert!(style.structure.len() <= 32 && style.cosmetic.len() <= 32);
            assert_eq!(
                style,
                Style::arbitrary(&mut Unstructured::new(&data)).expect("total")
            );
        }
        // A length is a byte modulo 33: 32 picks the longest streams, 33 the empty ones.
        let style = Style::arbitrary(&mut Unstructured::new(&[32; 100])).expect("total");
        assert_eq!((style.structure.len(), style.cosmetic.len()), (32, 32));
        let style = Style::arbitrary(&mut Unstructured::new(&[33; 100])).expect("total");
        assert_eq!((style.structure.len(), style.cosmetic.len()), (0, 0));
    }
}
