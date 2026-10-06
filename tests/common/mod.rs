//! Shared helpers of the integration tests. A test crate declares this directory with `mod
//! common;`.

#![allow(dead_code, reason = "each test crate uses a part of the helpers")]

use std::cell::Cell;
use std::path::PathBuf;
use std::sync::LazyLock;

use molangx::catalog::{
    QueryCatalog, QueryDecl, QuerySetMask, QueryShape, ReturnType, Side, VersionRange,
    VersionRanges,
};
use molangx::version::{Experiment, ExperimentMask, MolangVersion};

#[cfg(feature = "compiler")]
pub mod compile_support;
pub mod declared;
#[cfg(feature = "vm")]
pub mod host;
#[cfg(feature = "compiler")]
pub mod measured;

pub const HELPER_SET: QuerySetMask = QuerySetMask::host(0).unwrap();
pub const HELPER_EXPERIMENT: Experiment = Experiment::new(63).unwrap();
pub const REFERENCE_SETS: QuerySetMask = QuerySetMask::DEFAULT.union(HELPER_SET);
pub const GET_NAME_TEST: &str = "query.get_name_test";
pub const SUM_TEST: &str = "query.sum_test";
pub const EXPERIMENTAL_TEST: &str = "query.experimental_test";

/// The 64-bit FNV-1 hash of `'a'`.
pub const HASH_OF_A: u64 = 0xaf63_bd4c_8601_b7be;
/// The hash of `'moo'`, what `query.get_name_test(0)` returns.
pub const HASH_OF_MOO: u64 = 0xd8b3_c718_6b8c_a314;
/// The hash of `'rabbit'`, what `query.get_name_test(1)` returns.
pub const HASH_OF_RABBIT: u64 = 0x702d_ce83_260a_61e9;

pub fn reference_catalog() -> &'static QueryCatalog {
    static CATALOG: LazyLock<QueryCatalog> = LazyLock::new(|| {
        let window = |first: i16, last: i16| {
            VersionRange::new(
                MolangVersion::from_i16(first).unwrap(),
                MolangVersion::from_i16(last).unwrap(),
                HELPER_SET,
            )
            .unwrap()
        };
        let shape = |returns: ReturnType, first: i16, last: i16| QueryShape {
            returns,
            ranges: VersionRanges::single(window(first, last)),
            ..QueryShape::DEFAULT
        };
        let experimental = QueryShape {
            experiments: reference_experiments(),
            ..shape(ReturnType::FLOAT, -1, 13)
        };
        let decls = [
            (GET_NAME_TEST, shape(ReturnType::STRING, -1, 13)),
            (SUM_TEST, shape(ReturnType::FLOAT, -1, 13)),
            (EXPERIMENTAL_TEST, experimental),
            ("query.valid_always", shape(ReturnType::FLOAT, 1, 13)),
            ("query.valid_early", shape(ReturnType::FLOAT, 1, 8)),
            ("query.valid_mid", shape(ReturnType::FLOAT, 4, 8)),
            ("query.valid_late", shape(ReturnType::FLOAT, 4, 13)),
        ];
        molangx::stdlib::queries(Side::Client)
            .extended(decls.map(|(name, shape)| QueryDecl::new(name, shape).unwrap()))
            .unwrap()
    });
    &CATALOG
}

pub fn reference_experiments() -> ExperimentMask {
    ExperimentMask::empty().with(HELPER_EXPERIMENT)
}

/// Panics on drop unless marked (and the thread is not already panicking), so a builder whose
/// terminal call (`check`, `replay`) is forgotten fails its test.
#[derive(Debug)]
pub struct CheckGuard {
    what: String,
    checked: Cell<bool>,
}

impl CheckGuard {
    pub fn new(what: &str) -> Self {
        Self {
            what: what.to_owned(),
            checked: Cell::new(false),
        }
    }

    /// Marks the guard and asserts `actual == stated`, so a deleted row fails the test.
    pub fn checked(&self, actual: usize, stated: usize) {
        self.checked.set(true);
        assert_eq!(
            actual, stated,
            "{}: holds {actual} row(s), its test states {stated}",
            self.what
        );
    }

    pub fn used_without_check(&self) {
        self.checked.set(true);
    }
}

impl Drop for CheckGuard {
    fn drop(&mut self) {
        if !self.checked.get() && !std::thread::panicking() {
            panic!(
                "{}: built but never checked: end it with its terminal call (`check` or `replay`)",
                self.what
            );
        }
    }
}

/// The word whose sample is `sample`.
#[cfg(feature = "compiler")]
pub fn word(sample: f32) -> u32 {
    molangx::rng::FixedRng::from_sample(sample)
        .unwrap_or_else(|| panic!("{sample} is no word's sample"))
        .0
}

/// Plays back the words of `samples` in order, then of `rest`, and counts the draws.
#[cfg(feature = "compiler")]
#[derive(Debug)]
pub struct Samples<'a> {
    samples: &'a [f32],
    rest: f32,
    pub draws: usize,
}

#[cfg(feature = "compiler")]
impl<'a> Samples<'a> {
    /// `samples`, then 0.
    pub fn new(samples: &'a [f32]) -> Self {
        Self {
            samples,
            rest: 0.0,
            draws: 0,
        }
    }

    /// `sample` on every draw.
    pub fn repeat(sample: f32) -> Self {
        Self {
            samples: &[],
            rest: sample,
            draws: 0,
        }
    }
}

#[cfg(feature = "compiler")]
impl molangx::rng::rand_core::TryRng for Samples<'_> {
    type Error = std::convert::Infallible;

    fn try_next_u32(&mut self) -> Result<u32, Self::Error> {
        let sample = self.samples.get(self.draws).copied().unwrap_or(self.rest);
        self.draws += 1;
        Ok(word(sample))
    }

    fn try_next_u64(&mut self) -> Result<u64, Self::Error> {
        unreachable!("a sample is one next_u32")
    }

    fn try_fill_bytes(&mut self, _: &mut [u8]) -> Result<(), Self::Error> {
        unreachable!("a sample is one next_u32")
    }
}

/// Evaluates `source`, compiled for the server at version 13, with [`Samples::new`]`(samples)` as
/// the random source: the value and the number of draws.
#[cfg(feature = "vm")]
pub fn eval_with_samples(source: &str, samples: &[f32]) -> (f32, usize) {
    let expr = compile_support::server_expr(source);
    let mut env = molangx::vm::NoHostEnv::new();
    let mut rng = Samples::new(samples);
    let mut cx = env.cx();
    cx.rng = &mut rng;
    let value = expr.eval_f32(&mut cx);
    (value, rng.draws)
}

/// The path of `tests/<name>`.
pub fn data_path(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join(name)
}

/// Panics unless the file's `schema` starts with `molangx/`.
pub fn data_json(name: &str) -> serde_json::Value {
    let path = data_path(name);
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()));
    let value: serde_json::Value = serde_json::from_str(&text)
        .unwrap_or_else(|e| panic!("cannot parse {}: {e}", path.display()));
    let schema = value["schema"]
        .as_str()
        .unwrap_or_else(|| panic!("{} has no schema", path.display()));
    assert!(
        schema.starts_with("molangx/"),
        "{}: schema {schema:?}",
        path.display()
    );
    value
}

/// Reads a data-file float, stored as its `f32` bits in eight hex digits.
#[cfg(feature = "compiler")]
pub fn hex_f32(value: &serde_json::Value) -> f32 {
    f32::from_bits(u32::from_str_radix(value.as_str().expect("hex bits"), 16).expect("hex bits"))
}

/// Restores an input that `tests/parse_vectors.json` stores as its UTF-8 bytes decoded as Latin-1.
pub fn latin1_to_utf8(text: &str) -> String {
    let bytes: Vec<u8> = text
        .chars()
        .map(|c| u8::try_from(u32::from(c)).unwrap_or(b'?'))
        .collect();
    String::from_utf8(bytes).unwrap_or_else(|e| String::from_utf8_lossy(e.as_bytes()).into_owned())
}

/// `x86_64` on an `X86_64` build, `arm64` on an `Arm64` one.
#[cfg(feature = "compiler")]
pub fn per_arch<T>(x86_64: T, arm64: T) -> T {
    match molangx::numeric::ARCH {
        molangx::numeric::Arch::X86_64 => x86_64,
        molangx::numeric::Arch::Arm64 => arm64,
    }
}

/// Bitwise equality, any NaN matching any NaN: for expectations that record only "a NaN".
pub fn nan_or_same_bits(a: f32, b: f32) -> bool {
    a.to_bits() == b.to_bits() || (a.is_nan() && b.is_nan())
}

/// `|actual − expected| <= tolerance`; a non-finite expectation must match to the bit.
pub fn within(actual: f32, expected: f32, tolerance: f32) -> bool {
    if expected.is_finite() {
        (actual - expected).abs() <= tolerance
    } else {
        actual.to_bits() == expected.to_bits()
    }
}

#[track_caller]
pub fn assert_bits(actual: f32, expected: f32, what: &str) {
    assert!(
        actual.to_bits() == expected.to_bits(),
        "{what}: {actual:e} ({:#010x}), expected {expected:e} ({:#010x})",
        actual.to_bits(),
        expected.to_bits()
    );
}

pub fn hex_to_string(hex: &str) -> String {
    let bytes: Vec<u8> = (0..hex.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).expect("hex"))
        .collect();
    String::from_utf8(bytes).expect("the inputs are UTF-8")
}
