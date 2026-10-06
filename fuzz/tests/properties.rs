//! Property tests over generated programs. `PROPTEST_CASES` overrides each module's case count.

mod common;

use arbitrary::{Arbitrary, Unstructured};
use molangx_fuzz::generator::FuzzCase;
use proptest::prelude::*;

/// `default_cases` cases, or `PROPTEST_CASES`.
fn config(default_cases: u32) -> ProptestConfig {
    let cases = std::env::var("PROPTEST_CASES")
        .ok()
        .and_then(|c| c.parse().ok())
        .unwrap_or(default_cases);
    // No failure file: an integration test has no `lib.rs` / `main.rs` for proptest to put it
    // beside.
    ProptestConfig {
        cases,
        failure_persistence: None,
        ..ProptestConfig::default()
    }
}

fn fuzz_case() -> impl Strategy<Value = FuzzCase> {
    prop::collection::vec(any::<u8>(), 32..600).prop_filter_map("not enough bytes", |bytes| {
        FuzzCase::arbitrary(&mut Unstructured::new(&bytes)).ok()
    })
}

mod lowering {
    //! The compiler lowers the source outside strings and skips whitespace before every token, so
    //! case flips outside strings and other whitespace between tokens change neither the program
    //! nor its evaluation.

    use super::config;
    use crate::common::client_at;
    use arbitrary::{Arbitrary, Unstructured};
    use molangx::compile::{Compiled, Expr, compile};
    use molangx::rng::Xorshift128;
    use molangx::vm::EvalLimits;
    use molangx_fuzz::generator::env::{
        FuzzEnv, FuzzRng, Subject, TempLifetime, same_value, state_difference,
    };
    use molangx_fuzz::generator::{Program, Style};
    use proptest::prelude::*;

    fn program() -> impl Strategy<Value = (Program, Style)> {
        prop::collection::vec(any::<u8>(), 16..400).prop_filter_map("not enough bytes", |bytes| {
            let mut u = Unstructured::new(&bytes);
            let program = Program::arbitrary(&mut u).ok()?;
            let style = Style::arbitrary(&mut u).ok()?;
            Some((program, style))
        })
    }

    /// Flips the case of the letters outside strings that `mask` selects. Strings are delimited as
    /// the lexer does: every `'` toggles, and a `\` exempts the next byte from both the flip and
    /// the quote test.
    fn flip_outside_strings(source: &str, mask: &[bool]) -> String {
        let mut out = String::with_capacity(source.len());
        let mut in_string = false;
        let mut escaped = false;
        for (i, c) in source.chars().enumerate() {
            if escaped {
                escaped = false;
                out.push(c);
                continue;
            }
            match c {
                '\\' => escaped = true,
                '\'' => in_string = !in_string,
                _ => {}
            }
            let flip = !in_string
                && c.is_ascii_alphabetic()
                && mask.get(i % mask.len().max(1)).copied().unwrap_or(false);
            out.push(if !flip {
                c
            } else if c.is_ascii_lowercase() {
                c.to_ascii_uppercase()
            } else {
                c.to_ascii_lowercase()
            });
        }
        out
    }

    /// What a compile produced that lowering must not change.
    fn outcome(compiled: &Compiled, with_spans: bool) -> (String, String, Vec<String>) {
        let program = compiled.expr().map_or_else(String::new, Expr::disassemble);
        let diagnostics = compiled
            .diagnostics()
            .iter()
            .map(|d| {
                if with_spans {
                    format!(
                        "{:?} {:?} {:?} {}",
                        d.code(),
                        d.severity(),
                        d.span(),
                        d.message().to_ascii_lowercase()
                    )
                } else {
                    format!("{:?} {:?}", d.code(), d.severity())
                }
            })
            .collect();
        (format!("{:?}", compiled.failure()), program, diagnostics)
    }

    proptest! {
        #![proptest_config(config(256))]

        #[test]
        fn case_outside_strings_does_not_matter(
            (program, style) in program(),
            mask in prop::collection::vec(any::<bool>(), 1..64),
            version in -1i16..=13,
        ) {
            let source = program.source(&style);
            let flipped = flip_outside_strings(&source, &mask);
            let options = client_at(version);
            let a = outcome(&compile(&source, &options), true);
            let b = outcome(&compile(&flipped, &options), true);
            prop_assert_eq!(a, b, "{:?} vs {:?}", source, flipped);
        }

        #[test]
        fn whitespace_between_tokens_does_not_matter(
            (program, style) in program(),
            cosmetic in prop::collection::vec(any::<u8>(), 0..48),
            version in -1i16..=13,
        ) {
            let source = program.source(&style);
            let respaced = program.source(&style.with_cosmetic(cosmetic));
            let options = client_at(version);
            let a = compile(&source, &options);
            let b = compile(&respaced, &options);
            prop_assert_eq!(outcome(&a, false), outcome(&b, false), "{:?} vs {:?}", source, respaced);
            if let (Some(x), Some(y)) = (&a.expr(), &b.expr()) {
                let limits = EvalLimits { loop_iterations: Some(16), total_steps: Some(2_000), ..EvalLimits::DEFAULT };
                let start = FuzzEnv::new(Subject::Actor, TempLifetime::PerEvaluation, limits, FuzzRng::Xorshift(Xorshift128::new()));
                let (mut first, mut second) = (start.clone(), start);
                let u = x.eval(&mut first.cx());
                let v = y.eval(&mut second.cx());
                prop_assert!(same_value(&u, &v), "{:?}: {:?} vs {:?}: {:?}", source, u, respaced, v);
                prop_assert!(state_difference(&first, &second).is_none(), "{:?} vs {:?}", source, respaced);
            }
        }
    }
}

mod vm_host {
    //! A `FLOAT_ONLY` program starts on the `f32` loop and hands over to the general loop at a
    //! value it cannot hold. Each float-only statement list runs as is and with a string assignment
    //! to an unused temp in front, which forces every instruction onto the general loop; both runs
    //! must give the same bits and leave the same state.
    //!
    //! The step budget is far above the programs' needs; a case that spends it is skipped, since
    //! the added assignment costs steps of its own.

    use super::{config, fuzz_case};
    use molangx::catalog::QueryAdmission;
    use molangx::compile::{CompileOptions, ProgramFlags, compile};
    use molangx::internals::reference_catalog;
    use molangx::version::RawVersion;
    use molangx::vm::{EvalLimits, TempName};
    use molangx_fuzz::generator::env::{FuzzEnv, state_difference};
    use molangx_fuzz::generator::{FuzzCase, Program};
    use proptest::prelude::*;

    const FORCING_TEMP: &str = "zz_general";

    fn case() -> impl Strategy<Value = FuzzCase> {
        fuzz_case().prop_filter("a statement list", |case| {
            matches!(case.program, Program::Complex(_))
        })
    }

    fn options(case: &FuzzCase) -> CompileOptions {
        CompileOptions {
            admission: QueryAdmission::Sets(reference_catalog::SETS),
            experiments: reference_catalog::EXPERIMENTS,
            ..CompileOptions::from_raw_version(
                reference_catalog::catalog().clone(),
                RawVersion(case.version),
            )
        }
    }

    fn check(case: &FuzzCase) -> Result<(), TestCaseError> {
        let source = case.source();
        let compiled = compile(&source, &options(case));
        let Some(expr) = compiled.expr().cloned().filter(|e| {
            compiled.is_success()
                && e.flags().contains(ProgramFlags::FLOAT_ONLY)
                && e.as_constant().is_none()
        }) else {
            return Ok(());
        };
        let forced_source = format!("t.{FORCING_TEMP} = 'general'; {source}");
        let forced = compile(&forced_source, &options(case));
        let forced_expr = forced
            .expr()
            .cloned()
            .ok_or_else(|| TestCaseError::fail(format!("{forced_source:?} does not compile")))?;
        prop_assert_eq!(forced.failure(), None, "{:?}", forced_source);
        prop_assert!(
            !forced_expr.flags().contains(ProgramFlags::FLOAT_ONLY),
            "{:?} is still float-only",
            forced_source
        );

        let limits = EvalLimits {
            loop_iterations: Some(case.loop_iterations),
            total_steps: Some(100_000),
            ..EvalLimits::DEFAULT
        };
        let start = FuzzEnv::new(case.subject, case.temps, limits, case.rng());
        let mut float_env = start.clone();
        let float = expr.eval_f32(&mut float_env.cx());
        let mut general_env = start;
        let general = forced_expr.eval_f32(&mut general_env.cx());
        let spent = |env: &FuzzEnv| {
            env.sink
                .messages
                .iter()
                .any(|m| m.contains("evaluation stopped after its budget"))
        };
        prop_assume!(
            !spent(&float_env) && !spent(&general_env),
            "step budget spent"
        );
        if let Some(temps) = &mut general_env.temps {
            temps.remove(TempName::new(FORCING_TEMP));
        }
        prop_assert!(
            float.to_bits() == general.to_bits(),
            "{source:?}: float loop {float:?} ({:#010x}), general loop {general:?} ({:#010x})",
            float.to_bits(),
            general.to_bits()
        );
        if let Some(why) = state_difference(&float_env, &general_env) {
            return Err(TestCaseError::fail(format!(
                "{source:?}: the two loops leave different states: {why}"
            )));
        }
        Ok(())
    }

    proptest! {
        #![proptest_config(config(2_048))]

        #[test]
        fn float_loop_equals_general_loop(case in case()) {
            check(&case)?;
        }
    }
}

mod vm_tree_walker {
    //! The property-test twin of the `compile_eval` fuzz target.

    use super::{config, fuzz_case};
    use molangx_fuzz::generator::FuzzCase;
    use molangx_fuzz::generator::env::{Verdict, differential};
    use proptest::prelude::*;

    fn check(case: &FuzzCase) -> Result<(), TestCaseError> {
        match differential(case) {
            Ok(Verdict::NotCompiled | Verdict::VmOnly | Verdict::Agree) => Ok(()),
            Err(why) => Err(TestCaseError::fail(why)),
        }
    }

    proptest! {
        #![proptest_config(config(512))]

        #[test]
        fn vm_equals_tree_walker(case in fuzz_case()) {
            check(&case)?;
        }
    }
}
