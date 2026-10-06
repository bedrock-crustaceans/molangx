//! The pack corpus: every Molang string of the 1.26.45 packs, at the version its pack declares. The
//! strings are **never committed**; the tests read `$MOLANG_CORPUS_DIR/corpus_pairs.json` (see
//! CONTRIBUTING.md) and are ignored otherwise:
//!
//! ```text
//! MOLANG_CORPUS_DIR=/path/to/corpus cargo test --release --features compiler --test corpus -- --ignored --nocapture
//! ```
//!
//! Every pair must compile without a rejection. A string that logs goes into [`ALLOWED`] by index
//! and reason, never by text.

#![cfg(all(feature = "compiler", feature = "stdlib"))]

use molangx::version::RawVersion;
use std::path::PathBuf;
use std::time::Instant;

use molangx::catalog::{QueryAdmission, QuerySetMask, Side};
use molangx::compile::{CompileFailure, CompileOptions, compile};
use molangx::diag::{LanguageMessage, Severity};
use serde_json::Value;

/// `(language message row, count)` pairs the corpus may produce. Expected empty.
const ALLOWED: &[(u8, usize)] = &[];

/// Of 460 distinct literals, 2 (two-decimal fractions) read differently from a correctly rounded
/// `f32`.
const MEASURED_LITERAL_DIFFERENCES: usize = 2;

/// `fragment`: every occurrence is an element of a client-entity / attachable `scripts.initialize`
/// or `scripts.pre_animation` array. Such elements may only parse together (`(…) ? {`), so they
/// only have to tokenise.
struct Pair {
    expr: String,
    version: i16,
    fragment: bool,
}

fn is_concatenated_script_element(pointer: &str) -> bool {
    let Some((parent, index)) = pointer.rsplit_once('/') else {
        return false;
    };
    index.bytes().all(|b| b.is_ascii_digit())
        && (parent.ends_with("/scripts/pre_animation") || parent.ends_with("/scripts/initialize"))
}

fn corpus() -> Option<Vec<Pair>> {
    let dir = std::env::var_os("MOLANG_CORPUS_DIR")?;
    let path = PathBuf::from(dir).join("corpus_pairs.json");
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()));
    let json: Value = serde_json::from_str(&text)
        .unwrap_or_else(|e| panic!("cannot parse {}: {e}", path.display()));
    let mut pairs = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for pair in json["pairs"].as_array().expect("pairs") {
        let expr = pair["expr"].as_str().expect("expr").to_owned();
        let version =
            i16::try_from(pair["version"].as_i64().expect("version")).expect("version fits i16");
        let occurrences = pair["occurrences"]
            .as_array()
            .map_or(&[][..], Vec::as_slice);
        let fragment = !occurrences.is_empty()
            && occurrences.iter().all(|o| {
                o["pointer"]
                    .as_str()
                    .is_some_and(is_concatenated_script_element)
            });
        // Strings selected by namespace token include shader paths of `.material` files
        // (`…/texture.…`), not Molang.
        if !occurrences.is_empty() && occurrences.iter().all(|o| o["family"] == "materials") {
            continue;
        }
        let mut versions = vec![version];
        // The per-field `version_effective` overrides the file's version for an occurrence.
        for occurrence in occurrences {
            if let Some(effective) = occurrence["version_effective"]
                .as_i64()
                .and_then(|v| i16::try_from(v).ok())
                && !versions.contains(&effective)
            {
                versions.push(effective);
            }
        }
        for (position, effective) in versions.into_iter().enumerate() {
            // An overridden version that is another pair's own version is that pair.
            if seen.insert((expr.clone(), effective)) || position == 0 {
                pairs.push(Pair {
                    expr: expr.clone(),
                    version: effective,
                    fragment,
                });
            }
        }
    }
    Some(pairs)
}

/// The corpus mixes every field of every pack family, so every query set is admitted; per-field
/// restrictions are the loader's.
fn options(version: i16) -> CompileOptions {
    CompileOptions {
        admission: QueryAdmission::Sets(QuerySetMask::BUILTIN),
        ..CompileOptions::from_raw_version(
            molangx::stdlib::queries(Side::Client).clone(),
            RawVersion(version),
        )
    }
}

struct Flipper(u64);

impl Flipper {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    /// Never flips the byte after a backslash, which lower-casing skips.
    fn flip(&mut self, source: &str) -> String {
        let mut out = String::with_capacity(source.len());
        let mut in_string = false;
        let mut protected = false;
        for c in source.chars() {
            if protected {
                protected = false;
                out.push(c);
                continue;
            }
            match c {
                '\'' => in_string = !in_string,
                '\\' => protected = true,
                _ => {}
            }
            if !in_string && c.is_ascii_alphabetic() && self.next() & 1 == 1 {
                out.push(if c.is_ascii_lowercase() {
                    c.to_ascii_uppercase()
                } else {
                    c.to_ascii_lowercase()
                })
            } else {
                out.push(c)
            }
        }
        out
    }
}

#[test]
#[ignore = "needs MOLANG_CORPUS_DIR (Mojang content, never committed)"]
fn corpus_compiles_without_rejections() {
    let Some(pairs) = corpus() else {
        eprintln!("MOLANG_CORPUS_DIR is not set; skipping");
        return;
    };
    let mut flipper = Flipper(0x9e37_79b9_7f4a_7c15);
    let mut found = std::collections::BTreeMap::<u8, usize>::new();
    let mut problems = 0usize;
    let mut rejected = 0usize;
    let mut results = std::collections::BTreeMap::<String, usize>::new();
    let mut fragments_rejected = 0usize;
    for (index, pair) in pairs.iter().enumerate() {
        let (expr, version) = (&pair.expr, pair.version);
        let opts = options(version);
        let compiled = compile(expr, &opts);
        *results
            .entry(format!("{:?}", compiled.failure()))
            .or_default() += 1;
        if pair.fragment && compiled.failure() == Some(CompileFailure::Rejected) {
            let lexer_error = compiled
                .diagnostics()
                .iter()
                .filter_map(|d| d.language_message())
                .any(|m| m.row() <= 6);
            assert!(
                !lexer_error,
                "pair #{index} (version {version}): a script fragment does not tokenise"
            );
            fragments_rejected += 1;
            continue;
        }
        for diagnostic in compiled
            .diagnostics()
            .iter()
            .filter(|d| d.language_message().is_some() || d.severity() == Severity::Error)
        {
            *found
                .entry(
                    diagnostic
                        .language_message()
                        .map_or(0, molangx::diag::LanguageMessage::row),
                )
                .or_default() += 1;
            problems += 1;
            // The text stays out of logs that might be shared; the index identifies the pair.
            eprintln!(
                "pair #{index} (version {version}): {:?} {:?}",
                diagnostic.code(),
                diagnostic.language_message()
            );
        }
        if compiled.failure() == Some(CompileFailure::Rejected) {
            eprintln!("pair #{index} (version {version}) is rejected");
            rejected += 1;
        }

        let again = compile(expr, &opts);
        #[cfg(feature = "fuzz")]
        assert_eq!(
            compiled.tree_notation(9),
            again.tree_notation(9),
            "pair #{index}: two compiles differ"
        );
        assert_eq!(
            compiled.diagnostics(),
            again.diagnostics(),
            "pair #{index}: two compiles differ"
        );

        let flipped = compile(&flipper.flip(expr), &opts);
        #[cfg(feature = "fuzz")]
        assert_eq!(
            compiled.tree_notation(9),
            flipped.tree_notation(9),
            "pair #{index}: case flips change the tree"
        );
        assert_eq!(
            compiled.failure(),
            flipped.failure(),
            "pair #{index}: case flips change the result"
        );
    }
    println!(
        "{fragments_rejected} script fragments tokenise but are not complete expressions on their own"
    );
    assert_eq!(rejected, 0, "{rejected} pairs are rejected");
    println!("{} pairs: {results:?}", pairs.len());
    let allowed: std::collections::BTreeMap<u8, usize> = ALLOWED.iter().copied().collect();
    assert_eq!(
        found, allowed,
        "{problems} diagnostics outside the allow-list (message row → count)"
    );
}

/// Why a query did not resolve in a server compile with the default options.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum ServerMiss {
    /// Declared only in another set (`world_gen`, `tags`), which the default options do not admit.
    OutsideDefaultSet(&'static str),
    /// Not in the server catalogue (`query.is_on_screen`, and names newer than its release).
    NotOnServer,
}

/// The client-side test admits every query set, so it cannot reject a query; with the server
/// defaults each rejection must be a query they do not admit. Rejections are counted by reason so
/// the test is not vacuous.
#[test]
#[ignore = "needs MOLANG_CORPUS_DIR (Mojang content, never committed)"]
fn corpus_on_the_server_rejects_only_queries_it_cannot_resolve() {
    let Some(pairs) = corpus() else {
        eprintln!("MOLANG_CORPUS_DIR is not set; skipping");
        return;
    };
    let server = molangx::stdlib::queries(Side::Server);
    let mut misses = std::collections::BTreeMap::<ServerMiss, usize>::new();
    let mut rejected = 0usize;
    let mut problems = Vec::new();
    for (index, pair) in pairs.iter().enumerate() {
        let compiled = compile(
            &pair.expr,
            &CompileOptions::from_raw_version(server.clone(), RawVersion(pair.version)),
        );
        let language: Vec<LanguageMessage> = compiled
            .diagnostics()
            .iter()
            .filter_map(|d| d.language_message())
            .collect();
        if compiled.failure() != Some(CompileFailure::Rejected) {
            if !language.is_empty()
                || compiled
                    .diagnostics()
                    .iter()
                    .any(|d| d.severity() == Severity::Error)
            {
                problems.push(format!(
                    "pair #{index} (version {}): compiles with {language:?}",
                    pair.version
                ));
            }
            continue;
        }
        if pair.fragment && !language.iter().any(|m| m.row() <= 6) {
            continue;
        }
        rejected += 1;
        // Exactly the two lines of an unresolved query, the first naming it.
        let named = compiled
            .diagnostics()
            .iter()
            .find(|d| d.language_message() == Some(LanguageMessage::QueryUnresolved))
            .and_then(|d| {
                let message = d.message();
                let name = message
                    .strip_prefix("Failed to resolve query ")?
                    .split(".  Either")
                    .next()?;
                server.get(name)
            });
        let Some(decl) = named.filter(|_| {
            language
                == [
                    LanguageMessage::QueryUnresolved,
                    LanguageMessage::UnrecognizedToken,
                ]
        }) else {
            problems.push(format!(
                "pair #{index} (version {}): rejected with {language:?}",
                pair.version
            ));
            continue;
        };
        let miss = if !decl.sets().intersects(QuerySetMask::DEFAULT) {
            ServerMiss::OutsideDefaultSet(decl.sets().name().unwrap_or("several sets"))
        } else if !decl.on_dedicated_server() {
            ServerMiss::NotOnServer
        } else {
            problems.push(format!(
                "pair #{index} (version {}): {} is in the default set and on the server",
                pair.version,
                decl.name()
            ));
            continue;
        };
        *misses.entry(miss).or_default() += 1;
    }
    println!(
        "{} pairs, {rejected} rejected on the server with the defaults: {misses:?}",
        pairs.len()
    );
    assert!(
        problems.is_empty(),
        "{} pairs:\n{}",
        problems.len(),
        problems.join("\n")
    );
    assert!(
        rejected > 0,
        "the defaults admit every query of the corpus: the run proves nothing"
    );
}

#[test]
#[ignore = "needs MOLANG_CORPUS_DIR (Mojang content, never committed)"]
fn corpus_compile_time() {
    let Some(pairs) = corpus() else {
        eprintln!("MOLANG_CORPUS_DIR is not set; skipping");
        return;
    };
    let options: Vec<CompileOptions> = pairs.iter().map(|pair| options(pair.version)).collect();
    let mut best = f64::MAX;
    for _ in 0..5 {
        let start = Instant::now();
        for (pair, opts) in pairs.iter().zip(&options) {
            std::hint::black_box(compile(&pair.expr, opts));
        }
        best = best.min(start.elapsed().as_secs_f64() * 1e3);
    }
    println!("{} pairs compiled in {best:.2} ms (best of 5)", pairs.len());
}

/// Before the per-field overrides [`corpus`] adds.
fn recorded_versions() -> Option<Vec<i16>> {
    let dir = std::env::var_os("MOLANG_CORPUS_DIR")?;
    let path = PathBuf::from(dir).join("corpus_pairs.json");
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()));
    let json: Value = serde_json::from_str(&text)
        .unwrap_or_else(|e| panic!("cannot parse {}: {e}", path.display()));
    let pairs = json["pairs"].as_array().expect("pairs");
    Some(
        pairs
            .iter()
            .map(|p| {
                i16::try_from(p["version"].as_i64().expect("version")).expect("version fits i16")
            })
            .collect(),
    )
}

#[test]
#[ignore = "needs MOLANG_CORPUS_DIR (Mojang content, never committed)"]
fn corpus_pair_counts_per_version() {
    let Some(versions) = recorded_versions() else {
        eprintln!("MOLANG_CORPUS_DIR is not set; skipping");
        return;
    };
    let mut counts = std::collections::BTreeMap::<i16, usize>::new();
    for version in versions {
        *counts.entry(version).or_default() += 1;
    }
    let expected: std::collections::BTreeMap<i16, usize> = [
        (0, 2_094),
        (1, 329),
        (2, 65),
        (4, 58),
        (5, 167),
        (6, 431),
        (7, 173),
        (8, 247),
        (9, 188),
        (10, 41),
        (11, 25),
        (12, 1_056),
        (13, 1_166),
    ]
    .into_iter()
    .collect();
    assert_eq!(counts, expected);
    assert_eq!(counts.values().sum::<usize>(), 6_040);
}

/// Outside `'…'` strings: digits with an optional fraction (or a fraction alone) and exponent, not
/// part of a name.
fn literals(expr: &str) -> Vec<&str> {
    let b = expr.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    let mut in_string = false;
    while i < b.len() {
        let c = b[i];
        if c == b'\'' {
            in_string = !in_string;
            i += 1;
            continue;
        }
        let starts =
            c.is_ascii_digit() || (c == b'.' && b.get(i + 1).is_some_and(u8::is_ascii_digit));
        let in_name =
            i > 0 && (b[i - 1].is_ascii_alphanumeric() || b[i - 1] == b'_' || b[i - 1] == b'.');
        if in_string || !starts || in_name {
            i += 1;
            continue;
        }
        let start = i;
        while i < b.len() && b[i].is_ascii_digit() {
            i += 1;
        }
        if b.get(i) == Some(&b'.') {
            i += 1;
            while i < b.len() && b[i].is_ascii_digit() {
                i += 1;
            }
        }
        if matches!(b.get(i), Some(b'e' | b'E')) {
            let mut j = i + 1;
            if matches!(b.get(j), Some(b'+' | b'-')) {
                j += 1;
            }
            if b.get(j).is_some_and(u8::is_ascii_digit) {
                while j < b.len() && b[j].is_ascii_digit() {
                    j += 1;
                }
                i = j;
            }
        }
        out.push(&expr[start..i]);
    }
    out
}

#[test]
#[ignore = "needs MOLANG_CORPUS_DIR (Mojang content, never committed)"]
fn corpus_literals_against_correct_rounding() {
    let Some(pairs) = corpus() else {
        eprintln!("MOLANG_CORPUS_DIR is not set; skipping");
        return;
    };
    let mut distinct = std::collections::BTreeSet::new();
    for pair in &pairs {
        distinct.extend(literals(&pair.expr).into_iter().map(str::to_owned));
    }
    let opts = options(13);
    let mut differ = Vec::new();
    for literal in &distinct {
        let ours = compile(literal, &opts)
            .expr()
            .cloned()
            .and_then(|e| e.as_constant())
            .unwrap_or_else(|| panic!("{literal} folds to a constant"));
        let rounded: f32 = literal.parse().unwrap_or_else(|e| panic!("{literal}: {e}"));
        if ours.to_bits() != rounded.to_bits() {
            differ.push(literal.clone());
        }
    }
    println!(
        "{} distinct literals, {} read differently from a correctly rounded f32",
        distinct.len(),
        differ.len()
    );
    assert!(distinct.len() > 100, "the scan found the literals");
    assert_eq!(differ.len(), MEASURED_LITERAL_DIFFERENCES, "{differ:?}");
}
