//! `CompileCache`: its key, cached diagnostics and thread safety.

#![cfg(all(feature = "cache", feature = "stdlib"))]

mod common;

use molangx::catalog::{QueryAllowList, Side};
use std::sync::{Arc, Barrier};

use common::compile_support::server_at;
use molangx::cache::CompileCache;
use molangx::catalog::{QueryAdmission, QuerySetMask};
use molangx::compile::{CompileFailure, CompileOptions, Compiled, Deviations, Expr, compile};
use molangx::diag::LanguageMessage;
use molangx::json::MolangSource;
use molangx::ops::OpSet;
use molangx::stdlib::query;
use molangx::version::{Experiment, ExperimentMask, MolangVersion};

/// Groups differently at 4, 5 and 6: below 5 `a ? b : c ? d : e` is `(a ? b : c) ? d : e` (here 3,
/// from 5 on 0); below 6 `||` binds tighter than `&&` (here 0, from 6 on 1).
const GROUPING: &str = "(1 ? 0 : 1 ? 2 : 3) + (1 || 0 && 0)";

fn constant(compiled: &Compiled) -> Option<f32> {
    compiled.expr().and_then(Expr::as_constant)
}

fn language_messages(compiled: &Compiled) -> Vec<LanguageMessage> {
    compiled
        .diagnostics()
        .iter()
        .filter_map(|d| d.language_message())
        .collect()
}

fn assert_two_entries(cache: &CompileCache, a: &Arc<Compiled>, b: &Arc<Compiled>) {
    assert!(!Arc::ptr_eq(a, b));
    assert_eq!((cache.len(), cache.hits(), cache.misses()), (2, 0, 2));
}

/// Compiles `src` under `a` then `b` in a fresh cache; asserts two entries, each equal to a fresh
/// compile.
fn two(src: &str, a: &CompileOptions, b: &CompileOptions) -> (Arc<Compiled>, Arc<Compiled>) {
    let cache = CompileCache::new();
    let (x, y) = (cache.compile(src, a), cache.compile(src, b));
    assert_two_entries(&cache, &x, &y);
    assert_eq!(
        format!("{:?}", x.diagnostics()),
        format!("{:?}", compile(src, a).diagnostics())
    );
    assert_eq!(
        format!("{:?}", y.diagnostics()),
        format!("{:?}", compile(src, b).diagnostics())
    );
    (x, y)
}

#[test]
fn hits_return_the_same_arc() {
    let cache = CompileCache::new();
    let opts = server_at(13);
    let first = cache.compile("v.x = 1; return v.x * 2;", &opts);
    let second = cache.compile("v.x = 1; return v.x * 2;", &opts);
    assert!(Arc::ptr_eq(&first, &second));
    // A `MolangSource` with the same text shares the entry with the plain string.
    let src = MolangSource::string("v.x = 1; return v.x * 2;", 13);
    assert!(Arc::ptr_eq(
        &first,
        &cache.compile_source(
            &src,
            &CompileOptions::for_source(molangx::stdlib::queries(Side::Server).clone(), &src)
                .expect("a source with a version")
        )
    ));
    assert_eq!((cache.len(), cache.hits(), cache.misses()), (1, 2, 1));
    // The text is compared byte for byte, although the lexer lower-cases.
    let upper = cache.compile("V.X = 1; return v.x * 2;", &opts);
    assert!(!Arc::ptr_eq(&first, &upper));
    assert_eq!(cache.len(), 2);
}

#[test]
fn same_text_at_different_versions_compiles_per_version() {
    let cache = CompileCache::new();
    let results: Vec<_> = [4, 5, 6]
        .into_iter()
        .map(|version| {
            let src = MolangSource::string(GROUPING, version);
            cache.compile_source(
                &src,
                &CompileOptions::for_source(molangx::stdlib::queries(Side::Server).clone(), &src)
                    .expect("a source with a version"),
            )
        })
        .collect();
    assert_eq!((cache.len(), cache.hits(), cache.misses()), (3, 0, 3));
    assert_eq!(
        results.iter().map(|c| constant(c)).collect::<Vec<_>>(),
        [Some(3.0), Some(0.0), Some(1.0)]
    );
    assert_eq!(
        results[0].expr().map(Expr::version),
        Some(MolangVersion::V4)
    );
    for (version, expected) in [4, 5, 6].into_iter().zip(&results) {
        assert!(Arc::ptr_eq(
            expected,
            &cache.compile(GROUPING, &server_at(version))
        ));
    }
    assert_eq!(cache.hits(), 3);
}

#[test]
fn the_source_version_wins_over_the_field_options() {
    let cache = CompileCache::new();
    let old = MolangSource::string(GROUPING, 4);
    let new = MolangSource::string(GROUPING, 6);
    let field = server_at(13);
    let a = cache.compile_source(&old, &field);
    let b = cache.compile_source(&new, &field);
    assert_eq!((constant(&a), constant(&b)), (Some(3.0), Some(1.0)));
    assert_eq!(cache.len(), 2);
    assert_eq!(
        constant(&a),
        constant(&molangx::compile::compile_source(&old, &field))
    );
    assert!(Arc::ptr_eq(&a, &cache.compile_source(&old, &field)));
}

#[test]
fn the_options_version_wins_when_the_source_version_is_ignored() {
    let cache = CompileCache::new();
    let old = MolangSource::string(GROUPING, 4);
    let new = MolangSource::string(GROUPING, 6);
    let a = cache.compile_source_ignoring_version(&old, &server_at(6));
    let b = cache.compile_source_ignoring_version(&new, &server_at(6));
    assert!(Arc::ptr_eq(&a, &b));
    assert_eq!(constant(&a), Some(1.0));
    let c = cache.compile_source_ignoring_version(
        &old,
        &CompileOptions::for_source(molangx::stdlib::queries(Side::Server).clone(), &old)
            .expect("a source with a version"),
    );
    assert_eq!(constant(&c), Some(3.0));
    assert_eq!(cache.len(), 2);
}

#[test]
fn key_version() {
    let (x, y) = two(
        GROUPING,
        &CompileOptions {
            raw_version: MolangVersion::V4.into(),
            ..server_at(13)
        },
        &server_at(6),
    );
    assert_eq!((constant(&x), constant(&y)), (Some(3.0), Some(1.0)));
}

#[test]
fn key_raw_version() {
    // 13 and 14 gate alike, but no query is declared above version 13.
    let (x, y) = two("query.is_baby", &server_at(13), &server_at(14));
    assert_eq!(x.diagnostics().len(), 0);
    assert_eq!(
        language_messages(&y).first(),
        Some(&LanguageMessage::QueryUnresolved)
    );
}

#[test]
fn key_query_sets() {
    let (x, y) = two(
        "query.any_tag('minecraft:stone')",
        &server_at(13),
        &CompileOptions {
            admission: QueryAdmission::Sets(QuerySetMask::TAGS),
            ..server_at(13)
        },
    );
    assert_eq!(
        language_messages(&x).first(),
        Some(&LanguageMessage::QueryUnresolved)
    );
    assert!(y.parses_cleanly());
}

#[test]
fn key_allowed_ops() {
    let (x, y) = two(
        "v.x = 1;",
        &server_at(13),
        &CompileOptions {
            allowed_ops: OpSet::all().without_assignments_or_random(),
            ..server_at(13)
        },
    );
    assert_eq!(x.failure(), None);
    assert_eq!(y.failure(), Some(CompileFailure::Rejected));
}

#[test]
fn key_allow_list() {
    fn only(list: &QueryAllowList) -> CompileOptions {
        CompileOptions {
            admission: QueryAdmission::Only(list.clone()),
            ..server_at(13)
        }
    }
    let list =
        |names: &[&str]| QueryAllowList::new(&server_at(13).catalog, names).expect("declared");
    let (block_state, is_baby) = (list(&[query::BLOCK_STATE]), list(&[query::IS_BABY]));
    let (x, y) = two("query.is_baby", &only(&block_state), &only(&is_baby));
    assert_eq!(
        language_messages(&x).first(),
        Some(&LanguageMessage::QueryUnresolved)
    );
    assert!(y.parses_cleanly());

    // The key holds the queries, not the list's address.
    let cache = CompileCache::new();
    let first = cache.compile("query.is_baby", &only(&is_baby));
    let copy = list(&[query::IS_BABY]);
    assert!(Arc::ptr_eq(
        &first,
        &cache.compile("query.is_baby", &only(&copy))
    ));
    // Order and repetition do not change a list, so they share an entry.
    let ab = list(&[query::IS_BABY, query::BLOCK_STATE]);
    let ba = list(&[query::BLOCK_STATE, query::IS_BABY, query::IS_BABY]);
    let x = cache.compile("query.is_baby", &only(&ab));
    let y = cache.compile("query.is_baby", &only(&ba));
    assert!(Arc::ptr_eq(&x, &y));
    assert_eq!(cache.len(), 2);
}

#[test]
fn key_experiments() {
    // No standard query is experiment-gated; the helper catalogue makes the difference visible.
    let on = ExperimentMask::empty().with(Experiment::new(3).expect("experiment 3"));
    let (x, y) = two(
        "1",
        &server_at(13),
        &CompileOptions {
            experiments: on,
            ..server_at(13)
        },
    );
    assert_eq!((constant(&x), constant(&y)), (Some(1.0), Some(1.0)));
    {
        let tests = CompileOptions {
            catalog: common::reference_catalog().clone(),
            admission: QueryAdmission::Sets(common::REFERENCE_SETS),
            ..server_at(13)
        };
        let (x, y) = two(
            "query.experimental_test",
            &tests.clone(),
            &CompileOptions {
                experiments: common::reference_experiments(),
                ..tests
            },
        );
        assert_eq!(
            language_messages(&x).first(),
            Some(&LanguageMessage::QueryUnresolved)
        );
        assert!(y.parses_cleanly());
    }
}

#[test]
fn key_keep_source() {
    let (x, y) = two(
        "1 + v.x",
        &server_at(13),
        &CompileOptions {
            keep_source: true,
            ..server_at(13)
        },
    );
    assert_eq!(x.expr().and_then(Expr::source), None);
    assert_eq!(y.expr().and_then(Expr::source), Some("1 + v.x"));
}

#[test]
fn key_side() {
    // The server catalogue lacks `query.is_on_screen`.
    let (x, y) = two(
        "query.is_on_screen",
        &CompileOptions {
            catalog: molangx::stdlib::queries(Side::Server).clone(),
            ..server_at(13)
        },
        &CompileOptions {
            catalog: molangx::stdlib::queries(Side::Client).clone(),
            ..server_at(13)
        },
    );
    assert_eq!(
        language_messages(&x).first(),
        Some(&LanguageMessage::QueryUnresolved)
    );
    assert!(y.parses_cleanly());
}

#[test]
fn key_deviations() {
    // `object_version_warning` informs about a raw version of −1.
    let (x, y) = two(
        "1",
        &server_at(-1),
        &CompileOptions {
            deviations: Deviations::NONE,
            ..server_at(-1)
        },
    );
    assert_eq!((x.diagnostics().len(), y.diagnostics().len()), (1, 0));
}

/// The loader attributes a hit's diagnostics to every JSON path that used the text.
#[test]
fn hits_keep_their_diagnostics_for_every_path() {
    let cache = CompileCache::new();
    let opts = server_at(13);
    let uses = [
        (
            "entity/a.json#/minecraft:entity/description/scripts/pre_animation/0",
            "1 +",
        ),
        (
            "entity/b.json#/minecraft:entity/description/scripts/pre_animation/0",
            "1 +",
        ),
        (
            "entity/b.json#/minecraft:entity/components/minecraft:health/value",
            "20",
        ),
    ];
    let mut report = Vec::new();
    for (path, text) in uses {
        let compiled = cache.compile(text, &opts);
        for diagnostic in compiled.diagnostics() {
            report.push((path, diagnostic.message().trim_end().to_owned()));
        }
    }
    assert_eq!((cache.hits(), cache.misses()), (1, 2));
    let paths: Vec<_> = report.iter().map(|(path, _)| *path).collect();
    assert_eq!(
        paths,
        [uses[0].0, uses[1].0],
        "both files that use `1 +` get its error: {report:?}"
    );
    assert_eq!(report[0].1, report[1].1);
}

#[test]
fn counters_len_and_clear() {
    let cache = CompileCache::new();
    assert!(cache.is_empty());
    assert_eq!((cache.len(), cache.hits(), cache.misses()), (0, 0, 0));
    let opts = server_at(13);
    let kept = cache.compile("math.sin(30)", &opts);
    for _ in 0..4 {
        let _ = cache.compile("math.sin(30)", &opts);
    }
    let _ = cache.compile("math.cos(30)", &opts);
    assert_eq!((cache.len(), cache.hits(), cache.misses()), (2, 4, 2));
    assert!(format!("{cache:?}").contains("hits: 4"));

    // `clear` drops the entries and keeps the counters; handed-out `Arc`s stay valid.
    cache.clear();
    assert!(cache.is_empty());
    assert_eq!((cache.hits(), cache.misses()), (4, 2));
    assert_eq!(constant(&kept), constant(&compile("math.sin(30)", &opts)));
    let again = cache.compile("math.sin(30)", &opts);
    assert!(!Arc::ptr_eq(&kept, &again), "a cleared entry compiles anew");
    assert_eq!((cache.len(), cache.misses()), (1, 3));
}

#[test]
fn types_are_send_and_sync() {
    const fn assert_send_sync<T: Send + Sync>() {}
    const {
        assert_send_sync::<CompileCache>();
        assert_send_sync::<Compiled>();
        assert_send_sync::<Expr>();
        assert_send_sync::<Arc<Compiled>>();
    }
}

/// Duplicate compiles under a race are allowed and discarded.
#[test]
fn concurrent_requests_share_one_arc() {
    const THREADS: usize = 16;
    let cache = CompileCache::new();
    let barrier = Barrier::new(THREADS);
    let opts = server_at(13);
    let results: Vec<Arc<Compiled>> = std::thread::scope(|scope| {
        let handles: Vec<_> = (0..THREADS)
            .map(|_| {
                scope.spawn(|| {
                    barrier.wait();
                    let first = cache.compile(GROUPING, &opts);
                    // Interleave other keys so the shards see writes while others read.
                    for v in 0..8 {
                        let _ = cache.compile(&format!("{GROUPING} + {v}"), &opts);
                    }
                    first
                })
            })
            .collect();
        handles
            .into_iter()
            .map(|h| h.join().expect("thread"))
            .collect()
    });
    let stored = cache.compile(GROUPING, &opts);
    assert!(results.iter().all(|r| Arc::ptr_eq(r, &stored)));
    assert_eq!(cache.len(), 9);
    let requests = (THREADS * 9 + 1) as u64;
    assert_eq!(cache.hits() + cache.misses(), requests);
    // At least one compile per key, at most one per thread and key.
    assert!(
        (9..=(THREADS * 9) as u64).contains(&cache.misses()),
        "{} misses",
        cache.misses()
    );
}

#[test]
fn threads_with_different_keys_get_their_own_entries() {
    const THREADS: usize = 12;
    let cache = CompileCache::new();
    let barrier = Barrier::new(THREADS);
    let versions = [(4_i16, 3.0_f32), (5, 0.0), (6, 1.0)];
    let results: Vec<(i16, Arc<Compiled>)> = std::thread::scope(|scope| {
        let handles: Vec<_> = (0..THREADS)
            .map(|n| {
                let (cache, barrier) = (&cache, &barrier);
                scope.spawn(move || {
                    let (version, _) = versions[n % versions.len()];
                    barrier.wait();
                    (version, cache.compile(GROUPING, &server_at(version)))
                })
            })
            .collect();
        handles
            .into_iter()
            .map(|h| h.join().expect("thread"))
            .collect()
    });
    assert_eq!(cache.len(), 3);
    for (version, compiled) in &results {
        let expected = versions
            .iter()
            .find(|(v, _)| v == version)
            .map(|(_, value)| *value);
        assert_eq!(constant(compiled), expected, "version {version}");
        assert_eq!(
            format!("{:?}", compiled.diagnostics()),
            format!(
                "{:?}",
                compile(GROUPING, &server_at(*version)).diagnostics()
            )
        );
    }
    for (version, _) in versions {
        let same: Vec<_> = results
            .iter()
            .filter(|(v, _)| *v == version)
            .map(|(_, c)| c)
            .collect();
        let stored = cache.compile(GROUPING, &server_at(version));
        assert!(
            same.iter().all(|c| Arc::ptr_eq(c, &stored)),
            "version {version}: one stored entry"
        );
    }
}
