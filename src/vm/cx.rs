//! [`EvalCx`], what one evaluation borrows, and the owning environments that build one.

use std::fmt::{self, Debug};

use super::host::{ContextMap, ContextProvider, Host, HostAccess, NoHost, Subjects};
use super::limits::EvalLimits;
use super::name::{ContextName, VariableName};
use super::query::QueryTable;
use super::sink::{BoundedSink, RuntimeSink};
use super::value::Value;
use super::vars::{TempMap, VariableMap, VariableStore};
use crate::rng::{Xorshift128, rand_core::Rng};

/// The context of one evaluation: its subjects and the host-owned state around it.
///
/// `'a` borrows the parts; `'w` is the world borrow of [`Host::Access`].
pub struct EvalCx<'a, 'w, H: Host> {
    /// What the expression is evaluated for; `->` switches it for its right side.
    pub subjects: Subjects<H>,
    /// The host's world-access object; [`NoHost`] for none.
    pub host: &'a mut H::Access<'w>,
    /// `variable.*` storage: [`VariableMap`], [`VariableStorage`](super::VariableStorage) or
    /// [`NoVariables`](super::NoVariables).
    pub variables: &'a mut dyn VariableStore<H>,
    /// Read-only `context.*`: [`ContextMap`] or [`NoContext`](super::NoContext).
    pub context: &'a dyn ContextProvider<H>,
    /// The host's query implementations; `None`: every call returns its declared default.
    pub queries: Option<&'a QueryTable<H>>,
    /// The random source: [`Xorshift128`], [`FixedRng`](crate::rng::FixedRng),
    /// [`ProcessRng`](super::ProcessRng) or any other `rand_core` generator.
    pub rng: &'a mut dyn Rng,
    /// Where run-time diagnostics go: [`BoundedSink`], [`LogOnce`](super::LogOnce),
    /// [`NullSink`](super::NullSink) or the unbounded [`CollectSink`](super::CollectSink).
    pub sink: &'a mut dyn RuntimeSink,
    /// The host-protection budgets.
    pub limits: EvalLimits,
    /// Where `temp.*` lives.
    pub temps: Temps<&'a mut TempMap<H>>,
}

/// Where `temp.*` lives: an environment owns the map (`Temps<TempMap<H>>`) and its
/// [`EvalCx`] borrows it (`Temps<&mut TempMap<H>>`).
#[derive(Clone, Debug, Default, PartialEq)]
pub enum Temps<M> {
    /// Every top-level evaluation starts with no temps.
    #[default]
    PerEvaluation,
    /// Temps live in the map, which the evaluator never clears, so later evaluations read them.
    Kept(M),
}

impl<M> Temps<M> {
    /// The kept map, borrowed mutably.
    pub fn as_mut(&mut self) -> Temps<&mut M> {
        match self {
            Self::PerEvaluation => Temps::PerEvaluation,
            Self::Kept(map) => Temps::Kept(map),
        }
    }

    /// The kept map; `None` for [`Temps::PerEvaluation`].
    pub fn kept(&self) -> Option<&M> {
        match self {
            Self::PerEvaluation => None,
            Self::Kept(map) => Some(map),
        }
    }

    /// The kept map, mutably; `None` for [`Temps::PerEvaluation`].
    pub fn kept_mut(&mut self) -> Option<&mut M> {
        match self {
            Self::PerEvaluation => None,
            Self::Kept(map) => Some(map),
        }
    }
}

impl<H: Host> Debug for EvalCx<'_, '_, H> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("EvalCx")
            .field("limits", &self.limits)
            .field("persistent_temps", &matches!(self.temps, Temps::Kept(_)))
            .finish_non_exhaustive()
    }
}

impl<H: Host> EvalCx<'_, '_, H> {
    /// The latest value of a `variable.` name: the subject actor's, or the detached map's without
    /// an actor.
    pub fn variable(&self, name: VariableName) -> Option<&Value<H>> {
        match self.subjects.actor {
            Some(actor) => self.variables.get(actor, name),
            None => self.variables.get_local(name),
        }
    }

    /// What a `variable.` read sees inside `->`: the subject actor's public snapshot; `None`
    /// without an actor.
    pub(crate) fn public_variable(&self, name: VariableName) -> Option<&Value<H>> {
        self.variables.get_public(self.subjects.actor?, name)
    }

    /// Writes a `variable.` name as an assignment does, private when new and dropped without a
    /// map. An actor passes through [`HostAccess::stored_actor`]; an actor array keeps only its
    /// resolvable entries, each converted the same way.
    pub fn set_variable(&mut self, name: VariableName, value: Value<H>) {
        let value = self.storable(value);
        match self.subjects.actor {
            Some(actor) => self.variables.set(actor, name, value),
            None => self.variables.set_local(name, value),
        }
    }

    /// The form of `value` kept in a variable: an actor passes through
    /// [`HostAccess::stored_actor`], and an actor array keeps only its resolvable entries, each
    /// converted the same way.
    ///
    /// A single actor is converted even if it no longer resolves; actors inside structs are not
    /// converted.
    pub(crate) fn storable(&self, value: Value<H>) -> Value<H> {
        match value {
            Value::Actor(actor) => Value::Actor(self.host.stored_actor(actor)),
            array @ Value::ActorArray(_) => array.map_actors(|actor| {
                self.host
                    .resolve_actor(&self.subjects, actor)
                    .map(|live| self.host.stored_actor(live))
            }),
            other => other,
        }
    }

    /// The value of a `context.` name; `None` is a missing variable.
    pub fn context(&self, name: ContextName) -> Option<Value<H>> {
        self.context.context(name)
    }

    /// The live actor `actor` refers to, looked up from the current subjects
    /// ([`HostAccess::resolve_actor`]).
    pub fn resolve_actor(&self, actor: H::ActorRef) -> Option<H::ActorRef> {
        self.host.resolve_actor(&self.subjects, actor)
    }

    /// The subjects `->` switches to for the left-side value `target`: a resolvable actor's or an
    /// item stack's; `None` for anything else, and the `->` then yields 0.
    pub(crate) fn arrow_target(&self, target: &Value<H>) -> Option<Subjects<H>> {
        match target {
            Value::Actor(actor) => self
                .resolve_actor(*actor)
                .map(|live| self.host.subjects_of(live)),
            Value::Item(item) => Some(self.host.subjects_of_item(&self.subjects, *item)),
            _ => None,
        }
    }
}

/// Everything an evaluation without a host needs, owned in one place.
///
/// ```
/// # #[cfg(feature = "stdlib")]
/// # {
/// use molangx::compile::{CompileOptions, compile};
/// use molangx::version::MolangVersion;
/// use molangx::vm::NoHostEnv;
///
/// let (expr, _) = compile(
///     "math.clamp(1 + 2 * 3, 0, 5)",
///     &CompileOptions::server(MolangVersion::LATEST),
/// )
/// .into_result()
/// .unwrap();
/// let mut env = NoHostEnv::new();
/// assert_eq!(expr.eval_f32(&mut env.cx()), 5.0);
/// # }
/// ```
///
/// ```
/// use molangx::vm::{NoHostEnv, Value, VariableName};
///
/// let mut env = NoHostEnv { this: 2.34, ..NoHostEnv::new() };
/// env.variables.set(VariableName::new("x"), Value::Float(3.0));
/// let cx = env.cx();
/// assert_eq!(cx.variable(VariableName::parse("v.x").unwrap()), Some(&Value::Float(3.0)));
/// assert_eq!(cx.subjects.this, 2.34);
/// ```
#[derive(Debug)]
pub struct NoHostEnv {
    /// `variable.*`: the detached map.
    pub variables: VariableMap<NoHost>,
    /// `context.*`.
    pub context: ContextMap<NoHost>,
    /// The query implementations; none by default (see [`EvalCx::queries`]).
    pub queries: Option<QueryTable<NoHost>>,
    /// The random source, from the standard seeds: the same sequence in every `NoHostEnv`.
    pub rng: Xorshift128,
    /// The last run-time messages.
    pub sink: BoundedSink,
    /// `this`.
    pub this: f32,
    /// The budgets.
    pub limits: EvalLimits,
    /// Where `temp.*` lives; per evaluation by default.
    pub temps: Temps<TempMap<NoHost>>,
    /// The world-access object, which has nothing to access.
    pub host: NoHost,
}

impl NoHostEnv {
    /// An empty environment with the default budgets and no query implemented.
    pub fn new() -> Self {
        Self {
            variables: VariableMap::new(),
            context: ContextMap::new(),
            queries: None,
            rng: Xorshift128::new(),
            sink: BoundedSink::new(),
            this: 0.0,
            limits: EvalLimits::DEFAULT,
            temps: Temps::PerEvaluation,
            host: NoHost,
        }
    }

    /// The evaluation context over this environment.
    pub fn cx(&mut self) -> EvalCx<'_, 'static, NoHost> {
        EvalCx {
            subjects: Subjects {
                this: self.this,
                ..Subjects::none()
            },
            host: &mut self.host,
            variables: &mut self.variables,
            context: &self.context,
            queries: self.queries.as_ref(),
            rng: &mut self.rng,
            sink: &mut self.sink,
            limits: self.limits,
            temps: self.temps.as_mut(),
        }
    }
}

impl Default for NoHostEnv {
    fn default() -> Self {
        Self::new()
    }
}

/// An owning evaluation environment for a host: everything an [`EvalCx`] borrows except the
/// world-access object and the subjects.
///
/// Each part is a type parameter with a ready-made default, replaced by the `with_*` methods; the
/// budgets and the temps are plain fields.
///
/// ```
/// # #[cfg(feature = "stdlib")]
/// # {
/// use molangx::catalog::Side;
/// use molangx::compile::{CompileOptions, compile};
/// use molangx::stdlib::{self, query};
/// use molangx::version::MolangVersion;
/// use molangx::vm::{
///     ContextName, Host, HostAccess, HostEnv, QueryCx, QueryResult, QueryTable, Subjects, Value,
/// };
///
/// struct Farm;
/// impl Host for Farm {
///     type ActorRef = u32;
///     type ItemRef = ();
///     type BlockRef = ();
///     type Access<'w> = World;
/// }
///
/// /// The world: the health of each entity.
/// struct World(Vec<f32>);
/// impl HostAccess<Farm> for World {
///     fn resolve_actor(&self, _from: &Subjects<Farm>, actor: u32) -> Option<u32> {
///         ((actor as usize) < self.0.len()).then_some(actor)
///     }
/// }
///
/// fn health(cx: &mut QueryCx<'_, '_, Farm>) -> QueryResult<Farm> {
///     let Some(actor) = cx.subjects().actor else { return Ok(cx.default_value()) };
///     Ok(Value::Float(cx.host().0[actor as usize]))
/// }
///
/// let mut queries = QueryTable::new(stdlib::queries(Side::Server));
/// queries.set(query::HEALTH, health)?;
/// let mut env = HostEnv::new(queries);
/// env.context.set(ContextName::new("damage"), Value::Float(3.0));
/// let mut world = World(vec![10.0, 4.0]);
/// let (expr, _) = compile(
///     "query.health - context.damage",
///     &CompileOptions::server(MolangVersion::LATEST),
/// )
/// .into_result()
/// .unwrap();
/// assert_eq!(expr.eval_f32(&mut env.cx(&mut world, Subjects::actor(1))), 1.0);
/// # }
/// # Ok::<(), molangx::vm::UnknownQuery>(())
/// ```
pub struct HostEnv<H: Host, V = VariableMap<H>, C = ContextMap<H>, R = Xorshift128, S = BoundedSink>
{
    /// `variable.*` storage.
    pub variables: V,
    /// `context.*`.
    pub context: C,
    /// The query implementations, if any (see [`EvalCx::queries`]).
    pub queries: Option<QueryTable<H>>,
    /// The random source. [`HostEnv::new`] starts it from the standard seeds, the same in every
    /// environment; seed each one
    /// ([`seed_from_u64`](crate::rng::rand_core::SeedableRng::seed_from_u64)) unless identical
    /// sequences are wanted.
    pub rng: R,
    /// Where run-time diagnostics go.
    pub sink: S,
    /// The budgets.
    pub limits: EvalLimits,
    /// Where `temp.*` lives.
    pub temps: Temps<TempMap<H>>,
}

impl<H: Host> HostEnv<H> {
    /// An environment with the given queries, the default parts and budgets and per-evaluation
    /// temps.
    pub fn new(queries: QueryTable<H>) -> Self {
        Self {
            queries: Some(queries),
            ..Self::default()
        }
    }
}

impl<H: Host> Default for HostEnv<H> {
    /// The default parts and budgets, per-evaluation temps and no queries: every call returns
    /// its declared default.
    fn default() -> Self {
        Self {
            variables: VariableMap::new(),
            context: ContextMap::new(),
            queries: None,
            rng: Xorshift128::new(),
            sink: BoundedSink::new(),
            limits: EvalLimits::DEFAULT,
            temps: Temps::PerEvaluation,
        }
    }
}

impl<H: Host, V, C, R, S> HostEnv<H, V, C, R, S> {
    /// The same environment with another variable store.
    pub fn with_variables<V2: VariableStore<H>>(self, variables: V2) -> HostEnv<H, V2, C, R, S> {
        HostEnv {
            variables,
            context: self.context,
            queries: self.queries,
            rng: self.rng,
            sink: self.sink,
            limits: self.limits,
            temps: self.temps,
        }
    }

    /// The same environment with another context provider.
    pub fn with_context<C2: ContextProvider<H>>(self, context: C2) -> HostEnv<H, V, C2, R, S> {
        HostEnv {
            variables: self.variables,
            context,
            queries: self.queries,
            rng: self.rng,
            sink: self.sink,
            limits: self.limits,
            temps: self.temps,
        }
    }

    /// The same environment with another random source.
    pub fn with_rng<R2: Rng>(self, rng: R2) -> HostEnv<H, V, C, R2, S> {
        HostEnv {
            variables: self.variables,
            context: self.context,
            queries: self.queries,
            rng,
            sink: self.sink,
            limits: self.limits,
            temps: self.temps,
        }
    }

    /// The same environment with another sink.
    pub fn with_sink<S2: RuntimeSink>(self, sink: S2) -> HostEnv<H, V, C, R, S2> {
        HostEnv {
            variables: self.variables,
            context: self.context,
            queries: self.queries,
            rng: self.rng,
            sink,
            limits: self.limits,
            temps: self.temps,
        }
    }

    /// The evaluation context over this environment, the world-access object and the subjects.
    pub fn cx<'a, 'w>(
        &'a mut self,
        host: &'a mut H::Access<'w>,
        subjects: Subjects<H>,
    ) -> EvalCx<'a, 'w, H>
    where
        V: VariableStore<H>,
        C: ContextProvider<H>,
        R: Rng,
        S: RuntimeSink,
    {
        EvalCx {
            subjects,
            host,
            variables: &mut self.variables,
            context: &self.context,
            queries: self.queries.as_ref(),
            rng: &mut self.rng,
            sink: &mut self.sink,
            limits: self.limits,
            temps: self.temps.as_mut(),
        }
    }
}

impl<H: Host, V: Debug, C: Debug, R: Debug, S: Debug> Debug for HostEnv<H, V, C, R, S> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("HostEnv")
            .field("variables", &self.variables)
            .field("context", &self.context)
            .field("queries", &self.queries)
            .field("rng", &self.rng)
            .field("sink", &self.sink)
            .field("limits", &self.limits)
            .field("temps", &self.temps)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unnecessary_wraps)]

    use super::*;
    use crate::catalog::Side;
    use crate::rng::{FixedRng, sample};
    use crate::stdlib::{self, query};
    use crate::vm::{
        Access, CollectSink, NoContext, NoVariables, NullSink, QueryCx, QueryError, ResourceRef,
        RuntimeMsg, StructValue, TempName, VariableStorage, WorldGenPos,
    };

    #[derive(Debug)]
    struct TestHost;

    #[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
    enum Actor {
        /// `Handle(0)` is the null handle.
        Handle(u32),
        Id(u32),
    }

    impl Host for TestHost {
        type ActorRef = Actor;
        type ItemRef = u16;
        type BlockRef = ();
        type Access<'w> = World<'w>;
    }

    struct World<'w> {
        alive: &'w [u32],
    }

    impl HostAccess<TestHost> for World<'_> {
        fn resolve_actor(&self, from: &Subjects<TestHost>, actor: Actor) -> Option<Actor> {
            match actor {
                Actor::Handle(0) => None,
                Actor::Handle(n) => Some(Actor::Handle(n)),
                // An id resolves only from a subject with an actor.
                Actor::Id(n) => {
                    (from.actor.is_some() && self.alive.contains(&n)).then_some(Actor::Handle(n))
                }
            }
        }

        fn subjects_of(&self, actor: Actor) -> Subjects<TestHost> {
            Subjects {
                this: 9.0,
                ..Subjects::actor(actor)
            }
        }

        fn stored_actor(&self, actor: Actor) -> Actor {
            match actor {
                Actor::Handle(n) | Actor::Id(n) => Actor::Id(n),
            }
        }
    }

    type V = Value<TestHost>;

    const BAA: VariableName = VariableName::new("baa");

    fn with_cx<R>(
        subjects: Subjects<TestHost>,
        vars: &mut dyn VariableStore<TestHost>,
        ctx: &dyn ContextProvider<TestHost>,
        f: impl FnOnce(&mut EvalCx<'_, '_, TestHost>) -> R,
    ) -> R {
        let mut world = World { alive: &[1, 2, 3] };
        let mut rng = FixedRng::HALF;
        let mut sink = NullSink;
        let mut cx = EvalCx {
            subjects,
            host: &mut world,
            variables: vars,
            context: ctx,
            queries: None,
            rng: &mut rng,
            sink: &mut sink,
            limits: EvalLimits::default(),
            temps: Temps::PerEvaluation,
        };
        f(&mut cx)
    }

    fn zero(_cx: &mut QueryCx<'_, '_, TestHost>) -> Result<V, QueryError> {
        Ok(V::ZERO)
    }

    #[test]
    fn no_variable_map_means_writes_are_ignored_and_reads_are_missing() {
        let mut store = VariableStorage::<TestHost>::new();
        with_cx(Subjects::none(), &mut store, &NoContext, |cx| {
            cx.set_variable(BAA, V::Float(1.0));
            assert_eq!(cx.variable(BAA), None);
            assert_eq!(cx.public_variable(BAA), None);
        });
        assert!(store.local().is_none());

        let mut none = NoVariables;
        with_cx(
            Subjects::actor(Actor::Handle(1)),
            &mut none,
            &NoContext,
            |cx| {
                cx.set_variable(BAA, V::Float(1.0));
                assert_eq!(cx.variable(BAA), None);
            },
        );
    }

    #[test]
    fn detached_map_serves_actorless_subjects() {
        let mut store = VariableStorage::<TestHost>::with_local();
        store
            .local_mut()
            .set(VariableName::new("worldx"), V::Float(16.0));
        with_cx(
            Subjects::world_gen(WorldGenPos {
                x: 16,
                y: 64,
                z: -32,
            }),
            &mut store,
            &NoContext,
            |cx| {
                assert_eq!(
                    cx.variable(VariableName::new("worldx")),
                    Some(&V::Float(16.0))
                );
                cx.set_variable(VariableName::new("count"), V::Float(1.0));
                assert_eq!(
                    cx.variable(VariableName::new("count")),
                    Some(&V::Float(1.0))
                );
            },
        );
        assert!(store.actor(Actor::Id(1)).is_none());
        assert_eq!(store.local().unwrap().len(), 2);
        assert!(store.remove_local().is_some());
        assert!(store.local().is_none());
    }

    #[test]
    fn eval_cx_reads_and_writes_the_subject_actors_map() {
        let mut store = VariableStorage::<TestHost>::with_local();
        let cow = Actor::Id(1);
        store.actor_mut(cow).set_public(BAA, V::Float(1.23));
        store.refresh_snapshots();
        with_cx(Subjects::actor(cow), &mut store, &NoContext, |cx| {
            cx.set_variable(BAA, V::Float(2.34));
            assert_eq!(cx.variable(BAA), Some(&V::Float(2.34)));
            assert_eq!(cx.public_variable(BAA), Some(&V::Float(1.23)));
        });
        assert_eq!(store.actor(cow).unwrap().access(BAA), Some(Access::Public));
        assert!(store.local().unwrap().is_empty());
    }

    #[test]
    fn assignments_store_actor_ids() {
        let mut map = VariableMap::<TestHost>::new();
        with_cx(
            Subjects::actor(Actor::Handle(1)),
            &mut map,
            &NoContext,
            |cx| {
                cx.set_variable(VariableName::new("friend"), V::Actor(Actor::Handle(2)));
                cx.set_variable(
                    VariableName::new("herd"),
                    V::actor_array([
                        Actor::Handle(1),
                        Actor::Handle(0),
                        Actor::Id(3),
                        Actor::Id(77),
                    ]),
                );
                cx.set_variable(VariableName::new("n"), V::Float(1.0));
            },
        );
        assert_eq!(
            map.get(VariableName::new("friend")),
            Some(&V::Actor(Actor::Id(2)))
        );
        assert_eq!(
            map.get(VariableName::new("herd")),
            Some(&V::actor_array([Actor::Id(1), Actor::Id(3)]))
        );
        assert_eq!(map.get(VariableName::new("n")), Some(&V::Float(1.0)));
    }

    #[test]
    fn arrow_targets() {
        let mut map = VariableMap::<TestHost>::new();
        with_cx(
            Subjects::actor(Actor::Handle(1)),
            &mut map,
            &NoContext,
            |cx| {
                assert_eq!(
                    cx.arrow_target(&V::Actor(Actor::Handle(2))),
                    Some(Subjects {
                        this: 9.0,
                        ..Subjects::actor(Actor::Handle(2))
                    })
                );
                assert_eq!(
                    cx.arrow_target(&V::Actor(Actor::Id(3))),
                    Some(Subjects {
                        this: 9.0,
                        ..Subjects::actor(Actor::Handle(3))
                    })
                );
                assert_eq!(cx.arrow_target(&V::Actor(Actor::Handle(0))), None);
                assert_eq!(cx.arrow_target(&V::Actor(Actor::Id(77))), None);
                assert_eq!(cx.arrow_target(&V::Float(1.0)), None);
                assert_eq!(cx.arrow_target(&V::string("a")), None);
                assert_eq!(cx.arrow_target(&V::actor_array([Actor::Handle(2)])), None);
                assert_eq!(cx.arrow_target(&V::Item(4)), Some(Subjects::item(4)));
                assert_eq!(cx.resolve_actor(Actor::Id(2)), Some(Actor::Handle(2)));
            },
        );
        with_cx(Subjects::none(), &mut map, &NoContext, |cx| {
            assert_eq!(cx.arrow_target(&V::Actor(Actor::Id(3))), None);
            assert_eq!(
                cx.arrow_target(&V::Actor(Actor::Handle(3))),
                Some(Subjects {
                    this: 9.0,
                    ..Subjects::actor(Actor::Handle(3))
                })
            );
        });
    }

    /// An item on the left of `->` takes its subjects from `HostAccess::subjects_of_item`, given
    /// the subjects the arrow is evaluated from.
    #[test]
    fn an_item_target_takes_its_subjects_from_the_host_and_the_current_subjects() {
        struct Shop;

        impl Host for Shop {
            type ActorRef = u32;
            type ItemRef = u16;
            type BlockRef = ();
            type Access<'w> = Shelf;
        }

        struct Shelf;

        impl HostAccess<Shop> for Shelf {
            fn resolve_actor(&self, _from: &Subjects<Shop>, actor: u32) -> Option<u32> {
                Some(actor)
            }

            /// The holder of the item is the actor the arrow is evaluated from, and `this` is
            /// doubled.
            fn subjects_of_item(&self, from: &Subjects<Shop>, item: u16) -> Subjects<Shop> {
                Subjects {
                    actor: from.actor,
                    this: from.this * 2.0,
                    ..Subjects::item(item)
                }
            }
        }

        let mut env: HostEnv<Shop> = HostEnv::default();
        let mut shelf = Shelf;
        let cx = env.cx(
            &mut shelf,
            Subjects {
                this: 1.5,
                ..Subjects::actor(5)
            },
        );
        let held = Subjects {
            actor: Some(5),
            item: Some(4),
            this: 3.0,
            ..Subjects::none()
        };
        assert_eq!(cx.arrow_target(&Value::Item(4)), Some(held));
        // An actor target does not go through `subjects_of_item`, and a number has no subjects.
        assert_eq!(cx.arrow_target(&Value::Actor(7)), Some(Subjects::actor(7)));
        assert_eq!(cx.arrow_target(&Value::Float(4.0)), None);
    }

    #[test]
    fn context_provider() {
        let moo = ContextName::new("moo");
        let mut context = ContextMap::<TestHost>::from([(moo, V::Actor(Actor::Handle(1)))]);
        assert_eq!(context.context(moo), Some(V::Actor(Actor::Handle(1))));
        assert_eq!(context.context(ContextName::new("other")), None);
        assert_eq!(context.len(), 1);
        assert_eq!(
            context.set(ContextName::new("null_actor"), V::Actor(Actor::Handle(0))),
            None
        );
        assert_eq!(
            context.get(ContextName::new("null_actor")),
            Some(&V::Actor(Actor::Handle(0)))
        );
        assert_eq!(context.remove(moo), Some(V::Actor(Actor::Handle(1))));
        let mut map = VariableMap::<TestHost>::new();
        with_cx(Subjects::none(), &mut map, &context, |cx| {
            assert_eq!(
                cx.context(ContextName::parse("c.null_actor").unwrap()),
                Some(V::Actor(Actor::Handle(0)))
            );
            assert_eq!(cx.context(moo), None);
        });
        context.clear();
        assert!(context.is_empty());
        assert_eq!(ContextProvider::<TestHost>::context(&NoContext, moo), None);
    }

    #[test]
    fn a_public_variable_read_needs_an_actor() {
        let mut store = VariableStorage::<TestHost>::with_local();
        store.local_mut().set_public(BAA, V::Float(1.0));
        store.local_mut().refresh_snapshots();
        with_cx(Subjects::none(), &mut store, &NoContext, |cx| {
            assert_eq!(cx.variable(BAA), Some(&V::Float(1.0)));
            assert_eq!(cx.public_variable(BAA), None);
        });
    }

    #[test]
    fn a_public_variable_read_is_the_subject_actors_snapshot() {
        let mut store = VariableStorage::<TestHost>::new();
        let (cow, pig) = (Actor::Id(1), Actor::Id(2));
        store.actor_mut(cow).set_public(BAA, V::Float(1.0));
        store.actor_mut(pig).set_public(BAA, V::Float(2.0));
        store.refresh_snapshots();
        with_cx(Subjects::actor(pig), &mut store, &NoContext, |cx| {
            assert_eq!(cx.public_variable(BAA), Some(&V::Float(2.0)));
        });
    }

    #[test]
    fn a_write_goes_through_the_storable_form_and_keeps_an_existing_access() {
        let mut store = VariableStorage::<TestHost>::new();
        let cow = Actor::Id(1);
        store.actor_mut(cow).set_access(BAA, Access::Public);
        with_cx(Subjects::actor(cow), &mut store, &NoContext, |cx| {
            cx.set_variable(BAA, V::Actor(Actor::Handle(2)));
            cx.set_variable(VariableName::new("fresh"), V::Float(1.0));
        });
        let map = store.actor(cow).unwrap();
        assert_eq!(map.get(BAA), Some(&V::Actor(Actor::Id(2))));
        assert_eq!(map.access(BAA), Some(Access::Public));
        assert_eq!(
            map.access(VariableName::new("fresh")),
            Some(Access::Private)
        );
    }

    #[test]
    fn storable_converts_an_actor_and_leaves_every_other_kind_untouched() {
        let mut map = VariableMap::<TestHost>::new();
        with_cx(Subjects::none(), &mut map, &NoContext, |cx| {
            assert_eq!(
                cx.storable(V::Actor(Actor::Handle(5))),
                V::Actor(Actor::Id(5))
            );
            assert_eq!(cx.storable(V::Actor(Actor::Id(5))), V::Actor(Actor::Id(5)));
            assert_eq!(
                cx.storable(V::Actor(Actor::Handle(0))),
                V::Actor(Actor::Id(0))
            );
            for value in [
                V::Float(1.5),
                V::string("a"),
                V::Item(4),
                V::identity_matrix(),
                V::structure(StructValue::xy(1.0, 2.0)),
            ] {
                assert_eq!(cx.storable(value.clone()), value);
            }
            let holder = V::structure(StructValue::from([("who", V::Actor(Actor::Handle(2)))]));
            assert_eq!(cx.storable(holder.clone()), holder);
        });
    }

    #[test]
    fn storable_keeps_the_resolvable_entries_of_an_array_in_order_as_ids() {
        let mut map = VariableMap::<TestHost>::new();
        with_cx(
            Subjects::actor(Actor::Handle(1)),
            &mut map,
            &NoContext,
            |cx| {
                let herd = V::actor_array([
                    Actor::Id(3),
                    Actor::Handle(0),
                    Actor::Id(77),
                    Actor::Handle(9),
                    Actor::Id(1),
                ]);
                assert_eq!(
                    cx.storable(herd),
                    V::actor_array([Actor::Id(3), Actor::Id(9), Actor::Id(1)])
                );
                assert_eq!(cx.storable(V::actor_array([])), V::actor_array([]));
                assert_eq!(
                    cx.storable(V::actor_array([Actor::Handle(0), Actor::Id(77)])),
                    V::actor_array([])
                );
            },
        );
        with_cx(Subjects::none(), &mut map, &NoContext, |cx| {
            assert_eq!(
                cx.storable(V::actor_array([Actor::Id(1), Actor::Handle(4)])),
                V::actor_array([Actor::Id(4)])
            );
        });
    }

    #[test]
    fn resolve_actor_looks_up_from_the_current_subjects() {
        let mut map = VariableMap::<TestHost>::new();
        with_cx(
            Subjects::actor(Actor::Handle(1)),
            &mut map,
            &NoContext,
            |cx| {
                assert_eq!(cx.resolve_actor(Actor::Handle(5)), Some(Actor::Handle(5)));
                assert_eq!(cx.resolve_actor(Actor::Handle(0)), None);
                assert_eq!(cx.resolve_actor(Actor::Id(2)), Some(Actor::Handle(2)));
                assert_eq!(cx.resolve_actor(Actor::Id(50)), None);
            },
        );
        with_cx(Subjects::none(), &mut map, &NoContext, |cx| {
            assert_eq!(cx.resolve_actor(Actor::Id(2)), None);
            assert_eq!(cx.resolve_actor(Actor::Handle(5)), Some(Actor::Handle(5)));
        });
    }

    #[test]
    fn arrow_targets_other_than_actors_and_items_are_missing() {
        let mut map = VariableMap::<TestHost>::new();
        with_cx(
            Subjects::actor(Actor::Handle(1)),
            &mut map,
            &NoContext,
            |cx| {
                for target in [
                    V::Float(0.0),
                    V::string(""),
                    V::identity_matrix(),
                    V::structure(StructValue::new()),
                    V::actor_array([]),
                    V::Resource(ResourceRef::new("texture.default")),
                ] {
                    assert_eq!(cx.arrow_target(&target), None, "{target:?}");
                }
            },
        );
    }

    #[test]
    fn the_context_reads_through_the_provider() {
        let context = ContextMap::<TestHost>::from([(ContextName::new("a"), V::Float(1.0))]);
        let mut map = VariableMap::<TestHost>::new();
        with_cx(Subjects::none(), &mut map, &context, |cx| {
            assert_eq!(cx.context(ContextName::new("a")), Some(V::Float(1.0)));
            assert_eq!(cx.context(ContextName::new("b")), None);
        });
    }

    #[test]
    fn no_host_environment() {
        let mut env = NoHostEnv {
            this: 2.34,
            ..NoHostEnv::new()
        };
        env.variables.set(VariableName::new("x"), Value::Float(3.0));
        env.context
            .set(ContextName::new("moo"), Value::string("moo"));
        {
            let mut cx = env.cx();
            assert_eq!(
                cx.subjects,
                Subjects::<NoHost> {
                    this: 2.34,
                    ..Subjects::<NoHost>::none()
                }
            );
            assert_eq!(
                cx.variable(VariableName::new("x")),
                Some(&Value::Float(3.0))
            );
            cx.set_variable(VariableName::new("y"), Value::Float(4.0));
            assert_eq!(
                cx.context(ContextName::new("moo")),
                Some(Value::string("moo"))
            );
            assert_eq!(cx.limits, EvalLimits::DEFAULT);
            let first = sample(cx.rng);
            assert_eq!(first, sample(&mut Xorshift128::new()));
            assert_eq!(cx.arrow_target(&Value::Actor(())), None);
            assert!(cx.queries.is_none());
        }
        assert_eq!(
            env.variables.get(VariableName::new("y")),
            Some(&Value::Float(4.0))
        );
        assert!(env.sink.is_empty());
    }

    #[test]
    fn a_new_no_host_environment_starts_empty_with_the_defaults() {
        for env in [NoHostEnv::new(), NoHostEnv::default()] {
            assert!(env.variables.is_empty());
            assert!(env.context.is_empty());
            assert!(env.queries.is_none());
            assert_eq!(env.rng, Xorshift128::new());
            assert!(env.sink.is_empty());
            assert_eq!(env.this.to_bits(), 0);
            assert_eq!(env.limits, EvalLimits::DEFAULT);
            assert!(matches!(env.temps, Temps::PerEvaluation));
        }
    }

    #[test]
    fn the_default_sink_of_the_environments_is_bounded() {
        let mut no_host = NoHostEnv::new();
        for i in 0..100 {
            no_host
                .cx()
                .sink
                .runtime(RuntimeMsg::LoopLimit { limit: i });
        }
        assert_eq!(no_host.sink.messages().len(), BoundedSink::DEFAULT_CAPACITY);
        assert_eq!(no_host.sink.dropped(), 36);

        let mut env = HostEnv::<TestHost>::default();
        let mut world = World { alive: &[] };
        for i in 0..100 {
            env.cx(&mut world, Subjects::none())
                .sink
                .runtime(RuntimeMsg::LoopLimit { limit: i });
        }
        assert_eq!(env.sink.messages().len(), BoundedSink::DEFAULT_CAPACITY);
        assert_eq!(env.sink.dropped(), 36);
    }

    #[test]
    fn the_no_host_context_wires_every_part_of_the_environment() {
        let mut env = NoHostEnv {
            this: -1.5,
            limits: EvalLimits::NONE,
            ..NoHostEnv::new()
        };
        env.variables.set(BAA, Value::Float(1.0));
        env.context.set(ContextName::new("moo"), Value::Float(2.0));
        {
            let cx = env.cx();
            assert_eq!(
                cx.subjects,
                Subjects {
                    this: -1.5,
                    ..Subjects::none()
                }
            );
            assert_eq!(cx.limits, EvalLimits::NONE);
            assert_eq!(cx.variable(BAA), Some(&Value::Float(1.0)));
            assert_eq!(cx.context(ContextName::new("moo")), Some(Value::Float(2.0)));
            assert!(matches!(cx.temps, Temps::PerEvaluation));
            cx.sink.runtime(RuntimeMsg::PublicAccessUnderflow);
            cx.sink
                .query_error(QueryError::new(query::IS_BABY, "Error: query.is_baby"));
        }
        assert_eq!(
            env.sink.take(),
            [
                "molangx: a public-access scope was closed while none was open",
                "Error: query.is_baby"
            ]
        );
    }

    #[test]
    fn persistent_temps_stay_in_the_environment_across_contexts() {
        let mut env = NoHostEnv {
            temps: Temps::Kept(TempMap::new()),
            ..NoHostEnv::new()
        };
        env.cx()
            .temps
            .kept_mut()
            .unwrap()
            .set(TempName::new("t"), Value::Float(5.0));
        assert!(matches!(env.cx().temps, Temps::Kept(_)));
        assert_eq!(
            env.temps.kept().unwrap().get(TempName::new("t")),
            Some(&Value::Float(5.0))
        );
        assert!(matches!(NoHostEnv::new().cx().temps, Temps::PerEvaluation));
    }

    #[test]
    fn the_no_host_environment_debug_lists_its_parts() {
        let text = format!("{:?}", NoHostEnv::new());
        assert!(text.starts_with("NoHostEnv {"), "{text}");
        for field in [
            "variables",
            "context",
            "queries",
            "rng",
            "sink",
            "this",
            "limits",
            "temps",
        ] {
            assert!(text.contains(&format!("{field}:")), "{field} in {text}");
        }
    }

    #[test]
    fn a_new_host_environment_has_the_default_random_source_and_budgets() {
        let mut queries = QueryTable::new(stdlib::queries(Side::Server));
        queries.set(query::IS_BABY, zero).unwrap();
        let env = HostEnv::<TestHost>::new(queries);
        assert_eq!(env.queries.as_ref().map(QueryTable::implemented), Some(1));
        assert_eq!(env.limits, EvalLimits::DEFAULT);
        assert!(matches!(env.temps, Temps::PerEvaluation));
        assert_eq!(env.rng, Xorshift128::new());
        assert!(env.variables.is_empty());
        assert!(env.context.is_empty());
        assert!(env.sink.is_empty());
        assert!(HostEnv::<TestHost>::default().queries.is_none());
    }

    #[test]
    fn each_builder_replaces_one_part_and_keeps_the_rest() {
        let limits = EvalLimits {
            loop_iterations: Some(7),
            ..EvalLimits::DEFAULT
        };
        let mut queries = QueryTable::new(stdlib::queries(Side::Server));
        queries.set(query::IS_BABY, zero).unwrap();
        let mut base = HostEnv::<TestHost>::new(queries);
        base.limits = limits;
        base.temps = Temps::Kept(TempMap::new());

        let env = base.with_variables(VariableStorage::<TestHost>::with_local());
        assert!(env.variables.local().is_some());
        assert_eq!(
            (
                env.limits,
                env.queries.as_ref().map(QueryTable::implemented),
                matches!(env.temps, Temps::Kept(_))
            ),
            (limits, Some(1), true)
        );

        let env = env.with_context(NoContext);
        assert_eq!(
            (
                env.limits,
                env.queries.as_ref().map(QueryTable::implemented),
                matches!(env.temps, Temps::Kept(_))
            ),
            (limits, Some(1), true)
        );
        assert!(env.variables.local().is_some());

        let mut env = env.with_rng(FixedRng::HALF);
        let mut world = World { alive: &[] };
        assert_eq!(sample(env.cx(&mut world, Subjects::none()).rng), 0.5);
        assert_eq!(
            (
                env.limits,
                env.queries.as_ref().map(QueryTable::implemented),
                matches!(env.temps, Temps::Kept(_))
            ),
            (limits, Some(1), true)
        );

        let mut env = env.with_sink(CollectSink::new());
        env.cx(&mut world, Subjects::none())
            .sink
            .runtime(RuntimeMsg::PublicAccessUnderflow);
        assert_eq!(env.sink.messages.len(), 1);
        assert_eq!(
            (
                env.limits,
                env.queries.as_ref().map(QueryTable::implemented),
                matches!(env.temps, Temps::Kept(_))
            ),
            (limits, Some(1), true)
        );
        assert_eq!(sample(env.cx(&mut world, Subjects::none()).rng), 0.5);
    }

    #[test]
    fn the_no_host_budgets_are_those_of_its_context() {
        let mut env = NoHostEnv {
            limits: EvalLimits::NONE,
            ..NoHostEnv::new()
        };
        assert_eq!(env.cx().limits, EvalLimits::NONE);
        assert_eq!(NoHostEnv::new().limits, EvalLimits::DEFAULT);
    }

    #[test]
    fn the_host_context_borrows_the_world_and_takes_the_subjects_of_the_call() {
        let limits = EvalLimits::NONE;
        let mut env = HostEnv::<TestHost> {
            limits,
            ..HostEnv::default()
        };
        env.variables.set(BAA, V::Float(1.0));
        env.context.set(ContextName::new("moo"), V::Float(2.0));
        let mut world = World { alive: &[1] };
        {
            let mut cx = env.cx(
                &mut world,
                Subjects {
                    this: 3.0,
                    ..Subjects::actor(Actor::Id(1))
                },
            );
            assert_eq!(cx.subjects.this, 3.0);
            assert_eq!(cx.subjects.actor, Some(Actor::Id(1)));
            assert_eq!(cx.limits, limits);
            assert!(matches!(cx.temps, Temps::PerEvaluation));
            assert_eq!(cx.variable(BAA), Some(&V::Float(1.0)));
            assert_eq!(cx.context(ContextName::new("moo")), Some(V::Float(2.0)));
            assert_eq!(cx.resolve_actor(Actor::Id(1)), Some(Actor::Handle(1)));
            cx.set_variable(VariableName::new("written"), V::Actor(Actor::Handle(1)));
        }
        assert_eq!(
            env.variables.get(VariableName::new("written")),
            Some(&V::Actor(Actor::Id(1)))
        );
    }

    #[test]
    fn the_host_context_passes_persistent_temps_through() {
        let mut env = HostEnv::<TestHost> {
            temps: Temps::Kept(TempMap::new()),
            ..HostEnv::default()
        };
        let mut world = World { alive: &[] };
        env.cx(&mut world, Subjects::none())
            .temps
            .kept_mut()
            .unwrap()
            .set(TempName::new("t"), V::Float(1.0));
        assert_eq!(
            env.temps.kept().unwrap().get(TempName::new("t")),
            Some(&V::Float(1.0))
        );
    }

    #[test]
    fn temps_are_per_evaluation_by_default_and_lend_a_kept_map() {
        let mut temps = Temps::<TempMap<TestHost>>::default();
        assert_eq!(temps, Temps::PerEvaluation);
        assert!(temps.kept().is_none() && temps.kept_mut().is_none());
        assert!(matches!(temps.as_mut(), Temps::PerEvaluation));
        temps = Temps::Kept(TempMap::new());
        if let Temps::Kept(map) = temps.as_mut() {
            map.set(TempName::new("t"), V::Float(1.0));
        }
        temps
            .kept_mut()
            .unwrap()
            .set(TempName::new("u"), V::Float(2.0));
        let map = temps.kept().unwrap();
        assert_eq!(map.get(TempName::new("t")), Some(&V::Float(1.0)));
        assert_eq!(map.get(TempName::new("u")), Some(&V::Float(2.0)));
    }

    #[test]
    fn the_host_environment_debug_names_every_part() {
        let env = HostEnv::<TestHost>::default();
        let text = format!("{env:?}");
        assert!(text.starts_with("HostEnv {"), "{text}");
        for field in [
            "variables",
            "context",
            "queries",
            "rng",
            "sink",
            "limits",
            "temps",
        ] {
            assert!(text.contains(&format!("{field}:")), "{field} in {text}");
        }
        assert!(text.contains("temps: PerEvaluation"), "{text}");
    }
}
