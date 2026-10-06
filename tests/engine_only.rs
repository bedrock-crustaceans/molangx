//! The engine with a host's own catalogues: passes with and without the `stdlib` feature.

#![cfg(feature = "compiler")]

use molangx::catalog::{Arity, QueryCatalog, QueryDecl, QueryShape, ReturnType, Side};
use molangx::compile::{CompileFailure, CompileOptions, compile};
use molangx::ops::{ExpressionOp, OpSet};
use molangx::version::MolangVersion;

/// `query.double(x)` and `query.actors`.
fn catalog() -> QueryCatalog {
    let double = QueryShape {
        args: Arity::exactly(1),
        returns: ReturnType::FLOAT,
        ..QueryShape::DEFAULT
    };
    let actors = QueryShape {
        returns: ReturnType::ACTOR_ARRAY,
        ..QueryShape::DEFAULT
    };
    let decls = [
        QueryDecl::new("query.double", double).expect("a declaration"),
        QueryDecl::new("query.actors", actors).expect("a declaration"),
    ];
    QueryCatalog::new(Side::Server, decls).expect("a catalogue")
}

fn messages(src: &str, options: &CompileOptions) -> Vec<String> {
    compile(src, options)
        .diagnostics()
        .iter()
        .map(ToString::to_string)
        .collect()
}

#[test]
fn the_op_table_is_the_same_in_both_builds() {
    assert_eq!(OpSet::all().len(), 111);
    assert_eq!(ExpressionOp::Sin.token(), Some("math.sin"));
    assert!(ExpressionOp::Sin.is_math_function());
}

#[test]
fn a_host_catalogue_resolves_its_queries() {
    let catalog = catalog();
    let options = CompileOptions::new(catalog, MolangVersion::LATEST);
    assert!(compile("query.double(21) + 1", &options).is_success());
    assert_eq!(messages("query.double(21)", &options), Vec::<String>::new());
    assert_eq!(
        compile("array.skins[v.index]", &options).failure(),
        Some(CompileFailure::UsesArrays)
    );
}

/// Every standard function's token with its fewest arguments (`math.pi` bare).
fn standard_calls() -> Vec<(String, String)> {
    let ops: Vec<ExpressionOp> = ExpressionOp::all()
        .iter()
        .copied()
        .filter(|op| op.is_math_function())
        .collect();
    assert_eq!(ops.len(), 61);
    ops.into_iter()
        .map(|op| {
            let token = op.token().expect("a standard function has a token");
            // A stand-in of the same length, so the messages differ only in the name.
            let stand_in = format!("math.{}", "z".repeat(token.len() - "math.".len()));
            let args = if op.max_children() == Some(0) {
                String::new()
            } else {
                format!("({})", vec!["1"; usize::from(op.min_children())].join(", "))
            };
            (format!("{token}{args}"), format!("{stand_in}{args}"))
        })
        .collect()
}

#[test]
fn the_standard_names_are_known_only_with_the_feature() {
    let catalog = catalog();
    let options = CompileOptions::new(catalog, MolangVersion::LATEST);
    for (src, stand_in) in standard_calls() {
        let unknown: Vec<String> = messages(&stand_in, &options)
            .iter()
            .map(|m| m.replace(&stand_in, &src))
            .collect();
        assert!(!unknown.is_empty(), "{stand_in}");
        if cfg!(feature = "stdlib") {
            assert!(
                compile(&src, &options).is_success(),
                "{src}: {:?}",
                messages(&src, &options)
            );
        } else {
            assert_eq!(messages(&src, &options), unknown, "{src}");
        }
    }
    let constant = |src: &str| {
        compile(src, &options)
            .expr()
            .and_then(molangx::compile::Expr::as_constant)
    };
    if cfg!(feature = "stdlib") {
        assert!((constant("math.sin(90)").expect("folded") - 1.0).abs() < 1e-6);
        assert_eq!(constant("math.pi"), Some(std::f32::consts::PI));
    }
}

#[cfg(feature = "vm")]
mod evaluation {
    use molangx::catalog::{Arity, MathCatalog, MathDecl, QueryCatalog};
    use molangx::compile::{CompileOptions, compile};
    use molangx::rng::{FixedRng, sample};
    use molangx::version::MolangVersion;
    use molangx::vm::{NoHost, NoHostEnv, QueryCx, QueryResult, QueryTable, Value, VariableName};

    fn double(cx: &mut QueryCx<'_, '_, NoHost>) -> QueryResult<NoHost> {
        Ok(Value::Float(cx.arg_f32(0).unwrap_or(0.0) * 2.0))
    }

    fn two_actors(_: &mut QueryCx<'_, '_, NoHost>) -> QueryResult<NoHost> {
        Ok(Value::actor_array([(), ()]))
    }

    /// `src` compiled against the host catalogue and evaluated with `query.double` installed.
    fn eval_with(src: &str, math: Option<&MathCatalog>, mut rng: FixedRng) -> f32 {
        let catalog: QueryCatalog = super::catalog();
        let options = CompileOptions {
            math: math.cloned(),
            ..CompileOptions::new(catalog.clone(), MolangVersion::LATEST)
        };
        let (expr, _) = compile(src, &options)
            .into_result()
            .unwrap_or_else(|error| panic!("{src}: {error}"));
        let mut queries = QueryTable::new(&catalog);
        queries.set("query.double", double).expect("declared");
        let mut env = NoHostEnv {
            queries: Some(queries),
            ..NoHostEnv::new()
        };
        env.variables.set(VariableName::new("x"), Value::Float(3.0));
        let mut cx = env.cx();
        cx.rng = &mut rng;
        expr.eval_f32(&mut cx)
    }

    fn eval(src: &str) -> f32 {
        eval_with(src, None, FixedRng::HALF)
    }

    /// Over the empty array of the uninstalled `query.actors`, `for_each` skips the instruction
    /// after it, so the constant store `v.f = 1` is lost; over a non-empty array nothing is
    /// skipped.
    #[test]
    fn for_each_over_a_host_actor_array() {
        let src = "v.f = 5; for_each(t.e, query.actors, {v.n = 1;}); v.f = 1; return v.f + 1;";
        assert_eq!(eval(src), 6.0);
        let catalog: QueryCatalog = super::catalog();
        let (expr, _) = compile(
            src,
            &CompileOptions::new(catalog.clone(), MolangVersion::LATEST),
        )
        .into_result()
        .expect("compiles");
        let mut queries = QueryTable::new(&catalog);
        queries.set("query.actors", two_actors).expect("declared");
        let mut env = NoHostEnv {
            queries: Some(queries),
            ..NoHostEnv::new()
        };
        assert_eq!(expr.eval_f32(&mut env.cx()), 2.0);
    }

    #[test]
    fn operators_loops_temps_strings_and_post_ops() {
        assert_eq!(eval("query.double(v.x) + 1"), 7.0);
        assert_eq!(eval("v.x * 2 + 1"), 7.0);
        assert_eq!(eval("-(v.x - 5) / 4"), 0.5);
        assert_eq!(eval("v.x > 2 && v.x <= 3 ? 10 : 20"), 10.0);
        assert_eq!(eval("(v.y ?? 4) + !(v.x == 3)"), 4.0);
        assert_eq!(
            eval("t.n = 0; loop(4, { t.n = t.n + v.x; }); return t.n;"),
            12.0
        );
        assert_eq!(
            eval("t.n = 0; loop(10, { t.n = t.n + 1; t.n > 2 ? break; }); return t.n;"),
            3.0
        );
        assert_eq!(eval("v.name = 'abc'; return v.name == 'abc';"), 1.0);
    }

    #[test]
    fn a_host_function_may_be_named_like_a_standard_one() {
        let math =
            MathCatalog::new([
                MathDecl::pure("math.sin", Arity::exactly(1), |a| a[0] * 2.0)
                    .expect("a declaration"),
            ])
            .expect("a catalogue");
        assert_eq!(
            eval_with("math.sin(v.x) + 1", Some(&math), FixedRng::HALF),
            7.0
        );
        assert_eq!(eval_with("math.sin(4)", Some(&math), FixedRng::HALF), 8.0);
    }

    #[test]
    fn a_volatile_host_function_draws_from_the_random_source() {
        let math =
            MathCatalog::new([
                MathDecl::volatile("math.draw", Arity::exactly(1), |rng, a| a[0] + sample(rng))
                    .expect("a declaration"),
            ])
            .expect("a catalogue");
        assert_eq!(
            eval_with(
                "math.draw(v.x)",
                Some(&math),
                FixedRng::from_sample(0.25).expect("a word's sample")
            ),
            3.25
        );
        assert_eq!(
            eval_with("math.draw(1) + math.draw(1)", Some(&math), FixedRng::ONE),
            4.0
        );
    }
}
