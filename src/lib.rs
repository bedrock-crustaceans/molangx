//! A pure Rust implementation of the Molang expression language of the 1.26 releases: grammar,
//! version gates, the standard library's queries and functions, error model and messages.
//!
//! Features (`stdlib` is the default):
//! - `stdlib`: the standard library, the `math.*` functions and the queries a script can name out
//!   of the box, and `CompileOptions::server` / `client`. Without it no `math.*` name is known and
//!   every query comes from a host catalogue.
//! - `compiler`: the modules `compile`, `diag`, `numeric` and `rng` (over the re-exported
//!   `rand_core` 0.10), host math functions (`catalog::MathCatalog`) and, with `stdlib`,
//!   `stdlib::math`.
//! - `vm`, `cache`: the modules of the same name (each implies `compiler`).
//! - `facet`: `Facet` impls for [`json::MolangSource`] and [`json::MolangValueRepr`]; the optional
//!   `facet` dependency is a pre-release and is not re-exported.
//! - `fuzz`: a hidden `internals` module for the repository's fuzz crate, with no `SemVer`
//!   guarantee (implies `stdlib`).
//!
//! Every public item has one path: the module it lives in. Modules, with the feature a module or
//! part of it needs:
//! - `catalog`: what a script may call and how a host declares it: `QueryCatalog`, `QueryDecl`,
//!   `QueryShape`, `Side`, and host `math.*` functions in a `MathCatalog` (`compiler`).
//! - `ops`: the operator table and the sets of operations a compilation allows (`OpSet`).
//! - `version`: what content targets: `MolangVersion`, `RawVersion`, `EngineVersion`, the game
//!   release (a re-exported `semver::Version`) and experiments (`ExperimentMask`).
//! - `json`: Molang fields of JSON documents: `MolangSource`, `MolangValueRepr` and their readers.
//! - `hash`: the string hash a Molang string value is (`HashedStr`).
//! - `stdlib` (`stdlib`): the standard `math.*` functions and queries (`queries`, `query`).
//! - `compile` (`compiler`): `compile`, `compile_source`, `CompileOptions`, `Compiled`, `Expr`,
//!   `ProgramFlags` and the limits `MAX_DEPTH`, `MAX_SOURCE_LEN` and `MAX_DIAGNOSTICS`.
//! - `diag` (`compiler`): `Diagnostic`, `DiagCode`, `Severity` and `LanguageMessage`.
//! - `numeric`, `rng` (`compiler`): the float semantics and the random sources.
//! - `vm` (`vm`): evaluation, `Value`, the host traits, variable storage and `EvalLimits`.
//! - `cache` (`cache`): `CompileCache`.
//!
//! The library does not implement queries. Every `query.*` call resolves at compile time against a
#![cfg_attr(
    feature = "stdlib",
    doc = "[`QueryCatalog`] ([`stdlib::queries`] plus the host's own declarations) and runs through the"
)]
#![cfg_attr(
    not(feature = "stdlib"),
    doc = "[`QueryCatalog`] (`stdlib::queries` plus the host's own declarations) and runs through the"
)]
//! host's `vm::QueryTable`. A host adds `math.*` functions of its own in a `catalog::MathCatalog`
//! passed in `CompileOptions::math`.
//!
//! # Compiling pack content
//!
//! `compile_source(&source, &field_options)` compiles a [`MolangSource`] at the source's version;
//! `compile(text, &options)` uses the options' version.
//!
//! The catalogue's [`Side`] has no default. `CompileOptions::server` leaves a query the server
//! catalogue lacks (`query.is_on_screen`) unresolved and lints a client-only query; resource-pack
//! Molang compiles with `CompileOptions::client`.
//!
//! # Quick start
//!
//! `compile` never fails: a rejected expression is the constant 0 with its `Diagnostic`s attached,
//! and `Compiled::into_result` turns that into a `Result`. `NoHostEnv` evaluates without a world:
//! numbers, strings, `variable.` / `temp.` / `context.` values and `math.*`.
//!
//! ```
//! # #[cfg(all(feature = "vm", feature = "stdlib"))]
//! # {
//! use molangx::compile::{CompileOptions, compile};
//! use molangx::version::MolangVersion;
//! use molangx::vm::{NoHostEnv, Value, VariableName};
//!
//! let options = CompileOptions::server(MolangVersion::LATEST);
//! // A success carries what the compile logged (warnings, notes) along with the expression.
//! let (expr, diagnostics) =
//!     compile("v.speed * 2 + math.clamp(1 + 2 * 3, 0, 5)", &options).into_result()?;
//! for diagnostic in &diagnostics {
//!     eprintln!("{diagnostic}");
//! }
//! assert!(diagnostics.is_empty());
//!
//! let mut env = NoHostEnv::new();
//! env.variables.set(VariableName::new("speed"), Value::Float(1.5));
//! assert_eq!(expr.eval_f32(&mut env.cx()), 8.0);
//! # }
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```
//!
//! For untrusted content keep the default `vm::EvalLimits` and bounded sink, seed each
//! environment's random source, and evaluate on a thread with the stack `Expr::eval` documents.
//!
//! # Float behaviour
//!
//! The target architecture decides how floats are computed, when the crate is built: an `aarch64`
//! build rounds a multiply-add once, ignores a NaN operand in `math.min` / `math.max` and makes `<`
//! / `<=` true for a NaN operand; every other target rounds every operation on its own and makes
//! every comparison with a NaN false. The `numeric` module documents both and what other targets
//! reproduce; `numeric::ARCH` names the behaviour of a build.
//!
//! [`MolangSource`]: json::MolangSource
//! [`QueryCatalog`]: catalog::QueryCatalog
//! [`Side`]: catalog::Side

#![forbid(unsafe_code)]
#![warn(missing_docs)]
#![warn(clippy::pedantic)]
// Deliberate: integer literals wrap, array indices truncate, the exponent goes through `f64`.
#![allow(clippy::cast_possible_truncation)]
#![allow(clippy::cast_possible_wrap)]
#![allow(clippy::cast_sign_loss)]
#![allow(clippy::cast_precision_loss)]
// Truthiness and `==` are bit-exact f32 comparisons.
#![allow(clippy::float_cmp)]
// `#[must_use]` goes on methods returning a changed copy and on values whose loss is a bug, not
// on every pure getter.
#![allow(clippy::must_use_candidate)]
// The error enums' variants document each failure.
#![allow(clippy::missing_errors_doc)]
// Names such as `MolangVersion` in `version` stay descriptive.
#![allow(clippy::module_name_repetitions)]
#![cfg_attr(docsrs, feature(doc_cfg))]

/// Every public item has its path in its module; none resolves at the crate root.
///
/// ```compile_fail,E0432
/// use molangx::CompileOptions;
/// ```
/// ```compile_fail,E0423
/// let _ = molangx::compile;
/// ```
/// ```compile_fail,E0432
/// use molangx::EvalLimits;
/// ```
/// ```compile_fail,E0432
/// use molangx::Side;
/// ```
#[cfg(doctest)]
struct NoItemAtTheRoot;

/// Compiles the README's code blocks as doctests.
#[cfg(all(doctest, feature = "vm", feature = "stdlib"))]
#[doc = include_str!("../README.md")]
struct ReadmeDoctests;

mod bitmask;
#[cfg(feature = "cache")]
#[cfg_attr(docsrs, doc(cfg(feature = "cache")))]
pub mod cache;
pub mod catalog;
#[cfg(feature = "compiler")]
#[cfg_attr(docsrs, doc(cfg(feature = "compiler")))]
pub mod compile;
#[cfg(feature = "compiler")]
#[cfg_attr(docsrs, doc(cfg(feature = "compiler")))]
pub mod diag;
pub mod hash;
#[cfg(feature = "fuzz")]
#[doc(hidden)]
pub mod internals;
pub mod json;
#[cfg(feature = "compiler")]
#[cfg_attr(docsrs, doc(cfg(feature = "compiler")))]
pub mod numeric;
pub mod ops;
#[cfg(all(any(test, feature = "fuzz"), feature = "stdlib"))]
mod reference_catalog;
#[cfg(feature = "compiler")]
#[cfg_attr(docsrs, doc(cfg(feature = "compiler")))]
pub mod rng;
#[cfg(feature = "stdlib")]
#[cfg_attr(docsrs, doc(cfg(feature = "stdlib")))]
pub mod stdlib;
pub mod version;
#[cfg(feature = "vm")]
#[cfg_attr(docsrs, doc(cfg(feature = "vm")))]
pub mod vm;

#[cfg(all(test, not(feature = "stdlib")))]
compile_error!("the unit tests need the `stdlib` feature; without it run `--test engine_only`");
