//! Cases decided by the helper query declarations alone: allow lists, version windows, the helper
//! catalogue and the hashes of string literals.

#![cfg(feature = "stdlib")]

mod common;

use common::declared::{AllowListCase, VersionWindowCase, decl};
use common::{
    EXPERIMENTAL_TEST, GET_NAME_TEST, HASH_OF_A, HASH_OF_MOO, HASH_OF_RABBIT, HELPER_SET,
    REFERENCE_SETS, SUM_TEST, reference_catalog, reference_experiments,
};
use molangx::catalog::{ParseQuerySetError, QueryAdmission, QuerySetMask, ReturnType, Side};
use molangx::hash::HashedStr;
use molangx::version::{ExperimentMask, MolangVersion, RawVersion};

#[test]
fn an_allow_list_flags_only_queries_outside_it() {
    let mut case = AllowListCase::new("query_allow_list-001");
    case.has_disallowed_queries(
        false,
        &[
            "1",
            "true",
            " 'what' ",
            "math.random(1,100)",
            "temp.foo = (math.random(0,100) > 50.0) ? 'hello' : 'goodbye';",
        ],
    );
    case.check(5);

    let mut case = AllowListCase::new("query_allow_list-002");
    case.has_disallowed_queries(
        false,
        &[
            "query.allowed",
            "query.also_allowed(1, 2)",
            "temp.foo = query.allowed;",
            "query.allowed( query.also_allowed, (query.allowed( 3, 2 ) ) ? query.allowed : query.allowed(3)) + math.random(1, 2)",
        ],
    );
    case.check(4);

    let mut case = AllowListCase::new("query_allow_list-003");
    case.has_disallowed_queries(
        true,
        &[
            "query.disallowed",
            "query.also_disallowed(1)",
            "query.disallowed(2) + query.also_disallowed",
            "(query.disallowed > 4) ? math.random(1,100) : 0",
            "query.disallowed( query.also_disallowed, (query.disallowed( 3, 2 ) ) ? query.disallowed : query.disallowed(3)) + math.random(1, 2)",
        ],
    );
    case.check(5);

    let mut case = AllowListCase::new("query_allow_list-004");
    case.has_disallowed_queries(
        true,
        &[
            "query.allowed + query.disallowed",
            "query.disallowed + query.allowed",
            "query.allowed( query.disallowed( math.random(0,100) ), 'foo' )",
            "query.allowed( query.also_allowed, (query.allowed( 3, 2 ) ) ? query.allowed : query.also_disallowed(3)) + math.random(1, 2)",
        ],
    );
    case.check(4);
}

/// Windows: `query.valid_always` 1–13, `query.valid_early` 1–8, `query.valid_mid` 4–8,
/// `query.valid_late` 4–13.
#[test]
fn a_query_parses_only_inside_its_version_window() {
    let mut case = VersionWindowCase::new("query_version_windows-001");
    case.all_parse(&[
        "1",
        "true",
        " 'what' ",
        "math.random(1,100)",
        "temp.foo = (math.random(0,100) > 50.0) ? 'hello' : 'goodbye';",
    ])
    .at(1);
    case.check(5);

    let mut case = VersionWindowCase::new("query_version_windows-002");
    case.parses("temp.always = query.valid_always;").at(1);
    case.check(1);

    let mut case = VersionWindowCase::new("query_version_windows-003");
    case.parses("temp.early = query.valid_early;").at(1);
    case.check(1);

    let mut case = VersionWindowCase::new("query_version_windows-004");
    case.parse_fails("temp.mid = query.valid_mid;").at(1);
    case.check(1);

    let mut case = VersionWindowCase::new("query_version_windows-005");
    case.parse_fails("temp.late = query.valid_late;").at(1);
    case.check(1);

    let mut case = VersionWindowCase::new("query_version_windows-006");
    case.parses("temp.always = query.valid_always;").at(6);
    case.check(1);

    let mut case = VersionWindowCase::new("query_version_windows-007");
    case.parses("temp.early = query.valid_early;").at(6);
    case.check(1);

    let mut case = VersionWindowCase::new("query_version_windows-008");
    case.parses("temp.mid = query.valid_mid;").at(6);
    case.check(1);

    let mut case = VersionWindowCase::new("query_version_windows-009");
    case.parses("temp.late = query.valid_late;").at(6);
    case.check(1);

    let mut case = VersionWindowCase::new("query_version_windows-010");
    case.parses("temp.always = query.valid_always;");
    case.check(1);

    let mut case = VersionWindowCase::new("query_version_windows-011");
    case.parse_fails("temp.early = query.valid_early;");
    case.check(1);

    let mut case = VersionWindowCase::new("query_version_windows-012");
    case.parse_fails("temp.mid = query.valid_mid;");
    case.check(1);

    let mut case = VersionWindowCase::new("query_version_windows-013");
    case.parses("temp.late = query.valid_late;");
    case.check(1);

    let mut case = VersionWindowCase::new("query_version_windows-014");
    case.parse_fails(
        "temp.two = query.valid_always(query.valid_early) + query.valid_mid(query.valid_late);",
    )
    .at(1);
    case.check(1);

    let mut case = VersionWindowCase::new("query_version_windows-015");
    case.parses(
        "temp.three = query.valid_always(query.valid_early) + query.valid_mid(query.valid_late);",
    )
    .at(4);
    case.check(1);

    let mut case = VersionWindowCase::new("query_version_windows-016");
    case.parses(
        "temp.four = query.valid_always(query.valid_early) + query.valid_mid(query.valid_late);",
    )
    .at(6);
    case.check(1);

    let mut case = VersionWindowCase::new("query_version_windows-017");
    case.parses(
        "temp.five = query.valid_always(query.valid_early) + query.valid_mid(query.valid_late);",
    )
    .at(8);
    case.check(1);

    let mut case = VersionWindowCase::new("query_version_windows-018");
    case.parse_fails(
        "temp.six = query.valid_always(query.valid_early) + query.valid_mid(query.valid_late);",
    );
    case.check(1);
}

/// Seven helper queries beside the client's standard ones; the helpers resolve at version −1 but
/// not below.
#[test]
fn the_helper_query_catalogue() {
    let base = molangx::stdlib::queries(Side::Client);
    assert_eq!(reference_catalog().len(), base.len() + 7);
    let sets = QueryAdmission::Sets(REFERENCE_SETS);
    for (name, returns) in [
        (GET_NAME_TEST, ReturnType::STRING),
        (SUM_TEST, ReturnType::FLOAT),
        (EXPERIMENTAL_TEST, ReturnType::FLOAT),
    ] {
        assert_eq!(
            (decl(name).sets(), decl(name).shape().returns),
            (HELPER_SET, returns),
            "{name}"
        );
        assert!(
            decl(name)
                .resolve(
                    RawVersion(13),
                    &QueryAdmission::Sets(QuerySetMask::DEFAULT),
                    reference_experiments()
                )
                .is_none(),
            "{name} needs the helper set"
        );
        assert!(!base.contains(name), "{name} is not a standard query");
    }
    assert!(
        decl(SUM_TEST)
            .resolve(RawVersion(13), &sets, ExperimentMask::empty())
            .is_some()
    );
    assert!(
        decl(EXPERIMENTAL_TEST)
            .resolve(RawVersion(13), &sets, ExperimentMask::empty())
            .is_none(),
        "experiment off"
    );
    assert!(
        decl(EXPERIMENTAL_TEST)
            .resolve(RawVersion(13), &sets, reference_experiments())
            .is_some()
    );
    assert_eq!(
        "test".parse::<QuerySetMask>(),
        Err(ParseQuerySetError),
        "the helper set is a host set"
    );

    for (name, first, last) in [
        ("query.valid_always", 1, 13),
        ("query.valid_early", 1, 8),
        ("query.valid_mid", 4, 8),
        ("query.valid_late", 4, 13),
    ] {
        assert_eq!(
            (decl(name).sets(), decl(name).shape().returns),
            (HELPER_SET, ReturnType::FLOAT),
            "{name}"
        );
        for raw in -1..=14 {
            assert_eq!(
                decl(name)
                    .resolve(RawVersion(raw), &sets, ExperimentMask::empty())
                    .is_some(),
                (first..=last).contains(&raw),
                "{name} at {raw}"
            );
        }
    }
    for name in [GET_NAME_TEST, SUM_TEST] {
        assert_eq!(
            decl(name).resolve(RawVersion(-1), &sets, ExperimentMask::empty()),
            Some(0),
            "{name}"
        );
        assert_eq!(
            decl(name).shape().ranges.as_slice()[0].first(),
            MolangVersion::Invalid,
            "{name}"
        );
        assert_eq!(
            decl(name).resolve(RawVersion(-2), &sets, ExperimentMask::empty()),
            None,
            "{name}"
        );
    }
    assert!(
        decl(EXPERIMENTAL_TEST)
            .resolve(RawVersion(-1), &sets, ExperimentMask::empty())
            .is_none(),
        "experiment off"
    );
    assert!(
        decl(EXPERIMENTAL_TEST)
            .resolve(RawVersion(-1), &sets, reference_experiments())
            .is_some()
    );
}

/// The expected string hashes are the 64-bit FNV-1 hashes of the literals.
#[test]
fn the_hash_rows_expect_the_fnv1_hash_of_their_literal() {
    for (literal, expected) in [
        ("moo", HASH_OF_MOO),
        ("rabbit", HASH_OF_RABBIT),
        ("a", HASH_OF_A),
        ("a", HASH_OF_A),
    ] {
        assert_eq!(HashedStr::new(literal).as_u64(), expected, "{literal:?}");
    }
}
