//! The NaN / signed-zero / infinity sweeps of `server_sweeps.json`, replayed on both releases.
//!
//! Each expression evaluates one math function or operator on NaN of either sign, ±0, ±infinity or
//! ordinary values inside `v.one ? (…) : 0` and reads the class of the result: ±infinity, finite
//! (for some split into equal to a stated value with either sign, and not equal), or NaN with the
//! sign bit clear or set. A record holds `run`, `id`, `expr`, an optional `version`, the
//! `question`, the `branches` it can answer, the branch each release took (`observed`) and
//! `predicted_x86_64`, which the replay ignores.

#![cfg(all(feature = "compiler", feature = "stdlib"))]

mod common;

use std::collections::{BTreeMap, BTreeSet};
use std::sync::LazyLock;

use common::measured::*;
use common::{data_json, data_path};
use serde_json::Value;

/// `math.ln` of a negative number is `0x7fc00000` under `X86_64`, as on 1.26.52.3; 1.26.36.1 sets
/// the sign bit.
const LN_NAN_SIGN: &str = "math.ln of a negative number: NaN sign of 1.26.36.1";

/// The entries not reproduced on 1.26.36.1 ([`LN_NAN_SIGN`]): run, id, and the lines logged instead
/// (all at Error level).
const EXPECTED_FAILURES: &[(u64, &str, &[&str])] = &[
    (
        44,
        "s0038",
        &["Error: unhandled request for unknown variable 'variable.s0038_nanp'"],
    ),
    (
        44,
        "s0039",
        &["Error: unhandled request for unknown variable 'variable.s0039_nanp'"],
    ),
    (
        44,
        "s0042",
        &["Error: unhandled request for unknown variable 'variable.s0042_nanp'"],
    ),
    (
        44,
        "s0043",
        &["Error: unhandled request for unknown variable 'variable.s0043_nanp'"],
    ),
    (
        44,
        "s0046",
        &["Error: unhandled request for unknown variable 'variable.s0046_nanp'"],
    ),
    (
        44,
        "s0047",
        &["Error: unhandled request for unknown variable 'variable.s0047_nanp'"],
    ),
    (
        44,
        "s0056",
        &["Error: unhandled request for unknown variable 'variable.s0056_nanp'"],
    ),
    (
        44,
        "s0057",
        &["Error: unhandled request for unknown variable 'variable.s0057_nanp'"],
    ),
];

/// Parsed once for every test of the file.
fn sweeps() -> &'static Value {
    static SWEEPS: LazyLock<Value> = LazyLock::new(|| data_json("server_sweeps.json"));
    &SWEEPS
}

fn str_field<'a>(record: &'a Value, key: &str) -> &'a str {
    record[key]
        .as_str()
        .unwrap_or_else(|| panic!("{key} is not a string in {record}"))
}

fn run_number(record: &Value) -> u64 {
    record["run"]
        .as_u64()
        .unwrap_or_else(|| panic!("run is not a number in {record}"))
}

fn sweep_run(run: u64) -> ServerRun {
    let data = sweeps();
    let meta = data["runs"]
        .as_array()
        .expect("runs")
        .iter()
        .find(|r| run_number(r) == run)
        .unwrap_or_else(|| panic!("run {run} is not in tests/server_sweeps.json"));
    let mut server_run = ServerRun::new(
        &format!("run_{run:02}"),
        BOTH_RELEASES,
        str_field(meta, "purpose"),
    );
    for record in data["probes"]
        .as_array()
        .expect("probes")
        .iter()
        .filter(|p| run_number(p) == run)
    {
        let id = str_field(record, "id");
        let probe = server_run.probe(id, str_field(record, "expr"));
        if let Some(version) = record.get("version") {
            let version = version
                .as_i64()
                .and_then(|v| i16::try_from(v).ok())
                .expect("version");
            probe.at(version);
        }
        for release in BOTH_RELEASES {
            probe.answers_on(*release, str_field(&record["observed"], release.text()));
        }
        for (_, _, ours) in EXPECTED_FAILURES
            .iter()
            .filter(|(r, i, _)| *r == run && *i == id)
        {
            probe.expected_failure(Release::V1_26_36_1, LN_NAN_SIGN, ours);
        }
    }
    server_run
}

/// Replays one run of `probes` entries and checks that every expected failure was met.
fn replay(run: u64, probes: usize) {
    let report = sweep_run(run).replay(probes);
    let expected = EXPECTED_FAILURES
        .iter()
        .filter(|(r, _, _)| *r == run)
        .count();
    assert_eq!(
        report.expected_failures, expected,
        "run_{run}: expected failures"
    );
    assert_eq!(
        report.not_run, 0,
        "run_{run}: every sweep probe was answered"
    );
}

/// `math.sqrt`, `math.ln`, `math.exp`, `math.asin`, `math.acos`, `math.sin`, `math.cos`,
/// `math.round`, `math.floor`, `math.ceil`, `math.trunc`, `math.min_angle`, `math.hermite_blend`.
#[test]
fn roots_logarithms_trigonometry_and_rounding_sweep_run_44() {
    replay(44, 406);
}

/// `math.hermite_blend`, `math.atan`, `math.abs`, `math.sign`, `math.atan2`, `math.copy_sign`,
/// `math.max`.
#[test]
fn abs_sign_atan2_copy_sign_and_max_sweep_run_45() {
    replay(45, 406);
}

/// `math.max`, `math.min`, `math.mod`.
#[test]
fn max_min_and_mod_sweep_run_46() {
    replay(46, 406);
}

/// `math.mod`, `math.pow`, `math.random`, `math.random_integer`.
#[test]
fn mod_pow_and_random_sweep_run_47() {
    replay(47, 406);
}

/// `math.random_integer`, `math.clamp`, `math.lerp`, `math.inverse_lerp`, `math.lerprotate`, and
/// the quadratic, cubic and quartic easings.
#[test]
fn clamp_lerps_and_polynomial_easings_sweep_run_48() {
    replay(48, 406);
}

/// The easings from `math.ease_out_quart` to `math.ease_in_out_back`.
#[test]
fn quartic_to_back_easings_sweep_run_49() {
    replay(49, 406);
}

/// `math.ease_in_out_back`, the elastic and bounce easings, then the binary `+` and `-`.
#[test]
fn elastic_and_bounce_easings_and_addition_sweep_run_50() {
    replay(50, 406);
}

/// The binary `-`, `*` and `/`.
#[test]
fn subtraction_multiplication_and_division_sweep_run_51() {
    replay(51, 406);
}

/// `/`, `==`, `!=`, `<`, `<=`.
#[test]
fn division_and_comparisons_sweep_run_52() {
    replay(52, 406);
}

/// `<=`, `>`, `>=`, `&&`, `||`, the unary `-` and `!`, and `0 - x`.
#[test]
fn comparisons_logical_operators_and_negation_sweep_run_53() {
    replay(53, 399);
}

/// Chains of `+`, `-`, `*` and `/`, sums and products inside other operations, lerps, clamp,
/// easings, `math.die_roll` and `math.die_roll_integer`.
#[test]
fn operator_chains_lerps_and_die_rolls_follow_up_sweep_run_54() {
    replay(54, 248);
}

/// Every easing with a NaN of each sign in two of its three arguments, in every order.
#[test]
fn easing_pairs_sweep_run_55() {
    replay(55, 204);
}

/// Ids are unique per run and every entry has both releases' branch; the releases differ exactly on
/// [`EXPECTED_FAILURES`] (`nann` on 1.26.36.1, `nanp` on 1.26.52.3).
#[test]
fn the_sweep_file_is_complete() {
    let data = sweeps();
    let releases: Vec<&str> = data["releases"]
        .as_array()
        .expect("releases")
        .iter()
        .map(|r| r.as_str().expect("release"))
        .collect();
    assert_eq!(
        releases,
        BOTH_RELEASES.iter().map(|r| r.text()).collect::<Vec<_>>()
    );

    let runs: Vec<u64> = data["runs"]
        .as_array()
        .expect("runs")
        .iter()
        .map(run_number)
        .collect();
    assert_eq!(runs, (44..=55).collect::<Vec<_>>());
    for run in &runs {
        for release in BOTH_RELEASES {
            let path = data_path(&format!("server_logs/run_{run:02}_{}.log", release.text()));
            assert!(path.is_file(), "missing {}", path.display());
        }
    }

    let probes = data["probes"].as_array().expect("probes");
    assert_eq!(probes.len(), 4_505);
    let mut ids: BTreeMap<u64, BTreeSet<&str>> = BTreeMap::new();
    let mut differ = Vec::new();
    let mut previous_run = 0;
    for record in probes {
        let run = run_number(record);
        assert!(runs.contains(&run), "probe of an unknown run: {record}");
        assert!(run >= previous_run, "probes out of run order: {record}");
        previous_run = run;
        let id = str_field(record, "id");
        assert!(
            ids.entry(run).or_default().insert(id),
            "run_{run}: id {id} repeats"
        );
        let observed = record["observed"].as_object().expect("observed");
        assert_eq!(
            observed.len(),
            2,
            "run_{run} {id}: observed on both releases"
        );
        let first = str_field(&record["observed"], Release::V1_26_36_1.text());
        let second = str_field(&record["observed"], Release::V1_26_52_3.text());
        if first != second {
            assert_eq!(first, format!("{id}_nann"), "run_{run} {id}");
            assert_eq!(second, format!("{id}_nanp"), "run_{run} {id}");
            differ.push((run, id));
        }
    }
    let expected: Vec<(u64, &str)> = EXPECTED_FAILURES
        .iter()
        .map(|(run, id, _)| (*run, *id))
        .collect();
    assert_eq!(differ, expected);
}
