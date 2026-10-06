//! Cases that the query declarations alone decide, so they run with no feature. With `compiler`,
//! each `check` also compiles and evaluates the rows as an `EvalCase`.

#[cfg(feature = "compiler")]
use super::measured::{EvalCase, ParseFailure};
use super::{CheckGuard, REFERENCE_SETS, reference_catalog};
use molangx::catalog::{QueryAdmission, QueryAllowList, QueryDecl};
use molangx::stdlib::query;
use molangx::version::{ExperimentMask, RawVersion};

/// The real queries that stand for the placeholder queries of the `has_disallowed_queries` rows.
pub const PLACEHOLDERS: [(&str, &str); 4] = [
    ("query.also_disallowed", query::VARIANT),
    ("query.also_allowed", query::HAD_COMPONENT_GROUP),
    ("query.disallowed", query::IS_BABY),
    ("query.allowed", query::BLOCK_STATE),
];

pub const ALLOWED: [&str; 2] = [query::BLOCK_STATE, query::HAD_COMPONENT_GROUP];

pub fn substitute(expr: &str) -> String {
    PLACEHOLDERS
        .iter()
        .fold(expr.to_owned(), |text, (from, to)| text.replace(from, to))
}

pub fn query_names(expr: &str) -> Vec<String> {
    let mut names = Vec::new();
    let mut rest = expr;
    while let Some(at) = rest.find("query.") {
        let tail = &rest[at + "query.".len()..];
        let len = tail
            .bytes()
            .take_while(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || *b == b'_')
            .count();
        names.push(format!("query.{}", &tail[..len]));
        rest = &tail[len..];
    }
    names
}

pub fn decl(name: &str) -> &'static QueryDecl {
    reference_catalog()
        .get(name)
        .unwrap_or_else(|| panic!("{name} is not in the reference catalogue"))
}

/// A case of the version-window helper queries: a `parses` / `parse_fails` row is decided by
/// whether its queries resolve at its version; a list row must hold no query.
#[derive(Debug)]
pub struct VersionWindowCase {
    id: String,
    guard: CheckGuard,
    rows: Vec<VersionRow>,
}

#[derive(Clone, Debug)]
enum WindowAssertion {
    Parses(String),
    ParseFails(String),
    AllParse(Vec<String>),
}

#[derive(Clone, Debug)]
pub struct VersionRow {
    assertion: WindowAssertion,
    version: i16,
}

impl VersionRow {
    /// Default 13.
    pub fn at(&mut self, version: i16) -> &mut Self {
        self.version = version;
        self
    }
}

impl VersionWindowCase {
    pub fn new(id: &str) -> Self {
        Self {
            id: id.to_owned(),
            guard: CheckGuard::new(id),
            rows: Vec::new(),
        }
    }

    pub fn parses(&mut self, expr: &str) -> &mut VersionRow {
        self.push(WindowAssertion::Parses(expr.to_owned()))
    }

    pub fn parse_fails(&mut self, expr: &str) -> &mut VersionRow {
        self.push(WindowAssertion::ParseFails(expr.to_owned()))
    }

    pub fn all_parse(&mut self, items: &[&str]) -> &mut VersionRow {
        assert!(!items.is_empty(), "{}: an empty list row", self.id);
        self.push(WindowAssertion::AllParse(
            items.iter().map(|item| (*item).to_owned()).collect(),
        ))
    }

    fn push(&mut self, assertion: WindowAssertion) -> &mut VersionRow {
        self.rows.push(VersionRow {
            assertion,
            version: 13,
        });
        self.rows.last_mut().expect("just pushed")
    }

    /// `rows` counts a list row once per item.
    pub fn check(&self, rows: usize) {
        let held = self
            .rows
            .iter()
            .map(|row| {
                if let WindowAssertion::AllParse(items) = &row.assertion {
                    items.len()
                } else {
                    1
                }
            })
            .sum();
        self.guard.checked(held, rows);
        let mut failures = Vec::new();
        for (index, row) in self.rows.iter().enumerate() {
            let name = format!("{} row {index}", self.id);
            let version = row.version;
            match &row.assertion {
                WindowAssertion::Parses(expr) | WindowAssertion::ParseFails(expr) => {
                    let parses = matches!(row.assertion, WindowAssertion::Parses(_));
                    let names = query_names(expr);
                    let resolve = names.iter().all(|query| {
                        decl(query)
                            .resolve(
                                RawVersion(version),
                                &QueryAdmission::Sets(REFERENCE_SETS),
                                ExperimentMask::empty(),
                            )
                            .is_some()
                    });
                    if names.is_empty() || resolve != parses {
                        failures.push(format!("{name} {expr:?} at {version}: queries {names:?} resolve {resolve}, expected {parses}"));
                    }
                }
                WindowAssertion::AllParse(items) => {
                    for item in items.iter().filter(|item| !query_names(item).is_empty()) {
                        failures.push(format!(
                            "{name} {item:?}: a list row of a version-window case holds no query"
                        ));
                    }
                }
            }
        }
        assert!(
            failures.is_empty(),
            "{}: {} failing row(s):\n{}",
            self.id,
            failures.len(),
            failures.join("\n")
        );
        #[cfg(feature = "compiler")]
        {
            let mut case = EvalCase::new(&self.id);
            for row in &self.rows {
                match &row.assertion {
                    WindowAssertion::Parses(expr) => {
                        case.parses(expr).at(row.version);
                    }
                    WindowAssertion::ParseFails(expr) => {
                        case.parse_fails(expr)
                            .at(row.version)
                            .because(ParseFailure::UnresolvedQuery);
                    }
                    WindowAssertion::AllParse(items) => {
                        let items: Vec<&str> = items.iter().map(String::as_str).collect();
                        case.all_parse(true, &items).at(row.version);
                    }
                }
            }
            case.check(rows);
        }
    }
}

/// A case decided by whether the queries a row names (placeholders replaced) resolve against
/// [`ALLOWED`].
#[derive(Debug)]
pub struct AllowListCase {
    id: String,
    guard: CheckGuard,
    rows: Vec<(bool, Vec<String>)>,
}

impl AllowListCase {
    pub fn new(id: &str) -> Self {
        Self {
            id: id.to_owned(),
            guard: CheckGuard::new(id),
            rows: Vec::new(),
        }
    }

    pub fn has_disallowed_queries(&mut self, expected: bool, items: &[&str]) {
        assert!(!items.is_empty(), "{}: an empty list row", self.id);
        self.rows.push((
            expected,
            items.iter().map(|item| (*item).to_owned()).collect(),
        ));
    }

    /// `rows` counts a row once per item.
    pub fn check(&self, rows: usize) {
        self.guard
            .checked(self.rows.iter().map(|(_, items)| items.len()).sum(), rows);
        let allowed = QueryAllowList::new(reference_catalog(), ALLOWED).expect("declared");
        let mut failures = Vec::new();
        for (index, (expected, items)) in self.rows.iter().enumerate() {
            for item in items {
                let names = query_names(item);
                let disallowed = names.iter().any(|query| {
                    let real = PLACEHOLDERS
                        .iter()
                        .find(|(placeholder, _)| placeholder == query)
                        .map_or(query.as_str(), |(_, real)| real);
                    decl(real)
                        .resolve(
                            RawVersion(13),
                            &QueryAdmission::Only(allowed.clone()),
                            ExperimentMask::empty(),
                        )
                        .is_none()
                });
                if disallowed != *expected {
                    failures.push(format!("{} row {index} {item:?}: queries {names:?}, a disallowed one {disallowed}, expected {expected}", self.id));
                }
            }
        }
        assert!(
            failures.is_empty(),
            "{}: {} failing row(s):\n{}",
            self.id,
            failures.len(),
            failures.join("\n")
        );
        #[cfg(feature = "compiler")]
        {
            let mut case = EvalCase::new(&self.id);
            for (expected, items) in &self.rows {
                let items: Vec<&str> = items.iter().map(String::as_str).collect();
                case.has_disallowed_queries(*expected, &items);
            }
            case.check(rows);
        }
    }
}
