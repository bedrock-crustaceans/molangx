//! Shared helpers of the fuzz crate's tests: the VM-against-tree-walker comparison, the reference
//! catalogue and the main crate's data files.

#![allow(dead_code, reason = "each test file uses a part of the helpers")]

use std::path::PathBuf;

use molangx::catalog::{QueryCatalog, Side};
use molangx::compile::CompileOptions;
use molangx::version::RawVersion;
use serde_json::Value;

pub mod differential;
#[path = "../../../tests/common/host.rs"]
pub mod host;

pub use molangx::internals::reference_catalog::{EXPERIMENTAL_TEST, GET_NAME_TEST, SUM_TEST};

/// The client's built-in queries plus the seven helper queries.
pub fn reference_catalog() -> &'static QueryCatalog {
    molangx::internals::reference_catalog::catalog()
}

pub fn client_at(version: i16) -> CompileOptions {
    CompileOptions::from_raw_version(
        molangx::stdlib::queries(Side::Client).clone(),
        RawVersion(version),
    )
}

pub fn server_at(version: i16) -> CompileOptions {
    CompileOptions::from_raw_version(
        molangx::stdlib::queries(Side::Server).clone(),
        RawVersion(version),
    )
}

/// Parses the main crate's data file `tests/<name>`.
pub fn data_json(name: &str) -> Value {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("tests")
        .join(name);
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()));
    let value: Value = serde_json::from_str(&text)
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

pub fn hex_to_string(hex: &str) -> String {
    let bytes: Vec<u8> = (0..hex.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).expect("hex"))
        .collect();
    String::from_utf8(bytes).expect("the inputs are UTF-8")
}
