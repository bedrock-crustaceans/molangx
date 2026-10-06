//! Variables: the `variable.` / `temp.` / `context.` scopes, `??`, what happens when an expression
//! reads a variable nobody set, and `->` reading another entity's public variables.
//!
//! Run with `cargo run --example variables --features vm`.

use molangx::compile::{CompileOptions, compile};
use molangx::version::MolangVersion;
use molangx::vm::{
    ContextName, Host, HostAccess, HostEnv, NoHostEnv, Subjects, TempMap, Temps, Value,
    VariableName, VariableStorage,
};

fn run(env: &mut NoHostEnv, source: &str) {
    let (expr, _) = compile(source, &CompileOptions::server(MolangVersion::LATEST))
        .into_result()
        .expect("compiles");
    let value = expr.eval_f32(&mut env.cx());
    println!("  {source:<56} = {value}");
    for message in env.sink.take() {
        println!("      logged: {message}");
    }
}

fn main() {
    let mut env = NoHostEnv::new();

    println!("variable. persists in the store:");
    run(&mut env, "v.speed = 2.5;");
    run(&mut env, "return v.speed * 2;");
    println!(
        "  the store now holds {:?}",
        env.variables.get(VariableName::new("speed"))
    );
    // Names are case-insensitive: the lexer lower-cases everything outside strings.
    run(&mut env, "return V.SPEED;");

    println!();
    println!("temp. is local to one evaluation:");
    run(&mut env, "t.scratch = 3;");
    run(&mut env, "return t.scratch;");
    println!("temp. persists (NoHostEnv::temps):");
    let mut persistent = NoHostEnv {
        temps: Temps::Kept(TempMap::new()),
        ..NoHostEnv::new()
    };
    run(&mut persistent, "t.scratch = 3;");
    run(&mut persistent, "return t.scratch;");

    println!();
    println!("context. is filled by the caller:");
    env.context
        .set(ContextName::new("damage"), Value::Float(4.0));
    run(&mut env, "return c.damage * 0.5;");

    // `??` does not react to 0.
    println!();
    println!("?? reacts to a missing variable only:");
    run(&mut env, "return v.never_set ?? 7;");
    run(&mut env, "v.zero = 0; return v.zero ?? 7;");
    run(&mut env, "return v.a ?? v.b ?? 9;");

    // Without `??`, reading a missing variable ends the whole expression with 0 and logs a message.
    // Assignments made before the read stay; nothing after it runs.
    println!();
    println!("a missing read without ?? aborts the expression:");
    run(
        &mut env,
        "v.before = 1; v.after = v.never_set; v.later = 2; return 3;",
    );
    println!(
        "  v.before = {:?}, v.after = {:?}, v.later = {:?}",
        env.variables.get(VariableName::new("before")),
        env.variables.get(VariableName::new("after")),
        env.variables.get(VariableName::new("later"))
    );

    // Structs: member writes create the struct on the way.
    println!();
    println!("members:");
    run(
        &mut env,
        "v.pos.x = 1; v.pos.y = 2; return v.pos.x + v.pos.y;",
    );

    // `->` needs a host that can resolve an entity handle, so it takes a small host: two entities
    // whose variables are kept by the ready-made `VariableStorage`.
    println!();
    println!("-> reads another entity's public variables:");
    arrow();
}

struct Pair;

impl Host for Pair {
    type ActorRef = u32;
    type ItemRef = ();
    type BlockRef = ();
    type Access<'w> = Pair;
}

impl HostAccess<Pair> for Pair {
    fn resolve_actor(&self, _from: &Subjects<Pair>, actor: u32) -> Option<u32> {
        (actor == 1 || actor == 2).then_some(actor)
    }
}

fn arrow() {
    // Variables in the ready-made per-entity `VariableStorage`.
    let mut env = HostEnv::<Pair>::default().with_variables(VariableStorage::new());
    let store = &mut env.variables;
    store
        .actor_mut(2)
        .set_public(VariableName::new("hp"), Value::Float(8.0));
    store
        .actor_mut(2)
        .set(VariableName::new("secret"), Value::Float(1.0));
    store
        .actor_mut(1)
        .set(VariableName::new("buddy"), Value::Actor(2));
    // Other entities read a public variable's *snapshot*, which the host refreshes once per tick.
    println!("  before the first update, entity 1 reads entity 2's hp as:");
    eval_for_entity_1(&mut env, "v.buddy->v.hp");
    env.variables.refresh_snapshots();
    println!("  after refresh_snapshots():");
    eval_for_entity_1(&mut env, "v.buddy->v.hp");
    // The owner always reads the latest value; a change is invisible through `->` until the next
    // update.
    env.variables
        .actor_mut(2)
        .set_public(VariableName::new("hp"), Value::Float(5.0));
    println!("  entity 2 lowers its hp to 5; entity 1 still sees the snapshot:");
    eval_for_entity_1(&mut env, "v.buddy->v.hp");
    println!("  a private variable reads 0 through ->, without aborting:");
    eval_for_entity_1(&mut env, "v.buddy->v.secret");
}

fn eval_for_entity_1(env: &mut HostEnv<Pair, VariableStorage<Pair>>, source: &str) {
    let (expr, _) = compile(source, &CompileOptions::server(MolangVersion::LATEST))
        .into_result()
        .expect("compiles");
    let value = expr.eval(&mut env.cx(&mut Pair, Subjects::actor(1)));
    println!(
        "      {source:<24} = {value:?}   (logged: {:?})",
        env.sink.take()
    );
}
