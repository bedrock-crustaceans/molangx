//! Compile diagnostics: the parser's messages and the lints.
//!
//! A diagnostic whose [`Diagnostic::language_message`] is `Some` is a parser log line; its
//! [`Severity`] says what it did to the expression (reject or only log).

use std::borrow::Cow;
use std::fmt::{self, Write as _};
use std::ops::Range;
use std::sync::Arc;

/// What a diagnostic is about.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum DiagCode {
    /// A lexer error: an unknown token, an unterminated string, a malformed exponent, an empty
    /// expression.
    Syntax,
    /// A query that does not resolve: unknown name, outside the allowed query sets or allow-list,
    /// not available in this version, or behind a disabled experiment. All log the one text #5.
    UnknownQuery,
    /// Lint (warning): a query called with an argument count outside its declared range. The
    /// call still runs.
    QueryArity,
    /// A query behind an experiment that is not enabled; no standard query is behind one.
    QueryExperiment,
    /// Lint (info): next to [`DiagCode::UnknownQuery`] when the name exists in another version
    /// range.
    QueryDeprecated,
    /// Lint (info): a client-only query compiled for the server side.
    QueryClientOnly,
    /// An operation the compile options do not allow in this context.
    InvalidOperation,
    /// A string, resource or value-less construct used where a number is required.
    StringMisuse,
    /// The assignment, pointer (`->`) and `??` rules.
    InvalidAssignment,
    /// Tree-building and statement-shape errors: `;` termination, sections, argument lists,
    /// operators without operands.
    StatementForm,
    /// The nesting-depth limit ([`MAX_DEPTH`](crate::compile::MAX_DEPTH)).
    DepthLimit,
    /// Lint (error): the source is longer than [`MAX_SOURCE_LEN`](crate::compile::MAX_SOURCE_LEN).
    SourceTooLong,
    /// Lint (warning, info for `Invalid`): a version outside −1..=13, or the `Invalid` version.
    InvalidVersion,
    /// Lint (info): the number of diagnostics the per-compile limit
    /// ([`MAX_DIAGNOSTICS`](crate::compile::MAX_DIAGNOSTICS)) left out.
    DiagnosticLimit,
}

/// How serious a diagnostic is.
///
/// Exhaustive: the set will not grow.
#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Severity {
    /// Informational; the expression is unaffected.
    Info,
    /// The expression compiles and runs; the message is only logged.
    Warning,
    /// The expression is rejected and evaluates to 0.
    Error,
}

/// What a parser message does to the expression.
///
/// Exhaustive: the set will not grow.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum Effect {
    /// The parse fails; the expression evaluates to 0.
    Reject,
    /// The message is logged and the expression compiles.
    Keep,
    /// Rejects when detected at the root node, is only logged when detected below it.
    RootOnly,
}

impl Effect {
    /// The severity a message with this effect is logged at; the validator picks its own for a
    /// [`Effect::RootOnly`] message below the root.
    pub(crate) const fn severity(self) -> Severity {
        match self {
            Self::Reject | Self::RootOnly => Severity::Error,
            Self::Keep => Severity::Warning,
        }
    }
}

macro_rules! language_messages {
    ($( $(#[$doc:meta])* $name:ident = ($row:literal, $code:ident, $effect:ident, $template:literal), )*) => {
        /// One of the parser's content-log messages.
        ///
        /// A table row with several texts has one variant per text.
        #[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
        #[non_exhaustive]
        pub enum LanguageMessage {
            $( $(#[$doc])* $name, )*
        }

        impl LanguageMessage {
            /// Every message, in table order.
            pub const ALL: &'static [Self] = &[$(Self::$name,)*];

            /// The row of the parser-message table (1…49).
            pub const fn row(self) -> u8 {
                match self { $(Self::$name => $row,)* }
            }

            /// The template: the logged text with `{}` where an argument goes and `{{` / `}}` for a
            /// literal brace, including any trailing newline.
            pub const fn template(self) -> &'static str {
                match self { $(Self::$name => $template,)* }
            }

            /// The [`DiagCode`] the message is classified under.
            pub const fn code(self) -> DiagCode {
                match self { $(Self::$name => DiagCode::$code,)* }
            }

            /// What the message does to the expression.
            pub const fn effect(self) -> Effect {
                match self { $(Self::$name => Effect::$effect,)* }
            }
        }
    };
}

language_messages! {
    /// #1 — empty or whitespace-only input, from version 4; below it the expression is rejected
    /// without a message.
    NoTokens = (1, Syntax, Reject, "No tokens found in expression"),
    /// #2 — a token failed; the argument is the rest of the input.
    UnrecognizedToken = (2, Syntax, Reject, "unrecognized token: {}"),
    /// #3 — an identifier or character that is no token, a `true` / `false` prefix, or a number;
    /// the argument is the rest of the input.
    UnknownToken = (3, Syntax, Reject, "Error: unknown token: {}"),
    /// #4 — an unterminated `'…'` string.
    StringMissingQuote = (4, Syntax, Reject, "Error: Molang string missing final ' character"),
    /// #5 — a query that does not resolve in this context: unknown, outside the allowed query sets
    /// or allow-list, not available at this version, or behind an experiment that is off.
    QueryUnresolved = (5, UnknownQuery, Reject, "Failed to resolve query {}.  Either the query does not exist or it is not supported in this context."),
    /// #6 — `e` not followed by a sign or digit; the literal becomes 0 and the parse goes on.
    BadExponent = (6, Syntax, Keep, "error parsing float string, expected '+' or '-' after 'e': {}"),
    /// #7 — an expression containing `=` or `;` whose last token is not `;`.
    ComplexMustEndWithSemicolon = (7, StatementForm, Reject, "Error: complex expressions (contains either '=' or ';') must end with a ';'\n"),
    /// #8 — more than one root after all grouping passes; the argument lists the remaining tokens.
    MultipleRoots = (8, StatementForm, Reject, "found multiple operations without a combining operation between them:\n{}"),
    /// #9 — a leading `.name`.
    LeadingMemberAccessor = (9, StatementForm, Reject, "Error: cannot start an expression with a member accessor; member accessors require a variable of which to access a member."),
    /// #10 — an unmatched `(`, `[` or `{`; the first argument is the opener's position. #12
    /// follows.
    NoClosingSymbol = (10, StatementForm, Reject, "Unable to find matching closing section symbol for symbol at {}({}) -- looking for {}"),
    /// #11 — a section closed by a different closer (`(]`).
    ClosingMismatch = (11, StatementForm, Reject, "Unable to match closing section symbol at {}({}) - looking for {}, found {} at {}"),
    /// #12 — follows #10 / #11.
    SectionNotClosed = (12, StatementForm, Reject, "Error: Could not find {} to close section started with {}\n"),
    /// #13 — `q.x()`, `loop()`.
    EmptyParameterList = (13, StatementForm, Reject, "Error: {} operators with no params should not use parentheses\n"),
    /// #14 — a leading `;`.
    LeadingSemicolon = (14, StatementForm, Reject, "Error: expressions can't begin with a semicolon\n"),
    /// #15 — an empty index section: `array.x[ ]`.
    EmptyArrayIndex = (15, StatementForm, Reject, "Error: array expression is empty\n"),
    /// #16 — a binary operator without an operand on one side (also `+1`, `--1`).
    BinaryAtEnd = (16, StatementForm, Reject, "Error: binary {} operator at end of expression\n"),
    /// #17 — a math function as the last token.
    MathAtEnd = (17, StatementForm, Reject, "Error: {} operator at end of expression without a parenthesis section\n"),
    /// #17 — a math function not followed by `(`.
    MathWithoutParenthesis = (17, StatementForm, Reject, "Error: {} operator not followed by parenthesis section\n"),
    /// #18 — a trailing `-`.
    NegateWithoutOperand = (18, StatementForm, Reject, "Error: '-' not followed by expression\n"),
    /// #18 — a trailing `!`.
    NotWithoutOperand = (18, StatementForm, Reject, "Error: logical-not ('!') must be followed by expression\n"),
    /// #18 — a `-` after `continue` or after an expression array.
    UnknownOperation = (18, StatementForm, Reject, "Error: unknown {} operation in expression\n"),
    /// #19 — a dangling `?` / `:`, from version 5; below it #16 is logged instead.
    TernaryWithoutOperands = (19, StatementForm, Reject, "Error: could not find sub-expressions for {} operator\n"),
    /// #20 — `return;`.
    UnaryWithoutOperand = (20, StatementForm, Reject, "Error: unary {} operator not followed by expression\n"),
    /// #21 — an operation outside the allowed set.
    OperationNotAllowed = (21, InvalidOperation, Reject, "Expression uses operation {} which is not allowed in this context"),
    /// #22 — more than 255 nested sub-expressions.
    DepthOverflow = (22, DepthLimit, Reject, "Error: Expression could not be parsed due to stack depth overflow (too many sub-expressions)"),
    /// #23 — a three-argument math function with another argument count.
    ParameterCount3 = (23, StatementForm, Reject, "Unexpected number of parameters to {} function - expected 3, found {}.\n"),
    /// #23 — a two-argument math function (`math.max`, `math.min`, `math.mod` and `math.pow`
    /// included) with another argument count.
    ParameterCount2 = (23, StatementForm, Reject, "Unexpected number of parameters to {} function - expected 2, found {}.\n"),
    /// #24 — an operator with too few or too many operands (`()`, `{}`, `math.sin()`,
    /// `array.x[1][2]`); an unbounded maximum prints as `-1`.
    Malformed = (24, StatementForm, Reject, "Malformed {} expression. It has {} children but should have between {} and {}"),
    /// #25 — a stray `:` (a nested conditional at version 4).
    UnsupportedInOptimization = (25, StatementForm, Reject, "Unsupported {} operator in expression optimization"),
    /// #26 — a `,` outside an argument list.
    UnexpectedComma = (26, StatementForm, Reject, "Error: Unexpected {} operator not inside an arguments list for a query, loop, or math function"),
    /// #27 — `for_each` with another number of arguments than three.
    ForEachParameters = (27, StatementForm, Reject, "Error: for_each requires three parameters - a variable to represent an element of an array, an expression resulting in an array, and an expression to run per element of that array."),
    /// #28 — an `=` that is a direct statement of a `;` list (at the root or in a brace block) and
    /// assigns to something that is not a variable or member. An `=` under `return`, `?:` or
    /// parentheses is not checked here (`return v.x + 1 = 2;` parses without a message).
    AssignToNonVariable = (28, InvalidAssignment, Reject, "Error: assignment to non-variable not allowed. Expression is trying to assign to a: {}"),
    /// #29 — a conditional without a condition: a `:` before its `?` (`1 : 2 ? 3`).
    ConditionalWithoutIf = (29, StatementForm, Reject, "Error: '?' operator couldn't find a valid preceding 'if' expression"),
    /// #30 — `loop` with another number of arguments than two.
    LoopParameters = (30, StatementForm, Reject, "Error: loop requires two parameters - an expression resulting in a number of times to loop, and a {{}}-delimited expression to loop."),
    /// #31 — a statement that is not one node (`v.x.1 = 1;`).
    StatementNotSingle = (31, StatementForm, Reject, "Error: Could not reduce sub-expression before a semicolon to a single operation to evaluate"),
    /// #32 — `(a b)` or `[a b]`, from version 4.
    SectionChildCount = (32, StatementForm, Reject, "Error: {} optimization expected only one child operation but found {}"),
    /// #33 — a `for_each` iteration variable that is not `v.` / `t.`.
    ForEachVariable = (33, StatementForm, Reject, "Error: for_each expressions require either a temp or entity variable as the iteration variable (the first parameter)"),
    /// #34 — `{1}`.
    BraceWithoutStatements = (34, StatementForm, Reject, "Brace sections must only contain semicolon-delimited expressions, even if only one expression is contained.\n"),
    /// #35 — a bare `loop` / `for_each`.
    ParametersNotOneChild = (35, StatementForm, Reject, "{} operator should have exactly one child (a left-parenthesis expression with the params as children of it) prior to optimization."),
    /// #35 — `math.max()`.
    ParametersEmpty = (35, StatementForm, Reject, "{} operator (math, query, loop, etc) with empty parameter list should have failed to parse"),
    /// #35 — `math.max(1,,2)`. A trailing or leading comma (`f(1,)`, `f(,1)`) fails earlier, with
    /// #16.
    ParametersDanglingComma = (35, StatementForm, Reject, "Error while optimizing parameters for {} operation: comma found without a following expression."),
    /// #36 — a string, resource, `loop`, `for_each` or assignment as the operand of an operator or
    /// math function, from version 3; below it the operand is used as it is.
    NonNumericalArgument = (36, StringMisuse, Reject, "'{}' expression cannot take a '{}' argument. It only supports numerical arguments."),
    /// #37 — a query that does not return a number as an arithmetic operand.
    QueryNotNumerical = (37, StringMisuse, Reject, "{} expressions may only contain query functions that return numbers"),
    /// #38 — a statement after `return` / `break` / `continue` in the same list.
    Unreachable = (38, StatementForm, RootOnly, "Error: unreachable statements after {}."),
    /// #39 — `t.x.y = …`.
    TempLhsNotAlone = (39, InvalidAssignment, RootOnly, "Error: left side of an assignment expression can only use temp variables if they are on their own and not part of a more complicated expression."),
    /// #40 — an operator in the left side of an assignment (a left side that is not a variable).
    OperatorOnLhs = (40, InvalidAssignment, RootOnly, "Error: cannot use {} operators on the left side of an assignment expression"),
    /// #41 — `A->B->C`.
    NestedPointer = (41, InvalidAssignment, RootOnly, "Error: nested pointer statements (eg: A->B->C) are not yet supported.  Store A->B in a variable (eg: D), then use D->C"),
    /// #42 — arithmetic on an array element, `array.x[i]`.
    MathOnArray = (42, StringMisuse, RootOnly, "Error: can't currently do math operations on resource array results"),
    /// #43 — `A->B = x`.
    AssignToPointer = (43, InvalidAssignment, RootOnly, "Error: Assignment attempted on Pointer result. Writing to another entity's public variables is not supported, only reading them."),
    /// #44 — a `??` whose left side is not a direct variable.
    CoalesceLhs = (44, InvalidAssignment, RootOnly, "Error: found left-hand-side of ?? expression that isn't a direct-variable reference - this is unsupported at this time."),
    /// #45 — `break` outside `loop` / `for_each`.
    BreakOutsideLoop = (45, StatementForm, RootOnly, "Error: break encountered outside of loop"),
    /// #46 — a `->` whose right side is not an entity variable or query.
    PointerRhs = (46, InvalidAssignment, RootOnly, "Error: right-hand-side of pointer expression did not evaluate to an entity variable or query function"),
    /// #47 — an assignment whose target is not a variable or member.
    AssignmentForm = (47, InvalidAssignment, RootOnly, "Error: assignment expressions may either be of the form 'A->B = D' or 'C = D' where B and C may be an entity variable or member accessor, or C may be a temp variable.  Found an expression where C is a {}"),
    /// #48 — the expression parsed but cannot be evaluated (an assignment target the checks kept);
    /// the argument is the source text.
    CompileFailed = (48, StatementForm, Reject, "expression '{}' compile failed"),
    /// #49 — an assignment through `->`.
    WriteToOtherMob = (49, InvalidAssignment, Reject, "Error: You cannot write to a variable on another mob."),
}

impl LanguageMessage {
    /// The id of the row, `E01`…`E49`.
    pub const fn id(self) -> &'static str {
        const IDS: [&str; 49] = [
            "E01", "E02", "E03", "E04", "E05", "E06", "E07", "E08", "E09", "E10", "E11", "E12",
            "E13", "E14", "E15", "E16", "E17", "E18", "E19", "E20", "E21", "E22", "E23", "E24",
            "E25", "E26", "E27", "E28", "E29", "E30", "E31", "E32", "E33", "E34", "E35", "E36",
            "E37", "E38", "E39", "E40", "E41", "E42", "E43", "E44", "E45", "E46", "E47", "E48",
            "E49",
        ];
        IDS[self.row() as usize - 1]
    }

    /// Formats the template with `args`, one per `{}` in order; `{{` and `}}` print one brace.
    pub(crate) fn format(self, args: &[&dyn fmt::Display]) -> String {
        let template = self.template();
        let mut out = String::with_capacity(template.len() + 16 * args.len());
        let mut args = args.iter();
        let mut rest = template;
        while let Some(at) = rest.find(['{', '}']) {
            let (text, tail) = rest.split_at(at);
            out.push_str(text);
            if tail.starts_with("{}") {
                if let Some(arg) = args.next() {
                    let _ = write!(out, "{arg}");
                }
            } else {
                // `{{` or `}}`: one literal brace.
                out.push_str(&tail[..1]);
            }
            rest = tail.get(2..).unwrap_or_default();
        }
        out.push_str(rest);
        out
    }

    /// Whether the template is the logged text itself: no argument and no doubled brace.
    fn is_plain(self) -> bool {
        !self.template().contains(['{', '}'])
    }
}

/// One compile diagnostic.
///
/// `Display` drops the trailing newline some templates carry. Cloning is cheap: a diagnostic that
/// quotes the source shares the compile's copy and formats its text when read.
#[derive(Clone)]
pub struct Diagnostic {
    severity: Severity,
    span: Range<u32>,
    kind: Kind,
}

#[derive(Clone)]
enum Kind {
    Lint {
        code: DiagCode,
        text: Cow<'static, str>,
    },
    Language {
        message: LanguageMessage,
        text: LanguageText,
    },
}

/// The text of a language message.
///
/// A message quoting the rest of the input (#2, #3, #6) or the source (#48) holds a byte range into
/// the compile's one shared source copy, so a compile's diagnostics hold the source once.
#[derive(Clone)]
enum LanguageText {
    /// The template is the text: it has no placeholder and no doubled brace.
    Template,
    /// The template formatted with its arguments.
    Formatted(Box<str>),
    /// A one-argument template whose argument is `source[range]`, formatted when read.
    Quoting { source: Arc<str>, range: Range<u32> },
}

impl LanguageText {
    fn read(&self, message: LanguageMessage) -> Cow<'_, str> {
        match self {
            Self::Template => Cow::Borrowed(message.template()),
            Self::Formatted(text) => Cow::Borrowed(text),
            Self::Quoting { source, range } => {
                // The bytes skipped after a `true` / `false` prefix (`true_false_prefix_advance`
                // off) can end inside a multi-byte character; such a tail prints lossily.
                let bytes = source
                    .as_bytes()
                    .get(range.start as usize..range.end as usize)
                    .unwrap_or_default();
                Cow::Owned(message.format(&[&String::from_utf8_lossy(bytes)]))
            }
        }
    }
}

impl Diagnostic {
    /// A lint: a diagnostic of this crate's own, outside the language-message table.
    pub(crate) fn lint(
        code: DiagCode,
        severity: Severity,
        span: Range<u32>,
        text: impl Into<Cow<'static, str>>,
    ) -> Self {
        let text = match text.into() {
            // Without spare capacity: a compile can keep many diagnostics.
            Cow::Owned(text) => Cow::Owned(text.into_boxed_str().into_string()),
            borrowed @ Cow::Borrowed(_) => borrowed,
        };
        Self {
            severity,
            span,
            kind: Kind::Lint { code, text },
        }
    }

    /// A language message with `args` (see [`LanguageMessage::format`]); a template that is its
    /// own text is not copied.
    pub(crate) fn language(
        message: LanguageMessage,
        severity: Severity,
        span: Range<u32>,
        args: &[&dyn fmt::Display],
    ) -> Self {
        let text = if message.is_plain() {
            LanguageText::Template
        } else {
            LanguageText::Formatted(message.format(args).into_boxed_str())
        };
        Self {
            severity,
            span,
            kind: Kind::Language { message, text },
        }
    }

    /// A language message whose only argument is `source[range]`.
    pub(crate) fn quoting(
        message: LanguageMessage,
        severity: Severity,
        span: Range<u32>,
        source: Arc<str>,
        range: Range<u32>,
    ) -> Self {
        Self {
            severity,
            span,
            kind: Kind::Language {
                message,
                text: LanguageText::Quoting { source, range },
            },
        }
    }

    /// What the message did: reject ([`Severity::Error`]), log and keep ([`Severity::Warning`]), or
    /// inform.
    pub fn severity(&self) -> Severity {
        self.severity
    }

    /// Byte range in the source the message is about; the whole source when the message has no
    /// position.
    pub fn span(&self) -> Range<u32> {
        self.span.clone()
    }

    /// The classification: the language message's code, or the lint's own.
    pub const fn code(&self) -> DiagCode {
        match self.kind {
            Kind::Lint { code, .. } => code,
            Kind::Language { message, .. } => message.code(),
        }
    }

    /// The parser message; `None` for a lint. An expression that logs one does not parse
    /// cleanly.
    pub const fn language_message(&self) -> Option<LanguageMessage> {
        match self.kind {
            Kind::Lint { .. } => None,
            Kind::Language { message, .. } => Some(message),
        }
    }

    /// The text, with the template's trailing newline where it has one.
    ///
    /// Formatted on each call for a message that quotes the source.
    pub fn message(&self) -> Cow<'_, str> {
        match &self.kind {
            Kind::Lint { text, .. } => Cow::Borrowed(text),
            Kind::Language { message, text } => text.read(*message),
        }
    }
}

/// Equal when message or lint code, severity, span and text are, whatever the text's
/// representation.
impl PartialEq for Diagnostic {
    fn eq(&self, other: &Self) -> bool {
        self.language_message() == other.language_message()
            && self.code() == other.code()
            && self.severity == other.severity
            && self.span == other.span
            && self.message() == other.message()
    }
}

impl fmt::Debug for Diagnostic {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Diagnostic")
            .field("code", &self.code())
            .field("severity", &self.severity)
            .field("span", &self.span)
            .field("language_message", &self.language_message())
            .field("text", &self.message())
            .finish_non_exhaustive()
    }
}

impl fmt::Display for Diagnostic {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.message().trim_end())
    }
}

/// A float with six decimals, as messages print floats; `inf` / `nan` are lower case and carry the
/// value's sign.
pub(crate) fn fixed6(value: f32) -> String {
    if value.is_finite() {
        return format!("{:.6}", f64::from(value));
    }
    let sign = if value.is_sign_negative() { "-" } else { "" };
    let name = if value.is_nan() { "nan" } else { "inf" };
    format!("{sign}{name}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    type Msg = LanguageMessage;

    fn placeholder_count(message: Msg) -> usize {
        message
            .template()
            .replace("{{", "")
            .replace("}}", "")
            .matches("{}")
            .count()
    }

    fn quoting(message: Msg, source: &str, start: u32, end: u32) -> Diagnostic {
        Diagnostic::quoting(
            message,
            Severity::Error,
            0..source.len() as u32,
            Arc::from(source),
            start..end,
        )
    }

    fn lint(text: &'static str) -> Diagnostic {
        Diagnostic::lint(DiagCode::Syntax, Severity::Error, 0..1, text)
    }

    const PINNED: [(Msg, &str); 55] = [
        (Msg::NoTokens, "No tokens found in expression"),
        (Msg::UnrecognizedToken, "unrecognized token: {}"),
        (Msg::UnknownToken, "Error: unknown token: {}"),
        (
            Msg::StringMissingQuote,
            "Error: Molang string missing final ' character",
        ),
        (
            Msg::QueryUnresolved,
            "Failed to resolve query {}.  Either the query does not exist or it is not supported in this context.",
        ),
        (
            Msg::BadExponent,
            "error parsing float string, expected '+' or '-' after 'e': {}",
        ),
        (
            Msg::ComplexMustEndWithSemicolon,
            "Error: complex expressions (contains either '=' or ';') must end with a ';'\n",
        ),
        (
            Msg::MultipleRoots,
            "found multiple operations without a combining operation between them:\n{}",
        ),
        (
            Msg::LeadingMemberAccessor,
            "Error: cannot start an expression with a member accessor; member accessors require a variable of which to access a member.",
        ),
        (
            Msg::NoClosingSymbol,
            "Unable to find matching closing section symbol for symbol at {}({}) -- looking for {}",
        ),
        (
            Msg::ClosingMismatch,
            "Unable to match closing section symbol at {}({}) - looking for {}, found {} at {}",
        ),
        (
            Msg::SectionNotClosed,
            "Error: Could not find {} to close section started with {}\n",
        ),
        (
            Msg::EmptyParameterList,
            "Error: {} operators with no params should not use parentheses\n",
        ),
        (
            Msg::LeadingSemicolon,
            "Error: expressions can't begin with a semicolon\n",
        ),
        (Msg::EmptyArrayIndex, "Error: array expression is empty\n"),
        (
            Msg::BinaryAtEnd,
            "Error: binary {} operator at end of expression\n",
        ),
        (
            Msg::MathAtEnd,
            "Error: {} operator at end of expression without a parenthesis section\n",
        ),
        (
            Msg::MathWithoutParenthesis,
            "Error: {} operator not followed by parenthesis section\n",
        ),
        (
            Msg::NegateWithoutOperand,
            "Error: '-' not followed by expression\n",
        ),
        (
            Msg::NotWithoutOperand,
            "Error: logical-not ('!') must be followed by expression\n",
        ),
        (
            Msg::UnknownOperation,
            "Error: unknown {} operation in expression\n",
        ),
        (
            Msg::TernaryWithoutOperands,
            "Error: could not find sub-expressions for {} operator\n",
        ),
        (
            Msg::UnaryWithoutOperand,
            "Error: unary {} operator not followed by expression\n",
        ),
        (
            Msg::OperationNotAllowed,
            "Expression uses operation {} which is not allowed in this context",
        ),
        (
            Msg::DepthOverflow,
            "Error: Expression could not be parsed due to stack depth overflow (too many sub-expressions)",
        ),
        (
            Msg::ParameterCount3,
            "Unexpected number of parameters to {} function - expected 3, found {}.\n",
        ),
        (
            Msg::ParameterCount2,
            "Unexpected number of parameters to {} function - expected 2, found {}.\n",
        ),
        (
            Msg::Malformed,
            "Malformed {} expression. It has {} children but should have between {} and {}",
        ),
        (
            Msg::UnsupportedInOptimization,
            "Unsupported {} operator in expression optimization",
        ),
        (
            Msg::UnexpectedComma,
            "Error: Unexpected {} operator not inside an arguments list for a query, loop, or math function",
        ),
        (
            Msg::ForEachParameters,
            "Error: for_each requires three parameters - a variable to represent an element of an array, an expression resulting in an array, and an expression to run per element of that array.",
        ),
        (
            Msg::AssignToNonVariable,
            "Error: assignment to non-variable not allowed. Expression is trying to assign to a: {}",
        ),
        (
            Msg::ConditionalWithoutIf,
            "Error: '?' operator couldn't find a valid preceding 'if' expression",
        ),
        (
            Msg::LoopParameters,
            "Error: loop requires two parameters - an expression resulting in a number of times to loop, and a {{}}-delimited expression to loop.",
        ),
        (
            Msg::StatementNotSingle,
            "Error: Could not reduce sub-expression before a semicolon to a single operation to evaluate",
        ),
        (
            Msg::SectionChildCount,
            "Error: {} optimization expected only one child operation but found {}",
        ),
        (
            Msg::ForEachVariable,
            "Error: for_each expressions require either a temp or entity variable as the iteration variable (the first parameter)",
        ),
        (
            Msg::BraceWithoutStatements,
            "Brace sections must only contain semicolon-delimited expressions, even if only one expression is contained.\n",
        ),
        (
            Msg::ParametersNotOneChild,
            "{} operator should have exactly one child (a left-parenthesis expression with the params as children of it) prior to optimization.",
        ),
        (
            Msg::ParametersEmpty,
            "{} operator (math, query, loop, etc) with empty parameter list should have failed to parse",
        ),
        (
            Msg::ParametersDanglingComma,
            "Error while optimizing parameters for {} operation: comma found without a following expression.",
        ),
        (
            Msg::NonNumericalArgument,
            "'{}' expression cannot take a '{}' argument. It only supports numerical arguments.",
        ),
        (
            Msg::QueryNotNumerical,
            "{} expressions may only contain query functions that return numbers",
        ),
        (Msg::Unreachable, "Error: unreachable statements after {}."),
        (
            Msg::TempLhsNotAlone,
            "Error: left side of an assignment expression can only use temp variables if they are on their own and not part of a more complicated expression.",
        ),
        (
            Msg::OperatorOnLhs,
            "Error: cannot use {} operators on the left side of an assignment expression",
        ),
        (
            Msg::NestedPointer,
            "Error: nested pointer statements (eg: A->B->C) are not yet supported.  Store A->B in a variable (eg: D), then use D->C",
        ),
        (
            Msg::MathOnArray,
            "Error: can't currently do math operations on resource array results",
        ),
        (
            Msg::AssignToPointer,
            "Error: Assignment attempted on Pointer result. Writing to another entity's public variables is not supported, only reading them.",
        ),
        (
            Msg::CoalesceLhs,
            "Error: found left-hand-side of ?? expression that isn't a direct-variable reference - this is unsupported at this time.",
        ),
        (
            Msg::BreakOutsideLoop,
            "Error: break encountered outside of loop",
        ),
        (
            Msg::PointerRhs,
            "Error: right-hand-side of pointer expression did not evaluate to an entity variable or query function",
        ),
        (
            Msg::AssignmentForm,
            "Error: assignment expressions may either be of the form 'A->B = D' or 'C = D' where B and C may be an entity variable or member accessor, or C may be a temp variable.  Found an expression where C is a {}",
        ),
        (Msg::CompileFailed, "expression '{}' compile failed"),
        (
            Msg::WriteToOtherMob,
            "Error: You cannot write to a variable on another mob.",
        ),
    ];

    #[test]
    fn every_template_is_pinned() {
        let pinned = PINNED;
        assert_eq!(pinned.len(), Msg::ALL.len());
        for ((message, template), listed) in pinned.iter().zip(Msg::ALL) {
            assert_eq!(message, listed, "table order");
            assert_eq!(message.template(), *template, "{message:?}");
        }
    }

    #[test]
    fn the_messages_are_in_row_order() {
        let rows: Vec<u8> = Msg::ALL.iter().map(|m| m.row()).collect();
        assert!(rows.windows(2).all(|pair| pair[0] <= pair[1]), "{rows:?}");
    }

    #[test]
    fn the_id_is_the_zero_padded_row() {
        for message in Msg::ALL {
            assert_eq!(
                message.id(),
                format!("E{:02}", message.row()),
                "{message:?}"
            );
        }
        assert_eq!(Msg::NoTokens.id(), "E01");
        assert_eq!(Msg::BadExponent.id(), "E06");
        assert_eq!(Msg::ClosingMismatch.id(), "E11");
        assert_eq!(Msg::DepthOverflow.id(), "E22");
        assert_eq!(Msg::ParametersDanglingComma.id(), "E35");
        assert_eq!(Msg::WriteToOtherMob.id(), "E49");
    }

    #[test]
    fn every_message_has_a_distinct_template() {
        let templates: BTreeSet<&str> = Msg::ALL.iter().map(|m| m.template()).collect();
        assert_eq!(templates.len(), Msg::ALL.len());
        assert!(Msg::ALL.iter().all(|m| !m.template().is_empty()));
    }

    #[test]
    fn every_brace_is_a_placeholder_or_doubled() {
        for message in Msg::ALL {
            let rest = message
                .template()
                .replace("{{", "")
                .replace("}}", "")
                .replace("{}", "");
            assert!(
                !rest.contains(['{', '}']),
                "{message:?}: a lone brace in {:?}",
                message.template()
            );
        }
    }

    #[test]
    fn formatting_with_one_argument_per_placeholder_leaves_no_placeholder() {
        for message in Msg::ALL {
            let count = placeholder_count(*message);
            let args: Vec<&dyn fmt::Display> =
                (0..count).map(|_| &"X" as &dyn fmt::Display).collect();
            let text = message.format(&args);
            assert!(
                !text.contains("{}") || *message == Msg::LoopParameters,
                "{message:?}: {text:?}"
            );
            assert_eq!(
                text.matches('X').count(),
                count + message.template().matches('X').count(),
                "{message:?}"
            );
        }
    }

    #[test]
    fn a_doubled_brace_prints_one_brace() {
        assert_eq!(Msg::LoopParameters.template().matches("{{}}").count(), 1);
        assert_eq!(
            Msg::LoopParameters.format(&[]),
            "Error: loop requires two parameters - an expression resulting in a number of times to loop, and a {}-delimited expression to loop."
        );
        assert!(!Msg::LoopParameters.is_plain());
        assert!(Msg::NoTokens.is_plain());
        assert!(!Msg::UnrecognizedToken.is_plain());
    }

    #[test]
    fn the_placeholder_counts_of_representative_messages() {
        for (message, count) in [
            (Msg::NoTokens, 0),
            (Msg::UnrecognizedToken, 1),
            (Msg::NoClosingSymbol, 3),
            (Msg::ClosingMismatch, 5),
            (Msg::ParameterCount3, 2),
            (Msg::Malformed, 4),
            (Msg::SectionChildCount, 2),
            (Msg::NonNumericalArgument, 2),
            (Msg::CompileFailed, 1),
            (Msg::WriteToOtherMob, 0),
        ] {
            assert_eq!(placeholder_count(message), count, "{message:?}");
        }
    }

    #[test]
    fn the_message_table_rows() {
        const ROWS: [(u8, usize, Effect, DiagCode); 49] = [
            (1, 1, Effect::Reject, DiagCode::Syntax),
            (2, 1, Effect::Reject, DiagCode::Syntax),
            (3, 1, Effect::Reject, DiagCode::Syntax),
            (4, 1, Effect::Reject, DiagCode::Syntax),
            (5, 1, Effect::Reject, DiagCode::UnknownQuery),
            (6, 1, Effect::Keep, DiagCode::Syntax),
            (7, 1, Effect::Reject, DiagCode::StatementForm),
            (8, 1, Effect::Reject, DiagCode::StatementForm),
            (9, 1, Effect::Reject, DiagCode::StatementForm),
            (10, 1, Effect::Reject, DiagCode::StatementForm),
            (11, 1, Effect::Reject, DiagCode::StatementForm),
            (12, 1, Effect::Reject, DiagCode::StatementForm),
            (13, 1, Effect::Reject, DiagCode::StatementForm),
            (14, 1, Effect::Reject, DiagCode::StatementForm),
            (15, 1, Effect::Reject, DiagCode::StatementForm),
            (16, 1, Effect::Reject, DiagCode::StatementForm),
            (17, 2, Effect::Reject, DiagCode::StatementForm),
            (18, 3, Effect::Reject, DiagCode::StatementForm),
            (19, 1, Effect::Reject, DiagCode::StatementForm),
            (20, 1, Effect::Reject, DiagCode::StatementForm),
            (21, 1, Effect::Reject, DiagCode::InvalidOperation),
            (22, 1, Effect::Reject, DiagCode::DepthLimit),
            (23, 2, Effect::Reject, DiagCode::StatementForm),
            (24, 1, Effect::Reject, DiagCode::StatementForm),
            (25, 1, Effect::Reject, DiagCode::StatementForm),
            (26, 1, Effect::Reject, DiagCode::StatementForm),
            (27, 1, Effect::Reject, DiagCode::StatementForm),
            (28, 1, Effect::Reject, DiagCode::InvalidAssignment),
            (29, 1, Effect::Reject, DiagCode::StatementForm),
            (30, 1, Effect::Reject, DiagCode::StatementForm),
            (31, 1, Effect::Reject, DiagCode::StatementForm),
            (32, 1, Effect::Reject, DiagCode::StatementForm),
            (33, 1, Effect::Reject, DiagCode::StatementForm),
            (34, 1, Effect::Reject, DiagCode::StatementForm),
            (35, 3, Effect::Reject, DiagCode::StatementForm),
            (36, 1, Effect::Reject, DiagCode::StringMisuse),
            (37, 1, Effect::Reject, DiagCode::StringMisuse),
            (38, 1, Effect::RootOnly, DiagCode::StatementForm),
            (39, 1, Effect::RootOnly, DiagCode::InvalidAssignment),
            (40, 1, Effect::RootOnly, DiagCode::InvalidAssignment),
            (41, 1, Effect::RootOnly, DiagCode::InvalidAssignment),
            (42, 1, Effect::RootOnly, DiagCode::StringMisuse),
            (43, 1, Effect::RootOnly, DiagCode::InvalidAssignment),
            (44, 1, Effect::RootOnly, DiagCode::InvalidAssignment),
            (45, 1, Effect::RootOnly, DiagCode::StatementForm),
            (46, 1, Effect::RootOnly, DiagCode::InvalidAssignment),
            (47, 1, Effect::RootOnly, DiagCode::InvalidAssignment),
            (48, 1, Effect::Reject, DiagCode::StatementForm),
            (49, 1, Effect::Reject, DiagCode::InvalidAssignment),
        ];
        for (row, texts, effect, code) in ROWS {
            let messages: Vec<Msg> = Msg::ALL
                .iter()
                .copied()
                .filter(|m| m.row() == row)
                .collect();
            assert_eq!(messages.len(), texts, "row {row}: {messages:?}");
            for message in messages {
                assert_eq!(message.effect(), effect, "{message:?}");
                assert_eq!(message.code(), code, "{message:?}");
            }
        }
        assert_eq!(
            ROWS.iter().map(|&(_, texts, _, _)| texts).sum::<usize>(),
            55
        );
        assert_eq!(Msg::ALL.len(), 55);
    }

    #[test]
    fn format_substitutes_arguments_in_order() {
        assert_eq!(
            Msg::NoClosingSymbol.format(&[&3, &"(", &")"]),
            "Unable to find matching closing section symbol for symbol at 3(() -- looking for )"
        );
        assert_eq!(
            Msg::ClosingMismatch.format(&[&3, &"(", &")", &"}", &9u64]),
            "Unable to match closing section symbol at 3(() - looking for ), found } at 9"
        );
        assert_eq!(
            Msg::Malformed.format(&[&"Add '+'", &1, &2, &-1]),
            "Malformed Add '+' expression. It has 1 children but should have between 2 and -1"
        );
        assert_eq!(
            Msg::ParameterCount3.format(&[&"Clamp", &2]),
            "Unexpected number of parameters to Clamp function - expected 3, found 2.\n"
        );
        assert_eq!(
            Msg::Malformed.format(&[&"A", &1, &2, &3]),
            "Malformed A expression. It has 1 children but should have between 2 and 3"
        );
        assert_eq!(
            Msg::NoClosingSymbol.format(&[&5, &"(", &")"]),
            "Unable to find matching closing section symbol for symbol at 5(() -- looking for )"
        );
    }

    #[test]
    fn format_accepts_any_display_argument() {
        assert_eq!(
            Msg::SectionChildCount.format(&[&String::from("Left Parenthesis '('"), &7usize]),
            "Error: Left Parenthesis '(' optimization expected only one child operation but found 7"
        );
        assert_eq!(
            Msg::SectionChildCount.format(&[&'x', &1.5f32]),
            "Error: x optimization expected only one child operation but found 1.5"
        );
    }

    #[test]
    fn format_drops_placeholders_without_an_argument() {
        assert_eq!(
            Msg::Malformed.format(&[&"Add"]),
            "Malformed Add expression. It has  children but should have between  and "
        );
        assert_eq!(Msg::UnrecognizedToken.format(&[]), "unrecognized token: ");
    }

    #[test]
    fn format_ignores_surplus_arguments() {
        assert_eq!(
            Msg::NoTokens.format(&[&"extra"]),
            "No tokens found in expression"
        );
        assert_eq!(
            Msg::UnrecognizedToken.format(&[&"a", &"b"]),
            "unrecognized token: a"
        );
    }

    #[test]
    fn format_does_not_re_scan_substituted_text() {
        assert_eq!(
            Msg::UnrecognizedToken.format(&[&"{} {{}} %s"]),
            "unrecognized token: {} {{}} %s"
        );
    }

    #[test]
    fn format_of_a_template_without_placeholders_is_the_template() {
        for message in Msg::ALL.iter().filter(|m| m.is_plain()) {
            assert_eq!(message.format(&[]), message.template(), "{message:?}");
        }
    }

    #[test]
    fn a_plain_language_diagnostic_carries_its_fields_and_borrows_its_template() {
        let diagnostic = Diagnostic::language(Msg::DepthOverflow, Severity::Warning, 3..9, &[]);
        assert_eq!(diagnostic.code(), Msg::DepthOverflow.code());
        assert_eq!(diagnostic.severity, Severity::Warning);
        assert_eq!(diagnostic.span, 3..9);
        assert_eq!(diagnostic.language_message(), Some(Msg::DepthOverflow));
        assert_eq!(diagnostic.message(), Msg::DepthOverflow.template());
        assert!(matches!(diagnostic.message(), Cow::Borrowed(_)));
        let surplus = Diagnostic::language(Msg::NoTokens, Severity::Error, 0..1, &[&"extra"]);
        assert!(matches!(
            surplus.kind,
            Kind::Language {
                text: LanguageText::Template,
                ..
            }
        ));
    }

    #[test]
    fn a_language_diagnostic_with_arguments_owns_its_formatted_text() {
        let diagnostic = Diagnostic::language(Msg::Unreachable, Severity::Error, 0..2, &[&"Break"]);
        assert_eq!(
            diagnostic.message(),
            "Error: unreachable statements after Break."
        );
        assert!(matches!(diagnostic.message(), Cow::Borrowed(_)));
        assert_eq!(
            Diagnostic::language(Msg::LoopParameters, Severity::Error, 0..1, &[]).message(),
            Msg::LoopParameters.format(&[])
        );
    }

    #[test]
    fn a_lint_owns_a_built_text_and_borrows_a_static_one() {
        let diagnostic = Diagnostic::lint(
            DiagCode::QueryArity,
            Severity::Info,
            0..4,
            "built text".to_owned(),
        );
        assert_eq!(diagnostic.message(), "built text");
        assert_eq!(
            (diagnostic.code(), diagnostic.language_message()),
            (DiagCode::QueryArity, None)
        );
        assert!(matches!(diagnostic.message(), Cow::Borrowed(_)));
        assert_eq!(diagnostic.severity, Severity::Info);
        assert_eq!(lint("static text").message(), "static text");
        assert!(matches!(lint("static text").message(), Cow::Borrowed(_)));
    }

    #[test]
    fn a_quoting_diagnostic_takes_its_code_from_the_message() {
        let diagnostic = Diagnostic::quoting(
            Msg::UnknownToken,
            Severity::Error,
            2..6,
            Arc::from("a bad token"),
            2..11,
        );
        assert_eq!(diagnostic.code(), DiagCode::Syntax);
        assert_eq!(diagnostic.language_message(), Some(Msg::UnknownToken));
        assert_eq!(diagnostic.span, 2..6);
        assert_eq!(diagnostic.severity, Severity::Error);
    }

    #[test]
    fn a_quoting_diagnostic_formats_the_template_with_the_source_slice() {
        let source = "1 + @bad";
        let diagnostic = quoting(Msg::UnrecognizedToken, source, 4, 8);
        assert_eq!(diagnostic.message(), "unrecognized token: @bad");
        assert_eq!(
            quoting(Msg::UnknownToken, source, 0, 8).message(),
            "Error: unknown token: 1 + @bad"
        );
        assert_eq!(
            quoting(Msg::BadExponent, "1e;", 0, 3).message(),
            "error parsing float string, expected '+' or '-' after 'e': 1e;"
        );
        assert_eq!(
            quoting(Msg::CompileFailed, "v.x", 0, 3).message(),
            "expression 'v.x' compile failed"
        );
    }

    #[test]
    fn a_quoting_diagnostic_whose_range_is_at_the_end_quotes_nothing() {
        let diagnostic = quoting(Msg::UnrecognizedToken, "abc", 3, 3);
        assert_eq!(diagnostic.message(), "unrecognized token: ");
        let empty = quoting(Msg::UnrecognizedToken, "", 0, 0);
        assert_eq!(empty.message(), "unrecognized token: ");
    }

    #[test]
    fn a_quoting_diagnostic_whose_range_leaves_the_source_quotes_nothing() {
        assert_eq!(
            quoting(Msg::UnrecognizedToken, "abc", 5, 9).message(),
            "unrecognized token: "
        );
        assert_eq!(
            quoting(Msg::UnrecognizedToken, "abc", 2, 9).message(),
            "unrecognized token: "
        );
        assert_eq!(
            quoting(Msg::UnrecognizedToken, "abc", 2, 1).message(),
            "unrecognized token: "
        );
    }

    #[test]
    fn a_quoted_range_may_hold_multi_byte_text() {
        let source = "x + héllo wörld";
        let start = 4;
        let diagnostic = quoting(Msg::UnrecognizedToken, source, start, source.len() as u32);
        assert_eq!(diagnostic.message(), "unrecognized token: héllo wörld");
        let accent = source.find('é').expect("an accent") as u32;
        assert_eq!(
            quoting(Msg::UnrecognizedToken, source, accent, accent + 2).message(),
            "unrecognized token: é"
        );
    }

    #[test]
    fn a_range_that_splits_a_multi_byte_character_is_printed_lossily() {
        // The bytes skipped after a `true` prefix can end inside a character.
        let source = "éa";
        let split = quoting(Msg::UnrecognizedToken, source, 1, 3);
        assert_eq!(split.message(), "unrecognized token: \u{fffd}a");
        let cut = quoting(Msg::UnrecognizedToken, source, 0, 1);
        assert_eq!(cut.message(), "unrecognized token: \u{fffd}");
    }

    #[test]
    fn a_quoting_diagnostic_formats_on_every_read() {
        let diagnostic = quoting(Msg::UnrecognizedToken, "abc def", 4, 7);
        let first = diagnostic.message().into_owned();
        assert_eq!(first, "unrecognized token: def");
        assert!(matches!(diagnostic.message(), Cow::Owned(_)));
        assert_eq!(diagnostic.message(), first);
    }

    #[test]
    fn diagnostics_of_one_compile_share_the_source_copy() {
        let source: Arc<str> = Arc::from("abcdef");
        let first = Diagnostic::quoting(
            Msg::UnrecognizedToken,
            Severity::Error,
            0..1,
            Arc::clone(&source),
            0..6,
        );
        let second = Diagnostic::quoting(
            Msg::UnknownToken,
            Severity::Error,
            0..1,
            Arc::clone(&source),
            3..6,
        );
        assert_eq!(Arc::strong_count(&source), 3);
        let cloned = second.clone();
        assert_eq!(Arc::strong_count(&source), 4);
        assert_eq!(cloned.message(), "Error: unknown token: def");
        drop((first, second, cloned));
        assert_eq!(Arc::strong_count(&source), 1);
    }

    #[test]
    fn language_message_is_some_exactly_for_language_messages() {
        assert!(
            Diagnostic::language(Msg::NoTokens, Severity::Info, 0..1, &[])
                .language_message()
                .is_some()
        );
        assert!(
            quoting(Msg::UnrecognizedToken, "a", 0, 1)
                .language_message()
                .is_some()
        );
        assert!(lint("our own lint").language_message().is_none());
        assert!(
            Diagnostic::lint(DiagCode::QueryArity, Severity::Warning, 0..1, String::new())
                .language_message()
                .is_none()
        );
    }

    #[test]
    fn language_message_does_not_depend_on_the_severity() {
        for severity in [Severity::Info, Severity::Warning, Severity::Error] {
            assert!(
                Diagnostic::language(Msg::BadExponent, severity, 0..1, &[&"x"])
                    .language_message()
                    .is_some()
            );
        }
    }

    #[test]
    fn display_writes_the_message_without_trailing_whitespace() {
        let with_newline = Diagnostic::language(Msg::LeadingSemicolon, Severity::Error, 0..1, &[]);
        assert_eq!(
            with_newline.message(),
            "Error: expressions can't begin with a semicolon\n"
        );
        assert_eq!(
            with_newline.to_string(),
            "Error: expressions can't begin with a semicolon"
        );
        assert_eq!(lint("plain").to_string(), "plain");
        assert_eq!(lint("  padded \n\n").to_string(), "  padded");
        assert_eq!(
            quoting(Msg::UnrecognizedToken, "abc", 0, 3).to_string(),
            "unrecognized token: abc"
        );
    }

    #[test]
    fn diagnostics_compare_by_every_field_and_the_text() {
        let language = |message, severity, span, arg: &str| {
            Diagnostic::language(message, severity, span, &[&arg])
        };
        let base = || language(Msg::UnrecognizedToken, Severity::Error, 0..3, "text");
        assert_eq!(base(), base());
        assert_ne!(
            base(),
            language(Msg::UnrecognizedToken, Severity::Warning, 0..3, "text")
        );
        assert_ne!(
            base(),
            language(Msg::UnrecognizedToken, Severity::Error, 0..4, "text")
        );
        assert_ne!(
            base(),
            language(Msg::UnrecognizedToken, Severity::Error, 1..3, "text")
        );
        assert_ne!(
            base(),
            language(Msg::UnknownToken, Severity::Error, 0..3, "text")
        );
        assert_ne!(
            base(),
            Diagnostic::lint(
                DiagCode::Syntax,
                Severity::Error,
                0..3,
                "unrecognized token: text"
            )
        );
        assert_ne!(
            base(),
            language(Msg::UnrecognizedToken, Severity::Error, 0..3, "other")
        );
        let lint = |code, text| Diagnostic::lint(code, Severity::Error, 0..3, text);
        assert_eq!(lint(DiagCode::Syntax, "a"), lint(DiagCode::Syntax, "a"));
        assert_ne!(lint(DiagCode::Syntax, "a"), lint(DiagCode::Syntax, "b"));
        assert_ne!(lint(DiagCode::Syntax, "a"), lint(DiagCode::QueryArity, "a"));
    }

    #[test]
    fn equal_text_compares_equal_whatever_the_representation() {
        let formatted =
            Diagnostic::language(Msg::UnrecognizedToken, Severity::Error, 0..3, &[&"abc"]);
        let quoting = Diagnostic::quoting(
            Msg::UnrecognizedToken,
            Severity::Error,
            0..3,
            Arc::from("abc"),
            0..3,
        );
        assert_eq!(formatted, quoting);
        assert_eq!(quoting, formatted);
        let borrowed = Diagnostic::lint(DiagCode::Syntax, Severity::Error, 0..3, "text");
        let owned = Diagnostic::lint(DiagCode::Syntax, Severity::Error, 0..3, "text".to_owned());
        assert_eq!(borrowed, owned);
    }

    #[test]
    fn debug_prints_the_formatted_text() {
        let slice = quoting(Msg::UnrecognizedToken, "abc", 0, 3);
        let debug = format!("{slice:?}");
        assert!(debug.contains("\"unrecognized token: abc\""), "{debug}");
        assert!(debug.contains("Syntax"), "{debug}");
        assert!(format!("{:?}", lint("static")).contains("\"static\""));
        assert_eq!(
            format!(
                "{:?}",
                Diagnostic::language(Msg::NoTokens, Severity::Error, 0..1, &[])
            ),
            "Diagnostic { code: Syntax, severity: Error, span: 0..1, \
             language_message: Some(NoTokens), text: \"No tokens found in expression\", .. }"
        );
    }

    #[test]
    fn language_text_variants_read_back_their_content() {
        assert_eq!(
            LanguageText::Template.read(Msg::NoTokens),
            "No tokens found in expression"
        );
        assert_eq!(LanguageText::Formatted("b".into()).read(Msg::NoTokens), "b");
        let slice = LanguageText::Quoting {
            source: Arc::from("xyz"),
            range: 1..3,
        };
        assert_eq!(slice.read(Msg::UnknownToken), "Error: unknown token: yz");
    }

    #[test]
    fn the_severity_of_an_effect() {
        assert_eq!(Msg::NoTokens.effect().severity(), Severity::Error);
        assert_eq!(Msg::BadExponent.effect().severity(), Severity::Warning);
        assert_eq!(Msg::BreakOutsideLoop.effect().severity(), Severity::Error);
    }

    #[test]
    fn severities_are_ordered_from_info_to_error() {
        assert!(Severity::Info < Severity::Warning);
        assert!(Severity::Warning < Severity::Error);
        assert_eq!(
            [Severity::Error, Severity::Info, Severity::Warning]
                .into_iter()
                .max(),
            Some(Severity::Error)
        );
        assert_eq!(
            [Severity::Error, Severity::Info, Severity::Warning]
                .into_iter()
                .min(),
            Some(Severity::Info)
        );
    }

    #[test]
    fn the_diagnostic_codes_are_distinct() {
        let codes = [
            DiagCode::Syntax,
            DiagCode::UnknownQuery,
            DiagCode::QueryArity,
            DiagCode::QueryExperiment,
            DiagCode::QueryDeprecated,
            DiagCode::QueryClientOnly,
            DiagCode::InvalidOperation,
            DiagCode::StringMisuse,
            DiagCode::InvalidAssignment,
            DiagCode::StatementForm,
            DiagCode::DepthLimit,
            DiagCode::SourceTooLong,
            DiagCode::InvalidVersion,
            DiagCode::DiagnosticLimit,
        ];
        let distinct: BTreeSet<String> = codes.iter().map(|code| format!("{code:?}")).collect();
        assert_eq!(distinct.len(), codes.len());
    }

    #[test]
    fn no_language_message_has_one_of_the_crates_own_codes() {
        let own = [
            DiagCode::QueryArity,
            DiagCode::QueryExperiment,
            DiagCode::QueryDeprecated,
            DiagCode::QueryClientOnly,
            DiagCode::SourceTooLong,
            DiagCode::InvalidVersion,
            DiagCode::DiagnosticLimit,
        ];
        for message in Msg::ALL {
            assert!(!own.contains(&message.code()), "{message:?}");
        }
    }

    #[test]
    fn fixed6_prints_six_decimals() {
        for (value, text) in [
            (0.0, "0.000000"),
            (1.0, "1.000000"),
            (-1.0, "-1.000000"),
            (0.5, "0.500000"),
            (2.25, "2.250000"),
            (123.456, "123.456001"),
            (1.0e10, "10000000000.000000"),
            (1.0e-7, "0.000000"),
            (0.1, "0.100000"),
            (std::f32::consts::PI, "3.141593"),
            (-0.000_000_4, "-0.000000"),
        ] {
            assert_eq!(fixed6(value), text, "{value}");
        }
    }

    #[test]
    fn fixed6_keeps_the_sign_of_zero() {
        assert_eq!(fixed6(0.0), "0.000000");
        assert_eq!(fixed6(-0.0), "-0.000000");
    }

    #[test]
    fn fixed6_spells_non_finite_values_in_lower_case_with_their_sign() {
        assert_eq!(fixed6(f32::NAN), "nan");
        assert_eq!(fixed6(-f32::NAN), "-nan");
        assert_eq!(fixed6(f32::INFINITY), "inf");
        assert_eq!(fixed6(f32::NEG_INFINITY), "-inf");
    }

    #[test]
    fn fixed6_prints_the_extremes_in_full() {
        assert_eq!(
            fixed6(f32::MAX),
            "340282346638528859811704183484516925440.000000"
        );
        assert_eq!(
            fixed6(f32::MIN),
            "-340282346638528859811704183484516925440.000000"
        );
        assert_eq!(fixed6(f32::MIN_POSITIVE), "0.000000");
    }

    #[test]
    fn fixed6_rounds_the_f32_exactly_as_the_double_it_widens_to() {
        // 16,777,217 is not an f32: the literal is the even neighbour.
        assert_eq!(fixed6(16_777_216.0), "16777216.000000");
        assert_eq!(fixed6(16_777_217.0), "16777216.000000");
        assert_eq!(fixed6(8_388_607.5), "8388607.500000");
        assert_eq!(fixed6(1.000_000_5), "1.000000");
        assert_eq!(fixed6(1.000_001), "1.000001");
    }
}
