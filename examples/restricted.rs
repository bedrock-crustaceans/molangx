//! Restricted contexts: query sets, allow-lists and the side-effect-free contexts
//! (`without_assignments`, `without_assignments_or_random`).
//!
//! Run with `cargo run --example restricted --features compiler`.

use molangx::catalog::{QueryAdmission, QueryAllowList, QuerySetMask};
use molangx::compile::{CompileOptions, compile};
use molangx::ops::OpSet;
use molangx::stdlib::query;
use molangx::version::MolangVersion;

fn check(source: &str, options: &CompileOptions) {
    let compiled = compile(source, options);
    let verdict = if compiled.is_success() {
        "accepted"
    } else {
        "rejected"
    };
    println!("  {source:<70} {verdict}");
    for d in compiled.diagnostics() {
        println!("      {}", d.message());
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let latest = CompileOptions::server(MolangVersion::LATEST);
    let in_sets = |sets| CompileOptions {
        admission: QueryAdmission::Sets(sets),
        ..latest.clone()
    };

    // The standard queries are in three sets; options admit `default` unless told otherwise.
    println!("query sets (default config uses the `default` set):");
    check("query.is_baby", &latest);
    check("query.noise(1, 2)", &latest);
    println!("    ... with the world_gen set:");
    check("query.noise(1, 2)", &in_sets(QuerySetMask::WORLD_GEN));
    println!("    ... with the tags set:");
    check(
        "query.any_tag('minecraft:log')",
        &in_sets(QuerySetMask::TAGS),
    );
    let both = QuerySetMask::DEFAULT.union(QuerySetMask::WORLD_GEN);
    println!("    ... with default + world_gen:");
    check("query.is_baby + query.noise(1, 2)", &in_sets(both));

    println!();
    println!("entity property default (allow-list [had_component_group], without_assignments()):");
    // An allow-list is built once against the catalogue; a name the catalogue does not declare
    // is an error here, not a list that silently admits less.
    let property_queries = QueryAllowList::new(&latest.catalog, [query::HAD_COMPONENT_GROUP])?;
    let property = CompileOptions {
        admission: QueryAdmission::Only(property_queries),
        allowed_ops: OpSet::all().without_assignments(),
        ..latest.clone()
    };
    check(
        "query.had_component_group('minecraft:baby') ? 1 : math.random(0, 1)",
        &property,
    );
    check("query.is_baby", &property);
    check("v.x = 1;", &property);

    println!();
    println!(
        "block permutation condition (allow-list [block_state], without_assignments_or_random()):"
    );
    let block_queries = QueryAllowList::new(&latest.catalog, [query::BLOCK_STATE])?;
    let block = CompileOptions {
        admission: QueryAdmission::Only(block_queries),
        allowed_ops: OpSet::all().without_assignments_or_random(),
        ..latest.clone()
    };
    check("query.block_state('facing') == 'west'", &block);
    check("math.random(0, 1) > 0.5", &block);
    check("math.die_roll(1, 0, 1) > 0.5", &block);

    println!();
    match block.admission {
        QueryAdmission::Sets(sets) => println!("the block condition admits the sets {sets:?}"),
        QueryAdmission::Only(list) => {
            println!("an allow-list replaces the query sets: only {list:?}")
        }
    }

    println!();
    let (expr, _) = compile("v.x = 1; return math.random(0, 1);", &latest).into_result()?;
    println!("assigns: {}", expr.assigns());
    println!(
        "has_side_effects(include_random = false): {}",
        expr.has_side_effects(false)
    );
    let (expr, _) = compile("math.random(0, 1)", &latest).into_result()?;
    println!(
        "math.random(0, 1) has_side_effects(false/true): {}/{}",
        expr.has_side_effects(false),
        expr.has_side_effects(true)
    );
    Ok(())
}
