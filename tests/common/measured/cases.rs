//! [`EvalCase`]: one case's setup and rows, checked on this build. The rows run in order on one
//! state (`this` = 2.34; the subject has no actor). Setup steps apply where they stand among the
//! rows, and a variable they set also enters the initial map: a row that assigns starts from the
//! initial map, a row that only reads sees the map as the previous rows left it. Random draws are
//! the case's [`EvalCase::fixed_random`], else the xorshift128 sequence.

use molangx::catalog::{QueryAdmission, QueryAllowList, QuerySetMask};
use molangx::compile::{CompileOptions, Expr, compile};
use molangx::diag::LanguageMessage;
use molangx::hash::HashedStr;
use molangx::rng::{FixedRng, Xorshift128};
use molangx::version::{ExperimentMask, MolangVersion, RawVersion};

use super::math_call::{evaluate_call, parse_call, uses_random};
use crate::common::{
    CheckGuard, REFERENCE_SETS,
    compile_support::client_at,
    declared::{ALLOWED, decl, query_names, substitute},
    reference_catalog, reference_experiments, within,
};

#[cfg(feature = "vm")]
use crate::common::host::{Actor, Env, LIVE_ACTOR, SECOND_ACTOR, TestHost};
#[cfg(feature = "vm")]
use molangx::compile::{Compiled, ProgramFlags};
#[cfg(feature = "vm")]
use molangx::rng::rand_core::Rng;
#[cfg(feature = "vm")]
use molangx::vm::{CollectSink, ContextName, Value, VariableMap, VariableName};
#[cfg(feature = "vm")]
use std::sync::Arc;

const DEFAULT_TOLERANCE: f32 = 1e-6;

/// What `v.baa` holds at the start of a case.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Actors {
    /// An array of three removed actors.
    Removed,
    /// An array of five actors, three of them live.
    Mixed,
    /// An array of three live actors.
    Live,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum CaseActor {
    /// The actor [`ContextActor::Live`] points at; its variable map is the case's map.
    Live,
    /// Nothing points at it.
    Second,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum ContextActor {
    Live,
    Null,
}

/// The first language message of a `parse_fails` row.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum ParseFailure {
    /// A string-returning query as an arithmetic operand; the row's queries resolve.
    StringOperand,
    /// A query that does not resolve under the case's query sets at the row's version.
    UnresolvedQuery,
}

impl ParseFailure {
    const ALL: [Self; 2] = [Self::StringOperand, Self::UnresolvedQuery];

    const fn message(self) -> LanguageMessage {
        match self {
            Self::StringOperand => LanguageMessage::QueryNotNumerical,
            Self::UnresolvedQuery => LanguageMessage::QueryUnresolved,
        }
    }
}

#[derive(Clone, Debug)]
enum Setup {
    Set {
        variable: String,
        value: f32,
        public: bool,
    },
    Context {
        name: String,
        actor: ContextActor,
    },
    BabyFlag(CaseActor),
    RefreshSnapshots,
}

/// `passes_with` is the variable and float that, set first, make the row pass.
#[derive(Clone, Debug)]
struct RowFailure {
    observed: f32,
    messages: Vec<String>,
    passes_with: (String, f32),
}

#[derive(Clone, Debug)]
pub struct EvalRow {
    expr: String,
    expected: f32,
    version: i16,
    tolerance: Option<f32>,
    failure: Option<RowFailure>,
}

impl EvalRow {
    /// Default 13.
    pub fn at(&mut self, version: i16) -> &mut Self {
        self.version = version;
        self
    }

    /// Overrides the case's tolerance.
    pub fn within(&mut self, tolerance: f32) -> &mut Self {
        self.tolerance = Some(tolerance);
        self
    }

    /// The row gives `observed` (to the bit) with exactly `messages`, and passes once the initial
    /// variable map also holds `passes_with`.
    pub fn expected_failure(
        &mut self,
        observed: f32,
        messages: &[&str],
        passes_with: (&str, f32),
    ) -> &mut Self {
        self.failure = Some(RowFailure {
            observed,
            messages: owned(messages),
            passes_with: (passes_with.0.to_owned(), passes_with.1),
        });
        self
    }
}

/// At version 13.
#[derive(Clone, Debug)]
pub struct RangeRow {
    expr: String,
    lo: f32,
    hi: f32,
}

#[derive(Clone, Debug)]
pub struct HashRow {
    expr: String,
    literal: String,
    expected: u64,
    version: i16,
}

impl HashRow {
    /// Default 13.
    pub fn at(&mut self, version: i16) -> &mut Self {
        self.version = version;
        self
    }
}

#[derive(Clone, Debug)]
pub struct ParsesRow {
    expr: String,
    version: i16,
    experiment: bool,
}

impl ParsesRow {
    /// Default 13.
    pub fn at(&mut self, version: i16) -> &mut Self {
        self.version = version;
        self
    }

    pub fn with_experiment(&mut self) -> &mut Self {
        self.experiment = true;
        self
    }
}

#[derive(Clone, Debug)]
pub struct ParseFailsRow {
    expr: String,
    version: i16,
    experiment: bool,
    because: Option<ParseFailure>,
}

impl ParseFailsRow {
    /// Default 13.
    pub fn at(&mut self, version: i16) -> &mut Self {
        self.version = version;
        self
    }

    pub fn with_experiment(&mut self) -> &mut Self {
        self.experiment = true;
        self
    }

    /// Asserts the first language message and whether the row's queries resolve.
    pub fn because(&mut self, reason: ParseFailure) -> &mut Self {
        self.because = Some(reason);
        self
    }
}

/// What a `parses` or `parse_fails` row expects.
#[derive(Copy, Clone, Debug)]
enum ParseExpectation {
    Parses,
    /// With the `parse_fails` row's stated reason, if any.
    Fails(Option<ParseFailure>),
}

#[derive(Clone, Debug)]
enum ListAssertion {
    AllParse(bool),
    EvaluatesTo(f32),
    FailsEvaluation,
}

#[derive(Clone, Debug)]
pub struct ListRow {
    assertion: ListAssertion,
    items: Vec<String>,
    version: i16,
}

impl ListRow {
    /// Default 13.
    pub fn at(&mut self, version: i16) -> &mut Self {
        self.version = version;
        self
    }
}

#[derive(Clone, Debug)]
enum Row {
    Setup(Setup),
    Eval(EvalRow),
    Range(RangeRow),
    Hash(HashRow),
    Parses(ParsesRow),
    ParseFails(ParseFailsRow),
    /// Counts random draws when `include_random`.
    SideEffects {
        expr: String,
        include_random: bool,
        expected: bool,
    },
    /// Compiled at the latest version.
    IsConstant {
        expected: bool,
        items: Vec<String>,
    },
    /// Compiled at version 13.
    HasDisallowedQueries {
        expected: bool,
        items: Vec<String>,
    },
    List(ListRow),
}

impl Row {
    fn label(&self) -> &'static str {
        match self {
            Self::Setup(_) => "setup",
            Self::Eval(_) => "eval",
            Self::Range(_) => "range",
            Self::Hash(_) => "hash",
            Self::Parses(_) => "parses",
            Self::ParseFails(_) => "parse_fails",
            Self::SideEffects { .. } => "side_effects",
            Self::IsConstant { .. } => "is_constant",
            Self::HasDisallowedQueries { .. } => "has_disallowed_queries",
            Self::List(ListRow {
                assertion: ListAssertion::AllParse(_),
                ..
            }) => "all_parse",
            Self::List(ListRow {
                assertion: ListAssertion::EvaluatesTo(_),
                ..
            }) => "evaluates_to",
            Self::List(ListRow {
                assertion: ListAssertion::FailsEvaluation,
                ..
            }) => "fails_evaluation",
        }
    }

    /// What the row adds to its case's row count: one per item for a list row, none for a setup
    /// step.
    fn weight(&self) -> usize {
        match self {
            Self::IsConstant { items, .. }
            | Self::HasDisallowedQueries { items, .. }
            | Self::List(ListRow { items, .. }) => items.len(),
            Self::Setup(_) => 0,
            Self::Eval(_)
            | Self::Range(_)
            | Self::Hash(_)
            | Self::Parses(_)
            | Self::ParseFails(_)
            | Self::SideEffects { .. } => 1,
        }
    }
}

#[derive(Debug)]
pub struct EvalCase {
    id: String,
    guard: CheckGuard,
    fixed_random: Option<FixedRng>,
    actors: Option<Actors>,
    default_set_only: bool,
    tolerance: Option<f32>,
    fresh_state: bool,
    rows: Vec<Row>,
}

macro_rules! push_row {
    ($case:expr, $variant:ident($row:expr)) => {{
        $case.rows.push(Row::$variant($row));
        match $case.rows.last_mut() {
            Some(Row::$variant(row)) => row,
            _ => unreachable!("just pushed"),
        }
    }};
}

impl EvalCase {
    /// Panics unless `id` is `<group>-NNN`.
    pub fn new(id: &str) -> Self {
        let numbered = id.rsplit_once('-').is_some_and(|(_, number)| {
            !number.is_empty() && number.bytes().all(|b| b.is_ascii_digit())
        });
        assert!(numbered, "{id:?} is not `<group>-NNN`");
        Self {
            id: id.to_owned(),
            guard: CheckGuard::new(id),
            fixed_random: None,
            actors: None,
            default_set_only: false,
            tolerance: None,
            fresh_state: false,
            rows: Vec::new(),
        }
    }

    pub fn fixed_random(&mut self, draw: f32) -> &mut Self {
        self.fixed_random = Some(FixedRng::from_sample(draw).expect("a word's sample"));
        self
    }

    pub fn actors(&mut self, actors: Actors) -> &mut Self {
        self.actors = Some(actors);
        self
    }

    /// Without the helper queries' set.
    pub fn default_set_only(&mut self) -> &mut Self {
        self.default_set_only = true;
        self
    }

    /// For the `eval` rows without their own; default 1e-6.
    pub fn tolerance(&mut self, tolerance: f32) -> &mut Self {
        self.tolerance = Some(tolerance);
        self
    }

    /// Also evaluates every `eval` row alone on a fresh state. Only for a case without setup (no
    /// setup step, fixed draw or actors, both query sets) whose `eval` rows, one at least, are all
    /// at the latest version without an expected failure; [`EvalCase::check`] panics otherwise.
    pub fn also_on_a_fresh_state(&mut self) -> &mut Self {
        self.fresh_state = true;
        self
    }

    /// Setup: `variable` (with its namespace) holds `value` from here on and in the initial map.
    pub fn set(&mut self, variable: &str, value: f32) -> &mut Self {
        self.setup(Setup::Set {
            variable: variable.to_owned(),
            value,
            public: false,
        })
    }

    /// Setup: [`EvalCase::set`] as a public variable, which reads through `->` see.
    pub fn set_public(&mut self, variable: &str, value: f32) -> &mut Self {
        self.setup(Setup::Set {
            variable: variable.to_owned(),
            value,
            public: true,
        })
    }

    pub fn context_actor(&mut self, name: &str, actor: ContextActor) -> &mut Self {
        self.setup(Setup::Context {
            name: name.to_owned(),
            actor,
        })
    }

    pub fn set_baby_flag(&mut self, actor: CaseActor) -> &mut Self {
        self.setup(Setup::BabyFlag(actor))
    }

    /// Setup: the snapshots that reads through `->` see are refreshed from the variables.
    pub fn refresh_snapshots(&mut self) -> &mut Self {
        self.setup(Setup::RefreshSnapshots)
    }

    fn setup(&mut self, setup: Setup) -> &mut Self {
        self.rows.push(Row::Setup(setup));
        self
    }

    /// A bare `math.<fn>(…)` row is also checked against `stdlib::math`.
    pub fn eval(&mut self, expr: &str, expected: f32) -> &mut EvalRow {
        push_row!(
            self,
            Eval(EvalRow {
                expr: expr.to_owned(),
                expected,
                version: 13,
                tolerance: None,
                failure: None,
            })
        )
    }

    /// `lo` and `hi` in either order.
    pub fn range(&mut self, expr: &str, lo: f32, hi: f32) -> &mut RangeRow {
        push_row!(
            self,
            Range(RangeRow {
                expr: expr.to_owned(),
                lo,
                hi,
            })
        )
    }

    pub fn hash(&mut self, expr: &str, literal: &str, expected: u64) -> &mut HashRow {
        push_row!(
            self,
            Hash(HashRow {
                expr: expr.to_owned(),
                literal: literal.to_owned(),
                expected,
                version: 13,
            })
        )
    }

    pub fn parses(&mut self, expr: &str) -> &mut ParsesRow {
        push_row!(
            self,
            Parses(ParsesRow {
                expr: expr.to_owned(),
                version: 13,
                experiment: false,
            })
        )
    }

    pub fn parse_fails(&mut self, expr: &str) -> &mut ParseFailsRow {
        push_row!(
            self,
            ParseFails(ParseFailsRow {
                expr: expr.to_owned(),
                version: 13,
                experiment: false,
                because: None,
            })
        )
    }

    pub fn side_effects(&mut self, expr: &str, include_random: bool, expected: bool) {
        self.rows.push(Row::SideEffects {
            expr: expr.to_owned(),
            include_random,
            expected,
        });
    }

    pub fn is_constant(&mut self, expected: bool, items: &[&str]) {
        assert!(!items.is_empty(), "{}: an empty list row", self.id);
        self.rows.push(Row::IsConstant {
            expected,
            items: owned(items),
        });
    }

    pub fn all_parse(&mut self, expected: bool, items: &[&str]) -> &mut ListRow {
        self.list(ListAssertion::AllParse(expected), items)
    }

    /// Each item gives `expected` with no message.
    pub fn evaluates_to(&mut self, expected: f32, items: &[&str]) -> &mut ListRow {
        self.list(ListAssertion::EvaluatesTo(expected), items)
    }

    pub fn fails_evaluation(&mut self, items: &[&str]) -> &mut ListRow {
        self.list(ListAssertion::FailsEvaluation, items)
    }

    fn list(&mut self, assertion: ListAssertion, items: &[&str]) -> &mut ListRow {
        assert!(!items.is_empty(), "{}: an empty list row", self.id);
        push_row!(
            self,
            List(ListRow {
                assertion,
                items: owned(items),
                version: 13,
            })
        )
    }

    /// Against an allow list of `query.block_state` and `query.had_component_group`; placeholder
    /// queries are replaced by real ones first.
    pub fn has_disallowed_queries(&mut self, expected: bool, items: &[&str]) {
        assert!(!items.is_empty(), "{}: an empty list row", self.id);
        self.rows.push(Row::HasDisallowedQueries {
            expected,
            items: owned(items),
        });
    }

    pub fn check(&self, rows: usize) {
        self.guard
            .checked(self.checked_rows().map(|(_, row)| row.weight()).sum(), rows);
        if self.fresh_state {
            self.validate_fresh_state();
        }
        let mut failures = Vec::new();
        self.check_compile_side(&mut failures);
        self.check_math_calls(&mut failures);
        #[cfg(feature = "vm")]
        self.check_evaluation(&mut failures);
        #[cfg(feature = "vm")]
        self.check_fresh_state(&mut failures);
        assert!(
            failures.is_empty(),
            "{}: {} failing row check(s):\n{}",
            self.id,
            failures.len(),
            failures.join("\n")
        );
    }

    fn validate_fresh_state(&self) {
        let id = &self.id;
        assert!(
            self.has_no_setup(),
            "{id}: `also_on_a_fresh_state` is for a case without setup (no setup step, no fixed draw, no actors, both query sets)"
        );
        let mut evals = 0;
        for (index, row) in self.checked_rows() {
            let Row::Eval(eval) = row else { continue };
            evals += 1;
            assert!(
                eval.version == MolangVersion::LATEST.as_i16() && eval.failure.is_none(),
                "{}: the fresh-state pass takes an `eval` row at the latest version without an expected failure",
                self.name(index, row)
            );
        }
        assert!(
            evals > 0,
            "{id}: `also_on_a_fresh_state` on a case with no `eval` row"
        );
    }

    /// Each with its index among all rows, setup steps included.
    fn checked_rows(&self) -> impl Iterator<Item = (usize, &Row)> {
        self.rows
            .iter()
            .enumerate()
            .filter(|(_, row)| !matches!(row, Row::Setup(_)))
    }

    fn name(&self, index: usize, row: &Row) -> String {
        format!("{} row {index} ({})", self.id, row.label())
    }

    fn tolerance_of(&self, row: &EvalRow) -> f32 {
        row.tolerance
            .or(self.tolerance)
            .unwrap_or(DEFAULT_TOLERANCE)
    }

    fn admission(&self) -> QueryAdmission {
        QueryAdmission::Sets(if self.default_set_only {
            QuerySetMask::DEFAULT
        } else {
            REFERENCE_SETS
        })
    }

    #[allow(clippy::too_many_lines)]
    fn check_compile_side(&self, failures: &mut Vec<String>) {
        let latest = MolangVersion::LATEST.as_i16();
        for (index, row) in self.checked_rows() {
            let name = self.name(index, row);
            match row {
                Row::Parses(ParsesRow {
                    expr,
                    version,
                    experiment,
                }) => self.check_parse(
                    &name,
                    expr,
                    *version,
                    *experiment,
                    ParseExpectation::Parses,
                    failures,
                ),
                Row::ParseFails(ParseFailsRow {
                    expr,
                    version,
                    experiment,
                    because,
                }) => {
                    self.check_parse(
                        &name,
                        expr,
                        *version,
                        *experiment,
                        ParseExpectation::Fails(*because),
                        failures,
                    );
                }
                Row::IsConstant { expected, items } => {
                    for item in items {
                        let compiled = compile(item, &reference_options(latest));
                        let constant =
                            compiled.expr().is_some_and(Expr::is_constant) && compiled.is_success();
                        if constant != *expected {
                            failures.push(format!(
                                "{name} {item:?}: is_constant {constant}, expected {expected}"
                            ));
                        }
                    }
                }
                Row::List(ListRow {
                    assertion: ListAssertion::AllParse(expected),
                    items,
                    version,
                }) => {
                    for item in items {
                        let compiled = compile(item, &reference_options(*version));
                        if compiled.parses_cleanly() != *expected {
                            failures.push(format!("{name} v{version} {item:?}: expected parses = {expected}, diagnostics {:?}", compiled.diagnostics()));
                        }
                    }
                }
                Row::HasDisallowedQueries { expected, items } => {
                    let allowed_only = |options: CompileOptions| {
                        let allowed =
                            QueryAllowList::new(&options.catalog, ALLOWED).expect("declared");
                        CompileOptions {
                            admission: QueryAdmission::Only(allowed),
                            ..options
                        }
                    };
                    let helpers = allowed_only(reference_options(13));
                    let client = allowed_only(client_at(13));
                    for item in items {
                        let text = substitute(item);
                        let compiled = compile(&text, &helpers);
                        let unresolved = compiled.diagnostics().iter().any(|d| {
                            d.language_message() == Some(LanguageMessage::QueryUnresolved)
                        });
                        if unresolved != *expected {
                            failures.push(format!("{name} {text:?}: has a disallowed query {unresolved}, expected {expected}: {:?}", compiled.diagnostics()));
                        }
                        let compiled = compile(&text, &client);
                        let logged = compiled
                            .diagnostics()
                            .iter()
                            .filter(|d| d.language_message().is_some())
                            .any(|d| d.message().starts_with("Failed to resolve query"));
                        if logged != *expected {
                            failures.push(format!("{name} {text:?} (client): logs `Failed to resolve query` {logged}, expected {expected}"));
                        }
                    }
                }
                Row::List(ListRow {
                    assertion: ListAssertion::EvaluatesTo(_) | ListAssertion::FailsEvaluation,
                    items,
                    ..
                }) => {
                    for item in items {
                        let compiled = compile(item, &reference_options(latest));
                        if !(compiled.parses_cleanly() && compiled.is_success()) {
                            failures.push(format!(
                                "{name} {item:?}: does not compile cleanly: {:?}",
                                compiled.diagnostics()
                            ));
                        }
                    }
                }
                Row::SideEffects {
                    expr,
                    include_random,
                    expected,
                } => {
                    let compiled = compile(expr, &reference_options(latest));
                    let has = compiled
                        .expr()
                        .is_some_and(|e| e.has_side_effects(*include_random));
                    if !compiled.parses_cleanly() || has != *expected {
                        failures.push(format!(
                            "{name} {expr:?} include_random = {include_random}: side effects {has}, expected {expected}: {:?}",
                            compiled.diagnostics()
                        ));
                    }
                }
                Row::Hash(HashRow {
                    literal, expected, ..
                }) => {
                    if HashedStr::new(literal).as_u64() != *expected {
                        failures.push(format!(
                            "{name}: the hash of {literal:?} is {:#018x}, not {expected:#018x}",
                            HashedStr::new(literal).as_u64()
                        ));
                    }
                }
                Row::Setup(_) | Row::Eval(_) | Row::Range(_) => {}
            }
        }
    }

    fn check_parse(
        &self,
        name: &str,
        expr: &str,
        version: i16,
        experiment: bool,
        expected: ParseExpectation,
        failures: &mut Vec<String>,
    ) {
        let parses = matches!(expected, ParseExpectation::Parses);
        let options = CompileOptions {
            admission: self.admission(),
            experiments: if experiment {
                reference_experiments()
            } else {
                ExperimentMask::empty()
            },
            ..reference_options(version)
        };
        let compiled = compile(expr, &options);
        if compiled.parses_cleanly() != parses {
            failures.push(format!(
                "{name} v{version} {expr:?}: expected parses = {parses}, diagnostics {:?}",
                compiled.diagnostics()
            ));
        }
        // Only the first language message counts: an unresolved query is followed by `unrecognized
        // token`.
        let language: Vec<_> = compiled
            .diagnostics()
            .iter()
            .filter_map(|d| d.language_message())
            .collect();
        match expected {
            ParseExpectation::Fails(Some(reason))
                if language.first() != Some(&reason.message()) =>
            {
                failures.push(format!(
                    "{name} v{version} {expr:?}: expected {:?} first, got {language:?}",
                    reason.message()
                ));
            }
            ParseExpectation::Fails(None) => {
                if let Some(reason) = ParseFailure::ALL
                    .into_iter()
                    .find(|reason| language.first() == Some(&reason.message()))
                {
                    failures.push(format!(
                        "{name} v{version} {expr:?}: fails with {:?} first; state it with `.because(ParseFailure::{reason:?})`",
                        reason.message()
                    ));
                }
            }
            _ => {}
        }
        if let ParseExpectation::Fails(Some(reason)) = expected {
            let names = query_names(expr);
            let resolve = names.iter().all(|query| {
                decl(query)
                    .resolve(RawVersion(version), &options.admission, options.experiments)
                    .is_some()
            });
            if names.is_empty() || resolve != (reason == ParseFailure::StringOperand) {
                failures.push(format!("{name} {expr:?} at {version}: queries {names:?} resolve {resolve}, which does not fit {reason:?}"));
            }
        }
    }

    fn check_math_calls(&self, failures: &mut Vec<String>) {
        for (index, row) in self.checked_rows() {
            let name = self.name(index, row);
            match row {
                Row::Eval(eval) => {
                    let Some(call) = parse_call(&eval.expr) else {
                        continue;
                    };
                    let (expr, expected, tolerance) =
                        (&eval.expr, eval.expected, self.tolerance_of(eval));
                    if uses_random(&call) && self.fixed_random.is_none() {
                        failures.push(format!("{name} {expr:?}: an exact value with the default generator is not reproducible"));
                        continue;
                    }
                    let mut rng = self.fixed_random.unwrap_or(FixedRng::ZERO);
                    match evaluate_call(&call, &mut rng) {
                        None => failures.push(format!(
                            "{name} {expr:?}: a bare call the math library cannot evaluate"
                        )),
                        Some(actual) if !within(actual, expected, tolerance) => {
                            failures.push(format!("{name} {expr:?}: math = {actual:e}, expected {expected:e} ± {tolerance:e}"));
                        }
                        Some(_) => {}
                    }
                }
                Row::Range(RangeRow { expr, lo, hi }) => {
                    let Some(call) = parse_call(expr) else {
                        continue;
                    };
                    let (lo, hi) = (lo.min(*hi), lo.max(*hi));
                    let mut sources: Vec<FixedRng> =
                        vec![FixedRng::ZERO, FixedRng::HALF, FixedRng::ONE];
                    if let Some(forced) = self.fixed_random {
                        sources = vec![forced];
                    }
                    for source in &mut sources {
                        match evaluate_call(&call, source) {
                            None => failures.push(format!(
                                "{name} {expr:?}: a bare call the math library cannot evaluate"
                            )),
                            Some(actual) if !(lo..=hi).contains(&actual) => {
                                failures.push(format!("{name} {expr:?}: math = {actual:e}, expected in [{lo:e}, {hi:e}]"));
                            }
                            Some(_) => {}
                        }
                    }
                    let mut generator = Xorshift128::new();
                    for _ in 0..256 {
                        let Some(actual) = evaluate_call(&call, &mut generator) else {
                            break;
                        };
                        if !(lo..=hi).contains(&actual) {
                            failures.push(format!("{name} {expr:?}: math = {actual:e} (xorshift), expected in [{lo:e}, {hi:e}]"));
                            break;
                        }
                    }
                }
                _ => {}
            }
        }
    }

    #[cfg(feature = "vm")]
    fn check_evaluation(&self, failures: &mut Vec<String>) {
        let options = |version: i16| CompileOptions {
            experiments: reference_experiments(),
            ..reference_options(version)
        };
        let mut state = CaseState::new(self);
        for (index, row) in self.rows.iter().enumerate() {
            let name = self.name(index, row);
            match row {
                Row::Setup(setup) => state.apply(setup),
                Row::Eval(eval) => {
                    let (expr, expected, v, tolerance) = (
                        &eval.expr,
                        eval.expected,
                        eval.version,
                        self.tolerance_of(eval),
                    );
                    let compiled = compile(expr, &options(v));
                    let clean = compiled.parses_cleanly() && compiled.is_success();
                    // The expected failure must pass in the state its analysis names, so the
                    // analysis is checked.
                    let explained = eval.failure.as_ref().map(|failure| {
                        let mut alternative = CaseState::new(self);
                        alternative.initial = state.initial.clone();
                        let (variable, value) = &failure.passes_with;
                        alternative.initial.set(VariableName::parse(variable).expect("a variable name"), Value::Float(*value));
                        let (value, _) = alternative.eval(&compiled);
                        matches!(value, Value::Float(actual) if (actual - expected).abs() <= DEFAULT_TOLERANCE)
                    });
                    let (value, messages) = state.eval(&compiled);
                    let passes = matches!(value, Value::Float(actual) if clean && (actual - expected).abs() <= tolerance);
                    match (passes, &eval.failure) {
                        (true, None) => {}
                        (true, Some(_)) => failures.push(format!("{name} {expr:?} is listed as an expected failure but passes")),
                        (false, Some(failure)) => {
                            let as_recorded = matches!(value, Value::Float(actual) if actual.to_bits() == failure.observed.to_bits()) && messages == failure.messages;
                            if !as_recorded {
                                failures.push(format!("{name} {expr:?} fails differently than recorded: {value:?} {messages:?}"));
                            }
                            if explained != Some(true) {
                                failures.push(format!("{name} {expr:?}: the expected failure's analysis does not hold"));
                            }
                        }
                        (false, None) => failures.push(format!("{name} v{v} {expr:?}: {value:?} {messages:?}, expected {expected} ± {tolerance} (clean parse {clean})")),
                    }
                }
                Row::Range(RangeRow { expr, lo, hi }) => {
                    let compiled = compile(expr, &options(13));
                    let clean = compiled.parses_cleanly() && compiled.is_success();
                    let (lo, hi) = (lo.min(*hi), lo.max(*hi));
                    let (value, _) = state.eval(&compiled);
                    if !matches!(value, Value::Float(actual) if clean && (lo..=hi).contains(&actual))
                    {
                        failures.push(format!("{name} {expr:?}: {value:?}, expected in [{lo}, {hi}] (clean parse {clean})"));
                    }
                }
                Row::Hash(HashRow {
                    expr,
                    expected,
                    version: v,
                    ..
                }) => {
                    let compiled = compile(expr, &options(*v));
                    let clean = compiled.parses_cleanly() && compiled.is_success();
                    let (value, _) = state.eval(&compiled);
                    if !(clean && value == Value::Hash(HashedStr::from_u64(*expected))) {
                        failures.push(format!("{name} v{v} {expr:?}: {value:?}, expected the hash {expected:#018x} (clean parse {clean})"));
                    }
                }
                Row::List(ListRow {
                    assertion: ListAssertion::EvaluatesTo(expected),
                    items,
                    version: v,
                }) => {
                    for item in items {
                        let compiled = compile(item, &options(*v));
                        let clean = compiled.parses_cleanly() && compiled.is_success();
                        let (value, messages) = state.eval(&compiled);
                        if !(clean && value == Value::Float(*expected) && messages.is_empty()) {
                            failures.push(format!("{name} v{v} {item:?}: {value:?} {messages:?}, expected {expected} with no message"));
                        }
                    }
                }
                Row::List(ListRow {
                    assertion: ListAssertion::FailsEvaluation,
                    items,
                    version: v,
                }) => {
                    for item in items {
                        let compiled = compile(item, &options(*v));
                        let clean = compiled.parses_cleanly() && compiled.is_success();
                        let (value, messages) = state.eval(&compiled);
                        if !(clean && !messages.is_empty()) {
                            failures.push(format!("{name} v{v} {item:?}: {value:?}, expected a run-time message, got none (clean parse {clean})"));
                        }
                    }
                }
                _ => {}
            }
        }
    }

    fn has_no_setup(&self) -> bool {
        self.fixed_random.is_none()
            && self.actors.is_none()
            && !self.default_set_only
            && self.rows.iter().all(|row| !matches!(row, Row::Setup(_)))
    }

    #[cfg(feature = "vm")]
    fn check_fresh_state(&self, failures: &mut Vec<String>) {
        if !self.fresh_state {
            return;
        }
        for (index, row) in self.checked_rows() {
            let Row::Eval(eval) = row else { continue };
            if let Some(failure) =
                fresh_state_failure(&eval.expr, eval.expected, self.tolerance_of(eval))
            {
                failures.push(format!(
                    "{} (fresh state) {:?}: {failure}",
                    self.name(index, row),
                    eval.expr
                ));
            }
        }
    }

    #[cfg(feature = "vm")]
    fn initial_env(&self) -> Env {
        let mut env = Env::reference();
        env.world.alive.extend([LIVE_ACTOR, SECOND_ACTOR]);
        let baa = VariableName::new("baa");
        let handles: &[u32] = match self.actors {
            None => return env,
            Some(Actors::Removed) => &[11, 12, 13],
            Some(Actors::Mixed) => &[21, 11, 22, 12, 23],
            Some(Actors::Live) => &[21, 22, 23],
        };
        if self.actors != Some(Actors::Removed) {
            env.world.alive.extend([21, 22, 23]);
        }
        env.vars.set(
            baa,
            Value::ActorArray(Arc::new(
                handles.iter().map(|&n| Actor::Handle(n)).collect(),
            )),
        );
        env
    }
}

#[cfg(feature = "vm")]
struct CaseState {
    env: Env,
    /// What a row that assigns starts from: the setup's map plus the setup steps met so far.
    initial: VariableMap<TestHost>,
    rng: Box<dyn Rng>,
}

#[cfg(feature = "vm")]
impl CaseState {
    fn new(case: &EvalCase) -> Self {
        let env = case.initial_env();
        let rng: Box<dyn Rng> = match case.fixed_random {
            Some(fixed) => Box::new(fixed),
            None => Box::new(Xorshift128::new()),
        };
        let initial = env.vars.clone();
        Self { env, initial, rng }
    }

    fn apply(&mut self, setup: &Setup) {
        let env = &mut self.env;
        match setup {
            Setup::RefreshSnapshots => {
                env.vars.refresh_snapshots();
                self.initial.refresh_snapshots();
            }
            Setup::BabyFlag(actor) => {
                env.world.baby.insert(match actor {
                    CaseActor::Live => LIVE_ACTOR,
                    CaseActor::Second => SECOND_ACTOR,
                });
            }
            Setup::Context { name, actor } => {
                let key = ContextName::parse(name)
                    .unwrap_or_else(|| panic!("setup: {name:?} is not a context name"));
                let handle = match actor {
                    ContextActor::Live => LIVE_ACTOR,
                    ContextActor::Null => 0,
                };
                env.context.set(key, Value::Actor(Actor::Handle(handle)));
            }
            Setup::Set {
                variable,
                value,
                public,
            } => {
                let key = VariableName::parse(variable)
                    .unwrap_or_else(|| panic!("setup: {variable:?} is not a variable name"));
                for map in [&mut env.vars, &mut self.initial] {
                    if *public {
                        map.set_public(key, Value::Float(*value));
                    } else {
                        map.set(key, Value::Float(*value));
                    }
                }
            }
        }
    }

    fn eval(&mut self, compiled: &Compiled) -> (Value<TestHost>, Vec<String>) {
        if compiled
            .expr()
            .is_some_and(|e| e.flags().contains(ProgramFlags::HAS_ASSIGNMENT))
        {
            self.env.vars = self.initial.clone();
        }
        let mut sink = CollectSink::new();
        let value = self.env.eval(compiled, self.rng.as_mut(), &mut sink);
        (value, sink.take())
    }
}

/// Temps are not kept; a value within `tolerance` or with the same bits passes.
#[cfg(feature = "vm")]
fn fresh_state_failure(expr: &str, expected: f32, tolerance: f32) -> Option<String> {
    let options = CompileOptions {
        experiments: reference_experiments(),
        ..reference_options(MolangVersion::LATEST.as_i16())
    };
    let compiled = compile(expr, &options);
    let fresh = || {
        let mut env = Env::reference();
        env.temps = None;
        env
    };
    let mut sink = CollectSink::new();
    let value = fresh().eval(&compiled, &mut Xorshift128::new(), &mut sink);
    let clean = compiled.parses_cleanly() && compiled.is_success();
    let passes = matches!(value, Value::Float(actual) if clean && ((actual - expected).abs() <= tolerance || actual.to_bits() == expected.to_bits()));
    (!passes).then(|| {
        format!(
            "{value:?} {:?}, expected {expected} ± {tolerance} (clean parse {clean})",
            sink.take()
        )
    })
}

fn reference_options(version: i16) -> CompileOptions {
    CompileOptions {
        admission: QueryAdmission::Sets(REFERENCE_SETS),
        ..CompileOptions::from_raw_version(reference_catalog().clone(), RawVersion(version))
    }
}

fn owned(items: &[&str]) -> Vec<String> {
    items.iter().map(|item| (*item).to_owned()).collect()
}
