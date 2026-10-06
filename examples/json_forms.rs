//! The serialised forms of a Molang field (`MolangValueRepr`): a number, a bool, a string and the
//! `{"expression": ..., "version": ...}` object, read the four ways this crate reads a field and
//! written back in the form they were read. Needs no features: the crate has no JSON dependency,
//! so the caller converts its JSON value into a `JsonScalar`.
//!
//! Run with `cargo run --example json_forms`.

use molangx::json::{JsonScalar, MolangValueRepr, OtherJson, ReadField, ReaderKind, SourceForm};
use molangx::version::RawVersion;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // The load context's version, taken by a plain string (here 6: engine version 1.18.20).
    let context_version = 6;
    let versioned = ReaderKind::StrictVersioned;

    // The four forms, read as a strict versioned field (a bool is not one of its forms).
    let forms = [
        ("number", JsonScalar::Number(0.5)),
        ("string", JsonScalar::String("q.is_baby ? 1 : 2".into())),
        (
            "object",
            JsonScalar::Object {
                expression: "q.is_baby ? 1 : 2".into(),
                version: RawVersion(4),
            },
        ),
        ("bool", JsonScalar::Bool(true)),
    ];
    println!("strict versioned field (context version {context_version}):");
    for (name, json) in &forms {
        match MolangValueRepr::read(json.clone(), versioned, context_version) {
            Ok(ReadField::Value(value)) => {
                println!("  {name:<6} -> {value:?}");
                // Writing gives back exactly the form that was read.
                assert_eq!(&value.write(), json);
            }
            Ok(field) => println!("  {name:<6} -> {field:?}"),
            Err(error) => println!("  {name:<6} -> error: {error}"),
        }
    }

    // A string takes the version of the load context, the object form keeps its own.
    let read =
        |json: &JsonScalar| match MolangValueRepr::read(json.clone(), versioned, context_version) {
            Ok(ReadField::Value(value)) => Ok(value),
            Ok(field) => unreachable!("a versioned field holds one value, not {field:?}"),
            Err(error) => Err(error),
        };
    let string = read(&forms[1].1)?;
    let object = read(&forms[2].1)?;
    for value in [&string, &object] {
        let source = value.source().expect("an expression");
        let (form, version) = match source.form() {
            SourceForm::String {
                context_version: Some(version),
            } => ("string form", version),
            SourceForm::Object { version } => ("object form", version),
            _ => continue,
        };
        println!(
            "{form}: {:?} at version {version} (effective {:?})",
            source.as_str(),
            source.effective_version()
        );
    }

    // The reader, not the field, decides which JSON types are accepted.
    println!();
    println!("who accepts what:");
    let readers = [
        ReaderKind::StrictVersioned,
        ReaderKind::LenientVersioned,
        ReaderKind::BiomeHeightRange,
        ReaderKind::ScalarOrArray,
        ReaderKind::SchemaValidated,
    ];
    print!("  {:<22}", "");
    for (name, _) in &forms {
        print!("{name:<8}");
    }
    println!();
    for reader in readers {
        print!("  {:<22}", reader.to_string());
        for (_, json) in &forms {
            let accepted = MolangValueRepr::read(json.clone(), reader, context_version).is_ok();
            print!("{:<8}", if accepted { "yes" } else { "no" });
        }
        println!();
    }

    // Constants need no compiler: a number and a bool know their value.
    println!();
    for value in [
        MolangValueRepr::Const(0.5),
        MolangValueRepr::Bool(true),
        string,
    ] {
        println!("{value:?}: constant_value() = {:?}", value.constant_value());
    }

    println!();
    for json in [JsonScalar::Bool(true), JsonScalar::Number(1e39)] {
        println!(
            "{versioned}: {json:?} -> {}",
            MolangValueRepr::read(json.clone(), versioned, context_version).unwrap_err()
        );
    }
    // A scalar-or-array field reads null as no expression and an array element by element.
    for other in [OtherJson::Null, OtherJson::Array] {
        let json = JsonScalar::Other(other);
        let field =
            MolangValueRepr::read(json.clone(), ReaderKind::ScalarOrArray, context_version)?;
        println!("{}: {json:?} -> {field:?}", ReaderKind::ScalarOrArray);
    }
    Ok(())
}
