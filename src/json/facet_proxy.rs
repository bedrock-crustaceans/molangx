//! The `facet` impls: each type goes through a proxy over one untagged enum of JSON shapes, and
//! the conversion rejects the shapes the type does not take.
//!
//! A plain string becomes [`MolangSource::string_without_context`]: a deserializer has no load
//! context.

use super::{JsonType, MolangSource, MolangValueRepr, SourceForm};

/// The versioned object form. The members are read as any JSON scalar and checked in the
/// conversion, because typed members let the deserializer coerce `5` into `"5"`, `"1"` or `1.0`
/// into `1`, and `null` into `""` or `0`.
#[derive(Debug, facet::Facet)]
pub struct ObjectForm {
    expression: Member,
    version: Member,
}

/// One member of the object form, by JSON type; integers have their own variants as in
/// [`JsonForm`].
#[derive(Debug, facet::Facet)]
#[facet(untagged)]
#[repr(u8)]
#[allow(dead_code, reason = "the bool and float payloads exist to be rejected")]
pub enum Member {
    Null,
    String(String),
    Bool(bool),
    UInt(u64),
    Int(i64),
    Number(f64),
}

/// Every JSON shape a deserializer can offer.
///
/// The variants a type does not accept exist so the conversion can reject them: without them the
/// untagged solver of `facet-json` coerces `null` into a number and a number into a string.
/// Integers have their own variants because the solver matches an integer token only against
/// integer types; `String` comes first so `"1"` is never parsed as a number.
#[derive(Debug, facet::Facet)]
#[facet(untagged)]
#[repr(u8)]
pub enum JsonForm {
    Null,
    String(String),
    Bool(bool),
    UInt(u64),
    Int(i64),
    Number(f64),
    Object(ObjectForm),
}

impl Member {
    fn json_type(&self) -> JsonType {
        match self {
            Self::Null => JsonType::Null,
            Self::String(_) => JsonType::String,
            Self::Bool(_) => JsonType::Bool,
            Self::UInt(_) | Self::Int(_) | Self::Number(_) => JsonType::Number,
        }
    }
}

impl ObjectForm {
    /// `version` must be an integer token, so `1.0` is rejected.
    fn into_parts(self) -> Result<(String, i16), Rejected> {
        let expression = match self.expression {
            Member::String(s) => s,
            other => return Err(Rejected::Member("expression", other.json_type())),
        };
        let version = match self.version {
            Member::UInt(v) => i16::try_from(v).map_err(|_| Rejected::VersionRange)?,
            Member::Int(v) => i16::try_from(v).map_err(|_| Rejected::VersionRange)?,
            Member::Number(_) => return Err(Rejected::VersionNotInteger),
            other => return Err(Rejected::Member("version", other.json_type())),
        };
        Ok((expression, version))
    }
}

/// Why the `facet` impl of a Molang type rejected a JSON value, or could not write one.
#[derive(thiserror::Error, Debug)]
pub enum Rejected {
    /// The value has a JSON type the Molang type does not take.
    #[error("a Molang value cannot be a JSON {0}")]
    Type(JsonType),
    /// A member of the object form has the wrong JSON type.
    #[error("the `{0}` member of a Molang object cannot be a JSON {1}")]
    Member(&'static str, JsonType),
    /// The object form's `version` is a float token.
    #[error("the `version` member of a Molang object must be an integer")]
    VersionNotInteger,
    /// The object form's `version` is an integer outside `i16`.
    #[error("the `version` member of a Molang object does not fit an i16")]
    VersionRange,
    /// A number that is not finite as an `f32` (read: out of range; write: a `Const`
    /// holding an infinity or NaN).
    #[error("a Molang constant must be a finite f32")]
    NonFinite,
}

/// [`MolangSource`] as JSON: a string or the object form.
#[derive(Debug, facet::Facet)]
#[facet(transparent)]
pub struct SourceProxy(JsonForm);

/// [`MolangValueRepr`] as JSON: a number, a bool, a string or the object form.
#[derive(Debug, facet::Facet)]
#[facet(transparent)]
pub struct ValueProxy(JsonForm);

impl TryFrom<SourceProxy> for MolangSource {
    type Error = Rejected;

    fn try_from(proxy: SourceProxy) -> Result<Self, Rejected> {
        match proxy.0 {
            JsonForm::String(src) => Ok(MolangSource::string_without_context(src)),
            JsonForm::Object(object) => object
                .into_parts()
                .map(|(expression, version)| MolangSource::object(expression, version)),
            JsonForm::Null => Err(Rejected::Type(JsonType::Null)),
            JsonForm::Bool(_) => Err(Rejected::Type(JsonType::Bool)),
            JsonForm::UInt(_) | JsonForm::Int(_) | JsonForm::Number(_) => {
                Err(Rejected::Type(JsonType::Number))
            }
        }
    }
}

impl From<&MolangSource> for SourceProxy {
    fn from(src: &MolangSource) -> Self {
        Self(match src.form {
            SourceForm::String { .. } => JsonForm::String(src.text.to_string()),
            SourceForm::Object { version } => JsonForm::Object(ObjectForm {
                expression: Member::String(src.text.to_string()),
                version: Member::Int(i64::from(version.0)),
            }),
        })
    }
}

impl TryFrom<ValueProxy> for MolangValueRepr {
    type Error = Rejected;

    fn try_from(proxy: ValueProxy) -> Result<Self, Rejected> {
        match proxy.0 {
            JsonForm::UInt(x) => constant(x as f32),
            JsonForm::Int(x) => constant(x as f32),
            JsonForm::Number(x) => constant(x as f32),
            JsonForm::Bool(b) => Ok(MolangValueRepr::Bool(b)),
            JsonForm::Null => Err(Rejected::Type(JsonType::Null)),
            form => MolangSource::try_from(SourceProxy(form)).map(MolangValueRepr::Expr),
        }
    }
}

/// A number token as a `Const`; one whose `f32` is not finite (`1e39`) is rejected: it has no JSON
/// form to write back.
fn constant(x: f32) -> Result<MolangValueRepr, Rejected> {
    if x.is_finite() {
        Ok(MolangValueRepr::Const(x))
    } else {
        Err(Rejected::NonFinite)
    }
}

/// Writing fails for a `Const` that is not finite: JSON has no infinity or NaN, and `facet-json`
/// would write `null`, which does not read back.
impl TryFrom<&MolangValueRepr> for ValueProxy {
    type Error = Rejected;

    fn try_from(value: &MolangValueRepr) -> Result<Self, Rejected> {
        Ok(Self(match value {
            MolangValueRepr::Const(x) if !x.is_finite() => return Err(Rejected::NonFinite),
            MolangValueRepr::Const(x) => JsonForm::Number(f64::from(*x)),
            MolangValueRepr::Bool(b) => JsonForm::Bool(*b),
            MolangValueRepr::Expr(src) => SourceProxy::from(src).0,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(s: &str) -> Member {
        Member::String(s.to_owned())
    }

    fn object_form(expression: Member, version: Member) -> ObjectForm {
        ObjectForm {
            expression,
            version,
        }
    }

    fn read(form: JsonForm) -> Result<MolangValueRepr, Rejected> {
        MolangValueRepr::try_from(ValueProxy(form))
    }

    fn reads_as(form: JsonForm, expected: f32) -> bool {
        matches!(read(form), Ok(MolangValueRepr::Const(x)) if x.to_bits() == expected.to_bits())
    }

    #[test]
    fn a_number_widens_integers_to_f32() {
        assert!(reads_as(JsonForm::UInt(7), 7.0));
        assert!(reads_as(JsonForm::Int(-7), -7.0));
        assert!(reads_as(JsonForm::Number(0.5), 0.5));
        assert!(reads_as(JsonForm::Number(2.5), 2.5));
        assert!(reads_as(JsonForm::UInt(3), 3.0));
        assert!(reads_as(JsonForm::Int(-3), -3.0));
    }

    #[test]
    fn a_number_may_be_an_extreme_integer() {
        assert!(reads_as(JsonForm::UInt(u64::MAX), 2.0_f32.powi(64)));
        assert!(reads_as(JsonForm::Int(i64::MIN), -(2.0_f32.powi(63))));
    }

    #[test]
    fn a_number_not_finite_as_f32_is_rejected() {
        for x in [1e39, -1e39, f64::INFINITY, f64::NEG_INFINITY, f64::NAN] {
            assert!(
                matches!(read(JsonForm::Number(x)), Err(Rejected::NonFinite)),
                "{x}"
            );
        }
    }

    #[test]
    fn no_other_shape_reads_as_a_constant() {
        for form in [
            JsonForm::Null,
            JsonForm::Bool(true),
            JsonForm::String("1".into()),
            JsonForm::Object(object_form(text("x"), Member::Int(1))),
        ] {
            assert!(!matches!(read(form), Ok(MolangValueRepr::Const(_))));
        }
    }

    #[test]
    fn member_json_type_for_every_variant() {
        assert_eq!(Member::Null.json_type(), JsonType::Null);
        assert_eq!(text("x").json_type(), JsonType::String);
        assert_eq!(Member::Bool(false).json_type(), JsonType::Bool);
        assert_eq!(Member::UInt(1).json_type(), JsonType::Number);
        assert_eq!(Member::Int(-1).json_type(), JsonType::Number);
        assert_eq!(Member::Number(1.0).json_type(), JsonType::Number);
    }

    #[test]
    fn into_parts_reads_expression_and_version() {
        let parts = object_form(text("q.x"), Member::Int(5))
            .into_parts()
            .unwrap();
        assert_eq!(parts, ("q.x".to_owned(), 5));
        let parts = object_form(text(""), Member::UInt(32767))
            .into_parts()
            .unwrap();
        assert_eq!(parts, (String::new(), i16::MAX));
        let parts = object_form(text("a"), Member::Int(-32768))
            .into_parts()
            .unwrap();
        assert_eq!(parts, ("a".to_owned(), i16::MIN));
    }

    #[test]
    fn into_parts_rejects_a_version_outside_i16() {
        for version in [
            Member::UInt(32768),
            Member::UInt(40000),
            Member::Int(-32769),
            Member::Int(-40000),
            Member::UInt(u64::MAX),
        ] {
            assert!(matches!(
                object_form(text("a"), version).into_parts(),
                Err(Rejected::VersionRange)
            ));
        }
    }

    #[test]
    fn into_parts_rejects_a_float_version() {
        assert!(matches!(
            object_form(text("a"), Member::Number(1.0)).into_parts(),
            Err(Rejected::VersionNotInteger)
        ));
    }

    #[test]
    fn into_parts_names_the_mistyped_version_member() {
        for (version, found) in [
            (text("1"), JsonType::String),
            (Member::Null, JsonType::Null),
            (Member::Bool(true), JsonType::Bool),
        ] {
            match object_form(text("a"), version).into_parts() {
                Err(Rejected::Member("version", ty)) => assert_eq!(ty, found),
                other => panic!("unexpected {other:?}"),
            }
        }
    }

    #[test]
    fn into_parts_names_the_mistyped_expression_member() {
        for (expression, found) in [
            (Member::UInt(1), JsonType::Number),
            (Member::Int(-1), JsonType::Number),
            (Member::Number(1.5), JsonType::Number),
            (Member::Null, JsonType::Null),
            (Member::Bool(false), JsonType::Bool),
        ] {
            match object_form(expression, Member::Int(1)).into_parts() {
                Err(Rejected::Member("expression", ty)) => assert_eq!(ty, found),
                other => panic!("unexpected {other:?}"),
            }
        }
    }

    #[test]
    fn into_parts_checks_the_expression_before_the_version() {
        assert!(matches!(
            object_form(Member::Null, Member::Number(1.0)).into_parts(),
            Err(Rejected::Member("expression", JsonType::Null))
        ));
    }

    #[test]
    fn source_proxy_takes_a_string_without_its_context_version() {
        let src = MolangSource::try_from(SourceProxy(JsonForm::String("Q.X".into()))).unwrap();
        assert_eq!(src, MolangSource::string_without_context("Q.X"));
        assert_eq!(src.raw_version(), None);
    }

    #[test]
    fn source_proxy_takes_the_object_form() {
        let form = JsonForm::Object(object_form(text("q.x"), Member::UInt(13)));
        let src = MolangSource::try_from(SourceProxy(form)).unwrap();
        assert_eq!(src, MolangSource::object("q.x", 13));
    }

    #[test]
    fn source_proxy_rejects_non_strings_and_names_the_type() {
        for (form, found) in [
            (JsonForm::Null, JsonType::Null),
            (JsonForm::Bool(true), JsonType::Bool),
            (JsonForm::UInt(1), JsonType::Number),
            (JsonForm::Int(-1), JsonType::Number),
            (JsonForm::Number(1.5), JsonType::Number),
        ] {
            match MolangSource::try_from(SourceProxy(form)) {
                Err(Rejected::Type(ty)) => assert_eq!(ty, found),
                other => panic!("unexpected {other:?}"),
            }
        }
    }

    #[test]
    fn source_proxy_passes_object_errors_through() {
        let form = JsonForm::Object(object_form(text("a"), Member::UInt(40000)));
        assert!(matches!(
            MolangSource::try_from(SourceProxy(form)),
            Err(Rejected::VersionRange)
        ));
    }

    #[test]
    fn source_proxy_from_a_string_source_is_a_string() {
        let SourceProxy(form) = SourceProxy::from(&MolangSource::string("a", 4));
        assert!(matches!(form, JsonForm::String(s) if s == "a"));
    }

    #[test]
    fn source_proxy_from_an_object_source_widens_the_version_to_i64() {
        for version in [i16::MIN, -1, 0, 13, i16::MAX] {
            let SourceProxy(form) = SourceProxy::from(&MolangSource::object("a", version));
            let JsonForm::Object(object) = form else {
                panic!("not an object")
            };
            assert!(matches!(&object.expression, Member::String(s) if s == "a"));
            assert!(matches!(object.version, Member::Int(v) if v == i64::from(version)));
        }
    }

    #[test]
    fn source_proxy_round_trips_both_forms() {
        for src in [
            MolangSource::object("x", -5),
            MolangSource::string_without_context("y"),
        ] {
            let back = MolangSource::try_from(SourceProxy::from(&src)).unwrap();
            assert_eq!(back, src);
        }
    }

    #[test]
    fn value_proxy_reads_bools_strings_and_objects() {
        assert_eq!(
            MolangValueRepr::try_from(ValueProxy(JsonForm::Bool(true))).unwrap(),
            MolangValueRepr::Bool(true)
        );
        assert_eq!(
            MolangValueRepr::try_from(ValueProxy(JsonForm::String("q.x".into()))).unwrap(),
            MolangValueRepr::Expr(MolangSource::string_without_context("q.x"))
        );
        let form = JsonForm::Object(object_form(text("q.x"), Member::Int(2)));
        assert_eq!(
            MolangValueRepr::try_from(ValueProxy(form)).unwrap(),
            MolangValueRepr::Expr(MolangSource::object("q.x", 2))
        );
    }

    #[test]
    fn value_proxy_rejects_null() {
        assert!(matches!(
            MolangValueRepr::try_from(ValueProxy(JsonForm::Null)),
            Err(Rejected::Type(JsonType::Null))
        ));
    }

    #[test]
    fn value_proxy_write_refuses_non_finite_constants() {
        for x in [f32::INFINITY, f32::NEG_INFINITY, f32::NAN] {
            assert!(
                matches!(
                    ValueProxy::try_from(&MolangValueRepr::Const(x)),
                    Err(Rejected::NonFinite)
                ),
                "{x}"
            );
        }
    }

    #[test]
    fn value_proxy_write_widens_finite_constants_exactly() {
        let ValueProxy(form) = ValueProxy::try_from(&MolangValueRepr::Const(0.1)).unwrap();
        assert!(matches!(form, JsonForm::Number(x) if x.to_bits() == f64::from(0.1_f32).to_bits()));
    }

    #[test]
    fn value_proxy_write_of_bools_and_sources() {
        let ValueProxy(form) = ValueProxy::try_from(&MolangValueRepr::Bool(true)).unwrap();
        assert!(matches!(form, JsonForm::Bool(true)));
        let ValueProxy(form) =
            ValueProxy::try_from(&MolangValueRepr::Expr(MolangSource::string("a", 0))).unwrap();
        assert!(matches!(form, JsonForm::String(s) if s == "a"));
        let ValueProxy(form) =
            ValueProxy::try_from(&MolangValueRepr::Expr(MolangSource::object("a", 6))).unwrap();
        assert!(matches!(form, JsonForm::Object(_)));
    }

    #[test]
    fn rejected_display_texts() {
        assert_eq!(
            Rejected::Type(JsonType::Bool).to_string(),
            "a Molang value cannot be a JSON bool"
        );
        assert_eq!(
            Rejected::Type(JsonType::Null).to_string(),
            "a Molang value cannot be a JSON null"
        );
        assert_eq!(
            Rejected::Member("version", JsonType::String).to_string(),
            "the `version` member of a Molang object cannot be a JSON string"
        );
        assert_eq!(
            Rejected::Member("expression", JsonType::Number).to_string(),
            "the `expression` member of a Molang object cannot be a JSON number"
        );
        assert_eq!(
            Rejected::VersionNotInteger.to_string(),
            "the `version` member of a Molang object must be an integer"
        );
        assert_eq!(
            Rejected::VersionRange.to_string(),
            "the `version` member of a Molang object does not fit an i16"
        );
        assert_eq!(
            Rejected::NonFinite.to_string(),
            "a Molang constant must be a finite f32"
        );
    }

    #[test]
    fn rejected_is_a_std_error() {
        let error: &dyn std::error::Error = &Rejected::NonFinite;
        assert_eq!(error.to_string(), "a Molang constant must be a finite f32");
        assert!(error.source().is_none());
    }
}
