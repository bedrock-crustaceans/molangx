//! Evaluating a `FLOAT_ONLY` program, with `eval_f32` or `eval`, performs no heap allocation.

#![cfg(all(feature = "vm", feature = "stdlib"))]

use molangx::catalog::Side;
use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

use molangx::compile::{CompileOptions, ProgramFlags, compile};
use molangx::rng::{Xorshift128, sample};
use molangx::stdlib::query;
use molangx::version::MolangVersion;
use molangx::vm::{
    ContextMap, ContextName, EvalCx, EvalLimits, NoContext, NoHost, NullSink, QueryCx, QueryResult,
    QueryTable, Subjects, Temps, Value, VariableMap, VariableName,
};

struct Counting;

thread_local! {
    static ALLOCATIONS: Cell<usize> = const { Cell::new(0) };
}

// SAFETY: every method forwards to `System` unchanged; the `const`-initialised thread-local
// counter does not allocate.
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        ALLOCATIONS.with(|count| count.set(count.get() + 1));
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        ALLOCATIONS.with(|count| count.set(count.get() + 1));
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}

#[global_allocator]
static GLOBAL: Counting = Counting;

fn allocations() -> usize {
    ALLOCATIONS.with(Cell::get)
}

#[allow(clippy::unnecessary_wraps, reason = "a query returns a `QueryResult`")]
fn one(_cx: &mut QueryCx<'_, '_, NoHost>) -> QueryResult<NoHost> {
    Ok(Value::Float(1.0))
}

#[allow(clippy::unnecessary_wraps, reason = "a query returns a `QueryResult`")]
fn first_plus_one(cx: &mut QueryCx<'_, '_, NoHost>) -> QueryResult<NoHost> {
    Ok(Value::Float(cx.arg_f32(0).unwrap_or(0.0) + 1.0))
}

#[test]
fn float_only_evaluation_does_not_allocate() {
    let sources = [
        "math.clamp(v.x * 2, 0, 5)",
        "t.a = 1; loop(10, {t.a = t.a * 2;}); return t.a;",
        "v.y = v.x * 3 + math.sin(v.x); return v.y > 1 ? v.y : -v.y;",
        "return (v.never_set ?? 5) + math.random(1, 2) + math.die_roll_integer(3, 1, 6);",
        "v.c = 0; loop(100, {v.c = v.c + 1; v.c > 50 ? break;}); return v.c && this;",
        "v.y = v.x / (v.x - 3) + math.mod(v.x, 4) + math.ease_in_out_elastic(0, 1, v.x / 10);",
        "v.missing_aborts + 1",
    ];
    let mut vars = VariableMap::<NoHost>::new();
    vars.set(VariableName::new("x"), Value::Float(3.5));
    vars.set(VariableName::new("y"), Value::Float(0.0));
    vars.set(VariableName::new("c"), Value::Float(0.0));
    let mut host = NoHost;
    let mut rng = Xorshift128::new();
    let mut sink = NullSink;
    for source in sources {
        let expr = compile(source, &CompileOptions::server(MolangVersion::LATEST))
            .expr()
            .cloned()
            .expect("an expression");
        assert!(expr.flags().contains(ProgramFlags::FLOAT_ONLY), "{source}");
        let mut cx = EvalCx {
            subjects: Subjects {
                this: 2.0,
                ..Subjects::none()
            },
            host: &mut host,
            variables: &mut vars,
            context: &NoContext,
            queries: None,
            rng: &mut rng,
            sink: &mut sink,
            limits: EvalLimits::DEFAULT,
            temps: Temps::PerEvaluation,
        };
        // The first run may grow the variable map.
        let first = expr.eval_f32(&mut cx);
        let before = allocations();
        let again = expr.eval_f32(&mut cx);
        let value = expr.eval(&mut cx);
        let after = allocations();
        assert_eq!(
            after - before,
            0,
            "{source}: {} allocations",
            after - before
        );
        assert!(
            first.to_bits() == again.to_bits()
                || source.contains("random")
                || source.contains("v.c"),
            "{source}"
        );
        assert!(matches!(value, Value::Float(_)), "{source}");
    }

    // Covers a call by position (same catalogue), by name (other catalogue) and no table.
    let client = QueryTable::<NoHost>::new(molangx::stdlib::queries(Side::Client));
    for queries in [Some(&client), None] {
        for options in [
            CompileOptions::client(MolangVersion::LATEST),
            CompileOptions::server(MolangVersion::LATEST),
        ] {
            let expr = compile("q.is_baby + q.position(v.x) * 2 + q.life_time", &options)
                .expr()
                .cloned()
                .expect("an expression");
            let mut cx = EvalCx {
                subjects: Subjects::none(),
                host: &mut host,
                variables: &mut vars,
                context: &NoContext,
                queries,
                rng: &mut rng,
                sink: &mut sink,
                limits: EvalLimits::DEFAULT,
                temps: Temps::PerEvaluation,
            };
            let _ = expr.eval(&mut cx);
            let before = allocations();
            let value = expr.eval(&mut cx);
            let float = expr.eval_f32(&mut cx);
            assert_eq!(
                allocations() - before,
                0,
                "{:?} with {queries:?}",
                options.catalog
            );
            assert_eq!((value, float), (Value::Float(0.0), 0.0));
        }
    }

    let mut implemented = QueryTable::<NoHost>::new(molangx::stdlib::queries(Side::Client));
    implemented
        .set(query::IS_BABY, one)
        .expect("a standard query name");
    implemented
        .set(query::POSITION, first_plus_one)
        .expect("a standard query name");
    for options in [
        CompileOptions::client(MolangVersion::LATEST),
        CompileOptions::server(MolangVersion::LATEST),
    ] {
        let expr = compile("q.is_baby + q.position(v.x) * 2 + q.life_time", &options)
            .expr()
            .cloned()
            .expect("an expression");
        let mut cx = EvalCx {
            subjects: Subjects::none(),
            host: &mut host,
            variables: &mut vars,
            context: &NoContext,
            queries: Some(&implemented),
            rng: &mut rng,
            sink: &mut sink,
            limits: EvalLimits::DEFAULT,
            temps: Temps::PerEvaluation,
        };
        let _ = expr.eval(&mut cx);
        let before = allocations();
        let value = expr.eval(&mut cx);
        let float = expr.eval_f32(&mut cx);
        assert_eq!(allocations() - before, 0, "{:?}", options.catalog);
        // 1 + (3.5 + 1) * 2 + 0: `q.life_time` is a stub.
        assert_eq!(
            (value, float),
            (Value::Float(10.0), 10.0),
            "{:?}",
            options.catalog
        );
    }

    let context = ContextMap::<NoHost>::from([
        (ContextName::new("k"), Value::Float(4.0)),
        (ContextName::new("other"), Value::Actor(())),
    ]);
    for source in [
        "context.k * 2 + c.k",
        "c.missing ?? c.k",
        "return c.k > 3 ? c.k : -1;",
    ] {
        let expr = compile(source, &CompileOptions::server(MolangVersion::LATEST))
            .expr()
            .cloned()
            .expect("an expression");
        let mut cx = EvalCx {
            subjects: Subjects::none(),
            host: &mut host,
            variables: &mut vars,
            context: &context,
            queries: None,
            rng: &mut rng,
            sink: &mut sink,
            limits: EvalLimits::DEFAULT,
            temps: Temps::PerEvaluation,
        };
        let _ = expr.eval(&mut cx);
        let before = allocations();
        let value = expr.eval(&mut cx);
        let float = expr.eval_f32(&mut cx);
        assert_eq!(allocations() - before, 0, "{source}");
        assert_eq!(value, Value::Float(float), "{source}");
        assert!(float == 12.0 || float == 4.0, "{source}: {float}");
    }

    // Proves the counter works: building a struct value allocates.
    let expr = compile("v.s.a = 1;", &CompileOptions::server(MolangVersion::LATEST))
        .expr()
        .cloned()
        .expect("an expression");
    let mut cx = EvalCx {
        subjects: Subjects::none(),
        host: &mut host,
        variables: &mut vars,
        context: &NoContext,
        queries: None,
        rng: &mut rng,
        sink: &mut sink,
        limits: EvalLimits::DEFAULT,
        temps: Temps::PerEvaluation,
    };
    let before = allocations();
    expr.eval(&mut cx);
    assert!(allocations() > before);
}

#[test]
fn a_host_math_call_does_not_allocate() {
    use molangx::catalog::{Arity, MathCatalog, MathDecl};
    let math = MathCatalog::new([
        MathDecl::pure("math.span", Arity::between(1, 8), |a| a.iter().sum())
            .expect("a declaration"),
        MathDecl::volatile("math.draw", Arity::exactly(1), |rng, a| a[0] + sample(rng))
            .expect("a declaration"),
    ])
    .expect("a catalogue");
    let options = CompileOptions {
        math: Some(math),
        ..CompileOptions::server(MolangVersion::LATEST)
    };
    let mut vars = VariableMap::<NoHost>::new();
    vars.set(VariableName::new("x"), Value::Float(3.5));
    let mut host = NoHost;
    let mut rng = Xorshift128::new();
    let mut sink = NullSink;
    for source in [
        "math.span(v.x) * 2 + 1",
        "math.span(v.x, 1, 2, 3, 4, 5, 6, v.x)",
        "math.draw(v.x) + math.span(math.draw(1), v.x)",
    ] {
        let expr = compile(source, &options)
            .expr()
            .cloned()
            .expect("an expression");
        assert!(expr.flags().contains(ProgramFlags::FLOAT_ONLY), "{source}");
        let mut cx = EvalCx {
            subjects: Subjects::none(),
            host: &mut host,
            variables: &mut vars,
            context: &NoContext,
            queries: None,
            rng: &mut rng,
            sink: &mut sink,
            limits: EvalLimits::DEFAULT,
            temps: Temps::PerEvaluation,
        };
        let _ = expr.eval_f32(&mut cx);
        let before = allocations();
        let float = expr.eval_f32(&mut cx);
        let value = expr.eval(&mut cx);
        assert_eq!(allocations() - before, 0, "{source}");
        assert!(
            float.is_finite() && matches!(value, Value::Float(_)),
            "{source}"
        );
    }
}
