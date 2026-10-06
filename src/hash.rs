//! [`HashedStr`]: the string hash, 64-bit FNV-1 (multiply, then xor; not FNV-1a) with the empty
//! string hashing to 0 instead of the offset basis, which `'' == 0` exposes.
//!
//! Every Molang string value is this hash at run time.
//!
//! ```
//! use molangx::hash::HashedStr;
//!
//! assert_eq!(HashedStr::new("a").as_u64(), 12_638_153_115_695_167_422);
//! assert_eq!(HashedStr::new(""), HashedStr::EMPTY);
//! assert_eq!(HashedStr::EMPTY.as_u64(), 0);
//! // `const`, so tables of names can be hashed at compile time.
//! const GREETING: HashedStr = HashedStr::new("hello");
//! assert_eq!(GREETING, HashedStr::new("hello"));
//! ```

/// The 64-bit FNV offset basis.
pub(crate) const FNV1_OFFSET_BASIS: u64 = 0xcbf2_9ce4_8422_2325;

/// The 64-bit FNV prime.
const FNV1_PRIME: u64 = 0x0000_0100_0000_01b3;

/// The string hash: 64-bit FNV-1 over `bytes`, with an empty input hashing to 0.
///
/// Hashing stops at the first NUL byte, so a leading NUL hashes to 0.
pub const fn fnv1_64(bytes: &[u8]) -> u64 {
    if let [] | [0, ..] = bytes {
        return 0;
    }
    let mut h = FNV1_OFFSET_BASIS;
    let mut rest = bytes;
    while let [byte @ 1..=u8::MAX, tail @ ..] = rest {
        h = fnv1_step(h, *byte);
        rest = tail;
    }
    h
}

/// One FNV-1 round: `hash` with `byte` mixed in.
pub(crate) const fn fnv1_step(hash: u64, byte: u8) -> u64 {
    hash.wrapping_mul(FNV1_PRIME) ^ byte as u64
}

/// A string as held at run time: its [`fnv1_64`] hash.
///
/// The text is not kept: `==` / `!=` compare hashes and names are keyed by the hash. A caller that
/// needs the text keeps it next to the hash.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct HashedStr(u64);

impl HashedStr {
    /// The hash of the empty string: 0.
    pub const EMPTY: Self = Self(0);

    /// The hash of `s`'s UTF-8 bytes as written: no case folding, no unescaping.
    pub const fn new(s: &str) -> Self {
        Self(fnv1_64(s.as_bytes()))
    }

    /// The hash of raw bytes, which need not be UTF-8.
    pub const fn from_bytes(bytes: &[u8]) -> Self {
        Self(fnv1_64(bytes))
    }

    /// A hash computed elsewhere (a data file, the network, a string value).
    pub const fn from_u64(hash: u64) -> Self {
        Self(hash)
    }

    /// The 64-bit hash value.
    pub const fn as_u64(self) -> u64 {
        self.0
    }

    /// Whether this is the hash of the empty string (0).
    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }
}

impl From<&str> for HashedStr {
    fn from(s: &str) -> Self {
        Self::new(s)
    }
}

impl From<u64> for HashedStr {
    fn from(hash: u64) -> Self {
        Self(hash)
    }
}

impl From<HashedStr> for u64 {
    fn from(h: HashedStr) -> Self {
        h.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn conformance_vectors() {
        assert_eq!(HashedStr::new("a").as_u64(), 12_638_153_115_695_167_422);
        assert_eq!(HashedStr::new("abc").as_u64(), 15_626_587_013_303_479_755);
        assert_eq!(HashedStr::new("ABC").as_u64(), 15_595_941_425_208_037_995);
        assert_eq!(HashedStr::new(" ").as_u64(), 12_638_153_115_695_167_487);
        assert_eq!(fnv1_64(b"moo"), 15_615_043_240_721_163_028);
    }

    #[test]
    fn the_empty_string_and_a_leading_nul_hash_to_zero_not_the_basis() {
        assert_eq!(HashedStr::new("").as_u64(), 0);
        assert_eq!(fnv1_64(b""), 0);
        assert_eq!(fnv1_64(b"\0"), 0);
        assert_eq!(fnv1_64(b"\0abc"), 0);
        assert_eq!(HashedStr::new(""), HashedStr::EMPTY);
        assert_eq!(HashedStr::EMPTY.as_u64(), 0);
        assert_eq!(HashedStr::from_bytes(&[0]), HashedStr::EMPTY);
        assert_eq!(HashedStr::from_bytes(b""), HashedStr::EMPTY);
        assert_eq!(HashedStr::new("\0x"), HashedStr::EMPTY);
        assert!(HashedStr::EMPTY.is_empty());
        assert!(!HashedStr::new("a").is_empty());
        assert_eq!(HashedStr::default(), HashedStr::EMPTY);
        assert_ne!(FNV1_OFFSET_BASIS, 0);
    }

    #[test]
    fn fnv1_multiplies_then_xors() {
        assert_eq!(FNV1_OFFSET_BASIS, 0xcbf2_9ce4_8422_2325);
        assert_eq!(FNV1_PRIME, 0x0000_0100_0000_01b3);
        // FNV-1a would be (basis ^ byte) * prime.
        let fnv1 = FNV1_OFFSET_BASIS.wrapping_mul(FNV1_PRIME) ^ u64::from(b'a');
        let fnv1a = (FNV1_OFFSET_BASIS ^ u64::from(b'a')).wrapping_mul(FNV1_PRIME);
        assert_eq!(fnv1_64(b"a"), fnv1);
        assert_ne!(fnv1_64(b"a"), fnv1a);
        let input = b"query.get_name";
        let mut h = FNV1_OFFSET_BASIS;
        for &b in input {
            h = h.wrapping_mul(FNV1_PRIME) ^ u64::from(b);
        }
        assert_eq!(fnv1_64(input), h);
    }

    #[test]
    fn bytes_are_hashed_as_written() {
        assert_ne!(HashedStr::new("abc"), HashedStr::new("ABC"));
        assert_eq!(HashedStr::new("a\\'b"), HashedStr::from_bytes(b"a\\'b"));
        assert_ne!(HashedStr::new("a\\'b"), HashedStr::new("a'b"));
        assert_eq!(
            HashedStr::from_bytes(&[0xff, 0xfe]).as_u64(),
            fnv1_64(&[0xff, 0xfe])
        );
    }

    #[test]
    fn hashing_stops_at_the_first_nul() {
        assert_eq!(fnv1_64(b"abc\0def"), fnv1_64(b"abc"));
        assert_eq!(fnv1_64(b"ab\0cd"), fnv1_64(b"ab"));
        assert_eq!(fnv1_64(b"ab\0\0"), fnv1_64(b"ab"));
        assert_eq!(HashedStr::from_bytes(b"ab\0cd"), HashedStr::new("ab"));
        assert_ne!(fnv1_64(b"ab"), 0);
    }

    #[test]
    fn hashed_str_is_a_copy_u64() {
        assert_eq!(size_of::<HashedStr>(), size_of::<u64>());
        let h = HashedStr::from("moo");
        let copy = h;
        assert_eq!(h, copy);
        assert_eq!(u64::from(h), 15_615_043_240_721_163_028);
        assert_eq!(HashedStr::from(15_615_043_240_721_163_028_u64), h);
        assert_eq!(HashedStr::from_u64(h.as_u64()), h);
    }

    #[test]
    fn hashing_is_const() {
        const GREETING: HashedStr = HashedStr::new("hello");
        const PLANET: u64 = fnv1_64(b"planet");
        assert_eq!(GREETING, HashedStr::new("hello"));
        assert_eq!(PLANET, HashedStr::new("planet").as_u64());
    }

    #[test]
    fn one_byte_is_basis_times_prime_xor_byte() {
        for b in 1..=255u8 {
            let fnv1 = FNV1_OFFSET_BASIS.wrapping_mul(FNV1_PRIME) ^ u64::from(b);
            let fnv1a = (FNV1_OFFSET_BASIS ^ u64::from(b)).wrapping_mul(FNV1_PRIME);
            assert_eq!(fnv1_64(&[b]), fnv1, "byte {b}");
            assert_ne!(fnv1_64(&[b]), fnv1a, "byte {b}");
        }
    }

    #[test]
    fn is_empty_is_a_test_of_the_hash_value() {
        assert!(HashedStr::from_u64(0).is_empty());
        assert!(!HashedStr::from_u64(1).is_empty());
        assert!(!HashedStr::from_u64(u64::MAX).is_empty());
    }

    #[test]
    fn a_byte_above_ascii_is_not_mistaken_for_nul() {
        let bytes = [0xff, 0xfe, 0x00, 0x01];
        assert_eq!(
            HashedStr::from_bytes(&bytes[..2]).as_u64(),
            fnv1_64(&[0xff, 0xfe])
        );
        assert_eq!(
            HashedStr::from_bytes(&bytes),
            HashedStr::from_bytes(&bytes[..2])
        );
        assert_ne!(fnv1_64(&[0xff]), 0);
        assert_ne!(fnv1_64(&[0xff]), fnv1_64(&[0x7f]));
    }

    #[test]
    fn multi_byte_utf8_is_hashed_byte_for_byte() {
        assert_eq!(HashedStr::new("é"), HashedStr::from_bytes(&[0xc3, 0xa9]));
        assert_ne!(HashedStr::new("é"), HashedStr::new("e"));
    }

    #[test]
    fn conversions_and_ordering() {
        assert_eq!(u64::from(HashedStr::from_u64(5)), 5);
        assert_eq!(HashedStr::from(5_u64), HashedStr::from_u64(5));
        assert_eq!(HashedStr::from("x"), HashedStr::new("x"));
        assert!(HashedStr::from_u64(1) < HashedStr::from_u64(2));
        assert!(HashedStr::from_u64(u64::MAX) > HashedStr::EMPTY);
        assert_eq!(HashedStr::default(), HashedStr::from_u64(0));
    }

    #[test]
    fn equal_hashes_hash_equal_under_std_hash() {
        use std::hash::{BuildHasher, RandomState};
        let state = RandomState::new();
        let a = state.hash_one(HashedStr::new("moo"));
        let b = state.hash_one(HashedStr::from_u64(15_615_043_240_721_163_028));
        assert_eq!(a, b);
    }

    #[test]
    fn debug_shows_the_wrapped_value() {
        assert_eq!(format!("{:?}", HashedStr::from_u64(7)), "HashedStr(7)");
        assert_eq!(format!("{:?}", HashedStr::EMPTY), "HashedStr(0)");
    }

    proptest::proptest! {
        #![proptest_config(proptest::prelude::ProptestConfig { failure_persistence: None, ..Default::default() })]

        #[test]
        fn appending_a_byte_multiplies_then_xors(
            prefix in proptest::collection::vec(1u8..=255, 1..32),
            byte in 1u8..=255,
        ) {
            let mut longer = prefix.clone();
            longer.push(byte);
            let expected = fnv1_64(&prefix).wrapping_mul(FNV1_PRIME) ^ u64::from(byte);
            proptest::prop_assert_eq!(fnv1_64(&longer), expected);
        }

        #[test]
        fn nothing_after_a_nul_matters(
            prefix in proptest::collection::vec(1u8..=255, 0..16),
            suffix in proptest::collection::vec(proptest::prelude::any::<u8>(), 0..16),
        ) {
            let mut text = prefix.clone();
            text.push(0);
            text.extend_from_slice(&suffix);
            proptest::prop_assert_eq!(fnv1_64(&text), fnv1_64(&prefix));
        }
    }
}
