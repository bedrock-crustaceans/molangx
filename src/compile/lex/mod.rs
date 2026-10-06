//! The lexer: splits the lowered source into leaf [`Node`] tokens for the parser to nest in place.
//!
//! The lowered copy, not the original text, is what is tokenised, hashed and quoted in messages.

use crate::compile::{
    Cx,
    ast::{Name, Node, Payload, Span},
};
use crate::diag::LanguageMessage as Msg;
use crate::hash::HashedStr;
use crate::ops::{ExpressionOp as Op, OpSet};
use std::sync::OnceLock;

mod chars;
mod literal;
mod query;
mod string;

pub(super) use chars::lower;
use chars::{is_name_char, is_name_start, is_whitespace};

/// The token list and the set of ops that occur in it.
pub(super) struct Tokens {
    pub(super) nodes: Vec<Node>,
    pub(super) used: OpSet,
}

/// The operator tokens that start with one byte, in op order.
type TokenBucket = Vec<(Op, &'static [u8])>;

/// The operator tokens bucketed by first byte, each bucket in byte order of the tokens.
fn token_table() -> &'static [TokenBucket; 256] {
    static TABLE: OnceLock<[TokenBucket; 256]> = OnceLock::new();
    TABLE.get_or_init(|| {
        let mut table: [TokenBucket; 256] = std::array::from_fn(|_| Vec::new());
        // Without the standard library a `math.*` name is only ever a host function.
        let known = |op: Op| cfg!(feature = "stdlib") || !op.is_math_function();
        for &op in Op::all().iter().filter(|&&op| known(op)) {
            if let Some(token) = op.token().filter(|t| !t.is_empty()) {
                table[usize::from(token.as_bytes()[0])].push((op, token.as_bytes()));
            }
        }
        for bucket in &mut table {
            // No two operators share a token (a unit test checks), so the order is total.
            bucket.sort_by_key(|&(_, token)| token);
        }
        table
    })
}

/// The longest token of `bucket` (in byte order) that `rest` starts with.
///
/// Every token `rest` starts with sorts at or before `rest`, and the longest of them sorts last.
fn longest_prefix(bucket: &[(Op, &'static [u8])], rest: &[u8]) -> Option<(Op, usize)> {
    let end = bucket.partition_point(|&(_, token)| token <= rest);
    bucket[..end]
        .iter()
        .rev()
        .find(|&&(_, token)| rest.starts_with(token))
        .map(|&(op, token)| (op, token.len()))
}

/// Why a token could not be read; every case logs `unrecognized token` and fails the lexer.
struct TokenFailed;

/// A token read at the lexer's position: its op, its value, and the byte it ends before.
struct Lexeme {
    op: Op,
    value: Payload,
    end: usize,
}

/// How far a name runs.
#[derive(Copy, Clone)]
enum NameChars {
    /// Letters, digits and underscores.
    Plain,
    /// Dots too (a resource name, `geometry.a.b`).
    Dotted,
}

/// What a namespace prefix makes of the name after it.
struct Namespace {
    /// The prefix of the canonical name (`variable.` for `v.` too).
    canonical: &'static str,
    chars: NameChars,
    payload: fn(Name) -> Payload,
}

/// The namespace an op's token opens, if it opens one.
fn namespace(op: Op) -> Option<Namespace> {
    let (canonical, chars, payload): (_, _, fn(Name) -> Payload) = match op {
        Op::EntityVariable => ("variable.", NameChars::Plain, Payload::Entity),
        Op::TempVariable => ("temp.", NameChars::Plain, Payload::Temp),
        Op::ContextVariable => ("context.", NameChars::Plain, Payload::Context),
        Op::ArrayVariable => ("array.", NameChars::Plain, Payload::ArrayVariable),
        Op::GeometryVariable => ("geometry.", NameChars::Dotted, Payload::Geometry),
        Op::MaterialVariable => ("material.", NameChars::Dotted, Payload::Material),
        Op::TextureVariable => ("texture.", NameChars::Dotted, Payload::Texture),
        // A member's name is stored without its dot.
        Op::MemberAccessor => ("", NameChars::Plain, Payload::Member),
        _ => return None,
    };
    Some(Namespace {
        canonical,
        chars,
        payload,
    })
}

struct Lexer<'c, 'o> {
    cx: &'c mut Cx<'o>,
    text: &'c [u8],
    pos: usize,
}

impl<'c> Lexer<'c, '_> {
    /// The byte at `i`, or the terminating NUL past the end.
    #[inline]
    fn at(&self, i: usize) -> u8 {
        self.text.get(i).copied().unwrap_or(0)
    }

    fn skip_whitespace(&mut self) {
        while is_whitespace(self.at(self.pos)) {
            self.pos += 1;
        }
    }

    fn span(&self, start: usize, end: usize) -> Span {
        Span::saturating(start, end.min(self.text.len()))
    }

    /// The end of a name starting at `start`, or `None` when no name starts there.
    fn scan_name(&self, start: usize, chars: NameChars) -> Option<usize> {
        if !is_name_start(self.at(start)) {
            return None;
        }
        let continues = |c: u8| match chars {
            NameChars::Plain => is_name_char(c),
            NameChars::Dotted => is_name_char(c) || c == b'.',
        };
        let mut end = start + 1;
        while continues(self.at(end)) {
            end += 1;
        }
        Some(end)
    }

    /// The text of a name [`scan_name`](Self::scan_name) found.
    fn name_str(&self, start: usize, end: usize) -> &'c str {
        // Names are ASCII by construction.
        std::str::from_utf8(&self.text[start..end]).unwrap_or("")
    }

    fn read_token(&mut self) -> Result<Lexeme, TokenFailed> {
        let start = self.pos;
        if let Some(host) = self.match_host_math(start) {
            return Ok(host);
        }
        let Some((op, len)) = self.match_operator(start) else {
            return self.read_literal(start, 0);
        };
        if let Some(namespace) = namespace(op) {
            return self.read_name(op, &namespace, start, len);
        }
        match op {
            Op::QueryFunction => self.read_query(start, len),
            Op::StringLiteral => self.read_string(start),
            _ => Ok(Lexeme {
                op,
                value: Payload::None,
                end: start + len,
            }),
        }
    }

    /// The host math function whose whole name starts at `start`.
    #[inline]
    fn match_host_math(&self, start: usize) -> Option<Lexeme> {
        let math = self.cx.opts.math?;
        if !self.text[start..].starts_with(b"math.") {
            return None;
        }
        let end = self.scan_name(start + "math.".len(), NameChars::Plain)?;
        let function = math.find(self.name_str(start, end))?;
        let op = if math.decl(function).is_volatile() {
            Op::HostMathVolatile
        } else {
            Op::HostMath
        };
        Some(Lexeme {
            op,
            value: Payload::HostMath(function),
            end,
        })
    }

    /// The operator at `start` and the length of its token, if one starts there.
    #[inline]
    fn match_operator(&self, start: usize) -> Option<(Op, usize)> {
        // A prefix test with no word boundary: `loopy` is `loop` then `y`.
        let longest = longest_prefix(
            &token_table()[usize::from(self.at(start))],
            &self.text[start..],
        );
        // An alias only when no longer operator matched.
        if longest.is_none_or(|(_, len)| len < 2)
            && self.at(start + 1) == b'.'
            && let Some(alias) = alias(self.at(start))
        {
            return Some((alias, 2));
        }
        longest
    }

    /// A namespace prefix of `len` bytes at `start` and the name after it; without a name, the
    /// text is read as a literal.
    #[inline]
    fn read_name(
        &mut self,
        op: Op,
        namespace: &Namespace,
        start: usize,
        len: usize,
    ) -> Result<Lexeme, TokenFailed> {
        let after = start + len;
        let Some(end) = self.scan_name(after, namespace.chars) else {
            return self.read_literal(start, len);
        };
        let name = Name::new(prefixed(namespace.canonical, self.name_str(after, end)));
        Ok(Lexeme {
            op,
            value: (namespace.payload)(name),
            end,
        })
    }

    /// A query prefix of `len` bytes at `start` and the name after it, resolved in the catalogue.
    #[inline]
    fn read_query(&mut self, start: usize, len: usize) -> Result<Lexeme, TokenFailed> {
        let after = start + len;
        // Unlike a variable prefix, a query prefix without a name does not fall through to the
        // literal scan: only `unrecognized token` is logged.
        let end = self.scan_name(after, NameChars::Plain).ok_or(TokenFailed)?;
        let suffix = self.name_str(after, end);
        let span = self.span(start, end);
        let Some(query) = self.resolve_query(suffix, span) else {
            let name = format!("query.{suffix}");
            self.cx.language(Msg::QueryUnresolved, span, &[&name]);
            return Err(TokenFailed);
        };
        Ok(Lexeme {
            op: Op::QueryFunction,
            value: Payload::Query(query),
            end,
        })
    }

    /// A `'…'` string starting at `start`, as the hash of its content.
    #[inline]
    fn read_string(&mut self, start: usize) -> Result<Lexeme, TokenFailed> {
        let closing = self.scan_string(start)?;
        let hash = HashedStr::from_bytes(&self.text[start + 1..closing]).as_u64();
        Ok(Lexeme {
            op: Op::StringLiteral,
            value: Payload::Hash(hash),
            end: closing + 1,
        })
    }
}

/// The op of the two-character alias (`v.`, `q.`, `c.`, `t.`) whose first byte is `first`.
#[inline]
fn alias(first: u8) -> Option<Op> {
    match first {
        b'v' => Some(Op::EntityVariable),
        b'q' => Some(Op::QueryFunction),
        b'c' => Some(Op::ContextVariable),
        b't' => Some(Op::TempVariable),
        _ => None,
    }
}

/// `prefix` followed by `name`, in a string of exactly that length (which becomes a name's
/// boxed text without a copy).
fn prefixed(prefix: &str, name: &str) -> String {
    let mut text = String::with_capacity(prefix.len() + name.len());
    text.push_str(prefix);
    text.push_str(name);
    text
}

/// The flat token list, or `None` after logging why there is none.
pub(super) fn scan(cx: &mut Cx<'_>, text: &[u8]) -> Option<Tokens> {
    let mut lexer = Lexer { cx, text, pos: 0 };
    lexer.skip_whitespace();
    if lexer.pos >= text.len() {
        // An empty expression is an error from version 4; below, it fails without a message.
        let span = Span::saturating(0, text.len());
        if lexer.cx.version().reports_unexpected_operators() {
            lexer.cx.language(Msg::NoTokens, span, &[]);
        } else {
            lexer.cx.silent_empty(span);
        }
        return None;
    }
    let mut tokens = Tokens {
        nodes: Vec::new(),
        used: OpSet::empty(),
    };
    while lexer.pos < text.len() {
        let start = lexer.pos;
        let Ok(Lexeme { op, value, end }) = lexer.read_token() else {
            let span = Span::saturating(start, text.len());
            lexer
                .cx
                .language_rest(Msg::UnrecognizedToken, span, start, text);
            return None;
        };
        tokens.used = tokens.used.with(op);
        tokens
            .nodes
            .push(Node::token(op, value, lexer.span(start, end)));
        lexer.pos = end;
        lexer.skip_whitespace();
    }
    Some(tokens)
}

#[cfg(test)]
pub(crate) mod test_support {
    use super::*;
    use crate::compile::CompileOptions;
    use crate::diag::Severity;
    use crate::version::MolangVersion;
    use proptest::prelude::*;

    pub(super) use crate::ops::ExpressionOp as Op;

    /// One token as the tests compare it: op, payload, span.
    pub(super) type Tok = (Op, Payload, (u32, u32));

    /// A logged diagnostic: language message id (`-` for a lint), severity, span, text.
    pub(super) type Logged = (&'static str, Severity, (u32, u32), String);

    pub(super) struct Run {
        pub tokens: Option<Vec<Tok>>,
        pub used: Vec<Op>,
        pub log: Vec<Logged>,
    }

    pub(super) fn client() -> CompileOptions {
        CompileOptions::client(MolangVersion::LATEST)
    }

    pub(super) fn run_with(src: &str, opts: &CompileOptions) -> Run {
        let mut cx = Cx::for_test(src, opts);
        let text = lower(src);
        let tokens = scan(&mut cx, &text);
        let log = cx
            .logged_diagnostics()
            .iter()
            .map(|d| {
                (
                    d.language_message().map_or("-", |v| v.id()),
                    d.severity(),
                    (d.span().start, d.span().end),
                    d.message().into_owned(),
                )
            })
            .collect();
        Run {
            used: tokens
                .as_ref()
                .map(|t| t.used.iter().collect())
                .unwrap_or_default(),
            tokens: tokens.map(|t| {
                t.nodes
                    .into_iter()
                    .map(|n| (n.op, n.value.clone(), (n.span.start, n.span.end)))
                    .collect()
            }),
            log,
        }
    }

    pub(super) fn run(src: &str) -> Run {
        run_with(src, &client())
    }

    /// The tokens of `src`, which must lex.
    pub(super) fn toks(src: &str) -> Vec<Tok> {
        let r = run(src);
        assert!(
            r.log.is_empty(),
            "unexpected messages for {src:?}: {:?}",
            r.log
        );
        r.tokens.unwrap_or_else(|| panic!("{src:?} did not lex"))
    }

    pub(super) fn ops(src: &str) -> Vec<Op> {
        toks(src).into_iter().map(|t| t.0).collect()
    }

    /// The single `Float` token `src` lexes to, as bits.
    pub(super) fn float_bits(src: &str) -> u32 {
        let t = toks(src);
        assert_eq!(t.len(), 1, "{src:?} lexed to {t:?}");
        assert_eq!(t[0].0, Op::Float);
        match t[0].1 {
            Payload::Float(v) => v.to_bits(),
            ref other => panic!("not a float: {other:?}"),
        }
    }

    pub(super) fn f(v: f32) -> u32 {
        v.to_bits()
    }

    /// The ids and texts of the messages `src` logs when it fails to lex.
    pub(super) fn failure(src: &str) -> Vec<(&'static str, String)> {
        let r = run(src);
        assert!(r.tokens.is_none(), "{src:?} lexed: {:?}", r.tokens);
        r.log
            .into_iter()
            .map(|(id, _, _, text)| (id, text))
            .collect()
    }

    pub(super) fn name(text: &str) -> Name {
        Name::new(text)
    }

    pub(super) fn ascii_source() -> impl Strategy<Value = String> {
        proptest::collection::vec(0x01u8..0x7f, 0..60)
            .prop_map(|b| b.into_iter().map(char::from).collect())
    }

    pub(super) fn config() -> ProptestConfig {
        ProptestConfig {
            cases: 256,
            failure_persistence: None,
            ..ProptestConfig::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::Side;
    use crate::compile::test_support::{host_math, math_opts, opts};
    use crate::compile::{CompileOptions, lex::test_support::*};
    use crate::diag::Severity;
    use crate::version::{MolangVersion, RawVersion};
    use proptest::prelude::*;

    fn host_tokens(src: &str) -> Vec<Tok> {
        let r = run_with(src, &math_opts());
        assert!(r.log.is_empty(), "{src}: {:?}", r.log);
        r.tokens.unwrap_or_default()
    }

    fn host(name: &str) -> Payload {
        Payload::HostMath(host_math().find(name).unwrap())
    }

    #[test]
    fn a_declared_host_math_name_lexes_as_its_function() {
        assert_eq!(
            host_tokens("math.twice"),
            [(Op::HostMath, host("math.twice"), (0, 10))]
        );
        assert_eq!(
            host_tokens("MATH.TWICE(1)")[0],
            (Op::HostMath, host("math.twice"), (0, 10))
        );
        assert_eq!(
            host_tokens("math.noise(1)")[0],
            (Op::HostMathVolatile, host("math.noise"), (0, 10))
        );
        assert_eq!(
            run_with("math.twice(1)", &math_opts()).used,
            [
                Op::LeftParenthesis,
                Op::RightParenthesis,
                Op::Float,
                Op::HostMath
            ]
        );
    }

    #[test]
    fn the_whole_name_must_match_a_declaration() {
        // A declared name with a standard function as its prefix is the host function; the standard
        // one stays.
        assert_eq!(
            host_tokens("math.sinh")[0],
            (Op::HostMath, host("math.sinh"), (0, 9))
        );
        assert_eq!(host_tokens("math.sin")[0], (Op::Sin, Payload::None, (0, 8)));
        // A name that only starts with a declared one lexes as before.
        let longer = run_with("math.twicex", &math_opts());
        assert_eq!((longer.tokens, longer.log), (None, run("math.twicex").log));
        assert_eq!(
            host_tokens("math.twice.x")
                .iter()
                .map(|t| t.0)
                .collect::<Vec<_>>(),
            [Op::HostMath, Op::MemberAccessor]
        );
    }

    #[test]
    fn without_a_catalogue_a_declared_name_is_unknown() {
        for src in ["math.twice(1)", "math.sinh(1)", "math.noise(1)"] {
            let with = run_with(src, &opts());
            assert_eq!(with.log.len(), run(src).log.len(), "{src}");
            assert!(
                with.tokens
                    .unwrap_or_default()
                    .iter()
                    .all(|t| !matches!(t.0, Op::HostMath | Op::HostMathVolatile)),
                "{src}"
            );
        }
    }

    #[test]
    fn token_table_holds_every_operator_once_under_its_first_byte() {
        let table = token_table();
        let mut total = 0;
        for &op in Op::all() {
            let Some(token) = op.token().filter(|t| !t.is_empty()) else {
                continue;
            };
            let bucket = &table[usize::from(token.as_bytes()[0])];
            let hits = bucket
                .iter()
                .filter(|&&(o, t)| o == op && t == token.as_bytes())
                .count();
            assert_eq!(hits, 1, "{op:?} {token:?}");
            total += 1;
        }
        assert_eq!(table.iter().map(Vec::len).sum::<usize>(), total);
    }

    #[test]
    fn token_table_buckets_of_bytes_that_start_no_operator_are_empty() {
        for c in [
            b'$', b'#', b'"', b'~', b'%', b'^', b'@', b'`', b'\\', b'0', b'9', b'A', 0, 0xff,
        ] {
            assert!(token_table()[usize::from(c)].is_empty(), "byte {c:#x}");
        }
    }

    #[test]
    fn token_table_less_than_bucket_has_both_spellings() {
        let mut tokens: Vec<_> = token_table()[usize::from(b'<')]
            .iter()
            .map(|&(op, t)| (op, t))
            .collect();
        tokens.sort_unstable();
        assert_eq!(
            tokens,
            [
                (Op::LessThan, b"<".as_slice()),
                (Op::LessEqual, b"<=".as_slice())
            ]
        );
    }

    #[test]
    fn token_table_buckets_are_in_byte_order_without_two_equal_tokens() {
        for bucket in token_table() {
            assert!(
                bucket.windows(2).all(|pair| pair[0].1 < pair[1].1),
                "{bucket:?}"
            );
        }
    }

    /// The longest token of `bucket` that `rest` starts with, by trying every token.
    fn longest_by_scan(bucket: &[(Op, &'static [u8])], rest: &[u8]) -> Option<(Op, usize)> {
        bucket
            .iter()
            .filter(|&&(_, token)| rest.starts_with(token))
            .max_by_key(|&&(_, token)| token.len())
            .map(|&(op, token)| (op, token.len()))
    }

    #[test]
    fn longest_prefix_finds_the_longest_token_the_text_starts_with() {
        let math = &token_table()[usize::from(b'm')];
        for (rest, expected) in [
            ("math.random(0, 1)", Some((Op::Random, 11))),
            ("math.random_integer(0, 1)", Some((Op::RandomInt, 19))),
            ("math.random_integerx", Some((Op::RandomInt, 19))),
            ("math.sinx", Some((Op::Sin, 8))),
            ("math.sin", Some((Op::Sin, 8))),
            ("math.si", None),
            ("math.", None),
            ("mzz", None),
            ("", None),
        ] {
            assert_eq!(longest_prefix(math, rest.as_bytes()), expected, "{rest:?}");
        }
        let less = &token_table()[usize::from(b'<')];
        assert_eq!(longest_prefix(less, b"<=1"), Some((Op::LessEqual, 2)));
        assert_eq!(longest_prefix(less, b"<1"), Some((Op::LessThan, 1)));
        assert_eq!(longest_prefix(&[], b"<1"), None);
    }

    #[test]
    fn longest_prefix_agrees_with_trying_every_token() {
        let mut texts: Vec<Vec<u8>> = Vec::new();
        for &op in Op::all() {
            let Some(token) = op.token().filter(|t| !t.is_empty()) else {
                continue;
            };
            for cut in 0..=token.len() {
                for tail in ["", "(", "x", "_integer", " ", "z", "\0", "."] {
                    texts.push([&token.as_bytes()[..cut], tail.as_bytes()].concat());
                }
            }
        }
        for text in &texts {
            let Some(&first) = text.first() else { continue };
            let bucket = &token_table()[usize::from(first)];
            assert_eq!(
                longest_prefix(bucket, text),
                longest_by_scan(bucket, text),
                "{:?}",
                String::from_utf8_lossy(text)
            );
        }
    }

    #[test]
    fn prefixed_joins_without_spare_capacity() {
        let text = prefixed("variable.", "speed");
        assert_eq!(text, "variable.speed");
        assert_eq!(text.capacity(), text.len());
        assert_eq!(prefixed("", ""), "");
    }

    #[test]
    fn token_table_is_built_once() {
        assert!(std::ptr::eq(token_table(), token_table()));
    }

    #[test]
    fn scan_name_reads_letters_digits_and_underscores() {
        let opts = client();
        let mut cx = Cx::for_test("", &opts);
        let lexer = Lexer {
            cx: &mut cx,
            text: b"ab_9.c d",
            pos: 0,
        };
        assert_eq!(lexer.scan_name(0, NameChars::Plain), Some(4));
        assert_eq!(lexer.scan_name(0, NameChars::Dotted), Some(6));
        assert_eq!(lexer.scan_name(1, NameChars::Plain), Some(4));
        assert_eq!(
            lexer.scan_name(4, NameChars::Plain),
            None,
            "a dot starts no name"
        );
        assert_eq!(lexer.scan_name(7, NameChars::Plain), Some(8));
        assert_eq!(
            lexer.scan_name(8, NameChars::Plain),
            None,
            "nothing at the end"
        );
    }

    #[test]
    fn scan_name_needs_a_letter_or_underscore_to_start() {
        let opts = client();
        let mut cx = Cx::for_test("", &opts);
        let lexer = Lexer {
            cx: &mut cx,
            text: b"1a _a",
            pos: 0,
        };
        assert_eq!(lexer.scan_name(0, NameChars::Plain), None);
        assert_eq!(lexer.scan_name(3, NameChars::Plain), Some(5));
    }

    #[test]
    fn only_space_tab_cr_lf_separate_tokens() {
        assert_eq!(ops("1 \t\r\n2"), [Op::Float, Op::Float]);
        assert_eq!(ops("  \n 1 \t "), [Op::Float]);
        assert_eq!(
            failure("1\u{b}2"),
            [("E02", "unrecognized token: \u{b}2".to_owned())]
        );
        assert_eq!(
            failure("1\u{c}"),
            [("E02", "unrecognized token: \u{c}".to_owned())]
        );
        assert_eq!(failure("\u{a0}1")[0].0, "E02");
    }

    #[test]
    fn spans_do_not_include_the_whitespace_between_tokens() {
        let t = toks("  1\t+  22 ");
        assert_eq!(
            t.iter().map(|t| t.2).collect::<Vec<_>>(),
            [(2, 3), (4, 5), (7, 9)]
        );
    }

    #[test]
    fn the_first_nul_ends_the_expression() {
        let t = toks("1 + 2\0 + 3 $");
        assert_eq!(t.len(), 3);
        let t = toks("1\0");
        assert_eq!(t.len(), 1);
    }

    #[test]
    fn empty_input_is_an_error_from_version_4() {
        for src in ["", "   ", "\t\r\n", "\0 1"] {
            let r = run_with(src, &CompileOptions::server(MolangVersion::LATEST));
            assert!(r.tokens.is_none(), "{src:?}");
            assert_eq!(r.log.len(), 1);
            assert_eq!(r.log[0].0, "E01");
            assert_eq!(r.log[0].1, Severity::Error);
            assert_eq!(r.log[0].3, "No tokens found in expression");
            assert_eq!(
                r.log[0].2,
                (0, lower(src).len() as u32),
                "the span covers the text up to the first NUL"
            );
        }
    }

    #[test]
    fn empty_input_below_version_4_is_rejected_without_a_language_message() {
        for raw in [-1, 0, 3] {
            let r = run_with(
                "  ",
                &CompileOptions::from_raw_version(
                    crate::stdlib::queries(Side::Server).clone(),
                    RawVersion(raw),
                ),
            );
            assert!(r.tokens.is_none(), "version {raw}");
            assert_eq!(r.log.len(), 1, "version {raw}");
            assert_eq!(r.log[0].0, "-");
            assert_eq!(r.log[0].1, Severity::Info);
            assert_eq!(r.log[0].2, (0, 2));
            assert_eq!(
                r.log[0].3,
                "empty expression: rejected without a message below MolangVersion 4, evaluates to 0"
            );
        }
        let r = run_with(
            "  ",
            &CompileOptions::from_raw_version(
                crate::stdlib::queries(Side::Server).clone(),
                RawVersion(4),
            ),
        );
        assert_eq!(r.log[0].0, "E01");
    }

    #[test]
    fn every_plain_operator_lexes_to_its_op_with_no_payload() {
        for (src, op) in [
            ("{", Op::LeftBrace),
            ("}", Op::RightBrace),
            ("[", Op::LeftBracket),
            ("]", Op::RightBracket),
            ("(", Op::LeftParenthesis),
            (")", Op::RightParenthesis),
            ("-", Op::Negate),
            ("!", Op::LogicalNot),
            ("+", Op::Add),
            ("/", Op::Div),
            ("*", Op::Mul),
            ("<", Op::LessThan),
            ("<=", Op::LessEqual),
            (">=", Op::GreaterEqual),
            (">", Op::GreaterThan),
            ("==", Op::LogicalEqual),
            ("!=", Op::LogicalNotEqual),
            ("||", Op::LogicalOr),
            ("&&", Op::LogicalAnd),
            ("??", Op::NullCoalescing),
            ("?", Op::Conditional),
            (":", Op::ConditionalElse),
            ("=", Op::Assignment),
            ("->", Op::Pointer),
            (";", Op::Semicolon),
            (",", Op::Comma),
            ("loop", Op::Loop),
            ("for_each", Op::ForEach),
            ("break", Op::Break),
            ("continue", Op::Continue),
            ("return", Op::Return),
            ("this", Op::This),
            ("math.pi", Op::Pi),
            ("math.abs", Op::Abs),
        ] {
            assert_eq!(
                toks(src),
                [(op, Payload::None, (0, src.len() as u32))],
                "{src:?}"
            );
        }
    }

    #[test]
    fn the_longest_operator_wins() {
        assert_eq!(ops("<="), [Op::LessEqual]);
        assert_eq!(ops("< ="), [Op::LessThan, Op::Assignment]);
        assert_eq!(ops("!="), [Op::LogicalNotEqual]);
        assert_eq!(ops("! ="), [Op::LogicalNot, Op::Assignment]);
        assert_eq!(ops("->"), [Op::Pointer]);
        assert_eq!(ops("- >"), [Op::Negate, Op::GreaterThan]);
        assert_eq!(ops("==="), [Op::LogicalEqual, Op::Assignment]);
        assert_eq!(ops("???"), [Op::NullCoalescing, Op::Conditional]);
        assert_eq!(ops("math.atan2"), [Op::Atan2]);
        assert_eq!(ops("math.die_roll_integer"), [Op::DieRollInt]);
        assert_eq!(ops("math.die_roll"), [Op::DieRoll]);
    }

    #[test]
    fn operators_match_by_prefix_with_no_word_boundary() {
        let r = run("loopy");
        assert!(r.tokens.is_none());
        assert_eq!(
            r.log
                .iter()
                .map(|l| (l.0, l.3.as_str(), l.2))
                .collect::<Vec<_>>(),
            [
                ("E03", "Error: unknown token: y", (4, 5)),
                ("E02", "unrecognized token: y", (4, 5))
            ]
        );
        assert_eq!(
            ops("loop(1,{})"),
            [
                Op::Loop,
                Op::LeftParenthesis,
                Op::Float,
                Op::Comma,
                Op::LeftBrace,
                Op::RightBrace,
                Op::RightParenthesis
            ]
        );
        assert_eq!(
            failure("thisx")[0],
            ("E03", "Error: unknown token: x".to_owned())
        );
        assert_eq!(
            failure("math.pix")[0],
            ("E03", "Error: unknown token: x".to_owned())
        );
        assert_eq!(failure("breakx")[0].0, "E03");
        assert_eq!(failure("returns")[0].1, "Error: unknown token: s");
    }

    #[test]
    fn a_prefix_match_that_is_a_complete_token_keeps_lexing() {
        assert_eq!(ops("this1"), [Op::This, Op::Float]);
        assert_eq!(ops("return1"), [Op::Return, Op::Float]);
        assert_eq!(ops("math.pi1"), [Op::Pi, Op::Float]);
        assert_eq!(ops("loop1"), [Op::Loop, Op::Float]);
    }

    #[test]
    fn operators_are_lowered_before_matching() {
        assert_eq!(
            ops("LOOP RETURN THIS BREAK"),
            [Op::Loop, Op::Return, Op::This, Op::Break]
        );
        assert_eq!(ops("Math.Pi"), [Op::Pi]);
    }

    #[test]
    fn the_used_set_holds_the_ops_of_the_tokens() {
        let r = run("1 + v.x * q.is_baby");
        assert_eq!(
            r.used,
            [
                Op::Add,
                Op::Mul,
                Op::QueryFunction,
                Op::EntityVariable,
                Op::Float
            ]
        );
        assert_eq!(run("v.x").used, [Op::EntityVariable]);
        assert_eq!(run("v.x.y").used, [Op::EntityVariable, Op::MemberAccessor]);
        assert!(run("(").used.contains(&Op::LeftParenthesis));
    }

    #[test]
    fn a_token_that_fails_is_not_in_the_used_set() {
        let r = run("1 + y");
        assert!(r.tokens.is_none());
        assert!(r.used.is_empty());
    }

    #[test]
    fn short_aliases_make_the_variable_tokens() {
        assert_eq!(
            toks("v.x"),
            [(
                Op::EntityVariable,
                Payload::Entity(name("variable.x")),
                (0, 3)
            )]
        );
        assert_eq!(
            toks("t.i"),
            [(Op::TempVariable, Payload::Temp(name("temp.i")), (0, 3))]
        );
        assert_eq!(
            toks("c.x"),
            [(
                Op::ContextVariable,
                Payload::Context(name("context.x")),
                (0, 3)
            )]
        );
    }

    #[test]
    fn long_spellings_make_the_same_tokens_with_longer_spans() {
        assert_eq!(
            toks("variable.x"),
            [(
                Op::EntityVariable,
                Payload::Entity(name("variable.x")),
                (0, 10)
            )]
        );
        assert_eq!(
            toks("temp.i"),
            [(Op::TempVariable, Payload::Temp(name("temp.i")), (0, 6))]
        );
        assert_eq!(
            toks("context.other"),
            [(
                Op::ContextVariable,
                Payload::Context(name("context.other")),
                (0, 13)
            )]
        );
        assert_eq!(
            toks("array.skins"),
            [(
                Op::ArrayVariable,
                Payload::ArrayVariable(name("array.skins")),
                (0, 11)
            )]
        );
    }

    #[test]
    fn an_alias_and_its_long_spelling_have_the_same_canonical_name() {
        assert_eq!(toks("v.speed")[0].1, toks("variable.speed")[0].1);
        assert_eq!(toks("t.a")[0].1, toks("temp.a")[0].1);
        assert_eq!(toks("c.a")[0].1, toks("context.a")[0].1);
    }

    #[test]
    fn names_are_lowered_and_may_hold_digits_and_underscores() {
        assert_eq!(
            toks("V.My_Var_2"),
            [(
                Op::EntityVariable,
                Payload::Entity(name("variable.my_var_2")),
                (0, 10)
            )]
        );
        assert_eq!(
            toks("v._x"),
            [(
                Op::EntityVariable,
                Payload::Entity(name("variable._x")),
                (0, 4)
            )]
        );
    }

    #[test]
    fn a_variable_name_stops_at_the_first_dot() {
        let t = toks("v.x.y");
        assert_eq!(t.len(), 2);
        assert_eq!(
            t[0],
            (
                Op::EntityVariable,
                Payload::Entity(name("variable.x")),
                (0, 3)
            )
        );
        assert_eq!(
            t[1],
            (Op::MemberAccessor, Payload::Member(name("y")), (3, 5))
        );
    }

    #[test]
    fn member_accessors_lex_to_the_member_name_without_its_dot() {
        assert_eq!(
            ops("v.a.b.c"),
            [Op::EntityVariable, Op::MemberAccessor, Op::MemberAccessor]
        );
        let t = toks("v.a.bc");
        assert_eq!(
            t[1],
            (Op::MemberAccessor, Payload::Member(name("bc")), (3, 6))
        );
        assert_eq!(
            toks(".x"),
            [(Op::MemberAccessor, Payload::Member(name("x")), (0, 2))]
        );
    }

    #[test]
    fn a_member_accessor_may_follow_any_token_in_the_flat_list() {
        // `1 .x` is two tokens; attaching is the parser's business.
        assert_eq!(ops("1 .x"), [Op::Float, Op::MemberAccessor]);
        assert_eq!(
            ops("(v.x).y"),
            [
                Op::LeftParenthesis,
                Op::EntityVariable,
                Op::RightParenthesis,
                Op::MemberAccessor
            ]
        );
    }

    #[test]
    fn geometry_material_and_texture_names_may_hold_dots() {
        assert_eq!(
            toks("geometry.a.b"),
            [(
                Op::GeometryVariable,
                Payload::Geometry(name("geometry.a.b")),
                (0, 12)
            )]
        );
        assert_eq!(
            toks("material.m"),
            [(
                Op::MaterialVariable,
                Payload::Material(name("material.m")),
                (0, 10)
            )]
        );
        assert_eq!(
            toks("texture.t.x_y"),
            [(
                Op::TextureVariable,
                Payload::Texture(name("texture.t.x_y")),
                (0, 13)
            )]
        );
    }

    #[test]
    fn a_resource_prefix_without_a_name_is_read_as_a_literal() {
        // The literal scan finds the identifier before the dot, which is no `true` / `false`.
        for src in ["geometry.", "material.1", "texture.+1"] {
            assert_eq!(
                failure(src),
                [
                    ("E03", format!("Error: unknown token: {src}")),
                    ("E02", format!("unrecognized token: {src}"))
                ],
                "{src}"
            );
        }
    }

    #[test]
    fn resource_variables_are_lowered_whole() {
        assert_eq!(
            toks("Geometry.Default"),
            [(
                Op::GeometryVariable,
                Payload::Geometry(name("geometry.default")),
                (0, 16)
            )]
        );
    }

    #[test]
    fn the_query_alias_is_lexed_like_the_long_prefix() {
        let short = toks("q.is_baby");
        let long = toks("query.is_baby");
        assert_eq!(short[0].0, Op::QueryFunction);
        assert_eq!(short[0].1, long[0].1);
        assert_eq!(short[0].2, (0, 9));
        assert_eq!(long[0].2, (0, 13));
    }

    #[test]
    fn the_aliases_are_two_characters_and_need_the_dot() {
        assert_eq!(
            failure("m.floor(1)"),
            [
                ("E03", "Error: unknown token: m.floor(1)".to_owned()),
                ("E02", "unrecognized token: m.floor(1)".to_owned())
            ]
        );
        assert_eq!(
            failure("ve.x")[0],
            ("E03", "Error: unknown token: ve.x".to_owned())
        );
        assert_eq!(
            failure("v")[0],
            ("E03", "Error: unknown token: v".to_owned())
        );
        // `t.x` is a temp variable, not the boolean `t`.
        assert_eq!(ops("t.x"), [Op::TempVariable]);
    }

    #[test]
    fn a_namespace_without_a_name_is_an_unknown_token() {
        assert_eq!(
            failure("variable."),
            [
                ("E03", "Error: unknown token: variable.".to_owned()),
                ("E02", "unrecognized token: variable.".to_owned())
            ]
        );
        assert_eq!(
            failure("v.1x")[0],
            ("E03", "Error: unknown token: v.1x".to_owned())
        );
        assert_eq!(failure("temp.")[0].0, "E03");
        assert_eq!(failure("c. x")[0].1, "Error: unknown token: c. x");
    }

    #[test]
    fn a_dot_alone_is_unrecognized_but_a_dot_with_digits_is_a_number() {
        assert_eq!(failure("."), [("E02", "unrecognized token: .".to_owned())]);
        assert_eq!(
            failure(". 1"),
            [("E02", "unrecognized token: . 1".to_owned())]
        );
        assert_eq!(float_bits(".5"), f(0.5));
    }

    #[test]
    fn a_query_prefix_without_a_name_fails_with_only_the_generic_message() {
        for src in ["q.", "query.", "q.1"] {
            let r = run(src);
            assert!(r.tokens.is_none());
            assert_eq!(r.log.len(), 1, "{src:?}: {:?}", r.log);
            assert_eq!(r.log[0].0, "E02");
            assert_eq!(r.log[0].3, format!("unrecognized token: {src}"));
        }
    }

    #[test]
    fn characters_that_start_no_token_fail_with_the_rest_of_the_input() {
        for c in ["#", "$", "%", "^", "&", "|", "@", "~", "`", "\\", "\""] {
            let src = format!("1 {c} rest");
            let r = run(&src);
            assert!(r.tokens.is_none(), "{src:?}");
            assert_eq!(r.log.len(), 1, "{src:?}: {:?}", r.log);
            assert_eq!(
                r.log[0],
                (
                    "E02",
                    Severity::Error,
                    (2, src.len() as u32),
                    format!("unrecognized token: {c} rest")
                ),
                "{src:?}"
            );
        }
    }

    #[test]
    fn a_comment_marker_is_not_special() {
        assert_eq!(
            failure("1 # comment"),
            [("E02", "unrecognized token: # comment".to_owned())]
        );
        let r = failure("1 // comment");
        assert_eq!(r[0], ("E03", "Error: unknown token: comment".to_owned()));
        assert_eq!(r[1], ("E02", "unrecognized token: comment".to_owned()));
    }

    #[test]
    fn a_non_ascii_character_is_unrecognized_and_spans_count_bytes() {
        let r = run("\u{e9} + v.x");
        assert!(r.tokens.is_none());
        assert_eq!(r.log[0].0, "E02");
        assert_eq!(r.log[0].2, (0, 8));
        assert_eq!(r.log[0].3, "unrecognized token: \u{e9} + v.x");
    }

    #[test]
    fn spans_after_a_multi_byte_string_count_bytes() {
        let t = toks("'\u{e9}' + 1");
        assert_eq!(t[0].2, (0, 4));
        assert_eq!(t[1].2, (5, 6));
        assert_eq!(t[2].2, (7, 8));
    }

    #[test]
    fn a_failure_reports_the_rest_of_the_input_from_the_failing_token() {
        let r = run("1 + v.x + $abc");
        assert_eq!(r.log[0].2, (10, 14));
        assert_eq!(r.log[0].3, "unrecognized token: $abc");
    }

    #[test]
    fn a_failure_quotes_the_lowered_text() {
        let r = run("1 + $ABC");
        assert_eq!(r.log[0].3, "unrecognized token: $abc");
    }

    #[test]
    fn a_string_protects_the_quoted_text_from_lowering_in_a_failure() {
        let r = run("$'ABC'");
        assert_eq!(r.log[0].3, "unrecognized token: $'ABC'");
    }

    #[test]
    fn a_full_expression_has_a_span_per_token() {
        let t = toks("V.X + Q.Is_Baby * 2.5");
        let spans: Vec<_> = t.iter().map(|t| t.2).collect();
        assert_eq!(spans, [(0, 3), (4, 5), (6, 15), (16, 17), (18, 21)]);
        let ops: Vec<_> = t.iter().map(|t| t.0).collect();
        assert_eq!(
            ops,
            [
                Op::EntityVariable,
                Op::Add,
                Op::QueryFunction,
                Op::Mul,
                Op::Float
            ]
        );
    }

    proptest! {
        #![proptest_config(config())]

        #[test]
        fn lexing_arbitrary_ascii_never_panics_and_spans_stay_in_the_source(src in ascii_source()) {
            let r = run(&src);
            for entry in &r.log {
                prop_assert!(entry.2.0 <= entry.2.1 && entry.2.1 as usize <= src.len(), "{:?}", entry);
            }
            if let Some(tokens) = r.tokens {
                let mut previous_end = 0;
                for (_, _, span) in tokens {
                    prop_assert!(span.0 >= previous_end, "tokens overlap in {:?}", src);
                    prop_assert!(span.0 < span.1 && span.1 as usize <= src.len());
                    previous_end = span.1;
                }
            }
        }
    }

    mod tree_shapes {
        use crate::compile::{
            CompileOptions, compile,
            test_support::pipeline::{tree, tree_at},
        };
        use crate::hash::HashedStr;
        use crate::version::MolangVersion;

        #[test]
        fn whitespace() {
            assert_eq!(tree("v.x\t+\n1\r"), "[v.x*1+1]");
            assert_eq!(tree("v.x+1"), "[v.x*1+1]");
            assert_eq!(tree("loop (3, {v.x = 1;});"), tree("loop(3, {v.x = 1;});"));
        }

        #[test]
        fn operators_are_prefix_matched() {
            assert_eq!(tree("v.a<=v.b"), "(LessEqual v.a v.b)");
            assert_eq!(tree("v.a->v.b"), "(Pointer v.a v.b)");
            assert_eq!(tree("v.a??1"), "(NullCoalescing v.a 1)");
        }

        #[test]
        fn aliases() {
            assert_eq!(tree("v.x"), tree("variable.x"));
            assert_eq!(tree("t.x"), tree("temp.x"));
            assert_eq!(tree("c.x"), tree("context.x"));
            assert_eq!(tree("v.x"), "v.x");
            assert_eq!(tree("t.x"), "t.x");
            assert_eq!(tree("c.x"), "c.x");
            let opts = CompileOptions::client(MolangVersion::LATEST);
            assert_eq!(
                compile("q.is_baby", &opts).tree_notation(9),
                compile("query.is_baby", &opts).tree_notation(9)
            );
        }

        #[test]
        fn names() {
            assert_eq!(tree("geometry.example.name"), "geometry.example.name");
            assert_eq!(tree("material.default"), "material.default");
            assert_eq!(tree("texture.a.b.c"), "texture.a.b.c");
            assert_eq!(tree("v.x.y.z"), "(MemberAccessor (MemberAccessor v.x))");
            assert_eq!(tree("v._x"), "v._x");
        }

        #[test]
        fn spans_index_the_original_text() {
            assert_eq!(tree_at("V.X", 0), "v.x");
        }

        #[test]
        fn minus_is_never_part_of_a_number() {
            assert_eq!(tree("v.x -1"), "[v.x*1+-1]");
        }

        #[test]
        fn string_hashes_of_raw_bytes() {
            assert_eq!(tree("'a\\'b'"), "3327885792095995213");
            assert_eq!(tree("'a\\\\b'"), "3327983648630905864");
        }

        #[test]
        fn each_opener_has_its_closer() {
            assert_eq!(tree("(1)"), "1");
            assert_eq!(tree("[1]"), "1");
            assert_eq!(
                tree("{v.x = 1;};"),
                "(Semicolon (Semicolon (Assignment v.x)))"
            );
        }

        #[test]
        fn member_accessors_carry_their_name() {
            assert_eq!(tree("v.x.y + v.x.y"), "[(MemberAccessor v.x)*2+0]");
            assert_eq!(
                tree("v.x.y + v.x.z"),
                "(Add (MemberAccessor v.x) (MemberAccessor v.x))"
            );
            assert_eq!(
                tree("v.x.Y + v.x.y"),
                "[(MemberAccessor v.x)*2+0]",
                "the name is lowered with the rest"
            );
        }

        #[test]
        fn literal_values_are_fnv1_hashes() {
            for (literal, hash) in [
                ("'a'", 12_638_153_115_695_167_422_u64),
                ("'abc'", 15_626_587_013_303_479_755),
                ("'ABC'", 15_595_941_425_208_037_995),
                ("' '", 12_638_153_115_695_167_487),
            ] {
                let inner = &literal[1..literal.len() - 1];
                assert_eq!(HashedStr::new(inner).as_u64(), hash, "{literal}");
                assert_eq!(tree(literal), hash.to_string(), "{literal}");
            }
        }

        /// Strings keep their case; names and keywords do not.
        #[test]
        fn pitfall_lowering_inside_strings() {
            assert_ne!(tree("'ABC'"), tree("'abc'"));
            assert_eq!(tree("'abc'"), "15626587013303479755");
            assert_eq!(
                tree("MATH.ABS(V.A) + Q.IS_BABY"),
                tree("math.abs(v.a) + q.is_baby")
            );
        }

        #[test]
        fn namespace_aliases_parse_alike() {
            for (long, short) in [
                ("variable.x", "v.x"),
                ("temp.x", "t.x"),
                ("context.x", "c.x"),
            ] {
                assert_eq!(tree(long), tree(short), "{long}");
            }
            assert_eq!(tree("V.X"), "v.x");
            assert_eq!(tree("Variable.X"), "v.x");
        }
    }
}
