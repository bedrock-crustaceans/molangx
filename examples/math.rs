//! Host math functions: declare `math.*` functions of your own in a `MathCatalog`, pass it in the
//! compile options, and call them like the standard ones. A pure function of constants is folded at
//! compile time; a volatile one draws from the evaluation's random source and is never folded. A
//! function may take a standard function's name and replace it while the catalogue is in the
//! options.
//!
//! Run with `cargo run --example math --features vm`.

use molangx::catalog::{Arity, MathCatalog, MathDecl};
use molangx::compile::{CompileOptions, compile};
use molangx::numeric::PostOp;
use molangx::rng::sample;
use molangx::stdlib::math;
use molangx::version::MolangVersion;
use molangx::vm::{NoHostEnv, Value, VariableName};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let catalog = MathCatalog::new([
        // Pure: a call of constants is folded at compile time.
        MathDecl::pure("math.smooth_max", Arity::exactly(2), |a| {
            math::max(a[0], a[1], PostOp::IDENTITY) * 0.9
        })?,
        // Volatile: `a + b · r` for one draw `r`.
        MathDecl::volatile("math.gauss", Arity::exactly(2), |rng, a| {
            a[0] + a[1] * (sample(rng) - 0.5)
        })?,
        // An override: `math.clamp` that also accepts its bounds in either order. `f32::clamp`
        // would panic on NaN bounds.
        MathDecl::pure("math.clamp", Arity::exactly(3), |a| {
            a[0].max(a[1].min(a[2])).min(a[1].max(a[2]))
        })?,
    ])?;
    let options = CompileOptions {
        math: Some(catalog),
        ..CompileOptions::server(MolangVersion::LATEST)
    };

    let folded = compile("math.smooth_max(2, 10)", &options).into_result()?.0;
    println!("math.smooth_max(2, 10) folds to {:?}", folded.as_constant());

    let (expr, _) =
        compile("math.smooth_max(v.speed, 1) + math.gauss(0, 0.1)", &options).into_result()?;
    let mut env = NoHostEnv::new();
    env.variables
        .set(VariableName::new("speed"), Value::Float(4.0));
    for _ in 0..3 {
        println!("with v.speed = 4: {}", expr.eval_f32(&mut env.cx()));
    }

    let swapped = "math.clamp(5, 3, 0)";
    println!(
        "{swapped}: {:?} with the override",
        compile(swapped, &options).into_result()?.0.as_constant()
    );
    let standard = compile(swapped, &CompileOptions::server(MolangVersion::LATEST))
        .into_result()?
        .0;
    println!(
        "{swapped}: {:?} with the standard function",
        standard.as_constant()
    );

    let wrong = compile("math.smooth_max(1, 2, 3)", &options);
    println!("math.smooth_max(1, 2, 3): {}", wrong.diagnostics()[0]);
    let unknown = compile(
        "math.smooth_max(1, 2)",
        &CompileOptions::server(MolangVersion::LATEST),
    );
    println!("without the catalogue: {}", unknown.diagnostics()[0]);
    Ok(())
}
