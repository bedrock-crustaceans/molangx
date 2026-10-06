//! [`ServerRun`]: the probes of one run, replayed against its logs
//! `tests/server_logs/<run>_<release>.log`. Each release is one fresh session in which the probes
//! run in order on one state. [`ServerRun::replay`] panics on a run built wrong (a repeated id,
//! releases that do not match the logs, a state no transition leads to) before any check.
//!
//! A probe answers by reading a never-set `variable.<branch>`; [`Probe::answers`] names that
//! branch, and the first such line is the answer unless [`Probe::continues_after_miss`].
//!
//! The logs print the unbounded maximum of `Malformed … between {} and {}` as
//! `18446744073709551615` and this crate prints `-1`; the checks compare modulo that token.

#[cfg(feature = "vm")]
use crate::common::host::{Env, LevelSink};
use molangx::compile::{CompileFailure, CompileOptions, Deviations, compile};
#[cfg(feature = "vm")]
use molangx::numeric::{ARCH, Arch};
#[cfg(feature = "vm")]
use molangx::rng::Xorshift128;
use molangx::version::MolangVersion;
#[cfg(feature = "vm")]
use molangx::vm::{LogLevel, Value};

use crate::common::compile_support::messages;
use crate::common::{CheckGuard, compile_support::server_at, data_path};

/// A new release needs its arm in `text` and its place at the end of [`Release::ALL`].
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum Release {
    V1_26_36_1,
    V1_26_52_3,
}

impl Release {
    pub const ALL: [Self; 2] = [Self::V1_26_36_1, Self::V1_26_52_3];

    fn from_text(text: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|release| release.text() == text)
    }

    pub const fn text(self) -> &'static str {
        match self {
            Self::V1_26_36_1 => "1.26.36.1",
            Self::V1_26_52_3 => "1.26.52.3",
        }
    }

    const fn slot(self) -> usize {
        self as usize
    }
}

// Every release's slot is its place in `Release::ALL`, which sizes a `PerRelease`.
const _: () = {
    let mut at = 0;
    while at < Release::ALL.len() {
        assert!(
            Release::ALL[at].slot() == at,
            "a release's slot is its place in `Release::ALL`"
        );
        at += 1;
    }
};

pub const FIRST_RELEASE: &[Release] = &[Release::V1_26_36_1];
pub const BOTH_RELEASES: &[Release] = &[Release::V1_26_36_1, Release::V1_26_52_3];

/// On `Arm64`, runs up to this number are replayed and must disagree exactly on the probes they
/// list ([`ServerRun::arm64_differs`]); later runs, including any new one, list none and are not
/// replayed there.
pub const LAST_RUN_REPLAYED_UNDER_ARM64: u32 = 30;

const MISS_START: &str = "Error: unhandled request for unknown variable";
const SERVER_UNBOUNDED: &str = "and 18446744073709551615";
const CRATE_UNBOUNDED: &str = "and -1";

/// A run-time console line; every one is at Error level.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Line {
    /// The unknown-variable message for `variable.<name>`.
    Miss(String),
    /// As printed after `[Molang] `.
    Text(String),
}

impl Line {
    fn rendered(&self) -> String {
        match self {
            Self::Miss(variable) => format!("{MISS_START} 'variable.{variable}'"),
            Self::Text(text) => text.clone(),
        }
    }
}

pub fn miss(variable: &str) -> Line {
    Line::Miss(variable.to_owned())
}

/// As printed after `[Molang] `.
pub fn text(line: &str) -> Line {
    Line::Text(line.to_owned())
}

#[derive(Clone, Debug)]
enum Outcome {
    Answered(Vec<Line>),
    NotRun,
    /// The session ended while the probe ran.
    NotAnswered,
}

/// `ours` is asserted exactly, so an unrelated regression cannot hide behind the entry.
#[derive(Clone, Debug)]
struct ExpectedFailure {
    reason: String,
    ours: Vec<String>,
}

#[derive(Clone, Debug)]
struct PerRelease<T>([Option<T>; Release::ALL.len()]);

impl<T> Default for PerRelease<T> {
    fn default() -> Self {
        Self(std::array::from_fn(|_| None))
    }
}

impl<T> PerRelease<T> {
    fn get(&self, release: Release) -> Option<&T> {
        self.0[release.slot()].as_ref()
    }

    fn set(&mut self, release: Release, value: T) {
        self.0[release.slot()] = Some(value);
    }

    fn releases(&self) -> impl Iterator<Item = Release> + '_ {
        Release::ALL
            .into_iter()
            .filter(|release| self.get(*release).is_some())
    }
}

#[derive(Clone, Debug)]
pub struct Probe {
    id: String,
    expr: String,
    version: i16,
    outcome: Outcome,
    outcome_on: PerRelease<Outcome>,
    load: Vec<String>,
    load_on: PerRelease<Vec<String>>,
    rejected_at_load: bool,
    conclusive: bool,
    state: Option<String>,
    continues_after_miss: bool,
    expected_failures: PerRelease<ExpectedFailure>,
}

impl Probe {
    fn new(id: &str, expr: &str) -> Self {
        Self {
            id: id.to_owned(),
            expr: expr.to_owned(),
            version: 13,
            outcome: Outcome::Answered(Vec::new()),
            outcome_on: PerRelease::default(),
            load: Vec::new(),
            load_on: PerRelease::default(),
            rejected_at_load: false,
            conclusive: true,
            state: None,
            continues_after_miss: false,
            expected_failures: PerRelease::default(),
        }
    }

    /// Default 13.
    pub fn at(&mut self, version: i16) -> &mut Self {
        self.version = version;
        self
    }

    pub fn answers(&mut self, branch: &str) -> &mut Self {
        self.logs(&[miss(branch)])
    }

    pub fn logs(&mut self, lines: &[Line]) -> &mut Self {
        self.outcome = Outcome::Answered(lines.to_vec());
        self
    }

    pub fn silent(&mut self) -> &mut Self {
        self.logs(&[])
    }

    pub fn not_run(&mut self) -> &mut Self {
        self.outcome = Outcome::NotRun;
        self
    }

    pub fn not_answered(&mut self) -> &mut Self {
        self.outcome = Outcome::NotAnswered;
        self
    }

    pub fn answers_on(&mut self, release: Release, branch: &str) -> &mut Self {
        self.logs_on(release, &[miss(branch)])
    }

    pub fn logs_on(&mut self, release: Release, lines: &[Line]) -> &mut Self {
        self.outcome_on
            .set(release, Outcome::Answered(lines.to_vec()));
        self
    }

    pub fn silent_on(&mut self, release: Release) -> &mut Self {
        self.logs_on(release, &[])
    }

    /// The texts after `| <expr> | `, in order.
    pub fn load_logs(&mut self, messages: &[&str]) -> &mut Self {
        self.load = messages.iter().map(|m| (*m).to_owned()).collect();
        self
    }

    pub fn load_logs_on(&mut self, release: Release, messages: &[&str]) -> &mut Self {
        self.load_on
            .set(release, messages.iter().map(|m| (*m).to_owned()).collect());
        self
    }

    pub fn rejected_at_load(&mut self) -> &mut Self {
        self.rejected_at_load = true;
        self
    }

    /// Not counted by [`ServerRun::conclusive_branch`].
    pub fn inconclusive(&mut self) -> &mut Self {
        self.conclusive = false;
        self
    }

    /// The probe is the `on_entry` of controller state `state`, so it runs only if a transition
    /// leads there.
    pub fn in_state(&mut self, state: &str) -> &mut Self {
        self.state = Some(state.to_owned());
        self
    }

    /// The answer is the last unknown-variable line, not the first. Required of a probe that logs
    /// two or more.
    pub fn continues_after_miss(&mut self) -> &mut Self {
        self.continues_after_miss = true;
        self
    }

    /// The probe does not reproduce on `release`; `ours` are the lines logged instead, at Error
    /// level.
    pub fn expected_failure(&mut self, release: Release, reason: &str, ours: &[&str]) -> &mut Self {
        let ours = ours.iter().map(|m| (*m).to_owned()).collect();
        self.expected_failures.set(
            release,
            ExpectedFailure {
                reason: reason.to_owned(),
                ours,
            },
        );
        self
    }

    fn outcome(&self, release: Release) -> &Outcome {
        self.outcome_on.get(release).unwrap_or(&self.outcome)
    }

    fn answered(&self, release: Release) -> Option<&[Line]> {
        match self.outcome(release) {
            Outcome::Answered(lines) => Some(lines),
            Outcome::NotRun | Outcome::NotAnswered => None,
        }
    }

    fn load_lines(&self, release: Release) -> &[String] {
        self.load_on.get(release).unwrap_or(&self.load)
    }

    fn observed_branch(&self, release: Release) -> Option<&str> {
        let mut misses = self
            .answered(release)?
            .iter()
            .filter_map(|line| match line {
                Line::Miss(variable) => Some(variable.as_str()),
                Line::Text(_) => None,
            });
        if self.continues_after_miss {
            misses.next_back()
        } else {
            misses.next()
        }
    }

    fn console_lines(&self, release: Release) -> Vec<String> {
        let mut lines = self.load_lines(release).to_vec();
        lines.extend(
            self.answered(release)
                .unwrap_or_default()
                .iter()
                .map(Line::rendered),
        );
        lines
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ReplayReport {
    /// Counted per release.
    pub not_run: usize,
    /// Counted per release.
    pub expected_failures: usize,
    /// Load-time texts compared with the unbounded maximum replaced.
    pub unbounded_substitutions: usize,
}

#[derive(Debug)]
pub struct ServerRun {
    run: String,
    guard: CheckGuard,
    number: u32,
    releases: Vec<Release>,
    purpose: String,
    format_version: Option<String>,
    transitions: Vec<(String, Vec<(String, String)>)>,
    checks_load: bool,
    arm64_differs: Option<Vec<String>>,
    probes: Vec<Probe>,
}

impl ServerRun {
    /// Panics unless `run` is `run_NN`.
    pub fn new(run: &str, releases: &[Release], purpose: &str) -> Self {
        let number = run
            .strip_prefix("run_")
            .filter(|n| n.len() == 2)
            .and_then(|n| n.parse().ok())
            .unwrap_or_else(|| panic!("{run:?} is not `run_NN`"));
        assert!(!releases.is_empty(), "{run}: no release");
        Self {
            run: run.to_owned(),
            guard: CheckGuard::new(run),
            number,
            releases: releases.to_vec(),
            purpose: purpose.to_owned(),
            format_version: None,
            transitions: Vec::new(),
            checks_load: false,
            arm64_differs: None,
            probes: Vec::new(),
        }
    }

    /// Every probe runs at the version this `format_version` text maps to.
    pub fn controller_format_version(&mut self, text: &str) -> &mut Self {
        self.format_version = Some(text.to_owned());
        self
    }

    /// `(target, condition)` pairs, in order. One call per source state.
    pub fn transitions(&mut self, from: &str, to: &[(&str, &str)]) -> &mut Self {
        assert!(
            self.transitions.iter().all(|(state, _)| state != from),
            "{}: the transitions of {from:?} are given twice",
            self.run
        );
        self.transitions.push((
            from.to_owned(),
            to.iter()
                .map(|(target, condition)| ((*target).to_owned(), (*condition).to_owned()))
                .collect(),
        ));
        self
    }

    /// Also compiles every probe at the latest version under both deviation settings and compares
    /// its load-time messages and rejection.
    pub fn checks_load_messages(&mut self) -> &mut Self {
        self.checks_load = true;
        self
    }

    /// The probes that take another branch under `Arm64`, in run order; required up to
    /// [`LAST_RUN_REPLAYED_UNDER_ARM64`], refused above.
    pub fn arm64_differs(&mut self, ids: &[&str]) -> &mut Self {
        self.arm64_differs = Some(ids.iter().map(|id| (*id).to_owned()).collect());
        self
    }

    /// Version 13, answered with no run-time line until told otherwise.
    pub fn probe(&mut self, id: &str, expr: &str) -> &mut Probe {
        self.probes.push(Probe::new(id, expr));
        self.probes.last_mut().expect("just pushed")
    }

    pub fn conclusive_branch(&self, branch: &str) -> bool {
        self.probes.iter().any(|probe| {
            probe.conclusive
                && self
                    .releases
                    .iter()
                    .any(|release| probe.observed_branch(*release) == Some(branch))
        })
    }

    /// For a test that only reads the run's answers while another test of the same file replays it.
    pub fn read_without_replay(&self) {
        self.guard.used_without_check();
    }

    pub fn replay(&self, probes: usize) -> ReplayReport {
        self.guard.checked(self.probes.len(), probes);
        self.validate();
        let mut report = self.counts();
        let mut failures = Vec::new();
        for &release in &self.releases {
            self.check_log(release, &mut failures);
            self.check_compile(release, &mut failures);
            #[cfg(feature = "vm")]
            if ARCH == Arch::X86_64 {
                self.check_replay(release, &mut failures);
            }
        }
        #[cfg(feature = "vm")]
        if ARCH == Arch::Arm64 && self.number <= LAST_RUN_REPLAYED_UNDER_ARM64 {
            let expected = self.arm64_differs.clone().unwrap_or_default();
            let differs = self.arm64_disagreements();
            if differs != expected {
                failures.push(format!(
                    "{}: under Arm64 the probes {differs:?} disagree; the run lists {expected:?}",
                    self.run
                ));
            }
        }
        if self.checks_load {
            report.unbounded_substitutions = self.check_load(&mut failures);
        }
        assert!(
            failures.is_empty(),
            "{} ({}): {} difference(s):\n{}",
            self.run,
            self.purpose,
            failures.len(),
            failures.join("\n")
        );
        report
    }

    #[cfg(feature = "vm")]
    fn arm64_disagreements(&self) -> Vec<String> {
        let mut ids: Vec<String> = Vec::new();
        for &release in &self.releases {
            for verdict in self.session(release) {
                if !verdict.agrees && !ids.contains(&verdict.probe.id) {
                    ids.push(verdict.probe.id.clone());
                }
            }
        }
        ids
    }

    fn row(&self, release: Release, probe: &Probe) -> String {
        format!("{} {} {}", self.run, release.text(), probe.id)
    }

    fn version_of(&self, probe: &Probe) -> i16 {
        match &self.format_version {
            Some(text) => MolangVersion::from_engine_version_str(text).as_i16(),
            None => probe.version,
        }
    }

    fn validate(&self) {
        let run = &self.run;
        for (at, probe) in self.probes.iter().enumerate() {
            assert!(
                self.probes[..at].iter().all(|other| other.id != probe.id),
                "{run}: probe id {:?} repeats",
                probe.id
            );
            for release in probe
                .outcome_on
                .releases()
                .chain(probe.load_on.releases())
                .chain(probe.expected_failures.releases())
            {
                assert!(
                    self.releases.contains(&release),
                    "{run} {}: an outcome is set for {}, which the run does not have",
                    probe.id,
                    release.text()
                );
            }
            for &release in &self.releases {
                let misses = probe
                    .answered(release)
                    .unwrap_or_default()
                    .iter()
                    .filter(|line| matches!(line, Line::Miss(_)))
                    .count();
                assert!(
                    misses < 2 || probe.continues_after_miss,
                    "{run} {}: logs {misses} unknown-variable lines on {}; a probe that reads on after a miss says so (`continues_after_miss`), and its answer is the last",
                    probe.id,
                    release.text()
                );
            }
            if let Some(state) = &probe.state {
                assert!(
                    self.transitions
                        .iter()
                        .any(|(_, to)| to.iter().any(|(target, _)| target == state)),
                    "{run} {}: no transition leads to {state:?}",
                    probe.id
                );
            }
        }
        if self.number <= LAST_RUN_REPLAYED_UNDER_ARM64 {
            assert!(
                self.arm64_differs.is_some(),
                "{run}: runs up to {LAST_RUN_REPLAYED_UNDER_ARM64} list the probes that disagree under Arm64 (`arm64_differs`)"
            );
        } else {
            assert!(
                self.arm64_differs.is_none(),
                "{run}: only runs up to {LAST_RUN_REPLAYED_UNDER_ARM64} are replayed under Arm64"
            );
        }
        self.validate_logs();
    }

    fn validate_logs(&self) {
        let run = &self.run;
        let directory = data_path("server_logs");
        let prefix = format!("{run}_");
        let mut logged: Vec<String> = std::fs::read_dir(&directory)
            .unwrap_or_else(|e| panic!("cannot read {}: {e}", directory.display()))
            .filter_map(Result::ok)
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .filter_map(|name| {
                name.strip_prefix(&prefix)
                    .and_then(|rest| rest.strip_suffix(".log"))
                    .map(str::to_owned)
            })
            .collect();
        logged.sort();
        for release in &self.releases {
            assert!(
                logged.iter().any(|text| text == release.text()),
                "{run}: the log tests/server_logs/{run}_{}.log of its release {} is missing",
                release.text(),
                release.text()
            );
        }
        for text in &logged {
            assert!(
                Release::from_text(text).is_some_and(|release| self.releases.contains(&release)),
                "{run}: tests/server_logs/{run}_{text}.log is unexpected: the run does not list the release {text}"
            );
        }
    }

    fn counts(&self) -> ReplayReport {
        let mut report = ReplayReport::default();
        for &release in &self.releases {
            for probe in &self.probes {
                report.not_run += usize::from(probe.answered(release).is_none());
                report.expected_failures +=
                    usize::from(probe.expected_failures.get(release).is_some());
            }
        }
        report
    }

    fn check_log(&self, release: Release, failures: &mut Vec<String>) {
        let log = server_log(&self.run, release);
        let at = format!("{} {}", self.run, release.text());
        let mut run_time = Vec::new();
        let mut load_time = Vec::new();
        for line in log.iter().filter(|line| line.contains("[Molang]")) {
            let Some((head, text)) = line.split_once("[Molang] ") else {
                failures.push(format!("{at}: not a Molang console line: {line:?}"));
                continue;
            };
            if head.trim_end().trim_end_matches(']').rsplit(' ').next() != Some("ERROR") {
                failures.push(format!("{at}: not at ERROR level: {line:?}"));
            }
            if !text.contains(" | ") {
                run_time.push(text.trim_end().to_owned());
                continue;
            }
            // The probe a load-time line names is the one with the longest expression that fits.
            let named = self
                .probes
                .iter()
                .filter(|probe| text.contains(&format!("| {} | ", probe.expr)))
                .max_by_key(|probe| probe.expr.len());
            let Some(named) = named else {
                failures.push(format!("{at}: a load-time line names no probe: {line:?}"));
                continue;
            };
            let expr = named.expr.as_str();
            if self
                .probes
                .iter()
                .filter(|probe| probe.expr == expr)
                .all(|probe| probe.answered(release).is_none())
            {
                continue;
            }
            let marker = format!("| {expr} | ");
            let message = &text[text.find(&marker).expect("found above") + marker.len()..];
            load_time.push((expr.to_owned(), message.trim_end().to_owned()));
        }
        let answered: Vec<&Probe> = self
            .probes
            .iter()
            .filter(|probe| probe.answered(release).is_some())
            .collect();
        let expected_run_time: Vec<String> = answered
            .iter()
            .flat_map(|probe| {
                probe
                    .answered(release)
                    .unwrap_or_default()
                    .iter()
                    .map(Line::rendered)
            })
            .collect();
        if run_time != expected_run_time {
            let at_line = run_time
                .iter()
                .zip(&expected_run_time)
                .position(|(a, b)| a != b)
                .unwrap_or(run_time.len().min(expected_run_time.len()));
            failures.push(format!(
                "{at}: the log's run-time lines differ from the rows' from line {at_line} on\n    log:  {:?}\n    rows: {:?}",
                &run_time[at_line.min(run_time.len())..],
                &expected_run_time[at_line.min(expected_run_time.len())..]
            ));
        }
        let mut expected_load_time: Vec<(String, String)> = answered
            .iter()
            .flat_map(|probe| {
                probe
                    .load_lines(release)
                    .iter()
                    .map(|m| (probe.expr.clone(), m.clone()))
            })
            .collect();
        expected_load_time.sort();
        load_time.sort();
        if load_time != expected_load_time {
            let extra: Vec<_> = load_time
                .iter()
                .filter(|line| !expected_load_time.contains(line))
                .collect();
            let missing: Vec<_> = expected_load_time
                .iter()
                .filter(|line| !load_time.contains(line))
                .collect();
            failures.push(format!(
                "{at}: the log's load-time lines differ from the rows'\n    only in the log:  {extra:?}\n    only in the rows: {missing:?}"
            ));
        }
    }

    fn check_compile(&self, release: Release, failures: &mut Vec<String>) {
        for probe in &self.probes {
            if probe.answered(release).is_none() || probe.expected_failures.get(release).is_some() {
                continue;
            }
            let compiled = compile(&probe.expr, &server_at(self.version_of(probe)));
            let ours: Vec<String> = messages(&compiled)
                .into_iter()
                .map(|m| m.replace(CRATE_UNBOUNDED, SERVER_UNBOUNDED))
                .collect();
            if ours != probe.load_lines(release) {
                failures.push(format!(
                    "{} {:?}: compile messages\n    ours:   {ours:?}\n    theirs: {:?}",
                    self.row(release, probe),
                    probe.expr,
                    probe.load_lines(release)
                ));
            }
        }
    }

    #[cfg(feature = "vm")]
    fn check_replay(&self, release: Release, failures: &mut Vec<String>) {
        for verdict in self.session(release) {
            let row = self.row(release, verdict.probe);
            match (verdict.agrees, verdict.probe.expected_failures.get(release)) {
                (true, Some(failure)) => failures.push(format!(
                    "{row} is listed as an expected failure ({}) but reproduces",
                    failure.reason
                )),
                (true, None) => {}
                (false, Some(failure)) => {
                    let recorded: Vec<(LogLevel, String)> = failure
                        .ours
                        .iter()
                        .map(|m| (LogLevel::Error, m.clone()))
                        .collect();
                    if verdict.ours != recorded {
                        failures.push(format!(
                            "{row} ({}) fails differently than recorded\n    ours:     {:?}\n    recorded: {recorded:?}",
                            failure.reason, verdict.ours
                        ));
                    }
                }
                (false, None) => failures.push(format!(
                    "{row} {:?}\n    ours:   {:?}\n    theirs: {:?}",
                    verdict.probe.expr, verdict.ours, verdict.theirs
                )),
            }
        }
    }

    #[cfg(feature = "vm")]
    fn session(&self, release: Release) -> Vec<Verdict<'_>> {
        let mut env = Env::new();
        let mut rng = Xorshift128::new();
        let mut entered: Option<Vec<String>> = None;
        let mut verdicts = Vec::new();
        for probe in &self.probes {
            if probe.answered(release).is_none() {
                continue;
            }
            let compiled = compile(&probe.expr, &server_at(self.version_of(probe)));
            let runs = match &probe.state {
                Some(state) => entered
                    .get_or_insert_with(|| self.entered_states(&mut env, &mut rng))
                    .contains(state),
                None => true,
            };
            let mut ours = Vec::new();
            if runs {
                ours.extend(
                    messages(&compiled)
                        .into_iter()
                        .map(|m| (LogLevel::Error, m)),
                );
                let mut sink = LevelSink::default();
                let _: Value<_> = env.eval(&compiled, &mut rng, &mut sink);
                ours.extend(sink.lines);
            }
            let ours: Vec<(LogLevel, String)> = ours
                .into_iter()
                .map(|(level, m)| (level, m.replace(CRATE_UNBOUNDED, SERVER_UNBOUNDED)))
                .collect();
            let theirs: Vec<(LogLevel, String)> = probe
                .console_lines(release)
                .into_iter()
                .map(|m| (LogLevel::Error, m))
                .collect();
            let mut misses = ours
                .iter()
                .map(|(_, m)| m)
                .filter(|m| m.starts_with(MISS_START));
            let ours_branch = if probe.continues_after_miss {
                misses.next_back()
            } else {
                misses.next()
            };
            let branch = probe
                .observed_branch(release)
                .map(|variable| miss(variable).rendered());
            let agrees = ours == theirs && ours_branch == branch.as_ref();
            verdicts.push(Verdict {
                probe,
                agrees,
                ours,
                theirs,
            });
        }
        verdicts
    }

    /// From `default`, follows the first transition whose condition is true on the session's state.
    /// The bound of 64 steps only guards against a cycle.
    #[cfg(feature = "vm")]
    fn entered_states(&self, env: &mut Env, rng: &mut Xorshift128) -> Vec<String> {
        let mut entered = Vec::new();
        let mut state = "default".to_owned();
        for _ in 0..64 {
            let Some((_, choices)) = self.transitions.iter().find(|(from, _)| *from == state)
            else {
                break;
            };
            let options = server_at(13);
            let next = choices.iter().find_map(|(target, condition)| {
                let expr = compile(condition, &options).expr().cloned()?;
                let mut sink = LevelSink::default();
                env.with_cx(rng, &mut sink, |cx| expr.eval(cx).truthy())
                    .then(|| target.clone())
            });
            let Some(next) = next else {
                break;
            };
            entered.push(next.clone());
            state = next;
        }
        entered
    }

    fn check_load(&self, failures: &mut Vec<String>) -> usize {
        let mut substituted = 0;
        for &release in &self.releases {
            for probe in &self.probes {
                let row = self.row(release, probe);
                if probe.version != 13 {
                    failures.push(format!(
                        "{row}: a load-checked probe runs at version 13, not {}",
                        probe.version
                    ));
                    continue;
                }
                let theirs: Vec<String> = probe
                    .load_lines(release)
                    .iter()
                    .map(|text| {
                        if text.contains(SERVER_UNBOUNDED) {
                            substituted += 1;
                            text.replace(SERVER_UNBOUNDED, CRATE_UNBOUNDED)
                        } else {
                            text.clone()
                        }
                    })
                    .collect();
                for deviations in [Deviations::ALL, Deviations::NONE] {
                    let compiled = compile(
                        &probe.expr,
                        &CompileOptions {
                            deviations,
                            ..CompileOptions::server(MolangVersion::LATEST)
                        },
                    );
                    let ours = messages(&compiled);
                    if ours != theirs {
                        failures.push(format!("{row} {:?} ({deviations:?}): load-time messages\n    ours:   {ours:?}\n    theirs: {theirs:?}", probe.expr));
                    }
                    if (compiled.failure() == Some(CompileFailure::Rejected))
                        != probe.rejected_at_load
                    {
                        let server = if probe.rejected_at_load {
                            "rejected it"
                        } else {
                            "kept it"
                        };
                        failures.push(format!(
                            "{row} {:?} ({deviations:?}): ours {:?}, the server {server}",
                            probe.expr,
                            compiled.failure()
                        ));
                    }
                }
            }
        }
        substituted
    }
}

#[cfg(feature = "vm")]
struct Verdict<'a> {
    probe: &'a Probe,
    agrees: bool,
    ours: Vec<(LogLevel, String)>,
    theirs: Vec<(LogLevel, String)>,
}

fn server_log(run: &str, release: Release) -> Vec<String> {
    let path = data_path(&format!("server_logs/{run}_{}.log", release.text()));
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()));
    text.lines().map(str::to_owned).collect()
}
