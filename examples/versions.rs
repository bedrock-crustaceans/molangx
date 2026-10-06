//! Molang versions: how a pack's `min_engine_version` becomes a `MolangVersion`, and how one text
//! is grouped differently at versions 4, 5 and 6.
//!
//! Run with `cargo run --example versions --features compiler`.

use molangx::compile::{CompileOptions, Expr, compile};
use molangx::version::MolangVersion;

fn main() {
    println!("engine version -> MolangVersion");
    for text in [
        "1.16.100",
        "1.17.0",
        "1.17.40",
        "1.18.10",
        "1.18.20",
        "1.21.100",
        "1.21.100-beta",
        "*",
        "1.21",
    ] {
        // A text that is not an engine version ("1.21") is MolangVersion::Invalid, -1.
        let version = MolangVersion::from_engine_version_str(text);
        println!("  {text:<14} -> {version:>2} ({version:?})");
    }

    // The way back: the first engine version of a Molang version. No engine version maps to 3, so
    // 3 lists 1.17.40, the first engine version of 4.
    let first = MolangVersion::V3.first_engine_version().expect("3 has one");
    let back = MolangVersion::from(first);
    println!("version 3 is listed at {first}, which maps to version {back}");

    // One text, three groupings:
    //   below 5 the ternary is left-associative; from 5 it is right-associative;
    //   below 6 `||` binds tighter than `&&`; from 6 `&&` binds tighter (as in C).
    println!();
    for text in ["1 ? 0 : 1 ? 2 : 3", "1 || 0 && 0"] {
        println!("{text}");
        for version in [MolangVersion::V4, MolangVersion::V5, MolangVersion::V6] {
            let compiled = compile(text, &CompileOptions::server(version));
            let value = compiled.expr().and_then(Expr::as_constant);
            println!("  version {version:>2}: {value:?}");
        }
    }

    println!();
    for v in [MolangVersion::V4, MolangVersion::V5, MolangVersion::V6] {
        println!(
            "version {v}: right-associative ternary {:<5}  C-like && / || precedence {}",
            v.right_assoc_ternary(),
            v.c_like_logic_precedence()
        );
    }
}
