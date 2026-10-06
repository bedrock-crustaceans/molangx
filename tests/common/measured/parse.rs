//! [`ParseGroup`]: each row compiles for the client with no query set, under both `Deviations::ALL`
//! and `Deviations::NONE`.

use crate::common::CheckGuard;
use molangx::catalog::{QueryAdmission, QuerySetMask, Side};
use molangx::compile::{CompileFailure, CompileOptions, Deviations, compile};
use molangx::diag::LanguageMessage;
use molangx::version::RawVersion;

#[derive(Clone, Debug)]
pub struct ParseRow {
    n: u32,
    expr: String,
    version: i16,
    rejected: bool,
    lines: Vec<String>,
}

impl ParseRow {
    /// Default 13.
    pub fn at(&mut self, version: i16) -> &mut Self {
        self.version = version;
        self
    }

    pub fn rejected(&mut self) -> &mut Self {
        self.rejected = true;
        self
    }

    /// In order, at Error level; a row that logs a line has an error.
    pub fn logs(&mut self, lines: &[&str]) -> &mut Self {
        self.lines = lines.iter().map(|line| (*line).to_owned()).collect();
        self
    }
}

#[derive(Debug)]
pub struct ParseGroup {
    group: String,
    guard: CheckGuard,
    rows: Vec<ParseRow>,
}

impl ParseGroup {
    pub fn new(group: &str) -> Self {
        Self {
            group: group.to_owned(),
            guard: CheckGuard::new(group),
            rows: Vec::new(),
        }
    }

    /// `n` is the row's 1-based position in its group.
    pub fn row(&mut self, n: u32, expr: &str) -> &mut ParseRow {
        assert!(
            self.rows.iter().all(|row| row.n != n),
            "{} #{n} repeats",
            self.group
        );
        self.rows.push(ParseRow {
            n,
            expr: expr.to_owned(),
            version: 13,
            rejected: false,
            lines: Vec::new(),
        });
        self.rows.last_mut().expect("just pushed")
    }

    pub fn check(&self, rows: usize) {
        self.guard.checked(self.rows.len(), rows);
        // Only the first line of "found multiple operations…" is compared.
        let multiple_roots = LanguageMessage::MultipleRoots
            .template()
            .lines()
            .next()
            .unwrap_or("");
        let first_line_only = |text: &str| -> String {
            if text.starts_with(multiple_roots) {
                text.lines().next().unwrap_or("").trim().to_owned()
            } else {
                text.trim().to_owned()
            }
        };
        let mut failures = Vec::new();
        for row in &self.rows {
            let theirs: Vec<String> = row.lines.iter().map(|line| first_line_only(line)).collect();
            for deviations in [Deviations::ALL, Deviations::NONE] {
                let options = CompileOptions {
                    admission: QueryAdmission::Sets(QuerySetMask::empty()),
                    deviations,
                    ..CompileOptions::from_raw_version(
                        molangx::stdlib::queries(Side::Client).clone(),
                        RawVersion(row.version),
                    )
                };
                let compiled = compile(&row.expr, &options);
                let ours: Vec<String> = compiled
                    .diagnostics()
                    .iter()
                    .filter(|d| d.language_message().is_some())
                    .map(|d| first_line_only(&d.message()))
                    .collect();
                let rejected = compiled.failure() == Some(CompileFailure::Rejected);
                // Every language message is at Error level.
                let has_error = !ours.is_empty();
                if ours != theirs || rejected != row.rejected || has_error == row.lines.is_empty() {
                    failures.push(format!(
                        "{} #{} v{} {:?} ({deviations:?}): rejected {rejected} (theirs {})\n    ours:   {ours:?}\n    theirs: {theirs:?}",
                        self.group, row.n, row.version, row.expr, row.rejected
                    ));
                }
            }
        }
        assert!(
            failures.is_empty(),
            "{}: {} difference(s):\n{}",
            self.group,
            failures.len(),
            failures.join("\n")
        );
    }
}
