//! [`CompileCache`]: one compile per distinct `(options, source text)` during a pack load.
//!
//! A hit returns exactly what a fresh compile would. The key is the text plus every
//! [`CompileOptions`] field that can change the result, compared in full, so a hash collision
//! never returns a wrong program. The query and math catalogues are keyed by identity; an
//! allow-list by its names, whatever their order.
//!
//! Diagnostics are cached with the program: a loader that reports per JSON path must attribute a
//! hit's diagnostics to every path that used the text.
//!
//! No map lock is held while compiling. Threads racing on one key may each compile, but all
//! receive the one stored `Arc`; the discarded compiles count as misses.

use std::fmt;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use dashmap::DashMap;

use crate::compile::{CompileOptions, Compiled, compile_at};
use crate::json::MolangSource;
use crate::version::{MolangVersion, RawVersion};

/// Every result-changing input of a compile but the text.
#[derive(PartialEq, Eq, Hash)]
struct OptionsKey {
    /// The options with their raw version replaced by the one the compile uses (`Invalid` without
    /// one).
    opts: CompileOptions,
    /// `None` for a plain-string source whose context version was never applied: it compiles with
    /// the rules of `Invalid` but resolves no query.
    raw_version: Option<RawVersion>,
}

impl OptionsKey {
    fn for_options(opts: &CompileOptions) -> Self {
        Self::new(opts, Some(opts.raw_version))
    }

    /// The key of `field_opts` at the source version `version`, as `compile_at` applies it.
    fn new(field_opts: &CompileOptions, version: Option<RawVersion>) -> Self {
        let opts = CompileOptions {
            raw_version: version.unwrap_or(MolangVersion::Invalid.into()),
            ..field_opts.clone()
        };
        Self {
            opts,
            raw_version: version,
        }
    }
}

#[derive(PartialEq, Eq, Hash)]
struct CacheKey {
    opts: OptionsKey,
    src: Arc<str>,
}

/// A concurrent compile cache for one pack load; see the [module documentation](self).
///
/// ```
/// use molangx::cache::CompileCache;
/// use molangx::compile::CompileOptions;
/// use molangx::json::MolangSource;
/// use molangx::version::MolangVersion;
///
/// let cache = CompileCache::new();
/// let src = MolangSource::string("1 ? 0 : 1 ? 2 : 3", 4);
/// let field = CompileOptions::server(MolangVersion::LATEST);
/// let first = cache.compile_source(&src, &field);
/// let again = cache.compile_source(&src, &field);
/// // Compiled at the source's version 4, not at the options' 13.
/// assert_eq!(first.expr().map(|e| e.version()), Some(MolangVersion::V4));
/// assert!(std::sync::Arc::ptr_eq(&first, &again));
/// assert_eq!((cache.len(), cache.hits(), cache.misses()), (1, 1, 1));
/// ```
#[derive(Default)]
pub struct CompileCache {
    map: DashMap<CacheKey, Arc<Compiled>>,
    hits: AtomicU64,
    misses: AtomicU64,
}

impl CompileCache {
    /// An empty cache.
    pub fn new() -> Self {
        Self::default()
    }

    /// The cached [`compile_source`](crate::compile::compile_source): `src` at its own version
    /// under `field_opts`.
    #[must_use]
    pub fn compile_source(&self, src: &MolangSource, field_opts: &CompileOptions) -> Arc<Compiled> {
        let key = CacheKey {
            opts: OptionsKey::new(field_opts, src.raw_version()),
            src: Arc::clone(src.shared_text()),
        };
        self.lookup(key)
    }

    /// `src`'s text compiled under `opts`, ignoring the source's version: sources that differ only
    /// in version share an entry.
    #[must_use]
    pub fn compile_source_ignoring_version(
        &self,
        src: &MolangSource,
        opts: &CompileOptions,
    ) -> Arc<Compiled> {
        let key = CacheKey {
            opts: OptionsKey::for_options(opts),
            src: Arc::clone(src.shared_text()),
        };
        self.lookup(key)
    }

    /// The cached [`compile`](crate::compile::compile()).
    ///
    /// The text is copied into the key on every call; a [`MolangSource`] shares its text instead.
    #[must_use]
    pub fn compile(&self, src: &str, opts: &CompileOptions) -> Arc<Compiled> {
        let key = CacheKey {
            opts: OptionsKey::for_options(opts),
            src: Arc::from(src),
        };
        self.lookup(key)
    }

    fn lookup(&self, key: CacheKey) -> Arc<Compiled> {
        // The read guard drops at the end of this statement, before any compile or insert.
        let cached = self.map.get(&key).map(|entry| Arc::clone(entry.value()));
        if let Some(compiled) = cached {
            self.hits.fetch_add(1, Ordering::Relaxed);
            return compiled;
        }
        self.misses.fetch_add(1, Ordering::Relaxed);
        // An entry a racing thread inserted first wins.
        let fresh = Arc::new(compile_at(&key.src, &key.opts.opts, key.opts.raw_version));
        Arc::clone(self.map.entry(key).or_insert(fresh).value())
    }

    /// The number of distinct `(options, text)` entries.
    pub fn len(&self) -> usize {
        self.map.len()
    }

    /// Whether the cache holds no entry.
    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }

    /// Drops every entry; the `Arc`s handed out stay valid and the counters are kept.
    pub fn clear(&self) {
        self.map.clear();
    }

    /// The number of requests answered from the cache without compiling.
    pub fn hits(&self) -> u64 {
        self.hits.load(Ordering::Relaxed)
    }

    /// The number of requests that compiled, including compiles that lost a race and were
    /// discarded.
    pub fn misses(&self) -> u64 {
        self.misses.load(Ordering::Relaxed)
    }
}

impl fmt::Debug for CompileCache {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CompileCache")
            .field("len", &self.len())
            .field("hits", &self.hits())
            .field("misses", &self.misses())
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::{QueryAdmission, QueryAllowList, QuerySetMask, Side};
    use crate::compile::{CompileFailure, Deviations, Expr, compile};
    use crate::ops::{ExpressionOp, OpSet};
    use crate::stdlib::queries;
    use crate::stdlib::query;
    use crate::version::{Experiment, ExperimentMask};
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};

    fn at(raw: i16) -> CompileOptions {
        CompileOptions {
            raw_version: RawVersion(raw),
            ..base()
        }
    }

    fn sets(sets: QuerySetMask) -> CompileOptions {
        CompileOptions {
            admission: QueryAdmission::Sets(sets),
            ..base()
        }
    }

    fn only(list: &QueryAllowList) -> CompileOptions {
        CompileOptions {
            admission: QueryAdmission::Only(list.clone()),
            ..base()
        }
    }

    fn ops(allowed_ops: OpSet) -> CompileOptions {
        CompileOptions {
            allowed_ops,
            ..base()
        }
    }

    fn base() -> CompileOptions {
        CompileOptions::server(MolangVersion::LATEST)
    }

    fn client() -> CompileOptions {
        CompileOptions::client(MolangVersion::LATEST)
    }

    fn key_of(opts: &CompileOptions) -> OptionsKey {
        OptionsKey::for_options(opts)
    }

    fn hash_of<T: Hash>(value: &T) -> u64 {
        let mut hasher = DefaultHasher::new();
        value.hash(&mut hasher);
        hasher.finish()
    }

    fn assert_keys_differ(a: &CompileOptions, b: &CompileOptions) {
        assert!(key_of(a) != key_of(b), "{a:?} and {b:?} have the same key");
        let key = |opts: &CompileOptions| CacheKey {
            opts: OptionsKey::for_options(opts),
            src: Arc::from("v.x"),
        };
        assert!(key(a) != key(b));
    }

    #[test]
    fn equal_options_have_equal_keys_and_equal_hashes() {
        assert!(key_of(&base()) == key_of(&base()));
        assert_eq!(hash_of(&key_of(&base())), hash_of(&key_of(&base())));
        let busy = CompileOptions {
            raw_version: RawVersion(5),
            catalog: queries(Side::Client).clone(),
            keep_source: true,
            ..base()
        };
        assert!(key_of(&busy) == key_of(&busy));
        assert_eq!(hash_of(&key_of(&busy)), hash_of(&key_of(&busy)));
    }

    #[test]
    fn the_raw_version_is_part_of_the_key() {
        for raw in [-1, 0, 1, 12, 13, 14, 99, i16::MIN, i16::MAX] {
            let a = at(raw);
            let b = CompileOptions {
                raw_version: RawVersion(raw.wrapping_add(1)),
                ..base()
            };
            assert_keys_differ(&a, &b);
        }
    }

    #[test]
    fn versions_with_the_same_gates_but_different_raw_values_have_different_keys() {
        // 13, 14 and 99 gate alike, but 14 and 99 resolve no query.
        assert_keys_differ(&at(13), &at(14));
        assert_keys_differ(&at(14), &at(99));
        assert_keys_differ(&at(-1), &at(-2));
    }

    #[test]
    fn the_gate_version_is_a_function_of_the_raw_version_and_adds_nothing() {
        let by_enum = CompileOptions {
            raw_version: MolangVersion::V1.into(),
            ..base()
        };
        let by_raw = at(1);
        assert!(key_of(&by_enum) == key_of(&by_raw));
        assert_eq!(key_of(&base()).raw_version, Some(RawVersion(13)));
    }

    #[test]
    fn a_source_without_its_context_version_has_its_own_key() {
        let unapplied = OptionsKey::new(&base(), None);
        assert!(
            unapplied == OptionsKey::new(&at(5), None),
            "the options' version does not count"
        );
        assert!(unapplied != OptionsKey::new(&base(), Some(RawVersion(-1))));
        assert!(
            unapplied
                != key_of(&CompileOptions {
                    raw_version: MolangVersion::Invalid.into(),
                    ..base()
                })
        );
        assert_eq!(unapplied.raw_version, None);
    }

    #[test]
    fn the_query_sets_are_part_of_the_key() {
        assert_keys_differ(&base(), &sets(QuerySetMask::TAGS));
        assert_keys_differ(&base(), &sets(QuerySetMask::empty()));
        assert_keys_differ(&base(), &sets(QuerySetMask::BUILTIN));
    }

    #[test]
    fn the_allowed_operations_are_part_of_the_key() {
        assert_keys_differ(&base(), &ops(OpSet::all().without_assignments()));
        assert_keys_differ(
            &ops(OpSet::all().without_assignments()),
            &ops(OpSet::all().without_assignments_or_random()),
        );
        assert_keys_differ(&base(), &ops(OpSet::empty()));
        assert_keys_differ(&base(), &ops(OpSet::all().without(ExpressionOp::Loop)));
    }

    fn list(names: &[&str]) -> QueryAllowList {
        QueryAllowList::new(&base().catalog, names).expect("standard query names")
    }

    #[test]
    fn the_allow_list_is_compared_by_its_queries() {
        let (first, same) = (
            list(&[query::IS_BABY, query::IS_ON_GROUND]),
            list(&[query::IS_BABY, query::IS_ON_GROUND]),
        );
        let (a, b) = (only(&first), only(&same));
        assert!(key_of(&a) == key_of(&b), "two lists of the same queries");
        assert_eq!(hash_of(&key_of(&a)), hash_of(&key_of(&b)));
    }

    #[test]
    fn a_different_allow_list_is_a_different_key() {
        let (one, two, other) = (
            list(&[query::IS_BABY]),
            list(&[query::IS_BABY, query::IS_ON_GROUND]),
            list(&[query::IS_ON_GROUND]),
        );
        assert_keys_differ(&only(&one), &only(&two));
        assert_keys_differ(&only(&one), &only(&other));
        assert_keys_differ(&base(), &only(&one));
        assert_keys_differ(&sets(QuerySetMask::empty()), &only(&one));
    }

    #[test]
    fn the_allow_list_order_and_repetition_do_not_matter() {
        let ab = list(&[query::IS_BABY, query::IS_ON_GROUND]);
        let ba = list(&[query::IS_ON_GROUND, query::IS_BABY, query::IS_ON_GROUND]);
        assert!(key_of(&only(&ab)) == key_of(&only(&ba)));
        assert_eq!(hash_of(&key_of(&only(&ab))), hash_of(&key_of(&only(&ba))));
    }

    #[test]
    fn lists_of_the_same_names_from_different_catalogues_share_a_key() {
        let theirs = QueryAllowList::new(queries(Side::Client), [query::IS_BABY])
            .expect("standard query names");
        let ours = list(&[query::IS_BABY]);
        assert!(key_of(&only(&ours)) == key_of(&only(&theirs)));
        assert_eq!(
            hash_of(&key_of(&only(&ours))),
            hash_of(&key_of(&only(&theirs)))
        );
        let cache = CompileCache::new();
        let first = cache.compile("q.is_baby", &only(&ours));
        assert!(Arc::ptr_eq(
            &first,
            &cache.compile("q.is_baby", &only(&theirs))
        ));
    }

    #[test]
    fn no_list_is_the_default_key() {
        assert_eq!(
            key_of(&base()).opts.admission,
            QueryAdmission::Sets(QuerySetMask::DEFAULT)
        );
    }

    #[test]
    fn the_math_catalogue_is_part_of_the_key_by_identity() {
        use crate::catalog::{Arity, MathCatalog, MathDecl};
        let catalog = || {
            MathCatalog::new([MathDecl::pure("math.f", Arity::exactly(1), |a| a[0]).unwrap()])
                .unwrap()
        };
        let (one, other) = (catalog(), catalog());
        let with = |math: &MathCatalog| CompileOptions {
            math: Some(math.clone()),
            ..base()
        };
        assert_keys_differ(&base(), &with(&one));
        assert_keys_differ(&with(&one), &with(&other));
        let clone = one.clone();
        assert!(key_of(&with(&one)) == key_of(&with(&clone)));
        assert_eq!(
            hash_of(&key_of(&with(&one))),
            hash_of(&key_of(&with(&clone)))
        );
    }

    #[test]
    fn the_catalogue_is_part_of_the_key_by_identity() {
        let copy = queries(Side::Server).extended([]).unwrap();
        assert_keys_differ(
            &base(),
            &CompileOptions {
                catalog: copy.clone(),
                ..base()
            },
        );
        let clone = copy.clone();
        assert!(
            key_of(&CompileOptions {
                catalog: copy.clone(),
                ..base()
            }) == key_of(&CompileOptions {
                catalog: clone,
                ..base()
            })
        );
        let cache = CompileCache::new();
        let first = cache.compile("q.is_baby", &base());
        let other = cache.compile(
            "q.is_baby",
            &CompileOptions {
                catalog: copy,
                ..base()
            },
        );
        assert!(!Arc::ptr_eq(&first, &other));
        assert_eq!((cache.len(), cache.misses()), (2, 2));
    }

    #[test]
    fn the_experiments_are_part_of_the_key() {
        let five = ExperimentMask::empty().with(Experiment::new(5).expect("an experiment"));
        let six = ExperimentMask::empty().with(Experiment::new(6).expect("an experiment"));
        assert_keys_differ(
            &base(),
            &CompileOptions {
                experiments: five,
                ..base()
            },
        );
        assert_keys_differ(
            &CompileOptions {
                experiments: five,
                ..base()
            },
            &CompileOptions {
                experiments: six,
                ..base()
            },
        );
    }

    #[test]
    fn each_deviation_switch_is_part_of_the_key() {
        let all = Deviations::ALL;
        for changed in [
            Deviations {
                true_false_prefix_advance: false,
                ..all
            },
            Deviations {
                source_length_limit: false,
                ..all
            },
            Deviations {
                validate_nested: false,
                ..all
            },
            Deviations {
                query_arity_lint: false,
                ..all
            },
            Deviations {
                query_client_only: false,
                ..all
            },
            Deviations {
                object_version_warning: false,
                ..all
            },
            Deviations {
                diagnostic_limit: false,
                ..all
            },
        ] {
            assert_keys_differ(
                &base(),
                &CompileOptions {
                    deviations: changed,
                    ..base()
                },
            );
        }
    }

    #[test]
    fn options_that_differ_in_exactly_one_field_never_share_a_key() {
        let five = ExperimentMask::empty().with(Experiment::new(5).expect("an experiment"));
        let list = list(&[query::IS_BABY]);
        let variants = [
            at(5),
            sets(QuerySetMask::TAGS),
            ops(OpSet::empty()),
            only(&list),
            CompileOptions {
                experiments: five,
                ..base()
            },
            CompileOptions {
                keep_source: true,
                ..base()
            },
            client(),
            CompileOptions {
                deviations: Deviations::NONE,
                ..base()
            },
        ];
        for (index, a) in variants.iter().enumerate() {
            assert_keys_differ(&base(), a);
            for b in &variants[index + 1..] {
                assert_keys_differ(a, b);
            }
        }
    }

    #[test]
    fn a_cache_key_holds_the_options_and_the_text() {
        let key = |text: &str, opts: &CompileOptions| CacheKey {
            opts: OptionsKey::for_options(opts),
            src: Arc::from(text),
        };
        assert!(key("v.x", &base()) == key("v.x", &base()));
        assert_eq!(hash_of(&key("v.x", &base())), hash_of(&key("v.x", &base())));
        assert!(key("v.x", &base()) != key("v.y", &base()));
        assert!(key("v.x", &base()) != key("v.x", &client()));
        assert!(
            key("v.x", &base()) != key("V.x", &base()),
            "the text is compared as written"
        );
        assert!(key("", &base()) == key("", &base()));
    }

    #[test]
    fn a_new_cache_is_empty() {
        let cache = CompileCache::new();
        assert!(cache.is_empty());
        assert_eq!(cache.len(), 0);
        assert_eq!(cache.hits(), 0);
        assert_eq!(cache.misses(), 0);
        let default = CompileCache::default();
        assert!(default.is_empty());
    }

    #[test]
    fn the_first_request_misses_and_the_second_hits_with_the_same_arc() {
        let cache = CompileCache::new();
        let first = cache.compile("v.x*2", &base());
        assert_eq!((cache.len(), cache.hits(), cache.misses()), (1, 0, 1));
        let second = cache.compile("v.x*2", &base());
        assert_eq!((cache.len(), cache.hits(), cache.misses()), (1, 1, 1));
        assert!(Arc::ptr_eq(&first, &second));
        assert!(!cache.is_empty());
    }

    #[test]
    fn a_hit_returns_exactly_what_a_fresh_compile_returns() {
        let cache = CompileCache::new();
        for src in ["v.x*2+1", "1 +", "q.is_name_any", "1e;", "math.min(1)"] {
            let cached = cache.compile(src, &client());
            let hit = cache.compile(src, &client());
            let fresh = compile(src, &client());
            assert_eq!(hit.failure(), fresh.failure(), "{src}");
            assert_eq!(hit.diagnostics(), fresh.diagnostics(), "{src}");
            assert_eq!(hit.tree_notation(9), fresh.tree_notation(9), "{src}");
            assert!(Arc::ptr_eq(&cached, &hit));
        }
    }

    #[test]
    fn different_texts_and_different_options_are_different_entries() {
        let cache = CompileCache::new();
        let a = cache.compile("v.x", &base());
        let b = cache.compile("v.y", &base());
        let c = cache.compile("v.x", &client());
        let d = cache.compile("v.x", &at(5));
        assert_eq!(cache.len(), 4);
        assert_eq!((cache.hits(), cache.misses()), (0, 4));
        assert!(!Arc::ptr_eq(&a, &b) && !Arc::ptr_eq(&a, &c) && !Arc::ptr_eq(&a, &d));
    }

    #[test]
    fn compile_source_compiles_at_the_version_of_the_source() {
        let cache = CompileCache::new();
        let field = base();
        let old = MolangSource::string("'a' + 1", 2);
        let new = MolangSource::string("'a' + 1", 13);
        let accepted = cache.compile_source(&old, &field);
        let rejected = cache.compile_source(&new, &field);
        assert_eq!(accepted.failure(), None);
        assert_eq!(rejected.failure(), Some(CompileFailure::Rejected));
        assert_eq!(cache.len(), 2, "the source's version is in the key");
        assert!(Arc::ptr_eq(&accepted, &cache.compile_source(&old, &field)));
        assert_eq!((cache.hits(), cache.misses()), (1, 2));
    }

    #[test]
    fn compile_source_keys_like_the_options_with_the_sources_version_applied() {
        let cache = CompileCache::new();
        let source = MolangSource::string("v.x", 7);
        let by_source = cache.compile_source(&source, &base());
        let by_text = cache.compile("v.x", &at(7));
        assert!(Arc::ptr_eq(&by_source, &by_text));
        assert_eq!((cache.len(), cache.hits(), cache.misses()), (1, 1, 1));
    }

    #[test]
    fn compile_source_ignoring_version_shares_entries_between_versions() {
        let cache = CompileCache::new();
        let opts = base();
        let one = MolangSource::string("v.x", 1);
        let two = MolangSource::object("v.x", 9);
        let first = cache.compile_source_ignoring_version(&one, &opts);
        let second = cache.compile_source_ignoring_version(&two, &opts);
        assert!(Arc::ptr_eq(&first, &second));
        assert_eq!((cache.len(), cache.hits(), cache.misses()), (1, 1, 1));
        assert_eq!(first.expr().map(Expr::version), Some(MolangVersion::LATEST));
        assert!(Arc::ptr_eq(&first, &cache.compile("v.x", &opts)));
        assert_eq!(cache.len(), 1);
    }

    #[test]
    fn the_two_source_entry_points_with_one_version_share_an_entry() {
        let cache = CompileCache::new();
        let source = MolangSource::string("v.x", 13);
        let a = cache.compile_source(&source, &base());
        let b = cache.compile_source_ignoring_version(&source, &base());
        assert!(Arc::ptr_eq(&a, &b));
    }

    #[test]
    fn clear_empties_the_cache_but_keeps_the_counters_and_the_arcs() {
        let cache = CompileCache::new();
        let held = cache.compile("v.x", &base());
        let _ = cache.compile("v.x", &base());
        let _ = cache.compile("v.y", &base());
        assert_eq!((cache.len(), cache.hits(), cache.misses()), (2, 1, 2));
        cache.clear();
        assert!(cache.is_empty());
        assert_eq!(cache.len(), 0);
        assert_eq!((cache.hits(), cache.misses()), (1, 2));
        assert!(held.expr().is_some(), "the handed-out value stays valid");
        let again = cache.compile("v.x", &base());
        assert!(
            !Arc::ptr_eq(&held, &again),
            "a cleared entry compiles again"
        );
        assert_eq!((cache.len(), cache.hits(), cache.misses()), (1, 1, 3));
    }

    #[test]
    fn clearing_an_empty_cache_does_nothing() {
        let cache = CompileCache::new();
        cache.clear();
        assert!(cache.is_empty());
        assert_eq!((cache.hits(), cache.misses()), (0, 0));
    }

    #[test]
    fn a_rejected_compile_is_cached_too() {
        let cache = CompileCache::new();
        let first = cache.compile("1 +", &base());
        let second = cache.compile("1 +", &base());
        assert_eq!(first.failure(), Some(CompileFailure::Rejected));
        assert!(Arc::ptr_eq(&first, &second));
        assert_eq!((cache.hits(), cache.misses()), (1, 1));
    }

    #[test]
    fn the_empty_text_is_a_key_like_any_other() {
        let cache = CompileCache::new();
        let a = cache.compile("", &base());
        let b = cache.compile("", &base());
        assert!(Arc::ptr_eq(&a, &b));
        assert_eq!(cache.len(), 1);
    }

    #[test]
    fn debug_shows_the_length_and_the_counters() {
        let cache = CompileCache::new();
        assert_eq!(
            format!("{cache:?}"),
            "CompileCache { len: 0, hits: 0, misses: 0, .. }"
        );
        let _ = cache.compile("v.x", &base());
        let _ = cache.compile("v.x", &base());
        let _ = cache.compile("v.y", &base());
        assert_eq!(
            format!("{cache:?}"),
            "CompileCache { len: 2, hits: 1, misses: 2, .. }"
        );
    }

    #[test]
    fn the_cache_is_shareable_between_threads() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<CompileCache>();
    }

    #[test]
    fn threads_racing_on_one_key_all_get_the_stored_arc() {
        let cache = CompileCache::new();
        let results: Vec<Arc<Compiled>> = std::thread::scope(|scope| {
            let handles: Vec<_> = (0..8)
                .map(|_| scope.spawn(|| cache.compile("v.x*2+1", &base())))
                .collect();
            handles
                .into_iter()
                .map(|handle| handle.join().expect("a thread"))
                .collect()
        });
        assert_eq!(cache.len(), 1);
        assert!(
            results
                .windows(2)
                .all(|pair| Arc::ptr_eq(&pair[0], &pair[1]))
        );
        assert_eq!(cache.hits() + cache.misses(), 8);
        assert!(cache.misses() >= 1);
    }

    #[test]
    fn many_distinct_texts_each_miss_once() {
        let cache = CompileCache::new();
        for index in 0..200 {
            let _ = cache.compile(&format!("v.x*{index}"), &base());
        }
        for index in 0..200 {
            let _ = cache.compile(&format!("v.x*{index}"), &base());
        }
        assert_eq!((cache.len(), cache.hits(), cache.misses()), (200, 200, 200));
    }
}
