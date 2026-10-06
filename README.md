# molangx

[![Crates.io](https://img.shields.io/crates/v/molangx.svg)](https://crates.io/crates/molangx) [![docs.rs](https://img.shields.io/docsrs/molangx)](https://docs.rs/molangx) [![CI](https://github.com/bedrock-crustaceans/molangx/actions/workflows/ci.yml/badge.svg?branch=main)](https://github.com/bedrock-crustaceans/molangx/actions/workflows/ci.yml) [![License](https://img.shields.io/badge/license-Apache--2.0-blue.svg)](https://github.com/bedrock-crustaceans/molangx/blob/main/LICENSE)

A pure-Rust compiler and bytecode VM for [Molang](https://learn.microsoft.com/en-us/minecraft/creator/reference/content/molangreference/examples/molangconcepts/molangintroduction), the expression language of Minecraft: Bedrock Edition, at every Molang version. Part of the [bedrock-crustaceans](https://github.com/bedrock-crustaceans) ecosystem.

molangx compiles Molang text to bytecode and evaluates it. Queries are declared, not implemented: a `QueryCatalog` holds their names, argument counts, version windows and query sets, so the compiler can tell that `query.does_not_exist` does not resolve, while the implementations come from a `QueryTable` your embedding fills in, one function per query name; a query without one returns the default of its declaration. The crate is not a JSON loader either: a field that holds Molang reaches it through `MolangValueRepr` as a plain `JsonScalar`.

## Installation

molangx uses edition 2024 and needs Rust 1.92 or newer and `std`:

```toml
[dependencies]
molangx = { version = "0.1", features = ["vm"] }
```

`vm` is everything needed to compile and evaluate; the other flags are listed under [Features](#features). Without `stdlib` (`default-features = false`) the crate is the engine alone: no `math.*` name and no query is known, so a host declares its queries with `QueryCatalog::new`, its math functions in a `MathCatalog`, and compiles with `CompileOptions::new`.

## Quick start

`compile` never fails: a rejected expression compiles to the constant 0 with its diagnostics attached (`Compiled::expr_or_zero`), and `Compiled::into_result` gives a `Result` instead. `NoHostEnv` evaluates without a world (numbers, strings, `variable.` / `temp.` / `context.`, `math.*`); an embedding with a world uses `HostEnv` (see the [`host`](https://github.com/bedrock-crustaceans/molangx/blob/main/examples/host.rs) example).

```rust
use molangx::compile::{CompileOptions, compile};
use molangx::version::MolangVersion;
use molangx::vm::{NoHostEnv, Value, VariableName};

fn main() -> Result<(), molangx::compile::CompileError> {
    let options = CompileOptions::server(MolangVersion::LATEST);
    // A success carries what the compile logged (warnings, notes) along with the expression.
    let source = "v.speed * 2 + math.clamp(1 + 2 * 3, 0, 5)";
    let (expr, diagnostics) = compile(source, &options).into_result()?;
    for diagnostic in &diagnostics {
        eprintln!("{diagnostic}");
    }
    assert!(diagnostics.is_empty());

    let mut env = NoHostEnv::new();
    env.variables.set(VariableName::new("speed"), Value::Float(1.5));
    assert_eq!(expr.eval_f32(&mut env.cx()), 8.0);
    Ok(())
}
```

## Versions and sides

`MolangVersion` covers `-1` (`Invalid`) and `V0` to `V13`. `MolangVersion::from_engine_version_str` maps an engine-version text (`"1.18.10"` is `V5`), and an `EngineVersion` is `"*"` or a `semver::Version`. Queries resolve against the raw version (`RawVersion`): above 13 a text parses like 13 and resolves no query.

A `MolangSource` is text plus version. `compile_source(&source, &options)` compiles it at its own version, and `CompileCache::compile_source` does the same through the cache; `compile(text, &options)` uses the options' version instead.

`CompileOptions::server` and `CompileOptions::client` compile against `stdlib::queries(Side::Server)` and `stdlib::queries(Side::Client)`; there is no default side. For the catalogue of an older release, `stdlib::queries_at(side, &release)` takes a `semver::Version` (`semver` is re-exported as `molangx::version::semver`; a new major version of it is a breaking change of this crate).

## Queries of your own

Declare them in an extended catalogue, compile against it, and install the functions in a `QueryTable` built from it. `NoHostEnv::new()` and the default `HostEnv` hold no `QueryTable`, so every query returns its declared default. To change a query the catalogue already declares (a standard query's argument count or return type), build a catalogue with `QueryCatalog::overriding`, which replaces each declaration of the same name in place.

```rust
use molangx::catalog::{Arity, QueryDecl, QueryShape, ReturnType, Side};
use molangx::compile::{CompileOptions, compile};
use molangx::stdlib;
use molangx::version::MolangVersion;
use molangx::vm::{NoHost, NoHostEnv, QueryCx, QueryResult, QueryTable, Value};

fn double(cx: &mut QueryCx<'_, '_, NoHost>) -> QueryResult<NoHost> {
    Ok(Value::Float(cx.arg_f32(0).unwrap_or(0.0) * 2.0))
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let shape = QueryShape {
        args: Arity::exactly(1),
        returns: ReturnType::FLOAT,
        ..QueryShape::DEFAULT
    };
    let catalog = stdlib::queries(Side::Server).extended([QueryDecl::new("query.double", shape)?])?;
    let options = CompileOptions::new(catalog.clone(), MolangVersion::LATEST);
    let (expr, _) = compile("query.double(21)", &options).into_result()?;

    let mut queries = QueryTable::new(&catalog);
    queries.set("query.double", double)?;
    let mut env = NoHostEnv { queries: Some(queries), ..NoHostEnv::new() };
    assert_eq!(expr.eval_f32(&mut env.cx()), 42.0);
    Ok(())
}
```

## Contexts of your own

`context.*` is read-only and filled by the caller for one evaluation. Nothing is declared at compile time: any name compiles, and a read the provider does not answer is a missing variable, which ends the expression with 0 and logs a message unless `??` catches it. `NoHostEnv` and the default `HostEnv` hold a `ContextMap`, filled before each evaluation; `HostEnv::with_context` swaps in any `ContextProvider` of your own, one that reads straight from the host's state for instance (see the [`host`](https://github.com/bedrock-crustaceans/molangx/blob/main/examples/host.rs) example), and `NoContext` answers nothing.

```rust
use molangx::compile::{CompileOptions, compile};
use molangx::version::MolangVersion;
use molangx::vm::{ContextName, NoHostEnv, Value};

fn main() -> Result<(), molangx::compile::CompileError> {
    let options = CompileOptions::server(MolangVersion::LATEST);
    let (expr, _) = compile("c.damage * 0.5 + (c.bonus ?? 1)", &options).into_result()?;

    let mut env = NoHostEnv::new();
    env.context.set(ContextName::new("damage"), Value::Float(4.0));
    assert_eq!(expr.eval_f32(&mut env.cx()), 3.0);

    env.context.set(ContextName::new("bonus"), Value::Float(2.0));
    assert_eq!(expr.eval_f32(&mut env.cx()), 4.0);
    Ok(())
}
```

## Math functions of your own

Declare them in a `MathCatalog` and pass it in `CompileOptions::math`; `math.<name>(…)` then calls them like the standard functions, with float arguments (at most `MAX_MATH_ARGS`) and the argument count checked at compile time. A pure function must be deterministic: a call of constants is folded at compile time and equal calls are merged. A volatile function gets the evaluation's random source (`&mut dyn rand_core::Rng`; `rng::sample` draws a sample as the standard functions do), is never folded or merged, and is forbidden by `OpSet::without_assignments_or_random`. A function must not panic. A function may take a standard function's name (all but the constant `math.pi`) and then replaces the standard function while the catalogue is in the options, as an ordinary host call.

```rust
use molangx::catalog::{Arity, MathCatalog, MathDecl};
use molangx::compile::{CompileOptions, compile};
use molangx::rng::sample;
use molangx::version::MolangVersion;
use molangx::vm::NoHostEnv;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let math = MathCatalog::new([
        MathDecl::pure("math.hypot", Arity::exactly(2), |a| a[0].hypot(a[1]))?,
        MathDecl::volatile("math.jitter", Arity::exactly(1), |rng, a| a[0] * sample(rng))?,
    ])?;
    let options = CompileOptions {
        math: Some(math),
        ..CompileOptions::server(MolangVersion::LATEST)
    };
    let hypot = compile("math.hypot(3, 4)", &options);
    assert_eq!(hypot.expr().and_then(|e| e.as_constant()), Some(5.0));
    let (expr, _) = compile("math.jitter(10) + math.hypot(v.x ?? 3, 4)", &options).into_result()?;
    assert!((5.0..=15.0).contains(&expr.eval_f32(&mut NoHostEnv::new().cx())));
    Ok(())
}
```

## Features

`stdlib` is on by default. Every public item has one path, the module this table names (`molangx::compile::CompileOptions`); [docs.rs](https://docs.rs/molangx) shows the feature each item needs.

| Feature | Contents | Implies |
| --- | --- | --- |
| (always) | `version`: `MolangVersion`, `RawVersion`, `EngineVersion` and the engine-version map, experiments, the re-exported `semver`; `json`: `MolangSource`, `MolangValueRepr` and its readers; `hash`: `HashedStr`; `catalog`: the query catalogue and declaration types; `ops`: the operator table and `OpSet` | |
| `stdlib` | `stdlib`: the queries (`stdlib::queries`, `stdlib::query`), the `math.*` table (`MathFn`) and, with `compiler`, the math library (`stdlib::math`) and `CompileOptions::server` / `client` | |
| `compiler` | `compile`: `compile`, `CompileOptions`, `Compiled`, `Expr`, `ProgramFlags` and the limits (`MAX_DEPTH`, `MAX_SOURCE_LEN`, `MAX_DIAGNOSTICS`); `diag`: `Diagnostic` and `LanguageMessage`; `numeric`: the float semantics; `rng`: the random sources, over `rand_core` 0.10; `catalog::MathCatalog`: host math functions | |
| `vm` | `vm`: `Value`, the `Host` traits, variable storage and `EvalLimits`; `Expr::eval` | `compiler` |
| `cache` | `cache::CompileCache`, a concurrent compile cache keyed by version, options and source | `compiler` |
| `facet` | `Facet` impls for `json::MolangSource` and `json::MolangValueRepr` | |
| `fuzz` | test support, **no SemVer guarantee**: a hidden `internals` module read by the repository's fuzz crate | `stdlib` |

## Compile options

`CompileOptions` is a plain struct: start from `new`, `server` or `client` and name the fields you change.

| Field | Meaning | Default |
| --- | --- | --- |
| `catalog` | The `QueryCatalog` queries resolve against. | Given to the constructor. |
| `raw_version` | The `RawVersion` queries resolve with; parser gates use its effective value. | Given to the constructor. |
| `admission` | Which queries of the catalogue may be called: query sets or an allow-list. | The `default` query set. |
| `allowed_ops` | The `OpSet` of allowed operations. | All. |
| `experiments` | The enabled experiments (`ExperimentMask`). No standard query is experiment-gated. | None. |
| `math` | The host's own `math.*` functions (`MathCatalog`). | None. |
| `keep_source` | Whether the compiled expression keeps the source text. | `false` |
| `deviations` | The compile switches below. | `Deviations::DEFAULT` |

`Deviations::NONE` turns every switch off.

| Switch | On | Off | Default |
| --- | --- | --- | --- |
| `source_length_limit` | A source longer than `MAX_SOURCE_LEN` (65,536 bytes) is rejected with `DiagCode::SourceTooLong`. | No length limit. | on |
| `validate_nested` | Validation findings below the root are warnings. | They are errors. Either way the expression compiles. | on |
| `true_false_prefix_advance` | A prefix of `true` / `false` (`t`, `tr`, `fals`) is a boolean token and the lexer advances by its length. | The lexer advances four bytes for `true`, five for `false`. | on |
| `query_arity_lint` | A warning for a query call with an argument count outside its declared range. | No warning. | on |
| `query_client_only` | An informational note for a client-only query compiled against a server catalogue. | No note. | on |
| `object_version_warning` | A warning for an object-form version outside -1..=13, a note for `Invalid`. | No diagnostic. | on |
| `diagnostic_limit` | At most `MAX_DIAGNOSTICS` (256) diagnostics, then one `DiagCode::DiagnosticLimit` note counting the rest. | Every diagnostic is kept. | on |

```rust
use molangx::compile::{CompileOptions, Deviations};
use molangx::version::MolangVersion;

let options = CompileOptions {
    keep_source: true,
    deviations: Deviations { validate_nested: false, ..Deviations::DEFAULT },
    ..CompileOptions::server(MolangVersion::LATEST)
};
assert_eq!(options.version(), MolangVersion::LATEST);
```

## Float behaviour

The target architecture chooses the float behaviour when the crate is built (`numeric::ARCH`); there is no option for it.

On `aarch64` (`Arch::Arm64`), `x·S + O` and the other multiply-adds are rounded once, `math.min` / `math.max` ignore a NaN operand, `<` / `<=` are true for a NaN operand (so a `loop` with a NaN count makes no pass), a NaN divisor gives 0 and a NaN `math.asin` / `math.acos` argument counts as −1. An arithmetic operation with NaN operands returns the first signalling one quietened, else the first quiet one; an invalid one gives `0x7fc00000`.

On x86-64 and every other target (`Arch::X86_64`), every multiply, add and divide is rounded on its own in formula order, every comparison with a NaN is false and `math.min` / `math.max` return their second operand when either is NaN. An arithmetic operation with NaN operands returns its left NaN operand quietened; an invalid one gives `0xffc00000`. `math.max` / `math.min`, the comparisons and the sorting of random and die-roll bounds are not arithmetic operations and treat a NaN by their own rules.

The easings, `math.hermite_blend`, the random interpolation and the inverse-trigonometric degree conversion also differ in shape. A target other than x86-64 and aarch64 (wasm32, riscv64, 32-bit arm, 32-bit x86 with SSE2, …) has the `X86_64` behaviour and gives the same bits for every result that is not a NaN; 32-bit x86 without SSE2 does not compile. The arithmetic and the standard functions choose the sign and payload of the NaNs they return, so those match too, except where an instruction of the target makes the NaN: `math.floor`, `math.ceil`, `math.round` and `math.trunc` of a NaN, a NaN bound of `math.die_roll` / `math.die_roll_integer` (floored), and the result of a comparison or logical node whose folded post-op holds a NaN. On 32-bit x86 a value returned through the x87 register may have a signalling NaN quietened.

## Evaluation

`EvalLimits` budgets one evaluation. Each budget is an `Option` (`None` is no limit); `EvalLimits::DEFAULT` is the column below and `EvalLimits::NONE` sets none. Keep a `total_steps` budget for untrusted content: a member store pays steps for the structs it copies, so the step budget bounds memory too. The operand stack is capped at 65,536 slots under every budget.

| Field | Meaning | Default |
| --- | --- | --- |
| `loop_iterations` | Iterations one `loop` / `for_each` may run before it is left. | 1,024 |
| `total_steps` | Steps one evaluation may take before it ends with 0. | 1,048,576 |
| `struct_depth` | Struct levels a member store may leave in the variable or temp it writes; a deeper store ends the evaluation with 0. | 32 |
| `struct_members` | Members a member store may leave in a struct it adds a member to; a wider store ends the evaluation with 0. | 256 |
| `query_depth` | How deep query arguments may nest at run time (the top level is 0); `Some(0)` lets no query evaluate an argument. | None |

```rust
use molangx::vm::EvalLimits;

let limits = EvalLimits {
    loop_iterations: Some(5), // a tighter per-loop budget
    struct_depth: None,       // no struct depth budget
    query_depth: Some(0),     // no query may evaluate an argument
    ..EvalLimits::DEFAULT
};
assert!(!limits.is_unlimited());
```

`temp.*` starts empty in every evaluation; `temps: Temps::Kept(TempMap::new())` on a `HostEnv` or `NoHostEnv` keeps them in a map nothing clears.

The random source is any `rand_core` 0.10 `Rng`, re-exported as `molangx::rng::rand_core` (a new major version of `rand_core` is a breaking change of this crate). Each sample is one `next_u32` `u` mapped to `(u & 0x7fff_ffff) · 2^-31` (`rng::sample`), which can be exactly 1. Each environment has its own `Xorshift128` started from the same standard seeds, so give each one `Xorshift128::seed_from_u64(...)` (`SeedableRng`) unless identical sequences are wanted; `HostEnv::with_rng(ProcessRng)` gives one process-wide generator, `HostEnv::with_rng(generator)` a generator of your own, and `FixedRng` the same sample on every draw.

The default message sink, `BoundedSink`, keeps the last 64 messages, each cut to 1 KiB; `LogOnce` drops repeats.

The deepest nesting `compile` accepts needs about 320 KiB of stack in a release build and 1.9 MiB in a debug build; evaluate on threads of at least 512 KiB / 4 MiB, or set `EvalLimits::query_depth`. Structs share members, so walk a stored value with `Value::distinct_structs` or bound the walk.

## Examples

Every example prints what it demonstrates. Start with `hello_world`: `cargo run --example hello_world --features vm`.

| Example | What it covers | Features |
| --- | --- | --- |
| [`hello_world`](https://github.com/bedrock-crustaceans/molangx/blob/main/examples/hello_world.rs) | Compile and evaluate with no host; variables, a loop and a missing read. | `vm` |
| [`versions`](https://github.com/bedrock-crustaceans/molangx/blob/main/examples/versions.rs) | `MolangVersion::from` an `EngineVersion`, and one text grouped differently at versions 4, 5 and 6. | `compiler` |
| [`json_forms`](https://github.com/bedrock-crustaceans/molangx/blob/main/examples/json_forms.rs) | `MolangValueRepr` round trip of the number, string, `{expression, version}` and bool forms. | none |
| [`diagnostics`](https://github.com/bedrock-crustaceans/molangx/blob/main/examples/diagnostics.rs) | Error texts, reject versus keep, byte spans, and the crate's own lints. | `compiler` |
| [`restricted`](https://github.com/bedrock-crustaceans/molangx/blob/main/examples/restricted.rs) | Query sets, allow-lists and the side-effect-free contexts (`without_assignments`, `without_assignments_or_random`). | `compiler` |
| [`host`](https://github.com/bedrock-crustaceans/molangx/blob/main/examples/host.rs) | Implementing `Host`, `VariableStore`, `ContextProvider` and queries over a toy world, evaluated through `HostEnv`. | `vm` |
| [`variables`](https://github.com/bedrock-crustaceans/molangx/blob/main/examples/variables.rs) | `v.` / `t.` / `c.` scope, `??`, the missing-variable abort, `->` and public variables. | `vm` |
| [`math`](https://github.com/bedrock-crustaceans/molangx/blob/main/examples/math.rs) | Host math functions in a `MathCatalog`: a pure one folded, a volatile one drawing, the argument-count error. | `vm` |
| [`cache`](https://github.com/bedrock-crustaceans/molangx/blob/main/examples/cache.rs) | `CompileCache` compiling one text at several versions. | `cache` |

## Contributing

See [CONTRIBUTING.md](https://github.com/bedrock-crustaceans/molangx/blob/main/CONTRIBUTING.md).

## License

Licensed under the **Apache License 2.0**; see [LICENSE](https://github.com/bedrock-crustaceans/molangx/blob/main/LICENSE).
