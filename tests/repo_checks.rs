//! Checks of the repository rather than of the library: manifests, CI, documents, layout, test data
//! and wording. They read files the package does not ship, so they would fail in a packaged crate.

#![cfg(feature = "stdlib")]

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use molangx::catalog::{QueryAdmission, QueryDecl, QuerySetMask, Side};
use molangx::version::{ExperimentMask, MolangVersion, semver::Version};
use serde_json::Value;

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn read(path: &str) -> String {
    std::fs::read_to_string(root().join(path)).unwrap_or_else(|e| panic!("{path}: {e}"))
}

/// The entries of the repository directory `dir`, by name, sorted.
fn file_names(dir: &str) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(root().join(dir))
        .unwrap_or_else(|e| panic!("{dir}: {e}"))
        .filter_map(Result::ok)
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

/// The topic files `measured_*.rs` of `tests/`, sorted.
fn topic_files() -> Vec<String> {
    file_names("tests")
        .into_iter()
        .filter(|name| name.starts_with("measured_") && name.ends_with(".rs"))
        .collect()
}

/// A TOML file of the repository, parsed.
fn toml(path: &str) -> toml::Table {
    read(path).parse().unwrap_or_else(|e| panic!("{path}: {e}"))
}

/// The table at `key` of `table`.
fn table<'a>(table: &'a toml::Table, key: &str) -> &'a toml::Table {
    table
        .get(key)
        .and_then(toml::Value::as_table)
        .unwrap_or_else(|| panic!("no table `{key}`"))
}

/// The strings of a TOML array.
fn strings(value: &toml::Value) -> Vec<&str> {
    value
        .as_array()
        .unwrap_or_else(|| panic!("{value:?} is not an array"))
        .iter()
        .map(|item| {
            item.as_str()
                .unwrap_or_else(|| panic!("{item:?} is not a string"))
        })
        .collect()
}

/// The `[[name]]` tables of the manifest (`example`, `bench`), keyed by their `name`.
fn targets(kind: &str) -> BTreeMap<String, toml::Table> {
    toml("Cargo.toml")
        .get(kind)
        .and_then(toml::Value::as_array)
        .unwrap_or_else(|| panic!("no [[{kind}]]"))
        .iter()
        .map(|target| {
            let target = target.as_table().expect("a table");
            (
                target["name"].as_str().expect("a name").to_owned(),
                target.clone(),
            )
        })
        .collect()
}

/// The `clippy::` lints a crate-level `#![allow(…)]` names, with the line before each.
fn allowed_lints(lib: &str) -> Vec<(String, String)> {
    let lines: Vec<&str> = lib.lines().map(str::trim).collect();
    lines
        .iter()
        .enumerate()
        .filter_map(|(i, line)| {
            let lint = line.strip_prefix("#![allow(clippy::")?.strip_suffix(")]")?;
            Some((
                lint.to_owned(),
                if i == 0 {
                    String::new()
                } else {
                    lines[i - 1].to_owned()
                },
            ))
        })
        .collect()
}

const ALLOWED: [&str; 8] = [
    "cast_possible_truncation",
    "cast_possible_wrap",
    "cast_sign_loss",
    "cast_precision_loss",
    "float_cmp",
    "must_use_candidate",
    "missing_errors_doc",
    "module_name_repetitions",
];

fn example_features() -> BTreeMap<String, String> {
    targets("example")
        .into_iter()
        .map(|(name, example)| {
            // The README names the features to turn on beyond the default `stdlib`.
            let features: Vec<&str> = example.get("required-features").map_or_else(Vec::new, |f| {
                strings(f).into_iter().filter(|f| *f != "stdlib").collect()
            });
            (name, features.join(", "))
        })
        .collect()
}

fn json(path: &str) -> Value {
    serde_json::from_str(&read(path)).unwrap_or_else(|e| panic!("{path}: {e}"))
}

/// Every version, `Invalid` included.
fn versions() -> impl Iterator<Item = MolangVersion> {
    (-1..=13).map(|v| MolangVersion::from_i16(v).unwrap())
}

fn get(name: &str) -> &'static QueryDecl {
    molangx::stdlib::queries(Side::Client)
        .get(name)
        .unwrap_or_else(|| panic!("{name} is not a standard query"))
}

/// Resolution with every built-in set admitted: only the version decides.
fn impl_at(name: &str, v: MolangVersion) -> Option<u8> {
    get(name).resolve(
        molangx::version::RawVersion(v.as_i16()),
        &QueryAdmission::Sets(QuerySetMask::BUILTIN),
        ExperimentMask::empty(),
    )
}

/// The standard queries every supported release has.
fn baseline() -> impl Iterator<Item = &'static QueryDecl> {
    molangx::stdlib::queries(Side::Client)
        .iter()
        .filter(|decl| decl.shape().first_release.is_none())
}

#[test]
fn one_crate_without_bedrock_dependencies() {
    let manifest = toml("Cargo.toml");
    let package = table(&manifest, "package");
    assert_eq!(package["name"].as_str(), Some("molangx"));
    assert_eq!(package["edition"].as_str(), Some("2024"));
    assert_eq!(package["license"].as_str(), Some("Apache-2.0"));
    assert!(!manifest.contains_key("workspace"), "no workspace");
    for section in ["dependencies", "dev-dependencies"] {
        for name in table(&manifest, section).keys() {
            assert!(
                !name.contains("bedrock") && !name.contains("nbtx") && !name.contains("chorus"),
                "{section}: {name}"
            );
        }
    }
}

#[test]
fn feature_set() {
    let manifest = toml("Cargo.toml");
    let features = table(&manifest, "features");
    let expected = BTreeMap::from([
        ("default", vec!["stdlib"]),
        ("stdlib", vec!["dep:libm"]),
        ("compiler", vec!["dep:rand_core"]),
        (
            "vm",
            vec![
                "compiler",
                "dep:smallvec",
                "dep:rustc-hash",
                "dep:nohash-hasher",
            ],
        ),
        ("cache", vec!["compiler", "dep:dashmap"]),
        ("facet", vec!["dep:facet"]),
        ("fuzz", vec!["stdlib"]),
    ]);
    let got: BTreeMap<&str, Vec<&str>> = features
        .iter()
        .map(|(k, v)| (k.as_str(), strings(v)))
        .collect();
    assert_eq!(got, expected);
}

/// The names of the `pub(crate)` functions and constants of `path` before its tests, sorted.
fn crate_items(path: &str) -> Vec<String> {
    let source = read(path);
    let code = source
        .split("#[cfg(test)]\nmod tests")
        .next()
        .unwrap_or(&source);
    let mut names: Vec<String> = code
        .lines()
        .filter_map(|line| line.strip_prefix("pub(crate) "))
        .filter_map(|rest| {
            rest.strip_prefix("fn ")
                .or_else(|| rest.strip_prefix("const "))
        })
        .filter_map(|rest| {
            rest.split(|c: char| !(c.is_alphanumeric() || c == '_'))
                .next()
        })
        .map(str::to_owned)
        .collect();
    names.sort_unstable();
    names
}

/// The engine's two architecture modules define the same functions and constants.
#[test]
fn the_engine_arch_modules_define_the_same_items() {
    let x86_64 = crate_items("src/numeric/arch/x86_64.rs");
    assert_eq!(x86_64, crate_items("src/numeric/arch/arm64.rs"));
    assert!(
        x86_64.iter().any(|name| name == "sub")
            && x86_64.iter().any(|name| name == "DEFAULT_NAN")
            && x86_64.len() > 15,
        "{x86_64:?}"
    );
}

#[test]
fn facet_is_optional_and_not_re_exported() {
    let manifest = toml("Cargo.toml");
    let facet = table(table(&manifest, "dependencies"), "facet");
    assert_eq!(
        facet.get("optional").and_then(toml::Value::as_bool),
        Some(true),
        "{facet:?}"
    );
    assert_eq!(
        facet.get("version").and_then(toml::Value::as_str),
        Some("0.50.0-rc.7"),
        "{facet:?}"
    );
    let lib = read("src/lib.rs");
    assert!(!lib.lines().any(
        |l| l.trim_start().starts_with("pub use facet") || l.contains("pub extern crate facet")
    ));
}

/// Each dependency and whether it is optional, and each dev-dependency: a new unconditional
/// dependency fails here.
#[test]
fn dependency_lists() {
    let manifest = toml("Cargo.toml");
    let optional = |entry: &toml::Value| {
        entry
            .as_table()
            .and_then(|t| t.get("optional"))
            .and_then(toml::Value::as_bool)
            .unwrap_or(false)
    };
    let dependencies: BTreeMap<&str, bool> = table(&manifest, "dependencies")
        .iter()
        .map(|(name, entry)| (name.as_str(), optional(entry)))
        .collect();
    let expected = BTreeMap::from([
        ("bitflags", false),
        ("dashmap", true),
        ("facet", true),
        ("libm", true),
        ("nohash-hasher", true),
        ("rand_core", true),
        ("rustc-hash", true),
        ("semver", false),
        ("smallvec", true),
        ("thiserror", false),
    ]);
    assert_eq!(dependencies, expected);
    let dev: Vec<&str> = table(&manifest, "dev-dependencies")
        .keys()
        .map(String::as_str)
        .collect();
    assert_eq!(
        dev,
        ["criterion", "facet-json", "proptest", "serde_json", "toml"]
    );
}

/// The directories and files below `base`, relative to the crate root, sorted.
fn tree_below(base: &str) -> (Vec<String>, Vec<String>) {
    fn walk(dir: &Path, dirs: &mut Vec<String>, files: &mut Vec<String>) {
        let mut entries: Vec<PathBuf> = std::fs::read_dir(dir)
            .unwrap_or_else(|e| panic!("{}: {e}", dir.display()))
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .collect();
        entries.sort();
        for path in entries {
            let relative = path
                .strip_prefix(root())
                .unwrap()
                .to_string_lossy()
                .replace('\\', "/");
            if path.is_dir() {
                dirs.push(relative);
                walk(&path, dirs, files);
            } else {
                files.push(relative);
            }
        }
    }
    let (mut dirs, mut files) = (Vec::new(), Vec::new());
    walk(&root().join(base), &mut dirs, &mut files);
    (dirs, files)
}

/// A new module file is added to the list on purpose.
#[test]
fn module_layout() {
    const FILES: [&str; 104] = [
        "src/cache.rs",
        "src/catalog/arity.rs",
        "src/catalog/math.rs",
        "src/catalog/mod.rs",
        "src/catalog/query/decl.rs",
        "src/catalog/query/mod.rs",
        "src/catalog/query/resolve.rs",
        "src/catalog/query/returns_and_reads.rs",
        "src/catalog/query/sets.rs",
        "src/compile/ast.rs",
        "src/compile/codegen/arith.rs",
        "src/compile/codegen/calls.rs",
        "src/compile/codegen/code.rs",
        "src/compile/codegen/control.rs",
        "src/compile/codegen/logic.rs",
        "src/compile/codegen/mod.rs",
        "src/compile/codegen/pools.rs",
        "src/compile/codegen/values.rs",
        "src/compile/cx.rs",
        "src/compile/lex/chars.rs",
        "src/compile/lex/literal.rs",
        "src/compile/lex/mod.rs",
        "src/compile/lex/query.rs",
        "src/compile/lex/string.rs",
        "src/compile/mod.rs",
        "src/compile/options.rs",
        "src/compile/parse/binary.rs",
        "src/compile/parse/calls.rs",
        "src/compile/parse/conditional.rs",
        "src/compile/parse/driver.rs",
        "src/compile/parse/mod.rs",
        "src/compile/parse/sections.rs",
        "src/compile/parse/statements.rs",
        "src/compile/parse/unary.rs",
        "src/compile/program.rs",
        "src/compile/result.rs",
        "src/compile/sema/fold.rs",
        "src/compile/sema/mod.rs",
        "src/compile/sema/numerical.rs",
        "src/compile/sema/shape.rs",
        "src/compile/sema/sum.rs",
        "src/compile/sema/validate.rs",
        "src/bitmask.rs",
        "src/diag.rs",
        "src/hash.rs",
        "src/internals.rs",
        "src/json/facet_proxy.rs",
        "src/json/mod.rs",
        "src/lib.rs",
        "src/numeric/arch/arm64.rs",
        "src/numeric/arch/mod.rs",
        "src/numeric/arch/x86_64.rs",
        "src/numeric/arith.rs",
        "src/numeric/instr.rs",
        "src/numeric/mod.rs",
        "src/numeric/post_op.rs",
        "src/ops/mod.rs",
        "src/ops/op.rs",
        "src/ops/set.rs",
        "src/ops/table.rs",
        "src/reference_catalog.rs",
        "src/rng.rs",
        "src/stdlib/math/arch/arm64/ease.rs",
        "src/stdlib/math/arch/arm64/mod.rs",
        "src/stdlib/math/arch/mod.rs",
        "src/stdlib/math/arch/x86_64/ease.rs",
        "src/stdlib/math/arch/x86_64/mod.rs",
        "src/stdlib/math/arithmetic.rs",
        "src/stdlib/math/ease.rs",
        "src/stdlib/math/fold.rs",
        "src/stdlib/math/interpolation.rs",
        "src/stdlib/math/mod.rs",
        "src/stdlib/math/random/dice.rs",
        "src/stdlib/math/random/mod.rs",
        "src/stdlib/math/rounding.rs",
        "src/stdlib/math/transcendental.rs",
        "src/stdlib/math/trig.rs",
        "src/stdlib/math_fn.rs",
        "src/stdlib/mod.rs",
        "src/stdlib/query_table.rs",
        "src/version/engine.rs",
        "src/version/experiment.rs",
        "src/version/mod.rs",
        "src/vm/cx.rs",
        "src/vm/error.rs",
        "src/vm/eval/handover.rs",
        "src/vm/eval/interp.rs",
        "src/vm/eval/loops.rs",
        "src/vm/eval/machine.rs",
        "src/vm/eval/missing.rs",
        "src/vm/eval/mod.rs",
        "src/vm/eval/query.rs",
        "src/vm/eval/slot.rs",
        "src/vm/global_rng.rs",
        "src/vm/host.rs",
        "src/vm/limits.rs",
        "src/vm/mod.rs",
        "src/vm/name.rs",
        "src/vm/query.rs",
        "src/vm/sink.rs",
        "src/vm/value/members.rs",
        "src/vm/value/mod.rs",
        "src/vm/value/walks.rs",
        "src/vm/vars.rs",
    ];
    assert_layout("src", &FILES);
}

#[test]
fn fuzz_module_layout() {
    const FILES: [&str; 18] = [
        "fuzz/src/generator/ast.rs",
        "fuzz/src/generator/env/compare.rs",
        "fuzz/src/generator/env/differential.rs",
        "fuzz/src/generator/env/mod.rs",
        "fuzz/src/generator/env/table.rs",
        "fuzz/src/generator/front_end.rs",
        "fuzz/src/generator/mod.rs",
        "fuzz/src/generator/pools.rs",
        "fuzz/src/generator/print.rs",
        "fuzz/src/generator/style.rs",
        "fuzz/src/lib.rs",
        "fuzz/src/tree_walker/arith.rs",
        "fuzz/src/tree_walker/flow.rs",
        "fuzz/src/tree_walker/mod.rs",
        "fuzz/src/tree_walker/predicate.rs",
        "fuzz/src/tree_walker/query.rs",
        "fuzz/src/tree_walker/values.rs",
        "fuzz/src/tree_walker/walker.rs",
    ];
    assert_layout("fuzz/src", &FILES);
}

fn assert_layout(base: &str, files: &[&str]) {
    for path in files {
        assert!(
            root().join(path).is_file(),
            "{path}: a module of the layout is missing"
        );
    }
    let known: BTreeSet<&str> = files.iter().copied().collect();
    let (_, found) = tree_below(base);
    for file in found.iter().filter(|file| file.ends_with(".rs")) {
        assert!(
            known.contains(file.as_str()),
            "{file}: not in the module layout of {base}/; a new module file is added to its list on purpose"
        );
    }
}

/// Test-only modules besides `tests` and a `mod.rs`'s `test_support`, inline or in a file of their
/// own: `(file declaring it, module, reason)`.
const TEST_SEAMS: [(&str, &str, &str); 3] = [
    (
        "fuzz/src/generator/env/differential.rs",
        "hooks",
        "production code calls `hooks::tamper` behind `#[cfg(test)]`, so the seam cannot live in `mod tests`",
    ),
    (
        "src/compile/mod.rs",
        "pipeline",
        "nested in `test_support`: the whole-pipeline helpers the unit tests of lex, parse and sema share",
    ),
    (
        "src/lib.rs",
        "reference_catalog",
        "the catalogue of helper queries the reference cases use, shared by the unit tests and, through `internals` (feature `fuzz`), the fuzz crate",
    ),
];

/// Files of `src/` and `fuzz/src/` that may lack a `mod tests`, with the reason.
const MAY_LACK_TESTS: [(&str, &str); 3] = [
    (
        "src/bitmask.rs",
        "a macro and a helper, tested through the operators and the `Debug` output of the bitmask types that use them",
    ),
    (
        "src/lib.rs",
        "the crate docs and the module list; its doc tests pin that no item resolves at the root",
    ),
    (
        "fuzz/src/lib.rs",
        "the module list of the fuzz crate's library",
    ),
];

#[derive(Clone, Copy)]
struct UnitTestRules<'a> {
    may_lack_tests: &'a [(&'a str, &'a str)],
    /// The tests read nothing outside `src/`, the only directory the package ships.
    self_contained: bool,
}

/// A line that is a `#[cfg(…)]` or `#[cfg_attr(…)]` attribute naming `test` (not `not(test)`). An
/// architecture gate `#[cfg(any(<target_arch condition>, test))]` compiles its module on that
/// architecture too, so it is not one.
fn names_cfg_test(line: &str) -> bool {
    let line = line.trim();
    let arch_gate = line.starts_with("#[cfg(any(") && line.contains("target_arch");
    (line.starts_with("#[cfg(") || line.starts_with("#[cfg_attr("))
        && !arch_gate
        && !line.contains("not(test")
        && line
            .split(|c: char| !(c.is_alphanumeric() || c == '_'))
            .any(|word| word == "test")
}

/// `mod name;` or `mod name {` (with a `pub`/`pub(…)` prefix): the name and whether the body is
/// inline.
fn module_declaration(line: &str) -> Option<(&str, bool)> {
    let line = line.trim_start();
    let line = line.strip_prefix("pub").map_or(line, |rest| {
        let rest = rest.trim_start();
        if rest.starts_with('(') {
            rest.split_once(')')
                .map_or(rest, |(_, after)| after.trim_start())
        } else {
            rest
        }
    });
    let rest = line.strip_prefix("mod ")?.trim_start();
    let end = rest.find(|c: char| !(c.is_alphanumeric() || c == '_'))?;
    let (name, after) = rest.split_at(end);
    match after.trim_start().chars().next()? {
        ';' => Some((name, false)),
        '{' => Some((name, true)),
        _ => None,
    }
}

/// Whether `file` is the out-of-line body of a module of [`TEST_SEAMS`], test code throughout.
fn is_seam_file(file: &str) -> bool {
    TEST_SEAMS.iter().any(|(declaring, module, _)| {
        let dir = match declaring.rsplit_once('/') {
            Some((dir, "lib.rs" | "mod.rs")) => format!("{dir}/"),
            _ => format!("{}/", declaring.trim_end_matches(".rs")),
        };
        file == format!("{dir}{module}.rs") || file == format!("{dir}{module}/mod.rs")
    })
}

/// Every break of the unit-test layout in one source file, one message each.
fn unit_test_violations(file: &str, text: &str, rules: UnitTestRules<'_>) -> Vec<String> {
    let mut bad = Vec::new();
    if file.rsplit('/').next() == Some("tests.rs") {
        bad.push(format!(
            "{file}: no `tests.rs`; the tests are inline in the file that holds the code they test"
        ));
    }
    let lines: Vec<&str> = text.lines().collect();
    // Where the test code starts: the first line of a seam file, else the first `#[cfg(test)]` at
    // column 0 (every test-only module is below the code).
    let test_region = if is_seam_file(file) {
        0
    } else {
        lines
            .iter()
            .position(|line| line.starts_with("#[cfg(") && names_cfg_test(line))
            .unwrap_or(lines.len())
    };
    let modules: Vec<usize> = (0..lines.len())
        .filter(|&i| {
            lines[i]
                .trim_start()
                .trim_start_matches("pub ")
                .starts_with("mod tests")
        })
        .collect();
    for i in (0..lines.len()).filter(|&i| !lines[i].trim_start().starts_with("//")) {
        let (line, n) = (lines[i].trim_start(), i + 1);
        bad.extend(module_file_violations(file, &lines, i));
        bad.extend(inclusion_violations(file, line, n));
        if rules.self_contained {
            bad.extend(file_read_violations(file, line, n, i >= test_region));
        }
        if (line.starts_with("#[test]") || line.starts_with("proptest!"))
            && modules.first().is_none_or(|&first| i < first)
        {
            bad.push(format!("{file}:{n}: a test outside `mod tests {{ … }}`; every test is inside the one module at the end of the file"));
        }
        if line.starts_with("#[cfg(") && names_cfg_test(line) {
            bad.extend(test_only_module_violation(file, &lines, i));
        }
    }
    bad.extend(nested_seam_violations(file, &lines));
    bad.extend(tests_module_violations(file, &lines, &modules, rules));
    bad
}

/// Whether `name` is a module of [`TEST_SEAMS`] declared in `file`.
fn is_seam(file: &str, name: &str) -> bool {
    TEST_SEAMS
        .iter()
        .any(|(path, module, _)| *path == file && *module == name)
}

/// The breaks of an out-of-line `mod name;` or an inline `test_support` on line `i`.
fn module_file_violations(file: &str, lines: &[&str], i: usize) -> Vec<String> {
    let (line, n) = (lines[i].trim_start(), i + 1);
    let mut bad = Vec::new();
    match module_declaration(line) {
        Some((name, false)) => {
            let mut start = i;
            while start > 0
                && (lines[start - 1].trim_start().starts_with("#[")
                    || lines[start - 1].trim_start().starts_with("//"))
            {
                start -= 1;
            }
            if !is_seam(file, name)
                && lines[start..i]
                    .iter()
                    .any(|attribute| names_cfg_test(attribute))
            {
                bad.push(format!("{file}:{n}: `#[cfg(test)]` before the out-of-line `mod {name};`; test modules are inline blocks"));
            }
            if name.contains("test") {
                bad.push(format!("{file}:{n}: no out-of-line `mod {name};`; the tests are an inline `mod tests {{ … }}` block"));
            }
        }
        Some(("test_support", true)) if !file.ends_with("/mod.rs") => {
            bad.push(format!("{file}:{n}: `test_support` is the inline helper module of a `mod.rs`, not of this file"));
        }
        _ => {}
    }
    bad
}

/// The breaks of a `path` attribute or an `include!` on line `n`.
fn inclusion_violations(file: &str, line: &str, n: usize) -> Vec<String> {
    let mut bad = Vec::new();
    if line.starts_with("#[")
        && (line.contains("path =") || line.contains("path=") || line.starts_with("#[path"))
    {
        bad.push(format!(
            "{file}:{n}: no `path` attribute (also none inside `cfg_attr`); modules are found by their names (`name/mod.rs`)"
        ));
    }
    if line.contains("include!(") || line.contains("include! (") {
        bad.push(format!(
            "{file}:{n}: no `include!`; unit tests are inline and self-contained"
        ));
    }
    bad
}

/// The reads outside `src/` on line `n`, and in test code any file read.
fn file_read_violations(file: &str, line: &str, n: usize, in_test_code: bool) -> Vec<String> {
    let mut bad = Vec::new();
    // The README ships in the package, so its doctest is the one allowed read outside `src/`.
    let readme_doctest =
        file == "src/lib.rs" && line.trim() == "#[doc = include_str!(\"../README.md\")]";
    let outside = [
        "tests/",
        "CARGO_MANIFEST_DIR",
        "include_str!",
        "include_bytes!",
    ];
    for word in outside
        .into_iter()
        .filter(|word| !readme_doctest && line.contains(word))
    {
        bad.push(format!(
            "{file}:{n}: `{word}`: unit tests are self-contained and read nothing outside `src/` (the published crate ships only `src/`)"
        ));
    }
    let reads = [
        "std::fs",
        "fs::read",
        "fs::File",
        "File::open",
        "read_to_string",
        "read_dir",
        "\"../",
    ];
    for word in reads
        .into_iter()
        .filter(|word| in_test_code && line.contains(word))
    {
        bad.push(format!(
            "{file}:{n}: `{word}` in test code: unit tests read nothing outside `src/`; expected values are written in the test"
        ));
    }
    bad
}

/// The break of the test-only module the `#[cfg(test)]` on line `i` gates, if it is one that may
/// not be.
fn test_only_module_violation(file: &str, lines: &[&str], i: usize) -> Option<String> {
    let line = lines[i].trim_start();
    let j = (i + 1..lines.len()).find(|&j| {
        !lines[j].trim_start().starts_with("#[") && !lines[j].trim_start().starts_with("//")
    })?;
    let (name, _) = module_declaration(lines[j])?;
    let indent = lines[j].len() - lines[j].trim_start().len();
    let allowed = (name == "tests" && indent == 0 && line == "#[cfg(test)]")
        || (name == "test_support"
            && indent == 0
            && file.ends_with("/mod.rs")
            && lines[j].starts_with("pub(crate) mod test_support {"))
        || is_seam(file, name);
    (!allowed).then(|| {
        format!(
            "{file}:{}: a test-only `mod {name}`; only `mod tests`, a `mod.rs`'s inline `pub(crate) mod test_support` and the modules of TEST_SEAMS may be",
            i + 1
        )
    })
}

/// Modules nested in `test_support`, which carry no attribute of their own, that are not listed.
fn nested_seam_violations(file: &str, lines: &[&str]) -> Vec<String> {
    let Some(start) = lines
        .iter()
        .position(|line| line.starts_with("pub(crate) mod test_support {"))
    else {
        return Vec::new();
    };
    let end = (start + 1..lines.len())
        .find(|&j| lines[j] == "}")
        .unwrap_or(lines.len());
    let mut bad = Vec::new();
    for (j, line) in lines.iter().enumerate().take(end).skip(start + 1) {
        if let Some((name, _)) = module_declaration(line)
            && !is_seam(file, name)
        {
            bad.push(format!(
                "{file}:{}: `mod {name}` nested in `test_support` is not in TEST_SEAMS",
                j + 1
            ));
        }
    }
    bad
}

/// The breaks of the one `mod tests` at the end of the file; `modules` are the lines naming it.
fn tests_module_violations(
    file: &str,
    lines: &[&str],
    modules: &[usize],
    rules: UnitTestRules<'_>,
) -> Vec<String> {
    // A line that opens an item at column 0 (attributes and comments do not).
    let opens_item = |line: &str| {
        [
            "mod ",
            "pub",
            "fn ",
            "impl",
            "struct ",
            "enum ",
            "const ",
            "static ",
            "type ",
            "trait ",
            "use ",
            "unsafe ",
            "extern ",
            "macro_rules!",
        ]
        .iter()
        .any(|start| line.starts_with(start))
    };
    let mut bad = Vec::new();
    if modules.is_empty() {
        if !rules.may_lack_tests.iter().any(|(path, _)| *path == file) {
            bad.push(format!("{file}: every source file ends with a `#[cfg(test)] mod tests {{ … }}`; only the listed files may lack one"));
        }
        return bad;
    }
    if modules.len() != 1 || lines[modules[0]] != "mod tests {" {
        bad.push(format!("{file}: exactly one top-level `mod tests {{` per file (found {} lines naming `mod tests`)", modules.len()));
        return bad;
    }
    if modules[0] == 0 || lines[modules[0] - 1] != "#[cfg(test)]" {
        bad.push(format!(
            "{file}: `mod tests {{` is preceded by `#[cfg(test)]`"
        ));
    }
    if lines.iter().rev().find(|line| opens_item(line)) != Some(&"mod tests {") {
        bad.push(format!(
            "{file}: `mod tests {{ … }}` is the last item of the file"
        ));
    }
    if lines.iter().rev().find(|line| !line.trim().is_empty()) != Some(&"}") {
        bad.push(format!(
            "{file}: the file ends with the closing brace of `mod tests`"
        ));
    }
    bad
}

#[test]
fn unit_tests_are_inline() {
    // The fuzz crate is not published, so its tests may read files.
    let trees: [(&str, UnitTestRules<'_>); 2] = [
        (
            "src",
            UnitTestRules {
                may_lack_tests: &MAY_LACK_TESTS,
                self_contained: true,
            },
        ),
        (
            "fuzz/src",
            UnitTestRules {
                may_lack_tests: &MAY_LACK_TESTS,
                self_contained: false,
            },
        ),
    ];
    for (base, rules) in trees {
        let (dirs, files) = tree_below(base);
        for dir in &dirs {
            assert!(
                dir.rsplit('/').next() != Some("tests"),
                "{dir}: a `tests/` directory under {base}/ holds test-only files; unit tests are inline `mod tests {{ … }}` blocks"
            );
            assert!(
                root().join(dir).join("mod.rs").is_file(),
                "{dir}: a module directory has a `mod.rs` (a module with child files is `name/mod.rs`)"
            );
            assert!(
                !files.contains(&format!("{dir}.rs")),
                "{dir}.rs: `name.rs` beside `name/` is not used; the module is `name/mod.rs`"
            );
        }
        let mut violations = Vec::new();
        for file in files.iter().filter(|file| file.ends_with(".rs")) {
            violations.extend(unit_test_violations(file, &read(file), rules));
        }
        assert!(
            violations.is_empty(),
            "the unit-test layout is broken:\n{}",
            violations.join("\n")
        );
    }
    for (file, module, reason) in TEST_SEAMS {
        assert!(
            !reason.is_empty()
                && [format!("mod {module} {{"), format!("mod {module};")]
                    .iter()
                    .any(|declaration| read(file).contains(declaration.as_str())),
            "{file}: the seam `{module}` of TEST_SEAMS is gone or has no reason"
        );
    }
}

#[test]
fn the_unit_test_layout_check_notices_what_it_forbids() {
    let rules = UnitTestRules {
        may_lack_tests: &[],
        self_contained: true,
    };
    let tail = "\n#[cfg(test)]\nmod tests {\n    #[test]\n    fn a() {}\n}\n";
    let clean = format!("//! x\nfn f() {{}}\n{tail}");
    assert_eq!(
        unit_test_violations("src/a.rs", &clean, rules),
        Vec::<String>::new()
    );
    let arch_gate = format!(
        "//! x\n#[cfg(any(target_arch = \"aarch64\", test))]\npub(crate) mod arm64;\nfn f() {{}}\n{tail}"
    );
    assert_eq!(
        unit_test_violations("src/a.rs", &arch_gate, rules),
        Vec::<String>::new()
    );
    for (text, file, rule) in [
        (
            format!("//! x\n#[cfg(test)]\nmod helpers;\n{tail}"),
            "src/a.rs",
            "`#[cfg(test)]` before the out-of-line `mod helpers;`",
        ),
        (
            format!("//! x\n#[cfg(all(test, feature = \"vm\"))]\npub(crate) mod helpers;\n{tail}"),
            "src/a.rs",
            "`#[cfg(test)]` before the out-of-line",
        ),
        (
            format!("//! x\nmod unit_tests;\n{tail}"),
            "src/a.rs",
            "no out-of-line `mod unit_tests;`",
        ),
        (
            format!("//! x\nmod tests;\n{tail}"),
            "src/a.rs",
            "no out-of-line `mod tests;`",
        ),
        (
            format!("//! x\n#[cfg_attr(test, path = \"x.rs\")]\nmod x;\n{tail}"),
            "src/a.rs",
            "no `path` attribute",
        ),
        (
            format!("//! x\n#[path = \"x.rs\"]\nmod x;\n{tail}"),
            "src/a.rs",
            "no `path` attribute",
        ),
        (
            format!("//! x\ninclude!(\"x.rs\");\n{tail}"),
            "src/a.rs",
            "no `include!`",
        ),
        (
            "//! x\n#[cfg(test)]\nmod tests {\n    fn f() { let _ = std::fs::read(\"x\"); }\n}\n"
                .to_owned(),
            "src/a.rs",
            "`std::fs` in test code",
        ),
        (
            "//! x\n#[cfg(test)]\nmod tests {\n    fn f() { let _ = File::open(\"x\"); }\n}\n"
                .to_owned(),
            "src/a.rs",
            "`File::open` in test code",
        ),
        (
            format!("//! x\nfn f() {{ let _ = std::fs::read(\"x\"); }}\n{tail}"),
            "src/reference_catalog.rs",
            "`std::fs` in test code",
        ),
        (
            format!("//! x\n#[test]\nfn stray() {{}}\n{tail}"),
            "src/a.rs",
            "a test outside `mod tests",
        ),
        (
            format!("//! x\n#[cfg(test)]\nmod scratch {{\n}}\n{tail}"),
            "src/a.rs",
            "a test-only `mod scratch`",
        ),
        (
            format!("//! x\n#[cfg(test)]\npub(crate) mod test_support {{\n}}\n{tail}"),
            "src/a.rs",
            "`test_support` is the inline helper module of a `mod.rs`",
        ),
        (
            format!(
                "//! x\n#[cfg(test)]\npub(crate) mod test_support {{\n    pub(crate) mod extra {{\n    }}\n}}\n{tail}"
            ),
            "src/m/mod.rs",
            "nested in `test_support` is not in TEST_SEAMS",
        ),
        (
            "//! x\nfn f() {}\n".to_owned(),
            "src/a.rs",
            "every source file ends with a `#[cfg(test)] mod tests",
        ),
        (
            format!("//! x\n{tail}fn after() {{}}\n"),
            "src/a.rs",
            "is the last item of the file",
        ),
    ] {
        let found = unit_test_violations(file, &text, rules);
        assert!(
            found
                .iter()
                .any(|message| message.starts_with(file) && message.contains(rule)),
            "{rule}: not noticed in {text:?}: {found:?}"
        );
    }
    let reading =
        "//! x\n#[cfg(test)]\nmod tests {\n    fn f() { let _ = std::fs::read(\"x\"); }\n}\n";
    assert!(
        unit_test_violations(
            "tools/a.rs",
            reading,
            UnitTestRules {
                self_contained: false,
                ..rules
            }
        )
        .is_empty()
    );
}

#[test]
fn unsafe_is_forbidden() {
    let lib = read("src/lib.rs");
    assert!(lib.lines().any(|l| l.trim() == "#![forbid(unsafe_code)]"));
    assert!(!lib.contains("unchecked-stack"));
}

#[test]
fn lint_configuration() {
    let lib = read("src/lib.rs");
    assert!(
        lib.lines()
            .any(|l| l.trim() == "#![warn(clippy::pedantic)]")
    );
    let lints: Vec<String> = allowed_lints(&lib)
        .into_iter()
        .map(|(lint, _)| lint)
        .collect();
    assert_eq!(lints, ALLOWED);
}

/// The four `cast_` lints share one reason.
#[test]
fn every_allowed_lint_has_a_reason() {
    for (lint, before) in allowed_lints(&read("src/lib.rs")) {
        let shares_the_cast_reason =
            lint.starts_with("cast_") && before.starts_with("#![allow(clippy::cast_");
        assert!(
            before.starts_with("//") || shares_the_cast_reason,
            "{lint}: no reason above it"
        );
    }
}

#[test]
fn the_built_in_tables_exist() {
    let table = read("src/stdlib/query_table.rs");
    let rows = table
        .lines()
        .filter(|line| {
            line.trim_start()
                .split_once(" = \"")
                .is_some_and(|(name, _)| {
                    name.chars()
                        .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_')
                })
        })
        .count();
    assert_eq!(
        rows,
        molangx::stdlib::queries(Side::Client).len(),
        "one line per standard query"
    );
    let body = table
        .split_once("\nqueries! {\n")
        .and_then(|(_, rest)| rest.split_once("\n}\n"))
        .map(|(body, _)| body)
        .expect("the queries! block");
    assert!(body.lines().count() < 2 * rows, "a compact table");
    for table in [
        "src/stdlib/query_table.rs",
        "src/stdlib/math_fn.rs",
        "src/ops/table.rs",
    ] {
        assert!(root().join(table).is_file(), "{table}");
    }
}

#[test]
fn benchmarks_exist() {
    let benches = targets("bench");
    for (name, feature) in [("eval", "vm, stdlib"), ("compile", "compiler, stdlib")] {
        let entries = benches
            .get(name)
            .unwrap_or_else(|| panic!("no [[bench]] `{name}`"));
        assert_eq!(
            entries.get("harness").and_then(toml::Value::as_bool),
            Some(false),
            "{name}: criterion needs harness = false"
        );
        assert_eq!(
            entries
                .get("required-features")
                .map(|f| strings(f).join(", "))
                .as_deref(),
            Some(feature),
            "{name}"
        );
        assert!(
            root().join("benches").join(format!("{name}.rs")).is_file(),
            "benches/{name}.rs"
        );
    }
    let eval = read("benches/eval.rs");
    for group in [
        "\"budget\"",
        "\"shapes\"",
        "\"tick\"",
        "\"constant\"",
        "\"clamp_query\"",
        "\"twenty_statements_temps\"",
    ] {
        assert!(eval.contains(group), "benches/eval.rs has no {group}");
    }
    let compile = read("benches/compile.rs");
    assert!(
        compile.contains("MOLANG_CORPUS_DIR") && compile.contains("corpus_pairs.json"),
        "the corpus is read at run time"
    );
    let sample = read("benches/compile_sample.txt");
    assert!(
        sample.lines().filter(|l| !l.is_empty()).count() >= 200,
        "the committed compile sample"
    );
}

#[test]
fn examples_and_their_readme_rows() {
    let declared = example_features();
    let names = [
        "hello_world",
        "versions",
        "json_forms",
        "diagnostics",
        "restricted",
        "host",
        "variables",
        "math",
        "cache",
    ];
    let mut expected: Vec<&str> = names.to_vec();
    expected.sort_unstable();
    assert_eq!(
        declared.keys().map(String::as_str).collect::<Vec<_>>(),
        expected
    );
    let readme = read("README.md");
    for name in names {
        assert!(
            root().join(format!("examples/{name}.rs")).is_file(),
            "{name}"
        );
        let row = readme
            .lines()
            .find(|l| l.starts_with(&format!("| [`{name}`]")))
            .unwrap_or_else(|| panic!("README has no row for {name}"));
        let features = row
            .trim_end_matches('|')
            .rsplit('|')
            .next()
            .expect("a features cell")
            .trim();
        let want = if declared[name].is_empty() {
            "none".to_owned()
        } else {
            format!("`{}`", declared[name])
        };
        assert_eq!(features, want, "{name}");
    }
}

#[test]
fn examples_run_in_ci() {
    assert!(read(".github/workflows/ci.yml").contains("--example"));
}

#[test]
fn repository_files_and_ci() {
    for path in [
        "README.md",
        "CONTRIBUTING.md",
        "LICENSE",
        ".github/dependabot.yml",
    ] {
        assert!(root().join(path).is_file(), "{path}");
    }
    let license = read("LICENSE");
    assert!(license.contains("Apache License") && license.contains("Version 2.0"));
    for path in [
        "rustfmt.toml",
        ".rustfmt.toml",
        "fuzz/rustfmt.toml",
        "fuzz/.rustfmt.toml",
    ] {
        assert!(
            !root().join(path).exists(),
            "{path}: the code has the default rustfmt style"
        );
    }
    let ci = read(".github/workflows/ci.yml");
    for step in [
        "cargo check --locked --no-default-features",
        "cargo check --locked --all-features",
        "cargo test --locked --no-default-features --features stdlib",
        "cargo test --locked --all-features",
        "cargo test --locked --no-default-features --features compiler --test engine_only",
        "cargo test --locked --no-default-features --features vm --test engine_only",
        "cargo test --locked --doc --no-default-features --features vm",
        "cargo fmt --all -- --check",
        "cargo clippy --locked --no-default-features",
        "cargo clippy --locked --all-targets --all-features",
        "cargo publish --dry-run --locked",
        "cargo test --doc --all-features",
        "cargo check --locked --manifest-path fuzz/Cargo.toml --all-targets",
        "cargo test --locked --manifest-path fuzz/Cargo.toml",
        "cargo clippy --locked --manifest-path fuzz/Cargo.toml --all-targets -- -D warnings",
        "cargo fmt --manifest-path fuzz/Cargo.toml --all -- --check",
        "cargo doc --locked --no-deps --all-features",
        "cargo doc --locked --no-deps --no-default-features",
        "cargo doc --locked --no-deps --no-default-features --features vm",
        "cargo clippy --locked --no-default-features --features compiler,vm -- -D warnings",
        "--cfg docsrs",
    ] {
        assert!(ci.contains(step), "ci.yml: {step}");
    }
    // Clippy runs with each feature alone: a `cfg` that leaves an item unused in one of those
    // builds fails it.
    let manifest = toml("Cargo.toml");
    let features = table(&manifest, "features");
    let mut single: Vec<&str> = features
        .keys()
        .map(String::as_str)
        .filter(|f| *f != "default")
        .collect();
    single.sort_unstable();
    assert_eq!(
        single,
        ["cache", "compiler", "facet", "fuzz", "stdlib", "vm"],
        "the features clippy runs alone"
    );
    for feature in single {
        let step = format!(
            "cargo clippy --locked --no-default-features --features {feature} -- -D warnings"
        );
        assert!(ci.contains(&step), "ci.yml: {step}");
    }
    let rust_version = table(&manifest, "package")["rust-version"]
        .as_str()
        .expect("a rust-version");
    assert!(
        ci.contains(&format!("MSRV: \"{rust_version}.0\"")),
        "ci.yml: MSRV is not rust-version {rust_version}"
    );
    assert_eq!(
        ci.matches("toolchain: \"").count(),
        0,
        "ci.yml: a toolchain pin outside the env block"
    );
    assert!(
        ci.contains("ubuntu-24.04-arm"),
        "ci.yml: no Linux arm64 test job"
    );
    // The arm64 behaviour runs only on an aarch64 build: its job lints, tests per feature, builds
    // the docs and tests the fuzz crate.
    let arm64_job = ci
        .split("  test-arm64:")
        .nth(1)
        .and_then(|rest| rest.split("\n  fmt:").next())
        .expect("the arm64 job");
    assert!(
        arm64_job.contains("runs-on: ubuntu-24.04-arm"),
        "ci.yml: the arm64 job runs on arm64"
    );
    for step in [
        "cargo clippy --locked --all-targets --all-features -- -D warnings",
        "cargo clippy --locked --no-default-features -- -D warnings",
        "cargo test --locked --all-features",
        "cargo test --locked --no-default-features --features stdlib\n",
        "cargo test --locked --no-default-features --features stdlib,compiler",
        "cargo test --locked --no-default-features --features stdlib,vm",
        "cargo test --locked --no-default-features --features stdlib,cache",
        "cargo test --locked --no-default-features --features stdlib,facet",
        "cargo test --locked --no-default-features --features fuzz",
        "cargo test --locked --no-default-features --features compiler --test engine_only",
        "cargo test --locked --no-default-features --features vm --test engine_only",
        "cargo doc --locked --no-deps --all-features",
        "cargo clippy --locked --manifest-path fuzz/Cargo.toml --all-targets -- -D warnings",
        "cargo test --locked --manifest-path fuzz/Cargo.toml",
    ] {
        assert!(arm64_job.contains(step), "ci.yml, arm64 job: {step}");
    }
    assert!(
        !ci.contains("windows-") && !ci.contains("macos-"),
        "ci.yml: a job that does not run on Linux"
    );
    let dependabot = read(".github/dependabot.yml");
    assert!(
        dependabot.contains("package-ecosystem: \"cargo\"")
            && dependabot.contains("interval: \"weekly\"")
    );
}

#[test]
fn publication_metadata() {
    let manifest = toml("Cargo.toml");
    let package = table(&manifest, "package");
    assert_eq!(package["rust-version"].as_str(), Some("1.92"));
    assert_eq!(
        package["documentation"].as_str(),
        Some("https://docs.rs/molangx")
    );
    let docs_rs = table(table(table(package, "metadata"), "docs"), "rs");
    assert_eq!(strings(&docs_rs["rustdoc-args"]), ["--cfg", "docsrs"]);
    for text in [read("README.md"), read("src/lib.rs")] {
        assert!(
            text.contains("SemVer") && text.contains("guarantee"),
            "the README and the crate docs label `fuzz`"
        );
    }
    let readme = read("README.md");
    assert!(
        readme.contains("molangx = { version = \"0.1\""),
        "the README installs from crates.io"
    );
    assert!(
        !readme.contains("git = \""),
        "the README must not tell users to depend on a git URL"
    );
    assert!(
        read("src/lib.rs").contains("include_str!(\"../README.md\")"),
        "the README is compiled as a doctest"
    );
}

/// Each `ServerRun::new("run_NN", FIRST_RELEASE | BOTH_RELEASES, …)` of `tests/measured_*.rs` and
/// each run of `tests/server_sweeps.json` (on the file's `releases`), with its releases.
fn server_runs() -> BTreeMap<String, Vec<String>> {
    let release_list = |name: &str| -> Vec<String> {
        match name {
            "FIRST_RELEASE" => vec!["1.26.36.1".to_owned()],
            "BOTH_RELEASES" => vec!["1.26.36.1".to_owned(), "1.26.52.3".to_owned()],
            other => panic!("ServerRun::new takes FIRST_RELEASE or BOTH_RELEASES, not {other}"),
        }
    };
    let mut runs = BTreeMap::new();
    for file in topic_files() {
        let text = read(&format!("tests/{file}"));
        for (at, _) in text.match_indices("ServerRun::new(") {
            let rest = text[at + "ServerRun::new(".len()..].trim_start();
            let Some(rest) = rest.strip_prefix('"') else {
                continue;
            };
            let (run, rest) = rest.split_once('"').expect("a quoted run name");
            let releases = rest
                .trim_start()
                .strip_prefix(',')
                .expect("a release list")
                .trim_start();
            let name: String = releases
                .chars()
                .take_while(|c| c.is_ascii_uppercase() || *c == '_')
                .collect();
            assert!(
                runs.insert(run.to_owned(), release_list(&name)).is_none(),
                "{run} is built twice"
            );
        }
    }
    let sweeps = json("tests/server_sweeps.json");
    let releases: Vec<String> = sweeps["releases"]
        .as_array()
        .expect("releases")
        .iter()
        .map(|r| r.as_str().expect("release").to_owned())
        .collect();
    for run in sweeps["runs"].as_array().expect("runs") {
        let run = format!("run_{:02}", run["run"].as_u64().expect("run number"));
        assert!(
            runs.insert(run.clone(), releases.clone()).is_none(),
            "{run} is built twice"
        );
    }
    runs
}

/// The quoted text right after `call` at `at` in `text` (`call("text"`), or `None`.
fn quoted_after<'a>(text: &'a str, at: usize, call: &str) -> Option<&'a str> {
    let rest = text[at + call.len()..].trim_start().strip_prefix('"')?;
    rest.split_once('"').map(|(quoted, _)| quoted)
}

/// Case ids are unique across the topic files, probe ids within their run, row numbers within their
/// group.
#[test]
fn measured_row_ids_are_unique() {
    let files = topic_files();
    let mut cases: BTreeMap<String, String> = BTreeMap::new();
    let mut probes: BTreeMap<(String, String), String> = BTreeMap::new();
    let mut rows: BTreeMap<(String, String), String> = BTreeMap::new();
    let mut repeats = Vec::new();
    for file in &files {
        let text = read(&format!("tests/{file}"));
        // Every builder call of the file, in order: the run or group it opens, or the id of the row
        // it adds.
        let mut calls: Vec<(usize, &str)> = Vec::new();
        for call in [
            "fn run_",
            "RunGroup::new(",
            "ParseGroup::new(",
            "LoopCapGroup::new(",
            "SmokeGroup::new(",
            "EvalCase::new(",
            "VersionWindowCase::new(",
            "AllowListCase::new(",
            "run.probe(",
            ".row(",
        ] {
            calls.extend(text.match_indices(call));
        }
        calls.sort_unstable();
        let (mut run, mut group) = (String::new(), String::new());
        for (start, call) in calls {
            let at = format!("{file}:{}", text[..start].lines().count());
            let rest = &text[start + call.len()..];
            match call {
                "fn run_" => run = format!("run_{}", rest.split_once('(').expect("fn run_NN()").0),
                "LoopCapGroup::new(" => group = "loop_cap".to_owned(),
                "SmokeGroup::new(" => group = "smoke".to_owned(),
                "RunGroup::new(" | "ParseGroup::new(" => {
                    group = quoted_after(&text, start, call)
                        .expect("a quoted group")
                        .to_owned()
                }
                "run.probe(" => {
                    let id = quoted_after(&text, start, call).expect("a quoted probe id");
                    if let Some(first) = probes.insert((run.clone(), id.to_owned()), at.clone()) {
                        repeats.push(format!("probe {run} {id} at {at} and {first}"));
                    }
                }
                ".row(" => {
                    let number: String = rest
                        .trim_start()
                        .chars()
                        .take_while(char::is_ascii_digit)
                        .collect();
                    if let Some(first) = rows.insert((group.clone(), number.clone()), at.clone()) {
                        repeats.push(format!("row {group} #{number} at {at} and {first}"));
                    }
                }
                _ => {
                    let id = quoted_after(&text, start, call).expect("a quoted case id");
                    if let Some(first) = cases.insert(id.to_owned(), at.clone()) {
                        repeats.push(format!("case {id} at {at} and {first}"));
                    }
                }
            }
        }
    }
    assert!(
        repeats.is_empty(),
        "measured row ids repeat:\n{}",
        repeats.join("\n")
    );
    // A broken scan finds nothing and would pass vacuously.
    assert!(
        cases.len() > 200 && probes.len() > 500 && rows.len() > 200,
        "{} cases, {} probes, {} group rows",
        cases.len(),
        probes.len(),
        rows.len()
    );
}

/// Reading a run's answers (`ServerRun::read_without_replay`) does not count as replaying it.
#[test]
fn every_server_run_is_replayed() {
    let files = topic_files();
    let (mut built, mut unreplayed) = (0, Vec::new());
    for file in &files {
        let text = read(&format!("tests/{file}"));
        for (at, _) in text.match_indices("fn run_") {
            // `fn run_NN()`, not a test whose name starts with `run_`.
            let number: String = text[at + "fn run_".len()..]
                .chars()
                .take_while(char::is_ascii_digit)
                .collect();
            if number.is_empty() {
                continue;
            }
            built += 1;
            if !text.contains(&format!("run_{number}().replay(")) {
                unreplayed.push(format!("tests/{file}: run_{number}"));
            }
        }
    }
    assert!(
        unreplayed.is_empty(),
        "server runs built but never replayed in their file:\n{}",
        unreplayed.join("\n")
    );
    // A broken scan finds nothing and would pass vacuously.
    assert!(built > 40, "{built} server runs built in the topic files");
}

/// The expected logs derive from `server_runs`: a new run needs no change here.
#[test]
fn test_data_files() {
    let tests = root().join("tests");
    let data: Vec<String> = file_names("tests")
        .into_iter()
        .filter(|name| name.ends_with(".json"))
        .collect();
    assert_eq!(
        data,
        [
            "numeric_bits.json",
            "parse_vectors.json",
            "server_sweeps.json"
        ],
        "the data files of tests/"
    );
    for name in &data {
        let value = json(&format!("tests/{name}"));
        assert!(
            value["schema"]
                .as_str()
                .is_some_and(|schema| schema.starts_with("molangx/")),
            "tests/{name}: a `molangx/` schema"
        );
    }
    assert!(
        !tests.join("corpus_pairs.json").exists(),
        "the pack corpus is Mojang content"
    );
    let logs = file_names("tests/server_logs");
    let runs = server_runs();
    let numbers: Vec<u32> = runs
        .keys()
        .map(|run| run["run_".len()..].parse().expect("run_NN"))
        .collect();
    assert_eq!(
        numbers,
        (1..=u32::try_from(numbers.len()).expect("run count")).collect::<Vec<_>>(),
        "the runs are numbered from 1 without a gap"
    );
    let mut expected: Vec<String> = runs
        .iter()
        .flat_map(|(run, releases)| {
            releases
                .iter()
                .map(move |release| format!("{run}_{release}.log"))
        })
        .collect();
    expected.sort();
    assert_eq!(
        logs, expected,
        "tests/server_logs holds one console log per release of each run"
    );
}

#[test]
fn integration_test_files() {
    let testing = read("CONTRIBUTING.md");
    let tests = root().join("tests");
    assert!(tests.join("common").is_dir());
    assert!(tests.join("common/mod.rs").is_file());
    assert!(!tests.join("fixtures").exists());
    let crates = [
        "compile.rs",
        "diagnostics.rs",
        "evaluation.rs",
        "numerics.rs",
        "limits.rs",
        "host.rs",
        "host_math.rs",
        "cache.rs",
        "serialised_forms.rs",
        "public_api.rs",
        "properties.rs",
        "measured_variables.rs",
        "measured_arithmetic.rs",
        "measured_math.rs",
        "measured_rounding_and_random.rs",
        "measured_logic.rs",
        "measured_control_flow.rs",
        "measured_queries.rs",
        "measured_parsing.rs",
        "measured_declarations.rs",
        "data_parse.rs",
        "data_bits.rs",
        "data_sweeps.rs",
        "operator_rules.rs",
        "corpus.rs",
        "repo_checks.rs",
        "alloc_free.rs",
        "compile_memory.rs",
        "engine_only.rs",
    ];
    for file in crates {
        assert!(tests.join(file).is_file(), "tests/{file}");
        assert!(
            testing.contains(file),
            "CONTRIBUTING.md does not name {file}"
        );
    }
    let found: Vec<String> = file_names("tests")
        .into_iter()
        .filter(|name| name.ends_with(".rs"))
        .collect();
    let mut expected: Vec<String> = crates.iter().map(|name| (*name).to_owned()).collect();
    expected.sort();
    assert_eq!(
        found, expected,
        "tests/ holds exactly the crates the guide names"
    );
}

/// The guide runs the arm64 suite under emulation: the runner, the timing override the test reads,
/// and the fuzz crate's stub archive.
#[test]
fn the_guide_runs_the_arm64_suite_under_emulation() {
    let guide = read("CONTRIBUTING.md");
    for text in [
        "CARGO_TARGET_AARCH64_UNKNOWN_LINUX_GNU_RUNNER=\"qemu-aarch64 -L /usr/aarch64-linux-gnu\"",
        "CC_aarch64_unknown_linux_gnu=aarch64-linux-gnu-gcc",
        "export MOLANGX_TIME_SCALE=",
        "cargo test --locked --target aarch64-unknown-linux-gnu --all-features",
        "aarch64-linux-gnu-ar rcs target/no-libfuzzer/libfuzzer.a",
        "CUSTOM_LIBFUZZER_STD_CXX=none",
        "MOLANGX_HOSTILE_STACK_KIB",
    ] {
        assert!(guide.contains(text), "CONTRIBUTING.md: {text}");
    }
    assert!(
        !guide.contains("only through the unit tests"),
        "CONTRIBUTING.md: the whole suite runs under emulation"
    );
    let limits = read("tests/limits.rs");
    for variable in ["MOLANGX_TIME_SCALE", "MOLANGX_HOSTILE_STACK_KIB"] {
        assert!(
            limits.contains(&format!("std::env::var(\"{variable}\")")),
            "tests/limits.rs reads {variable}"
        );
    }
}

#[test]
fn fuzz_test_files() {
    let testing = read("CONTRIBUTING.md");
    let tests = root().join("fuzz/tests");
    assert!(tests.join("common/mod.rs").is_file());
    let crates = ["properties.rs", "recorded.rs", "tree_walker.rs"];
    for file in crates {
        assert!(
            testing.contains(&format!("fuzz/tests/{file}")),
            "CONTRIBUTING.md does not name fuzz/tests/{file}"
        );
    }
    assert_eq!(
        file_names("fuzz/tests"),
        ["common", "properties.rs", "recorded.rs", "tree_walker.rs"],
        "fuzz/tests/ holds exactly the crates the guide names"
    );
}

/// Version windows include their last version; every query not named here is `[0, 13]`.
#[test]
fn version_ranged_implementations() {
    use MolangVersion as V;
    let ranges = |name: &str| {
        get(name)
            .shape()
            .ranges
            .as_slice()
            .iter()
            .map(|r| (r.first().as_i16(), r.last().as_i16()))
            .collect::<Vec<_>>()
    };
    let two = [
        ("query.item_remaining_use_duration", (0, 1), (2, 13)),
        ("query.cape_flap_amount", (0, 7), (8, 13)),
        ("query.surface_particle_color", (0, 11), (12, 13)),
        (
            "query.surface_particle_texture_coordinate",
            (0, 11),
            (12, 13),
        ),
        ("query.surface_particle_texture_size", (0, 11), (12, 13)),
        ("query.is_carrying_block", (0, 12), (13, 13)),
    ];
    for (name, old, new) in two {
        assert_eq!(ranges(name), vec![old, new], "{name}");
        for v in versions() {
            let raw = v.as_i16();
            let expected = if raw < 0 {
                None
            } else if raw <= old.1 {
                Some(0)
            } else {
                Some(1)
            };
            assert_eq!(impl_at(name, v), expected, "{name} at {v:?}");
        }
    }
    assert_eq!(impl_at("query.item_remaining_use_duration", V::V1), Some(0));
    assert_eq!(impl_at("query.item_remaining_use_duration", V::V2), Some(1));
    assert_eq!(impl_at("query.cape_flap_amount", V::V8), Some(1));
    assert_eq!(impl_at("query.is_carrying_block", V::V12), Some(0));
    assert_eq!(impl_at("query.is_carrying_block", V::V13), Some(1));

    for name in ["query.block_property", "query.has_block_property"] {
        assert_eq!(ranges(name), vec![(0, 9)], "{name}");
        assert_eq!(impl_at(name, V::V9), Some(0));
        assert_eq!(impl_at(name, V::V10), None);
    }
    for name in ["query.block_state", "query.has_block_state"] {
        assert_eq!(ranges(name), vec![(0, 13)], "{name}");
    }
    for name in [
        "query.dash_cooldown_progress",
        "query.is_scenting",
        "query.is_rising",
        "query.is_feeling_happy",
    ] {
        assert_eq!(ranges(name), vec![(0, 10)], "{name}");
        assert_eq!(impl_at(name, V::V11), None);
    }

    let special: BTreeSet<&str> = two
        .iter()
        .map(|(n, _, _)| *n)
        .chain([
            "query.block_property",
            "query.has_block_property",
            "query.dash_cooldown_progress",
            "query.is_scenting",
            "query.is_rising",
            "query.is_feeling_happy",
        ])
        .collect();
    for decl in baseline().filter(|d| !special.contains(d.name())) {
        assert_eq!(decl.shape().ranges.as_slice().len(), 1, "{}", decl.name());
        assert_eq!(
            (
                decl.shape().ranges.as_slice()[0].first(),
                decl.shape().ranges.as_slice()[0].last()
            ),
            (V::V0, V::V13),
            "{}",
            decl.name()
        );
    }
}

#[test]
fn one_built_in_catalogue() {
    let baseline_release = Version::new(1, 26, 0);
    let later: Vec<(&str, &Version)> = molangx::stdlib::queries(Side::Client)
        .iter()
        .filter_map(|d| d.shape().first_release.as_ref().map(|r| (d.name(), r)))
        .collect();
    assert_eq!(later.len(), 4);
    assert!(
        later
            .iter()
            .all(|(_, release)| **release > baseline_release),
        "{later:?}"
    );
    assert_eq!(baseline().count(), 315);
    assert_eq!(
        molangx::stdlib::queries_at(Side::Client, &baseline_release).len(),
        315
    );
}

/// The tracked files, from `git ls-files`.
fn tracked_files() -> Vec<String> {
    let output = std::process::Command::new("git")
        .args(["ls-files", "-z"])
        .current_dir(root())
        .output()
        .expect("git ls-files: the wording check reads the repository's tracked files");
    assert!(
        output.status.success(),
        "git ls-files failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout)
        .expect("UTF-8 paths")
        .split('\0')
        .filter(|path| !path.is_empty())
        .map(str::to_owned)
        .collect()
}

/// Files the shape check skips: the console logs (kept verbatim), the lock file and the generated
/// fuzz seeds.
fn wording_exempt(path: &str) -> bool {
    path.starts_with("tests/server_logs/")
        || path == "Cargo.lock"
        || path.starts_with("fuzz/seeds/")
}

fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// Whether `text[at..]` starts a word.
fn word_start(text: &str, at: usize) -> bool {
    !text[..at].chars().next_back().is_some_and(is_word_char)
}

/// Whether `text[..at]` ends a word.
fn word_end(text: &str, at: usize) -> bool {
    !text[at..].chars().next().is_some_and(is_word_char)
}

/// The length of the ASCII identifier at the start of `text`.
fn identifier_length(text: &str) -> usize {
    text.find(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
        .unwrap_or(text.len())
}

/// The words after "binary" that give it its ordinary programming sense (an operator with two
/// operands, a binary search, a binary digit).
const BINARY_OPERATOR_WORDS: [&str; 15] = [
    "operator",
    "operand",
    "minus",
    "plus",
    "op",
    "form",
    "node",
    "level",
    "pass",
    "expression",
    "tree",
    "search",
    "arithmetic",
    "digit",
    "representation",
];

/// The words before "binary" that make it a program file rather than an operator.
const BINARY_FILE_ARTICLES: [&str; 8] = [
    "the", "both", "its", "from", "in", "a", "game's", "engine's",
];

/// Two or more `_`-separated parts, the first capitalised, each a capitalised word or a number, at
/// least one camel-case; or one such part or more followed by a trailing `_` (a wildcard over such
/// names).
fn test_name_shape(name: &str) -> bool {
    let (name, trailing) = match name.strip_suffix('_') {
        Some(stem) => (stem, true),
        None => (name, false),
    };
    let parts: Vec<&str> = name.split('_').collect();
    let capitalised = |part: &str| {
        part.starts_with(|c: char| c.is_ascii_uppercase())
            && part.chars().all(|c| c.is_ascii_alphanumeric())
    };
    let camel = |part: &str| {
        part.as_bytes()
            .windows(2)
            .any(|w| w[0].is_ascii_lowercase() && w[1].is_ascii_uppercase())
    };
    (parts.len() >= 2 || trailing)
        && capitalised(parts[0])
        && parts.iter().all(|part| {
            capitalised(part) || (!part.is_empty() && part.chars().all(|c| c.is_ascii_digit()))
        })
        && parts.iter().any(|part| camel(part))
}

/// One line's hits, as (rule, matched text).
fn wording_hits(line: &str) -> Vec<(&'static str, String)> {
    let mut hits = Vec::new();
    let lower = line.to_ascii_lowercase();
    let bytes = line.as_bytes();
    for (at, c) in line.char_indices() {
        let rest = &line[at..];
        let after = &rest[c.len_utf8()..];
        // A member name: `m`, an upper-case letter and a lower-case one, starting a word.
        if c == 'm'
            && word_start(line, at)
            && after.starts_with(|c: char| c.is_ascii_uppercase())
            && after[1..].starts_with(|c: char| c.is_ascii_lowercase())
        {
            hits.push(("member name", rest[..identifier_length(rest)].to_owned()));
        }
        // A private function name in backticks: `_` and a camel-case identifier.
        if c == '`' && after.starts_with('_') {
            let name = &after[..identifier_length(after)];
            if name[1..].starts_with(|c: char| c.is_ascii_lowercase())
                && name.contains(|c: char| c.is_ascii_uppercase())
            {
                hits.push(("private function name", name.to_owned()));
            }
        }
        // A class and a method in camel case starting lower-case: a Rust path segment is snake case
        // or starts upper-case.
        if c == ':' && after.starts_with(':') && at > 0 && bytes[at - 1].is_ascii_alphanumeric() {
            let method = &after[1..1 + identifier_length(&after[1..])];
            let class_start = line[..at]
                .rfind(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
                .map_or(0, |i| {
                    i + line[i..].chars().next().map_or(1, char::len_utf8)
                });
            let class = &line[class_start..at];
            let camel = method.starts_with(|c: char| c.is_ascii_lowercase())
                && method.contains(|c: char| c.is_ascii_uppercase())
                && !method.contains('_');
            if class.starts_with(|c: char| c.is_ascii_uppercase()) && camel {
                hits.push(("class method", format!("{class}::{method}")));
            }
        }
        // A compiler-generated name: `$_` and a digit.
        if c == '$'
            && after.starts_with('_')
            && after[1..].starts_with(|c: char| c.is_ascii_digit())
        {
            hits.push((
                "generated name",
                rest[..2 + identifier_length(&after[1..])].to_owned(),
            ));
        }
        // A design-record id standing in for a reason: `D`, an optional `-` and one to four digits,
        // as a word.
        if c == 'D' && word_start(line, at) {
            let number = after.strip_prefix('-').unwrap_or(after);
            let digits = number.len()
                - number
                    .trim_start_matches(|c: char| c.is_ascii_digit())
                    .len();
            let end = rest.len() - number.len() + digits;
            if (1..=4).contains(&digits) && word_end(rest, end) {
                hits.push(("record id", rest[..end].to_owned()));
            }
        }
        // An evidence tag with an address: `[`, a word, a space and a hexadecimal number, `]`.
        if c == '['
            && let Some(close) = after.find(']')
            && let Some((tag, address)) = after[..close].split_once(" 0x")
            && tag.starts_with(|c: char| c.is_ascii_alphabetic())
            && tag.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
            && !address.is_empty()
            && address.chars().all(|c| c.is_ascii_hexdigit())
        {
            hits.push(("evidence tag", rest[..close + 2].to_owned()));
        }
        if word_start(line, at) && c.is_ascii_alphabetic() {
            let name = &rest[..identifier_length(rest)];
            // An identifier ending in `Tests`: `Tests` after a lower-case letter, at the
            // identifier's end or before an underscore.
            let suite = name.match_indices("Tests").any(|(at, _)| {
                name[..at].ends_with(|c: char| c.is_ascii_lowercase())
                    && (name[at + 5..].is_empty() || name[at + 5..].starts_with('_'))
            });
            if suite {
                hits.push(("identifier ending in Tests", name.to_owned()));
            } else if test_name_shape(name) {
                hits.push(("identifier shaped like a C++ test name", name.to_owned()));
            }
        }
    }
    for (at, _) in line.match_indices("binar") {
        let word = &line[at..at + identifier_length(&line[at..])];
        let before = lower[..at].trim_end().rsplit(' ').next().unwrap_or("");
        let tail = lower[at + word.len()..].trim_start();
        let next = &tail[..identifier_length(tail)];
        let operator = tail.starts_with('`')
            || BINARY_OPERATOR_WORDS
                .iter()
                .any(|w| next == *w || next.strip_suffix('s') == Some(*w));
        if word_start(line, at)
            && (word.strip_prefix("binar") == Some("ies")
                || (word == "binary" && BINARY_FILE_ARTICLES.contains(&before) && !operator))
        {
            hits.push(("binary", format!("{before} {word}")));
        }
    }
    hits.dedup();
    hits
}

/// No tracked text file holds a member, private, camel-case method or generated name, a `…Tests` or
/// `Camel_Case` test name, an address tag, a design-record id, or "binary" meaning a program file.
/// It catches shapes, not meaning.
#[test]
fn no_tracked_file_holds_foreign_identifier_shapes() {
    let mut found = Vec::new();
    for path in tracked_files()
        .into_iter()
        .filter(|path| !wording_exempt(path))
    {
        // A file that is not UTF-8 is not prose.
        let Ok(text) = std::fs::read_to_string(root().join(&path)) else {
            continue;
        };
        for (n, line) in text.lines().enumerate() {
            for (rule, matched) in wording_hits(line) {
                found.push(format!("{path}:{}: {rule}: {matched}", n + 1));
            }
        }
    }
    assert!(
        found.is_empty(),
        "{} hit(s) of the wording rule (CONTRIBUTING.md):\n{}",
        found.len(),
        found.join("\n")
    );
}

/// The samples carry a `~`, removed before the check, so that this file holds none of the shapes.
#[test]
fn the_wording_check_notices_what_it_forbids() {
    for line in [
        "the `m~QuuxFrob` member",
        "the `_~frobQuux` function",
        "`Quux::~frobNicate`",
        "the `$~_12` name",
        "(record D~-123)",
        "measured (D~5)",
        "a tag `[quux 0x~1234]`",
        "read from the bin~ary",
        "`QuuxFrob~Tests::Case`",
        "`Quux_~FrobNicate`",
        "(`FrobNic~ate_*`: 2 → 3)",
        "the `QuuxFr~ob_` cases",
    ] {
        let line = line.replace('~', "");
        assert!(!wording_hits(&line).is_empty(), "not noticed: {line}");
    }
    for line in [
        "a binary operator, the binary minus, a binary `-`, the binary levels, a binary search, `BINARY`",
        "the dedicated server 1.26.36.1 and 1.26.52.3",
        "[[example]], [0x1234], x86-64, UTF-8",
        "`EvalLimits::new`, `MolangVersion::from_i16`, `Self::new()`, `mod tests`, `No_Such_Query`, `My_Var_2`",
        "the unit tests of this crate; `cargo test --features fuzz`; message E05; `FrobTestRules`",
        "quuxVal, DEFAULT, E-12, 0xD1, `Deviations::NONE`, MAX_STEPS",
        "`hi·f32::EPSILON`, `(−f32::EPSILON)`",
    ] {
        assert_eq!(wording_hits(line), Vec::<(&str, String)>::new(), "{line}");
    }
}

/// The word no tracked file outside `tests/server_logs/` may hold, in any case; split so that this
/// file does not hold it.
const BANNED_WORD: &str = concat!("van", "illa");

/// The 1-based numbers of the lines of `text` that hold [`BANNED_WORD`] in any case.
fn banned_word_lines(text: &[u8]) -> Vec<usize> {
    let word = BANNED_WORD.as_bytes();
    text.split(|&b| b == b'\n')
        .enumerate()
        .filter(|(_, line)| {
            line.windows(word.len())
                .any(|w| w.eq_ignore_ascii_case(word))
        })
        .map(|(n, _)| n + 1)
        .collect()
}

/// The 1-based numbers of the lines over 100 columns in the Rust blocks of `text`: those of its
/// doc comments (`doc`), else those of a Markdown file.
fn wide_rust_block_lines(text: &str, doc: bool) -> Vec<usize> {
    let mut wide = Vec::new();
    let mut fence: Option<bool> = None;
    for (n, line) in text.lines().enumerate() {
        let trimmed = line.trim_start();
        let body = if doc {
            trimmed
                .strip_prefix("///")
                .or_else(|| trimmed.strip_prefix("//!"))
        } else {
            Some(line)
        };
        let Some(body) = body else {
            fence = None;
            continue;
        };
        if let Some(info) = body.trim().strip_prefix("```") {
            fence = match fence {
                Some(_) => None,
                None => Some(info.is_empty() || info.starts_with("rust")),
            };
            continue;
        }
        if fence == Some(true) && line.chars().count() > 100 {
            wide.push(n + 1);
        }
    }
    wide
}

/// rustfmt does not format doc examples or the README's Rust blocks; their lines are wrapped by hand
/// within 100 columns.
#[test]
fn doc_example_code_fits_in_100_columns() {
    let mut wide = Vec::new();
    for path in tracked_files() {
        let doc = path.ends_with(".rs");
        if !doc && path != "README.md" {
            continue;
        }
        let lines = wide_rust_block_lines(&read(&path), doc);
        wide.extend(lines.into_iter().map(|n| format!("{path}:{n}")));
    }
    assert!(
        wide.is_empty(),
        "doc example lines over 100 columns:\n{}",
        wide.join("\n")
    );
}

#[test]
fn the_wide_line_check_reads_doc_comments_and_markdown_blocks() {
    let long = format!("let x = {};", "1".repeat(100));
    let doc = format!("/// ```\n/// {long}\n/// ```\n/// {long}\n");
    assert_eq!(wide_rust_block_lines(&doc, true), [2]);
    let markdown = format!("```rust\n{long}\n```\n```toml\n{long}\n```\n{long}\n");
    assert_eq!(wide_rust_block_lines(&markdown, false), [2]);
}

/// Read as bytes, so a file that is not UTF-8 is checked too.
#[test]
fn no_tracked_file_holds_the_banned_word() {
    let mut found = Vec::new();
    for path in tracked_files()
        .into_iter()
        .filter(|path| !path.starts_with("tests/server_logs/"))
    {
        let Ok(bytes) = std::fs::read(root().join(&path)) else {
            continue;
        };
        found.extend(
            banned_word_lines(&bytes)
                .into_iter()
                .map(|n| format!("{path}:{n}")),
        );
    }
    assert!(
        found.is_empty(),
        "{} line(s) hold the banned word:\n{}",
        found.len(),
        found.join("\n")
    );
}

#[test]
fn the_banned_word_check_notices_it_in_any_case() {
    let upper = BANNED_WORD.to_ascii_uppercase();
    let capitalised = format!("V{}", &BANNED_WORD[1..]);
    for text in [
        BANNED_WORD.to_owned(),
        upper,
        format!("fn eval_{BANNED_WORD}() {{}}"),
        format!("x\nNot{capitalised}Parity\n"),
    ] {
        assert!(
            !banned_word_lines(text.as_bytes()).is_empty(),
            "not noticed: {text}"
        );
    }
    assert_eq!(
        banned_word_lines(format!("a\nb\n{BANNED_WORD}").as_bytes()),
        [3]
    );
    assert!(banned_word_lines(b"a vanishing illusion, van illa, vanill").is_empty());
    assert!(
        banned_word_lines(&read("README.md").into_bytes()).is_empty(),
        "README.md"
    );
}

/// The module path of a source file under `src/`: `src/vm/mod.rs` is `["vm"]`.
fn module_path_of(file: &str) -> Vec<String> {
    let path = file.strip_prefix("src/").unwrap_or(file);
    let path = path.strip_suffix(".rs").unwrap_or(path);
    let mut segments: Vec<String> = path.split('/').map(str::to_owned).collect();
    if matches!(segments.last().map(String::as_str), Some("mod" | "lib")) {
        segments.pop();
    }
    segments
}

/// The leaf paths of a use tree (`a::{b, c::{d}}` gives `a::b` and `a::c::d`), renames dropped.
fn use_tree_leaves(tree: &str) -> Vec<String> {
    let tree = tree.split_whitespace().collect::<Vec<_>>().join(" ");
    let tree = ["::", "{", "}", ","].iter().fold(tree, |tree, sep| {
        tree.replace(&format!(" {sep}"), sep)
            .replace(&format!("{sep} "), sep)
    });
    let Some(open) = tree.find('{') else {
        let leaf = tree.split(" as ").next().unwrap_or_default();
        return vec![leaf.to_owned()];
    };
    let (head, rest) = tree.split_at(open);
    let body = &rest[1..rest.len() - 1];
    let mut parts = Vec::new();
    let (mut depth, mut start) = (0, 0);
    for (at, c) in body.char_indices() {
        match c {
            '{' => depth += 1,
            '}' => depth -= 1,
            ',' if depth == 0 => {
                parts.push(&body[start..at]);
                start = at + 1;
            }
            _ => {}
        }
    }
    parts.push(&body[start..]);
    parts
        .into_iter()
        .filter(|part| !part.is_empty())
        .flat_map(use_tree_leaves)
        .map(|leaf| match leaf.as_str() {
            "self" => head.trim_end_matches("::").to_owned(),
            _ => format!("{head}{leaf}"),
        })
        .collect()
}

/// The text of every `pub use … ;` and `pub type … ;` of `text`, whitespace collapsed.
fn public_statements(text: &str, keyword: &str) -> Vec<String> {
    let mut found = Vec::new();
    let mut rest = text;
    let start = format!("pub {keyword} ");
    while let Some(at) = rest.find(&start) {
        let line_start = rest[..at].rfind('\n').map_or(0, |n| n + 1);
        let before = rest[line_start..at].trim();
        let after = &rest[at + start.len()..];
        let end = after.find(';').unwrap_or(after.len());
        if before.is_empty() {
            found.push(
                after[..end]
                    .split_whitespace()
                    .collect::<Vec<_>>()
                    .join(" "),
            );
        }
        rest = &after[end..];
    }
    found
}

/// Every second public path the sources open, one message each. `files` are `(path, text)` pairs
/// under `src/`.
///
/// The crate root declares modules only. Elsewhere a `pub use` names a whole external crate or an
/// item of a module that is not public, each once, and never a glob; there is no
/// `pub extern crate`. A `pub type` names an instantiation of a public generic type or a `dyn`
/// type: it never renames a type, and no two aliases name the same type.
fn second_public_paths(files: &[(String, String)]) -> Vec<String> {
    let public_module = public_modules(files);
    let mut violations = Vec::new();
    let mut sources: BTreeMap<String, String> = BTreeMap::new();
    let mut aliases: BTreeMap<String, String> = BTreeMap::new();
    for (file, text) in files {
        let module = module_path_of(file);
        if module.is_empty() {
            violations.extend(root_items(file, text));
        }
        for name in public_statements(text, "extern crate") {
            violations.push(format!(
                "{file}: `pub extern crate {name}` re-exports a crate; use `pub use`"
            ));
        }
        for tree in public_statements(text, "use") {
            for leaf in use_tree_leaves(&tree) {
                let Some(source) =
                    re_exported(file, &module, &leaf, &public_module, &mut violations)
                else {
                    continue;
                };
                if let Some(first) = sources.insert(source.clone(), file.clone()) {
                    violations.push(format!(
                        "{file}: `{source}` is re-exported here and in {first}"
                    ));
                }
            }
        }
        for alias in public_statements(text, "type") {
            violations.extend(alias_violation(file, &alias, &mut aliases));
        }
    }
    violations
}

/// Whether each declared module is `pub` (not `pub(…)`), by module path.
fn public_modules(files: &[(String, String)]) -> BTreeMap<Vec<String>, bool> {
    let mut public_module = BTreeMap::new();
    for (file, text) in files {
        let parent = module_path_of(file);
        for line in text.lines() {
            if let Some((name, _)) = module_declaration(line) {
                let mut path = parent.clone();
                path.push(name.to_owned());
                public_module.insert(path, line.trim_start().starts_with("pub mod "));
            }
        }
    }
    public_module
}

/// The crate root's public items other than modules, one message each.
fn root_items(file: &str, text: &str) -> Vec<String> {
    text.lines()
        .map(str::trim_start)
        .filter(|line| line.starts_with("pub ") && !line.starts_with("pub mod "))
        .map(|line| format!("{file}: the crate root holds `{line}`"))
        .collect()
}

/// What the leaf `leaf` of a `pub use` in `module` re-exports, as a path from the crate root or an
/// external crate's name; `None` for a glob, which is a violation.
fn re_exported(
    file: &str,
    module: &[String],
    leaf: &str,
    public_module: &BTreeMap<Vec<String>, bool>,
    violations: &mut Vec<String>,
) -> Option<String> {
    let segments: Vec<&str> = leaf.split("::").collect();
    if segments.last() == Some(&"*") {
        violations.push(format!("{file}: `pub use {leaf}` re-exports a glob"));
        return None;
    }
    if segments.len() == 1 && !matches!(segments[0], "self" | "super" | "crate") {
        return Some(leaf.to_owned());
    }
    let mut path: Vec<String> = match segments[0] {
        "crate" => Vec::new(),
        "super" => module[..module.len().saturating_sub(1)].to_vec(),
        _ => module.to_vec(),
    };
    let skip = usize::from(matches!(segments[0], "crate" | "self" | "super"));
    path.extend(segments[skip..].iter().map(|s| (*s).to_owned()));
    let source = path.join("::");
    if (1..path.len()).all(|n| public_module.get(&path[..n]).copied().unwrap_or(true)) {
        violations.push(format!(
            "{file}: `pub use {leaf}` re-exports `{source}`, which has a public path"
        ));
    }
    Some(source)
}

/// The violation of the `pub type` statement `alias`, if any; `aliases` maps each aliased type
/// seen so far to its first alias.
fn alias_violation(
    file: &str,
    alias: &str,
    aliases: &mut BTreeMap<String, String>,
) -> Option<String> {
    let (name, target) = alias.split_once(" = ")?;
    let params = name.find('<').map_or("", |at| &name[at..]);
    let args = target.find('<').map_or("", |at| &target[at..]);
    let plain = !target.contains(['<', '(', '&']) && !target.starts_with("dyn ");
    if plain || (!args.is_empty() && args == params) {
        return Some(format!(
            "{file}: `pub type {alias}` is a second name of a type"
        ));
    }
    let first = aliases.insert(last_segments(target), format!("{file}: `pub type {alias}`"))?;
    Some(format!(
        "{file}: `pub type {alias}` names the same type as {first}"
    ))
}

/// `target` with whitespace removed and each path cut to its last segment:
/// `crate::vm::Name<namespace::Temp>` is `Name<Temp>`.
fn last_segments(target: &str) -> String {
    let target: String = target.split_whitespace().collect();
    let pieces: Vec<&str> = target.split("::").collect();
    let Some((last, init)) = pieces.split_last() else {
        return target;
    };
    init.iter()
        .map(|piece| piece.trim_end_matches(|c: char| c.is_alphanumeric() || c == '_'))
        .chain([*last])
        .collect()
}

/// The `.rs` files under `src/`, with their text.
fn source_files() -> Vec<(String, String)> {
    let (_, files) = tree_below("src");
    files
        .into_iter()
        .filter(|file| file.ends_with(".rs"))
        .map(|file| {
            let text = read(&file);
            (file, text)
        })
        .collect()
}

#[test]
fn every_public_item_has_one_path() {
    let violations = second_public_paths(&source_files());
    assert!(
        violations.is_empty(),
        "an item has a second public path:\n{}",
        violations.join("\n")
    );
}

#[test]
fn the_one_path_check_notices_a_second_path() {
    let with = |extra: &[(&str, &str)]| {
        let mut files = source_files();
        for (file, text) in extra {
            match files.iter_mut().find(|(path, _)| path == file) {
                Some((_, existing)) => {
                    existing.push('\n');
                    existing.push_str(text);
                }
                None => files.push(((*file).to_owned(), (*text).to_owned())),
            }
        }
        second_public_paths(&files)
    };
    for (file, text, message) in [
        (
            "src/lib.rs",
            "pub use catalog::Side;",
            "src/lib.rs: the crate root holds `pub use catalog::Side;`",
        ),
        (
            "src/lib.rs",
            "pub const LIMIT: u32 = 1;",
            "src/lib.rs: the crate root holds `pub const LIMIT: u32 = 1;`",
        ),
        (
            "src/vm/mod.rs",
            "pub use crate::catalog::{QueryDecl, Side};",
            "src/vm/mod.rs: `pub use crate::catalog::Side` re-exports `catalog::Side`, which has a public path",
        ),
        (
            "src/numeric/mod.rs",
            "pub use arith::add;",
            "src/numeric/mod.rs: `pub use arith::add` re-exports `numeric::arith::add`, which has a public path",
        ),
        (
            "src/stdlib/mod.rs",
            "pub use crate::catalog::query::Reads;",
            "src/stdlib/mod.rs: `catalog::query::Reads` is re-exported here and in src/catalog/mod.rs",
        ),
        (
            "src/version/mod.rs",
            "pub type Release = semver::Version;",
            "src/version/mod.rs: `pub type Release = semver::Version` is a second name of a type",
        ),
        (
            "src/vm/mod.rs",
            "pub type Map<H, N> = VariableMap<H, N>;",
            "src/vm/mod.rs: `pub type Map<H, N> = VariableMap<H, N>` is a second name of a type",
        ),
        (
            "src/vm/mod.rs",
            "pub use semver;",
            "src/vm/mod.rs: `semver` is re-exported here and in src/version/mod.rs",
        ),
        (
            "src/vm/mod.rs",
            "pub extern crate semver;",
            "src/vm/mod.rs: `pub extern crate semver` re-exports a crate; use `pub use`",
        ),
        (
            "src/compile/mod.rs",
            "pub use crate::catalog::math::*;",
            "src/compile/mod.rs: `pub use crate::catalog::math::*` re-exports a glob",
        ),
        (
            "src/catalog/mod.rs",
            "pub use query::Side::*;",
            "src/catalog/mod.rs: `pub use query::Side::*` re-exports a glob",
        ),
        (
            "src/vm/vars.rs",
            "pub type VarName = crate::vm::Name<crate::vm::namespace::Variable>;",
            "src/vm/vars.rs: `pub type VarName = crate::vm::Name<crate::vm::namespace::Variable>` \
             names the same type as src/vm/name.rs: `pub type VariableName = Name<Variable>`",
        ),
    ] {
        let found = with(&[(file, text)]);
        assert!(found.iter().any(|v| v == message), "{text}: {found:?}");
    }
}
