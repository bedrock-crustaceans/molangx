//! Run rows compile for the server under `Deviations::NONE` and run on this build. [`RunGroup`]
//! rows run in order on one state, so variables and temps persist from row to row; [`LoopCapGroup`]
//! and [`SmokeGroup`] rows each run on a fresh one. On `Arm64` every row holds; on `X86_64` a
//! [`RunGroup`]'s rows on [`RunGroup::x86_64_differs`] differ and the rest hold.

use molangx::compile::{CompileOptions, Deviations, compile};
use molangx::rng::FixedRng;

use super::math_call::{evaluate_call, parse_call, uses_random};
#[cfg(feature = "vm")]
use crate::common::compile_support::messages;
use crate::common::{CheckGuard, compile_support::server_at, nan_or_same_bits};
#[cfg(feature = "vm")]
use molangx::numeric::{ARCH, Arch};

#[cfg(feature = "vm")]
use crate::common::host::{Env, LevelSink};
#[cfg(feature = "vm")]
use molangx::rng::Xorshift128;
#[cfg(feature = "vm")]
use molangx::vm::{EvalLimits, LogLevel, Value, VariableName};

#[cfg(feature = "vm")]
const PASSES: &str = "v.loop_cap_passes";
#[cfg(feature = "vm")]
const STEP_BUDGET: u64 = 100_000_000;

#[derive(Clone, Debug)]
pub struct RunRow {
    seq: u32,
    expr: String,
    expected: f32,
    version: i16,
    lines: Vec<String>,
    clears_variables: bool,
    compiled: bool,
}

impl RunRow {
    /// Default 13.
    pub fn at(&mut self, version: i16) -> &mut Self {
        self.version = version;
        self
    }

    /// In order, at Error level.
    pub fn logs(&mut self, lines: &[&str]) -> &mut Self {
        self.lines = lines.iter().map(|line| (*line).to_owned()).collect();
        self
    }

    /// Clears the variable map before the row; the temps stay.
    pub fn clears_variables(&mut self) -> &mut Self {
        self.clears_variables = true;
        self
    }

    /// The row compiles to no expression.
    pub fn not_compiled(&mut self) -> &mut Self {
        self.compiled = false;
        self
    }
}

#[derive(Debug)]
pub struct RunGroup {
    group: String,
    guard: CheckGuard,
    rows: Vec<RunRow>,
    x86_64_differs: Vec<u32>,
}

impl RunGroup {
    pub fn new(group: &str) -> Self {
        Self {
            group: group.to_owned(),
            guard: CheckGuard::new(group),
            rows: Vec::new(),
            x86_64_differs: Vec::new(),
        }
    }

    /// The rows whose value or lines differ on `X86_64`.
    pub fn x86_64_differs(&mut self, seqs: &[u32]) -> &mut Self {
        self.x86_64_differs = seqs.to_vec();
        self
    }

    /// `expr` gives `expected` to the bit. A row that is one bare `math.<fn>(…)` call must also
    /// give `expected` to the bit from `stdlib::math`.
    pub fn row(&mut self, seq: u32, expr: &str, expected: f32) -> &mut RunRow {
        assert!(
            self.rows.iter().all(|row| row.seq != seq),
            "{} #{seq} repeats",
            self.group
        );
        self.rows.push(RunRow {
            seq,
            expr: expr.to_owned(),
            expected,
            version: 13,
            lines: Vec::new(),
            clears_variables: false,
            compiled: true,
        });
        self.rows.last_mut().expect("just pushed")
    }

    pub fn check(&self, rows: usize) {
        self.guard.checked(self.rows.len(), rows);
        let mut failures = Vec::new();
        for row in &self.rows {
            let name = format!("{} #{}", self.group, row.seq);
            let compiled = compile(&row.expr, &run_options(row.version));
            if compiled.expr().is_some() != row.compiled {
                failures.push(format!(
                    "{name} {:?}: keeps an expression {}, the row was compiled {}",
                    row.expr,
                    compiled.expr().is_some(),
                    row.compiled
                ));
            }
            if let Some(call) = parse_call(&row.expr) {
                assert!(
                    !uses_random(&call),
                    "{name} {:?}: a random call has no exact value",
                    row.expr
                );
                let mut zero = FixedRng::ZERO;
                // A call of the wrong arity (`math.max(3)`) is not the math library's to evaluate.
                if let Some(actual) = evaluate_call(&call, &mut zero)
                    && !nan_or_same_bits(actual, row.expected)
                {
                    failures.push(format!(
                        "{name} {:?}: math = {actual:e} ({:#010x}), expected {:e} ({:#010x})",
                        row.expr,
                        actual.to_bits(),
                        row.expected,
                        row.expected.to_bits()
                    ));
                }
            }
        }
        for seq in &self.x86_64_differs {
            assert!(
                self.rows.iter().any(|row| row.seq == *seq),
                "{}: x86_64_differs names #{seq}, which is not a row",
                self.group
            );
        }
        #[cfg(feature = "vm")]
        self.check_evaluation(&mut failures);
        assert!(
            failures.is_empty(),
            "{}: {} failing row(s):\n{}",
            self.group,
            failures.len(),
            failures.join("\n")
        );
    }

    #[cfg(feature = "vm")]
    fn check_evaluation(&self, failures: &mut Vec<String>) {
        let mut env = Env::reference();
        let mut rng = Xorshift128::new();
        for row in &self.rows {
            if row.clears_variables {
                env.vars.clear();
            }
            let compiled = compile(&row.expr, &run_options(row.version));
            let mut ours: Vec<(LogLevel, String)> = messages(&compiled)
                .into_iter()
                .map(|m| (LogLevel::Error, m))
                .collect();
            let mut sink = LevelSink::language_only();
            let value = env.eval(&compiled, &mut rng, &mut sink);
            ours.extend(sink.lines);
            let theirs: Vec<(LogLevel, String)> = row
                .lines
                .iter()
                .map(|line| (LogLevel::Error, line.trim_end().to_owned()))
                .collect();
            let agrees = matches!(value, Value::Float(x) if nan_or_same_bits(x, row.expected))
                && ours == theirs;
            let listed = ARCH == Arch::X86_64 && self.x86_64_differs.contains(&row.seq);
            if listed && agrees {
                failures.push(format!(
                    "{} #{} {:?}: listed in x86_64_differs, but agrees",
                    self.group, row.seq, row.expr
                ));
            } else if !listed && !agrees {
                failures.push(format!(
                    "{} #{} v{} {:?}: {value:?} (expected {:?} = {:#010x})\n    ours:   {ours:?}\n    theirs: {theirs:?}",
                    self.group,
                    row.seq,
                    row.version,
                    row.expr,
                    row.expected,
                    row.expected.to_bits()
                ));
            }
        }
    }
}

/// One row of the group `loop_cap`.
#[derive(Clone, Debug)]
pub struct LoopCapRow {
    n: u32,
    expr: String,
    iterations: u32,
    loop_passes: u32,
    x86_64_runs_away: bool,
}

impl LoopCapRow {
    /// On `X86_64` the loop runs until the step budget ends the evaluation; on `Arm64` the row
    /// holds.
    pub fn x86_64_runs_until_the_step_budget(&mut self) -> &mut Self {
        self.x86_64_runs_away = true;
        self
    }
}

/// Each row runs with the per-loop guard off and a step budget of 100,000,000, and must log
/// nothing.
#[derive(Debug)]
pub struct LoopCapGroup {
    guard: CheckGuard,
    rows: Vec<LoopCapRow>,
}

impl LoopCapGroup {
    pub fn new() -> Self {
        Self {
            guard: CheckGuard::new("loop_cap"),
            rows: Vec::new(),
        }
    }

    /// `expr` returns `iterations`; `loop_passes` counts the passes of all its loops, inner loops
    /// included.
    pub fn row(
        &mut self,
        n: u32,
        expr: &str,
        iterations: u32,
        loop_passes: u32,
    ) -> &mut LoopCapRow {
        assert!(
            self.rows.iter().all(|row| row.n != n),
            "loop_cap #{n} repeats"
        );
        self.rows.push(LoopCapRow {
            n,
            expr: expr.to_owned(),
            iterations,
            loop_passes,
            x86_64_runs_away: false,
        });
        self.rows.last_mut().expect("just pushed")
    }

    pub fn check(&self, rows: usize) {
        self.guard.checked(self.rows.len(), rows);
        for row in &self.rows {
            row.check();
        }
    }
}

impl LoopCapRow {
    // `self` is used only with `vm`.
    #[cfg_attr(not(feature = "vm"), allow(clippy::unused_self))]
    fn check(&self) {
        let (n, expr) = (self.n, self.expr.as_str());
        let name = format!("loop_cap #{n} {expr:?}");
        let compiled = compile(expr, &run_options(13));
        assert!(
            compiled.expr().is_some(),
            "{name}: does not compile: {:?}",
            compiled.diagnostics()
        );
        #[cfg(feature = "vm")]
        {
            let fresh = || {
                let mut env = Env::reference();
                env.limits = EvalLimits {
                    loop_iterations: None,
                    total_steps: Some(STEP_BUDGET),
                    ..EvalLimits::NONE
                };
                env
            };
            let mut env = fresh();
            // Budget messages included, so a row ended by the step budget fails.
            let mut sink = LevelSink::default();
            let value = env.eval(&compiled, &mut Xorshift128::new(), &mut sink);
            if self.x86_64_runs_away && ARCH == Arch::X86_64 {
                let budget =
                    format!("molangx: evaluation stopped after its budget of {STEP_BUDGET} steps");
                let as_described = value == Value::ZERO && sink.lines == [(LogLevel::Warn, budget)];
                assert!(
                    as_described,
                    "{name}: {value:?} {:?}, expected to run until the step budget",
                    sink.lines
                );
            } else {
                let expected = self.iterations as f32;
                let ok = matches!(value, Value::Float(x) if nan_or_same_bits(x, expected) && x as i64 == i64::from(self.iterations))
                    && sink.lines.is_empty();
                assert!(
                    ok,
                    "{name}: {value:?}, expected {} iterations, messages {:?}",
                    self.iterations, sink.lines
                );
                // Counts the passes with a counter at the start of every loop body, which assumes
                // each `{` opens one.
                assert_eq!(
                    expr.matches('{').count(),
                    expr.matches("loop(").count(),
                    "{name}: a `{{` that opens no loop body"
                );
                let counting = format!(
                    "{PASSES} = 0; {}",
                    expr.replace('{', &format!("{{{PASSES} = {PASSES} + 1; "))
                );
                let counted = compile(&counting, &run_options(13))
                    .expr()
                    .cloned()
                    .expect("the counting form compiles");
                let mut env = fresh();
                let mut sink = LevelSink::default();
                let _: Value<_> =
                    env.with_cx(&mut Xorshift128::new(), &mut sink, |cx| counted.eval(cx));
                let passes = env
                    .vars
                    .get(VariableName::new(&PASSES["v.".len()..]))
                    .cloned();
                assert_eq!(
                    passes,
                    Some(Value::Float(self.loop_passes as f32)),
                    "{name}: loop passes, messages {:?}",
                    sink.lines
                );
            }
        }
    }
}

#[derive(Clone, Debug)]
pub struct SmokeRow {
    n: u32,
    expr: String,
    expected: f32,
}

#[derive(Debug)]
pub struct SmokeGroup {
    guard: CheckGuard,
    rows: Vec<SmokeRow>,
}

impl SmokeGroup {
    pub fn new() -> Self {
        Self {
            guard: CheckGuard::new("smoke"),
            rows: Vec::new(),
        }
    }

    /// `expr` gives `expected` on a fresh state, logging nothing.
    pub fn row(&mut self, n: u32, expr: &str, expected: f32) {
        assert!(self.rows.iter().all(|row| row.n != n), "smoke #{n} repeats");
        self.rows.push(SmokeRow {
            n,
            expr: expr.to_owned(),
            expected,
        });
    }

    pub fn check(&self, rows: usize) {
        self.guard.checked(self.rows.len(), rows);
        for row in &self.rows {
            let name = format!("smoke #{} {:?}", row.n, row.expr);
            let compiled = compile(&row.expr, &run_options(13));
            assert!(
                compiled.expr().is_some(),
                "{name}: does not compile: {:?}",
                compiled.diagnostics()
            );
            #[cfg(feature = "vm")]
            {
                let mut env = Env::reference();
                let mut sink = LevelSink::language_only();
                let value = env.eval(&compiled, &mut Xorshift128::new(), &mut sink);
                assert!(
                    matches!(value, Value::Float(x) if x == row.expected),
                    "{name}: {value:?}, expected {}",
                    row.expected
                );
                assert!(sink.lines.is_empty(), "{name}: {:?}", sink.lines);
            }
        }
    }
}

fn run_options(version: i16) -> CompileOptions {
    CompileOptions {
        deviations: Deviations::NONE,
        ..server_at(version)
    }
}
