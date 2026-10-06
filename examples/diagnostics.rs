//! Diagnostics: which errors reject an expression and which are only logged, the byte span each one
//! points at, and the crate's own lints next to them.
//!
//! A rejected expression evaluates to the constant 0; a kept one compiles with its messages as
//! warnings.
//!
//! Run with `cargo run --example diagnostics --features compiler`.

use molangx::compile::{CompileOptions, CompileOutcome, Deviations, compile};
use molangx::ops::OpSet;
use molangx::version::MolangVersion;

fn show(label: &str, source: &str, options: &CompileOptions) {
    let compiled = compile(source, options);
    println!("{label}: {source:?} at version {}", options.version());
    let outcome = match compiled.outcome() {
        CompileOutcome::Success(_) => "compiled",
        CompileOutcome::Rejected(_) => "rejected, evaluates to 0",
        CompileOutcome::UsesArrays => "needs array.* resolved before it can be linked",
        CompileOutcome::UsesResources => {
            "needs geometry./material./texture. resolved before it can be linked"
        }
        // `CompileOutcome` is `#[non_exhaustive]`: a later version may add outcomes.
        _ => "another outcome",
    };
    println!(
        "  result: {outcome}; parses_cleanly() = {}",
        compiled.parses_cleanly()
    );
    for d in compiled.diagnostics() {
        let span = d.span().start as usize..d.span().end as usize;
        // `language_message` is Some for a Molang error message, None for this crate's own lint.
        let origin = d
            .language_message()
            .map_or("lint".to_owned(), |m| format!("message {}", m.row()));
        println!(
            "  {:?} {:?} [{origin}] bytes {span:?} = {:?}",
            d.severity(),
            d.code(),
            source.get(span.clone()).unwrap_or("")
        );
        for line in d.message().lines() {
            println!("      {line}");
        }
    }
    println!();
}

fn main() {
    let latest = CompileOptions::server(MolangVersion::LATEST);

    show("reject (lexer)", "1 + $", &latest);
    show("reject (tree)", "1 +", &latest);
    show("reject (statement)", "v.x = 1", &latest);
    show("reject (query)", "query.does_not_exist", &latest);

    // Kept: logged, but the expression still compiles and runs.
    show("keep", "1e", &latest);

    // The same text can be silent below a version gate: an empty expression is logged from
    // version 4.
    for version in [MolangVersion::V3, MolangVersion::V4] {
        show("version gate", "", &CompileOptions::server(version));
    }

    let read_only = CompileOptions {
        allowed_ops: OpSet::all().without_assignments(),
        ..latest.clone()
    };
    show("reject (allow-list)", "v.x = 1;", &read_only);

    // The crate's own lints (`language_message: None`) only warn or inform and never reject a valid
    // text; `Deviations::NONE` switches them off.
    let no_deviations = CompileOptions {
        deviations: Deviations::NONE,
        ..latest.clone()
    };
    show("lint (arity)", "query.armor_color_slot(1)", &latest);
    show(
        "lint (arity, no deviations)",
        "query.armor_color_slot(1)",
        &no_deviations,
    );
    show("lint (client-only)", "query.is_first_person", &latest);
    show(
        "lint (client side)",
        "query.is_first_person",
        &CompileOptions::client(MolangVersion::LATEST),
    );
}
