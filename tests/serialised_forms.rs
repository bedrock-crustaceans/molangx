//! Reading and writing serialised Molang values, and compiling a read source.

#![cfg(feature = "stdlib")]

use molangx::json::{
    JsonScalar, JsonType, MolangSource, MolangValueRepr, OtherJson, ReadField, ReaderKind,
    ReprError, SourceForm,
};
use molangx::version::RawVersion;

const STRICT: ReaderKind = ReaderKind::StrictVersioned;
const LENIENT: ReaderKind = ReaderKind::LenientVersioned;

fn string(text: &str) -> JsonScalar {
    JsonScalar::String(text.into())
}

fn object(text: &str, version: i16) -> JsonScalar {
    JsonScalar::Object {
        expression: text.into(),
        version: RawVersion(version),
    }
}

/// `None` for a string form without a context version.
fn raw(source: &MolangSource) -> Option<i16> {
    source.raw_version().map(i16::from)
}

/// [`MolangValueRepr::read`] of a JSON value that reads as one value or fails.
fn read(
    json: JsonScalar,
    reader: ReaderKind,
    context_version: i16,
) -> Result<MolangValueRepr, ReprError> {
    MolangValueRepr::read(json, reader, context_version).map(|field| match field {
        ReadField::Value(value) => value,
        other => panic!("{other:?} is not one value"),
    })
}

fn is_string_form(source: &MolangSource) -> bool {
    matches!(source.form(), SourceForm::String { .. })
}

mod readers {
    use super::*;

    #[test]
    fn numbers_are_constants_under_every_reader() {
        for reader in [
            STRICT,
            LENIENT,
            ReaderKind::BiomeHeightRange,
            ReaderKind::ScalarOrArray,
            ReaderKind::SchemaValidated,
        ] {
            assert_eq!(
                read(JsonScalar::Number(2.5), reader, 13),
                Ok(MolangValueRepr::Const(2.5)),
                "{reader}"
            );
            assert_eq!(
                read(JsonScalar::Number(-0.0), reader, 13)
                    .unwrap()
                    .number()
                    .map(f32::to_bits),
                Some((-0.0_f32).to_bits()),
                "{reader}"
            );
        }
    }

    #[test]
    fn bools_are_refused_only_by_the_strict_versioned_reader() {
        assert_eq!(
            read(JsonScalar::Bool(true), STRICT, 13),
            Err(ReprError::UnsupportedType {
                reader: STRICT,
                found: JsonType::Bool
            })
        );
        for reader in [
            LENIENT,
            ReaderKind::BiomeHeightRange,
            ReaderKind::ScalarOrArray,
            ReaderKind::SchemaValidated,
        ] {
            assert_eq!(
                read(JsonScalar::Bool(false), reader, 13),
                Ok(MolangValueRepr::Bool(false)),
                "{reader}"
            );
        }
        assert_eq!(MolangValueRepr::Bool(true).constant_value(), Some(1.0));
        assert_eq!(MolangValueRepr::Bool(false).constant_value(), Some(0.0));
        assert_eq!(
            MolangValueRepr::Bool(true).number(),
            None,
            "a bool is not a `Const`"
        );
    }

    #[test]
    fn the_strict_reader_requires_numbers_that_fit_an_f32() {
        assert_eq!(
            read(JsonScalar::Number(1e300), STRICT, 13),
            Err(ReprError::NumberOutOfRange { value: 1e300 })
        );
        assert_eq!(
            read(JsonScalar::Number(f64::from(f32::MAX)), STRICT, 13),
            Ok(MolangValueRepr::Const(f32::MAX))
        );
        for reader in [
            LENIENT,
            ReaderKind::BiomeHeightRange,
            ReaderKind::ScalarOrArray,
            ReaderKind::SchemaValidated,
        ] {
            assert_eq!(
                read(JsonScalar::Number(1e300), reader, 13),
                Ok(MolangValueRepr::Const(f32::INFINITY)),
                "{reader}"
            );
        }
    }

    #[test]
    fn the_strict_reader_refuses_a_nan() {
        let Err(ReprError::NumberOutOfRange { value }) =
            read(JsonScalar::Number(f64::NAN), STRICT, 13)
        else {
            panic!("the strict reader accepted NaN");
        };
        assert!(value.is_nan());
        assert!(
            matches!(MolangValueRepr::Const(f32::NAN).check(STRICT), Err(ReprError::NumberOutOfRange { value }) if value.is_nan())
        );
        for reader in [
            LENIENT,
            ReaderKind::BiomeHeightRange,
            ReaderKind::ScalarOrArray,
            ReaderKind::SchemaValidated,
        ] {
            let read = read(JsonScalar::Number(f64::NAN), reader, 13);
            assert!(
                matches!(read, Ok(MolangValueRepr::Const(x)) if x.is_nan()),
                "{reader}: {read:?}"
            );
            assert_eq!(
                MolangValueRepr::Const(f32::NAN).check(reader),
                Ok(()),
                "{reader}"
            );
        }
    }

    #[test]
    fn the_object_form_is_read_only_by_the_versioned_readers() {
        for reader in [STRICT, LENIENT] {
            let value = read(object("q.is_baby", 42), reader, 6).unwrap();
            assert_eq!(
                value,
                MolangValueRepr::Expr(MolangSource::object("q.is_baby", 42)),
                "{reader}"
            );
        }
        for reader in [
            ReaderKind::BiomeHeightRange,
            ReaderKind::ScalarOrArray,
            ReaderKind::SchemaValidated,
        ] {
            assert_eq!(
                read(object("1", 1), reader, 6),
                Err(ReprError::UnsupportedType {
                    reader,
                    found: JsonType::Object
                })
            );
        }
    }

    #[test]
    fn a_string_takes_the_context_version() {
        for context in [-1_i16, 0, 6, 13, 99] {
            let value = read(string("v.x"), STRICT, context).unwrap();
            let source = value.source().expect("an expression");
            assert_eq!((raw(source), is_string_form(source)), (Some(context), true));
        }
        let mut value = MolangValueRepr::Expr(MolangSource::string_without_context("v.x"));
        value.set_context_version(9);
        assert_eq!(value.source().and_then(raw), Some(9));
        let mut object_form = read(object("v.x", 3), STRICT, 13).unwrap();
        object_form.set_context_version(9);
        assert_eq!(
            object_form.source().and_then(raw),
            Some(3),
            "the object form keeps its own version"
        );
        let mut constant = MolangValueRepr::Const(1.0);
        constant.set_context_version(9);
        assert_eq!(constant, MolangValueRepr::Const(1.0));
    }

    #[test]
    fn null_arrays_and_other_objects() {
        assert_eq!(
            MolangValueRepr::read(
                JsonScalar::Other(OtherJson::Null),
                ReaderKind::ScalarOrArray,
                13
            ),
            Ok(ReadField::NoExpression)
        );
        assert_eq!(
            MolangValueRepr::read(
                JsonScalar::Other(OtherJson::Array),
                ReaderKind::ScalarOrArray,
                13
            ),
            Ok(ReadField::ExpressionArray)
        );
        for reader in [
            STRICT,
            LENIENT,
            ReaderKind::BiomeHeightRange,
            ReaderKind::SchemaValidated,
        ] {
            for (other, found) in [
                (OtherJson::Null, JsonType::Null),
                (OtherJson::Array, JsonType::Array),
                (OtherJson::Object, JsonType::Object),
            ] {
                assert_eq!(
                    read(JsonScalar::Other(other), reader, 13),
                    Err(ReprError::UnsupportedType { reader, found }),
                    "{reader} {found:?}"
                );
            }
        }
        assert_eq!(
            read(
                JsonScalar::Other(OtherJson::Object),
                ReaderKind::ScalarOrArray,
                13
            ),
            Err(ReprError::UnsupportedType {
                reader: ReaderKind::ScalarOrArray,
                found: JsonType::Object
            })
        );
    }

    #[test]
    fn write_gives_back_the_form_that_was_read() {
        for json in [
            JsonScalar::Number(0.5),
            JsonScalar::Bool(true),
            string("q.is_baby ? 1 : 2"),
            object("1 + 2", 42),
            object("v.x", -1),
        ] {
            let value = read(json.clone(), LENIENT, 6).unwrap();
            assert_eq!(value.write(), json);
        }
        // A constant is widened exactly: `0.1` is written as the `f64` of the `f32` it was stored
        // as.
        let tenth = read(JsonScalar::Number(0.1), STRICT, 13).unwrap();
        assert_eq!(tenth.write(), JsonScalar::Number(f64::from(0.1_f32)));
        assert_eq!(tenth.number(), Some(0.1));
    }

    /// `check` is `read` of `write`.
    #[test]
    fn check_asks_the_reader_whether_it_could_have_produced_the_value() {
        let infinity = MolangValueRepr::Const(f32::INFINITY);
        assert_eq!(
            infinity.check(STRICT),
            Err(ReprError::NumberOutOfRange {
                value: f64::INFINITY
            })
        );
        assert_eq!(infinity.check(LENIENT), Ok(()));
        let flag = MolangValueRepr::Bool(true);
        assert_eq!(
            flag.check(STRICT),
            Err(ReprError::UnsupportedType {
                reader: STRICT,
                found: JsonType::Bool
            })
        );
        assert_eq!(flag.check(ReaderKind::SchemaValidated), Ok(()));
        let object_form = MolangValueRepr::Expr(MolangSource::object("1", 1));
        assert_eq!(object_form.check(STRICT), Ok(()));
        assert_eq!(
            object_form.check(ReaderKind::BiomeHeightRange),
            Err(ReprError::UnsupportedType {
                reader: ReaderKind::BiomeHeightRange,
                found: JsonType::Object
            })
        );
        assert_eq!(
            MolangValueRepr::Expr(MolangSource::string("1", 1)).check(ReaderKind::BiomeHeightRange),
            Ok(())
        );
    }

    #[test]
    fn conversions_and_accessors() {
        assert_eq!(MolangValueRepr::from(1.5_f32), MolangValueRepr::Const(1.5));
        let source = MolangSource::object("Q.X", 12);
        assert_eq!(
            (
                source.as_str(),
                raw(&source),
                is_string_form(&source),
                source.is_empty()
            ),
            ("Q.X", Some(12), false, false)
        );
        assert_eq!(
            MolangValueRepr::from(source.clone()).source(),
            Some(&source)
        );
        assert!(MolangSource::string("", 0).is_empty());
        assert_eq!(MolangValueRepr::Const(1.0).source(), None);
    }

    #[test]
    fn the_context_version_replaces_a_string_form_and_never_an_object_form() {
        for context in [-1, 0, 1, 13, 14, i16::MAX] {
            for own in [i16::MIN, -1, 0, 1, 13, 14] {
                let mut object_form = MolangSource::object("1", own);
                object_form.set_context_version(context);
                assert_eq!(
                    raw(&object_form),
                    Some(own),
                    "object form {own}, context {context}"
                );
                let mut string_form = MolangSource::string("1", own);
                string_form.set_context_version(context);
                assert_eq!(
                    raw(&string_form),
                    Some(context),
                    "string form {own}, context {context}"
                );
                let mut without = MolangSource::string_without_context("1");
                without.set_context_version(context);
                assert_eq!(without, MolangSource::string("1", context));
            }
        }
        let mut value = MolangValueRepr::Expr(MolangSource::object("1", 7));
        value.set_context_version(0);
        assert_eq!(value.write(), object("1", 7));
    }

    #[test]
    fn a_bool_is_the_constant_one_or_positive_zero() {
        assert_eq!(
            MolangValueRepr::Bool(false)
                .constant_value()
                .map(f32::to_bits),
            Some(0)
        );
        assert_eq!(
            MolangValueRepr::Bool(true)
                .constant_value()
                .map(f32::to_bits),
            Some(1.0_f32.to_bits())
        );
    }
}

#[cfg(feature = "facet")]
mod facet_json_forms {
    use super::{LENIENT, STRICT, read};
    use facet::Facet;
    use molangx::json::{JsonScalar, MolangSource, MolangValueRepr, ReaderKind, SourceForm};

    #[derive(Facet, Debug, PartialEq)]
    struct Field {
        value: MolangValueRepr,
        source: MolangSource,
    }

    fn value(json: &str) -> MolangValueRepr {
        facet_json::from_str::<MolangValueRepr>(json).unwrap()
    }

    #[test]
    fn reads_number_bool_string_object() {
        assert_eq!(value("2.5"), MolangValueRepr::Const(2.5));
        assert_eq!(value("7"), MolangValueRepr::Const(7.0));
        assert_eq!(value("-3"), MolangValueRepr::Const(-3.0));
        assert_eq!(value("-0.0"), MolangValueRepr::Const(-0.0));
        assert_eq!(value("true"), MolangValueRepr::Bool(true));
        // A numeric string is an expression, not a number.
        assert_eq!(
            value(r#""1""#),
            MolangValueRepr::Expr(MolangSource::string_without_context("1"))
        );
        assert_eq!(
            value(r#""true""#),
            MolangValueRepr::Expr(MolangSource::string_without_context("true"))
        );
        assert_eq!(
            value(r#""q.is_baby""#),
            MolangValueRepr::Expr(MolangSource::string_without_context("q.is_baby"))
        );
        assert_eq!(
            value(r#"{"expression": "1 + 2", "version": 12}"#),
            MolangValueRepr::Expr(MolangSource::object("1 + 2", 12))
        );
    }

    #[test]
    fn rejects_other_types() {
        assert!(facet_json::from_str::<MolangValueRepr>("[0, 0, 0]").is_err());
        assert!(facet_json::from_str::<MolangValueRepr>("null").is_err());
        assert!(facet_json::from_str::<MolangValueRepr>(r#"{"expression": "1"}"#).is_err());
        assert!(
            facet_json::from_str::<MolangValueRepr>(r#"{"expression": "1", "version": 70000}"#)
                .is_err()
        );
        assert!(facet_json::from_str::<MolangSource>("1").is_err());
        assert!(facet_json::from_str::<MolangSource>("true").is_err());
        assert!(facet_json::from_str::<MolangSource>("null").is_err());
    }

    #[test]
    fn rejects_mistyped_members() {
        // `expression` is a string and `version` an i16 integer; nothing is coerced.
        for json in [
            r#"{"expression": 5, "version": 1}"#,
            r#"{"expression": true, "version": 1}"#,
            r#"{"expression": null, "version": 1}"#,
            r#"{"expression": "1", "version": "1"}"#,
            r#"{"expression": "1", "version": 1.0}"#,
            r#"{"expression": "1", "version": 1.5}"#,
            r#"{"expression": "1", "version": null}"#,
            r#"{"expression": "1", "version": true}"#,
            r#"{"expression": "1", "version": [1]}"#,
            r#"{"expression": "1", "version": -32769}"#,
            r#"{"expression": "1", "version": 32768}"#,
        ] {
            assert!(
                facet_json::from_str::<MolangValueRepr>(json).is_err(),
                "{json}"
            );
            assert!(
                facet_json::from_str::<MolangSource>(json).is_err(),
                "{json}"
            );
        }
        assert_eq!(
            value(r#"{"expression": "1", "version": -32768}"#),
            MolangValueRepr::Expr(MolangSource::object("1", i16::MIN))
        );
        assert_eq!(
            value(r#"{"expression": "1", "version": 32767}"#),
            MolangValueRepr::Expr(MolangSource::object("1", i16::MAX))
        );
    }

    #[test]
    fn rejects_numbers_that_are_not_finite_as_f32() {
        // 1e39 would be Const(inf), which facet-json writes as `null`.
        for json in ["1e39", "-1e39", "1e400"] {
            assert!(
                facet_json::from_str::<MolangValueRepr>(json).is_err(),
                "{json}"
            );
        }
        assert_eq!(value("3.4028235e38"), MolangValueRepr::Const(f32::MAX));
        // facet-json refuses what the lenient versioned reader reads as an infinity.
        assert!(facet_json::from_str::<MolangValueRepr>("1e300").is_err());
        assert_eq!(
            read(JsonScalar::Number(1e300), LENIENT, 0),
            Ok(MolangValueRepr::Const(f32::INFINITY))
        );
    }

    #[test]
    fn ignores_extra_members_of_the_object_form() {
        let json = r#"{"expression": "1 + 2", "version": 12, "comment": "x", "extra": [1]}"#;
        assert_eq!(
            value(json),
            MolangValueRepr::Expr(MolangSource::object("1 + 2", 12))
        );
        assert_eq!(
            facet_json::from_str::<MolangSource>(json).unwrap(),
            MolangSource::object("1 + 2", 12)
        );
    }

    #[test]
    fn refuses_to_write_a_non_finite_constant() {
        // An error instead of `null`, which would not read back.
        for x in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            assert!(
                facet_json::to_string(&MolangValueRepr::Const(x)).is_err(),
                "{x}"
            );
        }
        assert_eq!(
            facet_json::to_string(&MolangValueRepr::Const(-2.5)).unwrap(),
            "-2.5"
        );
    }

    #[test]
    fn facet_result_is_checked_against_the_field_reader() {
        assert!(value("true").check(STRICT).is_err());
        assert!(
            value(r#"{"expression": "1", "version": 1}"#)
                .check(ReaderKind::SchemaValidated)
                .is_err()
        );
        assert!(
            value(r#"{"expression": "1", "version": 1}"#)
                .check(STRICT)
                .is_ok()
        );
    }

    #[test]
    fn source_reads_string_and_object() {
        let s = facet_json::from_str::<MolangSource>(r#""Q.X""#).unwrap();
        assert_eq!(s, MolangSource::string_without_context("Q.X"));
        let o = facet_json::from_str::<MolangSource>(r#"{"expression": "q.x", "version": 13}"#)
            .unwrap();
        assert_eq!(o, MolangSource::object("q.x", 13));
        assert_eq!(
            facet_json::to_string(&o).unwrap(),
            r#"{"expression":"q.x","version":13}"#
        );
        assert_eq!(facet_json::to_string(&s).unwrap(), r#""Q.X""#);
    }

    #[test]
    fn round_trips_each_form() {
        // Not `-0.0` and not constants of 2^64 and above: facet-json writes them as integer tokens
        // that read back as +0.0 or as a string.
        for json in [
            "1.5",
            "0.1",
            "-3",
            "1e19",
            "false",
            r#""v.x""#,
            r#"{"expression":"v.x","version":-1}"#,
        ] {
            let v = value(json);
            let written = facet_json::to_string(&v).unwrap();
            let back = value(&written);
            assert_eq!(back, v, "{json} -> {written}");
            if let (MolangValueRepr::Const(a), MolangValueRepr::Const(b)) = (&v, &back) {
                assert_eq!(a.to_bits(), b.to_bits(), "{json} -> {written}");
            }
        }
    }

    #[test]
    fn works_as_a_struct_field() {
        let f: Field = facet_json::from_str(
            r#"{"value": {"expression": "q.x", "version": 4}, "source": "1"}"#,
        )
        .unwrap();
        assert_eq!(
            f.value,
            MolangValueRepr::Expr(MolangSource::object("q.x", 4))
        );
        assert_eq!(
            f.source.form(),
            SourceForm::String {
                context_version: None
            }
        );
        let back: Field = facet_json::from_str(&facet_json::to_string(&f).unwrap()).unwrap();
        assert_eq!(back, f);
    }
}

#[cfg(feature = "compiler")]
mod compiled_sources {
    use super::*;
    use molangx::catalog::Side;
    use molangx::compile::{CompileFailure, CompileOptions, Deviations, compile};
    use molangx::diag::{DiagCode, Severity};
    use molangx::version::RawVersion;

    #[test]
    fn invalid_expression_loads_and_logs_when_compiled() {
        let value = read(JsonScalar::String("1 +* ((".into()), STRICT, 13)
            .expect("the load does not check the parse");
        let source = value.source().expect("an expression");
        let compiled = compile(
            source.as_str(),
            &CompileOptions::for_source(molangx::stdlib::queries(Side::Server).clone(), source)
                .expect("a source with a version"),
        );
        assert_eq!(compiled.failure(), Some(CompileFailure::Rejected));
        assert!(
            compiled
                .diagnostics()
                .iter()
                .any(|d| d.language_message().is_some() && d.severity() == Severity::Error),
            "{:?}",
            compiled.diagnostics()
        );
        assert_eq!(value.write(), JsonScalar::String("1 +* ((".into()));
    }

    /// An object form at version −1 compiles with only an `Info` diagnostic, none with
    /// `Deviations::NONE`.
    #[test]
    fn object_form_at_minus_one_compiles_at_minus_one() {
        let json = JsonScalar::Object {
            expression: "v.x + 1".into(),
            version: RawVersion(-1),
        };
        let value = read(json, STRICT, 13).expect("no range check on the version");
        let source = value.source().expect("an expression");
        let options =
            CompileOptions::for_source(molangx::stdlib::queries(Side::Server).clone(), source)
                .expect("a source with a version");
        assert_eq!(options.raw_version, RawVersion(-1));
        let compiled = compile(source.as_str(), &options);
        assert_eq!(compiled.failure(), None);
        assert!(compiled.parses_cleanly());
        let ours: Vec<_> = compiled
            .diagnostics()
            .iter()
            .map(|d| (d.code(), d.severity(), d.language_message().is_some()))
            .collect();
        assert_eq!(ours, [(DiagCode::InvalidVersion, Severity::Info, false)]);
        let no_deviations = compile(
            source.as_str(),
            &CompileOptions {
                deviations: Deviations::NONE,
                ..options
            },
        );
        assert_eq!(no_deviations.failure(), None);
        assert!(
            no_deviations.diagnostics().is_empty(),
            "{:?}",
            no_deviations.diagnostics()
        );
    }
}

mod readers_matrix {
    use std::sync::Arc;

    use super::{LENIENT, STRICT, object, raw, read, string};
    use molangx::json::{
        JsonScalar, JsonType, MolangSource, MolangValueRepr, OtherJson, ReadField, ReaderKind,
        ReprError, SourceForm,
    };
    use molangx::version::{MolangVersion, RawVersion};

    const ALL_READERS: [ReaderKind; 5] = [
        STRICT,
        LENIENT,
        ReaderKind::BiomeHeightRange,
        ReaderKind::ScalarOrArray,
        ReaderKind::SchemaValidated,
    ];

    fn unsupported(reader: ReaderKind, found: JsonType) -> Result<MolangValueRepr, ReprError> {
        Err(ReprError::UnsupportedType { reader, found })
    }

    #[test]
    fn value_repr_has_three_arms() {
        // `#[non_exhaustive]` requires the wildcard.
        let describe = |v: &MolangValueRepr| match v {
            MolangValueRepr::Const(_) => "const",
            MolangValueRepr::Bool(_) => "bool",
            MolangValueRepr::Expr(_) => "expr",
            _ => "other",
        };
        assert_eq!(describe(&MolangValueRepr::Const(1.5)), "const");
        assert_eq!(describe(&MolangValueRepr::Bool(true)), "bool");
        assert_eq!(
            describe(&MolangValueRepr::Expr(MolangSource::string("1", 0))),
            "expr"
        );
    }

    #[test]
    fn source_keeps_text_raw_version_and_form() {
        let src = MolangSource::object(Arc::from("Query.Is_Baby ? 1 : 0"), 300);
        assert_eq!(src.as_str(), "Query.Is_Baby ? 1 : 0", "case preserved");
        assert_eq!(raw(&src), Some(300), "raw, not clamped");
        assert_eq!(src.effective_version(), MolangVersion::LATEST);
        assert_eq!(
            MolangSource::object("x", -7).effective_version(),
            MolangVersion::Invalid
        );
        assert_eq!(
            MolangSource::string("x", 5).form(),
            SourceForm::String {
                context_version: Some(RawVersion(5))
            }
        );
        assert_eq!(
            MolangSource::object("x", 5).form(),
            SourceForm::Object {
                version: RawVersion(5)
            }
        );
    }

    #[test]
    fn versioned_accepts_number_string_and_object() {
        for reader in [STRICT, LENIENT] {
            assert_eq!(
                read(JsonScalar::Number(2.5), reader, 6),
                Ok(MolangValueRepr::Const(2.5))
            );
            assert_eq!(
                read(JsonScalar::Number(-3.0), reader, 6),
                Ok(MolangValueRepr::Const(-3.0))
            );
            assert_eq!(
                read(string("v.x + 1"), reader, 6),
                Ok(MolangValueRepr::Expr(MolangSource::string("v.x + 1", 6)))
            );
            assert_eq!(
                read(object("v.x + 1", 12), reader, 6),
                Ok(MolangValueRepr::Expr(MolangSource::object("v.x + 1", 12)))
            );
        }
    }

    #[test]
    fn string_takes_the_context_version_object_keeps_its_own() {
        for ctx in [-1, 0, 4, 13] {
            let s = read(string("1"), STRICT, ctx).unwrap();
            assert_eq!(raw(s.source().unwrap()), Some(ctx));
            let o = read(object("1", 9), STRICT, ctx).unwrap();
            assert_eq!(raw(o.source().unwrap()), Some(9));
        }
    }

    #[test]
    fn object_version_is_unconstrained() {
        // The gates see the effective version; the raw value is kept.
        for (version, effective) in [
            (i16::MIN, MolangVersion::Invalid),
            (-2, MolangVersion::Invalid),
            (-1, MolangVersion::Invalid),
            (0, MolangVersion::V0),
            (3, MolangVersion::V3),
            (13, MolangVersion::V13),
            (14, MolangVersion::V13),
            (1000, MolangVersion::V13),
            (i16::MAX, MolangVersion::V13),
        ] {
            let v = read(object("q.x", version), STRICT, 0).unwrap();
            let src = v.source().unwrap();
            assert_eq!(src.raw_version(), Some(RawVersion(version)));
            assert_eq!(src.effective_version(), effective, "{version}");
        }
    }

    #[test]
    fn bool_only_under_the_lenient_versioned_reader() {
        assert_eq!(
            read(JsonScalar::Bool(true), STRICT, 0),
            unsupported(STRICT, JsonType::Bool)
        );
        assert_eq!(
            read(JsonScalar::Bool(false), STRICT, 0),
            unsupported(STRICT, JsonType::Bool)
        );
        let t = read(JsonScalar::Bool(true), LENIENT, 0).unwrap();
        let f = read(JsonScalar::Bool(false), LENIENT, 0).unwrap();
        assert_eq!(t, MolangValueRepr::Bool(true));
        assert_eq!(f, MolangValueRepr::Bool(false));
        assert_eq!(t.constant_value(), Some(1.0));
        assert_eq!(f.constant_value(), Some(0.0));
    }

    #[test]
    fn strict_versioned_number_must_fit_f32() {
        let max = f64::from(f32::MAX);
        assert_eq!(
            read(JsonScalar::Number(max), STRICT, 0),
            Ok(MolangValueRepr::Const(f32::MAX))
        );
        assert_eq!(
            read(JsonScalar::Number(-max), STRICT, 0),
            Ok(MolangValueRepr::Const(f32::MIN))
        );
        assert_eq!(
            read(JsonScalar::Number(1e39), STRICT, 0),
            Err(ReprError::NumberOutOfRange { value: 1e39 })
        );
        assert_eq!(
            read(JsonScalar::Number(-1e39), STRICT, 0),
            Err(ReprError::NumberOutOfRange { value: -1e39 })
        );
        assert_eq!(
            read(JsonScalar::Number(1e39), LENIENT, 0),
            Ok(MolangValueRepr::Const(f32::INFINITY))
        );
    }

    #[test]
    fn versioned_rejects_wrong_types() {
        for reader in [STRICT, LENIENT] {
            for found in [OtherJson::Array, OtherJson::Null, OtherJson::Object] {
                assert_eq!(
                    read(JsonScalar::Other(found), reader, 0),
                    unsupported(reader, found.into())
                );
            }
        }
    }

    #[test]
    fn empty_expression_reads_and_is_reported_empty() {
        let v = read(object("", 7), STRICT, 0).unwrap();
        assert!(v.source().unwrap().is_empty());
        assert_eq!(v.write(), object("", 7));
        let s = read(string(""), STRICT, 4).unwrap();
        assert!(s.source().unwrap().is_empty());
        assert_eq!(s.write(), string(""));
    }

    #[test]
    fn invalid_molang_still_reads() {
        let v = read(string("query.no_such_query"), STRICT, 13).unwrap();
        assert_eq!(v.source().unwrap().as_str(), "query.no_such_query");
        let v = read(string("1 +* (("), STRICT, 13).unwrap();
        assert_eq!(v.source().unwrap().as_str(), "1 +* ((");
    }

    #[test]
    fn biome_height_range_accepts_number_bool_string_only() {
        let r = ReaderKind::BiomeHeightRange;
        assert_eq!(
            read(JsonScalar::Number(64.0), r, 1),
            Ok(MolangValueRepr::Const(64.0))
        );
        assert_eq!(
            read(JsonScalar::Bool(true), r, 1),
            Ok(MolangValueRepr::Bool(true))
        );
        assert_eq!(
            read(string("q.noise(1, 2)"), r, 1),
            Ok(MolangValueRepr::Expr(MolangSource::string(
                "q.noise(1, 2)",
                1
            )))
        );
        assert_eq!(read(object("1", 1), r, 1), unsupported(r, JsonType::Object));
        for found in [OtherJson::Null, OtherJson::Array, OtherJson::Object] {
            assert_eq!(
                read(JsonScalar::Other(found), r, 1),
                unsupported(r, found.into())
            );
        }
    }

    #[test]
    fn scalar_or_array_reader() {
        let r = ReaderKind::ScalarOrArray;
        assert_eq!(
            read(JsonScalar::Number(0.25), r, 8),
            Ok(MolangValueRepr::Const(0.25))
        );
        assert_eq!(
            read(JsonScalar::Bool(false), r, 8),
            Ok(MolangValueRepr::Bool(false))
        );
        assert_eq!(
            read(string("1"), r, 8),
            Ok(MolangValueRepr::Expr(MolangSource::string("1", 8)))
        );
        assert_eq!(
            MolangValueRepr::read(JsonScalar::Other(OtherJson::Null), r, 8),
            Ok(ReadField::NoExpression)
        );
        assert_eq!(
            MolangValueRepr::read(JsonScalar::Other(OtherJson::Array), r, 8),
            Ok(ReadField::ExpressionArray)
        );
        assert_eq!(
            read(JsonScalar::Other(OtherJson::Object), r, 8),
            unsupported(r, JsonType::Object)
        );
        assert_eq!(read(object("1", 8), r, 8), unsupported(r, JsonType::Object));
    }

    #[test]
    fn schema_validated_reader() {
        let r = ReaderKind::SchemaValidated;
        assert_eq!(
            read(JsonScalar::Number(3.0), r, 10),
            Ok(MolangValueRepr::Const(3.0))
        );
        assert_eq!(
            read(JsonScalar::Bool(true), r, 10),
            Ok(MolangValueRepr::Bool(true))
        );
        assert_eq!(
            read(string("1"), r, 10),
            Ok(MolangValueRepr::Expr(MolangSource::string("1", 10)))
        );
        assert_eq!(
            read(object("1", 10), r, 10),
            unsupported(r, JsonType::Object)
        );
        for found in [OtherJson::Null, OtherJson::Array, OtherJson::Object] {
            assert_eq!(
                read(JsonScalar::Other(found), r, 10),
                unsupported(r, found.into())
            );
        }
    }

    #[test]
    fn only_versioned_has_the_object_form() {
        for reader in ALL_READERS {
            let accepted = read(object("1", 3), reader, 0).is_ok();
            assert_eq!(
                accepted,
                matches!(
                    reader,
                    ReaderKind::StrictVersioned | ReaderKind::LenientVersioned
                ),
                "{reader}"
            );
        }
    }

    #[test]
    fn write_emits_the_form_that_was_read() {
        let inputs = [
            JsonScalar::Number(0.0),
            JsonScalar::Number(-0.0),
            JsonScalar::Number(1.5),
            JsonScalar::Number(f64::from(f32::MAX)),
            JsonScalar::Bool(true),
            JsonScalar::Bool(false),
            string("Variable.X = 1; return v.x;"),
            string(""),
            object("math.sin(q.anim_time)", 0),
            object("1", -1),
            object("1", 14),
            object("1", i16::MIN),
        ];
        for json in inputs {
            let mut accepted_by_any = false;
            for reader in ALL_READERS {
                let Ok(v) = read(json.clone(), reader, 6) else {
                    continue;
                };
                accepted_by_any = true;
                let written = v.write();
                assert_eq!(written, json, "{reader}");
                if let JsonScalar::Number(x) = json {
                    let JsonScalar::Number(y) = written else {
                        unreachable!()
                    };
                    assert_eq!(x.to_bits(), y.to_bits());
                }
                assert_eq!(read(written, reader, 6), Ok(v), "{reader}");
            }
            assert!(accepted_by_any, "{json:?}");
        }
    }

    #[test]
    fn number_round_trip_is_through_f32() {
        let v = read(JsonScalar::Number(0.1), STRICT, 0).unwrap();
        assert_eq!(v, MolangValueRepr::Const(0.1_f32));
        assert_eq!(v.write(), JsonScalar::Number(f64::from(0.1_f32)));
    }

    #[test]
    fn write_widens_the_f32_exactly() {
        // `write` gives the f32's exact value, not the shortest decimal.
        let v = read(JsonScalar::Number(0.1), STRICT, 0).unwrap();
        assert_eq!(v.write(), JsonScalar::Number(0.100_000_001_490_116_12));
        assert_eq!(v.number().map(|x| x.to_string()).as_deref(), Some("0.1"));
        assert_eq!(MolangValueRepr::Bool(true).number(), None);
        assert_eq!(
            MolangValueRepr::Expr(MolangSource::string("1", 0)).number(),
            None
        );
    }

    #[test]
    fn non_finite_constants_write_as_non_finite_numbers() {
        // JSON has no such number; the caller decides.
        let JsonScalar::Number(x) = MolangValueRepr::Const(f32::NAN).write() else {
            unreachable!()
        };
        assert!(x.is_nan());
        assert_eq!(
            MolangValueRepr::Const(f32::NEG_INFINITY).write(),
            JsonScalar::Number(f64::NEG_INFINITY)
        );
    }

    #[test]
    fn check_matches_read_for_every_reader() {
        let values = [
            MolangValueRepr::Const(1.5),
            MolangValueRepr::Const(f32::MAX),
            MolangValueRepr::Const(f32::INFINITY),
            MolangValueRepr::Const(f32::NEG_INFINITY),
            MolangValueRepr::Bool(true),
            MolangValueRepr::Bool(false),
            MolangValueRepr::Expr(MolangSource::string_without_context("q.x")),
            MolangValueRepr::Expr(MolangSource::object("q.x", 14)),
        ];
        for v in &values {
            for reader in ALL_READERS {
                assert_eq!(
                    v.check(reader),
                    read(v.write(), reader, 0).map(drop),
                    "{v:?} {reader}"
                );
            }
        }
        let obj = MolangValueRepr::Expr(MolangSource::object("1", 3));
        for reader in ALL_READERS {
            let versioned = matches!(
                reader,
                ReaderKind::StrictVersioned | ReaderKind::LenientVersioned
            );
            assert_eq!(obj.check(reader).is_ok(), versioned, "{reader}");
            assert_eq!(
                MolangValueRepr::Bool(true).check(reader).is_ok(),
                reader != STRICT,
                "{reader}"
            );
            assert_eq!(
                MolangValueRepr::Const(f32::INFINITY).check(reader).is_ok(),
                reader != STRICT,
                "{reader}"
            );
            assert_eq!(
                MolangValueRepr::Expr(MolangSource::string("1", 0)).check(reader),
                Ok(()),
                "{reader}"
            );
        }
        assert_eq!(
            MolangValueRepr::Bool(false).check(STRICT),
            Err(ReprError::UnsupportedType {
                reader: STRICT,
                found: JsonType::Bool
            })
        );
        assert_eq!(
            MolangValueRepr::Const(f32::INFINITY).check(STRICT),
            Err(ReprError::NumberOutOfRange {
                value: f64::INFINITY
            })
        );
        assert_eq!(
            obj.check(ReaderKind::ScalarOrArray),
            Err(ReprError::UnsupportedType {
                reader: ReaderKind::ScalarOrArray,
                found: JsonType::Object
            })
        );
    }

    #[test]
    fn context_version_applies_to_string_form_only() {
        let mut s = MolangValueRepr::Expr(MolangSource::string_without_context("1"));
        assert_eq!(
            s.source().unwrap().effective_version(),
            MolangVersion::Invalid
        );
        assert_eq!(raw(s.source().unwrap()), None);
        s.set_context_version(11);
        assert_eq!(raw(s.source().unwrap()), Some(11));
        let mut o = MolangValueRepr::Expr(MolangSource::object("1", 4));
        o.set_context_version(11);
        assert_eq!(raw(o.source().unwrap()), Some(4));
        let mut c = MolangValueRepr::Const(2.0);
        c.set_context_version(11);
        assert_eq!(c, MolangValueRepr::Const(2.0));
    }

    #[test]
    fn accessors() {
        assert_eq!(MolangValueRepr::Const(2.0).constant_value(), Some(2.0));
        assert_eq!(MolangValueRepr::from(2.0).source(), None);
        let e = MolangValueRepr::from(MolangSource::string("1", 0));
        assert_eq!(e.constant_value(), None);
        assert_eq!(e.source(), Some(&MolangSource::string("1", 0)));
        assert_eq!(object("x", 1).json_type(), JsonType::Object);
        assert_eq!(
            JsonScalar::Other(OtherJson::Null).json_type(),
            JsonType::Null
        );
        assert_eq!(string("x").json_type(), JsonType::String);
    }

    #[test]
    fn errors_display() {
        let e = ReprError::UnsupportedType {
            reader: STRICT,
            found: JsonType::Bool,
        };
        assert_eq!(
            e.to_string(),
            "a strict versioned Molang field does not accept a JSON bool"
        );
        assert_eq!(
            ReprError::NumberOutOfRange { value: 1e39 }.to_string(),
            "the number 1e39 does not fit an f32"
        );
        // Scientific notation, not a 301-digit decimal.
        assert_eq!(
            ReprError::NumberOutOfRange { value: -1e300 }.to_string(),
            "the number -1e300 does not fit an f32"
        );
        assert_eq!(
            ReprError::NumberOutOfRange { value: 3.5e38 }.to_string(),
            "the number 3.5e38 does not fit an f32"
        );
    }

    #[test]
    fn other_json_types() {
        assert_eq!(
            JsonScalar::Other(OtherJson::Null).json_type(),
            JsonType::Null
        );
        assert_eq!(
            JsonScalar::Other(OtherJson::Array).json_type(),
            JsonType::Array
        );
        assert_eq!(
            JsonScalar::Other(OtherJson::Object).json_type(),
            JsonType::Object
        );
        assert_eq!(OtherJson::Array.to_string(), "array");
    }
}
