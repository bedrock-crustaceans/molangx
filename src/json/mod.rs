//! Molang fields of JSON documents: the source and the form it was written in ([`MolangSource`],
//! [`SourceForm`]), the field's value ([`MolangValueRepr`]) and how it is read ([`ReaderKind`]).
//!
//! There is no JSON library dependency: the caller converts its JSON value into a [`JsonScalar`]
//! and back.
//!
//! ```
//! use molangx::json::{JsonScalar, MolangValueRepr, ReadField, ReaderKind, SourceForm};
//! use molangx::version::RawVersion;
//!
//! let versioned = ReaderKind::StrictVersioned;
//! // A string takes the version of the load context (here 6) ...
//! let s = MolangValueRepr::read(JsonScalar::String("q.is_baby ? 1 : 2".into()), versioned, 6);
//! let Ok(ReadField::Value(s)) = s else { unreachable!() };
//! let MolangValueRepr::Expr(src) = &s else { unreachable!() };
//! assert_eq!(src.form(), SourceForm::String { context_version: Some(RawVersion(6)) });
//! // ... the object form keeps its own, raw.
//! let o = JsonScalar::Object { expression: "1 + 2".into(), version: RawVersion(42) };
//! let Ok(ReadField::Value(obj)) = MolangValueRepr::read(o.clone(), versioned, 6) else {
//!     unreachable!()
//! };
//! assert_eq!(obj.source().and_then(|src| src.raw_version()), Some(RawVersion(42)));
//! assert_eq!(obj.write(), o);
//! // Writing back gives exactly the form that was read.
//! assert_eq!(s.write(), JsonScalar::String("q.is_baby ? 1 : 2".into()));
//! ```

use std::fmt;
use std::sync::Arc;

use thiserror::Error;

use crate::version::{MolangVersion, RawVersion};

/// How a Molang expression was written in its document, and the version that comes with it.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "facet", derive(facet::Facet))]
#[repr(u8)]
#[non_exhaustive]
pub enum SourceForm {
    /// A plain JSON string, whose version is the load context's.
    String {
        /// The load context's raw version; `None` until
        /// [`MolangSource::set_context_version`] applies it (a deserializer has no load context).
        context_version: Option<RawVersion>,
    },
    /// The object form `{"expression": <string>, "version": <i16>}`, whose version is its own.
    Object {
        /// The `version` member, raw.
        version: RawVersion,
    },
}

impl SourceForm {
    /// The raw version that comes with the form; `None` for a string whose context version was not
    /// applied.
    pub const fn raw_version(self) -> Option<RawVersion> {
        match self {
            Self::String { context_version } => context_version,
            Self::Object { version } => Some(version),
        }
    }
}

/// Molang source text together with the form it was written in and the version it compiles at.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "facet", derive(facet::Facet), facet(proxy = crate::json::facet_proxy::SourceProxy))]
pub struct MolangSource {
    /// As written: case preserved (the lexer lower-cases), not trimmed, not unescaped.
    text: Arc<str>,
    form: SourceForm,
}

impl MolangSource {
    /// A plain-string source compiled at the load context's version `context_version`.
    pub fn string(text: impl Into<Arc<str>>, context_version: impl Into<RawVersion>) -> Self {
        Self {
            text: text.into(),
            form: SourceForm::String {
                context_version: Some(context_version.into()),
            },
        }
    }

    /// A plain-string source whose load-context version is not applied yet.
    ///
    /// Apply it with [`MolangSource::set_context_version`] before compiling: compiled as it is, it
    /// gets a warning, the rules of [`MolangVersion::Invalid`], and no query resolves.
    pub fn string_without_context(text: impl Into<Arc<str>>) -> Self {
        Self {
            text: text.into(),
            form: SourceForm::String {
                context_version: None,
            },
        }
    }

    /// An object-form source `{"expression": text, "version": version}`.
    pub fn object(text: impl Into<Arc<str>>, version: impl Into<RawVersion>) -> Self {
        Self {
            text: text.into(),
            form: SourceForm::Object {
                version: version.into(),
            },
        }
    }

    /// The source text.
    pub fn as_str(&self) -> &str {
        &self.text
    }

    #[cfg(feature = "cache")]
    pub(crate) fn shared_text(&self) -> &Arc<str> {
        &self.text
    }

    /// How the text was written, with its version.
    pub const fn form(&self) -> SourceForm {
        self.form
    }

    /// Whether the text is empty.
    pub fn is_empty(&self) -> bool {
        self.text.is_empty()
    }

    /// The raw version ([`SourceForm::raw_version`]), which query resolution compares unclamped.
    pub const fn raw_version(&self) -> Option<RawVersion> {
        self.form.raw_version()
    }

    /// The version the parser and evaluator rules use:
    /// [`RawVersion::effective`](crate::version::RawVersion::effective) of
    /// [`MolangSource::raw_version`], or [`MolangVersion::Invalid`] without one.
    pub const fn effective_version(&self) -> MolangVersion {
        match self.raw_version() {
            Some(raw) => raw.effective(),
            None => MolangVersion::Invalid,
        }
    }

    /// Applies the load context's version; an object-form source keeps its own.
    pub fn set_context_version(&mut self, context_version: impl Into<RawVersion>) {
        if let SourceForm::String { .. } = self.form {
            self.form = SourceForm::String {
                context_version: Some(context_version.into()),
            };
        }
    }
}

/// The value of a document field that holds Molang: a constant, a bool, or an expression.
#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(feature = "facet", derive(facet::Facet), facet(proxy = crate::json::facet_proxy::ValueProxy))]
#[repr(u8)]
#[non_exhaustive]
pub enum MolangValueRepr {
    /// A JSON number: a constant.
    Const(f32),
    /// A JSON bool, which evaluates as 1.0 / 0.0; kept as a bool so it is written back as one.
    Bool(bool),
    /// A Molang expression in string or object form.
    Expr(MolangSource),
}

impl MolangValueRepr {
    /// Reads a JSON value as a field of kind `reader`; a plain string takes `context_version`.
    ///
    /// An invalid expression string is not an error here; the compiler reports it.
    pub fn read(
        json: JsonScalar,
        reader: ReaderKind,
        context_version: impl Into<RawVersion>,
    ) -> Result<ReadField, ReprError> {
        let unsupported = |found| Err(ReprError::UnsupportedType { reader, found });
        let value = match json {
            JsonScalar::Number(value) => {
                if reader == ReaderKind::StrictVersioned
                    && (value.is_nan() || value.abs() > f64::from(f32::MAX))
                {
                    return Err(ReprError::NumberOutOfRange { value });
                }
                Self::Const(value as f32)
            }
            JsonScalar::Bool(b) => match reader {
                ReaderKind::StrictVersioned => return unsupported(JsonType::Bool),
                _ => Self::Bool(b),
            },
            JsonScalar::String(src) => Self::Expr(MolangSource::string(src, context_version)),
            JsonScalar::Object {
                expression,
                version,
            } => match reader {
                ReaderKind::StrictVersioned | ReaderKind::LenientVersioned => {
                    Self::Expr(MolangSource::object(expression, version))
                }
                _ => return unsupported(JsonType::Object),
            },
            JsonScalar::Other(found) => {
                return match (reader, found) {
                    (ReaderKind::ScalarOrArray, OtherJson::Null) => Ok(ReadField::NoExpression),
                    (ReaderKind::ScalarOrArray, OtherJson::Array) => Ok(ReadField::ExpressionArray),
                    _ => unsupported(found.into()),
                };
            }
        };
        Ok(ReadField::Value(value))
    }

    /// Writes the value back in exactly the form it was read.
    ///
    /// `Const` is widened to `f64` exactly: JSON `0.1` is written as `0.100000001490116…`, which
    /// reads back as the same `f32`. For the shortest decimal, format
    /// [`MolangValueRepr::number`].
    ///
    /// A non-finite `Const` is written as a non-finite `JsonScalar::Number`, which JSON cannot
    /// hold: the caller's writer must reject or replace it. The `facet` impl refuses to serialise
    /// it.
    pub fn write(&self) -> JsonScalar {
        match self {
            Self::Const(x) => JsonScalar::Number(f64::from(*x)),
            Self::Bool(b) => JsonScalar::Bool(*b),
            Self::Expr(src) => match src.form {
                SourceForm::String { .. } => JsonScalar::String(Arc::clone(&src.text)),
                SourceForm::Object { version } => JsonScalar::Object {
                    expression: Arc::clone(&src.text),
                    version,
                },
            },
        }
    }

    /// Whether `reader` could have produced this value: the result of [`MolangValueRepr::read`] of
    /// [`MolangValueRepr::write`]'s output, which is never null or an array. The version is not
    /// checked.
    ///
    /// For a value obtained elsewhere and stored in a field whose reader is narrower. The `facet`
    /// impls accept what a lenient versioned field does, except a number not finite as an `f32`,
    /// and ignore other members of the object form.
    pub fn check(&self, reader: ReaderKind) -> Result<(), ReprError> {
        Self::read(self.write(), reader, 0).map(drop)
    }

    /// The `f32` of a `Const`, a JSON number; `None` for a `Bool`, which
    /// [`MolangValueRepr::constant_value`] counts.
    pub const fn number(&self) -> Option<f32> {
        match self {
            Self::Const(x) => Some(*x),
            _ => None,
        }
    }

    /// The value without compiling: a `Const`, or a `Bool` as 1.0 / 0.0; `None` for an `Expr`.
    pub const fn constant_value(&self) -> Option<f32> {
        match self {
            Self::Const(x) => Some(*x),
            Self::Bool(true) => Some(1.0),
            Self::Bool(false) => Some(0.0),
            Self::Expr(_) => None,
        }
    }

    /// The expression source, if this is an expression.
    pub const fn source(&self) -> Option<&MolangSource> {
        match self {
            Self::Expr(src) => Some(src),
            _ => None,
        }
    }

    /// [`MolangSource::set_context_version`] on an expression; anything else is unchanged.
    pub fn set_context_version(&mut self, context_version: impl Into<RawVersion>) {
        if let Self::Expr(src) = self {
            src.set_context_version(context_version);
        }
    }
}

impl From<f32> for MolangValueRepr {
    fn from(x: f32) -> Self {
        Self::Const(x)
    }
}

impl From<MolangSource> for MolangValueRepr {
    fn from(src: MolangSource) -> Self {
        Self::Expr(src)
    }
}

/// The JSON types.
///
/// Exhaustive: the set will not grow.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum JsonType {
    /// `null`.
    Null,
    /// `true` / `false`.
    Bool,
    /// A number.
    Number,
    /// A string.
    String,
    /// An array.
    Array,
    /// An object.
    Object,
}

impl fmt::Display for JsonType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Null => "null",
            Self::Bool => "bool",
            Self::Number => "number",
            Self::String => "string",
            Self::Array => "array",
            Self::Object => "object",
        })
    }
}

/// A JSON value as far as a Molang field cares.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum JsonScalar {
    /// A JSON number.
    Number(f64),
    /// A JSON bool.
    Bool(bool),
    /// A JSON string.
    String(Arc<str>),
    /// The versioned object form `{"expression": <string>, "version": <integer>}`.
    ///
    /// An object that lacks either key, has a non-string `expression`, or a `version` outside `i16`
    /// is not this form: pass it as `Other(OtherJson::Object)`. Other members are the caller's to
    /// ignore or reject.
    Object {
        /// The `expression` member.
        expression: Arc<str>,
        /// The `version` member, unchecked.
        version: RawVersion,
    },
    /// Any other JSON value, by type.
    Other(OtherJson),
}

/// The JSON types without a [`JsonScalar`] variant of their own.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum OtherJson {
    /// `null`.
    Null,
    /// An array.
    Array,
    /// An object that is not the versioned `{expression, version}` form.
    Object,
}

impl From<OtherJson> for JsonType {
    fn from(other: OtherJson) -> Self {
        match other {
            OtherJson::Null => Self::Null,
            OtherJson::Array => Self::Array,
            OtherJson::Object => Self::Object,
        }
    }
}

impl fmt::Display for OtherJson {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&JsonType::from(*self), f)
    }
}

impl JsonScalar {
    /// The JSON type of this value.
    pub const fn json_type(&self) -> JsonType {
        match self {
            Self::Number(_) => JsonType::Number,
            Self::Bool(_) => JsonType::Bool,
            Self::String(_) => JsonType::String,
            Self::Object { .. } => JsonType::Object,
            Self::Other(t) => match t {
                OtherJson::Null => JsonType::Null,
                OtherJson::Array => JsonType::Array,
                OtherJson::Object => JsonType::Object,
            },
        }
    }
}

/// How a document field that holds Molang is read: which JSON types it accepts. The caller picks
/// the kind per field.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ReaderKind {
    /// The usual field of a strict document: a number that fits an `f32`, a string, or the
    /// versioned object form.
    StrictVersioned,
    /// The usual field of a lenient document: a number, a bool, a string, or the versioned object
    /// form.
    LenientVersioned,
    /// A number, bool or string, no object form: the biome `surface_material_adjustments`
    /// `height_range` field.
    BiomeHeightRange,
    /// A number, bool, string, or an array of expressions; null gives no expression and an
    /// object fails.
    ScalarOrArray,
    /// A field checked against the document's schema: a number, bool or string.
    SchemaValidated,
}

impl fmt::Display for ReaderKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::StrictVersioned => "strict versioned",
            Self::LenientVersioned => "lenient versioned",
            Self::BiomeHeightRange => "biome height range",
            Self::ScalarOrArray => "scalar-or-array",
            Self::SchemaValidated => "schema-validated",
        })
    }
}

/// What [`MolangValueRepr::read`] found in a field.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum ReadField {
    /// One value.
    Value(MolangValueRepr),
    /// JSON `null` in a [`ReaderKind::ScalarOrArray`] field: no expression.
    NoExpression,
    /// A JSON array in a [`ReaderKind::ScalarOrArray`] field: an array of expressions
    /// ([`ExpressionOp::ExpressionArray`](crate::ops::ExpressionOp::ExpressionArray)). Read each
    /// element the same way.
    ExpressionArray,
}

/// Why [`MolangValueRepr::read`] failed.
#[derive(Error, Debug, Clone, Copy, PartialEq)]
#[non_exhaustive]
pub enum ReprError {
    /// The field does not accept this JSON type.
    #[error("a {reader} Molang field does not accept a JSON {found}")]
    UnsupportedType {
        /// How the field is read.
        reader: ReaderKind,
        /// The JSON type that was found.
        found: JsonType,
    },
    /// A strict versioned field only accepts a number that fits an `f32`; NaN does not.
    #[error("the number {value:e} does not fit an f32")]
    NumberOutOfRange {
        /// The JSON number as read.
        value: f64,
    },
}

#[cfg(feature = "facet")]
mod facet_proxy;

#[cfg(test)]
mod tests {
    use super::*;

    const STRICT: ReaderKind = ReaderKind::StrictVersioned;
    const LENIENT: ReaderKind = ReaderKind::LenientVersioned;
    const ALL_READERS: [ReaderKind; 5] = [
        STRICT,
        LENIENT,
        ReaderKind::BiomeHeightRange,
        ReaderKind::ScalarOrArray,
        ReaderKind::SchemaValidated,
    ];

    fn string(s: &str) -> JsonScalar {
        JsonScalar::String(s.into())
    }

    fn object(expression: &str, version: i16) -> JsonScalar {
        JsonScalar::Object {
            expression: expression.into(),
            version: RawVersion(version),
        }
    }

    fn raw(src: &MolangSource) -> Option<i16> {
        src.raw_version().map(i16::from)
    }

    fn is_string_form(src: &MolangSource) -> bool {
        matches!(src.form(), SourceForm::String { .. })
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

    fn unsupported(reader: ReaderKind, found: JsonType) -> Result<MolangValueRepr, ReprError> {
        Err(ReprError::UnsupportedType { reader, found })
    }

    #[test]
    fn value_repr_has_three_arms() {
        let describe = |v: &MolangValueRepr| match v {
            MolangValueRepr::Const(_) => "const",
            MolangValueRepr::Bool(_) => "bool",
            MolangValueRepr::Expr(_) => "expr",
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
        assert_eq!(
            MolangSource::string_without_context("x").form(),
            SourceForm::String {
                context_version: None
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
        for (reader, text, version) in [(STRICT, "1", 9), (LENIENT, "x", 5)] {
            for ctx in [i16::MIN, -1, 0, 4, 13, 14, i16::MAX] {
                let s = read(string(text), reader, ctx).unwrap();
                assert_eq!(
                    s.source().unwrap().form(),
                    SourceForm::String {
                        context_version: Some(RawVersion(ctx))
                    }
                );
                let o = read(object(text, version), reader, ctx).unwrap();
                assert_eq!(
                    o.source().unwrap().form(),
                    SourceForm::Object {
                        version: RawVersion(version)
                    }
                );
            }
        }
    }

    #[test]
    fn object_version_is_unconstrained() {
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
    fn bool_only_in_a_lenient_versioned_field() {
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
        let v = read(string("query.does_not_exist"), STRICT, 13).unwrap();
        assert_eq!(v.source().unwrap().as_str(), "query.does_not_exist");
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
    fn scalar_or_array_field() {
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
    fn schema_validated_field() {
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
    fn only_versioned_fields_take_the_object_form() {
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
                    MolangValueRepr::read(v.write(), reader, 0).map(drop),
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

    fn arm(result: Result<ReadField, ReprError>) -> Result<&'static str, ReprError> {
        result.map(|field| match field {
            ReadField::Value(MolangValueRepr::Const(_)) => "const",
            ReadField::Value(MolangValueRepr::Bool(_)) => "bool",
            ReadField::Value(MolangValueRepr::Expr(src)) if !is_string_form(&src) => "object",
            ReadField::Value(MolangValueRepr::Expr(_)) => "string",
            ReadField::NoExpression => "no expression",
            ReadField::ExpressionArray => "expression array",
        })
    }

    type Cell = Result<&'static str, ReprError>;

    fn bad(reader: ReaderKind, found: JsonType) -> Cell {
        Err(ReprError::UnsupportedType { reader, found })
    }

    #[test]
    fn reader_matrix_is_the_documented_table() {
        let (height, scalar_or_array, schema) = (
            ReaderKind::BiomeHeightRange,
            ReaderKind::ScalarOrArray,
            ReaderKind::SchemaValidated,
        );
        // Columns follow ALL_READERS.
        let rows: [(JsonScalar, [Cell; 5]); 7] = [
            (
                JsonScalar::Number(1.5),
                [
                    Ok("const"),
                    Ok("const"),
                    Ok("const"),
                    Ok("const"),
                    Ok("const"),
                ],
            ),
            (
                JsonScalar::Bool(true),
                [
                    bad(STRICT, JsonType::Bool),
                    Ok("bool"),
                    Ok("bool"),
                    Ok("bool"),
                    Ok("bool"),
                ],
            ),
            (
                string("s"),
                [
                    Ok("string"),
                    Ok("string"),
                    Ok("string"),
                    Ok("string"),
                    Ok("string"),
                ],
            ),
            (
                object("s", 2),
                [
                    Ok("object"),
                    Ok("object"),
                    bad(height, JsonType::Object),
                    bad(scalar_or_array, JsonType::Object),
                    bad(schema, JsonType::Object),
                ],
            ),
            (
                JsonScalar::Other(OtherJson::Null),
                [
                    bad(STRICT, JsonType::Null),
                    bad(LENIENT, JsonType::Null),
                    bad(height, JsonType::Null),
                    Ok("no expression"),
                    bad(schema, JsonType::Null),
                ],
            ),
            (
                JsonScalar::Other(OtherJson::Array),
                [
                    bad(STRICT, JsonType::Array),
                    bad(LENIENT, JsonType::Array),
                    bad(height, JsonType::Array),
                    Ok("expression array"),
                    bad(schema, JsonType::Array),
                ],
            ),
            (
                JsonScalar::Other(OtherJson::Object),
                [
                    bad(STRICT, JsonType::Object),
                    bad(LENIENT, JsonType::Object),
                    bad(height, JsonType::Object),
                    bad(scalar_or_array, JsonType::Object),
                    bad(schema, JsonType::Object),
                ],
            ),
        ];
        for (json, expected) in rows {
            for (reader, cell) in ALL_READERS.into_iter().zip(expected) {
                assert_eq!(
                    arm(MolangValueRepr::read(json.clone(), reader, 3)),
                    cell,
                    "{json:?} under {reader}"
                );
            }
        }
    }

    #[test]
    fn strict_number_range_boundary() {
        let max = f64::from(f32::MAX);
        let above = max * 1.000_000_1;
        assert_eq!(
            read(JsonScalar::Number(above), STRICT, 0),
            Err(ReprError::NumberOutOfRange { value: above })
        );
        assert_eq!(
            read(JsonScalar::Number(-above), STRICT, 0),
            Err(ReprError::NumberOutOfRange { value: -above })
        );
        assert_eq!(
            read(JsonScalar::Number(f64::INFINITY), STRICT, 0),
            Err(ReprError::NumberOutOfRange {
                value: f64::INFINITY
            })
        );
        assert_eq!(
            read(JsonScalar::Number(f64::NEG_INFINITY), STRICT, 0),
            Err(ReprError::NumberOutOfRange {
                value: f64::NEG_INFINITY
            })
        );
    }

    #[test]
    fn strict_number_nan_does_not_fit_an_f32() {
        for nan in [f64::NAN, -f64::NAN] {
            let Err(ReprError::NumberOutOfRange { value }) =
                read(JsonScalar::Number(nan), STRICT, 0)
            else {
                panic!("NaN was accepted");
            };
            assert_eq!(value.to_bits(), nan.to_bits());
        }
    }

    #[test]
    fn other_readers_keep_a_nan_they_are_handed() {
        for reader in [
            LENIENT,
            ReaderKind::BiomeHeightRange,
            ReaderKind::ScalarOrArray,
            ReaderKind::SchemaValidated,
        ] {
            let Ok(MolangValueRepr::Const(x)) = read(JsonScalar::Number(f64::NAN), reader, 0)
            else {
                panic!("{reader}: NaN was rejected");
            };
            assert!(x.is_nan(), "{reader}");
        }
    }

    #[test]
    fn other_readers_turn_huge_numbers_into_infinities() {
        for reader in [
            LENIENT,
            ReaderKind::BiomeHeightRange,
            ReaderKind::ScalarOrArray,
            ReaderKind::SchemaValidated,
        ] {
            assert_eq!(
                read(JsonScalar::Number(1e39), reader, 0),
                Ok(MolangValueRepr::Const(f32::INFINITY))
            );
            assert_eq!(
                read(JsonScalar::Number(-1e300), reader, 0),
                Ok(MolangValueRepr::Const(f32::NEG_INFINITY))
            );
        }
    }

    #[test]
    fn tiny_numbers_underflow_to_zero() {
        assert_eq!(
            read(JsonScalar::Number(1e-50), STRICT, 0),
            Ok(MolangValueRepr::Const(0.0))
        );
        let Ok(MolangValueRepr::Const(z)) = read(JsonScalar::Number(-1e-50), STRICT, 0) else {
            panic!("rejected");
        };
        assert_eq!(z.to_bits(), (-0.0_f32).to_bits());
    }

    #[test]
    fn write_inverts_read_for_each_form() {
        for value in [
            MolangValueRepr::Const(-4.25),
            MolangValueRepr::Bool(true),
            MolangValueRepr::Expr(MolangSource::string("q.x", 9)),
            MolangValueRepr::Expr(MolangSource::object("q.x", 9)),
        ] {
            assert_eq!(read(value.write(), LENIENT, 9), Ok(value));
        }
    }

    #[test]
    fn write_of_each_arm_is_the_matching_json_shape() {
        assert_eq!(MolangValueRepr::Const(0.5).write(), JsonScalar::Number(0.5));
        assert_eq!(
            MolangValueRepr::Bool(false).write(),
            JsonScalar::Bool(false)
        );
        assert_eq!(
            MolangValueRepr::Expr(MolangSource::string("a", 7)).write(),
            string("a")
        );
        assert_eq!(
            MolangValueRepr::Expr(MolangSource::object("a", 7)).write(),
            object("a", 7)
        );
    }

    #[test]
    fn check_follows_the_documented_table() {
        let finite = MolangValueRepr::Const(2.0);
        let infinite = MolangValueRepr::Const(f32::INFINITY);
        let not_a_number = MolangValueRepr::Const(f32::NAN);
        let boolean = MolangValueRepr::Bool(true);
        let text = MolangValueRepr::Expr(MolangSource::string("1", 0));
        let object_form = MolangValueRepr::Expr(MolangSource::object("1", 0));
        for reader in ALL_READERS {
            let strict = reader == STRICT;
            let versioned = matches!(
                reader,
                ReaderKind::StrictVersioned | ReaderKind::LenientVersioned
            );
            assert_eq!(finite.check(reader), Ok(()), "{reader}");
            assert_eq!(not_a_number.check(reader).is_ok(), !strict, "{reader}");
            assert_eq!(text.check(reader), Ok(()), "{reader}");
            assert_eq!(infinite.check(reader).is_ok(), !strict, "{reader}");
            assert_eq!(boolean.check(reader).is_ok(), !strict, "{reader}");
            assert_eq!(object_form.check(reader).is_ok(), versioned, "{reader}");
        }
        assert_eq!(
            MolangValueRepr::Const(f32::NEG_INFINITY).check(STRICT),
            Err(ReprError::NumberOutOfRange {
                value: f64::NEG_INFINITY
            })
        );
    }

    proptest::proptest! {
        #![proptest_config(proptest::prelude::ProptestConfig { failure_persistence: None, ..Default::default() })]

        #[test]
        fn check_accepts_every_finite_constant_under_every_reader(x in proptest::num::f32::NORMAL | proptest::num::f32::ZERO | proptest::num::f32::SUBNORMAL) {
            for reader in ALL_READERS {
                proptest::prop_assert_eq!(MolangValueRepr::Const(x).check(reader), Ok(()));
            }
        }

        #[test]
        fn read_of_write_keeps_a_finite_constant_bit_for_bit(x in proptest::num::f32::NORMAL | proptest::num::f32::ZERO | proptest::num::f32::SUBNORMAL) {
            for reader in ALL_READERS {
                let back = read(MolangValueRepr::Const(x).write(), reader, 0).unwrap();
                let MolangValueRepr::Const(y) = back else { panic!("not a constant") };
                proptest::prop_assert_eq!(x.to_bits(), y.to_bits());
            }
        }
    }

    #[test]
    fn source_constructors_set_the_form() {
        let s = MolangSource::string("a b", 3);
        assert_eq!(
            (s.as_str(), raw(&s), is_string_form(&s)),
            ("a b", Some(3), true)
        );
        let o = MolangSource::object(Arc::<str>::from("a b"), -3);
        assert_eq!(
            (o.as_str(), raw(&o), is_string_form(&o)),
            ("a b", Some(-3), false)
        );
        let u = MolangSource::string_without_context("a b");
        assert_eq!(
            (u.as_str(), raw(&u), is_string_form(&u)),
            ("a b", None, true)
        );
    }

    #[test]
    fn source_is_empty_only_for_the_empty_text() {
        assert!(MolangSource::string("", 0).is_empty());
        assert!(!MolangSource::string(" ", 0).is_empty());
        assert!(!MolangSource::string("\0", 0).is_empty());
    }

    #[test]
    fn a_string_without_its_context_version_has_no_raw_version_and_the_rules_of_invalid() {
        let s = MolangSource::string_without_context("q.x");
        assert_eq!(s.effective_version(), MolangVersion::Invalid);
        assert_eq!(s.raw_version(), None);
        assert_ne!(s, MolangSource::string("q.x", i16::MIN));
    }

    #[test]
    fn source_effective_and_raw_versions() {
        let s = MolangSource::string("x", 20);
        assert_eq!(s.raw_version(), Some(RawVersion(20)));
        assert_eq!(s.effective_version(), MolangVersion::LATEST);
        let s = MolangSource::string("x", 0);
        assert_eq!(s.effective_version(), MolangVersion::V0);
    }

    #[test]
    fn source_set_context_version_changes_the_string_form_only() {
        let mut s = MolangSource::string("x", 1);
        s.set_context_version(8);
        assert_eq!(raw(&s), Some(8));
        let mut o = MolangSource::object("x", 1);
        o.set_context_version(8);
        assert_eq!(raw(&o), Some(1));
        let mut u = MolangSource::string_without_context("x");
        u.set_context_version(MolangVersion::V8);
        assert_eq!(u, MolangSource::string("x", 8));
    }

    #[test]
    fn an_object_form_keeps_its_version_even_when_the_context_version_is_zero() {
        for context in [0, -1, 1, 13] {
            let mut o = MolangSource::object("x", 5);
            o.set_context_version(context);
            assert_eq!(raw(&o), Some(5), "context {context}");
            let mut s = MolangSource::string("x", 5);
            s.set_context_version(context);
            assert_eq!(raw(&s), Some(context));
        }
    }

    #[test]
    fn a_false_bool_is_positive_zero_bit_for_bit() {
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

    #[test]
    fn value_set_context_version_leaves_constants_and_bools_alone() {
        let mut b = MolangValueRepr::Bool(true);
        b.set_context_version(5);
        assert_eq!(b, MolangValueRepr::Bool(true));
        let mut s = MolangValueRepr::Expr(MolangSource::string("x", 1));
        s.set_context_version(5);
        assert_eq!(s, MolangValueRepr::Expr(MolangSource::string("x", 5)));
    }

    #[test]
    fn constant_value_and_number_differ_on_bools() {
        assert_eq!(MolangValueRepr::Bool(true).constant_value(), Some(1.0));
        assert_eq!(MolangValueRepr::Bool(false).constant_value(), Some(0.0));
        assert_eq!(MolangValueRepr::Bool(true).number(), None);
        assert_eq!(MolangValueRepr::Bool(false).number(), None);
        assert_eq!(MolangValueRepr::Const(2.5).number(), Some(2.5));
        let negative_zero = MolangValueRepr::Const(-0.0);
        assert_eq!(
            negative_zero.constant_value().map(f32::to_bits),
            Some((-0.0_f32).to_bits())
        );
        assert_eq!(
            negative_zero.number().map(f32::to_bits),
            Some((-0.0_f32).to_bits())
        );
        let expr = MolangValueRepr::Expr(MolangSource::string("1", 0));
        assert_eq!((expr.constant_value(), expr.number()), (None, None));
    }

    #[test]
    fn source_accessor_returns_the_source_only_for_expressions() {
        assert_eq!(MolangValueRepr::Const(1.0).source(), None);
        assert_eq!(MolangValueRepr::Bool(true).source(), None);
        let src = MolangSource::object("x", 2);
        assert_eq!(MolangValueRepr::Expr(src.clone()).source(), Some(&src));
    }

    #[test]
    fn from_impls_build_the_matching_arm() {
        assert_eq!(MolangValueRepr::from(2.5_f32), MolangValueRepr::Const(2.5));
        let src = MolangSource::string("1", 0);
        assert_eq!(
            MolangValueRepr::from(src.clone()),
            MolangValueRepr::Expr(src)
        );
    }

    #[test]
    fn json_type_display_names() {
        let names = [
            (JsonType::Null, "null"),
            (JsonType::Bool, "bool"),
            (JsonType::Number, "number"),
            (JsonType::String, "string"),
            (JsonType::Array, "array"),
            (JsonType::Object, "object"),
        ];
        for (ty, name) in names {
            assert_eq!(ty.to_string(), name);
        }
    }

    #[test]
    fn other_json_display_and_conversion() {
        for (other, ty, name) in [
            (OtherJson::Null, JsonType::Null, "null"),
            (OtherJson::Array, JsonType::Array, "array"),
            (OtherJson::Object, JsonType::Object, "object"),
        ] {
            assert_eq!(JsonType::from(other), ty);
            assert_eq!(other.to_string(), name);
            assert_eq!(JsonScalar::Other(other).json_type(), ty);
        }
    }

    #[test]
    fn json_scalar_json_type_for_every_shape() {
        assert_eq!(JsonScalar::Number(0.0).json_type(), JsonType::Number);
        assert_eq!(JsonScalar::Bool(false).json_type(), JsonType::Bool);
        assert_eq!(string("").json_type(), JsonType::String);
        assert_eq!(object("", 0).json_type(), JsonType::Object);
    }

    #[test]
    fn unsupported_type_message_names_the_reader_and_the_type() {
        for (reader, name) in [
            (STRICT, "strict versioned"),
            (LENIENT, "lenient versioned"),
            (ReaderKind::BiomeHeightRange, "biome height range"),
            (ReaderKind::ScalarOrArray, "scalar-or-array"),
            (ReaderKind::SchemaValidated, "schema-validated"),
        ] {
            assert_eq!(reader.to_string(), name);
            for (found, json) in [(JsonType::Array, "array"), (JsonType::Null, "null")] {
                assert_eq!(
                    ReprError::UnsupportedType { reader, found }.to_string(),
                    format!("a {name} Molang field does not accept a JSON {json}")
                );
            }
        }
    }

    #[test]
    fn the_raw_version_of_a_form() {
        assert_eq!(
            SourceForm::String {
                context_version: None
            }
            .raw_version(),
            None
        );
        assert_eq!(
            SourceForm::String {
                context_version: Some(RawVersion(4))
            }
            .raw_version(),
            Some(RawVersion(4))
        );
        assert_eq!(
            SourceForm::Object {
                version: RawVersion(-4)
            }
            .raw_version(),
            Some(RawVersion(-4))
        );
        assert_ne!(
            SourceForm::String {
                context_version: Some(RawVersion(4))
            },
            SourceForm::Object {
                version: RawVersion(4)
            }
        );
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
    fn repr_error_is_copy_and_compares_by_value() {
        let a = ReprError::NumberOutOfRange { value: 2.0 };
        let b = a;
        assert_eq!(a, b);
        assert_ne!(a, ReprError::NumberOutOfRange { value: 3.0 });
        let error: &dyn std::error::Error = &a;
        assert!(error.source().is_none());
    }
}
