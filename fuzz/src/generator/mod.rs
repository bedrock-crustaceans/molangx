//! A grammar-aware Molang program generator, generators of the serialised forms, and the
//! differential check of one case.
//!
//! A rejected text compares as the constant 0, so a generator rejected too often compares nothing:
//! about one program in 16 is *loose* and the printer sometimes drops a required parenthesis or
//! `;`, which keeps about 98 % of the texts compiling while exercising the error paths.

pub mod env;
pub mod front_end;

mod ast;
mod pools;
mod print;
mod style;

pub use ast::{BinOp, Ex, Ns, Program, Stmt, Var};
pub use style::Style;

use arbitrary::{Arbitrary, Result, Unstructured};
use env::{FuzzRng, Subject, TempLifetime};
use molangx::json::{JsonScalar, MolangSource, MolangValueRepr, OtherJson, ReaderKind, SourceForm};
use molangx::rng::{FixedRng, Xorshift128};
use molangx::version::RawVersion;
use molangx::vm::EvalLimits;
use std::sync::Arc;

/// The struct width budget of a case that does not draw a small one.
const DEFAULT_STRUCT_MEMBERS: u32 = EvalLimits::DEFAULT_STRUCT_MEMBERS;

/// One differential run: a program, how it is printed, and the options it is compiled and
/// evaluated with.
#[derive(Clone, Debug, PartialEq)]
pub struct FuzzCase {
    /// The program.
    pub program: Program,
    /// How it is printed.
    pub style: Style,
    /// The raw `MolangVersion` (−2 … 15: below, within and above the defined range).
    pub version: i16,
    /// How long `temp.*` lives.
    pub temps: TempLifetime,
    /// The per-evaluation step budget.
    pub steps: u64,
    /// The per-loop iteration budget.
    pub loop_iterations: u32,
    /// The struct width budget: small enough that the generated programs reach it, or the
    /// default.
    pub struct_members: u32,
    /// The query-argument nesting budget: small enough that the generated programs reach it, or
    /// none.
    pub query_depth: Option<u32>,
    /// The forced random word, or `None` for the xorshift sequence.
    pub forced_random: Option<FixedRng>,
    /// Whom the expression runs for.
    pub subject: Subject,
}

impl<'a> Arbitrary<'a> for FuzzCase {
    fn arbitrary(u: &mut Unstructured<'a>) -> Result<Self> {
        let version = u.int_in_range(-2..=15i16)?;
        // Drawn and unused, so a recorded input keeps giving the same case.
        let _: bool = u.arbitrary()?;
        let temps = u.arbitrary()?;
        let steps = steps(u)?;
        let loop_iterations = loop_iterations(u)?;
        let struct_members = struct_members(u)?;
        let query_depth = query_depth(u)?;
        let forced_random = forced_random(u)?;
        let subject = u.arbitrary()?;
        let program = Program::generate(u, Some(version))?;
        let style = u.arbitrary()?;
        Ok(Self {
            program,
            style,
            version,
            temps,
            steps,
            loop_iterations,
            struct_members,
            query_depth,
            forced_random,
            subject,
        })
    }
}

fn steps(u: &mut Unstructured<'_>) -> Result<u64> {
    Ok(match u.int_in_range(0..=3u8)? {
        0 => u.int_in_range(0..=64u64)?,
        1 => u.int_in_range(0..=2_000u64)?,
        // Small enough that no value grows without bound.
        _ => 5_000,
    })
}

fn loop_iterations(u: &mut Unstructured<'_>) -> Result<u32> {
    Ok(match u.int_in_range(0..=3u8)? {
        0 => 0,
        1 => u.int_in_range(1..=5u32)?,
        _ => 64,
    })
}

fn struct_members(u: &mut Unstructured<'_>) -> Result<u32> {
    Ok(match u.int_in_range(0..=3u8)? {
        0 => u.int_in_range(0..=3u32)?,
        _ => DEFAULT_STRUCT_MEMBERS,
    })
}

fn query_depth(u: &mut Unstructured<'_>) -> Result<Option<u32>> {
    Ok(match u.int_in_range(0..=3u8)? {
        0 => Some(u.int_in_range(0..=2u32)?),
        _ => None,
    })
}

fn forced_random(u: &mut Unstructured<'_>) -> Result<Option<FixedRng>> {
    Ok(match u.int_in_range(0..=5u8)? {
        0 => Some(FixedRng::ZERO),
        1 => Some(FixedRng::HALF),
        2 => Some(FixedRng::ONE),
        3 => Some(FixedRng(u.arbitrary()?)),
        _ => None,
    })
}

impl FuzzCase {
    /// The source text of the case.
    pub fn source(&self) -> String {
        self.program.source(&self.style)
    }

    /// The budgets of the case; the struct depth budget is always the default.
    pub fn limits(&self) -> EvalLimits {
        EvalLimits {
            loop_iterations: Some(self.loop_iterations),
            total_steps: Some(self.steps),
            struct_depth: Some(EvalLimits::DEFAULT_STRUCT_DEPTH),
            struct_members: Some(self.struct_members),
            query_depth: self.query_depth,
        }
    }

    /// The random source the case starts with.
    pub fn rng(&self) -> FuzzRng {
        match self.forced_random {
            Some(fixed) => FuzzRng::Fixed(fixed),
            None => FuzzRng::Xorshift(Xorshift128::new()),
        }
    }
}

fn text(u: &mut Unstructured<'_>) -> Result<Arc<str>> {
    let s: &str = u.arbitrary()?;
    Ok(Arc::from(s))
}

/// Every form, with any version: a string with or without its context version, an object with
/// any `i16`.
fn source_form(u: &mut Unstructured<'_>) -> Result<SourceForm> {
    Ok(if u.arbitrary()? {
        SourceForm::String {
            context_version: u.arbitrary::<Option<i16>>()?.map(RawVersion),
        }
    } else {
        SourceForm::Object {
            version: RawVersion(u.arbitrary()?),
        }
    })
}

/// A source built through the constructors, so every value is one the crate can produce.
fn molang_source(u: &mut Unstructured<'_>) -> Result<MolangSource> {
    let text = text(u)?;
    Ok(match source_form(u)? {
        SourceForm::String {
            context_version: Some(version),
        } => MolangSource::string(text, version),
        SourceForm::String {
            context_version: None,
        } => MolangSource::string_without_context(text),
        SourceForm::Object { version } => MolangSource::object(text, version),
        form => unreachable!("`source_form` does not draw {form:?}"),
    })
}

/// A constant (any bit pattern), a bool or a source.
fn value_repr(u: &mut Unstructured<'_>) -> Result<MolangValueRepr> {
    Ok(match u.int_in_range(0..=2u8)? {
        0 => MolangValueRepr::Const(f32::from_bits(u.arbitrary()?)),
        1 => MolangValueRepr::Bool(u.arbitrary()?),
        _ => MolangValueRepr::Expr(molang_source(u)?),
    })
}

/// Any scalar: a number (any bit pattern), a bool, a string, the versioned object with any `i16`,
/// or another type.
fn json_scalar(u: &mut Unstructured<'_>) -> Result<JsonScalar> {
    Ok(match u.int_in_range(0..=4u8)? {
        0 => JsonScalar::Number(f64::from_bits(u.arbitrary()?)),
        1 => JsonScalar::Bool(u.arbitrary()?),
        2 => JsonScalar::String(text(u)?),
        3 => JsonScalar::Object {
            expression: text(u)?,
            version: RawVersion(u.arbitrary()?),
        },
        _ => {
            JsonScalar::Other(*u.choose(&[OtherJson::Null, OtherJson::Array, OtherJson::Object])?)
        }
    })
}

/// The input of the `repr_roundtrip` target: a value, a JSON scalar and a context version.
#[derive(Clone, Debug)]
pub struct ReprCase {
    /// The value written and read back.
    pub value: MolangValueRepr,
    /// The JSON read by every reader.
    pub json: JsonScalar,
    /// The context version of the reads.
    pub context: i16,
}

impl<'a> Arbitrary<'a> for ReprCase {
    fn arbitrary(u: &mut Unstructured<'a>) -> Result<Self> {
        Ok(Self {
            value: value_repr(u)?,
            json: json_scalar(u)?,
            context: u.arbitrary()?,
        })
    }
}

/// Every reader, for the round-trip checks.
pub const READERS: [ReaderKind; 5] = [
    ReaderKind::StrictVersioned,
    ReaderKind::LenientVersioned,
    ReaderKind::BiomeHeightRange,
    ReaderKind::ScalarOrArray,
    ReaderKind::SchemaValidated,
];

/// Bits of a float, NaN normalised: for comparing values where any NaN equals any NaN.
pub fn normalised_bits(x: f32) -> u32 {
    if x.is_nan() {
        f32::NAN.to_bits()
    } else {
        x.to_bits()
    }
}

#[cfg(test)]
pub(crate) mod test_support {
    use crate::generator::{
        FuzzCase,
        ast::{Ex, Ns, Program, Var},
        style::Style,
    };
    use arbitrary::{Arbitrary, Unstructured};

    /// `len` pseudo-random bytes from `seed` (xorshift64).
    pub(crate) fn buffer(seed: u64, len: usize) -> Vec<u8> {
        let mut state = seed.wrapping_mul(0x9e37_79b9_7f4a_7c15) | 1;
        (0..len)
            .map(|_| {
                state ^= state << 13;
                state ^= state >> 7;
                state ^= state << 17;
                (state >> 24) as u8
            })
            .collect()
    }

    pub(crate) fn program_of(data: &[u8]) -> Program {
        Program::arbitrary(&mut Unstructured::new(data)).expect("generation is total")
    }

    pub(crate) fn case_of(data: &[u8]) -> FuzzCase {
        FuzzCase::arbitrary(&mut Unstructured::new(data)).expect("generation is total")
    }

    pub(crate) fn n(i: u8) -> Ex {
        Ex::Num(i)
    }

    #[allow(clippy::unnecessary_box_returns)]
    pub(crate) fn b(ex: Ex) -> Box<Ex> {
        Box::new(ex)
    }

    pub(crate) fn var(ns: Ns, name: u8, members: &[u8]) -> Var {
        Var {
            ns,
            name,
            members: members.to_vec(),
        }
    }

    pub(crate) fn vx() -> Var {
        var(Ns::Entity, 0, &[])
    }

    pub(crate) fn simple(ex: Ex) -> Program {
        Program::Simple(ex)
    }

    pub(crate) fn style(structure: &[u8], cosmetic: &[u8]) -> Style {
        Style {
            structure: structure.to_vec(),
            cosmetic: cosmetic.to_vec(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::generator::test_support::*;
    use std::collections::BTreeSet;

    #[test]
    fn normalised_bits_collapses_nans_only() {
        let nan = f32::NAN.to_bits();
        for payload in [
            0x7fc0_0000u32,
            0x7fc0_0001,
            0xffc0_0000,
            0x7f80_0001,
            0xffff_ffff,
        ] {
            assert!(f32::from_bits(payload).is_nan());
            assert_eq!(
                normalised_bits(f32::from_bits(payload)),
                nan,
                "{payload:#x}"
            );
        }
        assert_eq!(normalised_bits(0.0), 0);
        assert_eq!(normalised_bits(-0.0), 0x8000_0000);
        assert_eq!(normalised_bits(1.5), 1.5f32.to_bits());
        assert_eq!(normalised_bits(f32::INFINITY), f32::INFINITY.to_bits());
        assert_eq!(
            normalised_bits(f32::NEG_INFINITY),
            f32::NEG_INFINITY.to_bits()
        );
        assert_ne!(
            normalised_bits(f32::INFINITY),
            normalised_bits(f32::NEG_INFINITY)
        );
        assert_eq!(
            normalised_bits(f32::MIN_POSITIVE / 2.0),
            (f32::MIN_POSITIVE / 2.0).to_bits(),
            "a subnormal is not a NaN"
        );
    }

    #[test]
    fn the_case_of_an_empty_buffer_takes_the_first_value_of_every_range() {
        let case = case_of(&[]);
        assert_eq!(case.version, -2);
        assert_eq!(case.temps, TempLifetime::PerEvaluation);
        assert_eq!(case.subject, Subject::Detached);
        assert_eq!(case.steps, 0);
        assert_eq!(case.loop_iterations, 0);
        assert_eq!(case.query_depth, Some(0));
        assert_eq!(case.forced_random, Some(FixedRng::ZERO));
        assert_eq!(case.program, simple(n(0)));
        assert_eq!(case.style, Style::default());
        assert_eq!(case.source(), "0");
    }

    #[test]
    fn the_struct_width_of_an_empty_case_is_zero() {
        assert_eq!(case_of(&[]).struct_members, 0);
    }

    #[test]
    fn every_limit_of_a_case_stays_in_its_range() {
        let mut seen_random = [false; 5];
        let mut seen_steps = [false; 3];
        let mut seen_loops = [false; 3];
        for seed in 0..2000 {
            let case = case_of(&buffer(seed, 120));
            assert!(
                (-2..=15).contains(&case.version),
                "seed {seed}: {}",
                case.version
            );
            assert!(case.steps <= 5000, "seed {seed}: {}", case.steps);
            seen_steps[usize::from(case.steps == 5000)] = true;
            seen_steps[2] |= case.steps > 64 && case.steps < 5000;
            assert!(
                matches!(case.loop_iterations, 0..=5 | 64),
                "seed {seed}: {}",
                case.loop_iterations
            );
            seen_loops[usize::from(case.loop_iterations == 64)] = true;
            seen_loops[2] |= (1..=5).contains(&case.loop_iterations);
            assert!(
                case.struct_members <= 3
                    || case.struct_members == EvalLimits::DEFAULT_STRUCT_MEMBERS,
                "seed {seed}: {}",
                case.struct_members
            );
            assert!(
                case.query_depth.is_none_or(|depth| depth <= 2),
                "seed {seed}: {:?}",
                case.query_depth
            );
            match case.forced_random {
                None => seen_random[0] = true,
                Some(FixedRng::ZERO) => seen_random[1] = true,
                Some(FixedRng::HALF) => seen_random[2] = true,
                Some(FixedRng::ONE) => seen_random[3] = true,
                Some(_) => seen_random[4] = true,
            }
            assert_eq!(case.source(), case.program.source(&case.style));
        }
        assert_eq!(
            seen_random, [true; 5],
            "every kind of forced sample is generated"
        );
        assert_eq!(seen_steps, [true; 3]);
        assert_eq!(seen_loops, [true; 3]);
    }

    #[test]
    fn every_version_of_the_range_is_generated() {
        let mut seen = BTreeSet::new();
        for seed in 0..3000 {
            seen.insert(case_of(&buffer(seed, 40)).version);
        }
        assert_eq!(seen.len(), 18, "{seen:?}");
        assert_eq!((seen.first(), seen.last()), (Some(&-2), Some(&15)));
    }

    #[test]
    fn a_case_compares_by_every_field() {
        let base = case_of(&buffer(3, 80));
        assert_eq!(base, base.clone());
        let mut other = base.clone();
        other.steps += 1;
        assert_ne!(base, other);
        let mut other = base.clone();
        other.style.cosmetic.push(1);
        assert_ne!(base, other);
        let mut other = base.clone();
        other.subject = match other.subject {
            Subject::Actor => Subject::Detached,
            Subject::Detached => Subject::Actor,
        };
        assert_ne!(base, other);
    }

    #[test]
    fn the_readers_are_five_distinct_kinds() {
        for (i, a) in READERS.iter().enumerate() {
            for b in &READERS[i + 1..] {
                assert_ne!(a, b);
            }
        }
        assert_eq!(READERS[0], ReaderKind::StrictVersioned);
        assert_eq!(READERS[1], ReaderKind::LenientVersioned);
        assert_eq!(READERS[4], ReaderKind::SchemaValidated);
    }

    #[test]
    fn the_arbitrary_forms_reach_every_variant() {
        let mut forms = BTreeSet::new();
        let mut scalar_arms = BTreeSet::new();
        let mut value_arms = BTreeSet::new();
        for seed in 0..600 {
            let data = buffer(seed, 64);
            forms.insert(
                match source_form(&mut Unstructured::new(&data)).expect("total") {
                    SourceForm::String {
                        context_version: Some(_),
                    } => "string",
                    SourceForm::String {
                        context_version: None,
                    } => "string without context",
                    SourceForm::Object { .. } => "object",
                    _ => "another",
                },
            );
            let scalar = json_scalar(&mut Unstructured::new(&data)).expect("total");
            scalar_arms.insert(match scalar {
                JsonScalar::Number(_) => "number",
                JsonScalar::Bool(_) => "bool",
                JsonScalar::String(_) => "string",
                JsonScalar::Object { .. } => "object",
                JsonScalar::Other(_) => "other",
                _ => "another",
            });
            let value = value_repr(&mut Unstructured::new(&data)).expect("total");
            value_arms.insert(match value {
                MolangValueRepr::Const(_) => "const",
                MolangValueRepr::Bool(_) => "bool",
                MolangValueRepr::Expr(_) => "expr",
                _ => "another",
            });
            let source = molang_source(&mut Unstructured::new(&data)).expect("total");
            assert_eq!(
                source,
                molang_source(&mut Unstructured::new(&data)).expect("total")
            );
        }
        assert_eq!(forms.len(), 3, "{forms:?}");
        assert_eq!(scalar_arms.len(), 5, "{scalar_arms:?}");
        assert_eq!(value_arms.len(), 3, "{value_arms:?}");
    }

    #[test]
    fn the_arbitrary_forms_are_total_on_an_empty_buffer() {
        let empty = || Unstructured::new(&[]);
        assert_eq!(
            source_form(&mut empty()).expect("total"),
            SourceForm::Object {
                version: RawVersion(0)
            }
        );
        assert!(value_repr(&mut empty()).is_ok());
        assert!(json_scalar(&mut empty()).is_ok());
        assert!(molang_source(&mut empty()).is_ok());
    }

    #[test]
    fn a_nan_constant_is_generated_with_any_payload_and_compared_by_the_helper() {
        let mut nans = BTreeSet::new();
        for seed in 0..4000 {
            if let Ok(MolangValueRepr::Const(x)) =
                value_repr(&mut Unstructured::new(&buffer(seed, 8)))
                && x.is_nan()
            {
                nans.insert(x.to_bits());
                assert_eq!(normalised_bits(x), f32::NAN.to_bits());
            }
        }
        assert!(
            nans.len() > 1,
            "a NaN with several payloads is generated: {nans:?}"
        );
    }

    /// What the compiler makes of generated programs: how many it accepts, why it rejects the
    /// others, and how many of those it accepts fold to a constant.
    mod acceptance {
        use super::*;
        use molangx::compile::{CompileFailure, compile};

        use molangx::diag::Severity;

        use std::collections::BTreeMap;
        use std::sync::OnceLock;

        /// The seeds each test draws its cases from, with a buffer length that varies as proptest's
        /// does (32 to 600).
        const SEEDS: u64 = 6000;

        #[derive(Default)]
        struct Tally {
            accepted: u32,
            rejected: u32,
            /// Accepted, and a constant after folding: the result does not depend on the
            /// environment.
            constant: u32,
            /// Accepted, with a pure and with a volatile host math call left to run.
            host_calls: [u32; 2],
            /// The first error of each rejected program, by language message.
            why: BTreeMap<String, u32>,
        }

        impl Tally {
            fn total(&self) -> u32 {
                self.accepted + self.rejected
            }
        }

        /// Every case of the seed range, compiled the way the differential check compiles it,
        /// tallied by its version. The three tests read the same tally: it is made once, by
        /// whichever runs first.
        fn tally() -> &'static BTreeMap<i16, Tally> {
            static TALLY: OnceLock<BTreeMap<i16, Tally>> = OnceLock::new();
            TALLY.get_or_init(compile_all)
        }

        fn compile_all() -> BTreeMap<i16, Tally> {
            let mut by_version: BTreeMap<i16, Tally> = BTreeMap::new();
            for seed in 0..SEEDS {
                let case = case_of(&buffer(seed, 32 + (seed.wrapping_mul(7919) % 568) as usize));
                let options = molangx::internals::reference_catalog::options(case.version);
                let compiled = compile(&case.source(), &options);
                let tally = by_version.entry(case.version).or_default();
                if compiled.failure() == Some(CompileFailure::Rejected) {
                    tally.rejected += 1;
                    let first = compiled
                        .diagnostics()
                        .iter()
                        .find(|d| d.severity() == Severity::Error);
                    *tally
                        .why
                        .entry(first.map_or_else(
                            || "no error".to_owned(),
                            |d| format!("{:?}", d.language_message()),
                        ))
                        .or_default() += 1;
                } else {
                    tally.accepted += 1;
                    tally.constant += u32::from(
                        compiled
                            .expr()
                            .is_some_and(molangx::compile::Expr::is_constant),
                    );
                    let calls = |op| {
                        compiled
                            .expr()
                            .and_then(molangx::internals::tree)
                            .is_some_and(|tree| contains(tree, op))
                    };
                    tally.host_calls[0] += u32::from(calls(molangx::ops::ExpressionOp::HostMath));
                    tally.host_calls[1] +=
                        u32::from(calls(molangx::ops::ExpressionOp::HostMathVolatile));
                }
            }
            by_version
        }

        fn sum(by_version: &BTreeMap<i16, Tally>) -> Tally {
            let mut all = Tally::default();
            for tally in by_version.values() {
                all.accepted += tally.accepted;
                all.rejected += tally.rejected;
                all.constant += tally.constant;
                all.host_calls[0] += tally.host_calls[0];
                all.host_calls[1] += tally.host_calls[1];
                for (why, count) in &tally.why {
                    *all.why.entry(why.clone()).or_default() += count;
                }
            }
            all
        }

        /// About 98 % are accepted; the threshold leaves room for a changed pool.
        #[test]
        fn nearly_every_generated_program_is_accepted_by_the_compiler() {
            let by_version = tally();
            let all = sum(by_version);
            assert_eq!(all.total(), SEEDS as u32);
            assert!(
                all.accepted * 100 >= all.total() * 94,
                "{} of {} accepted; why not: {:?}",
                all.accepted,
                all.total(),
                all.why
            );
            // Every version of the range, the ones outside the defined range included.
            assert_eq!(
                by_version.keys().copied().collect::<Vec<_>>(),
                (-2..=15).collect::<Vec<_>>()
            );
            for (version, tally) in by_version {
                assert!(
                    tally.accepted * 100 >= tally.total() * 90,
                    "version {version}: {} of {} accepted; why not: {:?}",
                    tally.accepted,
                    tally.total(),
                    tally.why
                );
            }
        }

        #[test]
        fn a_small_share_of_the_generated_programs_is_rejected_on_purpose() {
            let all = sum(tally());
            // Loose programs, a parenthesis or a final `;` left out, a call with another count of
            // arguments: the rejection paths of the compiler are tested too, but they are not the
            // bulk of the cases.
            assert!(
                all.rejected * 200 >= all.total(),
                "{} of {} rejected",
                all.rejected,
                all.total()
            );
            assert!(
                all.rejected * 20 <= all.total(),
                "{} of {} rejected: {:?}",
                all.rejected,
                all.total(),
                all.why
            );
            assert!(all.why.len() >= 8, "{:?}", all.why);
        }

        fn contains(node: &molangx::internals::Node, op: molangx::ops::ExpressionOp) -> bool {
            node.op() == op || node.children().iter().any(|child| contains(child, op))
        }

        /// The host math functions of the reference options are called, pure and volatile ones,
        /// each by at least 2 % of the programs.
        #[test]
        fn some_generated_programs_call_host_math_functions() {
            let all = sum(tally());
            for (kind, calls) in ["pure", "volatile"].into_iter().zip(all.host_calls) {
                assert!(
                    calls * 50 >= all.total(),
                    "{calls} of {} accepted programs call a {kind} host function",
                    all.total()
                );
            }
        }

        /// A program that folds to a constant tests nothing about the evaluators; most must read
        /// their environment.
        #[test]
        fn few_generated_programs_fold_to_a_constant() {
            let all = sum(tally());
            assert!(
                all.constant * 10 <= all.total(),
                "{} of {} fold to a constant",
                all.constant,
                all.total()
            );
            // Some do: the folder and the evaluators meet on them.
            assert!(
                all.constant * 100 >= all.total(),
                "{} of {} fold to a constant",
                all.constant,
                all.total()
            );
        }
    }
}
