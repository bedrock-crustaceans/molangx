//! The compile entry point and its pipeline: `lex` (lower-case and tokenize), `parse` (group into a
//! tree), `sema` (optimise and validate) and `codegen` (link into bytecode).
//!
//! [`compile`] never panics. A rejected compile ([`CompileOutcome::Rejected`]) carries at least one
//! diagnostic; [`CompileOutcome::UsesArrays`] and [`CompileOutcome::UsesResources`] carry none of
//! their own.

use crate::diag::{DiagCode, LanguageMessage, Severity};
use crate::json::MolangSource;
use crate::ops::{ExpressionOp, OpSet};
use crate::version::RawVersion;
use ast::{Friendly, Node, Payload};
use codegen::LinkError;
use std::sync::Arc;

pub(crate) mod ast;
mod codegen;
mod cx;
mod lex;
mod options;
mod parse;
pub(crate) mod program;
mod result;
mod sema;

use cx::{Cx, Failed, Pass};
pub use options::{CompileOptions, Deviations};
pub use program::ProgramFlags;
#[cfg(feature = "vm")]
pub(crate) use result::Evaluation;
pub use result::{CompileError, CompileFailure, CompileOutcome, Compiled, Expr};

/// Maximum nesting depth of sub-expressions the parser accepts.
///
/// Depth 256 fails with `Error: Expression could not be parsed due to stack depth overflow (too
/// many sub-expressions)`. The parser counts depth explicitly; the later tree walks recurse
/// natively and, bounded by this depth, fit in a spawned thread's default 2 MiB stack.
pub const MAX_DEPTH: u32 = 255;

/// Maximum length, in bytes, of a source text `compile` accepts
/// (`Deviations::source_length_limit`).
///
/// Every compile stage is linear in the source, so this bounds one compile's work and memory. With
/// `Deviations::NONE` no length is rejected.
pub const MAX_SOURCE_LEN: usize = 65_536;

/// Maximum number of diagnostics one compile keeps (`Deviations::diagnostic_limit`); past it, one
/// informational note counts the diagnostics left out.
pub const MAX_DIAGNOSTICS: usize = 256;

/// Compiles `src` with `opts`.
///
/// A **rejected** expression (a lexer, tree-building or optimiser error, a root-level validation
/// error, or a link failure) is the constant 0 with the error attached; a **kept** expression
/// carries its language messages as warnings (as errors with `validate_nested` off).
///
/// Compiles at [`CompileOptions::version`]; [`compile_source`] compiles a [`MolangSource`] at its
/// own version. Queries resolve against [`CompileOptions::catalog`].
///
/// # Stack
///
/// Work is linear in the source. Passes over unbounded input use explicit stacks; walks of an
/// accepted tree recurse, bounded by [`MAX_DEPTH`]. On x86-64 the deepest
/// accepted nesting (250 nested `q.is_baby(` calls) needs at most 128 KiB of stack optimised and
/// 384 KiB unoptimised.
#[must_use = "the result carries the expression and its diagnostics"]
pub fn compile(src: &str, opts: &CompileOptions) -> Compiled {
    compile_at(src, opts, Some(opts.raw_version))
}

/// `src` under `opts` at `version` instead of the options' raw version; `None` is a string whose
/// context version was never applied (rules of `Invalid`, no query resolves).
pub(crate) fn compile_at(
    src: &str,
    opts: &CompileOptions,
    version: Option<RawVersion>,
) -> Compiled {
    let mut cx = Cx::new(src, opts, version);
    let raw_version = cx.raw_version();
    let source: Option<Arc<str>> = opts.keep_source.then(|| Arc::from(src));
    cx.lint_version();

    let Some(tree) = parse_source(&mut cx, src) else {
        let rejected = Expr::rejected(opts, raw_version, source);
        return Compiled::done(cx, CompileOutcome::Rejected(rejected), None);
    };
    let tree = Arc::new(tree);
    let outcome = match codegen::build(&mut cx, &tree) {
        Ok(program) => {
            CompileOutcome::Success(Expr::new(Arc::clone(&tree), program, raw_version, source))
        }
        Err(LinkError::UsesArrays) => CompileOutcome::UsesArrays,
        Err(LinkError::UsesResources) => CompileOutcome::UsesResources,
        Err(LinkError::Failed) => {
            // The message quotes the text up to its first NUL byte.
            let end = src.find('\0').unwrap_or(src.len());
            let quoted = source.clone().unwrap_or_else(|| Arc::from(&src[..end]));
            cx.quote(LanguageMessage::CompileFailed, cx.whole(), quoted, 0..end);
            CompileOutcome::Rejected(Expr::rejected(opts, raw_version, source))
        }
    };
    Compiled::done(cx, outcome, Some(tree))
}

/// Compiles a [`MolangSource`] at **its** version with the rest of `field_opts`.
///
/// The options' version is replaced by the source's ([`CompileOptions::raw_version`]), so one
/// options value per field is enough:
///
/// ```
/// # #[cfg(feature = "stdlib")]
/// # {
/// use molangx::compile::{CompileFailure, CompileOptions, compile_source};
/// use molangx::json::MolangSource;
/// use molangx::version::MolangVersion;
///
/// // A field's options, version-free in effect.
/// let field = CompileOptions::server(MolangVersion::LATEST);
/// // Version 2 predates the string-arithmetic error of version 3.
/// let old = MolangSource::string("'a' + 1", 2);
/// assert!(compile_source(&old, &field).is_success());
/// let new = MolangSource::string("'a' + 1", 13);
/// assert_eq!(compile_source(&new, &field).failure(), Some(CompileFailure::Rejected));
/// # }
/// ```
///
/// To compile at another version, call [`compile`] with `src.as_str()`.
///
/// A string source whose context version was never applied
/// ([`MolangSource::string_without_context`]) compiles with a warning, under the rules of
/// [`MolangVersion::Invalid`], and resolves no query.
///
/// [`MolangVersion::Invalid`]: crate::version::MolangVersion::Invalid
#[must_use = "the result carries the expression and its diagnostics"]
pub fn compile_source(src: &MolangSource, field_opts: &CompileOptions) -> Compiled {
    compile_at(src.as_str(), field_opts, src.raw_version())
}

/// The optimised and validated tree of `src`, or `None` once a rejection is logged.
fn parse_source(cx: &mut Cx<'_>, src: &str) -> Option<Node> {
    if cx.opts.deviations.source_length_limit && src.len() > MAX_SOURCE_LEN {
        let text = format!(
            "expression is {} bytes long; the limit is {MAX_SOURCE_LEN}",
            src.len()
        );
        cx.lint(DiagCode::SourceTooLong, Severity::Error, cx.whole(), text);
        return None;
    }
    parse_core(cx, &lex::lower(src))
}

fn parse_core(cx: &mut Cx<'_>, text: &[u8]) -> Option<Node> {
    let tokens = lex::scan(cx, text)?;
    let used = tokens.used;
    let allowed = cx.opts.allowed_ops.contains(used);
    // Named before grouping, which consumes the tokens.
    let denied_calls = if allowed {
        Vec::new()
    } else {
        denied_host_calls(cx, &tokens.nodes)
    };
    let mut root = parse::group_tokens(cx, tokens.nodes, used)?;
    if !allowed {
        log_denied_ops(cx, used, &denied_calls);
        return None;
    }
    sema::simplify(cx, &mut root).ok()?;
    sema::validate_root(cx, &root).ok()?;
    Some(root)
}

/// The host math functions whose op the options deny, each once, in source order.
fn denied_host_calls<'o>(cx: &Cx<'o>, tokens: &[Node]) -> Vec<(ExpressionOp, Friendly<'o>)> {
    let mut seen: Vec<&Payload> = Vec::new();
    let mut calls = Vec::new();
    for node in tokens {
        if !cx.opts.allowed_ops.contains(node.op)
            && matches!(node.value, Payload::HostMath(_))
            && !seen.contains(&&node.value)
        {
            seen.push(&node.value);
            calls.push((node.op, cx.friendly(node)));
        }
    }
    calls
}

/// Logs #21 for each denied op in `used`: once per denied host math function with that op, or
/// once by the op's name when there is none.
fn log_denied_ops(cx: &mut Cx<'_>, used: OpSet, host_calls: &[(ExpressionOp, Friendly<'_>)]) {
    let allowed = cx.opts.allowed_ops;
    let span = cx.whole();
    for op in used.iter().filter(|&op| !allowed.contains(op)) {
        let mut calls = host_calls
            .iter()
            .filter(|(call_op, _)| *call_op == op)
            .peekable();
        if calls.peek().is_none() {
            cx.language(
                LanguageMessage::OperationNotAllowed,
                span,
                &[&op.friendly_name()],
            );
        }
        for (_, call) in calls {
            cx.language(LanguageMessage::OperationNotAllowed, span, &[call]);
        }
    }
}

/// The lexer's view of a source text, for the fuzz crate's printer tests.
#[cfg(feature = "fuzz")]
pub(crate) mod probe {
    use crate::compile::{CompileOptions, Cx, ast::Payload, lex};
    use crate::ops::ExpressionOp;

    /// The tokens the lexer makes of `source`, or the first message it logged when it refused.
    pub fn tokens(
        source: &str,
        options: &CompileOptions,
    ) -> Result<Vec<(ExpressionOp, Payload)>, String> {
        let mut cx = Cx::for_test(source, options);
        let text = lex::lower(source);
        match lex::scan(&mut cx, &text) {
            Some(tokens) => Ok(tokens
                .nodes
                .into_iter()
                .map(|n| (n.op, n.value.clone()))
                .collect()),
            None => Err(cx
                .logged_diagnostics()
                .first()
                .map(|d| d.message().into_owned())
                .unwrap_or_default()),
        }
    }
}

#[cfg(test)]
pub(crate) mod test_support {
    //! Helpers shared by the compile unit tests.

    use crate::catalog::{Arity, MathCatalog, MathDecl};
    use crate::compile::{CompileOptions, Compiled};
    use crate::diag::{DiagCode, Diagnostic};
    use crate::version::{MolangVersion, RawVersion};
    use std::sync::LazyLock;

    /// Default options at the latest version, server catalogue.
    pub(crate) fn opts() -> CompileOptions {
        CompileOptions::server(MolangVersion::LATEST)
    }

    /// [`opts`] at the raw version `raw`.
    pub(crate) fn opts_at(raw: i16) -> CompileOptions {
        CompileOptions {
            raw_version: RawVersion(raw),
            ..opts()
        }
    }

    pub(crate) fn codes(compiled: &Compiled) -> Vec<DiagCode> {
        compiled
            .diagnostics()
            .iter()
            .map(Diagnostic::code)
            .collect()
    }

    pub(crate) fn messages(compiled: &Compiled) -> Vec<String> {
        compiled
            .diagnostics()
            .iter()
            .map(|d| d.message().into_owned())
            .collect()
    }

    /// Host math functions for the unit tests: `math.twice` and `math.double` (both `2·x`),
    /// `math.sum` (1 to 3 arguments), `math.sinh` (`x + 0.5`), `math.tag` (`x + 0.25`) and the
    /// volatile `math.noise` (`x` plus one draw).
    pub(crate) fn host_math() -> &'static MathCatalog {
        static MATH: LazyLock<MathCatalog> = LazyLock::new(|| {
            let one = Arity::exactly(1);
            MathCatalog::new([
                MathDecl::pure("math.twice", one, |a| a[0] * 2.0).unwrap(),
                MathDecl::pure("math.double", one, |a| a[0] * 2.0).unwrap(),
                MathDecl::pure("math.sum", Arity::between(1, 3), |a| a.iter().sum()).unwrap(),
                MathDecl::pure("math.sinh", one, |a| a[0] + 0.5).unwrap(),
                MathDecl::pure("math.tag", one, |a| a[0] + 0.25).unwrap(),
                MathDecl::volatile("math.noise", one, |rng, a| a[0] + crate::rng::sample(rng))
                    .unwrap(),
            ])
            .unwrap()
        });
        &MATH
    }

    /// [`opts`] with [`host_math`].
    pub(crate) fn math_opts() -> CompileOptions {
        CompileOptions {
            math: Some(host_math().clone()),
            ..opts()
        }
    }

    /// Whole-pipeline helpers for tests that assert the optimised tree or the language messages,
    /// against the client catalogue.
    pub(crate) mod pipeline {
        use crate::catalog::Side;
        use crate::compile::{CompileFailure, CompileOptions, Compiled, Expr, compile};
        use crate::diag::LanguageMessage;
        use crate::version::RawVersion;

        /// Default options at `version`, client catalogue.
        pub(crate) fn options(version: i16) -> CompileOptions {
            CompileOptions::from_raw_version(
                crate::stdlib::queries(Side::Client).clone(),
                RawVersion(version),
            )
        }

        pub(crate) fn at(source: &str, version: i16) -> Compiled {
            compile(source, &options(version))
        }

        /// The language messages, without their trailing newlines.
        pub(crate) fn messages(compiled: &Compiled) -> Vec<String> {
            compiled
                .diagnostics()
                .iter()
                .filter(|d| d.language_message().is_some())
                .map(|d| d.message().trim_end_matches('\n').to_owned())
                .collect()
        }

        pub(crate) fn messages_at(source: &str, version: i16) -> Vec<String> {
            messages(&at(source, version))
        }

        pub(crate) fn tree_at(source: &str, version: i16) -> String {
            let compiled = at(source, version);
            compiled.tree_notation(9).unwrap_or_else(|| {
                panic!(
                    "{source:?} does not parse at version {version}: {:?}",
                    messages(&compiled)
                )
            })
        }

        pub(crate) fn tree(source: &str) -> String {
            tree_at(source, 13)
        }

        pub(crate) fn constant(source: &str) -> f32 {
            let compiled = at(source, 13);
            compiled
                .expr()
                .and_then(Expr::as_constant)
                .filter(|_| compiled.parsed())
                .unwrap_or_else(|| {
                    panic!(
                        "{source:?} is not a constant: {:?} {:?}",
                        compiled.tree_notation(9),
                        messages(&compiled)
                    )
                })
        }

        pub(crate) fn ids(compiled: &Compiled) -> Vec<&'static str> {
            compiled
                .diagnostics()
                .iter()
                .filter_map(|d| d.language_message().map(LanguageMessage::id))
                .collect()
        }

        pub(crate) fn ids_at(source: &str, version: i16) -> Vec<&'static str> {
            ids(&at(source, version))
        }

        pub(crate) fn assert_rejected(source: &str, version: i16, expected: &[&str]) {
            let compiled = at(source, version);
            assert!(
                !compiled.parsed(),
                "{source:?} v{version} should be rejected"
            );
            assert_eq!(
                compiled.failure(),
                Some(CompileFailure::Rejected),
                "{source:?} v{version}"
            );
            assert_eq!(ids(&compiled), expected, "{source:?} v{version}");
            assert_eq!(
                compiled.expr_or_zero().and_then(Expr::as_constant),
                Some(0.0),
                "{source:?}: the failed node is 0"
            );
        }

        pub(crate) fn assert_parsed(source: &str, version: i16, expected: &[&str]) {
            let compiled = at(source, version);
            assert!(
                compiled.parsed(),
                "{source:?} v{version} should parse: {:?}",
                compiled.diagnostics()
            );
            assert_eq!(ids(&compiled), expected, "{source:?} v{version}");
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::catalog::Side;
    use crate::compile::{
        CompileFailure, CompileOptions, Deviations, MAX_DEPTH, MAX_DIAGNOSTICS, MAX_SOURCE_LEN,
        compile, compile_source, test_support::*,
    };
    use crate::version::RawVersion;

    use crate::catalog::{QueryAdmission, QueryAllowList, QueryDecl, QuerySetMask, QueryShape};
    use crate::diag::{DiagCode, Diagnostic, LanguageMessage, Severity};
    use crate::ops::OpSet;
    use crate::stdlib::query;

    use crate::json::MolangSource;
    use crate::version::MolangVersion;

    #[test]
    fn the_limits_have_their_documented_values() {
        assert_eq!(MAX_DEPTH, 255);
        assert_eq!(MAX_SOURCE_LEN, 64 * 1024);
        assert_eq!(MAX_DIAGNOSTICS, 256);
    }

    #[test]
    fn the_limits_are_the_ones_the_compiler_enforces() {
        let options = CompileOptions::server(MolangVersion::LATEST);
        let deepest = format!("{}1", "!".repeat(MAX_DEPTH as usize));
        assert!(compile(&deepest, &options).parses_cleanly());
        let too_deep = format!("{}1", "!".repeat(MAX_DEPTH as usize + 1));
        assert!(!compile(&too_deep, &options).parses_cleanly());
        let longest = format!("1{}", " ".repeat(MAX_SOURCE_LEN - 1));
        assert!(compile(&longest, &options).parses_cleanly());
        let too_long = format!("1{}", " ".repeat(MAX_SOURCE_LEN));
        assert!(!compile(&too_long, &options).parses_cleanly());
        let noisy = "1e;".repeat(MAX_DIAGNOSTICS + 1);
        assert_eq!(
            compile(&noisy, &options).diagnostics().len(),
            MAX_DIAGNOSTICS + 1
        );
    }

    #[test]
    fn versions_inside_the_defined_range_log_nothing() {
        for raw in 0..=13 {
            let compiled = compile("1", &opts_at(raw));
            assert!(
                compiled.diagnostics().is_empty(),
                "version {raw}: {:?}",
                messages(&compiled)
            );
        }
    }

    #[test]
    fn the_invalid_version_informs() {
        let compiled = compile("1", &opts_at(-1));
        assert_eq!(codes(&compiled), [DiagCode::InvalidVersion]);
        assert_eq!(compiled.diagnostics()[0].severity(), Severity::Info);
        assert_eq!(
            compiled.diagnostics()[0].message(),
            "MolangVersion Invalid (-1): version-0 parser rules, no query resolves"
        );
        assert_eq!(compiled.diagnostics()[0].span(), 0..1);
        assert_eq!(compiled.failure(), None);
    }

    #[test]
    fn a_version_outside_the_range_warns_with_the_rules_it_gets() {
        for (raw, rules) in [(14, 13), (i16::MAX, 13), (-2, -1), (-300, -1)] {
            let compiled = compile("12", &opts_at(raw));
            assert_eq!(codes(&compiled), [DiagCode::InvalidVersion], "raw {raw}");
            assert_eq!(compiled.diagnostics()[0].severity(), Severity::Warning);
            assert_eq!(
                compiled.diagnostics()[0].message(),
                format!(
                    "MolangVersion {raw} is outside -1..=13: parser rules as version {rules}, no query resolves"
                )
            );
            assert_eq!(compiled.diagnostics()[0].span(), 0..2);
            assert!(compiled.diagnostics()[0].language_message().is_none());
            assert_eq!(compiled.failure(), None);
        }
    }

    #[test]
    fn the_version_warning_can_be_switched_off() {
        let off = Deviations {
            object_version_warning: false,
            ..Deviations::ALL
        };
        for raw in [-2, -1, 14] {
            let compiled = compile(
                "1",
                &CompileOptions {
                    deviations: off,
                    ..opts_at(raw)
                },
            );
            assert!(compiled.diagnostics().is_empty(), "raw {raw}");
        }
        let no_deviations = compile(
            "1",
            &CompileOptions {
                deviations: Deviations::NONE,
                ..opts_at(14)
            },
        );
        assert!(no_deviations.diagnostics().is_empty());
    }

    #[test]
    fn an_unresolved_context_version_warns_whatever_the_deviations() {
        let unresolved = MolangSource::string_without_context("1");
        for deviations in [Deviations::ALL, Deviations::NONE] {
            let compiled = compile_source(
                &unresolved,
                &CompileOptions {
                    deviations,
                    ..CompileOptions::server(MolangVersion::LATEST)
                },
            );
            assert_eq!(codes(&compiled), [DiagCode::InvalidVersion]);
            assert_eq!(compiled.diagnostics()[0].severity(), Severity::Warning);
            assert!(compiled.diagnostics()[0].message().starts_with(
                "this string source has no MolangVersion: it was made without its load context"
            ));
            assert!(
                compiled.diagnostics()[0]
                    .message()
                    .ends_with("compiled as MolangVersion Invalid, no query resolves")
            );
            assert_eq!(compiled.failure(), None);
        }
    }

    #[test]
    fn an_unresolved_version_gives_one_warning_not_two() {
        let compiled = compile("1", &opts_at(i16::MIN));
        assert_eq!(compiled.diagnostics().len(), 1);
    }

    #[test]
    fn a_source_of_exactly_the_limit_compiles() {
        let src = format!("1{}", " ".repeat(MAX_SOURCE_LEN - 1));
        assert_eq!(src.len(), MAX_SOURCE_LEN);
        let compiled = compile(&src, &opts());
        assert_eq!(compiled.failure(), None);
        assert!(compiled.diagnostics().is_empty());
        assert_eq!(
            compiled
                .expr()
                .cloned()
                .expect("an expression")
                .as_constant(),
            Some(1.0)
        );
    }

    #[test]
    fn a_source_one_byte_over_the_limit_is_rejected_without_a_parse() {
        let src = format!("1{}", " ".repeat(MAX_SOURCE_LEN));
        assert_eq!(src.len(), MAX_SOURCE_LEN + 1);
        let compiled = compile(&src, &opts());
        assert_eq!(compiled.failure(), Some(CompileFailure::Rejected));
        assert_eq!(compiled.diagnostics().len(), 1);
        let diagnostic = &compiled.diagnostics()[0];
        assert_eq!(diagnostic.code(), DiagCode::SourceTooLong);
        assert_eq!(diagnostic.severity(), Severity::Error);
        assert_eq!(diagnostic.span(), 0..MAX_SOURCE_LEN as u32 + 1);
        assert_eq!(
            diagnostic.message(),
            "expression is 65537 bytes long; the limit is 65536"
        );
        assert!(diagnostic.language_message().is_none());
        assert!(!compiled.parsed());
        assert!(!compiled.parses_cleanly());
        assert_eq!(
            compiled
                .expr_or_zero()
                .cloned()
                .expect("the failed node")
                .as_constant(),
            Some(0.0)
        );
    }

    #[test]
    fn with_the_length_deviation_off_a_long_source_compiles() {
        let src = format!("1{}", " ".repeat(MAX_SOURCE_LEN));
        let off = CompileOptions {
            deviations: Deviations {
                source_length_limit: false,
                ..Deviations::ALL
            },
            ..opts()
        };
        let compiled = compile(&src, &off);
        assert_eq!(compiled.failure(), None);
        assert!(compiled.diagnostics().is_empty());
        assert_eq!(
            compile(
                &src,
                &CompileOptions {
                    deviations: Deviations::NONE,
                    ..opts()
                }
            )
            .failure(),
            None
        );
    }

    #[test]
    fn compile_of_a_literal_succeeds_with_a_constant_expression() {
        let compiled = compile("1+2", &opts());
        assert_eq!(compiled.failure(), None);
        assert!(compiled.diagnostics().is_empty());
        assert!(compiled.parses_cleanly());
        assert!(compiled.parsed());
        let expr = compiled.expr().cloned().expect("an expression");
        assert_eq!(expr.as_constant(), Some(3.0));
    }

    #[test]
    fn a_rejected_compile_has_a_diagnostic_and_the_constant_zero() {
        for src in ["1 +", "$", "1 + 'a'", "v.x=", "math.min(1)"] {
            let compiled = compile(src, &opts());
            assert_eq!(compiled.failure(), Some(CompileFailure::Rejected), "{src}");
            assert!(!compiled.diagnostics().is_empty(), "{src}");
            assert!(!compiled.parses_cleanly(), "{src}");
            let expr = compiled.expr_or_zero().cloned().expect("the failed node");
            assert_eq!(expr.as_constant(), Some(0.0), "{src}");
            assert!(expr.is_constant());
        }
    }

    #[test]
    fn an_empty_expression_is_a_message_from_version_4_and_a_note_below() {
        let new = compile("", &opts_at(13));
        assert_eq!(new.failure(), Some(CompileFailure::Rejected));
        assert_eq!(new.diagnostics().len(), 1);
        assert_eq!(
            new.diagnostics()[0].language_message(),
            Some(LanguageMessage::NoTokens)
        );
        let old = compile("", &opts_at(3));
        assert_eq!(old.failure(), Some(CompileFailure::Rejected));
        assert_eq!(old.diagnostics().len(), 1);
        assert_eq!(old.diagnostics()[0].language_message(), None);
        assert_eq!(old.diagnostics()[0].severity(), Severity::Info);
    }

    #[test]
    fn a_disallowed_operation_is_logged_once_per_op_and_rejects() {
        let no_assignment = CompileOptions {
            allowed_ops: OpSet::all().without_assignments(),
            ..opts()
        };
        let compiled = compile("v.x=1;", &no_assignment);
        assert_eq!(compiled.failure(), Some(CompileFailure::Rejected));
        assert!(
            compiled
                .diagnostics()
                .iter()
                .all(|d| d.language_message() == Some(LanguageMessage::OperationNotAllowed))
        );
        assert!(
            messages(&compiled).contains(
                &"Expression uses operation Assignment '=' which is not allowed in this context"
                    .to_owned()
            )
        );
        assert_eq!(
            compiled
                .diagnostics()
                .iter()
                .filter(|d| d.message().contains("Assignment"))
                .count(),
            1
        );
        assert_eq!(compiled.diagnostics()[0].span(), 0..6);
    }

    #[test]
    fn disallowing_random_rejects_random_but_not_the_dice() {
        let strict = CompileOptions {
            allowed_ops: OpSet::all().without_assignments_or_random(),
            ..opts()
        };
        assert_eq!(
            compile("math.random(0,1)", &strict).failure(),
            Some(CompileFailure::Rejected)
        );
        assert_eq!(
            compile("math.random_integer(0,5)", &strict).failure(),
            Some(CompileFailure::Rejected)
        );
        assert_eq!(compile("math.die_roll(1,2,3)", &strict).failure(), None);
        assert_eq!(
            compile(
                "math.random(0,1)",
                &CompileOptions {
                    allowed_ops: OpSet::all().without_assignments(),
                    ..opts()
                }
            )
            .failure(),
            None
        );
    }

    #[test]
    fn the_allowed_query_sets_decide_what_resolves() {
        assert_eq!(compile("q.is_baby", &opts()).failure(), None);
        let none = compile(
            "q.is_baby",
            &CompileOptions {
                admission: QueryAdmission::Sets(QuerySetMask::empty()),
                ..opts()
            },
        );
        assert_eq!(none.failure(), Some(CompileFailure::Rejected));
        assert_eq!(
            none.diagnostics()
                .iter()
                .map(Diagnostic::language_message)
                .collect::<Vec<_>>(),
            [
                Some(LanguageMessage::QueryUnresolved),
                Some(LanguageMessage::UnrecognizedToken)
            ]
        );
    }

    #[test]
    fn the_allow_list_decides_what_resolves() {
        let list = QueryAllowList::new(&opts().catalog, [query::IS_BABY]).unwrap();
        let only_baby = CompileOptions {
            admission: QueryAdmission::Only(list),
            ..opts()
        };
        assert_eq!(compile("q.is_baby", &only_baby).failure(), None);
        assert_eq!(
            compile("q.is_on_ground", &only_baby).failure(),
            Some(CompileFailure::Rejected)
        );
    }

    #[test]
    fn an_allow_list_restricts_a_compile_against_any_catalogue() {
        let client = crate::stdlib::queries(Side::Client);
        let list = QueryAllowList::new(&opts().catalog, [query::BLOCK_STATE]).unwrap();
        let o = CompileOptions {
            catalog: client.clone(),
            admission: QueryAdmission::Only(list),
            ..opts()
        };
        assert_eq!(
            compile("q.is_baby", &o).failure(),
            Some(CompileFailure::Rejected)
        );
        assert_eq!(
            compile("q.is_on_screen", &o).failure(),
            Some(CompileFailure::Rejected)
        );
        assert_eq!(compile("q.block_state('facing')", &o).failure(), None);
        // A list of a name the compile's catalogue does not declare admits nothing.
        let own = client
            .extended([QueryDecl::new("query.mine", QueryShape::DEFAULT).unwrap()])
            .unwrap();
        let mine = QueryAllowList::new(&own, ["query.mine"]).unwrap();
        let o = CompileOptions {
            admission: QueryAdmission::Only(mine),
            ..opts()
        };
        for src in ["q.is_baby", "q.block_state('facing')", "q.mine"] {
            assert_eq!(
                compile(src, &o).failure(),
                Some(CompileFailure::Rejected),
                "{src}"
            );
        }
        assert_eq!(
            compile(
                "q.mine",
                &CompileOptions {
                    catalog: own.clone(),
                    ..o.clone()
                }
            )
            .failure(),
            None
        );
        assert_eq!(
            compile("q.is_baby", &CompileOptions { catalog: own, ..o }).failure(),
            Some(CompileFailure::Rejected)
        );
    }

    #[test]
    fn compile_source_applies_the_raw_version_of_the_source() {
        let field = opts();
        let old = MolangSource::string("'a' + 1", 2);
        let compiled = compile_source(&old, &field);
        assert_eq!(compiled.failure(), None);
        assert_eq!(
            compiled.expr().cloned().expect("an expression").version(),
            MolangVersion::V2
        );
        let new = MolangSource::string("'a' + 1", 13);
        assert_eq!(
            compile_source(&new, &field).failure(),
            Some(CompileFailure::Rejected)
        );
    }

    #[test]
    fn compile_source_ignores_the_version_of_the_field_options() {
        let low = opts_at(0);
        let high = opts_at(13);
        let source = MolangSource::object("'a' + 1", 13);
        assert_eq!(
            compile_source(&source, &low).failure(),
            Some(CompileFailure::Rejected)
        );
        assert_eq!(
            compile_source(&source, &high).failure(),
            Some(CompileFailure::Rejected)
        );
        let lenient = MolangSource::object("'a' + 1", 1);
        assert_eq!(compile_source(&lenient, &high).failure(), None);
        assert_eq!(
            compile_source(&lenient, &high)
                .expr()
                .cloned()
                .expect("an expression")
                .version(),
            MolangVersion::V1
        );
    }

    #[test]
    fn compile_source_keeps_the_rest_of_the_field_options() {
        let field = CompileOptions {
            catalog: crate::stdlib::queries(Side::Client).clone(),
            keep_source: true,
            ..opts()
        };
        let compiled = compile_source(&MolangSource::string("q.is_on_screen", 13), &field);
        assert_eq!(compiled.failure(), None);
        let expr = compiled.expr().cloned().expect("an expression");
        assert_eq!(expr.source(), Some("q.is_on_screen"));
    }

    #[test]
    fn compile_source_equals_compile_with_the_version_applied() {
        let source = MolangSource::string("v.x*2+1", 5);
        let direct = compile(
            "v.x*2+1",
            &CompileOptions {
                raw_version: RawVersion(5),
                ..opts()
            },
        );
        let through = compile_source(&source, &opts());
        assert_eq!(direct.failure(), through.failure());
        assert_eq!(direct.diagnostics(), through.diagnostics());
        assert_eq!(direct.tree_notation(9), through.tree_notation(9));
    }

    #[test]
    fn compile_source_warns_about_an_unresolved_version_and_not_about_a_set_one() {
        let mut source = MolangSource::string_without_context("1");
        assert_eq!(
            codes(&compile_source(&source, &opts())),
            [DiagCode::InvalidVersion]
        );
        source.set_context_version(13);
        assert!(compile_source(&source, &opts()).diagnostics().is_empty());
    }

    #[test]
    fn a_source_without_its_context_version_resolves_no_query_at_all() {
        let field = crate::reference_catalog::options(13);
        let text = "query.sum_test(1, 2)";
        let at_invalid = compile_source(&MolangSource::string(text, -1), &field);
        assert_eq!(at_invalid.failure(), None, "{:?}", at_invalid.diagnostics());
        let without = compile_source(&MolangSource::string_without_context(text), &field);
        assert_eq!(without.failure(), Some(CompileFailure::Rejected));
        assert_eq!(
            messages(&without)[..2],
            [
                "this string source has no MolangVersion: it was made without its load context (MolangSource::string_without_context) \
                 and MolangSource::set_context_version was not called; compiled as MolangVersion Invalid, no query resolves",
                "query.sum_test exists, but this source has no MolangVersion to resolve it at",
            ]
        );
        let constant = compile_source(&MolangSource::string_without_context("1"), &field)
            .expr()
            .cloned()
            .expect("an expression");
        assert_eq!(constant.version(), MolangVersion::Invalid);
    }

    #[test]
    fn a_message_that_quotes_the_source_reads_the_lower_cased_text() {
        let compiled = compile("1 + @Bad", &opts());
        assert_eq!(compiled.failure(), Some(CompileFailure::Rejected));
        assert_eq!(messages(&compiled), ["unrecognized token: @bad"]);
    }

    #[test]
    fn a_link_failure_is_rejected_with_the_compile_failed_message_quoting_the_source() {
        // `->` on the left of an assignment parses (with a warning) and then fails to link.
        let compiled = compile("v.x->v.y=1;", &opts());
        assert_eq!(compiled.failure(), Some(CompileFailure::Rejected));
        assert!(compiled.parsed(), "the tree is kept");
        let last = compiled.diagnostics().last().expect("a message");
        assert_eq!(
            last.language_message(),
            Some(LanguageMessage::CompileFailed)
        );
        assert_eq!(last.severity(), Severity::Error);
        assert_eq!(last.span(), 0..11);
        assert_eq!(last.message(), "expression 'v.x->v.y=1;' compile failed");
        assert_eq!(
            compiled
                .expr_or_zero()
                .cloned()
                .expect("the failed node")
                .as_constant(),
            Some(0.0)
        );
    }

    #[test]
    fn the_compile_failed_message_prints_the_text_up_to_a_nul_byte() {
        for preserve in [false, true] {
            let compiled = compile(
                "v.x->v.y=1;\0junk",
                &CompileOptions {
                    keep_source: preserve,
                    ..opts()
                },
            );
            let last = compiled.diagnostics().last().expect("a message");
            assert_eq!(
                last.message(),
                "expression 'v.x->v.y=1;' compile failed",
                "preserve {preserve}"
            );
            assert_eq!(last.span(), 0..16);
        }
    }

    #[test]
    fn a_source_with_a_nul_byte_ends_there() {
        let compiled = compile("1+2\0not molang $$$", &opts());
        assert_eq!(compiled.failure(), None);
        assert_eq!(
            compiled
                .expr()
                .cloned()
                .expect("an expression")
                .as_constant(),
            Some(3.0)
        );
    }
}
