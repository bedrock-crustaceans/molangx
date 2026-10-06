//! The crate as an external user sees it: the types layer that needs no feature, and a smoke test
//! per feature.

#![cfg(feature = "stdlib")]

use molangx::catalog::{
    Arity, ParseQuerySetError, QueryAdmission, QueryDecl, QuerySetMask, ReturnType, Side,
};
use molangx::hash::HashedStr;
use molangx::json::{MolangSource, MolangValueRepr};
use molangx::stdlib::query;
use molangx::version::{EngineVersion, ExperimentMask, MolangVersion, VERSION_THRESHOLDS};

fn ev(major: u64, minor: u64, patch: u64) -> EngineVersion {
    EngineVersion::Version(molangx::version::semver::Version::new(major, minor, patch))
}

fn ev_pre(major: u64, minor: u64, patch: u64) -> EngineVersion {
    EngineVersion::Version(molangx::version::semver::Version {
        pre: molangx::version::semver::Prerelease::new("pre").unwrap(),
        ..molangx::version::semver::Version::new(major, minor, patch)
    })
}

fn parse(text: &str) -> Option<EngineVersion> {
    text.parse().ok()
}

mod types_layer {
    use super::*;

    #[test]
    fn always_built_layer() {
        assert_eq!(VERSION_THRESHOLDS.len(), 13);
        let source = MolangSource::object("q.is_baby", 9);
        assert_eq!(source.raw_version(), Some(molangx::version::RawVersion(9)));
        let field = MolangValueRepr::read(
            molangx::json::JsonScalar::Number(1.5),
            molangx::json::ReaderKind::ScalarOrArray,
            13,
        );
        let Ok(molangx::json::ReadField::Value(repr)) = field else {
            panic!("a number reads as one value: {field:?}");
        };
        assert_eq!(repr.number(), Some(1.5));
        assert_eq!(HashedStr::new("").as_u64(), 0);
        assert!(!molangx::stdlib::queries(Side::Client).is_empty());
        assert_eq!(
            molangx::stdlib::queries(Side::Server)
                .get(query::IS_BABY)
                .map(QueryDecl::name),
            Some("query.is_baby")
        );
    }

    /// Versions 3 and 4 share 1.17.40.
    #[test]
    fn the_version_map_rows() {
        assert_eq!(VERSION_THRESHOLDS[0].version, MolangVersion::V1);
        assert_eq!(VERSION_THRESHOLDS[0].engine_version, ev(1, 17, 0));
        assert_eq!(VERSION_THRESHOLDS[1].engine_version, ev(1, 17, 30));
        assert_eq!(VERSION_THRESHOLDS[2].version, MolangVersion::V3);
        assert_eq!(
            VERSION_THRESHOLDS[2].engine_version,
            VERSION_THRESHOLDS[3].engine_version
        );
        assert_eq!(VERSION_THRESHOLDS[3].version, MolangVersion::V4);
        assert_eq!(VERSION_THRESHOLDS[12].version, MolangVersion::LATEST);
        assert_eq!(VERSION_THRESHOLDS[12].engine_version, ev(1, 21, 100));
        for (row, entry) in VERSION_THRESHOLDS.iter().enumerate() {
            assert_eq!(usize::try_from(entry.version.as_i16()).unwrap(), row + 1);
        }
    }

    /// Version 3 is never the answer, because 4 starts at the same engine version.
    #[test]
    fn an_engine_version_maps_to_the_newest_version_it_reaches() {
        let at = |major, minor, patch| MolangVersion::from(&ev(major, minor, patch));
        assert_eq!(at(1, 16, 100), MolangVersion::V0);
        assert_eq!(at(1, 17, 29), MolangVersion::V1);
        assert_eq!(at(1, 17, 30), MolangVersion::V2);
        assert_eq!(at(1, 17, 40), MolangVersion::V4);
        assert_eq!(at(1, 18, 10), MolangVersion::V5);
        assert_eq!(at(1, 21, 99), MolangVersion::V12);
        assert_eq!(at(1, 21, 100), MolangVersion::LATEST);
        assert_eq!(at(2, 0, 0), MolangVersion::LATEST);
        assert_eq!(
            parse("1.20.50")
                .as_ref()
                .map_or(MolangVersion::Invalid, MolangVersion::from),
            MolangVersion::V11
        );
        assert_eq!(
            parse("*")
                .as_ref()
                .map_or(MolangVersion::Invalid, MolangVersion::from),
            MolangVersion::LATEST
        );
    }

    #[test]
    fn the_gates_of_the_extreme_versions() {
        assert!(!MolangVersion::Invalid.reports_expression_errors());
        assert!(!MolangVersion::Invalid.signed_division_fix());
        assert!(MolangVersion::LATEST.reports_expression_errors());
        assert!(MolangVersion::LATEST.signed_division_fix());
        assert_eq!(
            molangx::version::RawVersion(200).effective(),
            MolangVersion::LATEST
        );
        assert_eq!(
            molangx::version::RawVersion(-200).effective(),
            MolangVersion::Invalid
        );
    }

    /// FNV-1 of the bytes as written; the empty string hashes to 0.
    #[test]
    fn hashing() {
        assert_eq!(HashedStr::new("").as_u64(), 0);
        assert_eq!(HashedStr::EMPTY, HashedStr::new(""));
        assert!(HashedStr::EMPTY.is_empty());
        assert_eq!(HashedStr::new("a").as_u64(), 12_638_153_115_695_167_422);
        assert_eq!(HashedStr::new("abc").as_u64(), 15_626_587_013_303_479_755);
        assert_eq!(
            HashedStr::new("ABC").as_u64(),
            15_595_941_425_208_037_995,
            "no case folding"
        );
        assert_eq!(HashedStr::new(" ").as_u64(), 12_638_153_115_695_167_487);
        assert_eq!(HashedStr::from("abc"), HashedStr::new("abc"));
        assert_eq!(HashedStr::from_u64(42).as_u64(), 42);
        assert_eq!(u64::from(HashedStr::new("a")), 12_638_153_115_695_167_422);
        assert!(!HashedStr::new("a").is_empty());
    }

    #[test]
    fn hashing_stops_at_a_nul_byte() {
        assert_eq!(HashedStr::from_bytes(b"a\0b"), HashedStr::new("a"));
        assert_eq!(HashedStr::from_bytes(b"\0a"), HashedStr::EMPTY);
        assert_eq!(
            molangx::hash::fnv1_64(b"abc"),
            HashedStr::new("abc").as_u64()
        );
        const A: HashedStr = HashedStr::new("a");
        assert_eq!(A.as_u64(), 12_638_153_115_695_167_422);
    }

    /// Only by the full name or the suffix after `query.`.
    #[test]
    fn query_lookup() {
        let catalog = molangx::stdlib::queries(Side::Client);
        let decl = catalog.get("query.is_baby").expect("a standard query");
        assert_eq!(query::IS_BABY, "query.is_baby");
        assert_eq!((decl.name(), decl.suffix()), ("query.is_baby", "is_baby"));
        assert_eq!(catalog.get_suffix("is_baby"), Some(decl));
        assert!(catalog.get("q.is_baby").is_none());
        assert!(catalog.get("QUERY.IS_BABY").is_none());
        assert!(catalog.get("query.no_such_query").is_none());
        assert_eq!(catalog.iter().len(), catalog.len());
        assert_eq!(decl.shape().returns, ReturnType::BOOL);
        assert_eq!(decl.args(), Arity::ANY);
    }

    /// The window is inclusive; `Invalid` and raw versions outside 0..=13 resolve no standard
    /// query.
    #[test]
    fn query_resolution_by_version() {
        let catalog = molangx::stdlib::queries(Side::Client);
        let default = QueryAdmission::Sets(QuerySetMask::DEFAULT);
        let resolve = |name: &str, raw| {
            let decl = catalog.get(name)?;
            let range = decl.shape().ranges.as_slice()[usize::from(decl.resolve(
                molangx::version::RawVersion(raw),
                &default,
                ExperimentMask::empty(),
            )?)];
            Some((range.first().as_i16(), range.last().as_i16()))
        };
        assert_eq!(resolve("query.is_baby", 0), Some((0, 13)));
        assert_eq!(resolve("query.is_baby", 13), Some((0, 13)));
        for raw in [-1, -7, 14, 200] {
            assert_eq!(resolve("query.is_baby", raw), None, "raw version {raw}");
        }
        // `query.block_property` ends at version 9; `query.block_state` has no end.
        assert_eq!(resolve("query.block_property", 9), Some((0, 9)));
        assert_eq!(resolve("query.block_property", 10), None);
        assert_eq!(resolve("query.block_state", 13), Some((0, 13)));
        // One query may have two version windows; `implementation_at` tells them apart.
        assert_eq!(resolve("query.cape_flap_amount", 7), Some((0, 7)));
        assert_eq!(resolve("query.cape_flap_amount", 8), Some((8, 13)));
        assert_eq!(
            catalog
                .get(query::CAPE_FLAP_AMOUNT)
                .and_then(|d| d.implementation_at(MolangVersion::LATEST)),
            Some(1)
        );
    }

    /// A list of names replaces the sets.
    #[test]
    fn query_resolution_by_set_and_allow_list() {
        let catalog = molangx::stdlib::queries(Side::Client);
        let resolves = |name: &str, admission: &QueryAdmission| {
            catalog
                .get(name)
                .and_then(|d| {
                    d.resolve(
                        molangx::version::RawVersion(13),
                        admission,
                        ExperimentMask::empty(),
                    )
                })
                .is_some()
        };
        let by_sets = |name, sets| resolves(name, &QueryAdmission::Sets(sets));
        assert!(by_sets("query.is_baby", QuerySetMask::DEFAULT));
        assert!(!by_sets("query.is_baby", QuerySetMask::empty()));
        assert!(!by_sets("query.is_baby", QuerySetMask::WORLD_GEN));
        assert!(!by_sets("query.noise", QuerySetMask::DEFAULT));
        assert!(by_sets("query.noise", QuerySetMask::WORLD_GEN));
        assert!(!by_sets("query.any_tag", QuerySetMask::DEFAULT));
        assert!(by_sets("query.any_tag", QuerySetMask::TAGS));
        assert!(by_sets("query.noise", QuerySetMask::BUILTIN));
        // The list ignores the sets and admits only what it names.
        let list =
            molangx::catalog::QueryAllowList::new(catalog, [query::BLOCK_STATE]).expect("declared");
        let allowed = QueryAdmission::Only(list);
        assert!(resolves("query.block_state", &allowed));
        assert!(!resolves("query.is_baby", &allowed));
        assert_eq!(
            "world_gen".parse::<QuerySetMask>(),
            Ok(QuerySetMask::WORLD_GEN)
        );
        assert_eq!(
            "no_such_set".parse::<QuerySetMask>(),
            Err(ParseQuerySetError)
        );
    }

    #[test]
    fn the_root_re_exports_the_types_layer() {
        let _: molangx::hash::HashedStr = HashedStr::EMPTY;
        let _: molangx::version::MolangVersion = MolangVersion::LATEST;
        let _: molangx::version::EngineVersion = EngineVersion::Any;
        let _: &molangx::catalog::QueryCatalog = molangx::stdlib::queries(Side::Client);
        let _: molangx::catalog::QueryAdmission =
            molangx::catalog::QueryAdmission::Sets(QuerySetMask::DEFAULT);
        let _: Option<&molangx::catalog::QueryDecl> = None::<&molangx::catalog::QueryDecl>;
        let _: molangx::catalog::Side = Side::Server;
        let _: molangx::catalog::QuerySetMask = QuerySetMask::DEFAULT;
        let _: molangx::version::ExperimentMask = ExperimentMask::empty();
        let _: molangx::ops::ExpressionOp =
            molangx::ops::ExpressionOp::from_ordinal(0).expect("the first op");
        let _: molangx::ops::OpSet = molangx::ops::OpSet::all();
        let _: molangx::json::MolangSource = MolangSource::string("1", 0);
        let _: molangx::json::ReaderKind = molangx::json::ReaderKind::SchemaValidated;
        let _: molangx::json::ReprError =
            molangx::json::ReprError::NumberOutOfRange { value: 1e39 };
        let _: molangx::json::ReadField = molangx::json::ReadField::NoExpression;
        assert_eq!(molangx::ops::ExpressionOp::COUNT, 111);
        assert_eq!(molangx::ops::OpSet::all().len(), 111);
        assert_eq!(molangx::stdlib::MathFn::all().len(), 61);
    }

    /// Whichever 64-bit word the op is in.
    #[test]
    fn an_op_set_with_one_op_is_never_empty() {
        for op in molangx::ops::ExpressionOp::all() {
            let single = molangx::ops::OpSet::empty().with(*op);
            assert!(!single.is_empty(), "{op:?}");
            assert_eq!(single.len(), 1, "{op:?}");
            assert!(single.without(*op).is_empty(), "{op:?}");
        }
        let second_word_only = molangx::ops::OpSet::from_words([0, u64::MAX]);
        assert!(!second_word_only.is_empty());
        assert_eq!(second_word_only.len(), 47);
    }
}

#[cfg(feature = "compiler")]
mod compiler {
    use std::sync::Arc;

    use molangx::compile::{CompileFailure, CompileOptions, Compiled, Expr, compile};
    use molangx::json::ReprError;

    use super::*;

    /// The error types are `std::error::Error`s; `ReprError` is `Copy`, so it holds no `String`.
    #[test]
    fn compile_returns_no_error() {
        fn is_error<E: std::error::Error + Clone>() {}
        fn is_copy<T: Copy>() {}
        let compile_fn: fn(&str, &CompileOptions) -> Compiled = compile;
        let compiled = compile_fn("1 +", &CompileOptions::server(MolangVersion::LATEST));
        assert_eq!(compiled.failure(), Some(CompileFailure::Rejected));
        is_error::<ReprError>();
        is_copy::<ReprError>();
    }

    /// The options own their catalogues: a host stores them, sends them and clones them.
    #[test]
    fn options_are_owned_values() {
        use molangx::catalog::{Arity, MathCatalog, MathDecl, QueryAllowList};
        const fn assert_owned<T: Send + Sync + Clone + Eq + std::hash::Hash + 'static>() {}
        const { assert_owned::<CompileOptions>() };
        struct Host {
            options: CompileOptions,
        }
        let math =
            MathCatalog::new([
                MathDecl::pure("math.twice", Arity::exactly(1), |a| a[0] * 2.0)
                    .expect("a declaration"),
            ])
            .expect("a catalogue");
        let list = QueryAllowList::new(molangx::stdlib::queries(Side::Server), [query::IS_BABY])
            .expect("a standard query");
        let host = Host {
            options: CompileOptions {
                admission: QueryAdmission::Only(list),
                math: Some(math.clone()),
                ..CompileOptions::server(MolangVersion::LATEST)
            },
        };
        let sent = std::thread::spawn(move || host.options)
            .join()
            .expect("the thread finishes");
        assert_eq!(sent.clone(), sent);
        assert_eq!(compile("math.twice(q.is_baby)", &sent).failure(), None);
        assert_eq!(sent.math, Some(math));
    }

    #[test]
    fn compiled_forms_are_send_and_sync() {
        const fn assert_send_sync<T: Send + Sync>() {}
        const {
            assert_send_sync::<Expr>();
            assert_send_sync::<Compiled>();
            assert_send_sync::<std::sync::Arc<Compiled>>();
        }
    }

    /// A malformed expression is rejected with a diagnostic and the constant 0.
    #[test]
    fn compile_smoke() {
        let options = CompileOptions::server(MolangVersion::LATEST);
        let folded = compile("1 + 2 * 3", &options);
        assert_eq!(folded.failure(), None);
        assert_eq!(folded.expr().and_then(Expr::as_constant), Some(7.0));
        assert!(folded.parses_cleanly());
        let program = compile("v.x * 2", &options);
        assert_eq!(program.failure(), None);
        assert_eq!(program.expr_or_zero().and_then(Expr::as_constant), None);
        let rejected = compile("1 +", &options);
        assert_eq!(rejected.failure(), Some(CompileFailure::Rejected));
        assert_eq!(
            rejected.diagnostics()[0].message(),
            "Error: binary Add '+' operator at end of expression\n"
        );
        assert_eq!(
            rejected.expr_or_zero().and_then(Expr::as_constant),
            Some(0.0)
        );
        let shared: Arc<Compiled> = Arc::new(folded);
        assert_eq!(
            shared.expr().map(Expr::version),
            Some(MolangVersion::LATEST)
        );
    }

    /// The options and their parts are plain data, set with struct-update syntax.
    #[test]
    fn options_are_set_by_struct_update() {
        use molangx::catalog::QueryAllowList;
        use molangx::compile::Deviations;
        use molangx::ops::OpSet;
        use molangx::version::RawVersion;
        let list =
            QueryAllowList::new(molangx::stdlib::queries(Side::Server), [query::BLOCK_STATE])
                .expect("a standard query");
        let block = CompileOptions {
            admission: QueryAdmission::Only(list),
            allowed_ops: OpSet::all().without_assignments_or_random(),
            deviations: Deviations {
                validate_nested: false,
                ..Deviations::DEFAULT
            },
            ..CompileOptions::server(MolangVersion::LATEST)
        };
        assert_eq!(
            compile("q.block_state('facing') == 'west'", &block).failure(),
            None
        );
        assert_eq!(
            compile("q.is_baby", &block).failure(),
            Some(CompileFailure::Rejected)
        );
        assert_eq!(
            compile("v.x = 1", &block).failure(),
            Some(CompileFailure::Rejected)
        );
        let raw = CompileOptions {
            raw_version: RawVersion(99),
            ..block
        };
        assert_eq!(
            (raw.version(), raw.raw_version),
            (MolangVersion::LATEST, RawVersion(99))
        );
    }

    /// Host math functions are declared as values and passed in the options.
    #[test]
    fn host_math_functions_are_plain_data() {
        use molangx::catalog::{
            Arity, MAX_MATH_ARGS, MathCatalog, MathDecl, MathError, MathImpl, PureMathFn,
            VolatileMathFn,
        };
        use molangx::rng::{rand_core::Rng, sample};
        type Pure = fn(&[f32]) -> f32;
        type Volatile = fn(&mut dyn Rng, &[f32]) -> f32;
        type Declared = Result<MathDecl, MathError>;
        let pure: fn(&str, Arity, Pure) -> Declared = |name, args, f| MathDecl::pure(name, args, f);
        let volatile: fn(&str, Arity, Volatile) -> Declared =
            |name, args, f| MathDecl::volatile(name, args, f);
        let twice = pure("math.twice", Arity::exactly(1), |a| a[0] * 2.0).expect("a declaration");
        let draw = volatile("math.draw", Arity::between(1, MAX_MATH_ARGS), |rng, a| {
            a[0] + sample(rng)
        })
        .expect("a declaration");
        let (_, _): (Option<&PureMathFn>, Option<&VolatileMathFn>) = (None, None);
        assert!(matches!(twice.implementation(), MathImpl::Pure(_)) && draw.is_volatile());
        let math: MathCatalog = MathCatalog::new([twice, draw]).expect("a catalogue");
        assert_eq!(
            (math.len(), math.get("math.twice").map(MathDecl::args)),
            (2, Some(Arity::exactly(1)))
        );
        let empty = MathCatalog::new([]).expect("an empty catalogue");
        assert!(empty.is_empty() && !math.is_empty());
        assert_eq!(empty.len(), 0);
        let options = CompileOptions {
            math: Some(math.clone()),
            ..CompileOptions::server(MolangVersion::LATEST)
        };
        let compiled = compile("math.twice(21)", &options);
        assert_eq!(compiled.expr().and_then(Expr::as_constant), Some(42.0));
        assert_eq!(compiled.expr().and_then(Expr::math), Some(&math));
        assert_eq!(CompileOptions::server(MolangVersion::LATEST).math, None);
    }

    #[test]
    fn compile_source_smoke() {
        let source = MolangSource::string("'a' + 1", 2);
        let compiled = molangx::compile::compile_source(
            &source,
            &CompileOptions::server(MolangVersion::LATEST),
        );
        assert_eq!(
            compiled.failure(),
            None,
            "string arithmetic is accepted before version 3"
        );
        let source = MolangSource::string("'a' + 1", 3);
        assert_eq!(
            molangx::compile::compile_source(
                &source,
                &CompileOptions::server(MolangVersion::LATEST)
            )
            .failure(),
            Some(CompileFailure::Rejected)
        );
    }
}

#[cfg(feature = "vm")]
mod vm {
    use std::cell::Cell;

    use molangx::catalog::{QueryAdmission, QuerySetMask, Side};
    use molangx::compile::{CompileOptions, compile};
    use molangx::rng::{Xorshift128, rand_core::Rng};
    use molangx::stdlib::query;
    use molangx::version::MolangVersion;
    use molangx::vm::{
        CollectSink, ContextMap, ContextName, ContextProvider, EvalCx, EvalLimits, Host,
        HostAccess, HostEnv, NoHost, NoHostEnv, QueryCx, QueryError, QueryTable, Subjects, TempMap,
        Temps, Value, VariableName, VariableStore,
    };

    /// The value maps build from arrays and iterators, and their `set` takes anything that converts
    /// to a `Value` and returns the value it replaces.
    #[test]
    fn value_maps_build_from_entries_and_set_returns_the_old_value() {
        use molangx::hash::HashedStr;
        use molangx::vm::{StructValue, TempName, VariableMap};
        let x = VariableName::new("x");
        let mut variables = VariableMap::<NoHost>::from([(x, 1.0), (VariableName::new("y"), 2.0)]);
        assert_eq!(variables.set(x, 3.0), Some(Value::Float(1.0)));
        assert_eq!(variables.set(VariableName::new("z"), true), None);
        assert_eq!(variables.len(), 3);
        let temps: TempMap<NoHost> = [(TempName::new("t"), HashedStr::new("a"))]
            .into_iter()
            .collect();
        assert_eq!(
            temps.get(TempName::new("t")),
            Some(&Value::Hash(HashedStr::new("a")))
        );
        let mut context = ContextMap::<NoHost>::new();
        assert_eq!(context.set(ContextName::new("c"), 1.0), None);
        assert_eq!(
            context.set(ContextName::new("c"), 2.0),
            Some(Value::Float(1.0))
        );
        let mut s = StructValue::<NoHost>::xy(1.0, 2.0);
        assert_eq!(
            s.set(StructValue::<NoHost>::key("x"), 5.0),
            Some(Value::Float(1.0))
        );
        assert_eq!(s.set(StructValue::<NoHost>::key("z"), 6.0), None);
    }

    #[test]
    fn eval_smoke() {
        let options = CompileOptions::server(MolangVersion::LATEST);
        let mut env = NoHostEnv::new();
        let expr = compile("v.x = 3; v.y = v.x * 2 + 1; return v.y;", &options)
            .expr()
            .cloned()
            .expect("expr");
        assert_eq!(expr.eval_f32(&mut env.cx()), 7.0);
        assert_eq!(
            env.variables.get(VariableName::new("y")),
            Some(&Value::Float(7.0))
        );
        assert_eq!(
            compile("1 + 2", &options)
                .expr()
                .cloned()
                .expect("expr")
                .eval(&mut env.cx()),
            Value::Float(3.0)
        );
        assert!(env.sink.is_empty());
    }

    /// `==` takes the right operand's kind, so a string equals the float of its own low bits only
    /// with the string on the left.
    #[test]
    fn as_f32_and_mixed_equality() {
        let low = f32::from_bits(molangx::hash::HashedStr::new("a").as_u64() as u32);
        assert_eq!(
            Value::<NoHost>::string("a").as_f32().to_bits(),
            low.to_bits()
        );
        assert_eq!(Value::<NoHost>::Actor(()).as_f32(), 0.0);
        let eval = |source: &str| {
            compile(source, &CompileOptions::server(MolangVersion::LATEST))
                .expr()
                .cloned()
                .expect("expr")
                .eval_f32(&mut NoHostEnv::new().cx())
        };
        assert_eq!(
            eval("v.one = 1; v.s = 'a'; v.f = v.s * v.one; return v.s == v.f;"),
            1.0
        );
        assert_eq!(
            eval("v.one = 1; v.s = 'a'; v.f = v.s * v.one; return v.f == v.s;"),
            0.0
        );
        assert_eq!(eval("v.s = 'a'; return v.s == 1;"), 0.0);
    }

    /// A host whose world counts the calls the evaluator makes.
    #[derive(Debug)]
    struct Farm;

    impl Host for Farm {
        type ActorRef = u32;
        type ItemRef = u8;
        type BlockRef = (i32, i32, i32);
        type Access<'w> = World;
    }

    struct World {
        resolved: Cell<u32>,
        subjects_of: Cell<u32>,
    }

    impl HostAccess<Farm> for World {
        fn resolve_actor(&self, _from: &Subjects<Farm>, actor: u32) -> Option<u32> {
            self.resolved.set(self.resolved.get() + 1);
            (actor < 10).then_some(actor)
        }

        fn subjects_of(&self, actor: u32) -> Subjects<Farm> {
            self.subjects_of.set(self.subjects_of.get() + 1);
            Subjects::actor(actor)
        }
    }

    fn actor_number(cx: &mut QueryCx<'_, '_, Farm>) -> Result<Value<Farm>, QueryError> {
        Ok(Value::Float(cx.subjects().actor.map_or(-1.0, |a| a as f32)))
    }

    /// An unresolvable target is 0 and never entered.
    #[test]
    fn arrow_goes_through_host_access() {
        let mut queries = QueryTable::<Farm>::new(molangx::stdlib::queries(Side::Server));
        queries.set(query::IS_BABY, actor_number).unwrap();
        let mut context = ContextMap::<Farm>::new();
        context.set(ContextName::new("other"), Value::Actor(7));
        context.set(ContextName::new("gone"), Value::Actor(70));
        let mut env = HostEnv::new(queries).with_context(context);
        let mut world = World {
            resolved: Cell::new(0),
            subjects_of: Cell::new(0),
        };
        let options = CompileOptions::server(MolangVersion::LATEST);
        let arrow = compile("c.other->q.is_baby", &options)
            .expr()
            .cloned()
            .expect("expr");
        assert_eq!(
            arrow.eval_f32(&mut env.cx(&mut world, Subjects::actor(1))),
            7.0
        );
        assert!(world.resolved.get() >= 1 && world.subjects_of.get() == 1);
        let gone = compile("c.gone->q.is_baby", &options)
            .expr()
            .cloned()
            .expect("expr");
        assert_eq!(
            gone.eval_f32(&mut env.cx(&mut world, Subjects::actor(1))),
            0.0
        );
        assert_eq!(
            world.subjects_of.get(),
            1,
            "an unresolvable target is never entered"
        );
    }

    #[test]
    fn eval_cx_bundles_the_evaluation() {
        let mut world = World {
            resolved: Cell::new(0),
            subjects_of: Cell::new(0),
        };
        let mut temps = TempMap::<Farm>::new();
        let mut store = molangx::vm::VariableStorage::<Farm>::new();
        let context = ContextMap::<Farm>::new();
        let mut queries = QueryTable::<Farm>::new(molangx::stdlib::queries(Side::Server));
        queries.set(query::IS_BABY, actor_number).unwrap();
        let mut rng = Xorshift128::new();
        let mut sink = CollectSink::new();
        let mut cx = EvalCx {
            subjects: Subjects::actor(3),
            host: &mut world,
            variables: &mut store as &mut dyn VariableStore<Farm>,
            context: &context as &dyn ContextProvider<Farm>,
            queries: Some(&queries),
            rng: &mut rng as &mut dyn Rng,
            sink: &mut sink,
            limits: EvalLimits {
                loop_iterations: Some(4),
                ..EvalLimits::DEFAULT
            },
            temps: Temps::Kept(&mut temps),
        };
        let expr = compile(
            "t.n = 0; loop(100, { t.n = t.n + 1; }); return t.n * 10 + q.is_baby;",
            &CompileOptions::server(MolangVersion::LATEST),
        )
        .expr()
        .cloned()
        .expect("expr");
        assert_eq!(
            expr.eval_f32(&mut cx),
            43.0,
            "the loop budget of the context applies"
        );
        assert_eq!(sink.messages.len(), 1, "the loop guard reported once");
    }

    /// An unhandled missing read ends the evaluation with 0.0; a `??` handler catches it.
    #[test]
    fn missing_reads_and_handlers() {
        let options = CompileOptions::server(MolangVersion::LATEST);
        let mut env = NoHostEnv::new();
        let missing = compile("v.before = 1; return v.missing + 5;", &options)
            .expr()
            .cloned()
            .expect("expr");
        assert_eq!(missing.eval(&mut env.cx()), Value::Float(0.0));
        let handled = compile("return (v.missing ?? 2) + 5;", &options)
            .expr()
            .cloned()
            .expect("expr");
        assert_eq!(handled.eval_f32(&mut env.cx()), 7.0);
    }

    /// Names are typed keys of the full canonical name, whatever the spelling.
    #[cfg(feature = "vm")]
    #[test]
    fn names_are_canonical_and_member_paths_split() {
        use molangx::hash::HashedStr;
        use molangx::vm::{NoHostEnv, Value, VariableName};
        let options = CompileOptions::server(MolangVersion::LATEST);
        let mut env = NoHostEnv::new();
        let write = compile("V.A.b = 3; variable.c = 4;", &options)
            .expr()
            .cloned()
            .expect("expr");
        write.eval(&mut env.cx());
        let base = env
            .variables
            .get(VariableName::new("a"))
            .expect("the base variable `variable.a`");
        assert_eq!(base.member(HashedStr::new("b")), Some(&Value::Float(3.0)));
        assert_eq!(
            env.variables.get(VariableName::new("c")),
            Some(&Value::Float(4.0))
        );
        let parsed: VariableName = "V.C".parse().expect("a variable name");
        assert_eq!(parsed, VariableName::new("c"));
        assert_eq!(HashedStr::from(parsed), HashedStr::new("variable.c"));
        assert_eq!(parsed.hashed(), HashedStr::new("variable.c"));
        assert_eq!(
            "t.c".parse::<VariableName>(),
            Err(molangx::vm::ParseNameError)
        );
        let any: molangx::vm::AnyName = "t.c".parse().expect("a name");
        assert_eq!(HashedStr::from(any), HashedStr::new("temp.c"));
        let read = compile("return variable.a.b + v.C;", &options)
            .expr()
            .cloned()
            .expect("expr");
        assert_eq!(read.eval_f32(&mut env.cx()), 7.0);
    }

    /// A query is any `Fn` over the call context: a plain `fn` or a closure with captured state.
    #[test]
    fn queries_are_closures_and_fns_and_the_environment_is_plain_data() {
        use std::sync::Arc;
        use std::sync::atomic::{AtomicU32, Ordering};

        use molangx::vm::{
            NoHost, NoHostEnv, Query, QueryCx, QueryResult, QueryTable, StructValue, Temps, Value,
        };
        fn seven(_cx: &mut QueryCx<'_, '_, NoHost>) -> QueryResult<NoHost> {
            Ok(Value::Float(7.0))
        }
        let catalog = molangx::stdlib::queries(Side::Server);
        let calls = Arc::new(AtomicU32::new(0));
        let counter = Arc::clone(&calls);
        let mut queries = QueryTable::new(catalog);
        assert_eq!(queries.set(query::LIFE_TIME, seven), Ok(()));
        queries
            .set(query::ANGER_LEVEL, move |_| {
                Ok(Value::Float(counter.fetch_add(1, Ordering::Relaxed) as f32))
            })
            .expect("declared");
        assert!(queries.set("query.no_such", seven).is_err());
        let installed: Option<&Arc<dyn Query<NoHost>>> =
            queries.get(query::LIFE_TIME).expect("declared");
        assert!(installed.is_some());
        let mut env = NoHostEnv {
            queries: Some(queries),
            limits: EvalLimits {
                total_steps: Some(10_000),
                ..EvalLimits::DEFAULT
            },
            temps: Temps::Kept(TempMap::new()),
            context: ContextMap::from([(ContextName::new("bonus"), 1.0)]),
            ..NoHostEnv::new()
        };
        let expr = compile(
            "t.n = q.life_time + q.anger_level + q.anger_level + c.bonus; return t.n;",
            &CompileOptions::server(MolangVersion::LATEST),
        )
        .expr()
        .cloned()
        .expect("expr");
        assert_eq!(expr.eval_f32(&mut env.cx()), 9.0);
        assert_eq!(calls.load(Ordering::Relaxed), 2);
        assert_eq!(
            env.temps
                .kept()
                .and_then(|temps| temps.get(molangx::vm::TempName::new("n"))),
            Some(&Value::Float(9.0))
        );
        let s: StructValue<NoHost> = [("x", 1.0f32), ("y", 2.0)].into_iter().collect();
        assert_eq!(s, StructValue::xy(1.0, 2.0));
        assert_eq!(StructValue::<NoHost>::from([("x", 1.0), ("y", 2.0)]), s);
    }

    /// A host type implementing `Query`, installed once and shared by two tables.
    #[test]
    fn set_shared_installs_one_implementation_in_several_tables() {
        use std::sync::Arc;

        use molangx::vm::{NoHost, NoHostEnv, Query, QueryCx, QueryResult, QueryTable, Value};
        struct Scaled(f32);
        impl Query<NoHost> for Scaled {
            fn call(&self, cx: &mut QueryCx<'_, '_, NoHost>) -> QueryResult<NoHost> {
                Ok(Value::Float(cx.arg_f32(0).unwrap_or(0.0) * self.0))
            }
        }
        let shared: Arc<dyn Query<NoHost>> = Arc::new(Scaled(3.0));
        let catalog = molangx::stdlib::queries(Side::Server);
        let mut first = QueryTable::new(catalog);
        let mut second = QueryTable::new(catalog);
        first
            .set_shared(query::LIFE_TIME, Arc::clone(&shared))
            .expect("declared");
        second
            .set_shared(query::LIFE_TIME, Arc::clone(&shared))
            .expect("declared");
        assert!(
            first
                .set_shared("query.no_such", Arc::clone(&shared))
                .is_err()
        );
        assert!(Arc::ptr_eq(
            first.get(query::LIFE_TIME).unwrap().unwrap(),
            &shared
        ));
        assert_eq!(Arc::strong_count(&shared), 3);
        let expr = compile(
            "q.life_time(2)",
            &CompileOptions::server(MolangVersion::LATEST),
        )
        .expr()
        .cloned()
        .expect("expr");
        for queries in [first, second] {
            let mut env = NoHostEnv {
                queries: Some(queries),
                ..NoHostEnv::new()
            };
            assert_eq!(expr.eval_f32(&mut env.cx()), 6.0);
        }
    }

    #[cfg(feature = "vm")]
    #[test]
    fn every_stub_returns_its_no_subject_default() {
        use molangx::compile::{CompileOptions, compile};
        use molangx::vm::{NoHostEnv, Value};
        let mut env = NoHostEnv::new();
        let mut non_zero = Vec::new();
        let catalog = molangx::stdlib::queries(Side::Client);
        for decl in catalog {
            let version = decl.shape().ranges.as_slice()[0].first().as_i16();
            let options = CompileOptions {
                admission: QueryAdmission::Sets(QuerySetMask::BUILTIN),
                ..CompileOptions::from_raw_version(
                    catalog.clone(),
                    molangx::version::RawVersion(version),
                )
            };
            let compiled = compile(decl.name(), &options);
            assert_eq!(
                compiled.failure(),
                None,
                "{}: {:?}",
                decl.name(),
                compiled.diagnostics()
            );
            let value = compiled.expr().cloned().expect("expr").eval(&mut env.cx());
            assert_eq!(
                value,
                Value::from(decl.shape().default_return),
                "{}",
                decl.name()
            );
            if value != Value::Float(0.0) {
                non_zero.push((decl.name(), value));
            }
        }
        assert!(
            env.sink.is_empty(),
            "a stub logs nothing: {:?}",
            env.sink.take()
        );
        // Exactly the listed exceptions differ from the float 0.0.
        let mut names: Vec<&str> = non_zero.iter().map(|(name, _)| *name).collect();
        names.sort_unstable();
        assert_eq!(
            names,
            [
                "query.armor_color_slot",
                "query.combine_entities",
                "query.get_equipped_item_name",
                "query.owner_identifier",
                "query.spellcolor",
                "query.ticks_since_last_kinetic_weapon_hit",
                "query.time_since_last_vibration_detection",
            ]
        );
        let find = |name: &str| {
            non_zero
                .iter()
                .find(|(n, _)| *n == name)
                .map(|(_, v)| v.clone())
        };
        assert_eq!(find("query.armor_color_slot"), Some(Value::Float(1.0)));
        assert_eq!(
            find("query.time_since_last_vibration_detection"),
            Some(Value::Float(-1.0))
        );
        assert_eq!(
            find("query.ticks_since_last_kinetic_weapon_hit"),
            Some(Value::Float(-1.0))
        );
        // The empty-string hash is the hash 0.
        assert_eq!(
            find("query.owner_identifier"),
            Some(Value::Hash(molangx::hash::HashedStr::EMPTY))
        );
        assert_eq!(
            find("query.get_equipped_item_name"),
            Some(Value::Hash(molangx::hash::HashedStr::EMPTY))
        );
        assert_eq!(find("query.combine_entities"), Some(Value::actor_array([])));
    }
}

#[cfg(feature = "cache")]
mod cache {
    use std::sync::Arc;

    use molangx::cache::CompileCache;
    use molangx::compile::CompileOptions;
    use molangx::version::MolangVersion;

    /// A different version is a different entry.
    #[test]
    fn cache_smoke() {
        let cache = CompileCache::new();
        let options = CompileOptions::server(MolangVersion::LATEST);
        let first = cache.compile("1 + 2", &options);
        let second = cache.compile("1 + 2", &options);
        assert!(Arc::ptr_eq(&first, &second));
        assert_eq!(first.failure(), None);
        let other = cache.compile(
            "1 + 2",
            &CompileOptions {
                raw_version: molangx::version::RawVersion(5),
                ..options
            },
        );
        assert!(!Arc::ptr_eq(&first, &other));
        assert_eq!((cache.len(), cache.hits(), cache.misses()), (2, 1, 2));
    }
}

/// The paths of the standard library.
mod stdlib {
    use molangx::catalog::{QueryCatalog, Side};
    use molangx::stdlib::{self, MATH_META, MathFn, MathMeta, query};
    use molangx::version::semver::Version;

    #[test]
    fn the_standard_library_paths() {
        let server: &'static QueryCatalog = stdlib::queries(Side::Server);
        assert!(server.contains(query::IS_BABY));
        let older: QueryCatalog = stdlib::queries_at(Side::Client, &Version::new(1, 26, 0));
        assert!(!older.contains(query::FUSE_TIME));
        let row: &MathMeta = &MATH_META[MathFn::Sin as usize];
        assert_eq!(row.token, "math.sin");
        assert_eq!(MathFn::from_token("math.pi"), Some(MathFn::Pi));
    }

    #[cfg(feature = "compiler")]
    #[test]
    fn the_math_library_paths() {
        use molangx::numeric::PostOp;
        use molangx::stdlib::math::{self, DieRoll, PI};
        assert_eq!(math::sin(90.0, PostOp::IDENTITY), 1.0);
        assert_eq!(math::ease_in_quad(0.0, 10.0, 0.5, PostOp::IDENTITY), 2.5);
        assert_eq!(math::pi(PostOp::IDENTITY), PI);
        let roll = DieRoll::new(1.0, 2.0, 2.0);
        assert_eq!(roll.remaining(), 1);
    }
}

#[cfg(all(feature = "vm", feature = "stdlib"))]
mod rng {
    use molangx::compile::{CompileOptions, compile};
    use molangx::rng::{
        FixedRng, Xorshift128,
        rand_core::{Infallible, Rng, SeedableRng, TryRng},
        sample,
    };
    use molangx::version::MolangVersion;
    use molangx::vm::{HostEnv, NoHost, NoHostEnv, Subjects};

    /// A host's own generator, written against the re-exported `rand_core`: each sample is one
    /// `next_u32`.
    #[derive(Debug, Default)]
    struct Counter(u32);

    impl TryRng for Counter {
        type Error = Infallible;
        fn try_next_u32(&mut self) -> Result<u32, Infallible> {
            self.0 += 1 << 28;
            Ok(self.0)
        }
        fn try_next_u64(&mut self) -> Result<u64, Infallible> {
            unreachable!("a sample is one next_u32")
        }
        fn try_fill_bytes(&mut self, _: &mut [u8]) -> Result<(), Infallible> {
            unreachable!("a sample is one next_u32")
        }
    }

    #[test]
    fn a_host_generator_is_the_random_source() {
        let mut env = HostEnv::<NoHost>::default().with_rng(Counter::default());
        let expr = compile(
            "math.random(0, 16) + math.random(0, 16) * 100",
            &CompileOptions::server(MolangVersion::LATEST),
        )
        .into_result()
        .expect("compiles")
        .0;
        assert_eq!(
            expr.eval_f32(&mut env.cx(&mut NoHost, Subjects::none())),
            2.0 + 4.0 * 100.0
        );
        assert_eq!(env.rng.0, 2 << 28);
        let dynamic: &mut dyn Rng = &mut env.rng;
        assert_eq!(sample(dynamic), 0.375);
    }

    #[test]
    fn the_sources_and_the_sample() {
        let mut fixed: FixedRng = FixedRng::from_sample(0.25).expect("a word's sample");
        assert_eq!((fixed.0, sample(&mut fixed)), (1 << 29, 0.25));
        assert_eq!(
            [FixedRng::ZERO, FixedRng::HALF, FixedRng::ONE].map(|mut f| sample(&mut f)),
            [0.0, 0.5, 1.0]
        );
        let mut seeded = Xorshift128::seed_from_u64(7);
        assert_eq!(
            seeded,
            Xorshift128::from_seed(bytes(Xorshift128::seed_from_u64(7).state()))
        );
        assert_eq!(
            sample(&mut seeded),
            sample(&mut Xorshift128::with_state(
                Xorshift128::seed_from_u64(7).state()
            ))
        );
        let _: fn(&mut NoHostEnv) -> &mut Xorshift128 = |env| &mut env.rng;
    }

    fn bytes(state: [u32; 4]) -> [u8; 16] {
        let bytes: Vec<u8> = state.iter().flat_map(|word| word.to_le_bytes()).collect();
        bytes.try_into().expect("16 bytes")
    }
}

#[cfg(feature = "facet")]
mod facet {
    use molangx::json::{MolangSource, MolangValueRepr};

    #[test]
    fn facet_json_smoke() {
        let read = |json: &str| facet_json::from_str::<MolangValueRepr>(json).expect("reads");
        assert_eq!(read("1.5"), MolangValueRepr::Const(1.5));
        assert_eq!(read("true"), MolangValueRepr::Bool(true));
        assert_eq!(
            read(r#"{"expression": "q.x", "version": 4}"#),
            MolangValueRepr::Expr(MolangSource::object("q.x", 4))
        );
        let written = facet_json::to_string(&MolangValueRepr::Expr(MolangSource::object("q.x", 4)))
            .expect("writes");
        assert_eq!(written, r#"{"expression":"q.x","version":4}"#);
    }
}

mod version_thresholds {
    use super::{ev, ev_pre};
    use molangx::version::{EngineVersion, MolangVersion, VERSION_THRESHOLDS};

    fn map(major: u64, minor: u64, patch: u64) -> i16 {
        MolangVersion::from(&ev(major, minor, patch)).as_i16()
    }

    fn map_pre(major: u64, minor: u64, patch: u64) -> i16 {
        MolangVersion::from(&ev_pre(major, minor, patch)).as_i16()
    }

    #[test]
    fn engine_version_1_16_maps_to_version_0() {
        assert_eq!(MolangVersion::from(&ev(1, 16, 0)), MolangVersion::V0);
    }

    #[test]
    fn current_releases_map_to_the_latest_version() {
        for current in [ev(1, 26, 0), ev(1, 26, 45)] {
            assert_eq!(MolangVersion::from(&current), MolangVersion::LATEST);
        }
    }

    #[test]
    fn invalid_is_minus_one() {
        assert_eq!(MolangVersion::Invalid as i16, -1);
        assert_eq!(MolangVersion::Invalid.as_i16(), -1);
    }

    #[test]
    fn version_0_is_zero() {
        assert_eq!(MolangVersion::V0 as i16, 0);
        assert_eq!(MolangVersion::V0.as_i16(), 0);
    }

    #[test]
    fn the_latest_version_is_positive() {
        assert!(MolangVersion::LATEST.as_i16() > 0);
    }

    #[test]
    fn the_latest_version_is_thirteen() {
        assert_eq!(MolangVersion::LATEST, MolangVersion::V13);
        assert_eq!(MolangVersion::LATEST.as_i16(), 13);
    }

    #[test]
    fn a_version_to_its_first_engine_version_and_back_is_never_lower() {
        for raw in 1..=13 {
            let v = MolangVersion::from_i16(raw).unwrap();
            let sem = v.first_engine_version().unwrap();
            let back = MolangVersion::from(sem);
            assert!(back >= v, "{v:?} -> {sem:?} -> {back:?}");
            // Only 3 does not round-trip to itself: its first version is also 4's.
            if raw == 3 {
                assert_eq!(back, MolangVersion::V4);
            } else {
                assert_eq!(back, v);
            }
        }
    }

    #[test]
    fn enum_values_are_minus_one_through_thirteen() {
        let names = [
            (-1, MolangVersion::Invalid),
            (0, MolangVersion::V0),
            (1, MolangVersion::V1),
            (2, MolangVersion::V2),
            (3, MolangVersion::V3),
            (4, MolangVersion::V4),
            (5, MolangVersion::V5),
            (6, MolangVersion::V6),
            (7, MolangVersion::V7),
            (8, MolangVersion::V8),
            (9, MolangVersion::V9),
            (10, MolangVersion::V10),
            (11, MolangVersion::V11),
            (12, MolangVersion::V12),
            (13, MolangVersion::V13),
        ];
        for (raw, v) in names {
            assert_eq!(v.as_i16(), raw);
            assert_eq!(MolangVersion::from_i16(raw), Some(v));
        }
        assert_eq!(MolangVersion::from_i16(-2), None);
        assert_eq!(MolangVersion::from_i16(14), None);
        assert_eq!(size_of::<MolangVersion>(), size_of::<i16>());
    }

    /// Versions 3 and 4 share 1.17.40.
    #[test]
    fn table_has_the_thirteen_thresholds() {
        let expected = [
            (1, 17, 0),
            (1, 17, 30),
            (1, 17, 40),
            (1, 17, 40),
            (1, 18, 10),
            (1, 18, 20),
            (1, 19, 60),
            (1, 20, 0),
            (1, 20, 10),
            (1, 20, 40),
            (1, 20, 50),
            (1, 20, 70),
            (1, 21, 100),
        ];
        assert_eq!(VERSION_THRESHOLDS.len(), expected.len());
        for (i, (entry, (major, minor, patch))) in
            VERSION_THRESHOLDS.iter().zip(expected).enumerate()
        {
            assert_eq!(entry.engine_version, ev(major, minor, patch), "row {i}");
            assert_eq!(entry.version.as_i16(), i as i16 + 1, "row {i}");
        }
    }

    #[test]
    fn every_band_maps_to_its_version() {
        type Triple = (u64, u64, u64);
        // (first version of the band, last version inside it, expected MolangVersion)
        let bands: [(Triple, Triple, i16); 12] = [
            ((0, 0, 0), (1, 16, 220), 0),
            ((1, 17, 0), (1, 17, 29), 1),
            ((1, 17, 30), (1, 17, 39), 2),
            ((1, 17, 40), (1, 18, 9), 4),
            ((1, 18, 10), (1, 18, 19), 5),
            ((1, 18, 20), (1, 19, 59), 6),
            ((1, 19, 60), (1, 19, 999), 7),
            ((1, 20, 0), (1, 20, 9), 8),
            ((1, 20, 10), (1, 20, 39), 9),
            ((1, 20, 40), (1, 20, 49), 10),
            ((1, 20, 50), (1, 20, 69), 11),
            ((1, 20, 70), (1, 21, 99), 12),
        ];
        for ((a, b, c), (x, y, z), v) in bands {
            assert_eq!(map(a, b, c), v, "{a}.{b}.{c}");
            assert_eq!(map(x, y, z), v, "{x}.{y}.{z}");
        }
        for (a, b, c) in [
            (1, 21, 100),
            (1, 21, 101),
            (1, 26, 0),
            (2, 0, 0),
            (65535, 65535, 65535),
        ] {
            assert_eq!(map(a, b, c), 13, "{a}.{b}.{c}");
        }
    }

    #[test]
    fn components_compare_numerically_not_lexically() {
        // 1.9.x < 1.17.0 < 1.100.0 numerically; a string compare would order them differently.
        assert_eq!(map(1, 9, 0), 0);
        assert_eq!(map(1, 100, 0), 13);
        assert_eq!(map(0, 99, 99), 0);
        assert_eq!(map(1, 20, 5), 8);
        assert_eq!(map(1, 20, 100), 12);
    }

    #[test]
    fn version_three_is_never_produced() {
        assert_eq!(map(1, 17, 39), 2);
        assert_eq!(map(1, 17, 40), 4);
        for major in 0..3 {
            for minor in 0..30 {
                for patch in (0..200).step_by(5) {
                    assert_ne!(map(major, minor, patch), 3);
                    assert_ne!(map_pre(major, minor, patch), 3);
                }
            }
        }
    }

    #[test]
    fn pre_release_sorts_below_its_release() {
        assert_eq!(map_pre(1, 21, 100), 12);
        assert_eq!(map(1, 21, 100), 13);
        assert_eq!(map_pre(1, 17, 0), 0);
        assert_eq!(map_pre(1, 17, 40), 2);
        assert_eq!(map_pre(1, 19, 60), 6);
        // A pre-release of a later patch is still above the previous threshold.
        assert_eq!(map_pre(1, 21, 101), 13);
        assert_eq!(map_pre(1, 17, 1), 1);
    }

    #[test]
    fn build_metadata_is_not_part_of_the_comparison() {
        let molang = |text| MolangVersion::from_engine_version_str(text).as_i16();
        assert_eq!(molang("1.21.100+build"), 13);
        assert_eq!(molang("1.21.100-beta+build"), 12);
        assert_eq!(map(1, 21, 100), 13);
    }

    #[test]
    fn any_version_maps_to_latest() {
        assert_eq!(
            MolangVersion::from(&EngineVersion::Any),
            MolangVersion::LATEST
        );
        assert_eq!(
            MolangVersion::from_engine_version_str("*"),
            MolangVersion::LATEST
        );
    }

    #[test]
    fn less_than_on_engine_versions() {
        let v = ev;
        assert!(v(1, 17, 39).is_less_than(&v(1, 17, 40)));
        assert!(!v(1, 17, 40).is_less_than(&v(1, 17, 40)));
        assert!(ev_pre(1, 17, 40).is_less_than(&v(1, 17, 40)));
        // A pre-release on the right.
        assert!(!v(1, 17, 40).is_less_than(&ev_pre(1, 17, 40)));
        assert!(!ev_pre(1, 17, 40).is_less_than(&ev_pre(1, 17, 40)));
        assert!(!EngineVersion::Any.is_less_than(&v(65535, 65535, 65535)));
    }

    #[test]
    fn is_less_than_is_the_only_order() {
        // `EngineVersion` has no `Ord` (a structural one inverts these cases); pin `is_less_than`.
        let v = ev;
        let beta = ev_pre(1, 20, 50);
        // A pre-release is below its release; with the pre-release on the right, its release is not
        // below it and the previous release is.
        assert!(beta.is_less_than(&v(1, 20, 50)));
        assert!(!v(1, 20, 50).is_less_than(&beta));
        assert!(v(1, 20, 49).is_less_than(&beta));
        // A release compares its triple numerically.
        assert!(v(1, 20, 50).is_less_than(&v(1, 20, 51)));
        assert!(v(1, 9, 0).is_less_than(&v(1, 20, 0)));
        assert!(!v(1, 20, 51).is_less_than(&v(1, 20, 50)));
        // `"*"` is never less than anything; on the right it compares as 0.0.0.
        assert!(!EngineVersion::Any.is_less_than(&v(1, 0, 0)));
        assert!(!EngineVersion::Any.is_less_than(&EngineVersion::Any));
        assert!(!v(1, 0, 0).is_less_than(&EngineVersion::Any));
    }

    #[test]
    fn first_engine_version_is_row_v_minus_one() {
        let v = |raw| MolangVersion::from_i16(raw).unwrap().first_engine_version();
        assert_eq!(v(1), Some(&ev(1, 17, 0)));
        assert_eq!(v(2), Some(&ev(1, 17, 30)));
        assert_eq!(v(3), Some(&ev(1, 17, 40)));
        assert_eq!(v(4), Some(&ev(1, 17, 40)));
        assert_eq!(v(13), Some(&ev(1, 21, 100)));
        for raw in 1..=13 {
            assert_eq!(
                v(raw),
                Some(&VERSION_THRESHOLDS[(raw - 1) as usize].engine_version)
            );
        }
    }

    #[test]
    fn first_engine_version_is_none_below_version_1() {
        assert_eq!(MolangVersion::V0.first_engine_version(), None);
        assert_eq!(MolangVersion::Invalid.first_engine_version(), None);
    }

    #[test]
    fn effective_clamps_the_raw_value() {
        for raw in -1..=13 {
            assert_eq!(molangx::version::RawVersion(raw).effective().as_i16(), raw);
        }
        // Above 13 passes every gate as 13 does.
        for raw in [14, 15, 100, i16::MAX] {
            assert_eq!(
                molangx::version::RawVersion(raw).effective(),
                MolangVersion::LATEST
            );
        }
        // Below −1 behaves as −1.
        for raw in [-2, -100, i16::MIN] {
            assert_eq!(
                molangx::version::RawVersion(raw).effective(),
                MolangVersion::Invalid
            );
        }
    }

    #[test]
    fn invalid_takes_the_version_zero_branch_of_every_gate() {
        let zero = MolangVersion::V0;
        let invalid = MolangVersion::Invalid;
        assert_eq!(
            invalid.reports_expression_errors(),
            zero.reports_expression_errors()
        );
        assert_eq!(
            invalid.reports_unexpected_operators(),
            zero.reports_unexpected_operators()
        );
        assert_eq!(invalid.right_assoc_ternary(), zero.right_assoc_ternary());
        assert_eq!(
            invalid.c_like_logic_precedence(),
            zero.c_like_logic_precedence()
        );
        assert_eq!(invalid.signed_division_fix(), zero.signed_division_fix());
        assert!(!invalid.reports_expression_errors());
    }

    /// Parser gates at 3, 4, 5, 6, the division gate at 7; no others.
    #[test]
    fn gates_switch_at_their_versions() {
        let at = |raw| MolangVersion::from_i16(raw).unwrap();
        for raw in -1..=13 {
            let v = at(raw);
            assert_eq!(v.reports_expression_errors(), raw >= 3, "{raw}");
            assert_eq!(v.reports_unexpected_operators(), raw >= 4, "{raw}");
            assert_eq!(v.right_assoc_ternary(), raw >= 5, "{raw}");
            assert_eq!(v.c_like_logic_precedence(), raw >= 6, "{raw}");
            assert_eq!(v.signed_division_fix(), raw >= 7, "{raw}");
        }
    }

    #[test]
    fn first_engine_version_is_const() {
        const S: Option<&EngineVersion> = MolangVersion::LATEST.first_engine_version();
        assert_eq!(S, Some(&ev(1, 21, 100)));
    }
}

mod engine_version {
    use super::{ev, ev_pre, parse};
    use molangx::version::{EngineVersion, MolangVersion};

    fn molang(text: &str) -> i16 {
        parse(text)
            .as_ref()
            .map_or(MolangVersion::Invalid, MolangVersion::from)
            .as_i16()
    }

    #[test]
    fn release_strings() {
        assert_eq!(parse("1.21.100"), Some(ev(1, 21, 100)));
        assert_eq!(parse("0.0.0"), Some(ev(0, 0, 0)));
        assert_eq!(parse("65535.65535.65535"), Some(ev(65535, 65535, 65535)));
        assert_eq!(parse("1.20.0"), Some(ev(1, 20, 0)));
        assert_eq!(molang("1.16.0"), 0);
        assert_eq!(molang("1.17.39"), 2);
        assert_eq!(molang("1.17.40"), 4);
        assert_eq!(molang("1.21.100"), 13);
    }

    #[test]
    fn pre_release_and_build_metadata() {
        // The tag's text and build metadata take no part in the comparison.
        for text in [
            "1.21.100-beta",
            "1.21.100-beta.1",
            "1.21.100-rc-2",
            "1.21.100-beta+b.5",
        ] {
            assert_eq!(molang(text), 12, "{text:?}");
        }
        assert_eq!(molang("1.21.100+build.5"), 13);
        assert_eq!(parse("1.21.100-pre"), Some(ev_pre(1, 21, 100)));
        assert_ne!(parse("1.21.100+build.5"), Some(ev(1, 21, 100)));
        // A pre-release sorts below its release (`1.21.100-beta` → 12).
        assert_eq!(molang("1.21.100-beta"), 12);
        assert_eq!(molang("1.21.100+abc"), 13);
    }

    /// `1.20.0` maps to 8 and `1.19.50` / `1.19.60-beta` to 6; `1.020.0`, `01.20.0`, `1.19.60-01`
    /// and `1.2.3-01` are ill-formatted and map to `Invalid`, not the version they would name.
    #[test]
    fn version_texts_the_server_refuses() {
        assert_eq!(molang("1.20.0"), 8);
        assert_eq!(molang("1.19.50"), 6);
        assert_eq!(molang("1.19.60-beta"), 6);
        for text in ["1.020.0", "01.20.0", "1.19.60-01", "1.2.3-01"] {
            assert_eq!(parse(text), None, "{text:?}");
            assert_eq!(
                parse(text)
                    .as_ref()
                    .map_or(MolangVersion::Invalid, MolangVersion::from),
                MolangVersion::Invalid,
                "{text:?}"
            );
        }
    }

    #[test]
    fn any_version() {
        assert_eq!(parse("*"), Some(EngineVersion::Any));
        assert_eq!(molang("*"), 13);
        for text in ["**", " *", "*.1.2", "1.*.0"] {
            assert_eq!(parse(text), None, "{text:?}");
        }
    }

    #[test]
    fn invalid_strings_map_to_invalid() {
        for text in [
            "",
            " ",
            "1",
            "1.21",
            "1.21.100.0",
            " 1.21.100",
            "1.21.100 ",
            "1..100",
            ".1.2",
            "1.2.",
            "-1.2.3",
            "+1.2.3",
            "1.2.-3",
            "1.2.3-",
            "1.2.3+",
            "1.2.3-beta+",
            "1.2.3-be ta",
            "1.2.99999999999999999999",
            "a.b.c",
            "1.2.3a",
            "1,2,3",
            "v1.2.3",
            "1.020.0",
            "01.21.100",
            "1.21.0100",
            "00.0.0",
        ] {
            assert_eq!(parse(text), None, "{text:?}");
            assert_eq!(molang(text), -1, "{text:?}");
        }
    }

    #[test]
    fn json_arrays() {
        assert_eq!(
            EngineVersion::from_json_array(&[1.0, 20.0, 0.0]),
            Some(ev(1, 20, 0))
        );
        assert_eq!(
            EngineVersion::from_json_array(&[1.0, 13.0, 0.0]).map(|v| MolangVersion::from(&v)),
            Some(MolangVersion::V0)
        );
        assert_eq!(
            EngineVersion::from_json_array(&[65536.0, 0.0, 0.0]),
            Some(ev(65536, 0, 0))
        );
        for array in [
            &[][..],
            &[1.0],
            &[1.0, 20.0],
            &[1.0, 20.0, 0.0, 0.0],
            &[1.0, 20.5, 0.0],
            &[1.0, -1.0, 0.0],
            &[1.0, 1e20, 0.0],
            &[1.0, f64::NAN, 0.0],
            &[1.0, f64::INFINITY, 0.0],
        ] {
            assert_eq!(EngineVersion::from_json_array(array), None, "{array:?}");
            assert_eq!(
                EngineVersion::from_json_array(array)
                    .as_ref()
                    .map_or(MolangVersion::Invalid, MolangVersion::from),
                MolangVersion::Invalid
            );
        }
    }
}

mod fnv_hash {
    //! `HashedStr`: 64-bit FNV-1 with the empty string hashing to 0.

    use molangx::hash::{HashedStr, fnv1_64};

    /// The 64-bit FNV offset basis.
    const OFFSET_BASIS: u64 = 0xcbf2_9ce4_8422_2325;
    /// The 64-bit FNV prime.
    const PRIME: u64 = 0x0000_0100_0000_01b3;

    #[test]
    fn conformance_vectors() {
        assert_eq!(HashedStr::new("a").as_u64(), 12_638_153_115_695_167_422);
        assert_eq!(HashedStr::new("abc").as_u64(), 15_626_587_013_303_479_755);
        assert_eq!(HashedStr::new("ABC").as_u64(), 15_595_941_425_208_037_995);
        assert_eq!(HashedStr::new(" ").as_u64(), 12_638_153_115_695_167_487);
    }

    #[test]
    fn empty_string_hashes_to_zero_not_the_basis() {
        assert_eq!(HashedStr::new("").as_u64(), 0);
        assert_eq!(fnv1_64(b""), 0);
        assert_eq!(HashedStr::new(""), HashedStr::EMPTY);
        assert!(HashedStr::EMPTY.is_empty());
        assert_eq!(HashedStr::default(), HashedStr::EMPTY);
    }

    #[test]
    fn fnv1_multiplies_then_xors() {
        // One byte: (basis * prime) ^ byte. FNV-1a would be (basis ^ byte) * prime.
        let fnv1 = OFFSET_BASIS.wrapping_mul(PRIME) ^ u64::from(b'a');
        let fnv1a = (OFFSET_BASIS ^ u64::from(b'a')).wrapping_mul(PRIME);
        assert_eq!(fnv1_64(b"a"), fnv1);
        assert_ne!(fnv1_64(b"a"), fnv1a);
        // Reference loop over a longer input.
        let input = b"query.get_name";
        let mut h = OFFSET_BASIS;
        for &b in input {
            h = h.wrapping_mul(PRIME) ^ u64::from(b);
        }
        assert_eq!(fnv1_64(input), h);
    }

    #[test]
    fn bytes_are_hashed_as_written() {
        // No case folding and no unescaping (`'a\'b'` hashes the 4 bytes `a\'b`).
        assert_ne!(HashedStr::new("abc"), HashedStr::new("ABC"));
        assert_eq!(HashedStr::new("a\\'b"), HashedStr::from_bytes(b"a\\'b"));
        assert_ne!(HashedStr::new("a\\'b"), HashedStr::new("a'b"));
        // Non-UTF-8 bytes hash like any other.
        assert_eq!(
            HashedStr::from_bytes(&[0xff, 0xfe]).as_u64(),
            fnv1_64(&[0xff, 0xfe])
        );
    }

    #[test]
    fn hashing_stops_at_the_first_nul() {
        assert_eq!(fnv1_64(b"abc\0def"), fnv1_64(b"abc"));
        assert_eq!(fnv1_64(b"\0abc"), 0);
    }

    #[test]
    fn hashed_str_is_a_copy_u64() {
        assert_eq!(size_of::<HashedStr>(), size_of::<u64>());
        let h = HashedStr::from("moo");
        let copy = h;
        assert_eq!(h, copy);
        assert_eq!(u64::from(h), 15_615_043_240_721_163_028);
        assert_eq!(HashedStr::from(15_615_043_240_721_163_028_u64), h);
        assert_eq!(HashedStr::from_u64(h.as_u64()), h);
    }

    #[test]
    fn hashing_is_const() {
        const GREETING: HashedStr = HashedStr::new("hello");
        const PLANET: u64 = fnv1_64(b"planet");
        assert_eq!(GREETING, HashedStr::new("hello"));
        assert_eq!(PLANET, HashedStr::new("planet").as_u64());
    }
}

mod catalog_and_tables {
    //! The standard query catalogue, the operator and the math metadata, through the public API.

    use std::collections::{BTreeMap, BTreeSet};

    use molangx::catalog::{
        Arity, DefaultReturn, ParseQuerySetError, QueryAdmission, QueryCatalog, QueryDecl,
        QuerySetMask, QuerySide, Reads, ReturnType, Side,
    };
    use molangx::ops::{ExpressionOp, OpFlags, OpSet};
    use molangx::stdlib::{MATH_META, MathFn, query};
    use molangx::version::{Experiment, ExperimentMask, semver::Version};

    use molangx::version::MolangVersion;

    fn catalog() -> &'static QueryCatalog {
        molangx::stdlib::queries(Side::Client)
    }

    /// Every version, `Invalid` included.
    fn versions() -> impl Iterator<Item = MolangVersion> {
        (-1..=13).map(|v| MolangVersion::from_i16(v).unwrap())
    }

    fn get(name: &str) -> &'static QueryDecl {
        catalog()
            .get(name)
            .unwrap_or_else(|| panic!("{name} is not a standard query"))
    }

    fn every_set() -> QueryAdmission {
        QueryAdmission::Sets(QuerySetMask::BUILTIN)
    }

    /// Resolution with every set admitted and every experiment enabled: only the version decides.
    fn impl_at(name: &str, v: MolangVersion) -> Option<u8> {
        get(name).resolve(
            molangx::version::RawVersion(v.as_i16()),
            &every_set(),
            ExperimentMask::all(),
        )
    }

    /// The queries every supported release has (not the four of later releases).
    fn baseline() -> impl Iterator<Item = &'static QueryDecl> {
        catalog()
            .iter()
            .filter(|decl| decl.shape().first_release.is_none())
    }

    #[test]
    fn ranges_and_names_per_set() {
        let mut ranges_per_set: BTreeMap<&str, usize> = BTreeMap::new();
        let mut names_per_set: BTreeMap<&str, BTreeSet<&str>> = BTreeMap::new();
        for decl in baseline() {
            for range in decl.shape().ranges.as_slice() {
                let set = range.sets().name().expect("one set per range");
                *ranges_per_set.entry(set).or_default() += 1;
                names_per_set.entry(set).or_default().insert(decl.name());
            }
        }
        assert_eq!(
            ranges_per_set,
            BTreeMap::from([("default", 315), ("tags", 2), ("world_gen", 4)])
        );
        let sizes: BTreeMap<&str, usize> =
            names_per_set.iter().map(|(k, v)| (*k, v.len())).collect();
        assert_eq!(
            sizes,
            BTreeMap::from([("default", 309), ("tags", 2), ("world_gen", 4)])
        );
        assert_eq!(
            names_per_set["tags"],
            BTreeSet::from(["query.all_tags", "query.any_tag"])
        );
        assert_eq!(
            names_per_set["world_gen"],
            BTreeSet::from([
                "query.above_top_solid",
                "query.has_biome_tag",
                "query.heightmap",
                "query.noise"
            ])
        );
        for name in ["query.any_tag", "query.all_tags"] {
            assert_eq!(get(name).sets(), QuerySetMask::TAGS);
        }
        // With the later releases: world_gen gains the two 1.26.50 biome-tag queries (six in all).
        let world_gen: BTreeSet<&str> = catalog()
            .iter()
            .filter(|d| d.sets() == QuerySetMask::WORLD_GEN)
            .map(QueryDecl::name)
            .collect();
        assert_eq!(world_gen.len(), 6);
        assert!(
            world_gen.contains("query.has_all_biome_tags")
                && world_gen.contains("query.has_any_biome_tags")
        );
        assert_eq!(
            catalog()
                .iter()
                .filter(|d| d.sets() == QuerySetMask::DEFAULT)
                .count(),
            311
        );
    }

    #[test]
    fn counts_and_names() {
        assert_eq!(catalog().len(), 315 + 4);
        assert_eq!(molangx::stdlib::queries(Side::Server).len(), 319);
        assert_eq!(baseline().count(), 315);
        for decl in catalog() {
            assert_eq!(catalog().get(decl.name()), Some(decl));
            assert_eq!(catalog().get_suffix(decl.suffix()), Some(decl));
            assert_eq!(format!("query.{}", decl.suffix()), decl.name());
        }
        let names: BTreeSet<&str> = catalog().iter().map(QueryDecl::name).collect();
        assert_eq!(names.len(), 319, "one declaration per name");
        assert_eq!(query::BLOCK_STATE, "query.block_state");
        assert_eq!(query::HAD_COMPONENT_GROUP, "query.had_component_group");
        assert_eq!(query::COUNT, "query.count");
        assert_eq!(query::FUSE_TIME, "query.fuse_time");
        assert_eq!(
            catalog().iter().next().map(QueryDecl::name),
            Some(query::ABOVE_TOP_SOLID)
        );
    }

    #[test]
    fn lookup_accepts_only_the_canonical_name() {
        for name in [
            "q.block_state",
            "QUERY.BLOCK_STATE",
            "Query.block_state",
            "query.",
            "query",
            "",
            "query.block_state ",
            "math.abs",
            "query.block_stat",
            "query.block_statex",
            "variable.x",
            "query.sum_test",
        ] {
            assert!(catalog().get(name).is_none(), "{name:?}");
        }
        for bad in [
            "",
            "query.block_state",
            "Block_State",
            "block_stat",
            "block_state ",
            "q.is_baby",
            "zzz",
        ] {
            assert!(catalog().get_suffix(bad).is_none(), "{bad:?}");
        }
    }

    #[test]
    fn names_have_the_query_prefix_and_a_lower_case_suffix() {
        for decl in catalog() {
            assert!(!decl.suffix().is_empty(), "{}", decl.name());
            assert!(
                decl.suffix()
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_'),
                "{}",
                decl.name()
            );
        }
    }

    #[test]
    fn version_ranges_are_disjoint_ascending_and_from_version_zero() {
        for decl in catalog() {
            assert!(
                !decl.shape().ranges.as_slice().is_empty(),
                "{}",
                decl.name()
            );
            for (i, range) in decl.shape().ranges.as_slice().iter().enumerate() {
                assert!(
                    range.first() <= range.last() && range.first() >= MolangVersion::V0,
                    "{}",
                    decl.name()
                );
                if i > 0 {
                    assert!(
                        decl.shape().ranges.as_slice()[i - 1].last() < range.first(),
                        "{}: ranges overlap or are out of order",
                        decl.name()
                    );
                }
            }
        }
    }

    #[test]
    fn each_implementation_serves_one_contiguous_window() {
        for decl in catalog() {
            let served: Vec<Option<u8>> = versions().map(|v| impl_at(decl.name(), v)).collect();
            for impl_idx in 0..u8::try_from(decl.shape().ranges.as_slice().len()).unwrap() {
                let hits: Vec<usize> = served
                    .iter()
                    .enumerate()
                    .filter(|(_, s)| **s == Some(impl_idx))
                    .map(|(i, _)| i)
                    .collect();
                assert!(
                    !hits.is_empty(),
                    "{}: implementation {impl_idx} is never served",
                    decl.name()
                );
                assert_eq!(
                    hits.last().unwrap() - hits.first().unwrap() + 1,
                    hits.len(),
                    "{}: implementation {impl_idx} is not contiguous",
                    decl.name()
                );
            }
            // Monotone: a later version never goes back to an earlier implementation.
            let order: Vec<u8> = served.iter().flatten().copied().collect();
            assert!(
                order.windows(2).all(|w| w[0] <= w[1]),
                "{}: {served:?}",
                decl.name()
            );
        }
    }

    #[test]
    fn version_windows_of_the_two_implementation_names() {
        let windows = |name: &str| {
            get(name)
                .shape()
                .ranges
                .as_slice()
                .iter()
                .map(|r| (r.first().as_i16(), r.last().as_i16()))
                .collect::<Vec<_>>()
        };
        let two = [
            ("query.item_remaining_use_duration", (0, 1), (2, 13)),
            ("query.cape_flap_amount", (0, 7), (8, 13)),
            ("query.surface_particle_color", (0, 11), (12, 13)),
            (
                "query.surface_particle_texture_coordinate",
                (0, 11),
                (12, 13),
            ),
            ("query.surface_particle_texture_size", (0, 11), (12, 13)),
            ("query.is_carrying_block", (0, 12), (13, 13)),
        ];
        for (name, old, new) in two {
            assert_eq!(windows(name), [old, new], "{name}");
            for v in versions() {
                let expected = match v.as_i16() {
                    raw if raw < 0 => None,
                    raw if raw <= old.1 => Some(0),
                    _ => Some(1),
                };
                assert_eq!(impl_at(name, v), expected, "{name} at {v:?}");
            }
        }
        let multi: Vec<&str> = catalog()
            .iter()
            .filter(|d| d.shape().ranges.as_slice().len() == 2)
            .map(QueryDecl::name)
            .collect();
        assert_eq!(multi.len(), 6);
    }

    #[test]
    fn return_types() {
        let kind = |ty: ReturnType| {
            [
                (ReturnType::FLOAT, "float"),
                (ReturnType::BOOL, "bool"),
                (ReturnType::STRING, "string"),
                (ReturnType::ACTOR, "actor"),
                (ReturnType::ACTOR_ARRAY, "actor array"),
                (ReturnType::STRUCT, "struct"),
                (ReturnType::MATRIX, "matrix"),
            ]
            .into_iter()
            .find(|&(k, _)| k == ty)
            .map(|(_, name)| name)
            .expect("one kind")
        };
        let mut counts: BTreeMap<&str, usize> = BTreeMap::new();
        for decl in baseline() {
            *counts.entry(kind(decl.shape().returns)).or_default() +=
                decl.shape().ranges.as_slice().len();
        }
        assert_eq!(
            counts,
            BTreeMap::from([
                ("actor", 1),
                ("actor array", 1),
                ("bool", 152),
                ("float", 151),
                ("matrix", 5),
                ("string", 4),
                ("struct", 7)
            ])
        );
        assert_eq!(
            get("query.combine_entities").shape().returns,
            ReturnType::ACTOR_ARRAY
        );
        assert_eq!(
            get("query.equipped_item_is_attachable").shape().returns,
            ReturnType::ACTOR
        );
        assert_eq!(
            ReturnType::NUMBER,
            ReturnType::FLOAT.union(ReturnType::BOOL)
        );
        assert!(
            ReturnType::BOOL.is_number()
                && ReturnType::FLOAT.is_number()
                && !ReturnType::STRING.is_number()
        );
        assert_eq!(ReturnType::NUMBER, ReturnType::FLOAT | ReturnType::BOOL);
    }

    #[test]
    fn raw_versions_outside_the_windows_resolve_nothing() {
        let all = ExperimentMask::all();
        for raw in [-1, 14, 15, 1000, i16::MAX, -2, -100, i16::MIN] {
            for decl in catalog() {
                let listed = molangx::catalog::QueryAllowList::new(catalog(), [decl.name()])
                    .expect("declared");
                assert_eq!(
                    decl.resolve(molangx::version::RawVersion(raw), &every_set(), all),
                    None,
                    "{} at {raw}",
                    decl.name()
                );
                assert_eq!(
                    decl.resolve(
                        molangx::version::RawVersion(raw),
                        &QueryAdmission::Only(listed),
                        all
                    ),
                    None,
                    "{} at {raw} (list)",
                    decl.name()
                );
                assert!(
                    decl.shape()
                        .ranges
                        .as_slice()
                        .iter()
                        .all(|r| !r.contains_raw(molangx::version::RawVersion(raw))),
                    "{} at {raw}",
                    decl.name()
                );
            }
        }
        // The gates see 14 and 32767 as 13; the window does not.
        assert_eq!(
            molangx::version::RawVersion(14).effective(),
            MolangVersion::LATEST
        );
        assert_eq!(
            get(query::IS_CARRYING_BLOCK).resolve(
                molangx::version::RawVersion(13),
                &every_set(),
                all
            ),
            Some(1)
        );
        assert_eq!(
            get(query::IS_CARRYING_BLOCK).resolve(
                molangx::version::RawVersion(12),
                &every_set(),
                all
            ),
            Some(0)
        );
        // A source carries the raw value next to the effective one.
        let src = molangx::json::MolangSource::object("q.is_baby", 14);
        let raw = src
            .raw_version()
            .expect("an object form has its own version");
        assert_eq!(
            (raw, src.effective_version()),
            (molangx::version::RawVersion(14), MolangVersion::LATEST)
        );
        assert_eq!(get(query::IS_BABY).resolve(raw, &every_set(), all), None);
    }

    #[test]
    fn sets_gate_resolution() {
        let none = ExperimentMask::empty();
        let at = |name: &str, sets| {
            get(name)
                .resolve(
                    molangx::version::RawVersion(13),
                    &QueryAdmission::Sets(sets),
                    none,
                )
                .is_some()
        };
        assert!(at(query::IS_BABY, QuerySetMask::DEFAULT));
        assert!(!at(query::ANY_TAG, QuerySetMask::DEFAULT));
        assert!(!at(query::NOISE, QuerySetMask::DEFAULT));
        assert!(at(query::ANY_TAG, QuerySetMask::TAGS));
        assert!(at(query::ALL_TAGS, QuerySetMask::TAGS));
        assert!(!at(query::IS_BABY, QuerySetMask::TAGS));
        for name in [
            "query.noise",
            "query.heightmap",
            "query.above_top_solid",
            "query.has_biome_tag",
        ] {
            assert!(at(name, QuerySetMask::WORLD_GEN), "{name}");
            assert!(
                !at(name, QuerySetMask::DEFAULT | QuerySetMask::TAGS),
                "{name}"
            );
        }
        assert!(!at(query::IS_BABY, QuerySetMask::WORLD_GEN));
        assert!(at(
            query::NOISE,
            QuerySetMask::DEFAULT | QuerySetMask::WORLD_GEN
        ));
        for decl in catalog() {
            assert_eq!(
                decl.resolve(
                    molangx::version::RawVersion(13),
                    &QueryAdmission::Sets(QuerySetMask::empty()),
                    ExperimentMask::all()
                ),
                None,
                "{}",
                decl.name()
            );
        }
    }

    #[test]
    fn a_list_replaces_the_sets() {
        let none = ExperimentMask::empty();
        let at = |name: &str, raw: i16, list: &[&str]| {
            let list = molangx::catalog::QueryAllowList::new(catalog(), list).expect("declared");
            get(name)
                .resolve(
                    molangx::version::RawVersion(raw),
                    &QueryAdmission::Only(list),
                    none,
                )
                .is_some()
        };
        let blocks = [query::BLOCK_STATE];
        assert!(at(query::BLOCK_STATE, 13, &blocks));
        assert!(
            !at(query::IS_BABY, 13, &blocks),
            "the list replaces the default set"
        );
        assert!(!at(query::HAS_BLOCK_STATE, 13, &blocks));
        let property = [query::HAD_COMPONENT_GROUP];
        assert!(at(query::HAD_COMPONENT_GROUP, 13, &property));
        assert!(!at(query::PROPERTY, 13, &property));
        // The list still obeys the version window.
        let old = [query::BLOCK_PROPERTY];
        assert!(at(query::BLOCK_PROPERTY, MolangVersion::V9.as_i16(), &old));
        assert!(!at(
            query::BLOCK_PROPERTY,
            MolangVersion::V10.as_i16(),
            &old
        ));
        // The list admits a query whatever its set.
        assert!(at(query::ANY_TAG, 13, &[query::ANY_TAG]));
        // No standard query needs an experiment.
        assert!(
            catalog()
                .iter()
                .all(|d| d.shape().experiments == ExperimentMask::empty())
        );
    }

    #[test]
    fn queries_of_later_releases() {
        let fuse = get(query::FUSE_TIME);
        assert_eq!(fuse.shape().first_release, Some(Version::new(1, 26, 30)));
        assert_eq!(fuse.sets(), QuerySetMask::DEFAULT);
        assert_eq!(fuse.shape().returns, ReturnType::FLOAT);
        assert_eq!(fuse.args(), Arity::ANY);
        assert_eq!(
            (
                fuse.shape().ranges.as_slice()[0].first(),
                fuse.shape().ranges.as_slice()[0].last()
            ),
            (MolangVersion::V0, MolangVersion::V13)
        );
        assert_eq!(fuse.shape().reads, Reads::ACTOR);
        assert_eq!(fuse.shape().default_return, DefaultReturn::Float0);
        for (name, set) in [
            ("query.has_all_biome_tags", QuerySetMask::WORLD_GEN),
            ("query.has_any_biome_tags", QuerySetMask::WORLD_GEN),
            ("query.head_is_in_water", QuerySetMask::DEFAULT),
        ] {
            assert_eq!(
                get(name).shape().first_release,
                Some(Version::new(1, 26, 50)),
                "{name}"
            );
            assert_eq!(get(name).sets(), set, "{name}");
            assert_eq!(get(name).shape().returns, ReturnType::BOOL, "{name}");
        }
        let later: Vec<&str> = catalog()
            .iter()
            .filter(|d| d.shape().first_release.is_some())
            .map(QueryDecl::name)
            .collect();
        assert_eq!(
            later,
            [
                "query.fuse_time",
                "query.has_all_biome_tags",
                "query.has_any_biome_tags",
                "query.head_is_in_water"
            ]
        );
        // Only `query.is_on_screen` is not `on_dedicated_server`.
        let absent: Vec<&str> = catalog()
            .iter()
            .filter(|d| !d.on_dedicated_server())
            .map(QueryDecl::name)
            .collect();
        assert_eq!(absent, ["query.is_on_screen"]);
    }

    /// The 1.26.36 server catalogue lacks `query.head_is_in_water` and the two biome-tag queries
    /// (1.26.50); it never resolves `query.is_on_screen` and has `query.fuse_time` (1.26.30).
    #[cfg(feature = "compiler")]
    #[test]
    fn the_catalogue_as_of_a_release() {
        use molangx::compile::{CompileFailure, CompileOptions, compile};
        let bds = molangx::stdlib::queries_at(Side::Server, &Version::new(1, 26, 36));
        let missing: Vec<&str> = catalog()
            .iter()
            .map(QueryDecl::name)
            .filter(|name| !bds.contains(name))
            .collect();
        assert_eq!(
            missing,
            [
                "query.has_all_biome_tags",
                "query.has_any_biome_tags",
                "query.head_is_in_water"
            ]
        );
        let options = CompileOptions::new(bds, MolangVersion::LATEST);
        assert_eq!(compile("q.fuse_time", &options).failure(), None);
        assert_eq!(
            compile("q.is_on_screen", &options).failure(),
            Some(CompileFailure::Rejected)
        );
        assert_eq!(
            compile("q.head_is_in_water", &options).failure(),
            Some(CompileFailure::Rejected)
        );
        // The bound is inclusive; an earlier client has none of the later queries.
        assert!(
            molangx::stdlib::queries_at(Side::Client, &Version::new(1, 26, 30))
                .contains(query::FUSE_TIME)
        );
        assert!(
            !molangx::stdlib::queries_at(Side::Client, &Version::new(1, 26, 29))
                .contains(query::FUSE_TIME)
        );
        assert_eq!(
            molangx::stdlib::queries_at(Side::Client, &Version::new(1, 26, 0)).len(),
            315
        );
        assert!(Version::new(1, 26, 9) < Version::new(1, 26, 30));
        assert_eq!(Version::new(1, 26, 50).to_string(), "1.26.50");
    }

    #[test]
    fn no_subject_defaults() {
        let exceptions = BTreeMap::from([
            ("query.armor_color_slot", DefaultReturn::Float1),
            (
                "query.time_since_last_vibration_detection",
                DefaultReturn::FloatNeg1,
            ),
            (
                "query.ticks_since_last_kinetic_weapon_hit",
                DefaultReturn::FloatNeg1,
            ),
            ("query.owner_identifier", DefaultReturn::EmptyString),
            ("query.get_equipped_item_name", DefaultReturn::EmptyString),
            ("query.combine_entities", DefaultReturn::EmptyActorArray),
            ("query.spellcolor", DefaultReturn::StructRgba0),
        ]);
        for decl in catalog() {
            let expected = exceptions
                .get(decl.name())
                .copied()
                .unwrap_or(DefaultReturn::Float0);
            assert_eq!(decl.shape().default_return, expected, "{}", decl.name());
        }
        assert_eq!(DefaultReturn::Float1.as_f32(), 1.0);
        assert_eq!(DefaultReturn::FloatNeg1.as_f32(), -1.0);
        assert_eq!(DefaultReturn::EmptyString.as_f32(), 0.0);
    }

    #[test]
    fn sides_and_subjects() {
        let server: Vec<&str> = catalog()
            .iter()
            .filter(|d| d.shape().side == QuerySide::Server)
            .map(QueryDecl::name)
            .collect();
        assert_eq!(
            server,
            [
                "query.anger_level",
                "query.scoreboard",
                "query.server_memory_tier"
            ]
        );
        for name in [
            "client_max_render_distance",
            "client_memory_tier",
            "get_pack_setting",
            "graphics_mode_is_any",
            "heartbeat_phase",
            "is_pack_setting_enabled",
            "is_pack_setting_selected",
            "last_input_mode_is_any",
            "surface_particle_color",
            "time_since_last_vibration_detection",
            "touch_only_affects_hotbar",
            "target_x_rotation",
            "bone_aabb",
        ] {
            assert_eq!(
                get(&format!("query.{name}")).shape().side,
                QuerySide::CLIENT,
                "{name}"
            );
        }
        for name in [
            "query.surface_particle_texture_coordinate",
            "query.surface_particle_texture_size",
        ] {
            assert_eq!(get(name).shape().side, QuerySide::BOTH, "{name}");
        }
        assert_eq!(
            get(query::TARGET_X_ROTATION).shape().reads,
            Reads::CLIENT_STATE
        );
        assert_eq!(get(query::IS_FIRST_PERSON).shape().reads, Reads::RENDER);
        assert_eq!(get(query::NOISE).shape().reads, Reads::WORLD_GEN);
        assert_eq!(get(query::APPROX_EQ).shape().reads, Reads::empty());
        assert!(get(query::APPROX_EQ).shape().reads.is_empty());
        assert_eq!(
            get(query::SURFACE_PARTICLE_COLOR).shape().reads,
            Reads::ACTOR.union(Reads::BLOCK)
        );
        assert_eq!(
            get(query::DISTANCE_FROM_CAMERA).shape().reads,
            Reads::CAMERA
        );
        assert_eq!(get(query::BLOCK_FACE).shape().reads, Reads::VARIABLES);
        assert!(
            catalog()
                .iter()
                .filter(|d| d.shape().reads.contains(Reads::ACTOR))
                .count()
                > 200
        );
    }

    #[test]
    fn op_table_spot_checks() {
        // Tokens and friendly names verbatim, as `Expression uses operation {} …` prints them.
        use ExpressionOp as Op;
        let rows = [
            (Op::LeftBrace, 0, Some("{"), "Left Brace '{'", 1, None),
            (Op::Negate, 6, Some("-"), "Negate '-'", 1, Some(1)),
            (Op::Add, 9, Some("+"), "Add '+'", 2, None),
            (
                Op::Atan2,
                13,
                Some("math.atan2"),
                "atan2 'math.atan2'",
                2,
                Some(2),
            ),
            (Op::Div, 20, Some("/"), "Divide '/'", 2, Some(2)),
            (
                Op::Random,
                33,
                Some("math.random"),
                "Random 'math.random'",
                2,
                Some(2),
            ),
            (
                Op::RandomInt,
                34,
                Some("math.random_integer"),
                "Random Integer 'math.random_integer'",
                2,
                Some(2),
            ),
            (
                Op::QueryFunction,
                40,
                Some("query."),
                "Query Function 'query.' or 'q.'",
                0,
                None,
            ),
            (Op::StringLiteral, 46, Some("'"), "String '''", 0, Some(0)),
            (Op::LogicalOr, 56, Some("||"), "Logical Or '||'", 2, Some(2)),
            (
                Op::Conditional,
                59,
                Some("?"),
                "Conditional '?'",
                2,
                Some(3),
            ),
            (Op::Float, 61, None, "Float", 0, Some(0)),
            (Op::Pi, 62, Some("math.pi"), "Pi", 0, Some(0)),
            (Op::Array, 63, Some("[]"), "Array '[]'", 1, Some(1)),
            (Op::Geometry, 64, None, "Geometry reference", 0, Some(0)),
            (Op::Assignment, 71, Some("="), "Assignment '='", 2, Some(2)),
            (Op::Pointer, 72, Some("->"), "Pointer '->'", 2, Some(2)),
            (Op::Semicolon, 73, Some(";"), "Semicolon ';'", 1, None),
            (Op::ExpressionArray, 77, None, "Expression array", 1, None),
            (
                Op::InverseLerp,
                78,
                Some("math.inverse_lerp"),
                "Inverse Lerp 'math.inverse_lerp'",
                3,
                Some(3),
            ),
            (
                Op::EaseInOutElastic,
                108,
                Some("math.ease_in_out_elastic"),
                "Ease In Out Elastic 'math.ease_in_out_elastic'",
                3,
                Some(3),
            ),
            (Op::HostMath, 109, None, "Host Math Function", 1, Some(8)),
            (
                Op::HostMathVolatile,
                110,
                None,
                "Volatile Host Math Function",
                1,
                Some(8),
            ),
        ];
        for (op, ordinal, token, friendly, min, max) in rows {
            assert_eq!(
                (
                    op.ordinal(),
                    op.token(),
                    op.friendly_name(),
                    op.min_children(),
                    op.max_children()
                ),
                (ordinal, token, friendly, min, max),
                "{op:?}"
            );
        }
        assert_eq!(Op::QueryFunction.meta().aliases, ["query.", "q."]);
        assert_eq!(Op::EntityVariable.meta().aliases, ["variable.", "v."]);
        let resources: Vec<Op> = Op::all()
            .iter()
            .copied()
            .filter(|op| op.is_resource_reference())
            .collect();
        assert_eq!(
            resources,
            [
                Op::GeometryVariable,
                Op::MaterialVariable,
                Op::TextureVariable
            ]
        );
    }

    #[test]
    fn op_set() {
        // All 111 bits by default.
        assert_eq!(OpSet::all().len(), 111);
        assert_eq!(OpSet::default(), OpSet::all());
        assert_eq!(OpSet::all().words(), [u64::MAX, (1 << 47) - 1]);
        assert!(
            ExpressionOp::all()
                .iter()
                .all(|&op| OpSet::all().contains(op))
        );
        assert!(OpSet::empty().is_empty() && OpSet::empty().iter().next().is_none());
        assert_eq!(OpSet::from_words([u64::MAX; 2]), OpSet::all());
        assert_eq!(OpSet::all().iter().count(), 111);

        // Removing assignments clears only bit 71; removing random draws as well also clears 33, 34
        // and 110.
        let property_default = OpSet::all().without_assignments();
        let removed: Vec<u8> = OpSet::all()
            .iter()
            .filter(|&op| !property_default.contains(op))
            .map(ExpressionOp::ordinal)
            .collect();
        assert_eq!(removed, [71]);
        let block_condition = OpSet::all().without_assignments_or_random();
        let removed: Vec<u8> = OpSet::all()
            .iter()
            .filter(|&op| !block_condition.contains(op))
            .map(ExpressionOp::ordinal)
            .collect();
        assert_eq!(removed, [33, 34, 71, 110]);
        assert!(
            block_condition.contains(ExpressionOp::DieRoll)
                && block_condition.contains(ExpressionOp::DieRollInt)
        );
        assert_eq!(block_condition.len(), 107);
        // The side-effect ops are exactly the flagged ones.
        let flagged: Vec<u8> = ExpressionOp::all()
            .iter()
            .filter(|op| op.meta().flags.contains(OpFlags::SIDE_EFFECT))
            .map(|op| op.ordinal())
            .collect();
        assert_eq!(flagged, [33, 34, 71, 110]);
        let only_add = OpSet::empty().with(ExpressionOp::Add);
        assert!(only_add.contains(ExpressionOp::Add) && only_add.len() == 1);
        assert!(only_add.without(ExpressionOp::Add).is_empty());
        assert!(OpSet::all().without(ExpressionOp::EaseInOutElastic).len() == 110);
    }

    #[test]
    fn math_functions() {
        // 61 ops, `math.pi` included, each one an op of the op table.
        assert_eq!(MathFn::COUNT, 61);
        assert_eq!(MATH_META.len(), 61);
        assert_eq!(
            ExpressionOp::all()
                .iter()
                .filter(|op| op.is_math_function())
                .count(),
            61
        );
        for (index, (&function, meta)) in MathFn::all().iter().zip(MATH_META.iter()).enumerate() {
            assert_eq!(function as usize, index);
            assert_eq!(meta.function, function);
            assert_eq!(function.meta(), meta);
            let op = function.op();
            assert!(op.is_math_function());
            assert_eq!(op.math_fn(), Some(function));
            assert_eq!(Some(function.token()), op.token());
            assert_eq!(function.friendly_name(), op.friendly_name());
            assert_eq!(
                (meta.min_args, Some(meta.max_args)),
                (op.min_children(), op.max_children())
            );
            assert_eq!(MathFn::from_token(function.token()), Some(function));
            // Math functions are not queries.
            assert!(!catalog().contains(function.token()));
        }
        let order: Vec<u8> = MathFn::all().iter().map(|f| f.op().ordinal()).collect();
        assert!(
            order.windows(2).all(|w| w[0] < w[1]),
            "dense in op-index order"
        );
        assert_eq!(MathFn::from_token("math.pi"), Some(MathFn::Pi));
        assert_eq!(
            (MathFn::Pi.meta().min_args, MathFn::Pi.meta().max_args),
            (0, 0)
        );
        assert_eq!(MathFn::from_token("math.clamp").unwrap().meta().max_args, 3);
        assert_eq!(
            MathFn::from_token("math.mod").unwrap().op(),
            ExpressionOp::Mod
        );
        assert_eq!(
            MathFn::from_token("math.random").unwrap().op(),
            ExpressionOp::Random
        );
        assert_eq!(MathFn::from_token("Math.abs"), None);
        assert_eq!(MathFn::from_token("math."), None);
        assert_eq!(ExpressionOp::Add.math_fn(), None);
        let eases = MathFn::all()
            .iter()
            .filter(|f| f.token().starts_with("math.ease_"))
            .count();
        assert_eq!(eases, 30);
    }

    #[test]
    fn query_set_mask() {
        assert_eq!("default".parse::<QuerySetMask>(), Ok(QuerySetMask::DEFAULT));
        assert_eq!("tags".parse::<QuerySetMask>(), Ok(QuerySetMask::TAGS));
        assert_eq!(
            "world_gen".parse::<QuerySetMask>(),
            Ok(QuerySetMask::WORLD_GEN)
        );
        assert_eq!("worldgen".parse::<QuerySetMask>(), Err(ParseQuerySetError));
        assert_eq!("test".parse::<QuerySetMask>(), Err(ParseQuerySetError));
        assert!(QuerySetMask::empty().is_empty());
        let both = QuerySetMask::DEFAULT | QuerySetMask::TAGS;
        assert!(both.contains(QuerySetMask::TAGS) && !both.contains(QuerySetMask::WORLD_GEN));
        assert_eq!(both.name(), None);
        assert_eq!(QuerySetMask::WORLD_GEN.name(), Some("world_gen"));
        assert_eq!(format!("{both:?}"), "{default, tags}");
        let e = Experiment::new(5).unwrap();
        assert!(ExperimentMask::empty().with(e).contains(e));
        assert!(ExperimentMask::empty().contains(ExperimentMask::empty()));
        assert!(!ExperimentMask::empty().contains(e));
        assert_eq!(Experiment::new(64), None);
    }

    /// `QuerySetMask::BUILTIN` is the three built-in sets and the standard catalogue holds no test
    /// query, whatever the features.
    #[test]
    fn the_fuzz_feature_changes_no_public_value() {
        assert_eq!(QuerySetMask::BUILTIN.bits(), 0b111);
        assert_eq!(
            QuerySetMask::BUILTIN,
            QuerySetMask::DEFAULT | QuerySetMask::TAGS | QuerySetMask::WORLD_GEN
        );
        assert_eq!(catalog().len(), 319);
        for name in [
            "query.get_name_test",
            "query.sum_test",
            "query.experimental_test",
            "query.valid_always",
        ] {
            assert!(!catalog().contains(name), "{name} is not a standard query");
        }
        let host = QuerySetMask::host(0).expect("a host set");
        assert!(
            !host.intersects(QuerySetMask::BUILTIN)
                && catalog().iter().all(|d| !d.sets().intersects(host))
        );
    }
}

mod api_surface {
    use molangx::catalog::{
        Arity, DefaultReturn, QueryCatalog, QueryDecl, QuerySetMask, QueryShape, QuerySide, Reads,
        ReturnType, Side, VersionRange,
    };

    use molangx::version::{EngineVersion, MolangVersion};

    #[test]
    fn molang_version_enum() {
        assert_eq!(size_of::<MolangVersion>(), 2);
        let all: Vec<MolangVersion> = (i16::MIN..=i16::MAX)
            .filter_map(MolangVersion::from_i16)
            .collect();
        assert_eq!(all.len(), 15);
        for (v, raw) in all.iter().zip(-1..=13) {
            assert_eq!(v.as_i16(), raw);
            assert_eq!(*v as i16, raw);
        }
        assert_eq!(MolangVersion::LATEST, MolangVersion::V13);
    }

    #[test]
    fn molang_version_conversions() {
        let owned = |major, minor, patch| super::ev(major, minor, patch);
        let (v1_16, v1_17_30, v1_21_100) = (owned(1, 16, 0), owned(1, 17, 30), owned(1, 21, 100));
        let v = |version: &'static str| match version {
            "1.16.0" => Some(&v1_16),
            "1.17.30" => Some(&v1_17_30),
            _ => Some(&v1_21_100),
        };
        assert_eq!(MolangVersion::from(&v1_16), MolangVersion::V0);
        assert_eq!(MolangVersion::from(&v1_17_30), MolangVersion::V2);
        assert_eq!(MolangVersion::from(&v1_21_100), MolangVersion::LATEST);
        // A pre-release sorts below its release, the any-version above everything.
        let beta: EngineVersion = "1.17.30-beta".parse().unwrap();
        assert_eq!(MolangVersion::from(&beta), MolangVersion::V1);
        assert_eq!(
            MolangVersion::from(&EngineVersion::Any),
            MolangVersion::LATEST
        );

        assert_eq!(MolangVersion::V2.first_engine_version(), v("1.17.30"));
        assert_eq!(MolangVersion::V0.first_engine_version(), None);
        assert_eq!(MolangVersion::from_i16(-1), Some(MolangVersion::Invalid));
        assert_eq!(MolangVersion::from_i16(14), None);
        assert_eq!(MolangVersion::from_i16(-2), None);
        assert_eq!(
            molangx::version::RawVersion(14).effective(),
            MolangVersion::LATEST
        );
        assert_eq!(
            molangx::version::RawVersion(-9).effective(),
            MolangVersion::Invalid
        );
        assert_eq!(
            molangx::version::RawVersion(5).effective(),
            MolangVersion::V5
        );
    }

    #[test]
    fn version_gates() {
        type Gate = fn(MolangVersion) -> bool;
        let gates: [(Gate, i16); 5] = [
            (MolangVersion::reports_expression_errors, 3),
            (MolangVersion::reports_unexpected_operators, 4),
            (MolangVersion::right_assoc_ternary, 5),
            (MolangVersion::c_like_logic_precedence, 6),
            (MolangVersion::signed_division_fix, 7),
        ];
        for (gate, from) in gates {
            for raw in -1..=13 {
                let v = MolangVersion::from_i16(raw).expect("a version");
                assert_eq!(gate(v), raw >= from, "gate from {from} at {raw}");
            }
        }
    }

    #[test]
    fn query_decl_fields() {
        let decl: &QueryDecl = molangx::stdlib::queries(Side::Client)
            .get("query.item_remaining_use_duration")
            .expect("a query");
        let name: &str = decl.name();
        let args: Arity = decl.args();
        let (min, max): (u8, Option<u8>) = (args.min(), args.max());
        let returns: ReturnType = decl.shape().returns;
        let ranges: &[VersionRange] = decl.shape().ranges.as_slice();
        let reads: Reads = decl.shape().reads;
        let side: QuerySide = decl.shape().side;
        let default: DefaultReturn = decl.shape().default_return;
        assert_eq!(name, "query.item_remaining_use_duration");
        assert!(max.is_none_or(|max| max >= min));
        assert_eq!(returns, ReturnType::FLOAT);
        let windows: Vec<(i16, i16, QuerySetMask)> = ranges
            .iter()
            .map(|r| (r.first().as_i16(), r.last().as_i16(), r.sets()))
            .collect();
        assert_eq!(
            windows,
            [
                (0, 1, QuerySetMask::DEFAULT),
                (2, 13, QuerySetMask::DEFAULT)
            ]
        );
        assert!(decl.shape().experiments == molangx::version::ExperimentMask::empty());
        assert!(reads.contains(Reads::ACTOR));
        assert_eq!(side, QuerySide::BOTH);
        assert_eq!(default, DefaultReturn::Float0);
        assert!(decl.on_dedicated_server());
        assert_eq!(decl.shape().first_release, None);
    }

    /// A declaration is a name and a shape of plain fields; the checks that span fields run in
    /// `QueryDecl::new`, the others in the field types. An ill-formed declaration does not exist,
    /// so no catalogue can hold one.
    #[test]
    fn a_host_declares_queries_from_a_shape() {
        use std::ops::RangeInclusive;

        use molangx::catalog::{CatalogError, DeclError, EmptyArity, VersionRanges};
        let shape = QueryShape {
            args: Arity::between(0, 2),
            returns: ReturnType::BOOL,
            reads: Reads::ACTOR,
            ..QueryShape::DEFAULT
        };
        let catalog = molangx::stdlib::queries(Side::Client)
            .extended([QueryDecl::new("query.my_thing", shape.clone()).expect("well-formed")])
            .expect("a new name");
        let decl = catalog.get("query.my_thing").expect("declared");
        assert_eq!(
            (decl.shape().args, decl.shape().returns, decl.shape()),
            (Arity::between(0, 2), ReturnType::BOOL, &shape)
        );
        assert_eq!(decl.args(), Arity::between(0, 2));
        // An ill-formed or duplicate declaration never enters a catalogue.
        assert_eq!(
            catalog.extended([decl.clone()]).err(),
            Some(CatalogError::Duplicate("query.my_thing".into()))
        );
        // `overriding` replaces a declaration; adding a name is `extended`'s job.
        let wider = QueryDecl::new(
            "query.my_thing",
            QueryShape {
                args: Arity::ANY,
                ..shape
            },
        )
        .expect("well-formed");
        let overridden: QueryCatalog = catalog.overriding([wider]).expect("a declared name");
        assert_eq!(
            overridden
                .get("query.my_thing")
                .map(|decl| decl.shape().args),
            Some(Arity::ANY)
        );
        assert_eq!(
            catalog
                .overriding([
                    QueryDecl::new("query.other", QueryShape::DEFAULT).expect("well-formed")
                ])
                .err(),
            Some(CatalogError::Undeclared("query.other".into()))
        );
        assert!(matches!(
            QueryDecl::new("Query.My_Thing", QueryShape::DEFAULT),
            Err(DeclError::Name(_))
        ));
        assert_eq!(Arity::checked_between(3, 1), None);
        assert_eq!(Arity::from(2..).to_string(), "at least 2");
        assert_eq!(
            QueryCatalog::new(Side::Server, [decl.clone()]).map(|c| c.len()),
            Ok(1)
        );
        assert_eq!(Arity::try_from(RangeInclusive::new(3, 1)), Err(EmptyArity));
        assert_eq!(
            (
                Arity::ANY,
                Arity::exactly(1).max(),
                Arity::at_least(2).min()
            ),
            (Arity::default(), Some(1), 2)
        );
        assert_eq!(VersionRanges::new([]), Err(DeclError::NoVersionRange));
        let tags = VersionRanges::ALWAYS.in_sets(QuerySetMask::TAGS);
        assert_eq!(
            tags.as_slice(),
            [VersionRange::ALWAYS.in_sets(QuerySetMask::TAGS)]
        );
        let fits = QueryShape {
            returns: ReturnType::STRING,
            default_return: DefaultReturn::Float1,
            ..QueryShape::DEFAULT
        };
        assert!(matches!(
            QueryDecl::new("query.x", fits),
            Err(DeclError::DefaultNotReturned { .. })
        ));
        let undefined = QueryShape {
            reads: Reads::from_bits_retain(1 << 12),
            ..QueryShape::DEFAULT
        };
        assert!(matches!(
            QueryDecl::new("query.x", undefined),
            Err(DeclError::UndefinedReads(_))
        ));
    }

    /// Every mask type: `all()`, `contains(Self)` as the subset test (`OpSet` and `ExperimentMask`
    /// also take one op or experiment), `intersects`, `union`, `insert`, `|` and `|=`. Every type
    /// but `ReturnType`, which is never empty: `empty()`, `is_empty`, `intersection`,
    /// `difference`, `remove`, `&`, `&=`, `-` and `-=`.
    #[test]
    fn the_mask_types_share_one_vocabulary() {
        use molangx::ops::{ExpressionOp, OpFlags, OpSet};
        use molangx::version::{Experiment, ExperimentMask};
        macro_rules! never_empty {
            ($t:ty, $a:expr, $b:expr) => {{
                let (a, b): ($t, $t) = ($a, $b);
                assert_eq!(a | b, a.union(b));
                let mut both = a;
                both |= b;
                assert_eq!(both, a | b);
                let mut inserted = a;
                inserted.insert(b);
                assert_eq!(inserted, a | b);
                assert!((a | b).contains(a) && (a | b).contains(b) && !a.contains(a | b));
                assert!((a | b).intersects(a) && !a.intersects(b));
                assert!(<$t>::all().contains(a | b) && <$t>::all().intersects(a));
            }};
        }
        macro_rules! mask {
            ($t:ty, $a:expr, $b:expr) => {{
                never_empty!($t, $a, $b);
                let (a, b): ($t, $t) = ($a, $b);
                assert!(a.contains(<$t>::empty()) && <$t>::empty().contains(<$t>::empty()));
                assert!(!a.intersects(<$t>::empty()));
                assert!(<$t>::empty().is_empty() && !a.is_empty() && !<$t>::all().is_empty());
                assert_eq!(
                    ((a | b) & a, (a | b).intersection(a), a & b),
                    (a, a, <$t>::empty())
                );
                assert_eq!(
                    (
                        (a | b) - a,
                        (a | b).difference(a),
                        <$t>::all() - <$t>::all()
                    ),
                    (b, b, <$t>::empty())
                );
                let (mut and, mut sub, mut removed) = (a | b, a | b, a | b);
                and &= a;
                sub -= a;
                removed.remove(a);
                assert_eq!((and, sub, removed), (a, b, b));
            }};
        }
        mask!(OpFlags, OpFlags::MATH_FUNCTION, OpFlags::SIDE_EFFECT);
        mask!(
            OpSet,
            OpSet::from(ExpressionOp::Add),
            OpSet::from(ExpressionOp::Mul)
        );
        mask!(QuerySetMask, QuerySetMask::DEFAULT, QuerySetMask::TAGS);
        mask!(
            ExperimentMask,
            Experiment::new(1).unwrap().into(),
            Experiment::new(2).unwrap().into()
        );
        mask!(Reads, Reads::ACTOR, Reads::ITEM);
        #[cfg(feature = "compiler")]
        mask!(
            molangx::compile::ProgramFlags,
            molangx::compile::ProgramFlags::FLOAT_ONLY,
            molangx::compile::ProgramFlags::CONSTANT
        );
        never_empty!(ReturnType, ReturnType::FLOAT, ReturnType::STRING);
        assert!(
            OpSet::all().contains(ExpressionOp::Add) && !OpSet::empty().contains(ExpressionOp::Add)
        );
        assert_eq!(
            OpSet::all()
                .without(ExpressionOp::Add)
                .with(ExpressionOp::Add),
            OpSet::all()
        );
        let experiments: ExperimentMask = [1, 2].into_iter().filter_map(Experiment::new).collect();
        assert!(
            experiments.contains(Experiment::new(2).unwrap())
                && !experiments.contains(Experiment::new(3).unwrap())
        );
        assert_eq!(
            experiments.without(Experiment::new(2).unwrap()),
            Experiment::new(1).unwrap().into()
        );
        assert_eq!(QuerySetMask::default(), QuerySetMask::DEFAULT);
    }
}

#[cfg(feature = "compiler")]
mod api_compiler {
    use molangx::diag::DiagCode;

    /// The limits live with the code that enforces them.
    #[test]
    fn the_limit_paths() {
        assert_eq!(molangx::compile::MAX_DEPTH, 255);
        assert_eq!(molangx::compile::MAX_SOURCE_LEN, 65_536);
        assert_eq!(molangx::compile::MAX_DIAGNOSTICS, 256);
        #[cfg(feature = "vm")]
        assert_eq!(molangx::vm::EvalLimits::DEFAULT_LOOP_ITERATIONS, 1_024);
    }

    /// The float behaviour is the build's: `ARCH` names it, `arith` holds its primitives, and the
    /// scalar functions take no behaviour argument.
    #[test]
    fn the_numeric_paths() {
        use molangx::numeric::{self, ARCH, Arch, PostOp, arith};
        let arch: Arch = ARCH;
        assert_eq!(
            arch,
            if cfg!(target_arch = "aarch64") {
                Arch::Arm64
            } else {
                Arch::X86_64
            }
        );
        assert_ne!(Arch::X86_64, Arch::Arm64);
        let binary: [fn(f32, f32) -> f32; 4] = [arith::add, arith::mul, arith::max, arith::min];
        let fused: [fn(f32, f32, f32) -> f32; 4] = [
            arith::mul_add,
            arith::mul_sub,
            arith::neg_mul_add,
            arith::neg_mul_sub,
        ];
        let to_int: fn(f32) -> i32 = arith::to_int;
        assert_eq!(binary.map(|f| f(2.0, 3.0)), [5.0, 6.0, 3.0, 2.0]);
        assert_eq!(fused.map(|f| f(2.0, 3.0, 1.0)), [7.0, -5.0, -7.0, 5.0]);
        assert_eq!(to_int(2.9), 2);
        let compare: [fn(f32, f32) -> bool; 4] =
            [numeric::lt, numeric::le, numeric::gt, numeric::ge];
        assert_eq!(compare.map(|f| f(1.0, 2.0)), [true, true, false, false]);
        let add: fn(f32, f32, PostOp) -> f32 = numeric::add;
        let guard: fn(bool, f32) -> Option<f32> = numeric::div_guard;
        let index: fn(f32, usize) -> Option<usize> = numeric::array_index;
        assert_eq!(
            (
                add(1.0, 2.0, PostOp::IDENTITY),
                guard(true, 2.0),
                index(1.0, 3)
            ),
            (3.0, Some(2.0), Some(1))
        );
        assert_eq!(PostOp::new(2.0, 1.0).apply(3.0), 7.0);
    }

    #[test]
    fn diag_codes() {
        let codes = [
            DiagCode::Syntax,
            DiagCode::UnknownQuery,
            DiagCode::QueryArity,
            DiagCode::QueryExperiment,
            DiagCode::QueryDeprecated,
            DiagCode::QueryClientOnly,
            DiagCode::InvalidOperation,
            DiagCode::StringMisuse,
            DiagCode::InvalidAssignment,
            DiagCode::StatementForm,
            DiagCode::DepthLimit,
            DiagCode::SourceTooLong,
            DiagCode::InvalidVersion,
        ];
        let distinct: std::collections::BTreeSet<String> =
            codes.iter().map(|c| format!("{c:?}")).collect();
        assert_eq!(distinct.len(), 13);
        // Every language message maps to one of them.
        for message in molangx::diag::LanguageMessage::ALL {
            assert!(codes.contains(&message.code()), "{message:?}");
        }
    }
}

#[cfg(feature = "vm")]
mod api_vm {
    use std::cell::Cell;

    use std::fmt::Debug;

    use molangx::vm::{Host, HostAccess, NoHost, Subjects, Value, ValueKind};

    #[test]
    fn value_arms() {
        fn kind(value: &Value<NoHost>) -> ValueKind {
            match value {
                Value::Float(_) => ValueKind::Float,
                Value::Hash(_) => ValueKind::Hash,
                Value::Actor(_) => ValueKind::Actor,
                Value::Item(_) => ValueKind::Item,
                Value::ActorArray(_) => ValueKind::ActorArray,
                Value::Struct(_) => ValueKind::Struct,
                Value::Matrix(_) => ValueKind::Matrix,
                Value::Resource(_) => ValueKind::Resource,
                // `Value` is `#[non_exhaustive]`: a new arm lands here and needs its own row.
                _ => unreachable!("a Value arm this test does not know"),
            }
        }
        assert_eq!(kind(&Value::Float(1.0)), Value::<NoHost>::Float(1.0).kind());
        assert_eq!(kind(&Value::string("a")), ValueKind::Hash);
    }

    #[test]
    fn host_handle_bounds() {
        fn bounds<H: Host>()
        where
            H::ActorRef: Copy + Eq + Debug,
            H::ItemRef: Copy + Eq + Debug,
            H::BlockRef: Copy + Eq + Debug,
        {
        }
        bounds::<NoHost>();
        bounds::<Farm>();
    }

    /// A host whose world counts the calls the evaluator makes.
    #[derive(Debug)]
    struct Farm;

    impl Host for Farm {
        type ActorRef = u32;
        type ItemRef = u8;
        type BlockRef = (i32, i32, i32);
        type Access<'w> = World;
    }

    struct World {
        resolved: Cell<u32>,
        subjects_of: Cell<u32>,
    }

    impl HostAccess<Farm> for World {
        fn resolve_actor(&self, _from: &Subjects<Farm>, actor: u32) -> Option<u32> {
            self.resolved.set(self.resolved.get() + 1);
            (actor < 10).then_some(actor)
        }

        fn subjects_of(&self, actor: u32) -> Subjects<Farm> {
            self.subjects_of.set(self.subjects_of.get() + 1);
            Subjects::actor(actor)
        }
    }
}

mod registry_conformance {
    use molangx::catalog::{
        Arity, DefaultReturn, QueryDecl, QuerySetMask, QuerySide, Reads, ReturnType, Side,
    };
    use molangx::stdlib::query;

    fn get(name: &str) -> &'static QueryDecl {
        molangx::stdlib::queries(Side::Client).get(name).unwrap()
    }

    #[test]
    fn query_decl_records_every_field() {
        let decl = get(query::CAPE_FLAP_AMOUNT);
        assert_eq!(decl.name(), "query.cape_flap_amount");
        assert_eq!(decl.args(), Arity::ANY);
        assert_eq!(decl.shape().returns, ReturnType::FLOAT);
        let ranges: Vec<(i16, i16, QuerySetMask)> = decl
            .shape()
            .ranges
            .as_slice()
            .iter()
            .map(|r| (r.first().as_i16(), r.last().as_i16(), r.sets()))
            .collect();
        assert_eq!(
            ranges,
            [
                (0, 7, QuerySetMask::DEFAULT),
                (8, 13, QuerySetMask::DEFAULT)
            ]
        );
        assert!(decl.shape().experiments == molangx::version::ExperimentMask::empty());
        assert!(decl.shape().reads.contains(Reads::ACTOR));
        assert_eq!(decl.shape().default_return, DefaultReturn::Float0);
        assert_eq!(decl.shape().side, QuerySide::BOTH);
        assert_eq!(
            get(query::CLIENT_MEMORY_TIER).shape().side,
            QuerySide::CLIENT
        );
        for decl in molangx::stdlib::queries(Side::Client) {
            assert!(decl.name().starts_with("query."), "{}", decl.name());
            assert!(
                !decl.shape().ranges.as_slice().is_empty(),
                "{}",
                decl.name()
            );
            let args = decl.args();
            assert!(
                args.max().is_none_or(|max| max >= args.min()),
                "{}",
                decl.name()
            );
        }
    }

    #[test]
    fn scoreboard_is_a_server_query() {
        assert_eq!(get(query::SCOREBOARD).shape().side, QuerySide::Server);
        assert_ne!(get(query::IS_LOCAL_PLAYER).shape().side, QuerySide::Server);
    }

    #[test]
    fn the_catalogue_has_no_short_names() {
        let catalog = molangx::stdlib::queries(Side::Client);
        assert!(catalog.get("q.is_baby").is_none());
        assert!(catalog.iter().all(|decl| !decl.name().starts_with("q.")));
        assert_eq!(
            catalog.get("query.is_baby").map(QueryDecl::name),
            Some(query::IS_BABY)
        );
    }
}

mod opset_conformance {
    use molangx::ops::ExpressionOp;

    #[test]
    fn resource_variable_ops() {
        let resources: Vec<u8> = ExpressionOp::all()
            .iter()
            .filter(|op| op.is_resource_reference())
            .map(|op| op.ordinal())
            .collect();
        assert_eq!(resources, [47, 48, 49]);
        assert_eq!(
            [47, 48, 49].map(|n| ExpressionOp::from_ordinal(n).and_then(ExpressionOp::token)),
            [Some("geometry."), Some("material."), Some("texture.")]
        );
    }
}
