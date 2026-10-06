//! Query admission, the allow-list and the resolution rule.

use std::fmt;
use std::sync::Arc;

use thiserror::Error;

use super::{QueryCatalog, QueryDecl, QuerySetMask};
use crate::version::{ExperimentMask, RawVersion};

/// Which queries of the catalogue an expression may call: by query set, or by an allow-list
/// that ignores the sets.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum QueryAdmission {
    /// Every query in one of these sets; an empty mask admits none.
    Sets(QuerySetMask),
    /// Only the queries on this list, whatever their sets.
    Only(QueryAllowList),
}

impl Default for QueryAdmission {
    fn default() -> Self {
        Self::Sets(QuerySetMask::DEFAULT)
    }
}

impl QueryAdmission {
    /// Whether `decl` is admitted. A list admits a declaration of any catalogue by name.
    pub fn admits(&self, decl: &QueryDecl) -> bool {
        match self {
            Self::Sets(sets) => sets.intersects(decl.sets()),
            Self::Only(list) => list.contains(decl.name()),
        }
    }
}

/// Why a [`QueryAllowList`] was not built.
#[derive(Error, Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum AllowListError {
    /// No name was given. To admit no query, use [`QueryAdmission::Sets`] of
    /// [`QuerySetMask::empty()`].
    #[error("an allow-list names at least one query")]
    Empty,
    /// The catalogue does not declare this name. Only the full spelling is declared
    /// (`"query.block_state"`, not `"q.block_state"`).
    #[error("`{0}` is not declared in the catalogue")]
    Undeclared(Box<str>),
}

/// The only queries a context accepts: a non-empty set of query names, each declared by the
/// catalogue the list was checked against.
///
/// Order and repetition of the names do not matter. Cheap to clone (a shared handle); compared
/// and hashed by its names, so lists of the same names share compile-cache entries.
///
/// Installed as [`QueryAdmission::Only`] in `CompileOptions::admission`, a list admits the queries
/// of the compile's catalogue whose names it holds.
///
/// ```
/// # #[cfg(feature = "stdlib")]
/// # {
/// use molangx::catalog::{AllowListError, QueryAllowList, Side};
/// use molangx::stdlib::{self, query};
///
/// let catalog = stdlib::queries(Side::Server);
/// let list = QueryAllowList::new(
///     catalog,
///     [
///         query::BLOCK_STATE,
///         query::HAS_BLOCK_STATE,
///         query::BLOCK_STATE,
///     ],
/// )?;
/// assert!(list.contains(query::BLOCK_STATE) && !list.contains(query::IS_BABY));
/// assert_eq!(list.iter().collect::<Vec<_>>(), [query::BLOCK_STATE, query::HAS_BLOCK_STATE]);
/// assert_eq!(
///     QueryAllowList::new(catalog, ["q.block_state"]),
///     Err(AllowListError::Undeclared("q.block_state".into()))
/// );
/// assert_eq!(QueryAllowList::new(catalog, [""; 0]), Err(AllowListError::Empty));
/// # }
/// # Ok::<(), molangx::catalog::AllowListError>(())
/// ```
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct QueryAllowList {
    /// Distinct and sorted.
    names: Arc<[Box<str>]>,
}

impl QueryAllowList {
    /// The list of the queries named `names` (full names), each declared by `catalog`.
    pub fn new<I>(catalog: &QueryCatalog, names: I) -> Result<Self, AllowListError>
    where
        I: IntoIterator,
        I::Item: AsRef<str>,
    {
        let mut entries = names
            .into_iter()
            .map(|name| {
                let name = name.as_ref();
                catalog
                    .index_of(name)
                    .map(|_| Box::from(name))
                    .ok_or_else(|| AllowListError::Undeclared(name.into()))
            })
            .collect::<Result<Vec<Box<str>>, _>>()?;
        if entries.is_empty() {
            return Err(AllowListError::Empty);
        }
        entries.sort_unstable();
        entries.dedup();
        Ok(Self {
            names: entries.into(),
        })
    }

    /// Whether the full name `name` is on the list.
    pub fn contains(&self, name: &str) -> bool {
        self.names
            .binary_search_by(|entry| (**entry).cmp(name))
            .is_ok()
    }

    /// The names on the list, sorted.
    pub fn iter(&self) -> impl ExactSizeIterator<Item = &str> + '_ {
        self.names.iter().map(|name| &**name)
    }
}

impl fmt::Debug for QueryAllowList {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_list().entries(self.iter()).finish()
    }
}

impl QueryDecl {
    /// The implementation that serves a call at `raw_version`, or `None` when the call does not
    /// resolve.
    ///
    /// The call resolves when `admission` admits the query and every experiment it needs is
    /// enabled; then the range containing `raw_version` serves it. A raw version outside
    /// `-1..=13` is in no range.
    pub fn resolve(
        &self,
        raw_version: RawVersion,
        admission: &QueryAdmission,
        experiments: ExperimentMask,
    ) -> Option<u8> {
        if !admission.admits(self) || !experiments.contains(self.shape().experiments) {
            return None;
        }
        self.implementation_at_raw(raw_version)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::{
        QueryShape, VersionRanges,
        query::{QueryCatalog, Side, VersionRange, test_support::*},
    };
    use crate::stdlib::query;
    use crate::version::{Experiment, MolangVersion};
    use proptest::prelude::*;

    fn stdlib_decl(name: &str) -> &'static QueryDecl {
        crate::stdlib::queries(Side::Client)
            .get(name)
            .unwrap_or_else(|| panic!("{name} is not a standard query"))
    }

    fn at(name: &str, raw: i16, admission: &QueryAdmission) -> Option<u8> {
        stdlib_decl(name).resolve(RawVersion(raw), admission, ExperimentMask::empty())
    }

    const DEFAULT: QueryAdmission = QueryAdmission::Sets(QuerySetMask::DEFAULT);

    fn mask(bits: u8) -> QuerySetMask {
        [
            QuerySetMask::DEFAULT,
            QuerySetMask::TAGS,
            QuerySetMask::WORLD_GEN,
        ]
        .into_iter()
        .enumerate()
        .filter(|(bit, _)| bits & (1 << bit) != 0)
        .fold(QuerySetMask::empty(), |mask, (_, set)| mask | set)
    }

    #[test]
    fn the_default_admission_is_the_default_set() {
        assert_eq!(QueryAdmission::default(), DEFAULT);
    }

    #[test]
    fn sets_gate_resolution() {
        let sets = |mask| QueryAdmission::Sets(mask);
        assert!(at(query::IS_BABY, 13, &DEFAULT).is_some());
        assert!(at(query::ANY_TAG, 13, &DEFAULT).is_none());
        assert!(at(query::NOISE, 13, &DEFAULT).is_none());
        assert!(at(query::ANY_TAG, 13, &sets(QuerySetMask::TAGS)).is_some());
        assert!(at(query::ALL_TAGS, 13, &sets(QuerySetMask::TAGS)).is_some());
        assert!(at(query::IS_BABY, 13, &sets(QuerySetMask::TAGS)).is_none());
        for name in [
            query::NOISE,
            query::HEIGHTMAP,
            query::ABOVE_TOP_SOLID,
            query::HAS_BIOME_TAG,
        ] {
            assert!(
                at(name, 13, &sets(QuerySetMask::WORLD_GEN)).is_some(),
                "{name}"
            );
            assert!(
                at(name, 13, &sets(QuerySetMask::DEFAULT | QuerySetMask::TAGS)).is_none(),
                "{name}"
            );
        }
        assert!(
            at(
                query::NOISE,
                13,
                &sets(QuerySetMask::DEFAULT | QuerySetMask::WORLD_GEN)
            )
            .is_some()
        );
        for decl in crate::stdlib::queries(Side::Client) {
            assert_eq!(
                decl.resolve(
                    RawVersion(13),
                    &sets(QuerySetMask::empty()),
                    ExperimentMask::all()
                ),
                None,
                "{}",
                decl.name()
            );
        }
    }

    fn list(names: &[&str]) -> QueryAllowList {
        QueryAllowList::new(crate::stdlib::queries(Side::Client), names).unwrap()
    }

    #[test]
    fn a_list_replaces_the_sets() {
        let blocks = list(&[query::BLOCK_STATE]);
        let blocks = QueryAdmission::Only(blocks);
        assert!(at(query::BLOCK_STATE, 13, &blocks).is_some());
        assert!(
            at(query::IS_BABY, 13, &blocks).is_none(),
            "the list replaces the default set"
        );
        assert!(at(query::HAS_BLOCK_STATE, 13, &blocks).is_none());
        let old = list(&[query::BLOCK_PROPERTY]);
        let old = QueryAdmission::Only(old);
        assert!(at(query::BLOCK_PROPERTY, MolangVersion::V9.as_i16(), &old).is_some());
        assert!(at(query::BLOCK_PROPERTY, MolangVersion::V10.as_i16(), &old).is_none());
        assert!(
            at(
                query::ANY_TAG,
                13,
                &QueryAdmission::Only(list(&[query::ANY_TAG]))
            )
            .is_some()
        );
        let long = list(&[query::BLOCK_STATE, query::HAS_ANY_FAMILY, query::IS_BABY]);
        assert!(at(query::IS_BABY, 13, &QueryAdmission::Only(long.clone())).is_some());
        assert!(at(query::IS_ON_FIRE, 13, &QueryAdmission::Only(long)).is_none());
    }

    #[test]
    fn a_list_admits_by_name_in_any_catalogue() {
        let server = crate::stdlib::queries(Side::Server);
        let blocks = QueryAllowList::new(server, [query::BLOCK_STATE]).unwrap();
        let blocks = QueryAdmission::Only(blocks);
        assert!(
            at(query::BLOCK_STATE, 13, &blocks).is_some(),
            "a client declaration of a listed name"
        );
        assert!(at(query::IS_BABY, 13, &blocks).is_none());
        let own = QueryCatalog::new(
            Side::Client,
            [QueryDecl::new("query.mine", QueryShape::DEFAULT).unwrap()],
        )
        .unwrap();
        let mine = QueryAllowList::new(&own, ["query.mine"]).unwrap();
        for decl in crate::stdlib::queries(Side::Client) {
            assert!(
                !QueryAdmission::Only(mine.clone()).admits(decl),
                "{}",
                decl.name()
            );
        }
    }

    #[test]
    fn a_list_holds_only_declared_names_and_at_least_one() {
        let catalog = crate::stdlib::queries(Side::Client);
        for bad in [
            "q.is_baby",
            "QUERY.IS_BABY",
            "query.is_bab",
            "query.no_such",
            "is_baby",
            "",
        ] {
            assert_eq!(
                QueryAllowList::new(catalog, [query::IS_BABY, bad]),
                Err(AllowListError::Undeclared(bad.into())),
                "{bad:?}"
            );
        }
        assert_eq!(
            QueryAllowList::new(catalog, Vec::<String>::new()),
            Err(AllowListError::Empty)
        );
        assert_eq!(
            AllowListError::Empty.to_string(),
            "an allow-list names at least one query"
        );
        assert_eq!(
            AllowListError::Undeclared("q.x".into()).to_string(),
            "`q.x` is not declared in the catalogue"
        );
        let owned = QueryAllowList::new(catalog, vec![String::from(query::IS_BABY)]).unwrap();
        assert!(owned.contains(query::IS_BABY));
    }

    #[test]
    fn a_list_ignores_order_and_repetition() {
        let catalog = crate::stdlib::queries(Side::Client);
        let ab = list(&[query::IS_BABY, query::BLOCK_STATE]);
        let ba = list(&[query::BLOCK_STATE, query::IS_BABY, query::BLOCK_STATE]);
        assert_eq!(ab, ba);
        assert_eq!(
            ab.iter().collect::<Vec<_>>(),
            [query::BLOCK_STATE, query::IS_BABY]
        );
        assert_eq!(
            format!("{ab:?}"),
            "[\"query.block_state\", \"query.is_baby\"]"
        );
        assert_ne!(ab, list(&[query::IS_BABY]));
        let hash = |l: &QueryAllowList| {
            use std::hash::{DefaultHasher, Hash, Hasher};
            let mut h = DefaultHasher::new();
            l.hash(&mut h);
            h.finish()
        };
        assert_eq!(hash(&ab), hash(&ba));
        let other = catalog.extended([]).unwrap();
        let theirs = QueryAllowList::new(&other, [query::IS_BABY, query::BLOCK_STATE]).unwrap();
        assert_eq!(
            theirs, ab,
            "the names decide, not the catalogue checked against"
        );
        assert_eq!(hash(&theirs), hash(&ab));
        assert_eq!(ab.clone(), ab);
    }

    #[test]
    fn the_window_is_on_the_raw_signed_version() {
        let cape = |raw| at(query::CAPE_FLAP_AMOUNT, raw, &DEFAULT);
        assert_eq!(
            [cape(-1), cape(0), cape(7), cape(8), cape(13), cape(14)],
            [None, Some(0), Some(0), Some(1), Some(1), None]
        );
        assert_eq!((cape(i16::MIN), cape(i16::MAX)), (None, None));
        for decl in crate::stdlib::queries(Side::Client) {
            for raw in [-1, -2, 14, 1000, i16::MIN, i16::MAX] {
                assert_eq!(
                    decl.resolve(
                        RawVersion(raw),
                        &QueryAdmission::Sets(QuerySetMask::BUILTIN),
                        ExperimentMask::all()
                    ),
                    None,
                    "{} at {raw}",
                    decl.name()
                );
            }
        }
    }

    #[test]
    fn a_query_behind_an_experiment_needs_it() {
        let experiment = Experiment::new(9).unwrap();
        let shape = QueryShape {
            experiments: ExperimentMask::empty().with(experiment),
            ..QueryShape::DEFAULT
        };
        let decl = QueryDecl::new("query.x", shape).unwrap();
        assert_eq!(
            decl.resolve(RawVersion(13), &DEFAULT, ExperimentMask::empty()),
            None
        );
        assert_eq!(
            decl.resolve(
                RawVersion(13),
                &DEFAULT,
                ExperimentMask::empty().with(Experiment::new(5).unwrap())
            ),
            None
        );
        assert_eq!(
            decl.resolve(
                RawVersion(13),
                &DEFAULT,
                ExperimentMask::empty().with(experiment)
            ),
            Some(0)
        );
        let catalog = QueryCatalog::new(Side::Client, [decl.clone()]).unwrap();
        let listed = QueryAllowList::new(&catalog, ["query.x"]).unwrap();
        assert_eq!(
            decl.resolve(
                RawVersion(13),
                &QueryAdmission::Only(listed),
                ExperimentMask::empty()
            ),
            None
        );
    }

    #[test]
    fn the_first_range_that_contains_the_version_serves_it() {
        let v = |raw| MolangVersion::from_i16(raw).unwrap();
        let ranges = VersionRanges::new([
            VersionRange::new(v(-1), v(3), QuerySetMask::TAGS).unwrap(),
            VersionRange::new(v(5), v(13), QuerySetMask::DEFAULT).unwrap(),
        ])
        .unwrap();
        let decl = QueryDecl::new(
            "query.x",
            QueryShape {
                ranges,
                ..QueryShape::DEFAULT
            },
        )
        .unwrap();
        // The union of the sets admits it; the version picks the range, whatever its own set.
        let tags = QueryAdmission::Sets(QuerySetMask::TAGS);
        assert_eq!(
            [-2, -1, 3, 4, 5, 13].map(|raw| decl.resolve(
                RawVersion(raw),
                &tags,
                ExperimentMask::empty()
            )),
            [None, Some(0), Some(0), None, Some(1), Some(1)]
        );
    }

    proptest! {
        #![proptest_config(config())]

        #[test]
        fn resolution_is_admission_then_the_window(index in 0usize..319, raw in prop_oneof![3 => -3i16..=16, 1 => any::<i16>()], bits in 0u8..8, listed in any::<bool>()) {
            let decl = crate::stdlib::queries(Side::Client).iter().nth(index).unwrap();
            let sets = QueryAdmission::Sets(mask(bits));
            let window = decl.implementation_at_raw(RawVersion(raw));
            prop_assert_eq!(decl.resolve(RawVersion(raw), &sets, ExperimentMask::empty()), window.filter(|_| mask(bits).intersects(decl.sets())));
            let names = list(&[if listed { decl.name() } else { query::IS_BABY }]);
            let on_list = listed || decl.name() == query::IS_BABY;
            prop_assert_eq!(decl.resolve(RawVersion(raw), &QueryAdmission::Only(names), ExperimentMask::empty()), window.filter(|_| on_list));
        }
    }
}
