//! The bytecode VM against the tree walker on the main crate's data files, compared by
//! `common::differential`:
//! - every accepted row of `tests/parse_vectors.json`;
//! - every entry of `tests/server_sweeps.json`.

mod common;

use common::differential::Tally;
use common::host::Env;
use molangx::catalog::{QueryAdmission, QuerySetMask};
use molangx::compile::{CompileOptions, compile};
use molangx::rng::Xorshift128;
use molangx::vm::{EvalLimits, Value, VariableName};
use molangx_fuzz::generator::env::FuzzRng;
use serde_json::Value as Json;

fn version(record: &Json) -> i16 {
    i16::try_from(record["version"].as_i64().expect("version")).expect("version fits i16")
}

/// Prints what a tally compared and fails on any disagreement.
fn report(what: &str, tally: &Tally) {
    println!(
        "{what}: {} expressions, {} evaluations compared, {} not modelled",
        tally.expressions, tally.evaluations, tally.not_modelled
    );
    assert!(tally.expressions > 0, "{what}: nothing compared");
    tally.assert_clean(what);
}

/// Compiled with no query set; evaluated on a fresh context.
#[test]
fn vm_equals_tree_walker_on_every_accepted_parse_row() {
    const FILE: &str = "parse_vectors.json";
    let file = common::data_json(FILE);
    let mut tally = Tally::default();
    for (index, row) in file["vectors"]
        .as_array()
        .expect("vectors")
        .iter()
        .enumerate()
    {
        if row["result"] != "OK" {
            continue;
        }
        let source = row
            .get("input_bytes_hex")
            .and_then(Json::as_str)
            .map_or_else(
                || row["expr"].as_str().expect("expr").to_owned(),
                common::hex_to_string,
            );
        let options = CompileOptions {
            admission: QueryAdmission::Sets(QuerySetMask::empty()),
            ..common::client_at(version(row))
        };
        let mut env = Env::reference();
        env.temps = None;
        env.limits = EvalLimits::DEFAULT;
        for (name, value) in [
            ("x", Value::Float(1.5)),
            ("a", Value::Float(-2.0)),
            ("s", Value::string("moo")),
        ] {
            env.vars.set(VariableName::new(name), value);
        }
        tally.check(
            &format!("{FILE} #{index}"),
            &source,
            &compile(&source, &options),
            &env,
            &FuzzRng::Xorshift(Xorshift128::new()),
        );
    }
    report(FILE, &tally);
}

/// Each sweep replays in order on one state, carried from entry to entry as the VM leaves it, at
/// version 13 unless the entry states one.
#[test]
fn vm_equals_tree_walker_on_every_sweep_probe() {
    const FILE: &str = "server_sweeps.json";
    let file = common::data_json(FILE);
    let probes = file["probes"].as_array().expect("probes");
    let runs: Vec<u64> = file["runs"]
        .as_array()
        .expect("runs")
        .iter()
        .map(|run| run["run"].as_u64().expect("run"))
        .collect();
    assert_eq!(
        runs,
        (44..=55).collect::<Vec<u64>>(),
        "{FILE}: the sweep runs"
    );
    let mut tally = Tally::default();
    for &run in &runs {
        let mut env = Env::new();
        let mut rng = FuzzRng::Xorshift(Xorshift128::new());
        for probe in probes
            .iter()
            .filter(|probe| probe["run"].as_u64() == Some(run))
        {
            let source = probe["expr"].as_str().expect("expr");
            let at = if probe.get("version").is_some() {
                version(probe)
            } else {
                13
            };
            let what = format!("run_{run} {}", probe["id"].as_str().expect("id"));
            let compiled = compile(source, &common::server_at(at));
            if let Some(vm) = tally.check(&what, source, &compiled, &env, &rng) {
                env = vm.env;
                rng = vm.rng;
            }
        }
    }
    report(FILE, &tally);
}
