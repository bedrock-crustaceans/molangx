//! The test host of the evaluation tests: a few actors, their baby flags, and implementations of
//! the eleven queries the tests call (the library implements none).

use std::collections::HashSet;

use molangx::compile::Compiled;
use molangx::rng::rand_core::Rng;
use molangx::stdlib::query;
use molangx::vm::{
    ContextMap, EvalCx, EvalLimits, Host, HostAccess, LogLevel, QueryCx, QueryError, QueryResult,
    QueryTable, RuntimeMsg, RuntimeSink, Subjects, TempMap, Temps, Value, VariableMap,
};

#[derive(Debug)]
pub struct TestHost;

/// `Handle(0)` is the null handle.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum Actor {
    Handle(u32),
    Id(u32),
}

impl Host for TestHost {
    type ActorRef = Actor;
    type ItemRef = u16;
    type BlockRef = ();
    type Access<'w> = World;
}

#[derive(Debug, Default)]
pub struct World {
    pub alive: HashSet<u32>,
    pub baby: HashSet<u32>,
}

impl HostAccess<TestHost> for World {
    fn resolve_actor(&self, from: &Subjects<TestHost>, actor: Actor) -> Option<Actor> {
        match actor {
            Actor::Handle(0) => None,
            Actor::Handle(n) => self.alive.contains(&n).then_some(Actor::Handle(n)),
            // An id resolves only while the current subject is an actor.
            Actor::Id(n) => {
                (from.actor.is_some() && self.alive.contains(&n)).then_some(Actor::Handle(n))
            }
        }
    }

    fn stored_actor(&self, actor: Actor) -> Actor {
        match actor {
            Actor::Handle(n) | Actor::Id(n) => Actor::Id(n),
        }
    }
}

/// An expression compiled against a built-in catalogue reaches these queries by name.
pub fn queries() -> QueryTable<TestHost> {
    let implementations: [(&str, Implementation); 11] = [
        (query::COUNT, count),
        (query::IS_BABY, is_baby),
        (query::ANY, any),
        (query::ALL, all),
        (query::IN_RANGE, in_range),
        (query::LOG, log),
        (query::CLIENT_MEMORY_TIER, client_memory_tier),
        (query::HAS_BLOCK_PROPERTY, has_block_property),
        (super::GET_NAME_TEST, get_name_test),
        (super::SUM_TEST, sum_test),
        (super::EXPERIMENTAL_TEST, experimental_test),
    ];
    let mut table = QueryTable::new(super::reference_catalog());
    for (name, f) in implementations {
        table.set(name, f).unwrap();
    }
    table
}

type Cx<'a, 'w> = QueryCx<'a, 'w, TestHost>;
type Result = QueryResult<TestHost>;
type Implementation = fn(&mut Cx<'_, '_>) -> Result;

fn get_name_test(cx: &mut Cx<'_, '_>) -> Result {
    let index = cx.arg_f32(0).unwrap_or(0.0);
    Ok(Value::string(if index == 1.0 { "rabbit" } else { "moo" }))
}

fn sum_test(cx: &mut Cx<'_, '_>) -> Result {
    let mut sum = 0.0f32;
    for i in 0..cx.arg_count() {
        sum += cx.arg(i).map_or(0.0, |v| v.as_f32());
    }
    Ok(Value::Float(sum))
}

fn experimental_test(_cx: &mut Cx<'_, '_>) -> Result {
    Ok(Value::ONE)
}

fn count(cx: &mut Cx<'_, '_>) -> Result {
    let mut total = 0usize;
    for i in 0..cx.arg_count() {
        total += match cx.arg(i) {
            Some(Value::ActorArray(array)) => array.len(),
            _ => 1,
        };
    }
    Ok(Value::Float(total as f32))
}

fn is_baby(cx: &mut Cx<'_, '_>) -> Result {
    let actor = cx.subjects().actor;
    let baby = match actor {
        Some(Actor::Handle(n) | Actor::Id(n)) => cx.host().baby.contains(&n),
        None => false,
    };
    Ok(Value::bool(baby))
}

fn same_arg(a: &Value<TestHost>, b: &Value<TestHost>) -> bool {
    match (a, b) {
        (Value::Float(x), Value::Float(y)) => x.to_bits() == y.to_bits(),
        _ => a == b,
    }
}

fn any(cx: &mut Cx<'_, '_>) -> Result {
    if cx.arg_count() < 3 {
        return Err(cx.error("query.any takes at least three arguments"));
    }
    let first = cx.arg(0).unwrap_or_default();
    for i in 1..cx.arg_count() {
        if cx.arg(i).is_some_and(|v| same_arg(&first, &v)) {
            return Ok(Value::ONE);
        }
    }
    Ok(Value::ZERO)
}

fn all(cx: &mut Cx<'_, '_>) -> Result {
    if cx.arg_count() < 3 {
        return Err(cx.error("query.all takes at least three arguments"));
    }
    let first = cx.arg(0).unwrap_or_default();
    for i in 1..cx.arg_count() {
        if !cx.arg(i).is_some_and(|v| same_arg(&first, &v)) {
            return Ok(Value::ZERO);
        }
    }
    Ok(Value::ONE)
}

fn in_range(cx: &mut Cx<'_, '_>) -> Result {
    if cx.arg_count() != 3 {
        return Err(cx.error("query.in_range takes three numbers"));
    }
    let messages = [
        "the first argument of query.in_range is not a number",
        "the second argument of query.in_range is not a number",
        "the third argument of query.in_range is not a number",
    ];
    let mut values = [0.0f32; 3];
    for (i, message) in messages.into_iter().enumerate() {
        values[i] = cx.arg_f32(i).ok_or_else(|| cx.error(message))?;
    }
    let [v, min, max] = values;
    Ok(Value::bool(min <= v && v <= max))
}

fn log(cx: &mut Cx<'_, '_>) -> Result {
    let first = cx.arg(0).unwrap_or_default();
    for i in 1..cx.arg_count() {
        let _ = cx.arg(i);
    }
    Ok(first)
}

fn client_memory_tier(cx: &mut Cx<'_, '_>) -> Result {
    Err(cx.error("Error: client_memory_tier isn't supported on the server (headless mode)."))
}

fn has_block_property(cx: &mut Cx<'_, '_>) -> Result {
    Err(cx.error("Error: query.has_block_property does not have a block."))
}

/// One test state. Its one variable map serves the subject and every actor.
pub struct Env {
    pub vars: VariableMap<TestHost>,
    pub context: ContextMap<TestHost>,
    /// `Some` keeps `temp.*` across evaluations.
    pub temps: Option<TempMap<TestHost>>,
    pub world: World,
    pub queries: QueryTable<TestHost>,
    pub this: f32,
    pub limits: EvalLimits,
}

/// The live actor `context.moo` points at.
pub const LIVE_ACTOR: u32 = 1;
/// A second live actor, never the subject.
pub const SECOND_ACTOR: u32 = 2;

impl Env {
    pub fn new() -> Self {
        Self {
            vars: VariableMap::new(),
            context: ContextMap::new(),
            temps: Some(TempMap::new()),
            world: World::default(),
            queries: queries(),
            this: 0.0,
            limits: EvalLimits::NONE,
        }
    }

    pub fn reference() -> Self {
        Self {
            this: 2.34,
            ..Self::new()
        }
    }

    pub fn with_cx<R>(
        &mut self,
        rng: &mut dyn Rng,
        sink: &mut dyn RuntimeSink,
        f: impl FnOnce(&mut EvalCx<'_, '_, TestHost>) -> R,
    ) -> R {
        let mut cx = EvalCx {
            subjects: Subjects {
                this: self.this,
                ..Subjects::none()
            },
            host: &mut self.world,
            variables: &mut self.vars,
            context: &self.context,
            queries: Some(&self.queries),
            rng,
            sink,
            limits: self.limits,
            temps: match &mut self.temps {
                Some(temps) => Temps::Kept(temps),
                None => Temps::PerEvaluation,
            },
        };
        f(&mut cx)
    }

    /// Evaluates `compiled`; a compile that kept no expression gives 0.
    pub fn eval(
        &mut self,
        compiled: &Compiled,
        rng: &mut dyn Rng,
        sink: &mut dyn RuntimeSink,
    ) -> Value<TestHost> {
        self.with_cx(rng, sink, |cx| {
            compiled.expr().map_or(Value::ZERO, |e| e.eval(cx))
        })
    }
}

impl Default for Env {
    fn default() -> Self {
        Self::new()
    }
}

/// Keeps each message with its level; a query failure is kept at Error level. With `language_only`,
/// `molangx: ` messages are dropped, while the longer unknown-variable wording logged inside `->`
/// is kept.
#[derive(Debug, Default)]
pub struct LevelSink {
    pub lines: Vec<(LogLevel, String)>,
    pub language_only: bool,
}

impl LevelSink {
    pub fn language_only() -> Self {
        Self {
            lines: Vec::new(),
            language_only: true,
        }
    }
}

impl RuntimeSink for LevelSink {
    fn runtime(&mut self, msg: RuntimeMsg<'_>) {
        if msg.level() == LogLevel::Error || !self.language_only {
            self.lines
                .push((msg.level(), msg.to_string().trim_end().to_owned()));
        }
    }

    fn query_error(&mut self, error: QueryError) {
        self.lines
            .push((LogLevel::Error, error.to_string().trim_end().to_owned()));
    }
}
