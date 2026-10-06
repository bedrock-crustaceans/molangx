//! What a compile returns: the outcome, the diagnostics and the expression.

use crate::catalog::{MathCatalog, QueryCatalog, QueryIndex};
use crate::compile::{
    CompileOptions, Cx,
    ast::{Node, Payload},
    program::{Program, ProgramFlags},
    sema,
};
use crate::diag::{Diagnostic, Severity};
use crate::ops::ExpressionOp as Op;
use crate::version::{MolangVersion, RawVersion};
use std::collections::HashSet;
use std::fmt;
use std::sync::Arc;

/// The outcome of a compile, with the expression where there is one.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub enum CompileOutcome {
    /// The expression compiled, possibly with non-rejecting diagnostics.
    Success(Expr),
    /// The parse or the link failed; the expression in its place evaluates to 0
    /// ([`Expr::is_rejected`]). At least one diagnostic says why.
    Rejected(Expr),
    /// The expression uses render-controller arrays (`array.x[i]`), which this crate cannot
    /// evaluate.
    UsesArrays,
    /// The expression uses `geometry.` / `material.` / `texture.` variables, which this crate
    /// cannot evaluate.
    UsesResources,
}

impl CompileOutcome {
    /// What kind of failure this is; `None` for a success.
    pub const fn failure(&self) -> Option<CompileFailure> {
        match self {
            Self::Success(_) => None,
            Self::Rejected(_) => Some(CompileFailure::Rejected),
            Self::UsesArrays => Some(CompileFailure::UsesArrays),
            Self::UsesResources => Some(CompileFailure::UsesResources),
        }
    }
}

/// How a compile failed: the [`CompileOutcome`]s other than a success, without their data.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum CompileFailure {
    /// [`CompileOutcome::Rejected`].
    Rejected,
    /// [`CompileOutcome::UsesArrays`].
    UsesArrays,
    /// [`CompileOutcome::UsesResources`].
    UsesResources,
}

/// What [`compile`](super::compile) returns: the outcome and everything the compile logged.
///
/// ```
/// # #[cfg(feature = "stdlib")]
/// # {
/// use molangx::compile::{CompileOptions, CompileOutcome, compile};
/// use molangx::version::MolangVersion;
///
/// let options = CompileOptions::server(MolangVersion::LATEST);
/// let compiled = compile("1 +", &options);
/// assert!(matches!(compiled.outcome(), CompileOutcome::Rejected(_)));
/// assert_eq!(compiled.errors().count(), 1);
/// // Rust-style handling: an error with the diagnostics ...
/// assert!(compiled.clone().into_result().is_err());
/// // ... or the expression in its place, here the constant 0.
/// assert_eq!(compiled.expr_or_zero().and_then(|expr| expr.as_constant()), Some(0.0));
/// # }
/// ```
#[derive(Clone, Debug)]
pub struct Compiled {
    outcome: CompileOutcome,
    diagnostics: Vec<Diagnostic>,
    /// The optimised tree of a source that parsed, also for the outcomes without an expression.
    tree: Option<Arc<Node>>,
    /// Whether a language message was logged, counting the ones the cap dropped.
    logged: bool,
}

impl Compiled {
    pub(super) fn done(cx: Cx<'_>, outcome: CompileOutcome, tree: Option<Arc<Node>>) -> Self {
        let (diagnostics, logged) = cx.finish();
        debug_assert!(
            !matches!(outcome, CompileOutcome::Rejected(_)) || !diagnostics.is_empty(),
            "a rejection without a diagnostic"
        );
        Self {
            outcome,
            diagnostics,
            tree,
            logged,
        }
    }

    /// The outcome.
    pub const fn outcome(&self) -> &CompileOutcome {
        &self.outcome
    }

    /// The outcome, by value.
    pub fn into_outcome(self) -> CompileOutcome {
        self.outcome
    }

    /// Everything the compile logged, in order. With the deviation
    /// [`diagnostic_limit`](super::Deviations::diagnostic_limit) at most
    /// [`MAX_DIAGNOSTICS`](crate::compile::MAX_DIAGNOSTICS) are kept, followed by one
    /// [`DiagCode::DiagnosticLimit`](crate::diag::DiagCode::DiagnosticLimit) note counting the
    /// rest.
    pub fn diagnostics(&self) -> &[Diagnostic] {
        &self.diagnostics
    }

    /// The diagnostics of [`Severity::Error`], in order.
    pub fn errors(&self) -> impl Iterator<Item = &Diagnostic> + '_ {
        self.diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.severity() == Severity::Error)
    }

    /// Whether the expression compiled ([`CompileOutcome::Success`]).
    pub const fn is_success(&self) -> bool {
        matches!(self.outcome, CompileOutcome::Success(_))
    }

    /// How the compile failed; `None` for a success.
    pub const fn failure(&self) -> Option<CompileFailure> {
        self.outcome.failure()
    }

    /// The compiled expression of a success; `None` otherwise, a rejection included (see
    /// [`Compiled::expr_or_zero`]).
    pub const fn expr(&self) -> Option<&Expr> {
        match &self.outcome {
            CompileOutcome::Success(expr) => Some(expr),
            _ => None,
        }
    }

    /// The expression of a success, or the constant 0 of a rejection ([`Expr::is_rejected`]);
    /// `None` for [`CompileOutcome::UsesArrays`] and [`CompileOutcome::UsesResources`].
    pub const fn expr_or_zero(&self) -> Option<&Expr> {
        match &self.outcome {
            CompileOutcome::Success(expr) | CompileOutcome::Rejected(expr) => Some(expr),
            CompileOutcome::UsesArrays | CompileOutcome::UsesResources => None,
        }
    }

    /// The expression of a success with everything the compile logged about it, or an error with
    /// the kind of failure and the diagnostics.
    ///
    /// A success can carry diagnostics: warnings, notes and, with
    /// [`Deviations::NONE`](super::Deviations::NONE), errors of a kept expression. They come with
    /// the expression so that `?` does not drop them.
    ///
    /// ```
    /// # #[cfg(feature = "stdlib")]
    /// # {
    /// use molangx::compile::{CompileOptions, compile};
    /// use molangx::diag::Severity;
    /// use molangx::version::MolangVersion;
    ///
    /// let (expr, diagnostics) =
    ///     compile("1e;", &CompileOptions::server(MolangVersion::LATEST)).into_result()?;
    /// assert!(!expr.is_rejected());
    /// assert_eq!(diagnostics.len(), 1);
    /// assert_eq!(diagnostics[0].severity(), Severity::Warning);
    /// # }
    /// # Ok::<(), molangx::compile::CompileError>(())
    /// ```
    pub fn into_result(self) -> Result<(Expr, Vec<Diagnostic>), CompileError> {
        let failure = match self.outcome {
            CompileOutcome::Success(expr) => return Ok((expr, self.diagnostics)),
            CompileOutcome::Rejected(_) => CompileFailure::Rejected,
            CompileOutcome::UsesArrays => CompileFailure::UsesArrays,
            CompileOutcome::UsesResources => CompileFailure::UsesResources,
        };
        Err(CompileError {
            failure,
            diagnostics: self.diagnostics,
        })
    }

    /// Whether the outcome is not a rejection and no language message was logged, including any
    /// the diagnostic limit dropped.
    pub const fn parses_cleanly(&self) -> bool {
        !matches!(self.outcome, CompileOutcome::Rejected(_)) && !self.logged
    }

    /// Whether the core parse succeeded (the tree was built, optimised and validated), whatever
    /// the link stage then decided.
    pub const fn parsed(&self) -> bool {
        self.tree.is_some()
    }

    /// The optimised tree in the parse-tree test notation, float leaves with `float_digits`
    /// significant digits.
    ///
    /// Test support, built only with the `fuzz` feature, outside the SemVer contract.
    #[cfg(any(feature = "fuzz", test))]
    #[doc(hidden)]
    pub fn tree_notation(&self, float_digits: usize) -> Option<String> {
        let math = self.expr_or_zero().and_then(Expr::math);
        self.tree
            .as_deref()
            .map(|tree| tree.tree_notation_in(float_digits, math))
    }
}

/// A compiled expression. Cheap to clone and share.
#[derive(Clone)]
pub struct Expr {
    inner: Arc<ExprInner>,
}

struct ExprInner {
    body: Body,
    raw_version: RawVersion,
    source: Option<Arc<str>>,
}

enum Body {
    /// No program, so it keeps the catalogues a program would carry.
    Rejected {
        catalog: QueryCatalog,
        math: Option<MathCatalog>,
    },
    /// A tree that folded to a float; the program loads it.
    Constant {
        value: f32,
        tree: Arc<Node>,
        program: Program,
    },
    Program {
        tree: Arc<Node>,
        program: Program,
        /// Distinct, in the order of their first call.
        queries: Box<[QueryIndex]>,
    },
}

#[cfg(feature = "vm")]
pub(crate) enum Evaluation<'a> {
    Constant(f32),
    Program(&'a Program),
}

impl Expr {
    pub(super) fn rejected(
        opts: &CompileOptions,
        raw_version: RawVersion,
        source: Option<Arc<str>>,
    ) -> Self {
        Self {
            inner: Arc::new(ExprInner {
                body: Body::Rejected {
                    catalog: opts.catalog.clone(),
                    math: opts.math.clone(),
                },
                raw_version,
                source,
            }),
        }
    }

    pub(super) fn new(
        tree: Arc<Node>,
        program: Program,
        raw_version: RawVersion,
        source: Option<Arc<str>>,
    ) -> Self {
        // The `Float` instruction loads the payload without a post-op, so the folded value stands.
        let body = if tree.is(Op::Float) {
            Body::Constant {
                value: tree.float(),
                tree,
                program,
            }
        } else {
            let mut seen = HashSet::with_capacity(program.calls.len());
            let queries = program
                .calls
                .iter()
                .map(|call| call.index)
                .filter(|index| seen.insert(*index))
                .collect();
            Body::Program {
                tree,
                program,
                queries,
            }
        };
        Self {
            inner: Arc::new(ExprInner {
                body,
                raw_version,
                source,
            }),
        }
    }

    /// The value of an expression that folded to a float (a rejected expression is 0).
    pub fn as_constant(&self) -> Option<f32> {
        match self.inner.body {
            Body::Rejected { .. } => Some(0.0),
            Body::Constant { value, .. } => Some(value),
            Body::Program { .. } => None,
        }
    }

    /// Whether this is the constant 0 of a rejected text, as opposed to one that compiled (a
    /// literal `0` included).
    pub fn is_rejected(&self) -> bool {
        matches!(self.inner.body, Body::Rejected { .. })
    }

    /// Whether the expression folded to a constant: a float or a string hash (a rejected expression
    /// is 0).
    pub fn is_constant(&self) -> bool {
        match &self.inner.body {
            Body::Rejected { .. } | Body::Constant { .. } => true,
            Body::Program { tree, .. } => {
                tree.is(Op::StringLiteral) && matches!(tree.value, Payload::Hash(_))
            }
        }
    }

    /// The source text, when the expression was compiled with
    /// [`keep_source`](CompileOptions::keep_source).
    pub fn source(&self) -> Option<&str> {
        self.inner.source.as_deref()
    }

    /// The version the parser rules were applied at:
    /// [`RawVersion::effective`](crate::version::RawVersion::effective) of [`Expr::raw_version`].
    pub fn version(&self) -> MolangVersion {
        self.inner.raw_version.effective()
    }

    /// The raw version the queries resolved against (−1 for a plain-string source whose context
    /// version was never applied).
    pub fn raw_version(&self) -> RawVersion {
        self.inner.raw_version
    }

    /// The catalogue the expression was compiled against.
    pub fn catalog(&self) -> &QueryCatalog {
        match &self.inner.body {
            Body::Rejected { catalog, .. } => catalog,
            Body::Constant { program, .. } | Body::Program { program, .. } => &program.catalog,
        }
    }

    /// The host math functions the expression was compiled with
    /// ([`CompileOptions::math`](crate::compile::CompileOptions::math)).
    pub fn math(&self) -> Option<&MathCatalog> {
        match &self.inner.body {
            Body::Rejected { math, .. } => math.as_ref(),
            Body::Constant { program, .. } | Body::Program { program, .. } => program.math.as_ref(),
        }
    }

    /// The full names of the queries the expression calls, each once, in the order of their first
    /// call. A rejected expression calls none.
    pub fn queries(&self) -> impl Iterator<Item = &str> + '_ {
        let queries: &[QueryIndex] = match &self.inner.body {
            Body::Program { queries, .. } => queries,
            Body::Rejected { .. } | Body::Constant { .. } => &[],
        };
        queries
            .iter()
            .map(|index| self.catalog().decl(*index).name())
    }

    /// What the compiled program does. A constant, a rejected expression included, is
    /// [`ProgramFlags::CONSTANT`] and [`ProgramFlags::FLOAT_ONLY`].
    pub fn flags(&self) -> ProgramFlags {
        match &self.inner.body {
            Body::Rejected { .. } | Body::Constant { .. } => {
                ProgramFlags::CONSTANT.union(ProgramFlags::FLOAT_ONLY)
            }
            Body::Program { program, .. } => program.flags,
        }
    }

    #[cfg(feature = "vm")]
    pub(crate) fn evaluation(&self) -> Evaluation<'_> {
        match &self.inner.body {
            Body::Rejected { .. } => Evaluation::Constant(0.0),
            Body::Constant { value, .. } => Evaluation::Constant(*value),
            Body::Program { program, .. } => Evaluation::Program(program),
        }
    }

    #[cfg(any(feature = "fuzz", test))]
    pub(crate) fn program(&self) -> Option<&Program> {
        match &self.inner.body {
            Body::Rejected { .. } => None,
            Body::Constant { program, .. } | Body::Program { program, .. } => Some(program),
        }
    }

    pub(crate) fn tree(&self) -> Option<&Node> {
        match &self.inner.body {
            Body::Rejected { .. } => None,
            Body::Constant { tree, .. } | Body::Program { tree, .. } => Some(tree),
        }
    }

    /// The program as text, one instruction per line; the format is not stable.
    ///
    /// Test support, built only with the `fuzz` feature, outside the SemVer contract.
    #[cfg(any(feature = "fuzz", test))]
    #[doc(hidden)]
    pub fn disassemble(&self) -> String {
        self.program()
            .map_or_else(|| "rejected: constant 0\n".to_owned(), Program::disassemble)
    }

    /// Whether the expression contains an assignment.
    pub fn assigns(&self) -> bool {
        self.tree()
            .is_some_and(|t| sema::contains_op(t, |op| op == Op::Assignment))
    }

    /// Whether evaluating the expression can change state: it contains an assignment or, when
    /// `include_random`, `math.random` / `math.random_integer` / a volatile host math function.
    /// Queries and `math.die_roll*` are not side effects.
    pub fn has_side_effects(&self, include_random: bool) -> bool {
        self.tree().is_some_and(|t| {
            sema::contains_op(t, |op| {
                op == Op::Assignment
                    || (include_random
                        && matches!(op, Op::Random | Op::RandomInt | Op::HostMathVolatile))
            })
        })
    }
}

impl fmt::Debug for Expr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Expr")
            .field("constant", &self.as_constant())
            .field("rejected", &self.is_rejected())
            .field("version", &self.version())
            .field(
                "tree",
                &self.tree().map(|t| t.tree_notation_in(9, self.math())),
            )
            .finish_non_exhaustive()
    }
}

/// A compile that produced no expression ([`Compiled::into_result`]): the kind of failure and
/// everything the compile logged.
///
/// Its text names the failure and, for a rejection, the first error (or the first diagnostic when
/// none is an error).
#[derive(Clone, Debug, PartialEq)]
pub struct CompileError {
    failure: CompileFailure,
    diagnostics: Vec<Diagnostic>,
}

impl CompileError {
    /// How the compile failed.
    pub const fn failure(&self) -> CompileFailure {
        self.failure
    }

    /// Everything the compile logged, in order.
    pub fn diagnostics(&self) -> &[Diagnostic] {
        &self.diagnostics
    }

    /// The diagnostics, by value.
    pub fn into_diagnostics(self) -> Vec<Diagnostic> {
        self.diagnostics
    }
}

impl fmt::Display for CompileError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.failure {
            CompileFailure::Rejected => {
                f.write_str("the expression was rejected")?;
                let first = self
                    .diagnostics
                    .iter()
                    .find(|d| d.severity() == Severity::Error)
                    .or(self.diagnostics.first());
                match first {
                    Some(diagnostic) => write!(f, ": {diagnostic}"),
                    None => Ok(()),
                }
            }
            CompileFailure::UsesArrays => f.write_str(
                "the expression uses render-controller arrays, which must be resolved before it \
                 is compiled",
            ),
            CompileFailure::UsesResources => f.write_str(
                "the expression uses geometry, material or texture variables, which must be \
                 resolved before it is compiled",
            ),
        }
    }
}

impl std::error::Error for CompileError {}

#[cfg(test)]
mod tests {
    use crate::compile::{
        CompileFailure, CompileOptions, CompileOutcome, Compiled, Cx, Deviations, Expr, ast::Span,
        compile, test_support::*,
    };
    use crate::diag::{Diagnostic, LanguageMessage, Severity};
    use crate::version::RawVersion;

    use crate::compile::program::ProgramFlags;

    use crate::version::MolangVersion;

    use std::sync::Arc;

    fn expr_of(src: &str, options: &CompileOptions) -> Expr {
        compile(src, options)
            .expr_or_zero()
            .cloned()
            .expect("an expression")
    }

    #[test]
    fn queries_lists_each_query_once_in_the_order_of_its_first_call() {
        let src = format!(
            "q.life_time + {}q.is_baby + q.life_time",
            "q.is_baby + q.position(q.life_time) + ".repeat(20)
        );
        let expr = expr_of(&src, &opts());
        assert_eq!(
            expr.queries().collect::<Vec<_>>(),
            ["query.life_time", "query.is_baby", "query.position"]
        );
        assert_eq!(expr_of("1 + v.x", &opts()).queries().count(), 0);
        assert_eq!(
            compile("$", &opts())
                .expr_or_zero()
                .cloned()
                .expect("the failed node")
                .queries()
                .count(),
            0
        );
    }

    #[test]
    fn parsed_is_false_when_the_front_end_rejects_and_true_when_only_the_link_does() {
        assert!(!compile("$", &opts()).parsed());
        assert!(!compile("1 +", &opts()).parsed());
        assert!(compile("1+2", &opts()).parsed());
        let arrays = compile("array.a[0]", &opts());
        assert_eq!(arrays.failure(), Some(CompileFailure::UsesArrays));
        assert!(arrays.parsed());
        assert!(arrays.expr().is_none());
        assert!(arrays.diagnostics.is_empty());
        let resources = compile("geometry.default", &opts());
        assert_eq!(resources.failure(), Some(CompileFailure::UsesResources));
        assert!(resources.parsed());
        assert!(resources.expr().is_none());
    }

    #[test]
    fn parses_cleanly_needs_a_non_rejected_result_and_no_language_message() {
        assert!(compile("v.x", &opts()).parses_cleanly());
        // A kept message (a malformed exponent) leaves the expression but not the clean parse.
        let kept = compile("1e;", &opts());
        assert_eq!(kept.failure(), None);
        assert!(!kept.parses_cleanly());
        assert!(!compile("$", &opts()).parses_cleanly());
        assert!(compile("array.a[0]", &opts()).parses_cleanly());
    }

    #[test]
    fn compiled_done_collects_the_context() {
        let options = opts();
        let mut cx = Cx::for_test("abc", &options);
        cx.language(LanguageMessage::NoTokens, Span::new(0, 1), &[]);
        let compiled = Compiled::done(cx, CompileOutcome::UsesArrays, None);
        assert_eq!(compiled.diagnostics.len(), 1);
        assert!(compiled.logged);
        assert!(!compiled.parsed());
        assert_eq!(compiled.failure(), Some(CompileFailure::UsesArrays));
        assert!(!compiled.parses_cleanly());
    }

    #[test]
    fn as_constant_and_is_constant() {
        let folded = expr_of("1+2*3", &opts());
        assert_eq!(folded.as_constant(), Some(7.0));
        assert!(folded.is_constant());
        let variable = expr_of("v.x", &opts());
        assert_eq!(variable.as_constant(), None);
        assert!(!variable.is_constant());
        let negative_zero = expr_of("-0", &opts());
        assert_eq!(
            negative_zero.as_constant().map(f32::to_bits),
            Some((-0.0f32).to_bits())
        );
    }

    #[test]
    fn a_comparison_with_a_string_keeps_the_hash_but_is_not_a_constant() {
        let comparison = expr_of("v.x == 'a'", &opts());
        assert!(!comparison.is_constant());
        assert_eq!(comparison.as_constant(), None);
        assert!(!expr_of("'a' != v.x", &opts()).is_constant());
    }

    #[test]
    fn a_string_literal_is_a_constant_without_a_float_value() {
        let string = expr_of("'hello'", &opts());
        assert!(string.is_constant());
        assert_eq!(string.as_constant(), None);
    }

    #[test]
    fn a_rejected_expression_is_the_constant_zero() {
        let failed = expr_of("1 +", &opts());
        assert_eq!(failed.as_constant(), Some(0.0));
        assert!(failed.is_constant());
        assert_eq!(
            failed.flags(),
            ProgramFlags::CONSTANT.union(ProgramFlags::FLOAT_ONLY)
        );
        assert!(failed.program().is_none());
        assert!(failed.disassemble().starts_with("rejected"));
    }

    #[test]
    fn a_rejected_expression_is_not_a_literal_zero() {
        let failed = expr_of("1 +", &opts());
        let zero = expr_of("0", &opts());
        assert_eq!(
            (failed.as_constant(), zero.as_constant()),
            (Some(0.0), Some(0.0))
        );
        assert!(failed.is_rejected());
        assert!(!zero.is_rejected());
        assert!(zero.program().is_some());
        assert!(format!("{failed:?}").contains("rejected: true"));
    }

    #[test]
    fn the_accessors_of_each_outcome() {
        let success = compile("1 + v.x", &opts());
        assert!(matches!(success.outcome(), CompileOutcome::Success(expr) if !expr.is_rejected()));
        assert!(success.is_success());
        assert_eq!(success.failure(), None);
        assert!(success.expr().is_some());
        assert!(success.expr_or_zero().is_some());
        assert_eq!(success.errors().count(), 0);
        let (expr, diagnostics) = success.clone().into_result().expect("a success");
        assert_eq!(expr.source(), success.expr().unwrap().source());
        assert!(diagnostics.is_empty());

        let rejected = compile("1 +", &opts());
        assert!(matches!(rejected.outcome(), CompileOutcome::Rejected(expr) if expr.is_rejected()));
        assert_eq!(rejected.failure(), Some(CompileFailure::Rejected));
        assert!(
            rejected.expr().is_none(),
            "a rejected text has no compiled expression"
        );
        assert_eq!(
            rejected.expr_or_zero().and_then(Expr::as_constant),
            Some(0.0)
        );
        assert_eq!(rejected.errors().count(), 1);
        let error = rejected.clone().into_result().expect_err("rejected");
        assert_eq!(error.failure(), CompileFailure::Rejected);
        assert_eq!(error.diagnostics(), rejected.diagnostics());
        assert_eq!(
            error.to_string(),
            format!("the expression was rejected: {}", rejected.diagnostics()[0])
        );
        assert!(matches!(
            rejected.into_outcome(),
            CompileOutcome::Rejected(_)
        ));

        for (source, failure) in [
            ("array.a[0]", CompileFailure::UsesArrays),
            ("geometry.default", CompileFailure::UsesResources),
        ] {
            let compiled = compile(source, &opts());
            assert_eq!(compiled.failure(), Some(failure), "{source}");
            assert!(
                compiled.expr().is_none() && compiled.expr_or_zero().is_none(),
                "{source}"
            );
            let error = compiled.into_result().expect_err("needs resolution");
            assert_eq!(error.failure(), failure);
            assert!(error.diagnostics().is_empty());
            let _: &dyn std::error::Error = &error;
        }
    }

    #[test]
    fn the_error_text_of_each_failure() {
        let text = |source| {
            compile(source, &opts())
                .into_result()
                .expect_err("a failure")
                .to_string()
        };
        assert_eq!(
            text("1 +"),
            "the expression was rejected: Error: binary Add '+' operator at end of expression"
        );
        assert_eq!(
            text("array.a[0]"),
            "the expression uses render-controller arrays, which must be resolved before it is \
             compiled"
        );
        assert_eq!(
            text("geometry.default"),
            "the expression uses geometry, material or texture variables, which must be resolved \
             before it is compiled"
        );
    }

    #[test]
    fn into_result_keeps_the_diagnostics_of_a_success() {
        let compiled = compile("1e;", &opts());
        assert!(compiled.is_success());
        let logged = compiled.diagnostics().to_vec();
        assert_eq!(logged.len(), 1);
        assert_eq!(logged[0].severity(), Severity::Warning);
        let (expr, diagnostics) = compiled.into_result().expect("a success");
        assert!(!expr.is_rejected());
        assert_eq!(diagnostics, logged);

        let no_deviations = CompileOptions {
            deviations: Deviations::NONE,
            ..opts()
        };
        let kept = compile("loop(2, {break; v.x = 1;});", &no_deviations);
        assert!(kept.is_success());
        let logged = kept.diagnostics().to_vec();
        assert_eq!(
            logged.iter().map(Diagnostic::severity).collect::<Vec<_>>(),
            [Severity::Error]
        );
        let (_, diagnostics) = kept.into_result().expect("a success despite the error");
        assert_eq!(diagnostics, logged);
    }

    #[test]
    fn an_expression_keeps_the_math_catalogue_it_was_compiled_with() {
        use crate::catalog::{Arity, MathCatalog, MathDecl};
        let math =
            MathCatalog::new([MathDecl::pure("math.f", Arity::exactly(1), |a| a[0]).unwrap()])
                .unwrap();
        let options = CompileOptions {
            math: Some(math.clone()),
            ..opts()
        };
        for source in ["v.x", "1 + 2", "1 +"] {
            let compiled = compile(source, &options);
            let expr = compiled.expr_or_zero().expect("an expression");
            assert_eq!(expr.math(), Some(&math), "{source}");
            assert_eq!(
                compile(source, &opts()).expr_or_zero().and_then(Expr::math),
                None,
                "{source}"
            );
        }
    }

    #[test]
    fn an_expression_keeps_its_raw_version() {
        for (raw, version) in [
            (13, MolangVersion::LATEST),
            (99, MolangVersion::LATEST),
            (-7, MolangVersion::Invalid),
            (5, MolangVersion::V5),
        ] {
            for source in ["1", "v.x", "1 +"] {
                let expr = compile(
                    source,
                    &CompileOptions {
                        raw_version: RawVersion(raw),
                        ..opts()
                    },
                )
                .expr_or_zero()
                .cloned()
                .expect("an expression");
                assert_eq!(
                    (expr.raw_version(), expr.version()),
                    (RawVersion(raw), version),
                    "{source} at {raw}"
                );
            }
        }
    }

    #[test]
    fn the_flags_of_a_constant_and_of_a_program() {
        let constant = expr_of("2*3", &opts());
        assert_eq!(
            constant.flags(),
            ProgramFlags::CONSTANT.union(ProgramFlags::FLOAT_ONLY)
        );
        let program = expr_of("v.x=1;", &opts());
        assert!(program.flags().contains(ProgramFlags::HAS_ASSIGNMENT));
        assert!(!program.flags().contains(ProgramFlags::CONSTANT));
        assert!(program.program().is_some());
    }

    #[test]
    fn source_is_kept_only_on_request() {
        assert_eq!(expr_of("v.x + 1", &opts()).source(), None);
        assert_eq!(
            expr_of(
                "v.x + 1",
                &CompileOptions {
                    keep_source: true,
                    ..opts()
                }
            )
            .source(),
            Some("v.x + 1")
        );
        assert_eq!(
            expr_of(
                "1 +",
                &CompileOptions {
                    keep_source: true,
                    ..opts()
                }
            )
            .source(),
            Some("1 +")
        );
        assert_eq!(
            expr_of(
                "",
                &CompileOptions {
                    keep_source: true,
                    ..opts()
                }
            )
            .source(),
            Some("")
        );
    }

    #[test]
    fn the_version_of_an_expression_is_the_effective_version_it_compiled_at() {
        assert_eq!(expr_of("1", &opts()).version(), MolangVersion::LATEST);
        assert_eq!(expr_of("1", &opts_at(5)).version(), MolangVersion::V5);
        assert_eq!(expr_of("1", &opts_at(99)).version(), MolangVersion::LATEST);
        assert_eq!(expr_of("1", &opts_at(-7)).version(), MolangVersion::Invalid);
        assert_eq!(expr_of("$", &opts_at(2)).version(), MolangVersion::V2);
    }

    #[test]
    fn variable_assignments_are_found_anywhere_in_the_tree() {
        for (src, expected) in [
            ("v.x=1;", true),
            ("t.x=1;", true),
            ("v.x.y=1;", true),
            ("loop(2,{t.i=1;});", true),
            ("v.a?1:2", false),
            ("v.x", false),
            ("1", false),
            ("math.random(0,1)", false),
        ] {
            assert_eq!(expr_of(src, &opts()).assigns(), expected, "{src}");
        }
        assert!(!expr_of("1 +", &opts()).assigns());
    }

    #[test]
    fn a_volatile_host_function_is_a_random_side_effect_and_a_pure_one_is_none() {
        let options = math_opts();
        let expr = |src: &str| {
            compile(src, &options)
                .expr()
                .cloned()
                .unwrap_or_else(|| panic!("{src} compiles"))
        };
        let noise = expr("math.noise(v.x)");
        assert!(noise.has_side_effects(true) && !noise.has_side_effects(false));
        let twice = expr("math.twice(v.x)");
        assert!(!twice.has_side_effects(true));
    }

    #[test]
    fn side_effects_are_assignments_and_optionally_the_random_functions() {
        for (src, plain, with_random) in [
            ("v.x=1;", true, true),
            ("loop(2,{t.i=1;});", true, true),
            ("math.random(0,1)", false, true),
            ("math.random_integer(0,5)", false, true),
            ("v.a?math.random(0,1):2", false, true),
            ("math.die_roll(1,2,3)", false, false),
            ("math.die_roll_integer(1,2,3)", false, false),
            ("q.is_baby", false, false),
            ("v.x", false, false),
            ("1", false, false),
        ] {
            let expr = expr_of(src, &opts());
            assert_eq!(expr.has_side_effects(false), plain, "{src}");
            assert_eq!(expr.has_side_effects(true), with_random, "{src}");
        }
        let failed = expr_of("1 +", &opts());
        assert!(!failed.has_side_effects(true));
    }

    #[test]
    fn a_cloned_expression_shares_the_compiled_state() {
        let expr = expr_of(
            "v.x*2",
            &CompileOptions {
                keep_source: true,
                ..opts()
            },
        );
        let copy = expr.clone();
        assert_eq!(copy.source(), expr.source());
        assert_eq!(copy.version(), expr.version());
        assert_eq!(copy.disassemble(), expr.disassemble());
        assert!(Arc::ptr_eq(&copy.inner, &expr.inner));
    }

    #[test]
    fn the_debug_form_of_an_expression_shows_the_constant_the_version_and_the_tree() {
        let debug = format!("{:?}", expr_of("v.x*2+1", &opts()));
        assert!(debug.starts_with("Expr {"), "{debug}");
        assert!(debug.contains("constant: None"), "{debug}");
        assert!(debug.contains("[v.x*2+1]"), "{debug}");
        let constant = format!("{:?}", expr_of("1+2", &opts()));
        assert!(constant.contains("constant: Some(3.0)"), "{constant}");
        let failed = format!("{:?}", expr_of("1 +", &opts()));
        assert!(
            failed.contains("constant: Some(0.0)") && failed.contains("tree: None"),
            "{failed}"
        );
    }

    #[test]
    fn the_tree_notation_of_a_compile_is_the_optimised_tree() {
        assert_eq!(
            compile("v.x*2+1", &opts()).tree_notation(9).as_deref(),
            Some("[v.x*2+1]")
        );
        assert_eq!(
            compile("1+2", &opts()).tree_notation(9).as_deref(),
            Some("3")
        );
        assert_eq!(compile("$", &opts()).tree_notation(9), None);
    }
}
