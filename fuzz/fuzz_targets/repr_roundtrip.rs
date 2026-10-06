//! Round trips of the serialised forms:
//! - `MolangValueRepr` → `JsonScalar` → back is the identity under every reader that accepts the
//!   value, and `check` agrees with `read` of `write`;
//! - a `JsonScalar` read by any reader never panics, and a value it reads reads back the same.
//!
//! Floats compare by bits with NaN normalised: an `f32` NaN may change payload through `f64`.
//!
//! ```text
//! cargo +nightly fuzz run repr_roundtrip -- -max_total_time=600
//! ```

#![no_main]

use libfuzzer_sys::fuzz_target;
use molangx::json::{MolangValueRepr, ReadField, SourceForm};
use molangx_fuzz::generator::{READERS, ReprCase, normalised_bits};

fn same(a: &MolangValueRepr, b: &MolangValueRepr) -> bool {
    match (a, b) {
        (MolangValueRepr::Const(x), MolangValueRepr::Const(y)) => {
            normalised_bits(*x) == normalised_bits(*y)
        }
        _ => a == b,
    }
}

fuzz_target!(|input: ReprCase| {
    let ReprCase {
        value,
        json,
        context,
    } = input;

    // A string reads back at the context version: give it its own, or the context's when it has
    // none.
    let own_version = match &value {
        MolangValueRepr::Expr(source) if matches!(source.form(), SourceForm::String { .. }) => {
            source.raw_version().map_or(context, i16::from)
        }
        _ => context,
    };
    for reader in READERS {
        let read = MolangValueRepr::read(value.write(), reader, own_version);
        assert_eq!(
            value.check(reader).is_ok(),
            read.is_ok(),
            "check and read disagree for {value:?} under {reader}"
        );
        if let Ok(ReadField::Value(back)) = read {
            // A string without its context version reads back with the one the reader applied.
            let mut expected = value.clone();
            expected.set_context_version(own_version);
            assert!(
                same(&expected, &back),
                "{value:?} under {reader} reads back as {back:?}"
            );
        }
    }

    for reader in READERS {
        if let Ok(ReadField::Value(first)) = MolangValueRepr::read(json.clone(), reader, context) {
            let second = MolangValueRepr::read(first.write(), reader, context);
            assert!(
                matches!(&second, Ok(ReadField::Value(second)) if same(&first, second)),
                "{json:?} under {reader}: {first:?}, then {second:?}"
            );
        }
    }
});
