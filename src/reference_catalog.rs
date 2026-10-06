//! `stdlib::queries(Side::Client)` extended with seven test helper queries in a host set of
//! their own, and three host math functions, for the tests and the fuzz crate.

use std::sync::LazyLock;

#[cfg(feature = "compiler")]
use crate::catalog::{Arity, MathCatalog, MathDecl};
use crate::catalog::{
    QueryAdmission, QueryCatalog, QueryDecl, QuerySetMask, QueryShape, ReturnType, Side,
    VersionRange, VersionRanges,
};
#[cfg(feature = "compiler")]
use crate::compile::CompileOptions;
#[cfg(feature = "compiler")]
use crate::numeric::PostOp;
#[cfg(feature = "compiler")]
use crate::stdlib::math::max;
use crate::stdlib::queries;
#[cfg(feature = "compiler")]
use crate::version::RawVersion;
use crate::version::{Experiment, ExperimentMask, MolangVersion};

/// The host set of the helper queries.
pub const HELPER_SET: QuerySetMask = QuerySetMask::host(0).unwrap();
/// The experiment `query.experimental_test` needs; the highest id, so it collides with no real
/// one.
pub const HELPER_EXPERIMENT: Experiment = Experiment::new(63).unwrap();
/// `default` and the helper set.
pub const SETS: QuerySetMask = QuerySetMask::DEFAULT.union(HELPER_SET);
/// Admits [`SETS`].
pub const ADMISSION: QueryAdmission = QueryAdmission::Sets(SETS);
/// Enables the helper experiment.
pub const EXPERIMENTS: ExperimentMask = ExperimentMask::empty().with(HELPER_EXPERIMENT);

/// `query.get_name_test`.
pub const GET_NAME_TEST: &str = "query.get_name_test";
/// `query.sum_test`.
pub const SUM_TEST: &str = "query.sum_test";
/// `query.experimental_test`.
pub const EXPERIMENTAL_TEST: &str = "query.experimental_test";

/// Compile options at `raw_version` with this catalogue, its sets, its experiment and
/// [`math`].
#[cfg(feature = "compiler")]
pub fn options(raw_version: i16) -> CompileOptions {
    CompileOptions {
        admission: ADMISSION,
        experiments: EXPERIMENTS,
        math: Some(math().clone()),
        ..CompileOptions::from_raw_version(catalog().clone(), RawVersion(raw_version))
    }
}

/// The pure `math.helper_mix(a, b)` (half the larger, less a quarter of `b`), the pure
/// `math.helper_sum` (1 to 8 arguments, each times its position) and the volatile
/// `math.helper_noise(x)` (`x` plus one draw): neither pure one is symmetric in its
/// arguments.
///
/// # Panics
///
/// When a declaration does not build.
#[cfg(feature = "compiler")]
pub fn math() -> &'static MathCatalog {
    static MATH: LazyLock<MathCatalog> = LazyLock::new(|| {
        let weighted = |a: &[f32]| {
            a.iter()
                .zip(1u8..)
                .map(|(x, weight)| x * f32::from(weight))
                .sum()
        };
        let decls = [
            MathDecl::pure("math.helper_mix", Arity::exactly(2), |a| {
                max(a[0], a[1], PostOp::IDENTITY) * 0.5 - a[1] * 0.25
            }),
            MathDecl::pure("math.helper_sum", Arity::between(1, 8), weighted),
            MathDecl::volatile("math.helper_noise", Arity::exactly(1), |rng, a| {
                a[0] + crate::rng::sample(rng)
            }),
        ]
        .map(|decl| decl.unwrap_or_else(|error| panic!("reference catalogue: {error}")));
        MathCatalog::new(decls).unwrap_or_else(|error| panic!("reference catalogue: {error}"))
    });
    &MATH
}

/// `stdlib::queries(Side::Client)` with the seven helper queries.
///
/// # Panics
///
/// When a helper declaration does not build or does not join the catalogue.
pub fn catalog() -> &'static QueryCatalog {
    static CATALOG: LazyLock<QueryCatalog> = LazyLock::new(|| {
        let window = |first: i16, last: i16| {
            let version = |raw| {
                MolangVersion::from_i16(raw)
                    .unwrap_or_else(|| panic!("reference catalogue: no version {raw}"))
            };
            VersionRange::new(version(first), version(last), HELPER_SET).unwrap_or_else(|| {
                panic!("reference catalogue: the window {first}..={last} is empty")
            })
        };
        let shape = |returns: ReturnType, first: i16, last: i16| QueryShape {
            returns,
            ranges: VersionRanges::single(window(first, last)),
            ..QueryShape::DEFAULT
        };
        let decls = [
            (GET_NAME_TEST, shape(ReturnType::STRING, -1, 13)),
            (SUM_TEST, shape(ReturnType::FLOAT, -1, 13)),
            (
                EXPERIMENTAL_TEST,
                QueryShape {
                    experiments: EXPERIMENTS,
                    ..shape(ReturnType::FLOAT, -1, 13)
                },
            ),
            ("query.valid_always", shape(ReturnType::FLOAT, 1, 13)),
            ("query.valid_early", shape(ReturnType::FLOAT, 1, 8)),
            ("query.valid_mid", shape(ReturnType::FLOAT, 4, 8)),
            ("query.valid_late", shape(ReturnType::FLOAT, 4, 13)),
        ]
        .map(|(name, shape)| {
            QueryDecl::new(name, shape)
                .unwrap_or_else(|error| panic!("reference catalogue: {error}"))
        });
        queries(Side::Client)
            .extended(decls)
            .unwrap_or_else(|error| panic!("reference catalogue: {error}"))
    });
    &CATALOG
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::version::RawVersion;

    #[test]
    fn the_seven_helper_queries_extend_the_client_catalogue() {
        let base = queries(Side::Client);
        assert_eq!(catalog().len(), base.len() + 7);
        assert_eq!(catalog().side(), Side::Client);
        let names: Vec<&str> = catalog()
            .iter()
            .skip(base.len())
            .map(QueryDecl::name)
            .collect();
        assert_eq!(
            names,
            [
                GET_NAME_TEST,
                SUM_TEST,
                EXPERIMENTAL_TEST,
                "query.valid_always",
                "query.valid_early",
                "query.valid_mid",
                "query.valid_late"
            ]
        );
        assert!(
            catalog()
                .iter()
                .skip(base.len())
                .all(|decl| decl.sets() == HELPER_SET)
        );
        assert!(base.iter().all(|decl| !decl.sets().intersects(HELPER_SET)));
    }

    #[test]
    fn the_helper_queries_resolve_with_the_helper_set_and_from_version_minus_one() {
        let get = |name| catalog().get(name).unwrap();
        assert_eq!(get(GET_NAME_TEST).shape().returns, ReturnType::STRING);
        for name in [SUM_TEST, GET_NAME_TEST] {
            assert_eq!(
                get(name).resolve(RawVersion(-1), &ADMISSION, ExperimentMask::empty()),
                Some(0),
                "{name}"
            );
            assert_eq!(
                get(name).resolve(RawVersion(-2), &ADMISSION, ExperimentMask::empty()),
                None,
                "{name}"
            );
            assert_eq!(
                get(name).resolve(
                    RawVersion(13),
                    &QueryAdmission::Sets(QuerySetMask::DEFAULT),
                    ExperimentMask::empty()
                ),
                None,
                "{name}"
            );
        }
        assert_eq!(
            get(EXPERIMENTAL_TEST).resolve(RawVersion(-1), &ADMISSION, ExperimentMask::empty()),
            None
        );
        assert_eq!(
            get(EXPERIMENTAL_TEST).resolve(RawVersion(-1), &ADMISSION, EXPERIMENTS),
            Some(0)
        );
        for (name, first, last) in [
            ("query.valid_always", 1, 13),
            ("query.valid_early", 1, 8),
            ("query.valid_mid", 4, 8),
            ("query.valid_late", 4, 13),
        ] {
            for raw in -1..=14 {
                assert_eq!(
                    get(name)
                        .resolve(RawVersion(raw), &ADMISSION, ExperimentMask::empty())
                        .is_some(),
                    (first..=last).contains(&raw),
                    "{name} at {raw}"
                );
            }
        }
    }
}
