//! The compile context: the diagnostics logged so far, their cap and the lints.

use crate::catalog::{MathCatalog, QueryAdmission, QueryCatalog, QueryDecl, QueryIndex, Side};
use crate::compile::{
    CompileOptions, Deviations, MAX_DIAGNOSTICS,
    ast::{Friendly, Node, Span},
};
use crate::diag::{DiagCode, Diagnostic, LanguageMessage, Severity};
use crate::ops::OpSet;
use crate::version::{ExperimentMask, MolangVersion, RawVersion};
use std::borrow::Cow;
use std::fmt;
use std::ops::Range;
use std::sync::Arc;

/// The fields of [`CompileOptions`] the passes read. Not the raw version: the compile's own is
/// [`Cx::raw_version`], which differs from the options' for a [`MolangSource`] and in the cache.
///
/// [`MolangSource`]: crate::json::MolangSource
#[derive(Copy, Clone)]
pub(super) struct Settings<'o> {
    pub(super) catalog: &'o QueryCatalog,
    pub(super) admission: &'o QueryAdmission,
    pub(super) allowed_ops: OpSet,
    pub(super) experiments: ExperimentMask,
    pub(super) math: Option<&'o MathCatalog>,
    pub(super) deviations: Deviations,
}

impl<'o> Settings<'o> {
    fn of(opts: &'o CompileOptions) -> Self {
        let CompileOptions {
            catalog,
            raw_version: _,
            admission,
            allowed_ops,
            experiments,
            math,
            keep_source: _,
            deviations,
        } = opts;
        Self {
            catalog,
            admission,
            allowed_ops: *allowed_ops,
            experiments: *experiments,
            math: math.as_ref(),
            deviations: *deviations,
        }
    }
}

/// A compile pass failed; the reason is already logged.
pub(super) struct Failed;

pub(super) type Pass = Result<(), Failed>;

pub(super) struct Cx<'o> {
    pub(super) opts: Settings<'o>,
    /// `None` for a plain-string source whose context version was never applied: no query resolves.
    pub(super) query_version: Option<RawVersion>,
    diagnostics: Vec<Diagnostic>,
    /// Diagnostics dropped by the cap.
    suppressed: usize,
    /// Whether a language message was logged, including one the cap dropped.
    logged: bool,
    /// The span of the whole source.
    whole: Span,
    /// The lower-cased source, shared by every diagnostic that quotes it; made on first use.
    lowered: Option<Arc<str>>,
}

#[cfg(any(test, feature = "fuzz"))]
impl<'o> Cx<'o> {
    pub(super) fn for_test(src: &str, opts: &'o CompileOptions) -> Self {
        Cx::new(src, opts, Some(opts.raw_version))
    }

    pub(super) fn logged_diagnostics(&self) -> &[Diagnostic] {
        &self.diagnostics
    }
}

impl<'o> Cx<'o> {
    /// The context of compiling `src` with `opts`, its raw version replaced by `query_version`
    /// (by [`MolangVersion::Invalid`] when `None`).
    pub(super) fn new(
        src: &str,
        opts: &'o CompileOptions,
        query_version: Option<RawVersion>,
    ) -> Self {
        Cx {
            opts: Settings::of(opts),
            query_version,
            diagnostics: Vec::new(),
            suppressed: 0,
            logged: false,
            whole: Span::saturating(0, src.len()),
            lowered: None,
        }
    }

    /// The raw version of the compile: the query version, or `Invalid` without one.
    pub(super) fn raw_version(&self) -> RawVersion {
        self.query_version.unwrap_or(MolangVersion::Invalid.into())
    }

    /// The version parser gates use: [`RawVersion::effective`] of [`Cx::raw_version`].
    pub(super) fn version(&self) -> MolangVersion {
        self.raw_version().effective()
    }

    /// `node` as messages name it, a host math function by its declared name.
    pub(super) fn friendly(&self, node: &Node) -> Friendly<'o> {
        node.friendly(self.opts.math)
    }
}

impl Cx<'_> {
    pub(super) fn language(
        &mut self,
        message: LanguageMessage,
        span: Span,
        args: &[&dyn fmt::Display],
    ) {
        self.language_as(message, message.effect().severity(), span, args);
    }

    /// A message that quotes the source goes through [`Cx::language_rest`] instead.
    pub(super) fn language_as(
        &mut self,
        message: LanguageMessage,
        severity: Severity,
        span: Span,
        args: &[&dyn fmt::Display],
    ) {
        self.push(Diagnostic::language(message, severity, span.into(), args));
    }

    /// Logs `message` quoting `lowered` from byte `from` to the end. The quoted text is copied once
    /// per compile, on first use.
    pub(super) fn language_rest(
        &mut self,
        message: LanguageMessage,
        span: Span,
        from: usize,
        lowered: &[u8],
    ) {
        // The lowering only changes ASCII bytes of a `str`, so the text is valid UTF-8.
        let shared = Arc::clone(
            self.lowered
                .get_or_insert_with(|| Arc::from(String::from_utf8_lossy(lowered))),
        );
        let end = shared.len();
        self.quote(message, span, shared, from.min(end)..end);
    }

    /// Logs `message` with `source[range]` as its one argument.
    pub(super) fn quote(
        &mut self,
        message: LanguageMessage,
        span: Span,
        source: Arc<str>,
        range: Range<usize>,
    ) {
        self.push(Diagnostic::quoting(
            message,
            message.effect().severity(),
            span.into(),
            source,
            Span::saturating(range.start, range.end).into(),
        ));
    }

    pub(super) fn lint(
        &mut self,
        code: DiagCode,
        severity: Severity,
        span: Span,
        text: impl Into<Cow<'static, str>>,
    ) {
        self.push(Diagnostic::lint(code, severity, span.into(), text));
    }

    /// Past [`MAX_DIAGNOSTICS`] a diagnostic is only counted (with `Deviations::diagnostic_limit`
    /// on); `logged` is set either way.
    fn push(&mut self, diagnostic: Diagnostic) {
        self.logged |= diagnostic.language_message().is_some();
        if self.opts.deviations.diagnostic_limit && self.diagnostics.len() >= MAX_DIAGNOSTICS {
            self.suppressed += 1;
        } else {
            self.diagnostics.push(diagnostic);
        }
    }

    pub(super) fn finish(mut self) -> (Vec<Diagnostic>, bool) {
        if self.suppressed > 0 {
            let text = format!(
                "{} more diagnostics suppressed (at most {MAX_DIAGNOSTICS} are kept per compile)",
                self.suppressed
            );
            // Past the cap, so not through `push`.
            let note = Diagnostic::lint(
                DiagCode::DiagnosticLimit,
                Severity::Info,
                self.whole.into(),
                text,
            );
            self.diagnostics.push(note);
        }
        (self.diagnostics, self.logged)
    }

    pub(super) fn whole(&self) -> Span {
        self.whole
    }

    /// Lints the compile's version: a missing one always, one outside −1..=13 or `Invalid` with
    /// `Deviations::object_version_warning` on.
    pub(super) fn lint_version(&mut self) {
        let span = self.whole();
        let Some(raw) = self.query_version else {
            // A caller bug, so reported whatever the deviations.
            self.lint(
                DiagCode::InvalidVersion,
                Severity::Warning,
                span,
                "this string source has no MolangVersion: it was made without its load context \
                 (MolangSource::string_without_context) and MolangSource::set_context_version was \
                 not called; compiled as MolangVersion Invalid, no query resolves",
            );
            return;
        };
        if !self.opts.deviations.object_version_warning {
            return;
        }
        match MolangVersion::from_i16(raw.0) {
            None => self.lint(
                DiagCode::InvalidVersion,
                Severity::Warning,
                span,
                format!(
                    "MolangVersion {raw} is outside -1..=13: parser rules as version {}, no query \
                     resolves",
                    raw.effective().as_i16()
                ),
            ),
            Some(MolangVersion::Invalid) => self.lint(
                DiagCode::InvalidVersion,
                Severity::Info,
                span,
                "MolangVersion Invalid (-1): version-0 parser rules, no query resolves",
            ),
            Some(_) => {}
        }
    }

    /// Below version 4 an empty expression is rejected without a language message; this info keeps
    /// every failed compile diagnosed.
    pub(super) fn silent_empty(&mut self, span: Span) {
        self.lint(
            DiagCode::Syntax,
            Severity::Info,
            span,
            "empty expression: rejected without a message below MolangVersion 4, evaluates to 0",
        );
    }

    pub(super) fn lint_query_side(&mut self, decl: &QueryDecl, span: Span) {
        if self.opts.deviations.query_client_only
            && self.opts.catalog.side() == Side::Server
            && decl.shape().side.is_client_only()
        {
            self.lint(
                DiagCode::QueryClientOnly,
                Severity::Info,
                span,
                format!(
                    "{} is a client-only query in this crate's table; compiled for Side::Server it \
                     evaluates to its default here",
                    decl.name()
                ),
            );
        }
    }

    /// A query missing from a server catalogue (`query.is_on_screen`): explains the
    /// `Failed to resolve query` that follows.
    pub(super) fn lint_query_not_on_server(&mut self, decl: &QueryDecl, span: Span) {
        if self.opts.deviations.query_client_only {
            self.lint(
                DiagCode::QueryClientOnly,
                Severity::Info,
                span,
                format!(
                    "{} is not registered on the dedicated server; it resolves only when compiled \
                     for Side::Client",
                    decl.name()
                ),
            );
        }
    }

    /// Why a known query name did not resolve.
    pub(super) fn lint_query_miss(&mut self, decl: &QueryDecl, span: Span) {
        if !self.opts.deviations.query_client_only {
            return;
        }
        let name = decl.name();
        let (code, text) = match self.query_version {
            None => (
                DiagCode::QueryDeprecated,
                format!("{name} exists, but this source has no MolangVersion to resolve it at"),
            ),
            Some(raw) if decl.implementation_at_raw(raw).is_none() => (
                DiagCode::QueryDeprecated,
                format!("{name} exists, but not at MolangVersion {raw}"),
            ),
            Some(_) if !self.opts.experiments.contains(decl.shape().experiments) => (
                DiagCode::QueryExperiment,
                format!("{name} is behind an experiment that is not enabled"),
            ),
            Some(_) => return,
        };
        self.lint(code, Severity::Info, span, text);
    }

    pub(super) fn lint_query_arity(&mut self, query: QueryIndex, call: &Node) {
        if !self.opts.deviations.query_arity_lint {
            return;
        }
        let decl = self.opts.catalog.decl(query);
        let count = call.children.len();
        if !decl.shape().args.contains(count) {
            self.lint(
                DiagCode::QueryArity,
                Severity::Warning,
                call.full_span(),
                format!(
                    "{} is registered with {}, {count} given (this crate's check)",
                    decl.name(),
                    decl.shape().args.arguments()
                ),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::catalog::{
        Arity, QueryAdmission, QueryDecl, QuerySetMask, QueryShape, QuerySide, Side,
    };
    use crate::compile::{
        CompileFailure, CompileOptions, Cx, Deviations, MAX_DIAGNOSTICS,
        ast::{Node, Payload, Span},
        compile,
        test_support::*,
    };
    use crate::diag::{DiagCode, Diagnostic, LanguageMessage, Severity};
    use crate::ops::ExpressionOp as Op;
    use crate::reference_catalog;
    use crate::stdlib::query;
    use crate::version::{ExperimentMask, RawVersion};
    use std::borrow::Cow;
    use std::sync::Arc;

    fn decl(name: &str) -> &'static QueryDecl {
        crate::stdlib::queries(Side::Server).get(name).unwrap()
    }

    fn client() -> CompileOptions {
        CompileOptions {
            catalog: crate::stdlib::queries(Side::Client).clone(),
            ..opts()
        }
    }

    fn with_cx<R>(
        src: &str,
        options: &CompileOptions,
        f: impl FnOnce(&mut Cx<'_>) -> R,
    ) -> (R, Vec<Diagnostic>) {
        let mut cx = Cx::for_test(src, options);
        let result = f(&mut cx);
        (result, cx.logged_diagnostics().to_vec())
    }

    fn language_note() -> Diagnostic {
        Diagnostic::language(LanguageMessage::NoTokens, Severity::Error, 0..1, &[])
    }

    fn own_note(index: usize) -> Diagnostic {
        Diagnostic::lint(
            DiagCode::QueryArity,
            Severity::Warning,
            0..1,
            format!("note {index}"),
        )
    }

    #[test]
    fn a_fresh_context_has_no_diagnostics_and_the_whole_source_span() {
        let options = opts();
        let cx = Cx::for_test("abcdef", &options);
        assert!(cx.logged_diagnostics().is_empty());
        assert_eq!(cx.whole(), Span::new(0, 6));
        let cx = Cx::for_test("", &options);
        assert_eq!(cx.whole(), Span::new(0, 0));
    }

    #[test]
    fn language_logs_with_the_severity_of_the_messages_effect() {
        let options = opts();
        let ((), log) = with_cx("source", &options, |cx| {
            cx.language(LanguageMessage::NoTokens, Span::new(0, 1), &[]);
            cx.language(LanguageMessage::BadExponent, Span::new(1, 2), &[]);
            cx.language(LanguageMessage::Unreachable, Span::new(2, 3), &[&"Break"]);
        });
        assert_eq!(
            log.iter().map(Diagnostic::severity).collect::<Vec<_>>(),
            [Severity::Error, Severity::Warning, Severity::Error]
        );
        assert_eq!(
            log.iter()
                .map(Diagnostic::language_message)
                .collect::<Vec<_>>(),
            [
                Some(LanguageMessage::NoTokens),
                Some(LanguageMessage::BadExponent),
                Some(LanguageMessage::Unreachable)
            ]
        );
        assert_eq!(
            log.iter().map(|d| d.span().clone()).collect::<Vec<_>>(),
            [0..1, 1..2, 2..3]
        );
        assert_eq!(
            log.iter().map(Diagnostic::code).collect::<Vec<_>>(),
            [DiagCode::Syntax, DiagCode::Syntax, DiagCode::StatementForm]
        );
        assert_eq!(
            log[2].message(),
            "Error: unreachable statements after Break."
        );
    }

    #[test]
    fn language_as_logs_with_the_given_severity() {
        let options = opts();
        let ((), log) = with_cx("source", &options, |cx| {
            cx.language_as(
                LanguageMessage::OperatorOnLhs,
                Severity::Warning,
                Span::new(0, 2),
                &[&"String"],
            );
            cx.language_as(
                LanguageMessage::NoTokens,
                Severity::Info,
                Span::new(0, 0),
                &[],
            );
        });
        assert_eq!(log[0].severity(), Severity::Warning);
        assert_eq!(
            log[0].message(),
            "Error: cannot use String operators on the left side of an assignment expression"
        );
        assert_eq!(log[1].severity(), Severity::Info);
        assert_eq!(log[1].message(), "No tokens found in expression");
    }

    #[test]
    fn a_message_without_arguments_or_placeholders_is_a_borrowed_static_text() {
        let options = opts();
        let ((), log) = with_cx("source", &options, |cx| {
            cx.language(LanguageMessage::NoTokens, Span::new(0, 0), &[]);
        });
        assert!(matches!(log[0].message(), Cow::Borrowed(_)));
    }

    #[test]
    fn a_message_with_placeholders_but_no_arguments_drops_them() {
        let options = opts();
        let ((), log) = with_cx("source", &options, |cx| {
            cx.language(LanguageMessage::UnrecognizedToken, Span::new(0, 1), &[]);
        });
        assert_eq!(log[0].message(), "unrecognized token: ");
    }

    #[test]
    fn language_rest_quotes_the_lowered_source_from_a_byte() {
        let options = opts();
        let lowered = crate::compile::lex::lower("A1 + $BC");
        let ((), log) = with_cx("A1 + $BC", &options, |cx| {
            cx.language_rest(
                LanguageMessage::UnrecognizedToken,
                Span::new(5, 6),
                5,
                &lowered,
            );
            cx.language_rest(LanguageMessage::UnknownToken, Span::new(0, 1), 0, &lowered);
            cx.language_rest(
                LanguageMessage::UnrecognizedToken,
                Span::new(8, 8),
                8,
                &lowered,
            );
            cx.language_rest(
                LanguageMessage::UnrecognizedToken,
                Span::new(9, 9),
                99,
                &lowered,
            );
        });
        assert_eq!(log[0].message(), "unrecognized token: $bc");
        assert_eq!(log[1].message(), "Error: unknown token: a1 + $bc");
        assert_eq!(log[2].message(), "unrecognized token: ");
        assert_eq!(log[3].message(), "unrecognized token: ");
        assert_eq!(log[0].span(), 5..6);
    }

    #[test]
    fn the_lowered_copy_is_shared_by_the_diagnostics_that_quote_it() {
        let options = opts();
        let mut cx = Cx::for_test("ABC", &options);
        assert!(
            cx.lowered.is_none(),
            "no copy before a message quotes the text"
        );
        cx.language_rest(
            LanguageMessage::UnrecognizedToken,
            Span::new(0, 1),
            0,
            b"abc",
        );
        let shared = cx.lowered.clone().expect("the copy");
        assert_eq!(&*shared, "abc");
        cx.language_rest(
            LanguageMessage::UnrecognizedToken,
            Span::new(0, 1),
            1,
            b"ignored: the copy is made once",
        );
        assert_eq!(
            Arc::strong_count(&shared),
            4,
            "the context, two diagnostics and this test"
        );
        assert_eq!(
            cx.logged_diagnostics()[1].message(),
            "unrecognized token: bc"
        );
    }

    #[test]
    fn a_compile_that_quotes_nothing_makes_no_copy_of_the_text() {
        let options = opts();
        let mut cx = Cx::for_test("ABC", &options);
        cx.language(LanguageMessage::NoTokens, Span::new(0, 1), &[]);
        assert!(cx.lowered.is_none());
    }

    #[test]
    fn lint_logs_a_diagnostic_without_a_language_message_row() {
        let options = opts();
        let ((), log) = with_cx("source", &options, |cx| {
            cx.lint(
                DiagCode::QueryArity,
                Severity::Warning,
                Span::new(1, 4),
                "static text",
            );
            cx.lint(
                DiagCode::QueryArity,
                Severity::Info,
                Span::new(2, 3),
                format!("built {}", 7),
            );
        });
        assert!(log[0].language_message().is_none() && log[1].language_message().is_none());
        assert_eq!(log[0].message(), "static text");
        assert_eq!(log[1].message(), "built 7");
        assert_eq!(log[0].span(), 1..4);
    }

    #[test]
    fn silent_empty_informs_about_an_empty_expression_rejected_without_a_language_message() {
        let options = opts();
        let ((), log) = with_cx("", &options, |cx| cx.silent_empty(Span::new(0, 0)));
        assert_eq!(log.len(), 1);
        assert_eq!(log[0].code(), DiagCode::Syntax);
        assert_eq!(log[0].severity(), Severity::Info);
        assert!(log[0].language_message().is_none());
        assert_eq!(
            log[0].message(),
            "empty expression: rejected without a message below MolangVersion 4, evaluates to 0"
        );
    }

    #[test]
    fn up_to_the_limit_every_diagnostic_is_kept_without_a_note() {
        let options = opts();
        let mut cx = Cx::for_test("source", &options);
        for index in 0..MAX_DIAGNOSTICS {
            cx.push(own_note(index));
        }
        assert_eq!(cx.logged_diagnostics().len(), MAX_DIAGNOSTICS);
        let (diagnostics, logged) = cx.finish();
        assert_eq!(diagnostics.len(), MAX_DIAGNOSTICS);
        assert!(diagnostics.iter().all(|d| d.code() == DiagCode::QueryArity));
        assert!(!logged);
    }

    #[test]
    fn past_the_limit_exactly_the_limit_is_kept_and_one_summary_note_counts_the_rest() {
        let options = opts();
        let mut cx = Cx::for_test("abcdef", &options);
        for index in 0..MAX_DIAGNOSTICS + 44 {
            cx.push(own_note(index));
        }
        assert_eq!(cx.logged_diagnostics().len(), MAX_DIAGNOSTICS);
        assert_eq!(cx.suppressed, 44);
        let (diagnostics, _) = cx.finish();
        assert_eq!(diagnostics.len(), MAX_DIAGNOSTICS + 1);
        let note = diagnostics.last().expect("a note");
        assert_eq!(note.code(), DiagCode::DiagnosticLimit);
        assert_eq!(note.severity(), Severity::Info);
        assert_eq!(note.span(), 0..6);
        assert_eq!(note.language_message(), None);
        assert_eq!(
            note.message(),
            "44 more diagnostics suppressed (at most 256 are kept per compile)"
        );
        assert_eq!(diagnostics[0].message(), "note 0");
        assert_eq!(
            diagnostics[MAX_DIAGNOSTICS - 1].message(),
            format!("note {}", MAX_DIAGNOSTICS - 1)
        );
    }

    #[test]
    fn one_diagnostic_past_the_limit_is_counted_as_one() {
        let options = opts();
        let mut cx = Cx::for_test("x", &options);
        for index in 0..=MAX_DIAGNOSTICS {
            cx.push(own_note(index));
        }
        let (diagnostics, _) = cx.finish();
        assert_eq!(diagnostics.len(), MAX_DIAGNOSTICS + 1);
        assert_eq!(
            diagnostics[MAX_DIAGNOSTICS].message(),
            "1 more diagnostics suppressed (at most 256 are kept per compile)"
        );
    }

    #[test]
    fn a_suppressed_language_message_still_counts_as_logged() {
        let options = opts();
        let mut cx = Cx::for_test("source", &options);
        for index in 0..MAX_DIAGNOSTICS {
            cx.push(own_note(index));
        }
        assert!(!cx.logged);
        cx.push(language_note());
        assert!(cx.logged, "the flag is set before the cap is applied");
        assert_eq!(cx.logged_diagnostics().len(), MAX_DIAGNOSTICS);
        let (diagnostics, logged) = cx.finish();
        assert!(logged);
        assert_eq!(diagnostics.len(), MAX_DIAGNOSTICS + 1);
        assert!(diagnostics.iter().all(|d| d.language_message().is_none()));
    }

    #[test]
    fn a_suppressed_own_diagnostic_does_not_count_as_logged() {
        let options = opts();
        let mut cx = Cx::for_test("source", &options);
        for index in 0..MAX_DIAGNOSTICS + 10 {
            cx.push(own_note(index));
        }
        let (_, logged) = cx.finish();
        assert!(!logged);
    }

    #[test]
    fn with_no_deviations_the_cap_is_off() {
        let options = CompileOptions {
            deviations: Deviations::NONE,
            ..opts()
        };
        let mut cx = Cx::for_test("source", &options);
        for index in 0..MAX_DIAGNOSTICS * 2 {
            cx.push(own_note(index));
        }
        assert_eq!(cx.suppressed, 0);
        let (diagnostics, _) = cx.finish();
        assert_eq!(diagnostics.len(), MAX_DIAGNOSTICS * 2);
        assert!(
            diagnostics
                .iter()
                .all(|d| d.code() != DiagCode::DiagnosticLimit)
        );
    }

    #[test]
    fn only_the_cap_deviation_switches_the_cap_off() {
        let options = CompileOptions {
            deviations: Deviations {
                diagnostic_limit: false,
                ..Deviations::ALL
            },
            ..opts()
        };
        let mut cx = Cx::for_test("source", &options);
        for index in 0..MAX_DIAGNOSTICS + 5 {
            cx.push(own_note(index));
        }
        assert_eq!(cx.finish().0.len(), MAX_DIAGNOSTICS + 5);
    }

    #[test]
    fn a_compile_that_logs_more_than_the_limit_keeps_the_limit_and_the_note() {
        // Each `1e;` logs one keep-message (a malformed exponent).
        let src = "1e;".repeat(300);
        let compiled = compile(&src, &opts());
        assert_eq!(compiled.diagnostics().len(), MAX_DIAGNOSTICS + 1);
        assert!(
            compiled.diagnostics()[..MAX_DIAGNOSTICS]
                .iter()
                .all(|d| d.language_message() == Some(LanguageMessage::BadExponent))
        );
        let note = compiled.diagnostics().last().expect("a note");
        assert_eq!(note.code(), DiagCode::DiagnosticLimit);
        assert_eq!(
            note.message(),
            "44 more diagnostics suppressed (at most 256 are kept per compile)"
        );
        assert!(!compiled.parses_cleanly());
        assert_eq!(compiled.failure(), None);
    }

    #[test]
    fn the_same_compile_without_deviations_keeps_every_message() {
        let src = "1e;".repeat(300);
        let compiled = compile(
            &src,
            &CompileOptions {
                deviations: Deviations::NONE,
                ..opts()
            },
        );
        assert_eq!(compiled.diagnostics().len(), 300);
        assert!(
            compiled
                .diagnostics()
                .iter()
                .all(|d| d.language_message() == Some(LanguageMessage::BadExponent))
        );
    }

    #[test]
    fn the_arity_lint_warns_about_a_call_outside_the_registered_counts() {
        let compiled = compile("q.ride_body_x_rotation(1)", &client());
        assert_eq!(compiled.failure(), None);
        assert_eq!(codes(&compiled), [DiagCode::QueryArity]);
        assert_eq!(compiled.diagnostics()[0].severity(), Severity::Warning);
        assert!(compiled.diagnostics()[0].language_message().is_none());
        assert_eq!(
            compiled.diagnostics()[0].message(),
            "query.ride_body_x_rotation is registered with 0 arguments, 1 given (this crate's check)"
        );
        assert!(
            compiled.parses_cleanly(),
            "our lints are not language messages"
        );
    }

    #[test]
    fn the_arity_lint_can_be_switched_off() {
        let off = CompileOptions {
            deviations: Deviations {
                query_arity_lint: false,
                ..Deviations::ALL
            },
            ..client()
        };
        assert!(
            compile("q.ride_body_x_rotation(1)", &off)
                .diagnostics()
                .is_empty()
        );
        assert!(
            compile(
                "q.ride_body_x_rotation(1)",
                &CompileOptions {
                    deviations: Deviations::NONE,
                    ..client()
                }
            )
            .diagnostics()
            .is_empty()
        );
    }

    #[test]
    fn the_arity_lint_is_quiet_within_the_counts_and_names_an_unbounded_maximum() {
        assert!(
            compile("q.ride_body_x_rotation", &client())
                .diagnostics()
                .is_empty()
        );
        assert!(
            compile("q.is_name_any('a','b')", &client())
                .diagnostics()
                .is_empty()
        );
        let compiled = compile("q.is_name_any", &client());
        assert_eq!(
            messages(&compiled),
            [
                "query.is_name_any is registered with at least 1 argument, 0 given (this crate's check)"
            ]
        );
    }

    #[test]
    fn the_arity_lint_directly() {
        let node = {
            let mut call = Node::token(Op::QueryFunction, Payload::None, Span::new(2, 5));
            call.children = vec![Node::token(Op::Float, Payload::Float(1.0), Span::new(6, 8))];
            call
        };
        let lint = |options: &CompileOptions, name| {
            let query = options.catalog.index_of(name).unwrap();
            with_cx("abcdefghij", options, |cx| {
                cx.lint_query_arity(query, &node);
            })
            .1
        };
        let log = lint(&opts(), query::RIDE_BODY_X_ROTATION);
        assert_eq!(log.len(), 1);
        assert_eq!(log[0].span(), 2..8);
        assert_eq!(log[0].code(), DiagCode::QueryArity);
        let no_deviations = CompileOptions {
            deviations: Deviations::NONE,
            ..opts()
        };
        assert!(lint(&no_deviations, query::RIDE_BODY_X_ROTATION).is_empty());
        assert!(
            lint(&opts(), query::IS_BABY).is_empty(),
            "is_baby takes any number of arguments"
        );
    }

    #[test]
    fn a_client_only_query_compiled_for_the_server_informs() {
        let compiled = compile("q.is_first_person", &opts());
        assert_eq!(compiled.failure(), None);
        assert_eq!(codes(&compiled), [DiagCode::QueryClientOnly]);
        assert_eq!(compiled.diagnostics()[0].severity(), Severity::Info);
        assert_eq!(compiled.diagnostics()[0].span(), 0..17);
        assert_eq!(
            compiled.diagnostics()[0].message(),
            "query.is_first_person is a client-only query in this crate's table; compiled for Side::Server it evaluates to its default here"
        );
    }

    #[test]
    fn the_client_only_info_is_off_for_the_client_and_without_the_deviation() {
        assert!(
            compile("q.is_first_person", &client())
                .diagnostics()
                .is_empty()
        );
        let off = CompileOptions {
            deviations: Deviations {
                query_client_only: false,
                ..Deviations::ALL
            },
            ..opts()
        };
        assert!(compile("q.is_first_person", &off).diagnostics().is_empty());
        assert!(
            compile(
                "q.is_first_person",
                &CompileOptions {
                    deviations: Deviations::NONE,
                    ..opts()
                }
            )
            .diagnostics()
            .is_empty()
        );
    }

    #[test]
    fn a_query_for_both_sides_has_no_client_only_info() {
        assert!(compile("q.is_baby", &opts()).diagnostics().is_empty());
    }

    #[test]
    fn the_client_only_lint_directly() {
        let options = opts();
        let ((), log) = with_cx("abcdef", &options, |cx| {
            cx.lint_query_side(decl(query::IS_FIRST_PERSON), Span::new(1, 3));
            cx.lint_query_side(decl(query::IS_BABY), Span::new(1, 3));
        });
        assert_eq!(log.len(), 1);
        assert_eq!(log[0].span(), 1..3);
        let on_client = client();
        assert!(
            with_cx("abcdef", &on_client, |cx| cx
                .lint_query_side(decl(query::IS_FIRST_PERSON), Span::new(0, 1)))
            .1
            .is_empty()
        );
    }

    #[test]
    fn a_query_the_server_does_not_have_explains_why_it_fails_to_resolve() {
        let compiled = compile("q.is_on_screen", &opts());
        assert_eq!(compiled.failure(), Some(CompileFailure::Rejected));
        assert_eq!(
            codes(&compiled),
            [
                DiagCode::QueryClientOnly,
                DiagCode::UnknownQuery,
                DiagCode::Syntax
            ]
        );
        assert_eq!(compiled.diagnostics()[0].severity(), Severity::Info);
        assert_eq!(
            compiled.diagnostics()[0].message(),
            "query.is_on_screen is not registered on the dedicated server; it resolves only when compiled for Side::Client"
        );
        assert_eq!(
            compiled.diagnostics()[1].language_message(),
            Some(LanguageMessage::QueryUnresolved)
        );
        let client_side = compile("q.is_on_screen", &client());
        assert_eq!(client_side.failure(), None);
        assert!(client_side.diagnostics().is_empty());
    }

    #[test]
    fn the_not_on_server_info_can_be_switched_off() {
        let off = CompileOptions {
            deviations: Deviations {
                query_client_only: false,
                ..Deviations::ALL
            },
            ..opts()
        };
        let compiled = compile("q.is_on_screen", &off);
        assert_eq!(codes(&compiled), [DiagCode::UnknownQuery, DiagCode::Syntax]);
    }

    #[test]
    fn the_not_on_server_lint_directly() {
        let options = opts();
        let ((), log) = with_cx("abcdef", &options, |cx| {
            cx.lint_query_not_on_server(decl(query::IS_ON_SCREEN), Span::new(0, 2));
        });
        assert_eq!(log.len(), 1);
        assert_eq!(log[0].severity(), Severity::Info);
        let off = CompileOptions {
            deviations: Deviations::NONE,
            ..opts()
        };
        assert!(
            with_cx("abcdef", &off, |cx| cx.lint_query_not_on_server(
                decl(query::IS_ON_SCREEN),
                Span::new(0, 2)
            ))
            .1
            .is_empty()
        );
    }

    #[test]
    fn a_query_that_does_not_exist_at_the_version_is_explained() {
        let quiet = Deviations {
            object_version_warning: false,
            ..Deviations::ALL
        };
        let compiled = compile(
            "q.is_first_person",
            &CompileOptions {
                raw_version: RawVersion(14),
                deviations: quiet,
                ..client()
            },
        );
        assert_eq!(
            codes(&compiled),
            [
                DiagCode::QueryDeprecated,
                DiagCode::UnknownQuery,
                DiagCode::Syntax
            ]
        );
        assert_eq!(compiled.diagnostics()[0].severity(), Severity::Info);
        assert!(
            compiled.diagnostics()[0]
                .message()
                .contains("exists, but not at MolangVersion 14")
        );
        assert_eq!(compiled.failure(), Some(CompileFailure::Rejected));
        let off = CompileOptions {
            raw_version: RawVersion(14),
            deviations: Deviations {
                query_client_only: false,
                object_version_warning: false,
                ..Deviations::ALL
            },
            ..client()
        };
        assert_eq!(
            codes(&compile("q.is_first_person", &off)),
            [DiagCode::UnknownQuery, DiagCode::Syntax]
        );
    }

    #[test]
    fn a_query_that_resolves_has_no_miss_lint() {
        let options = client();
        let ((), log) = with_cx("abcdef", &options, |cx| {
            cx.lint_query_miss(decl(query::IS_BABY), Span::new(0, 2));
        });
        assert!(log.is_empty());
    }

    #[test]
    fn a_query_behind_a_disabled_experiment_is_explained() {
        let tests = CompileOptions {
            experiments: ExperimentMask::empty(),
            ..reference_catalog::options(13)
        };
        let compiled = compile("q.experimental_test", &tests);
        assert_eq!(
            codes(&compiled),
            [
                DiagCode::QueryExperiment,
                DiagCode::UnknownQuery,
                DiagCode::Syntax
            ]
        );
        assert_eq!(
            compiled.diagnostics()[0].message(),
            "query.experimental_test is behind an experiment that is not enabled"
        );
    }

    #[test]
    fn an_enabled_experiment_is_not_blamed_for_a_query_set_miss() {
        // `query.experimental_test` is in the test set, which the default set does not admit.
        let enabled = CompileOptions {
            admission: QueryAdmission::Sets(QuerySetMask::DEFAULT),
            ..reference_catalog::options(13)
        };
        let compiled = compile("q.experimental_test", &enabled);
        assert_eq!(codes(&compiled), [DiagCode::UnknownQuery, DiagCode::Syntax]);
        let test_decl = reference_catalog::catalog()
            .get(reference_catalog::EXPERIMENTAL_TEST)
            .unwrap();
        let ((), log) = with_cx("abcdef", &enabled, |cx| {
            cx.lint_query_miss(test_decl, Span::new(0, 2));
        });
        assert!(log.is_empty(), "{log:?}");
    }

    #[test]
    fn a_host_declared_query_is_linted_from_its_declaration() {
        let mine = QueryShape {
            args: Arity::exactly(1),
            side: QuerySide::CLIENT,
            ..QueryShape::DEFAULT
        };
        let client_thing = QueryShape {
            side: QuerySide::Both {
                on_dedicated_server: false,
            },
            ..QueryShape::DEFAULT
        };
        let catalog = crate::stdlib::queries(Side::Server)
            .extended([
                QueryDecl::new("query.mine", mine).unwrap(),
                QueryDecl::new("query.client_thing", client_thing).unwrap(),
            ])
            .unwrap();
        let options = CompileOptions::new(catalog, crate::version::MolangVersion::LATEST);
        let compiled = compile("q.mine(1, 2)", &options);
        assert_eq!(compiled.failure(), None);
        assert_eq!(
            messages(&compiled),
            [
                "query.mine is a client-only query in this crate's table; compiled for Side::Server it evaluates to its default here",
                "query.mine is registered with 1 argument, 2 given (this crate's check)"
            ]
        );
        let compiled = compile("q.client_thing", &options);
        assert_eq!(
            codes(&compiled),
            [
                DiagCode::QueryClientOnly,
                DiagCode::UnknownQuery,
                DiagCode::Syntax
            ]
        );
    }
}
