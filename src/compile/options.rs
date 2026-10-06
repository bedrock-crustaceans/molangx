//! What a caller configures: the catalogue, the deviations and the compile options.

#[cfg(feature = "stdlib")]
use crate::catalog::Side;
use crate::catalog::{MathCatalog, QueryAdmission, QueryCatalog, QuerySetMask};
use crate::json::MolangSource;
use crate::ops::OpSet;
use crate::version::{ExperimentMask, MolangVersion, RawVersion};

/// Behaviour switches: `true` turns one on, `false` off.
#[allow(
    clippy::struct_excessive_bools,
    reason = "one independent switch per deviation: flags, not a state"
)]
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub struct Deviations {
    /// A prefix of `true` / `false` is a complete boolean token and the lexer advances by its
    /// length. Off, it advances four bytes for `true` and five for `false` (`t;` loses its `;`).
    pub true_false_prefix_advance: bool,
    /// Reject a source longer than [`MAX_SOURCE_LEN`](super::MAX_SOURCE_LEN) with
    /// [`DiagCode::SourceTooLong`](crate::diag::DiagCode::SourceTooLong).
    pub source_length_limit: bool,
    /// Report the validation messages of sub-expressions below the root as warnings; off, they are
    /// [`Severity::Error`](crate::diag::Severity::Error). The expression compiles either way.
    pub validate_nested: bool,
    /// Warn ([`DiagCode::QueryArity`](crate::diag::DiagCode::QueryArity)) when a query is called
    /// with an argument count outside its declared range. Such a call still compiles and runs.
    pub query_arity_lint: bool,
    /// Inform ([`DiagCode::QueryClientOnly`](crate::diag::DiagCode::QueryClientOnly)) when a
    /// client-only query is compiled against a [`Side::Server`](crate::catalog::Side::Server)
    /// catalogue, where it still resolves.
    pub query_client_only: bool,
    /// Warn ([`DiagCode::InvalidVersion`](crate::diag::DiagCode::InvalidVersion)) when the raw
    /// version is outside −1..=13, and inform when it is `Invalid` (−1).
    pub object_version_warning: bool,
    /// Keep at most [`MAX_DIAGNOSTICS`](super::MAX_DIAGNOSTICS) diagnostics per compile, then one
    /// [`DiagCode::DiagnosticLimit`](crate::diag::DiagCode::DiagnosticLimit) note with the number
    /// left out. Off, every message is kept. The compile itself is unchanged.
    pub diagnostic_limit: bool,
}

impl Deviations {
    /// The default: [`Deviations::ALL`].
    pub const DEFAULT: Self = Self::ALL;
    /// Every deviation on.
    pub const ALL: Self = Self {
        true_false_prefix_advance: true,
        source_length_limit: true,
        validate_nested: true,
        query_arity_lint: true,
        query_client_only: true,
        object_version_warning: true,
        diagnostic_limit: true,
    };
    /// Every deviation off.
    pub const NONE: Self = Self {
        true_false_prefix_advance: false,
        source_length_limit: false,
        validate_nested: false,
        query_arity_lint: false,
        query_client_only: false,
        object_version_warning: false,
        diagnostic_limit: false,
    };
}

impl Default for Deviations {
    /// [`Deviations::DEFAULT`].
    fn default() -> Self {
        Self::DEFAULT
    }
}

/// How an expression is compiled: query catalogue, version, admitted queries, allowed operations,
/// host math functions and deviations.
///
/// The [`QueryCatalog`] decides which queries exist and whether client-only ones are
/// flagged. The admitted queries ([`CompileOptions::admission`]) are set independently of it: an
/// allow-list admits the queries of the compile's catalogue that it names.
///
/// [`CompileOptions::version`] (parser gates) is derived from
/// [`CompileOptions::raw_version`] (query resolution). For a document field, compile its
/// [`MolangSource`] with [`compile_source`](super::compile_source), which applies the source's
/// version to the field's options.
///
/// Equality is the compile cache's key: a new field must compare unequal whenever it can change a
/// compile's result.
///
/// ```
/// # #[cfg(feature = "stdlib")]
/// # {
/// use molangx::catalog::{QueryAdmission, QueryAllowList, Side};
/// use molangx::compile::{CompileFailure, CompileOptions, compile};
/// use molangx::ops::OpSet;
/// use molangx::stdlib::{self, query};
/// use molangx::version::MolangVersion;
///
/// let list = QueryAllowList::new(stdlib::queries(Side::Server), [query::BLOCK_STATE])?;
/// let block = CompileOptions {
///     admission: QueryAdmission::Only(list),
///     allowed_ops: OpSet::all().without_assignments_or_random(),
///     ..CompileOptions::server(MolangVersion::LATEST)
/// };
/// assert!(compile("q.block_state('facing') == 'west'", &block).is_success());
/// assert_eq!(compile("q.is_baby", &block).failure(), Some(CompileFailure::Rejected));
/// # }
/// # Ok::<(), molangx::catalog::AllowListError>(())
/// ```
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct CompileOptions {
    /// The catalogue queries resolve against. An allow-list in `admission` admits the queries of
    /// this catalogue that it names.
    pub catalog: QueryCatalog,
    /// The raw version queries resolve with. Parser gates use
    /// [`RawVersion::effective`](crate::version::RawVersion::effective) of it; above 13 it gates
    /// like 13 and resolves no query.
    pub raw_version: RawVersion,
    /// Which queries of the catalogue may be called: query sets (by default `default`) or an
    /// allow-list.
    pub admission: QueryAdmission,
    /// The allowed operations (by default all).
    pub allowed_ops: OpSet,
    /// The enabled experiments (by default none). No standard query is experiment-gated.
    pub experiments: ExperimentMask,
    /// The host's own `math.*` functions (by default none), callable like the standard ones.
    pub math: Option<MathCatalog>,
    /// Whether the compiled expression keeps the source text (by default not).
    pub keep_source: bool,
    /// The active deviations (by default [`Deviations::DEFAULT`], every one on).
    pub deviations: Deviations,
}

impl CompileOptions {
    /// Default settings at `version` against `catalog`: the `default` query set, every operation
    /// allowed, no experiments, no host math functions, source not kept, [`Deviations::DEFAULT`].
    pub const fn new(catalog: QueryCatalog, version: MolangVersion) -> Self {
        Self::from_raw_version(catalog, RawVersion(version.as_i16()))
    }

    /// Default settings at a raw version: parser gates use
    /// [`RawVersion::effective`](crate::version::RawVersion::effective), queries resolve with the
    /// raw value.
    pub const fn from_raw_version(catalog: QueryCatalog, raw: RawVersion) -> Self {
        Self {
            catalog,
            raw_version: raw,
            admission: QueryAdmission::Sets(QuerySetMask::DEFAULT),
            allowed_ops: OpSet::all(),
            experiments: ExperimentMask::empty(),
            math: None,
            keep_source: false,
            deviations: Deviations::DEFAULT,
        }
    }

    /// [`CompileOptions::from_raw_version`] at the source's raw version; `None` for a string source
    /// whose context version was never applied.
    pub fn for_source(catalog: QueryCatalog, source: &MolangSource) -> Option<Self> {
        source
            .raw_version()
            .map(|raw| Self::from_raw_version(catalog, raw))
    }

    /// The version parser gates use:
    /// [`RawVersion::effective`](crate::version::RawVersion::effective) of
    /// [`CompileOptions::raw_version`].
    pub const fn version(&self) -> MolangVersion {
        self.raw_version.effective()
    }
}

#[cfg(feature = "stdlib")]
#[cfg_attr(docsrs, doc(cfg(feature = "stdlib")))]
impl CompileOptions {
    /// [`CompileOptions::new`] against the standard library's server queries.
    pub fn server(version: MolangVersion) -> Self {
        Self::new(crate::stdlib::queries(Side::Server).clone(), version)
    }

    /// [`CompileOptions::new`] against the standard library's client queries.
    pub fn client(version: MolangVersion) -> Self {
        Self::new(crate::stdlib::queries(Side::Client).clone(), version)
    }
}

#[cfg(test)]
mod tests {

    use crate::catalog::{
        QueryAdmission, QueryAllowList, QueryCatalog, QueryDecl, QuerySetMask, QueryShape, Side,
    };
    use crate::compile::{CompileOptions, Deviations, test_support::*};
    use crate::ops::{ExpressionOp as Op, OpSet};
    use crate::stdlib::query;
    use crate::version::{Experiment, ExperimentMask};

    use crate::json::MolangSource;
    use crate::version::{MolangVersion, RawVersion};

    fn server() -> &'static QueryCatalog {
        crate::stdlib::queries(Side::Server)
    }

    fn client() -> &'static QueryCatalog {
        crate::stdlib::queries(Side::Client)
    }

    /// The destructuring names every field, so a new deviation fails to compile here.
    #[test]
    fn all_has_every_deviation_on() {
        let Deviations {
            true_false_prefix_advance,
            source_length_limit,
            validate_nested,
            query_arity_lint,
            query_client_only,
            object_version_warning,
            diagnostic_limit,
        } = Deviations::ALL;
        assert!(true_false_prefix_advance);
        assert!(source_length_limit);
        assert!(validate_nested);
        assert!(query_arity_lint);
        assert!(query_client_only);
        assert!(object_version_warning);
        assert!(diagnostic_limit);
    }

    #[test]
    fn none_has_every_deviation_off() {
        let Deviations {
            true_false_prefix_advance,
            source_length_limit,
            validate_nested,
            query_arity_lint,
            query_client_only,
            object_version_warning,
            diagnostic_limit,
        } = Deviations::NONE;
        assert!(!true_false_prefix_advance);
        assert!(!source_length_limit);
        assert!(!validate_nested);
        assert!(!query_arity_lint);
        assert!(!query_client_only);
        assert!(!object_version_warning);
        assert!(!diagnostic_limit);
        assert_ne!(Deviations::ALL, Deviations::NONE);
    }

    #[test]
    fn the_default_deviations_are_all() {
        assert_eq!(Deviations::default(), Deviations::ALL);
        assert_eq!(Deviations::DEFAULT, Deviations::ALL);
    }

    #[test]
    fn new_is_the_default_configuration() {
        let o = CompileOptions::server(MolangVersion::LATEST);
        assert_eq!(o.version(), MolangVersion::LATEST);
        assert_eq!(o.raw_version, RawVersion(13));
        assert_eq!(o.admission, QueryAdmission::Sets(QuerySetMask::DEFAULT));
        assert_eq!(o.allowed_ops, OpSet::all());
        assert_eq!(o.experiments, ExperimentMask::empty());
        assert!(!o.keep_source);
        assert_eq!(o.catalog, server().clone());
        assert_eq!(o.catalog.side(), Side::Server);
        assert_eq!(
            CompileOptions::client(MolangVersion::LATEST).catalog,
            client().clone()
        );
        assert_eq!(
            o,
            CompileOptions::new(server().clone(), MolangVersion::LATEST)
        );
        assert_eq!(o.deviations, Deviations::ALL);
    }

    #[test]
    fn new_at_each_version_records_the_version_and_its_number() {
        for raw in -1..=13 {
            let version = MolangVersion::from_i16(raw).expect("a defined version");
            let o = CompileOptions::server(version);
            assert_eq!(o.version(), version);
            assert_eq!(o.raw_version, RawVersion(raw));
        }
    }

    #[test]
    fn the_version_is_the_effective_raw_version() {
        for (raw, gate) in [
            (i16::MIN, MolangVersion::Invalid),
            (-2, MolangVersion::Invalid),
            (-1, MolangVersion::Invalid),
            (0, MolangVersion::V0),
            (7, MolangVersion::V7),
            (13, MolangVersion::LATEST),
            (14, MolangVersion::LATEST),
            (i16::MAX, MolangVersion::LATEST),
        ] {
            let o = CompileOptions::from_raw_version(server().clone(), RawVersion(raw));
            assert_eq!(o.version(), gate, "raw {raw}");
            assert_eq!(o.raw_version, RawVersion(raw), "raw {raw}");
            let set = CompileOptions {
                raw_version: RawVersion(raw),
                ..opts()
            };
            assert_eq!(set, o, "raw {raw}");
            assert_eq!(set.version(), RawVersion(raw).effective(), "raw {raw}");
        }
    }

    #[test]
    fn for_source_uses_the_raw_version_of_the_source() {
        for (source, raw, gate) in [
            (MolangSource::string("1", 2), 2, MolangVersion::V2),
            (MolangSource::object("1", 99), 99, MolangVersion::LATEST),
            (MolangSource::string("1", -5), -5, MolangVersion::Invalid),
            (
                MolangSource::object("1", i16::MIN),
                i16::MIN,
                MolangVersion::Invalid,
            ),
        ] {
            let o = CompileOptions::for_source(server().clone(), &source)
                .expect("a source with a version");
            assert_eq!(o.raw_version, RawVersion(raw));
            assert_eq!(o.version(), gate);
            assert_eq!(
                o,
                CompileOptions::from_raw_version(server().clone(), RawVersion(raw))
            );
        }
        assert_eq!(
            CompileOptions::for_source(
                server().clone(),
                &MolangSource::string_without_context("1")
            ),
            None
        );
    }

    #[test]
    fn without_a_list_the_sets_decide() {
        let o = opts();
        assert_eq!(o.admission, QueryAdmission::Sets(QuerySetMask::DEFAULT));
        let decl = |name| o.catalog.get(name).unwrap();
        assert!(o.admission.admits(decl(query::IS_BABY)));
        assert!(!o.admission.admits(decl(query::ANY_TAG)));
        assert!(QueryAdmission::Sets(QuerySetMask::TAGS).admits(decl(query::ANY_TAG)));
    }

    /// Options compare their catalogue by identity.
    #[test]
    fn options_compare_the_catalogue_by_identity_and_a_list_never_changes_it() {
        let list = QueryAllowList::new(client(), [query::IS_BABY]).unwrap();
        let own = server()
            .extended([QueryDecl::new("query.mine", QueryShape::DEFAULT).unwrap()])
            .unwrap();
        let o = CompileOptions {
            catalog: own.clone(),
            admission: QueryAdmission::Only(list),
            ..opts()
        };
        assert_eq!(o.catalog, own);
        assert_ne!(
            o,
            CompileOptions {
                catalog: server().clone(),
                ..o.clone()
            }
        );
        assert_eq!(
            CompileOptions::from_raw_version(own, RawVersion(13)),
            CompileOptions {
                admission: opts().admission,
                ..o
            }
        );
    }

    #[test]
    fn the_op_set_presets_remove_the_assignment_and_the_random_functions() {
        let ops = OpSet::all().without_assignments();
        assert_eq!(ops, OpSet::all().without(Op::Assignment));
        assert!(ops.contains(Op::Random) && ops.contains(Op::RandomInt));
        assert_eq!(ops.len(), OpSet::all().len() - 1);
        let ops = OpSet::all().without_assignments_or_random();
        assert_eq!(
            ops,
            OpSet::all()
                .without(Op::Assignment)
                .without(Op::Random)
                .without(Op::RandomInt)
                .without(Op::HostMathVolatile)
        );
        assert!(
            ops.contains(Op::DieRoll) && ops.contains(Op::DieRollInt) && ops.contains(Op::HostMath)
        );
        assert_eq!(ops.len(), OpSet::all().len() - 4);
    }

    #[test]
    fn no_host_math_by_default_and_a_catalogue_by_struct_update() {
        use crate::catalog::{Arity, MathCatalog, MathDecl};
        assert_eq!(opts().math, None);
        assert_eq!(CompileOptions::client(MolangVersion::LATEST).math, None);
        let math =
            MathCatalog::new([MathDecl::pure("math.f", Arity::exactly(1), |a| a[0]).unwrap()])
                .unwrap();
        let o = CompileOptions {
            math: Some(math.clone()),
            ..opts()
        };
        assert_eq!(o.math.as_ref(), Some(&math));
        assert_ne!(o, opts());
        let copy =
            MathCatalog::new([MathDecl::pure("math.f", Arity::exactly(1), |a| a[0]).unwrap()])
                .unwrap();
        assert_ne!(
            o,
            CompileOptions {
                math: Some(copy),
                ..o.clone()
            },
            "compared by identity"
        );
    }

    #[test]
    fn struct_update_sets_only_the_fields_it_names() {
        let experiments =
            ExperimentMask::empty().with(Experiment::new(5).expect("a valid experiment"));
        let o = CompileOptions {
            catalog: client().clone(),
            deviations: Deviations::NONE,
            experiments,
            keep_source: true,
            ..opts()
        };
        assert_eq!(o.catalog.side(), Side::Client);
        assert_eq!(
            (o.deviations, o.experiments, o.keep_source),
            (Deviations::NONE, experiments, true)
        );
        assert_eq!(
            (o.raw_version, o.admission, o.allowed_ops),
            (opts().raw_version, opts().admission, opts().allowed_ops)
        );
        let deviations = Deviations {
            validate_nested: false,
            ..Deviations::ALL
        };
        assert!(!deviations.validate_nested && deviations.query_arity_lint);
    }
}
