//! One Molang text shared between packs of different versions, through a `CompileCache`.
//!
//! Run with `cargo run --example cache --features cache`.

use std::sync::Arc;

use molangx::cache::CompileCache;
use molangx::compile::{CompileOptions, Expr};
use molangx::json::MolangSource;
use molangx::version::MolangVersion;

fn main() {
    // Below MolangVersion 5 the ternary is left-grouped, below 6 `||` binds tighter than `&&`,
    // so this text means three different things.
    let text = "(1 ? 0 : 1 ? 2 : 3) + (1 || 0 && 0)";

    // The same text in five files of three packs: (path, MolangVersion).
    let uses = [
        ("old_pack/entity/a.json", 4),
        ("old_pack/entity/b.json", 4),
        ("mid_pack/entity/c.json", 5),
        ("new_pack/entity/d.json", 6),
        ("new_pack/entity/e.json", 6),
    ];

    // `compile_source` applies each source's own version to the field's options; the version is
    // part of the cache key.
    let field = CompileOptions::server(MolangVersion::LATEST);
    let cache = CompileCache::new();
    let mut compiled = Vec::new();
    for (path, version) in uses {
        let src = MolangSource::string(text, version);
        let result = cache.compile_source(&src, &field);
        let value = result.expr().and_then(Expr::as_constant);
        println!(
            "{path:<24} version {version}: folds to {value:?}, {} diagnostics",
            result.diagnostics().len()
        );
        compiled.push(result);
    }

    println!();
    println!(
        "{} uses, {} distinct (version, text) entries",
        uses.len(),
        cache.len()
    );
    println!("{} compiles, {} hits", cache.misses(), cache.hits());
    println!(
        "a.json and b.json share one program: {}",
        Arc::ptr_eq(&compiled[0], &compiled[1])
    );
    println!(
        "a.json (version 4) and d.json (version 6) do not: {}",
        !Arc::ptr_eq(&compiled[0], &compiled[3])
    );
}
