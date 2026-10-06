//! Embedding the evaluator: a toy world, the traits a host implements (`Host`, `HostAccess`,
//! `VariableStore`, `ContextProvider`), two standard queries and one of the host's own.
//!
//! A query without a function in the `QueryTable` returns the default of its declaration.
//!
//! Run with `cargo run --example host --features vm`.

use std::collections::HashMap;

use molangx::catalog::{Arity, QueryDecl, QueryShape, Reads, ReturnType, Side};
use molangx::compile::{CompileOptions, compile};
use molangx::hash::HashedStr;
use molangx::stdlib::{queries, query};
use molangx::version::MolangVersion;
use molangx::vm::{
    ContextName, ContextProvider, Host, HostAccess, HostEnv, QueryCx, QueryResult, QueryTable,
    Subjects, Value, VariableName, VariableStore,
};

struct Toy;

impl Host for Toy {
    type ActorRef = u32;
    type ItemRef = ();
    type BlockRef = ();
    type Access<'w> = World; // the world-access object (it could borrow the world for 'w)
}

struct Entity {
    name: &'static str,
    health: f32,
    baby: bool,
}

/// The toy world: entities by id.
struct World {
    entities: HashMap<u32, Entity>,
}

impl HostAccess<Toy> for World {
    /// `->` and `for_each` ask whether a handle still refers to something.
    fn resolve_actor(&self, _from: &Subjects<Toy>, actor: u32) -> Option<u32> {
        self.entities.contains_key(&actor).then_some(actor)
    }
}

/// `variable.*`, keyed by entity and name.
#[derive(Default)]
struct Variables {
    by_entity: HashMap<(u32, VariableName), Value<Toy>>,
}

impl VariableStore<Toy> for Variables {
    fn get(&self, actor: u32, name: VariableName) -> Option<&Value<Toy>> {
        self.by_entity.get(&(actor, name))
    }

    fn set(&mut self, actor: u32, name: VariableName, value: Value<Toy>) {
        self.by_entity.insert((actor, name), value);
    }

    /// What another entity sees through `->`. This toy has no public variables.
    fn get_public(&self, _actor: u32, _name: VariableName) -> Option<&Value<Toy>> {
        None
    }
}

/// `context.*`: read-only, filled by the caller for one evaluation.
struct Context {
    damage: f32,
}

impl ContextProvider<Toy> for Context {
    fn context(&self, name: ContextName) -> Option<Value<Toy>> {
        (name == ContextName::new("damage")).then_some(Value::Float(self.damage))
    }
}

/// `query.is_baby`: a query function receives the evaluation's `QueryCx` and its (unevaluated)
/// arguments.
fn is_baby(cx: &mut QueryCx<'_, '_, Toy>) -> QueryResult<Toy> {
    // No subject: the declared default (0).
    let Some(actor) = cx.subjects().actor else {
        return Ok(cx.default_value());
    };
    Ok(Value::bool(
        cx.host().entities.get(&actor).is_some_and(|e| e.baby),
    ))
}

/// `query.health` scaled by a host setting, 0 without a subject.
fn health(cx: &mut QueryCx<'_, '_, Toy>, scale: f32) -> QueryResult<Toy> {
    let Some(actor) = cx.subjects().actor else {
        return Ok(cx.default_value());
    };
    Ok(Value::Float(
        cx.host()
            .entities
            .get(&actor)
            .map_or(0.0, |e| e.health * scale),
    ))
}

/// `query.name_is('calf')`: the host's own query. It checks its argument itself and reports misuse
/// as an error, which goes to the sink while the call returns the default.
fn name_is(cx: &mut QueryCx<'_, '_, Toy>) -> QueryResult<Toy> {
    let Some(wanted) = cx.arg_hash(0) else {
        return Err(cx.error(format_args!(
            "Error: {} needs one string argument.",
            cx.name()
        )));
    };
    let Some(actor) = cx.subjects().actor else {
        return Ok(cx.default_value());
    };
    Ok(Value::bool(
        cx.host()
            .entities
            .get(&actor)
            .is_some_and(|e| HashedStr::new(e.name) == wanted),
    ))
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut world = World {
        entities: HashMap::from([
            (
                1,
                Entity {
                    name: "cow",
                    health: 10.0,
                    baby: false,
                },
            ),
            (
                2,
                Entity {
                    name: "calf",
                    health: 4.0,
                    baby: true,
                },
            ),
            (
                3,
                Entity {
                    name: "zombie",
                    health: 20.0,
                    baby: false,
                },
            ),
        ]),
    };
    let mut variables = Variables::default();
    // The calf follows the cow (an entity in a variable, stored the way the host chooses).
    variables.set(2, VariableName::new("parent"), Value::Actor(1));
    // A variable that points at an entity that is gone.
    variables.set(2, VariableName::new("stale"), Value::Actor(99));

    // The standard server catalogue plus one query of the host's own.
    let name_is_decl = QueryDecl::new(
        "query.name_is",
        QueryShape {
            args: Arity::exactly(1),
            returns: ReturnType::BOOL,
            reads: Reads::ACTOR,
            ..QueryShape::DEFAULT
        },
    )?;
    let catalog = queries(Side::Server).extended([name_is_decl])?;

    // One implementation per query we answer, registered by name; the rest stay stubs. A closure
    // can capture the host's settings.
    let health_scale = 1.0;
    let mut queries = QueryTable::new(&catalog);
    queries.set(query::IS_BABY, is_baby)?;
    queries.set(query::HEALTH, move |cx| health(cx, health_scale))?;
    queries.set("query.name_is", name_is)?;
    println!(
        "{} of the {} queries are implemented, the rest are stubs",
        queries.implemented(),
        catalog.len()
    );

    let mut env = HostEnv::new(queries)
        .with_variables(variables)
        .with_context(Context { damage: 3.0 });
    let options = CompileOptions::new(catalog, MolangVersion::LATEST);

    // The compiler checks a call of our query against its declaration, as it does a standard one.
    let misuse = compile("query.name_is", &options);
    for d in misuse.diagnostics() {
        println!("compiling `query.name_is`: {}", d.message());
    }

    let expressions = [
        "query.is_baby ? query.health * 2 : query.health",
        "query.health - context.damage",
        // `->` evaluates its right side for the entity on its left. The cow has no `v.parent`:
        // reading a variable nobody set ends the expression with 0 and logs (see the end).
        "v.parent->query.health",
        // The calf's `->` target is gone: the whole `->` is 0 and its right side never runs.
        "v.stale->query.health",
        // A query the host did not implement is a stub that returns its declared default (0).
        "query.is_on_ground",
        // Stored in a variable, readable by the next evaluation.
        "v.copy = query.health; return v.copy;",
        "query.name_is('calf') ? 100 : query.health",
    ];
    for source in expressions {
        let (expr, _) = compile(source, &options).into_result()?;
        println!("{source} calls {:?}", expr.queries().collect::<Vec<_>>());
        for id in [1, 2] {
            let name = world.entities[&id].name;
            // One context per evaluation: the world and the entity it runs for.
            let value = expr.eval(&mut env.cx(&mut world, Subjects::actor(id)));
            println!("{source:<52} for {name:<5} = {value:?}");
        }
    }
    println!(
        "{:?}",
        env.variables.by_entity.get(&(2, VariableName::new("copy")))
    );
    println!("logged: {:?}", env.sink.take());
    Ok(())
}
