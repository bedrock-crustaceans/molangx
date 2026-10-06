//! Case folding and the character classes of the lexer.

/// Lower-cases ASCII `A`–`Z` outside `'…'` strings and cuts the text at its first NUL byte.
///
/// A `\` protects the byte after it, inside and outside strings.
pub(in crate::compile) fn lower(src: &str) -> Vec<u8> {
    let bytes = src.as_bytes();
    let end = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
    let mut out = bytes[..end].to_vec();
    let mut in_string = false;
    let mut escaped = false;
    for c in &mut out {
        match *c {
            _ if escaped => escaped = false,
            b'\'' => in_string = !in_string,
            b'\\' => escaped = true,
            _ if !in_string => c.make_ascii_lowercase(),
            _ => {}
        }
    }
    out
}

pub(super) const fn is_whitespace(c: u8) -> bool {
    matches!(c, b' ' | b'\t' | b'\n' | b'\r')
}

pub(super) const fn is_name_start(c: u8) -> bool {
    c == b'_' || c.is_ascii_alphabetic()
}

pub(super) const fn is_name_char(c: u8) -> bool {
    c == b'_' || c.is_ascii_alphanumeric()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compile::{ast::Payload, lex::test_support::*};
    use crate::hash::HashedStr;
    use proptest::prelude::*;

    fn lowered(src: &str) -> String {
        String::from_utf8(lower(src)).unwrap()
    }

    #[test]
    fn lower_folds_ascii_letters_outside_strings() {
        assert_eq!(lowered("ABC xyZ"), "abc xyz");
        assert_eq!(lowered("Query.Is_Baby + V.X"), "query.is_baby + v.x");
        assert_eq!(lowered("0123 -+*/ []{}()"), "0123 -+*/ []{}()");
    }

    #[test]
    fn lower_leaves_non_ascii_bytes_alone() {
        assert_eq!(lowered("\u{c9}"), "\u{c9}");
        assert_eq!(lowered("\u{e9}A"), "\u{e9}a");
    }

    #[test]
    fn lower_keeps_the_contents_of_strings() {
        assert_eq!(lowered("'ABC'xyZ"), "'ABC'xyz");
        assert_eq!(lowered("X'AbC'Y"), "x'AbC'y");
    }

    #[test]
    fn lower_toggles_on_every_quote() {
        assert_eq!(lowered("'A'B'C'"), "'A'b'C'");
        assert_eq!(lowered("''A"), "''a");
        assert_eq!(lowered("'AB"), "'AB");
    }

    #[test]
    fn lower_backslash_protects_the_next_byte_outside_a_string() {
        assert_eq!(lowered("A\\B"), "a\\B");
        assert_eq!(lowered("\\AB"), "\\Ab");
    }

    #[test]
    fn lower_backslash_protects_a_quote_inside_a_string() {
        // `\'` does not close the string, so the `B` after it is still inside.
        assert_eq!(lowered("'A\\'B'C"), "'A\\'B'c");
    }

    #[test]
    fn lower_backslash_skips_exactly_one_byte() {
        // The first `\` protects the second one; the `A` after them is lowered.
        assert_eq!(lowered("\\\\A"), "\\\\a");
        // A backslash outside a string protects an opening quote, so the string never starts.
        assert_eq!(lowered("\\'A'"), "\\'a'");
    }

    #[test]
    fn lower_trailing_backslash_does_not_run_past_the_end() {
        assert_eq!(lowered("A\\"), "a\\");
        assert_eq!(lowered("\\"), "\\");
    }

    #[test]
    fn lower_ends_at_the_first_nul() {
        assert_eq!(lower("AB\0CD"), b"ab");
        assert_eq!(lower("\0A"), b"");
        assert_eq!(lower("'A\0B'"), b"'A");
        assert_eq!(lower(""), b"");
    }

    #[test]
    fn lower_string_scanner_and_lowering_disagree_after_a_backslash() {
        // `lower` skips one byte after `\`, the string scanner two: the `B` keeps its case.
        let src = "'a\\'B'";
        assert_eq!(lowered(src), "'a\\'B'");
        let t = toks(src);
        assert_eq!(
            t,
            [(
                Op::StringLiteral,
                Payload::Hash(HashedStr::from_bytes(b"a\\'B").as_u64()),
                (0, 6)
            )]
        );
    }

    #[test]
    fn whitespace_is_space_tab_cr_lf_only() {
        for c in 0..=255u8 {
            assert_eq!(
                is_whitespace(c),
                matches!(c, b' ' | b'\t' | b'\r' | b'\n'),
                "byte {c:#x}"
            );
        }
    }

    #[test]
    fn name_start_is_a_letter_or_underscore() {
        for c in 0..=255u8 {
            assert_eq!(
                is_name_start(c),
                c == b'_' || c.is_ascii_alphabetic(),
                "byte {c:#x}"
            );
        }
    }

    #[test]
    fn name_char_is_a_letter_digit_or_underscore() {
        for c in 0..=255u8 {
            assert_eq!(
                is_name_char(c),
                c == b'_' || c.is_ascii_alphanumeric(),
                "byte {c:#x}"
            );
        }
        assert!(!is_name_char(b'.'));
    }

    proptest! {
        #![proptest_config(config())]

        #[test]
        fn lowering_is_idempotent_and_keeps_the_length(src in ascii_source()) {
            let once = lower(&src);
            prop_assert_eq!(once.len(), src.len());
            let again = lower(std::str::from_utf8(&once).unwrap());
            prop_assert_eq!(&again, &once);
            for (a, b) in src.bytes().zip(&once) {
                prop_assert!(a == *b || (a.is_ascii_uppercase() && *b == a.to_ascii_lowercase()));
            }
        }
    }

    mod tree_shapes {
        use crate::compile::test_support::pipeline::tree;
        use crate::hash::HashedStr;

        fn hash_tree(text: &str) -> String {
            HashedStr::new(text).as_u64().to_string()
        }

        #[test]
        fn case_is_folded_outside_strings() {
            assert_eq!(tree("V.X"), "v.x");
            assert_eq!(tree("Variable.X_1"), "v.x_1");
            assert_eq!(tree("MATH.PI"), tree("math.pi"));
            assert_eq!(tree("LOOP(3, {V.X = 1;});"), tree("loop(3, {v.x = 1;});"));
            assert_eq!(tree("'abc'"), HashedStr::new("abc").as_u64().to_string());
            assert_eq!(tree("'ABC'"), HashedStr::new("ABC").as_u64().to_string());
            assert_ne!(tree("'abc'"), tree("'ABC'"));
        }

        #[test]
        fn backslash_protects_the_next_byte_from_lowering() {
            // Inside a string the lowering loop skips one byte after `\`, the scanner two.
            assert_eq!(
                tree("'a\\'B'"),
                HashedStr::new("a\\'B").as_u64().to_string()
            );
        }

        /// In `'\\'X'` lowering ends the string at the second quote (so `X` is lowered) while the
        /// scanner reads `\\'X` as the string's content.
        #[test]
        fn lowering_and_string_scanning_disagree_about_backslashes() {
            assert_eq!(tree("'\\\\'X'"), hash_tree("\\\\'x"));
            assert_eq!(tree("'\\'X'"), hash_tree("\\'X"));
        }
    }
}
