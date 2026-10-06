//! The check the `lex_parse` fuzz target runs: arbitrary text through the whole compiler front
//! end.

use molangx::catalog::{QueryAdmission, QuerySetMask, Side};
use molangx::compile::{CompileOptions, CompileOutcome, Deviations, compile};
use molangx::version::RawVersion;

/// The check the `lex_parse` fuzz target runs on one input: `source` compiled under the options
/// `version` and `flags` select must give a consistent result, and the same one twice.
///
/// A rejection has a diagnostic and the constant 0 expression, a success is an expression that
/// parsed, an array or resource outcome parsed and has no expression, every diagnostic's span
/// lies within the source, and a second compile gives the same failure, diagnostics and tree.
///
/// # Panics
///
/// When one of those does not hold: every panic is a finding.
pub fn check(version: u8, flags: u8, source: &str) {
    let options = options(version, flags);
    let compiled = compile(source, &options);
    match compiled.outcome() {
        CompileOutcome::Rejected(expr) => {
            assert!(
                !compiled.diagnostics().is_empty(),
                "rejected without a diagnostic"
            );
            assert!(expr.is_rejected());
            assert_eq!(expr.as_constant(), Some(0.0));
        }
        CompileOutcome::Success(expr) => assert!(!expr.is_rejected() && compiled.parsed()),
        CompileOutcome::UsesArrays | CompileOutcome::UsesResources => {
            assert!(compiled.expr_or_zero().is_none() && compiled.parsed());
        }
        outcome => panic!("an outcome this check does not know: {outcome:?}"),
    }
    for diagnostic in compiled.diagnostics() {
        let span = diagnostic.span();
        assert!(
            span.start <= span.end && span.end as usize <= source.len(),
            "span outside the source"
        );
    }

    let again = compile(source, &options);
    assert_eq!(compiled.failure(), again.failure());
    assert_eq!(compiled.diagnostics(), again.diagnostics());
    assert_eq!(compiled.tree_notation(9), again.tree_notation(9));
}

/// The compile options of an input: the raw version `version % 18 − 2` (−2..=15 covers `Invalid`,
/// every real version and raw values outside the enum), and from the bits of `flags` the client
/// catalogue (1), no deviations (4), every query set (8), and no assignments (16), nor random draws
/// with it (32); bit 2 is unused.
fn options(version: u8, flags: u8) -> CompileOptions {
    let raw_version = RawVersion(i16::from(version % 18) - 2);
    let side = if flags & 1 != 0 {
        Side::Client
    } else {
        Side::Server
    };
    let base =
        CompileOptions::from_raw_version(molangx::stdlib::queries(side).clone(), raw_version);
    let allowed_ops = match (flags & 16 != 0, flags & 32 != 0) {
        (false, _) => base.allowed_ops,
        (true, false) => base.allowed_ops.without_assignments(),
        (true, true) => base.allowed_ops.without_assignments_or_random(),
    };
    CompileOptions {
        deviations: if flags & 4 != 0 {
            Deviations::NONE
        } else {
            base.deviations
        },
        admission: if flags & 8 != 0 {
            QueryAdmission::Sets(QuerySetMask::BUILTIN)
        } else {
            base.admission
        },
        allowed_ops,
        ..base
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use molangx::ops::OpSet;

    fn base(raw: i16, side: Side) -> CompileOptions {
        CompileOptions::from_raw_version(molangx::stdlib::queries(side).clone(), RawVersion(raw))
    }

    #[test]
    fn the_version_byte_covers_every_raw_version_from_minus_two_to_fifteen() {
        assert_eq!(options(0, 0), base(-2, Side::Server));
        assert_eq!(options(15, 0), base(13, Side::Server));
        assert_eq!(options(17, 0), base(15, Side::Server));
        assert_eq!(options(18, 0), base(-2, Side::Server));
        assert_eq!(
            options(255, 0).raw_version,
            RawVersion(i16::from(255 % 18_u8) - 2)
        );
    }

    #[test]
    fn each_flag_bit_sets_its_option() {
        let server = base(13, Side::Server);
        assert_eq!(options(15, 1), base(13, Side::Client));
        assert_eq!(options(15, 2), server);
        assert_eq!(
            options(15, 4),
            CompileOptions {
                deviations: Deviations::NONE,
                ..server.clone()
            }
        );
        assert_eq!(
            options(15, 8),
            CompileOptions {
                admission: QueryAdmission::Sets(QuerySetMask::BUILTIN),
                ..server.clone()
            }
        );
        assert_eq!(
            options(15, 16),
            CompileOptions {
                allowed_ops: OpSet::all().without_assignments(),
                ..server.clone()
            }
        );
        assert_eq!(
            options(15, 48),
            CompileOptions {
                allowed_ops: OpSet::all().without_assignments_or_random(),
                ..server.clone()
            }
        );
        // Without bit 16, bit 32 changes nothing.
        assert_eq!(options(15, 32), server);
        assert_eq!(
            options(15, 1 | 2 | 4 | 8 | 16),
            CompileOptions {
                deviations: Deviations::NONE,
                admission: QueryAdmission::Sets(QuerySetMask::BUILTIN),
                allowed_ops: OpSet::all().without_assignments(),
                ..base(13, Side::Client)
            }
        );
    }

    #[test]
    fn every_compile_outcome_passes_the_check() {
        let cases = [
            ("1 + v.x", None),
            ("1 +", Some(molangx::compile::CompileFailure::Rejected)),
            (
                "array.a[0]",
                Some(molangx::compile::CompileFailure::UsesArrays),
            ),
            (
                "geometry.default",
                Some(molangx::compile::CompileFailure::UsesResources),
            ),
        ];
        for (source, failure) in cases {
            assert_eq!(
                compile(source, &options(15, 0)).failure(),
                failure,
                "{source}"
            );
            for flags in [0, 1, 2, 4, 8, 16, 48, 63] {
                check(15, flags, source);
            }
        }
    }

    #[test]
    fn arbitrary_text_and_any_version_pass_the_check() {
        for source in [
            "",
            ";",
            "((((",
            "v.x = 1; return v.x;",
            "q.foo(1,",
            "'abc",
            "\u{e9}\u{301} + 1",
            "loop(3, {t.a = 1;});",
        ] {
            for version in 0..18 {
                check(version, 0, source);
            }
        }
    }
}
