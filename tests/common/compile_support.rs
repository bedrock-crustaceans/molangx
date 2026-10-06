//! Compile helpers. Versions are raw: a test may compile at a version outside −1..=13.

use molangx::catalog::Side;
use molangx::compile::{CompileOptions, Compiled, Expr, compile};
use molangx::version::RawVersion;

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

/// Compiles for the client.
pub fn at(source: &str, version: i16) -> Compiled {
    compile(source, &client_at(version))
}

/// The expression `source` compiles to; panics with the diagnostics when it compiles to none.
pub fn compile_expr(source: &str, options: &CompileOptions) -> Expr {
    let compiled = compile(source, options);
    compiled
        .expr()
        .cloned()
        .unwrap_or_else(|| panic!("{source:?}: {:?}", compiled.diagnostics()))
}

/// [`compile_expr`] for the client at version 13.
pub fn client_expr(source: &str) -> Expr {
    client_expr_at(source, 13)
}

/// [`compile_expr`] for the client.
pub fn client_expr_at(source: &str, version: i16) -> Expr {
    compile_expr(source, &client_at(version))
}

/// [`compile_expr`] for the server at version 13.
pub fn server_expr(source: &str) -> Expr {
    server_expr_at(source, 13)
}

/// [`compile_expr`] for the server.
pub fn server_expr_at(source: &str, version: i16) -> Expr {
    compile_expr(source, &server_at(version))
}

/// The language-log messages of a compile, in order, without their trailing newlines.
pub fn messages(compiled: &Compiled) -> Vec<String> {
    compiled
        .diagnostics()
        .iter()
        .filter(|d| d.language_message().is_some())
        .map(|d| d.message().trim_end_matches('\n').to_owned())
        .collect()
}

/// The ids of a compile's language messages (`E38`), in order.
pub fn message_ids(compiled: &Compiled) -> Vec<&'static str> {
    compiled
        .diagnostics()
        .iter()
        .filter_map(|d| d.language_message())
        .map(|m| m.id())
        .collect()
}

pub fn messages_at(source: &str, version: i16) -> Vec<String> {
    messages(&at(source, version))
}

/// The value `source` folds to at version 13; panics if it does not fold.
pub fn constant(source: &str) -> f32 {
    let compiled = at(source, 13);
    compiled
        .expr()
        .and_then(Expr::as_constant)
        .filter(|_| compiled.parsed())
        .unwrap_or_else(|| panic!("{source:?} is not a constant: {:?}", messages(&compiled)))
}
