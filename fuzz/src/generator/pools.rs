//! The fixed pools the generator draws names, numbers, strings, functions and operators from, with
//! its size limits.

use molangx::catalog::MAX_MATH_ARGS;
use molangx::internals::reference_catalog;
use std::ops::RangeInclusive;
use std::sync::LazyLock;

pub(super) const MAX_DEPTH: u32 = 5;

pub(super) const MAX_STATEMENTS: usize = 3;

/// The position in [`MATH`] of the first host math function.
pub(super) const HOST_MATH: usize = 61;

/// The argument counts a generated call of `MATH[f]` has: a host function's declared ones (at most
/// eight), a standard function's arity.
pub(super) fn math_arguments(f: usize) -> RangeInclusive<u8> {
    static COUNTS: LazyLock<Vec<RangeInclusive<u8>>> = LazyLock::new(|| {
        let host = reference_catalog::math();
        let standard = MATH[..HOST_MATH].iter().map(|&(_, arity)| arity..=arity);
        let declared = MATH[HOST_MATH..].iter().map(|&(name, usual)| {
            match host.get(&format!("math.{name}")) {
                Some(decl) => {
                    let args = decl.args();
                    args.min()..=args.max().unwrap_or(MAX_MATH_ARGS).min(MAX_MATH_ARGS)
                }
                None => usual..=usual,
            }
        });
        standard.chain(declared).collect()
    });
    COUNTS[f].clone()
}

/// Number literals: exact small values, values at the guards' edges (`f32::EPSILON`, 2^-23, and
/// 2^24), loop counts around the default budget, and large magnitudes.
pub(super) const NUMBERS: &[f64] = &[
    0.0,
    1.0,
    2.0,
    3.0,
    4.0,
    7.0,
    10.0,
    0.5,
    0.25,
    1.5,
    2.5,
    0.1,
    0.3,
    100.0,
    180.0,
    255.0,
    360.0,
    1024.0,
    1025.0,
    1e-7,
    1.2e-7,
    16_777_216.0,
    16_777_217.0,
    1e10,
    3e38,
];

/// String literals (one with upper case: lowering stops at quotes).
pub(super) const STRINGS: &[&str] = &["moo", "rabbit", "", "Moo X", "a.b"];

/// Entity variable names: floats, a NaN, a string, a struct, actors and arrays in the fuzz
/// environment, plus names that are never set.
pub(super) const VARIABLES: &[&str] =
    &["x", "y", "n", "s", "st", "e", "arr", "a", "b", "i", "never"];

pub(super) const TEMPS: &[&str] = &["a", "b", "i", "e"];

/// Context names (`other` is an actor, `arr` an actor array, `missing` is never set).
pub(super) const CONTEXTS: &[&str] = &["other", "n", "arr", "missing"];

pub(super) const MEMBERS: &[&str] = &["x", "y", "z", "q"];

/// The math functions with their arity, then from [`HOST_MATH`] the reference options' host math
/// functions with a count their declared arity admits.
pub(super) const MATH: &[(&str, u8)] = &[
    ("abs", 1),
    ("acos", 1),
    ("asin", 1),
    ("atan", 1),
    ("atan2", 2),
    ("ceil", 1),
    ("clamp", 3),
    ("cos", 1),
    ("copy_sign", 2),
    ("die_roll", 3),
    ("die_roll_integer", 3),
    ("exp", 1),
    ("floor", 1),
    ("hermite_blend", 1),
    ("inverse_lerp", 3),
    ("lerp", 3),
    ("lerprotate", 3),
    ("ln", 1),
    ("max", 2),
    ("min", 2),
    ("min_angle", 1),
    ("mod", 2),
    ("pi", 0),
    ("pow", 2),
    ("random", 2),
    ("random_integer", 2),
    ("round", 1),
    ("sign", 1),
    ("sin", 1),
    ("sqrt", 1),
    ("trunc", 1),
    ("ease_in_quad", 3),
    ("ease_out_quad", 3),
    ("ease_in_out_quad", 3),
    ("ease_in_cubic", 3),
    ("ease_out_cubic", 3),
    ("ease_in_out_cubic", 3),
    ("ease_in_quart", 3),
    ("ease_out_quart", 3),
    ("ease_in_out_quart", 3),
    ("ease_in_quint", 3),
    ("ease_out_quint", 3),
    ("ease_in_out_quint", 3),
    ("ease_in_sine", 3),
    ("ease_out_sine", 3),
    ("ease_in_out_sine", 3),
    ("ease_in_expo", 3),
    ("ease_out_expo", 3),
    ("ease_in_out_expo", 3),
    ("ease_in_circ", 3),
    ("ease_out_circ", 3),
    ("ease_in_out_circ", 3),
    ("ease_in_bounce", 3),
    ("ease_out_bounce", 3),
    ("ease_in_out_bounce", 3),
    ("ease_in_back", 3),
    ("ease_out_back", 3),
    ("ease_in_out_back", 3),
    ("ease_in_elastic", 3),
    ("ease_out_elastic", 3),
    ("ease_in_out_elastic", 3),
    ("helper_mix", 2),
    ("helper_sum", 2),
    ("helper_noise", 1),
];

/// Queries with their usual argument counts: the ones the fuzz environment implements, the
/// helper queries of the reference catalogue, and stubs.
pub(super) const QUERIES: &[(&str, u8)] = &[
    ("log", 2),
    ("count", 1),
    ("any", 3),
    ("all", 3),
    ("in_range", 3),
    ("is_baby", 0),
    ("sum_test", 2),
    ("get_name_test", 1),
    ("experimental_test", 0),
    ("time_of_day", 0),
    ("has_any_family", 1),
];

/// The queries (of [`QUERIES`]) that return a string: not an operand of arithmetic.
pub(super) const STRING_QUERIES: &[&str] = &["get_name_test"];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::generator::READERS;

    #[test]
    fn the_pools_have_their_pinned_sizes() {
        assert_eq!((MAX_DEPTH, MAX_STATEMENTS), (5, 3));
        assert_eq!(NUMBERS.len(), 25);
        assert_eq!(STRINGS.len(), 5);
        assert_eq!(VARIABLES.len(), 11);
        assert_eq!(TEMPS.len(), 4);
        assert_eq!(CONTEXTS.len(), 4);
        assert_eq!(MEMBERS.len(), 4);
        assert_eq!(MATH.len(), 64);
        assert_eq!(QUERIES.len(), 11);
        assert_eq!(READERS.len(), 5);
    }

    #[test]
    fn every_math_function_has_a_usable_arity_and_a_unique_name() {
        let mut names: Vec<&str> = MATH.iter().map(|(name, _)| *name).collect();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), MATH.len());
        assert!(MATH.iter().all(|(_, arity)| *arity <= 3));
        assert_eq!(
            MATH.iter().filter(|(_, arity)| *arity == 0).count(),
            1,
            "only math.pi takes none"
        );
        assert_eq!(
            MATH.iter()
                .filter(|(name, _)| name.starts_with("ease_"))
                .count(),
            30
        );
        assert!(
            MATH.iter()
                .filter(|(name, _)| name.starts_with("ease_"))
                .all(|(_, arity)| *arity == 3)
        );
        // The host functions follow the standard ones, each declared in the reference options.
        let host = molangx::internals::reference_catalog::math();
        for (index, (name, arity)) in MATH.iter().enumerate() {
            let full = format!("math.{name}");
            assert_eq!(
                molangx::stdlib::MathFn::from_token(&full).is_some(),
                index < HOST_MATH,
                "{full}"
            );
            if let Some(decl) = host.get(&full) {
                assert!(decl.args().contains(usize::from(*arity)), "{full}");
            }
        }
        assert_eq!(
            MATH[HOST_MATH..]
                .iter()
                .filter(|(name, _)| host.contains(&format!("math.{name}")))
                .count(),
            3
        );
        let sum = MATH
            .iter()
            .position(|(name, _)| *name == "helper_sum")
            .expect("in the pool");
        assert_eq!(math_arguments(sum), 1..=8);
        assert_eq!(math_arguments(HOST_MATH), 2..=2);
        assert_eq!(math_arguments(0), MATH[0].1..=MATH[0].1);
    }

    #[test]
    fn the_pools_hold_no_quote_no_upper_case_name_and_no_blank() {
        for name in VARIABLES.iter().chain(TEMPS).chain(CONTEXTS).chain(MEMBERS) {
            assert!(
                !name.is_empty() && name.chars().all(|c| c.is_ascii_lowercase() || c == '_'),
                "{name:?}"
            );
        }
        for text in STRINGS {
            assert!(!text.contains('\''), "{text:?}");
        }
        assert!(
            STRINGS
                .iter()
                .any(|text| text.chars().any(|c| c.is_ascii_uppercase())),
            "one string keeps upper case"
        );
        let mut unique: Vec<u64> = NUMBERS.iter().map(|x| x.to_bits()).collect();
        unique.sort_unstable();
        unique.dedup();
        assert_eq!(unique.len(), NUMBERS.len());
    }
}
