//! Compile benchmarks.
//!
//! * `compile/sample_200`: the 200 expressions of `benches/compile_sample.txt`.
//! * `compile/corpus`: only when `MOLANG_CORPUS_DIR` is set, the pairs of
//!   `$MOLANG_CORPUS_DIR/corpus_pairs.json`, which is never committed. Budget: the whole corpus
//!   (6,040 pairs) in < 50 ms single-threaded.
//!
//! Run with `cargo bench --features compiler --bench compile`.

use std::hint::black_box;
use std::path::PathBuf;

use criterion::{Criterion, criterion_group, criterion_main};
use molangx::catalog::{QueryAdmission, QuerySetMask, Side};
use molangx::compile::{CompileFailure, CompileOptions, compile};
use molangx::stdlib::queries;
use molangx::version::{MolangVersion, RawVersion};

const SAMPLE: &str = include_str!("compile_sample.txt");

fn options(version: i16) -> CompileOptions {
    CompileOptions {
        admission: QueryAdmission::Sets(QuerySetMask::BUILTIN),
        ..CompileOptions::from_raw_version(queries(Side::Client).clone(), RawVersion(version))
    }
}

fn sample(c: &mut Criterion) {
    let lines: Vec<&str> = SAMPLE.lines().filter(|line| !line.is_empty()).collect();
    let opts = options(MolangVersion::LATEST.as_i16());
    for line in &lines {
        let compiled = compile(line, &opts);
        assert!(
            compiled.failure() != Some(CompileFailure::Rejected),
            "`{line}` is rejected: {:?}",
            compiled.diagnostics()
        );
    }
    let mut group = c.benchmark_group("compile");
    group.bench_function(format!("sample_{}", lines.len()), |b| {
        b.iter(|| {
            for line in &lines {
                black_box(compile(black_box(line), &opts));
            }
        });
    });
    group.finish();
}

fn corpus_pairs() -> Option<Vec<(String, i16)>> {
    let dir = std::env::var_os("MOLANG_CORPUS_DIR")?;
    let path = PathBuf::from(dir).join("corpus_pairs.json");
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()));
    let json: serde_json::Value = serde_json::from_str(&text)
        .unwrap_or_else(|e| panic!("cannot parse {}: {e}", path.display()));
    let pairs = json["pairs"]
        .as_array()
        .expect("pairs")
        .iter()
        .map(|pair| {
            let expr = pair["expr"].as_str().expect("expr").to_owned();
            let version = i16::try_from(pair["version"].as_i64().expect("version"))
                .expect("version fits i16");
            (expr, version)
        })
        .collect();
    Some(pairs)
}

fn corpus(c: &mut Criterion) {
    let Some(pairs) = corpus_pairs() else {
        eprintln!("MOLANG_CORPUS_DIR is not set: skipping compile/corpus");
        return;
    };
    let options: Vec<CompileOptions> = pairs.iter().map(|(_, version)| options(*version)).collect();
    let mut group = c.benchmark_group("compile");
    group.sample_size(20);
    group.bench_function(format!("corpus_{}", pairs.len()), |b| {
        b.iter(|| {
            for ((expr, _), opts) in pairs.iter().zip(&options) {
                black_box(compile(black_box(expr), opts));
            }
        });
    });
    group.finish();
}

criterion_group!(benches, sample, corpus);
criterion_main!(benches);
