//! Evaluation benchmarks.
//!
//! * `budget/*`: the three `eval_f32` budgets: a constant-folded expression (< 2 ns), a
//!   `math.clamp(q.anger_level / 80 * 1.5, 0, 1.5)`-sized expression with the query body excluded
//!   (< 40 ns, with a variable and with a trivial query), a 20-statement expression (< 400 ns).
//! * `shapes/*`: one benchmark per common expression shape.
//! * `tick/*`: one tick of 1,000 entities × 5 expressions, each entity with its own variable map.
//!
//! Run with `cargo bench --features vm --bench eval`.

use std::hint::black_box;

use criterion::{Criterion, criterion_group, criterion_main};
use molangx::catalog::Side;
use molangx::compile::{CompileOptions, Expr, ProgramFlags, compile};
use molangx::hash::HashedStr;
use molangx::stdlib::{queries, query};
use molangx::version::MolangVersion;
use molangx::vm::{
    Host, HostAccess, HostEnv, NoHostEnv, QueryCx, QueryError, QueryTable, Subjects, Value,
    VariableName, VariableStorage,
};

fn expr(source: &str) -> Expr {
    let compiled = compile(source, &CompileOptions::server(MolangVersion::LATEST));
    compiled
        .expr()
        .cloned()
        .unwrap_or_else(|| panic!("`{source}` does not compile: {:?}", compiled.diagnostics()))
}

/// `query.anger_level` with a trivial body, so the benchmark excludes the query's own cost.
#[allow(clippy::unnecessary_wraps)]
fn anger_level<H: Host>(_cx: &mut QueryCx<'_, '_, H>) -> Result<Value<H>, QueryError> {
    Ok(Value::Float(40.0))
}

fn env() -> NoHostEnv {
    // A table for the compile catalogue: calls go straight to their slot.
    let mut queries = QueryTable::new(queries(Side::Server));
    queries.set(query::ANGER_LEVEL, anger_level).unwrap();
    let mut env = NoHostEnv {
        queries: Some(queries),
        this: 2.34,
        ..NoHostEnv::new()
    };
    for (name, value) in [
        ("anger_level", 40.0),
        ("x", 1.5),
        ("y", -2.25),
        ("z", 0.75),
        ("speed", 0.3),
        ("phase", 12.0),
        ("health", 17.0),
        ("max_health", 20.0),
        ("is_angry", 1.0),
        ("foo", 3.0),
        ("bar", 7.0),
    ] {
        env.variables
            .set(VariableName::new(name), Value::Float(value));
    }
    env.variables.set(
        VariableName::new("state"),
        Value::Hash(HashedStr::new("walk")),
    );
    env
}

const TWENTY_STATEMENTS: &str = "\
t.a = v.x * 2 + 1; \
t.b = v.y / 4 - t.a; \
t.c = math.clamp(t.a * t.b, -10, 10); \
t.d = math.abs(t.c) + math.floor(v.z * 3); \
v.out = t.a + t.b * t.c - t.d; \
t.e = (t.a > t.b) ? t.c : t.d; \
t.f = math.lerp(t.a, t.b, 0.25); \
t.g = math.max(t.e, t.f) * 0.5; \
v.acc = v.out + t.g; \
t.h = t.g * t.g - t.a; \
t.a = t.h / (1 + math.abs(t.b)); \
t.b = t.a + t.c * 0.125; \
v.flag = t.a < 0 && t.b > -5 || t.c == 0; \
t.c = math.min(t.b, 4) + math.mod(t.d, 3); \
t.d = v.flag ? t.c * 2 : t.c / 2; \
v.res = t.d + v.acc * 0.01; \
t.e = math.round(v.res * 100) / 100; \
t.f = -t.e + math.sqrt(math.abs(t.d)); \
v.final = t.f * 0.5 + t.e; \
return v.final + t.a;";

/// The same twenty statements on `variable.` names only.
const TWENTY_STATEMENTS_VARS: &str = "\
v.a = v.x * 2 + 1; \
v.b = v.y / 4 - v.a; \
v.c = math.clamp(v.a * v.b, -10, 10); \
v.d = math.abs(v.c) + math.floor(v.z * 3); \
v.out = v.a + v.b * v.c - v.d; \
v.e = (v.a > v.b) ? v.c : v.d; \
v.f = math.lerp(v.a, v.b, 0.25); \
v.g = math.max(v.e, v.f) * 0.5; \
v.acc = v.out + v.g; \
v.h = v.g * v.g - v.a; \
v.a = v.h / (1 + math.abs(v.b)); \
v.b = v.a + v.c * 0.125; \
v.flag = v.a < 0 && v.b > -5 || v.c == 0; \
v.c = math.min(v.b, 4) + math.mod(v.d, 3); \
v.d = v.flag ? v.c * 2 : v.c / 2; \
v.res = v.d + v.acc * 0.01; \
v.e = math.round(v.res * 100) / 100; \
v.f = -v.e + math.sqrt(math.abs(v.d)); \
v.final = v.f * 0.5 + v.e; \
return v.final + v.a;";

/// Twenty statements each calling `math.sin` (the math-library share of a statement list).
fn twenty_sines() -> String {
    let mut source = String::new();
    for i in 0..19 {
        source.push_str(&format!(
            "t.s{} = math.sin(v.phase * {}) * 0.5; ",
            i % 6,
            i + 1
        ));
    }
    source.push_str("return t.s0 + t.s1 + t.s2 + t.s3 + t.s4 + t.s5;");
    source
}

fn budgets(c: &mut Criterion) {
    let mut env = env();
    let mut group = c.benchmark_group("budget");
    let cases = [
        (
            "constant",
            "math.clamp(1 + 2 * 3, 0, 5) / 2 + math.cos(60)".to_owned(),
        ),
        (
            "clamp_variable",
            "math.clamp(v.anger_level / 80 * 1.5, 0, 1.5)".to_owned(),
        ),
        (
            "clamp_query",
            "math.clamp(q.anger_level / 80 * 1.5, 0, 1.5)".to_owned(),
        ),
        ("twenty_statements_temps", TWENTY_STATEMENTS.to_owned()),
        ("twenty_statements_vars", TWENTY_STATEMENTS_VARS.to_owned()),
        ("twenty_statements_sin", twenty_sines()),
    ];
    for (name, source) in &cases {
        let e = expr(source);
        if *name == "constant" {
            assert!(
                e.flags().contains(ProgramFlags::CONSTANT),
                "{name} must fold"
            );
        }
        let mut cx = env.cx();
        let value = e.eval_f32(&mut cx);
        assert!(value.is_finite(), "{name} = {value}");
        group.bench_function(*name, |b| b.iter(|| black_box(&e).eval_f32(&mut cx)));
    }
    assert!(
        env.sink.take().is_empty(),
        "the budget expressions log nothing"
    );
    group.finish();
}

const SHAPES: &[(&str, &str)] = &[
    ("cos_one", "math.cos(v.phase)"),
    (
        "cos_sum_of_four",
        "math.cos(v.phase) + math.cos(v.phase * 2) + math.cos(v.phase * 3) + math.cos(v.phase * 4)",
    ),
    (
        "cos_nested",
        "math.cos(math.cos(math.cos(math.cos(v.phase))))",
    ),
    (
        "cos_scaled_with_this",
        "math.cos(v.phase * 57.3) * 15 + math.cos(this * 90) * -5",
    ),
    (
        "arithmetic_mix",
        "(v.x + v.y) * (v.x - v.z) / (v.y * v.z + 1) - v.x * 0.5",
    ),
    (
        "sqrt_pow_exp_ln",
        "math.sqrt(math.abs(v.y)) + math.pow(v.x, 2) + math.exp(v.z) - math.ln(v.speed)",
    ),
    (
        "clamp_of_lerp",
        "math.clamp(math.lerp(v.x, v.y, v.speed), -1, 1) * math.lerprotate(v.phase, 350, 0.5)",
    ),
    (
        "rounding_mix",
        "math.round(v.x * 10) + math.floor(v.y) + math.ceil(v.z) + math.trunc(v.phase / 7) + math.mod(v.phase, 5)",
    ),
    (
        "two_easings",
        "math.ease_in_out_cubic(0, 1, v.speed) + math.ease_out_bounce(0, 10, v.z)",
    ),
    ("random_one", "math.random(0, 1)"),
    (
        "random_and_dice_mix",
        "math.random(v.x, v.y) + math.random_integer(1, 6) + math.die_roll(2, 1, 6)",
    ),
    (
        "random_integer_mix",
        "math.random_integer(0, 100) * 0.01 + math.random(-1, 1)",
    ),
    (
        "conditional",
        "v.is_angry ? math.sin(v.phase * 90) * 10 : v.health / v.max_health * 2",
    ),
    (
        "logic_and_comparisons",
        "(v.health < v.max_health * 0.5 && v.is_angry) || v.speed > 0.25 ? 1 : 0",
    ),
    ("coalesce_missing", "(v.missing ?? 3) + (v.foo ?? 0) * 2"),
    (
        "loop_sum",
        "t.sum = 0; loop(8, { t.sum = t.sum + v.x; }); return t.sum;",
    ),
    // A string makes the program general (not float-only): the `Value` loop.
    (
        "string_compare",
        "v.state == 'idle' ? 1 : (v.state == 'walk' ? v.speed * 2 : 0)",
    ),
    ("read_one_variable", "v.foo"),
    (
        "read_eight_variables",
        "v.foo + v.bar + v.x + v.y + v.z + v.speed + v.phase + v.health",
    ),
    ("write_one_variable", "v.foo = 3;"),
    (
        "write_eight_variables",
        "v.a1 = 1; v.a2 = 2; v.a3 = 3; v.a4 = 4; v.a5 = 5; v.a6 = 6; v.a7 = 7; v.a8 = 8;",
    ),
    (
        "read_modify_write",
        "v.bar = v.bar + v.foo * 0.5; return v.bar;",
    ),
];

fn shapes(c: &mut Criterion) {
    let mut env = env();
    let mut group = c.benchmark_group("shapes");
    for (name, source) in SHAPES {
        let e = expr(source);
        let mut cx = env.cx();
        group.bench_function(*name, |b| b.iter(|| black_box(&e).eval_f32(&mut cx)));
    }
    // `coalesce_missing` reads a missing variable under `??`: no message.
    assert!(
        env.sink.take().is_empty(),
        "the shape expressions log nothing"
    );
    group.finish();
}

/// The tick host: an entity is its index into the world.
struct Mob;

impl Host for Mob {
    type ActorRef = u32;
    type ItemRef = ();
    type BlockRef = ();
    type Access<'w> = World;
}

struct Entity {
    health: f32,
    anim_time: f32,
    baby: bool,
}

struct World {
    entities: Vec<Entity>,
}

impl HostAccess<Mob> for World {
    fn resolve_actor(&self, _from: &Subjects<Mob>, actor: u32) -> Option<u32> {
        ((actor as usize) < self.entities.len()).then_some(actor)
    }
}

fn entity<'a>(cx: &'a mut QueryCx<'_, '_, Mob>) -> Option<&'a Entity> {
    let actor = cx.subjects().actor?;
    cx.host().entities.get(actor as usize)
}

#[allow(clippy::unnecessary_wraps)]
fn health(cx: &mut QueryCx<'_, '_, Mob>) -> Result<Value<Mob>, QueryError> {
    Ok(Value::Float(entity(cx).map_or(0.0, |e| e.health)))
}

#[allow(clippy::unnecessary_wraps)]
fn anim_time(cx: &mut QueryCx<'_, '_, Mob>) -> Result<Value<Mob>, QueryError> {
    Ok(Value::Float(entity(cx).map_or(0.0, |e| e.anim_time)))
}

#[allow(clippy::unnecessary_wraps)]
fn is_baby(cx: &mut QueryCx<'_, '_, Mob>) -> Result<Value<Mob>, QueryError> {
    Ok(Value::bool(entity(cx).is_some_and(|e| e.baby)))
}

/// Five per-entity expressions of the kinds a behaviour pack runs every tick.
const TICK: [&str; 5] = [
    "v.timer = v.timer + 0.05; return v.timer > 3 ? 0 : v.timer;",
    "math.clamp(v.anger / 80 * 1.5, 0, 1.5)",
    "math.lerp(v.prev_yaw, v.yaw, 0.5) - v.yaw * 0.1",
    "math.sin(q.anim_time * 180 + v.offset) * 15",
    "v.hostile = q.health < 10 && !q.is_baby;",
];

const ENTITIES: u32 = 1_000;

fn tick(c: &mut Criterion) {
    let exprs: Vec<Expr> = TICK.iter().map(|source| expr(source)).collect();
    let mut queries = QueryTable::<Mob>::new(queries(Side::Server));
    queries.set(query::HEALTH, health).unwrap();
    queries.set(query::ANIM_TIME, anim_time).unwrap();
    queries.set(query::IS_BABY, is_baby).unwrap();
    let mut env = HostEnv::new(queries).with_variables(VariableStorage::<Mob>::new());
    let mut world = World {
        entities: (0..ENTITIES)
            .map(|i| Entity {
                health: (i % 20) as f32,
                anim_time: i as f32 * 0.01,
                baby: i % 7 == 0,
            })
            .collect(),
    };
    for i in 0..ENTITIES {
        let map = env.variables.actor_mut(i);
        for (name, value) in [
            ("timer", 0.0),
            ("anger", (i % 80) as f32),
            ("prev_yaw", 10.0),
            ("yaw", i as f32),
            ("offset", 3.0),
        ] {
            map.set(VariableName::new(name), Value::Float(value));
        }
    }
    let mut group = c.benchmark_group("tick");
    group.bench_function("entities_1000_x5", |b| {
        b.iter(|| {
            let mut sum = 0.0f32;
            for i in 0..ENTITIES {
                for e in &exprs {
                    sum += e.eval_f32(&mut env.cx(&mut world, Subjects::actor(i)));
                }
            }
            sum
        });
    });
    group.finish();
    assert!(
        env.sink.take().is_empty(),
        "the tick expressions log nothing"
    );
}

criterion_group!(benches, budgets, shapes, tick);
criterion_main!(benches);
