//! String literals of the lexer.

use super::{Lexer, TokenFailed};
use crate::diag::LanguageMessage as Msg;

impl Lexer<'_, '_> {
    /// The index of the closing quote of the string that opens at `start`.
    ///
    /// A `\` skips the two bytes after it, so neither of them can close the string.
    pub(super) fn scan_string(&mut self, start: usize) -> Result<usize, TokenFailed> {
        let mut p = start + 1;
        loop {
            match self.at(p) {
                // Past the end `at` reads 0, so a skip off the end is the missing-quote error.
                b'\\' => p += 3,
                // `lower` cut the text at its first NUL, so a 0 is the end of the input.
                0 => break,
                b'\'' => return Ok(p),
                _ => p += 1,
            }
        }
        let span = self.span(start, self.text.len());
        self.cx.language(Msg::StringMissingQuote, span, &[]);
        Err(TokenFailed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compile::{Cx, ast::Payload, lex::test_support::*};
    use crate::diag::{LanguageMessage as Msg, Severity};
    use crate::hash::HashedStr;

    #[test]
    fn scan_string_returns_the_index_of_the_closing_quote() {
        let opts = client();
        for (text, closing) in [("'ab'", 3), ("''", 1), ("'a\\'b'", 5), ("'a b' c", 4)] {
            let mut cx = Cx::for_test(text, &opts);
            let mut lexer = Lexer {
                cx: &mut cx,
                text: text.as_bytes(),
                pos: 0,
            };
            assert_eq!(lexer.scan_string(0).ok(), Some(closing), "{text:?}");
            assert!(cx.logged_diagnostics().is_empty());
        }
    }

    #[test]
    fn scan_string_without_a_closing_quote_logs_the_missing_quote() {
        let opts = client();
        for text in ["'", "'unterminated", "'ab\\'", "'\\x'", "'\\", "'a\\b'"] {
            let mut cx = Cx::for_test(text, &opts);
            let mut lexer = Lexer {
                cx: &mut cx,
                text: text.as_bytes(),
                pos: 0,
            };
            assert!(lexer.scan_string(0).is_err(), "{text:?}");
            let log = cx.logged_diagnostics();
            assert_eq!(log.len(), 1, "{text:?}");
            assert_eq!(log[0].language_message().map(Msg::id), Some("E04"));
            assert_eq!(
                (log[0].span().start, log[0].span().end),
                (0, text.len() as u32)
            );
        }
    }

    #[test]
    fn a_string_is_a_hash_token_of_the_bytes_as_written() {
        assert_eq!(
            toks("'abc'"),
            [(
                Op::StringLiteral,
                Payload::Hash(HashedStr::new("abc").as_u64()),
                (0, 5)
            )]
        );
        assert_eq!(toks("''"), [(Op::StringLiteral, Payload::Hash(0), (0, 2))]);
    }

    #[test]
    fn a_string_keeps_its_case_and_spaces() {
        assert_eq!(
            toks("'A b'")[0].1,
            Payload::Hash(HashedStr::new("A b").as_u64())
        );
        assert_ne!(toks("'A'")[0].1, toks("'a'")[0].1);
    }

    #[test]
    fn a_string_is_followed_by_ordinary_tokens() {
        let t = toks("'a'=='b'");
        assert_eq!(
            t.iter().map(|t| t.0).collect::<Vec<_>>(),
            [Op::StringLiteral, Op::LogicalEqual, Op::StringLiteral]
        );
        assert_eq!(t[2].2, (5, 8));
    }

    #[test]
    fn a_backslash_makes_the_scanner_skip_three_bytes() {
        let t = toks("'a\\'xb'");
        assert_eq!(
            t,
            [(
                Op::StringLiteral,
                Payload::Hash(HashedStr::from_bytes(b"a\\'xb").as_u64()),
                (0, 7)
            )]
        );
        assert_eq!(toks("'a\\xy'")[0].2, (0, 6));
    }

    #[test]
    fn an_escape_far_from_the_start_still_leaves_room_for_the_closing_quote() {
        // Three bytes on from the backslash is the closing quote.
        assert_eq!(
            toks("'ab\\xy'"),
            [(
                Op::StringLiteral,
                Payload::Hash(HashedStr::from_bytes(b"ab\\xy").as_u64()),
                (0, 7)
            )]
        );
        let t = toks("1 + 'abcd\\xy' == 'q'");
        assert_eq!(t[2].2, (4, 13));
        assert_eq!(
            t[2].1,
            Payload::Hash(HashedStr::from_bytes(b"abcd\\xy").as_u64())
        );
    }

    #[test]
    fn a_quote_hidden_by_the_three_byte_skip_does_not_close_the_string() {
        let r = run("'\\xx'");
        assert!(r.tokens.is_some());
        let r = run("'\\x'");
        assert!(r.tokens.is_none());
        assert_eq!(r.log[0].0, "E04");
    }

    #[test]
    fn an_unterminated_string_logs_missing_quote_then_unrecognized() {
        let r = run("1 'ab");
        assert!(r.tokens.is_none());
        assert_eq!(
            r.log[0],
            (
                "E04",
                Severity::Error,
                (2, 5),
                "Error: Molang string missing final ' character".to_owned()
            )
        );
        assert_eq!(
            r.log[1],
            (
                "E02",
                Severity::Error,
                (2, 5),
                "unrecognized token: 'ab".to_owned()
            )
        );
    }

    #[test]
    fn a_string_may_hold_operators_and_non_ascii() {
        assert_eq!(toks("'+-*/ \u{e9}'").len(), 1);
    }

    mod tree_shapes {
        use crate::compile::test_support::pipeline::{at, tree};
        use crate::hash::HashedStr;

        /// A string is not unescaped: it hashes its raw bytes.
        #[test]
        fn strings() {
            for valid in ["'a\\'b'", "'a\\bc'", "'\\xy'", "''", "' '", "'a=b;c'"] {
                let compiled = at(valid, 13);
                assert_eq!(compiled.failure(), None, "{valid}");
                let inner = &valid[1..valid.len() - 1];
                assert_eq!(
                    compiled.tree_notation(9).as_deref(),
                    Some(HashedStr::new(inner).as_u64().to_string().as_str()),
                    "{valid}"
                );
            }
            assert_eq!(tree("''"), "0", "the empty string hashes to 0");
            assert_eq!(tree("'a'"), "12638153115695167422");
            assert_eq!(tree("'abc'"), "15626587013303479755");
        }
    }
}
