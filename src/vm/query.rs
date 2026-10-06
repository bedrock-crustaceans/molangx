//! Query implementations: the host's [`QueryTable`] and the [`QueryCx`] a running query sees.
//!
//! The crate implements no query; one the host did not implement returns its declared default.

use std::fmt::{self, Debug};
use std::sync::Arc;

use thiserror::Error;

use super::error::QueryError;
use super::host::{Host, Subjects};
use super::name::{ContextName, VariableName};
use super::sink::RuntimeSink;
use super::value::Value;
use crate::catalog::{QueryCatalog, QueryDecl, QueryIndex};
use crate::hash::HashedStr;
use crate::rng::rand_core::Rng;
use crate::version::MolangVersion;

/// What a query implementation returns; an `Err` goes to the [`RuntimeSink`] and the call
/// returns the query's default.
pub type QueryResult<H> = Result<Value<H>, QueryError>;

/// A query implementation: a closure or a `fn` taking `&mut QueryCx<'_, '_, H>`.
///
/// Arguments arrive unevaluated: [`QueryCx::arg`] evaluates one when asked, so the query decides
/// which run and in which order. Argument counts are not enforced; the query checks its own.
///
/// A query with several version ranges branches on [`QueryCx::implementation`].
///
/// An implementation may capture settings. It must be `Send + Sync` because a [`QueryTable`]
/// shares it between clones and threads; the world of one evaluation is reached through
/// [`QueryCx::host`], not captured.
pub trait Query<H: Host>: Send + Sync {
    /// Runs the query.
    fn call(&self, cx: &mut QueryCx<'_, '_, H>) -> QueryResult<H>;
}

impl<H: Host, F> Query<H> for F
where
    F: Fn(&mut QueryCx<'_, '_, H>) -> QueryResult<H> + Send + Sync,
{
    fn call(&self, cx: &mut QueryCx<'_, '_, H>) -> QueryResult<H> {
        self(cx)
    }
}

/// A query name the table's catalogue does not declare.
#[derive(Error, Clone, Debug, PartialEq, Eq)]
#[error("{name} is not declared in the query table's catalogue")]
pub struct UnknownQuery {
    name: Box<str>,
}

impl UnknownQuery {
    fn undeclared(name: &str) -> Self {
        Self { name: name.into() }
    }

    /// The name that was looked up.
    pub fn name(&self) -> &str {
        &self.name
    }
}

/// The host's query implementations: one slot per declaration of a [`QueryCatalog`], each a stub
/// until a [`Query`] is installed by name.
///
/// # Calls
///
/// - An expression compiled against this very catalogue (the same one, not an equal one) goes
///   straight to its slot.
/// - From another catalogue, the table resolves its own declaration of the name at the
///   expression's version. The function runs only if that declaration serves the version and
///   returns no kind the expression's declaration does not; it then gets a [`QueryCx`] of the
///   table's catalogue.
///
/// When no function runs, the call returns the default of the expression's declaration. When a
/// function returns `Err`, it returns the default of the declaration the function was called
/// with.
///
/// ```
/// # #[cfg(feature = "stdlib")]
/// # {
/// use std::collections::HashMap;
///
/// use molangx::catalog::Side;
/// use molangx::compile::{CompileOptions, compile};
/// use molangx::stdlib::{self, query};
/// use molangx::version::MolangVersion;
/// use molangx::vm::{Host, HostAccess, HostEnv, QueryCx, QueryResult, QueryTable, Subjects, Value};
///
/// struct Game;
/// impl Host for Game {
///     type ActorRef = u32;
///     type ItemRef = ();
///     type BlockRef = ();
///     // The world-access object borrows the world for one evaluation.
///     type Access<'w> = WorldView<'w>;
/// }
///
/// struct World {
///     health: HashMap<u32, f32>,
/// }
///
/// struct WorldView<'w>(&'w World);
///
/// impl HostAccess<Game> for WorldView<'_> {
///     fn resolve_actor(&self, _from: &Subjects<Game>, actor: u32) -> Option<u32> {
///         self.0.health.contains_key(&actor).then_some(actor)
///     }
/// }
///
/// /// `query.health`: reads the world through `cx.host()`, scaled by a world setting.
/// fn health(cx: &mut QueryCx<'_, '_, Game>, difficulty: f32) -> QueryResult<Game> {
///     let Some(actor) = cx.subjects().actor else { return Ok(cx.default_value()) };
///     Ok(Value::Float(cx.host().0.health.get(&actor).copied().unwrap_or(0.0) * difficulty))
/// }
///
/// let catalog = stdlib::queries(Side::Server);
/// let mut table = QueryTable::new(catalog);
/// let difficulty = 0.5;
/// table.set(query::HEALTH, move |cx| health(cx, difficulty))?;
/// let mut env = HostEnv::new(table);
/// let world = World { health: HashMap::from([(7, 10.0)]) };
/// let (expr, _) = compile(
///     "query.health",
///     &CompileOptions::new(catalog.clone(), MolangVersion::LATEST),
/// )
/// .into_result()
/// .unwrap();
/// let mut view = WorldView(&world);
/// assert_eq!(expr.eval_f32(&mut env.cx(&mut view, Subjects::actor(7))), 5.0);
/// # }
/// # Ok::<(), molangx::vm::UnknownQuery>(())
/// ```
pub struct QueryTable<H: Host> {
    catalog: QueryCatalog,
    /// One slot per declaration of `catalog`.
    impls: Box<[Option<Arc<dyn Query<H>>>]>,
}

impl<H: Host> QueryTable<H> {
    /// A table for `catalog` in which every query is a stub: it evaluates no argument and
    /// returns its declared default.
    pub fn new(catalog: &QueryCatalog) -> Self {
        Self {
            catalog: catalog.clone(),
            impls: vec![None; catalog.len()].into_boxed_slice(),
        }
    }

    /// The table's catalogue.
    pub fn catalog(&self) -> &QueryCatalog {
        &self.catalog
    }

    fn index_of(&self, name: &str) -> Result<usize, UnknownQuery> {
        self.catalog
            .index_of(name)
            .map(QueryIndex::index)
            .ok_or_else(|| UnknownQuery::undeclared(name))
    }

    fn slot_mut(&mut self, name: &str) -> Result<&mut Option<Arc<dyn Query<H>>>, UnknownQuery> {
        let index = self.index_of(name)?;
        Ok(&mut self.impls[index])
    }

    /// Installs the function `f` as the implementation of the query with the full name `name`
    /// (`"query.health"`), replacing the one installed before.
    pub fn set<F>(&mut self, name: &str, f: F) -> Result<(), UnknownQuery>
    where
        F: Fn(&mut QueryCx<'_, '_, H>) -> QueryResult<H> + Send + Sync + 'static,
    {
        self.set_shared(name, Arc::new(f))
    }

    /// Installs `query` as the implementation of the query named `name`: a host type
    /// implementing [`Query`], or one implementation shared by several tables.
    pub fn set_shared(&mut self, name: &str, query: Arc<dyn Query<H>>) -> Result<(), UnknownQuery> {
        *self.slot_mut(name)? = Some(query);
        Ok(())
    }

    /// Turns the query named `name` back into its stub.
    pub fn unset(&mut self, name: &str) -> Result<(), UnknownQuery> {
        *self.slot_mut(name)? = None;
        Ok(())
    }

    /// The installed implementation of the query named `name`, `None` for a stub.
    pub fn get(&self, name: &str) -> Result<Option<&Arc<dyn Query<H>>>, UnknownQuery> {
        Ok(self.impls[self.index_of(name)?].as_ref())
    }

    /// Whether the query named `name` is still its stub.
    pub fn is_stub(&self, name: &str) -> Result<bool, UnknownQuery> {
        Ok(self.get(name)?.is_none())
    }

    /// Number of queries with an installed implementation.
    pub fn implemented(&self) -> usize {
        self.impls.iter().filter(|f| f.is_some()).count()
    }

    /// Runs the query of `cx` as described under [Calls](QueryTable#calls).
    pub fn call(&self, cx: &mut QueryCx<'_, '_, H>) -> Value<H> {
        if self.catalog.same(cx.catalog) {
            return match &self.impls[cx.index.index()] {
                Some(query) => cx.run(&**query),
                None => cx.default_value(),
            };
        }
        let Some(index) = self.catalog.index_of(cx.name()) else {
            return cx.default_value();
        };
        let decl = self.catalog.decl(index);
        if !cx.decl().shape().returns.contains(decl.shape().returns) {
            return cx.default_value();
        }
        let (Some(query), Some(implementation)) = (
            &self.impls[index.index()],
            decl.implementation_at(cx.version),
        ) else {
            return cx.default_value();
        };
        let mut own = QueryCx::resolved(
            &self.catalog,
            index,
            cx.version,
            implementation,
            &mut *cx.backend,
        );
        own.run(&**query)
    }
}

impl<H: Host> Clone for QueryTable<H> {
    fn clone(&self) -> Self {
        Self {
            catalog: self.catalog.clone(),
            impls: self.impls.clone(),
        }
    }
}

impl<H: Host> Debug for QueryTable<H> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("QueryTable")
            .field("catalog", &self.catalog)
            .field("implemented", &self.implemented())
            .field("total", &self.impls.len())
            .finish()
    }
}

/// What the evaluator provides to a running query, through [`QueryCx`].
///
/// `'w` is the world borrow of [`Host::Access`].
pub trait QueryBackend<'w, H: Host> {
    /// The subjects of the evaluation at the call (inside `->`, the target's).
    fn subjects(&self) -> Subjects<H>;
    /// The host's world-access object.
    fn host(&mut self) -> &mut H::Access<'w>;
    /// The random source of the evaluation.
    fn rng(&mut self) -> &mut dyn Rng;
    /// The run-time diagnostics sink, for messages a query logs without failing.
    fn sink(&mut self) -> &mut dyn RuntimeSink;
    /// The current value of a `variable.` name in the subjects' map.
    fn variable(&self, name: VariableName) -> Option<Value<H>>;
    /// The value of a `context.` name.
    fn context(&self, name: ContextName) -> Option<Value<H>>;
    /// Number of arguments the running call was written with.
    fn arg_count(&self) -> usize;
    /// Evaluates argument `index` of the running call in the same evaluation state; `None`,
    /// evaluating nothing, when there is no such argument.
    fn eval_arg(&mut self, index: usize) -> Option<Value<H>>;
}

/// What a running query sees: which query and implementation was called, and its evaluation.
pub struct QueryCx<'a, 'w, H: Host> {
    catalog: &'a QueryCatalog,
    index: QueryIndex,
    version: MolangVersion,
    implementation: u8,
    backend: &'a mut dyn QueryBackend<'w, H>,
}

impl<'a, 'w, H: Host> QueryCx<'a, 'w, H> {
    /// A context for one call of `name` from `catalog` at `version`; `None` when the catalogue
    /// does not declare the name or no range of it serves `version`.
    pub fn new(
        catalog: &'a QueryCatalog,
        name: &str,
        version: MolangVersion,
        backend: &'a mut dyn QueryBackend<'w, H>,
    ) -> Option<Self> {
        let index = catalog.index_of(name)?;
        let implementation = catalog.decl(index).implementation_at(version)?;
        Some(Self::resolved(
            catalog,
            index,
            version,
            implementation,
            backend,
        ))
    }

    pub(crate) fn resolved(
        catalog: &'a QueryCatalog,
        index: QueryIndex,
        version: MolangVersion,
        implementation: u8,
        backend: &'a mut dyn QueryBackend<'w, H>,
    ) -> Self {
        Self {
            catalog,
            index,
            version,
            implementation,
            backend,
        }
    }

    /// The full name of the query being called (`"query.health"`).
    pub fn name(&self) -> &'a str {
        self.decl().name()
    }

    /// The declaration of the query being called.
    pub fn decl(&self) -> &'a QueryDecl {
        self.catalog.decl(self.index)
    }

    /// The Molang version the calling expression was compiled at.
    pub fn version(&self) -> MolangVersion {
        self.version
    }

    /// The position in [`QueryShape::ranges`](crate::catalog::QueryShape::ranges) of the range that
    /// serves the version.
    pub fn implementation(&self) -> u8 {
        self.implementation
    }

    /// The declared no-subject default of the query.
    pub fn default_value(&self) -> Value<H> {
        Value::from(self.decl().shape().default_return)
    }

    /// The subjects of the evaluation (inside `->`, the target's).
    pub fn subjects(&self) -> Subjects<H> {
        self.backend.subjects()
    }

    /// The host's world-access object, with its full type.
    pub fn host(&mut self) -> &mut H::Access<'w> {
        self.backend.host()
    }

    /// The random source; [`rng::sample`](crate::rng::sample) draws a sample as the operators do.
    pub fn rng(&mut self) -> &mut dyn Rng {
        self.backend.rng()
    }

    /// The run-time sink, for a message logged while still returning a value.
    pub fn sink(&mut self) -> &mut dyn RuntimeSink {
        self.backend.sink()
    }

    /// Number of arguments the call was written with (0 for `query.x` without parentheses).
    pub fn arg_count(&self) -> usize {
        self.backend.arg_count()
    }

    /// Evaluates argument `index` of the call; `None` when the call has no such argument.
    ///
    /// Each call evaluates the argument again, with its side effects; keep a value needed twice.
    pub fn arg(&mut self, index: usize) -> Option<Value<H>> {
        self.backend.eval_arg(index)
    }

    /// Evaluates argument `index` as a number. `None` when there is no such argument or it did
    /// not evaluate to a float.
    pub fn arg_f32(&mut self, index: usize) -> Option<f32> {
        match self.arg(index)? {
            Value::Float(x) => Some(x),
            _ => None,
        }
    }

    /// Evaluates argument `index` as a string (its hash). `None` when there is no such argument
    /// or it did not evaluate to a string.
    pub fn arg_hash(&mut self, index: usize) -> Option<HashedStr> {
        self.arg(index)?.as_hash()
    }

    /// Evaluates argument `index` as an actor. `None` when there is no such argument or it did
    /// not evaluate to an actor.
    pub fn arg_actor(&mut self, index: usize) -> Option<H::ActorRef> {
        self.arg(index)?.as_actor()
    }

    /// The current value of a `variable.` name in the subjects' map.
    pub fn variable(&self, name: VariableName) -> Option<Value<H>> {
        self.backend.variable(name)
    }

    /// The value of a `context.` name.
    pub fn context(&self, name: ContextName) -> Option<Value<H>> {
        self.backend.context(name)
    }

    /// An error of this query with the text `message`.
    pub fn error(&self, message: impl fmt::Display) -> QueryError {
        QueryError::new(self.decl().shared_name(), message)
    }

    /// Runs `query` in this context; an error goes to the sink and yields the default.
    fn run(&mut self, query: &dyn Query<H>) -> Value<H> {
        query.call(self).unwrap_or_else(|error| {
            self.sink().query_error(error);
            self.default_value()
        })
    }
}

impl<H: Host> Debug for QueryCx<'_, '_, H> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("QueryCx")
            .field("query", &self.name())
            .field("version", &self.version)
            .field("implementation", &self.implementation)
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unnecessary_wraps)]

    use std::cell::Cell;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;
    use crate::catalog::{
        DefaultReturn, QueryDecl, QuerySetMask, QueryShape, ReturnType, Side, VersionRange,
        VersionRanges,
    };
    use crate::rng::{FixedRng, sample};
    use crate::stdlib::query;
    use crate::vm::{CollectSink, HostAccess, RuntimeMsg, StructValue};

    #[derive(Debug)]
    struct TestHost;

    impl Host for TestHost {
        type ActorRef = u32;
        type ItemRef = ();
        type BlockRef = ();
        type Access<'w> = World<'w>;
    }

    struct World<'w> {
        babies: &'w [u32],
    }

    impl HostAccess<TestHost> for World<'_> {
        fn resolve_actor(&self, _from: &Subjects<TestHost>, actor: u32) -> Option<u32> {
            Some(actor)
        }
    }

    type V = Value<TestHost>;

    type Imp = fn(&mut QueryCx<'_, '_, TestHost>) -> QueryResult<TestHost>;

    fn table_of(catalog: &QueryCatalog, queries: &[(&str, Imp)]) -> QueryTable<TestHost> {
        let mut table = QueryTable::new(catalog);
        for &(name, query) in queries {
            table.set(name, query).unwrap();
        }
        table
    }

    fn decl(name: &str, returns: ReturnType, default_return: DefaultReturn) -> QueryDecl {
        QueryDecl::new(
            name,
            QueryShape {
                returns,
                default_return,
                ..QueryShape::DEFAULT
            },
        )
        .unwrap()
    }

    fn client() -> &'static QueryCatalog {
        crate::stdlib::queries(Side::Client)
    }

    struct MockVm<'a, 'w> {
        subjects: Subjects<TestHost>,
        world: &'a mut World<'w>,
        rng: FixedRng,
        sink: CollectSink,
        args: Vec<V>,
        evaluated: Vec<usize>,
        counted: Cell<usize>,
    }

    impl<'a, 'w> MockVm<'a, 'w> {
        fn new(world: &'a mut World<'w>, subjects: Subjects<TestHost>, args: Vec<V>) -> Self {
            Self {
                subjects,
                world,
                rng: FixedRng::HALF,
                sink: CollectSink::new(),
                args,
                evaluated: Vec::new(),
                counted: Cell::new(0),
            }
        }

        fn call(&mut self, table: &QueryTable<TestHost>, name: &str, version: MolangVersion) -> V {
            let mut cx = QueryCx::new(client(), name, version, self)
                .expect("query available at this version");
            table.call(&mut cx)
        }
    }

    impl<'w> QueryBackend<'w, TestHost> for MockVm<'_, 'w> {
        fn subjects(&self) -> Subjects<TestHost> {
            self.subjects
        }

        fn host(&mut self) -> &mut World<'w> {
            self.world
        }

        fn rng(&mut self) -> &mut dyn Rng {
            &mut self.rng
        }

        fn sink(&mut self) -> &mut dyn RuntimeSink {
            &mut self.sink
        }

        fn variable(&self, name: VariableName) -> Option<V> {
            (name == VariableName::new("speed")).then_some(V::Float(4.0))
        }

        fn context(&self, name: ContextName) -> Option<V> {
            (name == ContextName::new("moo")).then_some(V::Actor(1))
        }

        fn arg_count(&self) -> usize {
            self.counted.set(self.counted.get() + 1);
            self.args.len()
        }

        fn eval_arg(&mut self, index: usize) -> Option<V> {
            let value = self.args.get(index)?.clone();
            self.evaluated.push(index);
            Some(value)
        }
    }

    fn is_baby(cx: &mut QueryCx<'_, '_, TestHost>) -> QueryResult<TestHost> {
        let Some(actor) = cx.subjects().actor else {
            return Ok(V::ZERO);
        };
        Ok(V::bool(cx.host().babies.contains(&actor)))
    }

    fn sum(cx: &mut QueryCx<'_, '_, TestHost>) -> QueryResult<TestHost> {
        let mut total = 0.0;
        for i in 0..cx.arg_count() {
            total += cx.arg_f32(i).ok_or_else(|| {
                cx.error(format_args!(
                    "argument {i} of {} is not a number",
                    cx.name()
                ))
            })?;
        }
        Ok(V::Float(total))
    }

    fn second_only(cx: &mut QueryCx<'_, '_, TestHost>) -> QueryResult<TestHost> {
        Ok(cx.arg(1).unwrap_or(V::ZERO))
    }

    fn remaining_use_duration(cx: &mut QueryCx<'_, '_, TestHost>) -> QueryResult<TestHost> {
        Ok(V::Float(if cx.implementation() == 0 { 10.0 } else { 20.0 }))
    }

    fn needs_item(cx: &mut QueryCx<'_, '_, TestHost>) -> QueryResult<TestHost> {
        Err(cx.error(format_args!("{} has no item", cx.name())))
    }

    fn logs_and_draws(cx: &mut QueryCx<'_, '_, TestHost>) -> QueryResult<TestHost> {
        cx.sink().runtime(RuntimeMsg::PublicAccessUnderflow);
        let sample = sample(cx.rng());
        let moo = cx
            .context(ContextName::new("moo"))
            .and_then(|v| v.as_actor())
            .map_or(0.0, |a| a as f32);
        assert_eq!(cx.variable(VariableName::new("x")), None);
        assert_eq!(cx.name(), "query.life_time");
        Ok(V::Float(sample + moo))
    }

    fn returns_struct(_cx: &mut QueryCx<'_, '_, TestHost>) -> QueryResult<TestHost> {
        Ok(V::structure(StructValue::rgba(1.0, 0.5, 0.25, 1.0)))
    }

    fn fails_with_a_struct_default(cx: &mut QueryCx<'_, '_, TestHost>) -> QueryResult<TestHost> {
        Err(cx.error(format_args!("Error: {} failed.", cx.name())))
    }

    #[test]
    fn stubs_return_the_default_of_each_query() {
        let table = QueryTable::<TestHost>::new(client());
        assert_eq!(table.implemented(), 0);
        let mut world = World { babies: &[] };
        let mut vm = MockVm::new(&mut world, Subjects::none(), vec![V::Float(1.0)]);
        for decl in client() {
            assert!(table.is_stub(decl.name()).unwrap());
            assert!(table.get(decl.name()).unwrap().is_none());
            let version = decl.shape().ranges.as_slice()[0].first();
            let mut cx = QueryCx::new(client(), decl.name(), version, &mut vm).unwrap();
            assert_eq!(
                table.call(&mut cx),
                V::from(decl.shape().default_return),
                "{}",
                decl.name()
            );
        }
        assert!(vm.evaluated.is_empty());
        assert_eq!(vm.counted.get(), 0);
        assert!(vm.sink.is_empty());
        assert_eq!(
            client()
                .get(query::SPELLCOLOR)
                .unwrap()
                .shape()
                .default_return,
            DefaultReturn::StructRgba0
        );
        assert_eq!(
            vm.call(&table, query::SPELLCOLOR, MolangVersion::LATEST),
            V::structure(StructValue::rgba(0.0, 0.0, 0.0, 0.0))
        );
        assert_eq!(
            vm.call(&table, query::COMBINE_ENTITIES, MolangVersion::LATEST),
            V::actor_array([])
        );
        assert_eq!(
            vm.call(&table, query::IS_BABY, MolangVersion::LATEST),
            V::Float(0.0)
        );
    }

    #[test]
    fn installed_queries_see_subjects_and_the_world() {
        let mut table = QueryTable::<TestHost>::new(client());
        table.set(query::IS_BABY, is_baby).unwrap();
        table.set(query::LIFE_TIME, logs_and_draws).unwrap();
        assert_eq!(table.implemented(), 2);
        assert!(!table.is_stub(query::IS_BABY).unwrap());

        let babies = [1];
        let mut world = World { babies: &babies };
        let mut vm = MockVm::new(&mut world, Subjects::actor(1), vec![]);
        assert_eq!(
            vm.call(&table, query::IS_BABY, MolangVersion::LATEST),
            V::Float(1.0)
        );
        vm.subjects = Subjects::actor(2);
        assert_eq!(
            vm.call(&table, query::IS_BABY, MolangVersion::LATEST),
            V::Float(0.0)
        );
        vm.subjects = Subjects::none();
        assert_eq!(
            vm.call(&table, query::IS_BABY, MolangVersion::LATEST),
            V::Float(0.0)
        );

        assert_eq!(
            vm.call(&table, query::LIFE_TIME, MolangVersion::LATEST),
            V::Float(1.5)
        );
        assert_eq!(vm.sink.messages.len(), 1);

        table.unset(query::IS_BABY).unwrap();
        assert!(table.is_stub(query::IS_BABY).unwrap());
        assert_eq!(table.clone().implemented(), 1);
    }

    #[test]
    fn arguments_are_evaluated_lazily() {
        let table = table_of(
            client(),
            &[(query::LIFE_TIME, sum), (query::ANGER_LEVEL, second_only)],
        );
        let mut world = World { babies: &[] };
        let mut vm = MockVm::new(
            &mut world,
            Subjects::none(),
            vec![V::Float(1.0), V::Float(2.0), V::Float(3.0), V::Float(1.0)],
        );
        assert_eq!(
            vm.call(&table, query::LIFE_TIME, MolangVersion::LATEST),
            V::Float(7.0)
        );
        assert_eq!(vm.evaluated, [0, 1, 2, 3]);

        vm.evaluated.clear();
        assert_eq!(
            vm.call(&table, query::ANGER_LEVEL, MolangVersion::LATEST),
            V::Float(2.0)
        );
        assert_eq!(vm.evaluated, [1]);

        vm.args = vec![V::string("moo"), V::Actor(5)];
        let mut cx =
            QueryCx::new(client(), query::LIFE_TIME, MolangVersion::LATEST, &mut vm).unwrap();
        assert_eq!(cx.arg_count(), 2);
        assert_eq!(cx.arg_f32(0), None);
        assert_eq!(cx.arg_hash(0), V::string("moo").as_hash());
        assert_eq!(cx.arg_actor(1), Some(5));
        assert_eq!(cx.arg_hash(1), None);
        assert_eq!(cx.arg(2), None);
        assert_eq!(cx.arg_f32(9), None);
    }

    #[test]
    fn query_errors_are_logged_and_yield_the_default() {
        let table = table_of(
            client(),
            &[(query::MAX_DURABILITY, needs_item), (query::LIFE_TIME, sum)],
        );
        let mut world = World { babies: &[] };
        let mut vm = MockVm::new(
            &mut world,
            Subjects::none(),
            vec![V::Float(1.0), V::string("two")],
        );
        assert_eq!(
            vm.call(&table, query::MAX_DURABILITY, MolangVersion::LATEST),
            V::from(
                client()
                    .get(query::MAX_DURABILITY)
                    .unwrap()
                    .shape()
                    .default_return
            )
        );
        assert_eq!(
            vm.call(&table, query::LIFE_TIME, MolangVersion::LATEST),
            V::Float(0.0)
        );
        assert_eq!(
            vm.sink.messages,
            [
                "query.max_durability has no item",
                "argument 1 of query.life_time is not a number"
            ]
        );
    }

    #[test]
    fn names_with_two_ranges_branch_on_the_implementation() {
        let table = table_of(
            client(),
            &[(query::ITEM_REMAINING_USE_DURATION, remaining_use_duration)],
        );
        let mut world = World { babies: &[] };
        let mut vm = MockVm::new(&mut world, Subjects::none(), vec![]);
        assert_eq!(
            vm.call(
                &table,
                query::ITEM_REMAINING_USE_DURATION,
                MolangVersion::V1
            ),
            V::Float(10.0)
        );
        assert_eq!(
            vm.call(
                &table,
                query::ITEM_REMAINING_USE_DURATION,
                MolangVersion::V2
            ),
            V::Float(20.0)
        );
        assert_eq!(
            vm.call(
                &table,
                query::ITEM_REMAINING_USE_DURATION,
                MolangVersion::LATEST
            ),
            V::Float(20.0)
        );
        assert_eq!(table.implemented(), 1);
    }

    #[test]
    fn stubs_return_the_metadata_default_of_several_kinds() {
        let table = QueryTable::<TestHost>::new(client());
        let mut world = World { babies: &[] };
        let mut vm = MockVm::new(&mut world, Subjects::none(), vec![]);
        let latest = MolangVersion::LATEST;
        assert_eq!(vm.call(&table, query::IS_BABY, latest), V::Float(0.0));
        assert_eq!(
            vm.call(&table, query::ARMOR_COLOR_SLOT, latest),
            V::Float(1.0)
        );
        assert_eq!(
            vm.call(&table, query::TIME_SINCE_LAST_VIBRATION_DETECTION, latest),
            V::Float(-1.0)
        );
        assert_eq!(
            vm.call(&table, query::TICKS_SINCE_LAST_KINETIC_WEAPON_HIT, latest),
            V::Float(-1.0)
        );
        assert_eq!(
            vm.call(&table, query::OWNER_IDENTIFIER, latest),
            V::string("")
        );
        assert_eq!(
            vm.call(&table, query::GET_EQUIPPED_ITEM_NAME, latest),
            V::string("")
        );
        assert_eq!(
            vm.call(&table, query::COMBINE_ENTITIES, latest),
            V::actor_array([])
        );
        assert_eq!(
            vm.call(&table, query::SPELLCOLOR, latest),
            V::from(DefaultReturn::StructRgba0)
        );
        assert!(vm.sink.is_empty());
    }

    #[test]
    fn a_second_query_replaces_the_first() {
        let mut table = table_of(
            client(),
            &[(query::IS_BABY, is_baby), (query::LIFE_TIME, sum)],
        );
        assert_eq!(table.implemented(), 2);
        let mut world = World { babies: &[] };
        let mut vm = MockVm::new(
            &mut world,
            Subjects::none(),
            vec![V::Float(4.0), V::Float(5.0)],
        );
        assert_eq!(
            vm.call(&table, query::IS_BABY, MolangVersion::LATEST),
            V::Float(0.0)
        );
        table.set(query::IS_BABY, second_only).unwrap();
        assert_eq!(table.implemented(), 2);
        assert_eq!(
            vm.call(&table, query::IS_BABY, MolangVersion::LATEST),
            V::Float(5.0)
        );
        let get = table.get(query::IS_BABY).unwrap().expect("installed");
        let mut cx =
            QueryCx::new(client(), query::IS_BABY, MolangVersion::LATEST, &mut vm).unwrap();
        assert_eq!(get.call(&mut cx), Ok(V::Float(5.0)));
    }

    #[test]
    fn a_closure_query_keeps_what_it_captures() {
        let mut table = QueryTable::<TestHost>::new(client());
        let scale = 3.0;
        table
            .set(query::LIFE_TIME, move |cx| {
                Ok(V::Float(cx.arg_f32(0).unwrap_or(0.0) * scale))
            })
            .unwrap();
        let calls = Arc::new(AtomicUsize::new(0));
        let counter = Arc::clone(&calls);
        table
            .set(query::IS_BABY, move |_| {
                counter.fetch_add(1, Ordering::Relaxed);
                Ok(V::Float(1.0))
            })
            .unwrap();
        let mut world = World { babies: &[] };
        let mut vm = MockVm::new(&mut world, Subjects::none(), vec![V::Float(2.0)]);
        assert_eq!(
            vm.call(&table, query::LIFE_TIME, MolangVersion::LATEST),
            V::Float(6.0)
        );
        let copy = table.clone();
        vm.call(&table, query::IS_BABY, MolangVersion::LATEST);
        vm.call(&copy, query::IS_BABY, MolangVersion::LATEST);
        assert_eq!(
            calls.load(Ordering::Relaxed),
            2,
            "a clone shares the closure"
        );
    }

    #[test]
    fn a_table_is_send_and_sync() {
        fn shared<T: Send + Sync>() {}
        shared::<QueryTable<TestHost>>();
    }

    #[test]
    fn unset_turns_a_query_back_into_its_stub() {
        let mut table = QueryTable::<TestHost>::new(client());
        table.set(query::IS_BABY, is_baby).unwrap();
        table.unset(query::IS_BABY).unwrap();
        assert!(table.is_stub(query::IS_BABY).unwrap());
        assert!(table.get(query::IS_BABY).unwrap().is_none());
        assert_eq!(table.implemented(), 0);
        table.unset(query::IS_BABY).unwrap();
        table.unset(query::LIFE_TIME).unwrap();
        assert_eq!(table.implemented(), 0);
    }

    #[test]
    fn a_clone_of_the_table_is_independent() {
        let mut table = QueryTable::<TestHost>::new(client());
        let mut copy = table.clone();
        copy.set(query::IS_BABY, is_baby).unwrap();
        assert_eq!(table.implemented(), 0);
        assert_eq!(copy.implemented(), 1);
        table.set(query::LIFE_TIME, sum).unwrap();
        assert!(copy.is_stub(query::LIFE_TIME).unwrap());
        assert!(table.is_stub(query::IS_BABY).unwrap());
    }

    #[test]
    fn debug_counts_the_installed_and_the_total_queries() {
        let table = QueryTable::<TestHost>::new(client());
        assert_eq!(
            format!("{table:?}"),
            format!(
                "QueryTable {{ catalog: {:?}, implemented: 0, total: {} }}",
                client(),
                client().len()
            )
        );
        let table = table_of(
            client(),
            &[(query::IS_BABY, is_baby), (query::LIFE_TIME, sum)],
        );
        assert_eq!(
            format!("{table:?}"),
            format!(
                "QueryTable {{ catalog: {:?}, implemented: 2, total: {} }}",
                client(),
                client().len()
            )
        );
    }

    #[test]
    fn a_successful_call_returns_the_value_untouched_and_logs_nothing() {
        let table = table_of(client(), &[(query::SPELLCOLOR, returns_struct)]);
        let mut world = World { babies: &[] };
        let mut vm = MockVm::new(&mut world, Subjects::none(), vec![]);
        assert_eq!(
            vm.call(&table, query::SPELLCOLOR, MolangVersion::LATEST),
            V::structure(StructValue::rgba(1.0, 0.5, 0.25, 1.0))
        );
        assert!(vm.sink.is_empty());
    }

    #[test]
    fn an_error_returns_the_default_of_the_query_even_when_it_is_a_struct() {
        let table = table_of(
            client(),
            &[(query::SPELLCOLOR, fails_with_a_struct_default)],
        );
        let mut world = World { babies: &[] };
        let mut vm = MockVm::new(&mut world, Subjects::none(), vec![]);
        assert_eq!(
            vm.call(&table, query::SPELLCOLOR, MolangVersion::LATEST),
            V::structure(StructValue::rgba(0.0, 0.0, 0.0, 0.0))
        );
        assert_eq!(vm.sink.messages, ["Error: query.spellcolor failed."]);
    }

    #[test]
    fn every_failed_call_reports_one_message() {
        let table = table_of(client(), &[(query::MAX_DURABILITY, needs_item)]);
        let mut world = World { babies: &[] };
        let mut vm = MockVm::new(&mut world, Subjects::none(), vec![]);
        for _ in 0..3 {
            vm.call(&table, query::MAX_DURABILITY, MolangVersion::LATEST);
        }
        assert_eq!(vm.sink.messages.len(), 3);
    }

    #[test]
    fn an_argument_out_of_range_evaluates_nothing_and_one_in_range_evaluates_on_each_ask() {
        let mut world = World { babies: &[] };
        let mut vm = MockVm::new(
            &mut world,
            Subjects::none(),
            vec![V::Float(1.0), V::Float(2.0)],
        );
        {
            let mut cx =
                QueryCx::new(client(), query::LIFE_TIME, MolangVersion::LATEST, &mut vm).unwrap();
            assert_eq!(cx.arg(2), None);
            assert_eq!(cx.arg(usize::MAX), None);
            assert_eq!(cx.arg(0), Some(V::Float(1.0)));
            assert_eq!(cx.arg(0), Some(V::Float(1.0)));
            assert_eq!(cx.arg_f32(1), Some(2.0));
        }
        assert_eq!(vm.evaluated, [0, 0, 1]);
    }

    #[test]
    fn the_typed_argument_accessors_answer_only_for_their_own_kind() {
        let mut world = World { babies: &[] };
        let mut vm = MockVm::new(
            &mut world,
            Subjects::none(),
            vec![
                V::Float(1.5),
                V::string("moo"),
                V::Actor(7),
                V::structure(StructValue::xy(1.0, 2.0)),
            ],
        );
        let mut cx =
            QueryCx::new(client(), query::LIFE_TIME, MolangVersion::LATEST, &mut vm).unwrap();
        assert_eq!(cx.arg_f32(0), Some(1.5));
        assert_eq!(
            (cx.arg_f32(1), cx.arg_f32(2), cx.arg_f32(3)),
            (None, None, None)
        );
        assert_eq!(cx.arg_hash(1), Some(HashedStr::new("moo")));
        assert_eq!(
            (cx.arg_hash(0), cx.arg_hash(2), cx.arg_hash(3)),
            (None, None, None)
        );
        assert_eq!(cx.arg_actor(2), Some(7));
        assert_eq!(
            (cx.arg_actor(0), cx.arg_actor(1), cx.arg_actor(3)),
            (None, None, None)
        );
        assert_eq!(cx.arg(3), Some(V::structure(StructValue::xy(1.0, 2.0))));
    }

    #[test]
    fn the_argument_count_is_the_calls() {
        let mut world = World { babies: &[] };
        let mut three = MockVm::new(&mut world, Subjects::none(), vec![V::ZERO; 3]);
        assert_eq!(
            QueryCx::new(
                client(),
                query::LIFE_TIME,
                MolangVersion::LATEST,
                &mut three
            )
            .unwrap()
            .arg_count(),
            3
        );
        let mut world = World { babies: &[] };
        let mut none = MockVm::new(&mut world, Subjects::none(), vec![]);
        let mut cx =
            QueryCx::new(client(), query::LIFE_TIME, MolangVersion::LATEST, &mut none).unwrap();
        assert_eq!(cx.arg_count(), 0);
        assert_eq!(cx.arg(0), None);
    }

    #[test]
    fn the_context_forwards_every_call_to_the_backend() {
        let babies = [4];
        let mut world = World { babies: &babies };
        let mut vm = MockVm::new(
            &mut world,
            Subjects {
                this: 2.5,
                ..Subjects::actor(4)
            },
            vec![],
        );
        {
            let mut cx =
                QueryCx::new(client(), query::IS_BABY, MolangVersion::LATEST, &mut vm).unwrap();
            assert_eq!(cx.arg_count(), 0);
            assert_eq!(
                cx.subjects(),
                Subjects {
                    this: 2.5,
                    ..Subjects::actor(4)
                }
            );
            assert_eq!(cx.host().babies, [4]);
            assert_eq!(sample(cx.rng()), 0.5);
            cx.sink().runtime(RuntimeMsg::PublicAccessUnderflow);
            assert_eq!(cx.variable(VariableName::new("speed")), Some(V::Float(4.0)));
            assert_eq!(cx.variable(VariableName::new("other")), None);
            assert_eq!(cx.context(ContextName::new("moo")), Some(V::Actor(1)));
            assert_eq!(cx.context(ContextName::new("other")), None);
        }
        assert_eq!(vm.sink.messages.len(), 1);
    }

    #[test]
    fn the_context_carries_the_query_the_version_and_the_implementation() {
        let mut world = World { babies: &[] };
        let mut vm = MockVm::new(&mut world, Subjects::none(), vec![]);
        let cx = QueryCx::new(
            client(),
            query::ITEM_REMAINING_USE_DURATION,
            MolangVersion::LATEST,
            &mut vm,
        )
        .unwrap();
        assert_eq!(cx.name(), query::ITEM_REMAINING_USE_DURATION);
        assert_eq!(cx.version(), MolangVersion::LATEST);
        assert_eq!(cx.implementation(), 1);
        assert!(std::ptr::eq(
            cx.decl(),
            client().get(query::ITEM_REMAINING_USE_DURATION).unwrap()
        ));
        assert_eq!(cx.default_value(), V::Float(0.0));
        let old = QueryCx::new(
            client(),
            query::ITEM_REMAINING_USE_DURATION,
            MolangVersion::V1,
            &mut vm,
        )
        .unwrap();
        assert_eq!(old.implementation(), 0);
    }

    #[test]
    fn the_context_makes_errors_of_its_own_query() {
        let mut world = World { babies: &[] };
        let mut vm = MockVm::new(&mut world, Subjects::none(), vec![]);
        let cx = QueryCx::new(client(), query::IS_BABY, MolangVersion::LATEST, &mut vm).unwrap();
        let error = cx.error(format_args!("Error: {} is wrong for {}.", cx.name(), 3));
        assert_eq!(error.query(), query::IS_BABY);
        assert_eq!(error.message(), "Error: query.is_baby is wrong for 3.");
        assert!(
            std::ptr::eq(error.query().as_ptr(), cx.name().as_ptr()),
            "the catalogue's copy of the name"
        );
        assert_eq!(error.to_string(), "Error: query.is_baby is wrong for 3.");
    }

    #[test]
    fn the_context_debug_names_the_query_version_and_implementation() {
        let mut world = World { babies: &[] };
        let mut vm = MockVm::new(&mut world, Subjects::none(), vec![]);
        let cx = QueryCx::new(
            client(),
            query::CAPE_FLAP_AMOUNT,
            MolangVersion::LATEST,
            &mut vm,
        )
        .unwrap();
        let text = format!("{cx:?}");
        assert!(
            text.starts_with("QueryCx { query: \"query.cape_flap_amount\""),
            "{text}"
        );
        assert!(text.contains("implementation: 1"), "{text}");
        assert!(text.contains("version: V13"), "{text}");
        assert!(text.ends_with(".. }"), "{text}");
    }

    #[test]
    fn a_context_exists_only_for_a_declared_query_at_a_served_version() {
        let mut world = World { babies: &[] };
        let mut vm = MockVm::new(&mut world, Subjects::none(), vec![]);
        assert!(QueryCx::new(client(), "query.no_such", MolangVersion::LATEST, &mut vm).is_none());
        assert!(
            QueryCx::new(
                client(),
                query::BLOCK_PROPERTY,
                MolangVersion::LATEST,
                &mut vm
            )
            .is_none()
        );
        assert!(QueryCx::new(client(), query::IS_BABY, MolangVersion::Invalid, &mut vm).is_none());
    }

    #[test]
    fn registration_is_by_name_and_fails_for_an_unknown_one() {
        let mut table = QueryTable::<TestHost>::new(client());
        let unknown = UnknownQuery::undeclared;
        assert_eq!(
            table.set("query.no_such", is_baby).err(),
            Some(unknown("query.no_such"))
        );
        assert_eq!(table.unset("q.is_baby").err(), Some(unknown("q.is_baby")));
        assert!(table.get("query.no_such").is_err() && table.is_stub("query.no_such").is_err());
        assert_eq!(table.implemented(), 0);
        assert_eq!(
            unknown("query.x").to_string(),
            "query.x is not declared in the query table's catalogue"
        );
        assert!(table.catalog().same(client()));
    }

    #[test]
    fn calls_across_catalogues_go_by_name_and_default_to_the_callers_declaration() {
        let own = client()
            .extended([decl(
                "query.my_thing",
                ReturnType::FLOAT,
                DefaultReturn::Float1,
            )])
            .unwrap();
        let mut world = World { babies: &[] };
        let mut vm = MockVm::new(
            &mut world,
            Subjects::none(),
            vec![V::Float(2.0), V::Float(3.0)],
        );
        let table = table_of(&own, &[("query.my_thing", sum), (query::IS_BABY, sum)]);
        let mut cx = QueryCx::new(&own, "query.my_thing", MolangVersion::LATEST, &mut vm).unwrap();
        assert_eq!(table.call(&mut cx), V::Float(5.0));
        let client_table = table_of(client(), &[(query::IS_BABY, sum)]);
        let mut cx = QueryCx::new(&own, query::IS_BABY, MolangVersion::LATEST, &mut vm).unwrap();
        assert_eq!(client_table.call(&mut cx), V::Float(5.0));
        let mut cx = QueryCx::new(&own, "query.my_thing", MolangVersion::LATEST, &mut vm).unwrap();
        assert_eq!(client_table.call(&mut cx), V::Float(1.0));
        let mut cx =
            QueryCx::new(client(), query::IS_BABY, MolangVersion::LATEST, &mut vm).unwrap();
        assert_eq!(table.call(&mut cx), V::Float(5.0));
    }

    /// Encodes its context: implementation in the units, version in the tens, 1000 for a
    /// one-range declaration.
    fn reports_its_context(cx: &mut QueryCx<'_, '_, TestHost>) -> QueryResult<TestHost> {
        let one_range = cx.decl().shape().ranges.as_slice().len() == 1;
        Ok(V::Float(
            f32::from(cx.implementation())
                + 10.0 * f32::from(cx.version().as_i16())
                + if one_range { 1000.0 } else { 0.0 },
        ))
    }

    fn v(raw: i16) -> MolangVersion {
        MolangVersion::from_i16(raw).unwrap()
    }

    fn catalogue_of(ranges: &[(i16, i16)], default: DefaultReturn) -> QueryCatalog {
        let ranges = ranges.iter().map(|&(first, last)| {
            VersionRange::new(v(first), v(last), QuerySetMask::DEFAULT).unwrap()
        });
        let shape = QueryShape {
            ranges: VersionRanges::new(ranges).unwrap(),
            default_return: default,
            ..QueryShape::DEFAULT
        };
        QueryCatalog::new(Side::Client, [QueryDecl::new("query.x", shape).unwrap()]).unwrap()
    }

    #[test]
    fn a_call_from_another_catalogue_gets_the_tables_own_declaration() {
        let expr_catalog = catalogue_of(&[(0, 7), (8, 13)], DefaultReturn::Float1);
        let table_catalog = catalogue_of(&[(0, 13)], DefaultReturn::Float0);
        let table = table_of(&table_catalog, &[("query.x", reports_its_context)]);
        let mut world = World { babies: &[] };
        let mut vm = MockVm::new(&mut world, Subjects::none(), vec![]);
        let mut cx =
            QueryCx::new(&expr_catalog, "query.x", MolangVersion::LATEST, &mut vm).unwrap();
        assert_eq!(
            cx.implementation(),
            1,
            "the expression's declaration serves 13 with its second range"
        );
        assert_eq!(
            table.call(&mut cx),
            V::Float(1130.0),
            "the table's one range: index 0"
        );
        let mut cx = QueryCx::new(&expr_catalog, "query.x", v(3), &mut vm).unwrap();
        assert_eq!(table.call(&mut cx), V::Float(1030.0));
        let own = table_of(&expr_catalog, &[("query.x", reports_its_context)]);
        let mut cx =
            QueryCx::new(&expr_catalog, "query.x", MolangVersion::LATEST, &mut vm).unwrap();
        assert_eq!(own.call(&mut cx), V::Float(131.0));
    }

    #[test]
    fn a_version_the_tables_declaration_does_not_serve_runs_nothing() {
        let expr_catalog = catalogue_of(&[(0, 13)], DefaultReturn::Float1);
        let table_catalog = catalogue_of(&[(0, 3)], DefaultReturn::FloatNeg1);
        let table = table_of(&table_catalog, &[("query.x", reports_its_context)]);
        let mut world = World { babies: &[] };
        let mut vm = MockVm::new(&mut world, Subjects::none(), vec![]);
        let mut cx =
            QueryCx::new(&expr_catalog, "query.x", MolangVersion::LATEST, &mut vm).unwrap();
        assert_eq!(
            table.call(&mut cx),
            V::Float(1.0),
            "the expression's default, not the table's"
        );
        let mut cx = QueryCx::new(&expr_catalog, "query.x", v(3), &mut vm).unwrap();
        assert_eq!(
            table.call(&mut cx),
            V::Float(1030.0),
            "a version both serve runs it"
        );
        assert!(vm.sink.is_empty());
    }

    #[test]
    fn a_return_type_the_expressions_declaration_does_not_allow_runs_nothing() {
        let declare = |returns: ReturnType, default: DefaultReturn| {
            QueryCatalog::new(Side::Client, [decl("query.x", returns, default)]).unwrap()
        };
        let mut world = World { babies: &[] };
        let mut vm = MockVm::new(&mut world, Subjects::none(), vec![]);
        let expr_catalog = declare(ReturnType::FLOAT, DefaultReturn::Float1);
        let strings = declare(ReturnType::STRING, DefaultReturn::EmptyString);
        let table = table_of(&strings, &[("query.x", reports_its_context)]);
        let mut cx =
            QueryCx::new(&expr_catalog, "query.x", MolangVersion::LATEST, &mut vm).unwrap();
        assert_eq!(
            table.call(&mut cx),
            V::Float(1.0),
            "the expression's default"
        );
        let numbers = declare(ReturnType::NUMBER, DefaultReturn::Float0);
        let table = table_of(&numbers, &[("query.x", reports_its_context)]);
        let mut cx =
            QueryCx::new(&expr_catalog, "query.x", MolangVersion::LATEST, &mut vm).unwrap();
        assert_eq!(
            table.call(&mut cx),
            V::Float(1.0),
            "a bool the expression does not expect"
        );
        let wide = declare(ReturnType::NUMBER, DefaultReturn::Float0);
        let narrow = declare(ReturnType::FLOAT, DefaultReturn::Float0);
        let table = table_of(&narrow, &[("query.x", reports_its_context)]);
        let mut cx = QueryCx::new(&wide, "query.x", MolangVersion::LATEST, &mut vm).unwrap();
        assert_eq!(
            table.call(&mut cx),
            V::Float(1130.0),
            "a float where a number is expected runs"
        );
        assert!(vm.sink.is_empty());
    }

    #[test]
    fn an_error_across_catalogues_yields_the_tables_default() {
        let expr_catalog = catalogue_of(&[(0, 13)], DefaultReturn::Float1);
        let table_catalog = catalogue_of(&[(0, 13)], DefaultReturn::FloatNeg1);
        let table = table_of(&table_catalog, &[("query.x", needs_item)]);
        let mut world = World { babies: &[] };
        let mut vm = MockVm::new(&mut world, Subjects::none(), vec![]);
        let mut cx =
            QueryCx::new(&expr_catalog, "query.x", MolangVersion::LATEST, &mut vm).unwrap();
        assert_eq!(table.call(&mut cx), V::Float(-1.0));
        assert_eq!(vm.sink.messages, ["query.x has no item"]);
    }
}
