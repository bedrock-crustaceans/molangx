//! What a script can name out of the box: the standard library's `math.*` functions and queries.
//!
//! Without the `stdlib` feature this module is not built: no `math.*` name is known (a host
//! declares its own in a `MathCatalog`) and every query comes from a host `QueryCatalog`.

use std::sync::LazyLock;

use crate::catalog::{QueryCatalog, Side};
use crate::version::semver::Version;

#[cfg(feature = "compiler")]
#[cfg_attr(docsrs, doc(cfg(feature = "compiler")))]
pub mod math;

mod math_fn;
mod query_table;

pub use math_fn::{MATH_META, MathFn, MathMeta};
pub use query_table::query;

/// The standard library's queries for `side`, built once and shared.
pub fn queries(side: Side) -> &'static QueryCatalog {
    static CLIENT: LazyLock<QueryCatalog> =
        LazyLock::new(|| query_table::catalog(Side::Client, None));
    static SERVER: LazyLock<QueryCatalog> =
        LazyLock::new(|| query_table::catalog(Side::Server, None));
    match side {
        Side::Client => &CLIENT,
        Side::Server => &SERVER,
    }
}

/// The standard library's queries for `side` without the ones first present after `release`.
///
/// Releases compare by `SemVer` precedence, build metadata ignored: a query first present in 1.26.30
/// is missing from `1.26.30-beta`, a pre-release of 1.26.30, which comes before it.
///
/// Each call builds a **new** catalogue, unequal to every other: expressions compiled against
/// two of them share no compile-cache entry, and a `vm::QueryTable` built for one finds the
/// other's queries by name instead of by position. Build it once and keep it.
pub fn queries_at(side: Side, release: &Version) -> QueryCatalog {
    query_table::catalog(side, Some(release))
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::*;

    #[test]
    fn the_re_exports_are_the_table_items() {
        assert!(std::ptr::eq(queries(Side::Server), queries(Side::Server)));
        assert_eq!(queries(Side::Client).side(), Side::Client);
        assert_eq!(
            queries(Side::Server)
                .get(query::IS_BABY)
                .map(crate::catalog::QueryDecl::name),
            Some("query.is_baby")
        );
        let _: &'static [MathMeta; MathFn::COUNT] = &MATH_META;
        assert_eq!(MathFn::Pi.op().token(), Some(MathFn::Pi.token()));
    }

    #[test]
    fn a_pre_release_comes_before_its_release_and_build_metadata_does_not_count() {
        let has_fuse_time = |text: &str| {
            queries_at(Side::Client, &Version::parse(text).unwrap())
                .get(query::FUSE_TIME)
                .is_some()
        };
        assert!(!has_fuse_time("1.26.30-beta"));
        assert!(has_fuse_time("1.26.30"));
        assert!(has_fuse_time("1.26.30+build"));
        assert!(has_fuse_time("1.26.31-beta"));
    }

    /// The release bound is inclusive.
    #[test]
    fn a_release_bound_drops_the_later_queries_only() {
        let names = |side, release: &Version| {
            queries_at(side, release)
                .iter()
                .map(|d| d.name().to_owned())
                .collect::<BTreeSet<_>>()
        };
        let all: BTreeSet<String> = queries(Side::Client)
            .iter()
            .map(|d| d.name().to_owned())
            .collect();
        let missing = |side, release: &Version| {
            all.difference(&names(side, release))
                .cloned()
                .collect::<Vec<_>>()
        };
        assert_eq!(
            missing(Side::Server, &Version::new(1, 26, 36)),
            [
                query::HAS_ALL_BIOME_TAGS,
                query::HAS_ANY_BIOME_TAGS,
                query::HEAD_IS_IN_WATER
            ]
        );
        assert_eq!(
            missing(Side::Client, &Version::new(1, 26, 0)),
            [
                query::FUSE_TIME,
                query::HAS_ALL_BIOME_TAGS,
                query::HAS_ANY_BIOME_TAGS,
                query::HEAD_IS_IN_WATER
            ]
        );
        assert!(names(Side::Client, &Version::new(1, 26, 30)).contains(query::FUSE_TIME));
        assert!(!names(Side::Client, &Version::new(1, 26, 29)).contains(query::FUSE_TIME));
        assert!(missing(Side::Client, &Version::new(1, 26, 50)).is_empty());
        assert_eq!(
            queries_at(Side::Server, &Version::new(1, 26, 36)).side(),
            Side::Server
        );
        assert_ne!(
            queries_at(Side::Client, &Version::new(1, 26, 50)),
            *queries(Side::Client)
        );
        // Each call builds a catalogue unequal to every other: build it once and keep it.
        let release = &Version::new(1, 26, 50);
        let kept = queries_at(Side::Client, release);
        assert_ne!(queries_at(Side::Client, release), kept);
        assert_eq!(kept.clone(), kept);
    }
}
