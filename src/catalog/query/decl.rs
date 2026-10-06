//! One query declaration and the value types that describe it.

use core::fmt;
use core::hash::{Hash, Hasher};
use std::sync::Arc;

use thiserror::Error;

use super::{QuerySetMask, Reads, ReturnType};
use crate::catalog::{Arity, is_canonical_name};
use crate::version::{ExperimentMask, MolangVersion, RawVersion, semver::Version};

/// Which side a query is meaningful on, and whether a [`Side::Server`](super::Side::Server)
/// catalogue resolves it; [`Side`](super::Side) is the side a catalogue is for.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum QuerySide {
    /// Meaningful on client and server.
    Both {
        /// Whether a server catalogue resolves the query. It still declares it, so the compiler
        /// can say why it does not resolve.
        on_dedicated_server: bool,
    },
    /// Client or resource-pack only.
    Client {
        /// Whether a server catalogue resolves the query (and flags its calls as client-only).
        on_dedicated_server: bool,
    },
    /// Server or behavior-pack only; always resolves on a dedicated server.
    Server,
}

impl QuerySide {
    /// Both sides, on the dedicated server: the default.
    pub const BOTH: Self = Self::Both {
        on_dedicated_server: true,
    };
    /// Client only, still resolved by a server catalogue.
    pub const CLIENT: Self = Self::Client {
        on_dedicated_server: true,
    };

    /// Whether a [`Side::Server`](super::Side::Server) catalogue resolves the query.
    pub const fn on_dedicated_server(self) -> bool {
        match self {
            Self::Both {
                on_dedicated_server,
            }
            | Self::Client {
                on_dedicated_server,
            } => on_dedicated_server,
            Self::Server => true,
        }
    }

    /// Whether the query is meaningful on the client only.
    pub const fn is_client_only(self) -> bool {
        matches!(self, Self::Client { .. })
    }
}

impl Default for QuerySide {
    /// [`QuerySide::BOTH`].
    fn default() -> Self {
        Self::BOTH
    }
}

impl fmt::Display for QuerySide {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let side = match self {
            Self::Both { .. } => "both sides",
            Self::Client { .. } => "client only",
            Self::Server => "server only",
        };
        f.write_str(side)?;
        if self.on_dedicated_server() {
            Ok(())
        } else {
            f.write_str(", not on a dedicated server")
        }
    }
}

/// The value a query returns when it has no subject to read or no implementation.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum DefaultReturn {
    /// Float 0.0.
    #[default]
    Float0,
    /// Float 1.0.
    Float1,
    /// Float −1.0.
    FloatNeg1,
    /// The hash of `""`.
    EmptyString,
    /// An empty actor array.
    EmptyActorArray,
    /// A struct `.r .g .b .a` of 0.0.
    StructRgba0,
}

impl DefaultReturn {
    /// The float the default reads as in arithmetic; 0 for the non-float defaults (the empty
    /// string hashes to 0).
    pub const fn as_f32(self) -> f32 {
        match self {
            Self::Float1 => 1.0,
            Self::FloatNeg1 => -1.0,
            Self::Float0 | Self::EmptyString | Self::EmptyActorArray | Self::StructRgba0 => 0.0,
        }
    }

    /// Whether a query returning `returns` may have this default. `Float0` fits every type;
    /// `Float1` and `FloatNeg1` fit a number or a boolean.
    pub const fn fits(self, returns: ReturnType) -> bool {
        match self {
            Self::Float0 => true,
            Self::Float1 | Self::FloatNeg1 => returns.intersects(ReturnType::NUMBER),
            Self::EmptyString => returns.intersects(ReturnType::STRING),
            Self::EmptyActorArray => returns.intersects(ReturnType::ACTOR_ARRAY),
            Self::StructRgba0 => returns.intersects(ReturnType::STRUCT),
        }
    }
}

/// The Molang versions `first..=last` a query resolves at, and its query sets there.
///
/// A query has one implementation per window, numbered by position in its [`VersionRanges`].
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub struct VersionRange {
    first: MolangVersion,
    last: MolangVersion,
    sets: QuerySetMask,
}

impl VersionRange {
    /// [`MolangVersion::V0`] to [`MolangVersion::LATEST`] in the `default` set.
    pub const ALWAYS: Self = Self {
        first: MolangVersion::V0,
        last: MolangVersion::LATEST,
        sets: QuerySetMask::DEFAULT,
    };

    /// The window `first..=last` in `sets`; `None` when `first` is after `last`.
    pub const fn new(
        first: MolangVersion,
        last: MolangVersion,
        sets: QuerySetMask,
    ) -> Option<Self> {
        if first.as_i16() <= last.as_i16() {
            Some(Self { first, last, sets })
        } else {
            None
        }
    }

    /// The first version, inclusive.
    pub const fn first(&self) -> MolangVersion {
        self.first
    }

    /// The last version, inclusive.
    pub const fn last(&self) -> MolangVersion {
        self.last
    }

    /// The query sets of this window.
    pub const fn sets(&self) -> QuerySetMask {
        self.sets
    }

    /// The same window in other sets.
    #[must_use]
    pub const fn in_sets(self, sets: QuerySetMask) -> Self {
        Self { sets, ..self }
    }

    /// Whether `first ≤ version ≤ last`.
    pub const fn contains(&self, version: MolangVersion) -> bool {
        self.contains_raw(RawVersion(version.as_i16()))
    }

    /// Whether `first ≤ raw_version ≤ last`. A raw version outside `-1..=13` is in no window.
    pub const fn contains_raw(&self, raw_version: RawVersion) -> bool {
        self.first.as_i16() <= raw_version.0 && raw_version.0 <= self.last.as_i16()
    }
}

/// The version ranges of a query: non-empty, ascending and disjoint. Position `i` is
/// implementation `i`.
///
/// ```
/// use molangx::catalog::{DeclError, QuerySetMask, VersionRange, VersionRanges};
/// use molangx::version::MolangVersion;
///
/// let early =
///     VersionRange::new(MolangVersion::V0, MolangVersion::V7, QuerySetMask::DEFAULT).unwrap();
/// let late = VersionRange::new(
///     MolangVersion::V8,
///     MolangVersion::LATEST,
///     QuerySetMask::DEFAULT,
/// )
/// .unwrap();
/// let ranges = VersionRanges::new([early, late])?.in_sets(QuerySetMask::TAGS);
/// assert_eq!(
///     ranges.as_slice(),
///     [
///         early.in_sets(QuerySetMask::TAGS),
///         late.in_sets(QuerySetMask::TAGS)
///     ]
/// );
/// assert_eq!(VersionRanges::new([late, early]), Err(DeclError::OverlappingRanges));
/// # Ok::<(), DeclError>(())
/// ```
#[derive(Copy, Clone)]
pub struct VersionRanges {
    /// `ranges[..len]` are the ranges; the rest is unused.
    ranges: [VersionRange; MAX_RANGES],
    len: u8,
}

/// Disjoint ranges hold distinct versions, so there are at most as many ranges as versions.
const MAX_RANGES: usize =
    (MolangVersion::LATEST.as_i16() - MolangVersion::Invalid.as_i16() + 1) as usize;

impl VersionRanges {
    /// [`VersionRange::ALWAYS`] alone.
    pub const ALWAYS: Self = Self::single(VersionRange::ALWAYS);

    /// The one range `range`.
    pub const fn single(range: VersionRange) -> Self {
        Self {
            ranges: [range; MAX_RANGES],
            len: 1,
        }
    }

    /// These ranges, in order; an error when there is none or two overlap or are out of order.
    pub fn new(ranges: impl IntoIterator<Item = VersionRange>) -> Result<Self, DeclError> {
        let mut ranges = ranges.into_iter();
        let mut all = Self::single(ranges.next().ok_or(DeclError::NoVersionRange)?);
        for range in ranges {
            all = all.followed_by(range).ok_or(DeclError::OverlappingRanges)?;
        }
        Ok(all)
    }

    /// The ranges with `next` added after the last; `None` unless `next` starts after it ends.
    pub(crate) const fn followed_by(mut self, next: VersionRange) -> Option<Self> {
        let len = self.len as usize;
        if len == MAX_RANGES || self.ranges[len - 1].last.as_i16() >= next.first.as_i16() {
            return None;
        }
        self.ranges[len] = next;
        self.len += 1;
        Some(self)
    }

    /// The same ranges, each in `sets`.
    #[must_use]
    pub const fn in_sets(mut self, sets: QuerySetMask) -> Self {
        let mut i = 0;
        while i < self.len as usize {
            self.ranges[i] = self.ranges[i].in_sets(sets);
            i += 1;
        }
        self
    }

    /// The ranges, ascending.
    pub fn as_slice(&self) -> &[VersionRange] {
        &self.ranges[..usize::from(self.len)]
    }

    /// The union of the sets of every range.
    pub fn sets(&self) -> QuerySetMask {
        self.as_slice()
            .iter()
            .fold(QuerySetMask::empty(), |sets, range| sets.union(range.sets))
    }
}

impl Default for VersionRanges {
    /// [`VersionRanges::ALWAYS`].
    fn default() -> Self {
        Self::ALWAYS
    }
}

impl From<VersionRange> for VersionRanges {
    fn from(range: VersionRange) -> Self {
        Self::single(range)
    }
}

impl PartialEq for VersionRanges {
    fn eq(&self, other: &Self) -> bool {
        self.as_slice() == other.as_slice()
    }
}

impl Eq for VersionRanges {}

impl Hash for VersionRanges {
    fn hash<S: Hasher>(&self, state: &mut S) {
        self.as_slice().hash(state);
    }
}

impl fmt::Debug for VersionRanges {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_list().entries(self.as_slice()).finish()
    }
}

/// Why a [`QueryDecl`] or its [`VersionRanges`] were not built.
#[derive(Error, Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum DeclError {
    /// The name is not `query.` followed by a lower-case letter or `_`, then lower-case letters,
    /// digits or `_`: the only names an expression can spell after lowering.
    #[error(
        "`{0}` is not a canonical query name: `query.` followed by a lower-case letter or `_`, then lower-case letters, digits or `_`"
    )]
    Name(Box<str>),
    /// No version range was given.
    #[error("a query needs at least one version range")]
    NoVersionRange,
    /// Two version ranges overlap, or they are not in ascending order.
    #[error("the version ranges overlap or are not in ascending order")]
    OverlappingRanges,
    /// The default is not a value of the return type ([`DefaultReturn::fits`]).
    #[error("the default {default:?} is not a value of the return type {returns:?}")]
    DefaultNotReturned {
        /// The declared default.
        default: DefaultReturn,
        /// The declared return type.
        returns: ReturnType,
    },
    /// The read set has a bit no [`Reads`] constant names.
    #[error("the read set {0:?} has a bit no `Reads` constant names")]
    UndefinedReads(Reads),
    /// The first release has a pre-release tag or build metadata.
    #[error("the first release {0} has a pre-release tag or build metadata")]
    TaggedRelease(Version),
}

/// Everything a [`QueryDecl`] declares besides its name. Each field is valid on its own;
/// [`QueryDecl::new`] checks the rest.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct QueryShape {
    /// The argument counts. A call outside them only warns (`DiagCode::QueryArity`).
    pub args: Arity,
    /// The return type.
    pub returns: ReturnType,
    /// The version ranges the query resolves at, and its query sets there.
    pub ranges: VersionRanges,
    /// The experiments that must be enabled as well.
    pub experiments: ExperimentMask,
    /// What the implementation reads.
    pub reads: Reads,
    /// The side the query is meaningful on, and whether a server catalogue resolves it.
    pub side: QuerySide,
    /// The value returned without a subject or an implementation; it fits `returns`.
    pub default_return: DefaultReturn,
    /// The first game release that has the query, without a pre-release tag or build metadata;
    /// `None` for every supported release.
    pub first_release: Option<Version>,
}

impl QueryShape {
    /// Any number of arguments, a float result, [`VersionRanges::ALWAYS`], no experiment, reads
    /// nothing, both sides and on the dedicated server, a `0.0` default, every release.
    pub const DEFAULT: Self = Self {
        args: Arity::ANY,
        returns: ReturnType::FLOAT,
        ranges: VersionRanges::ALWAYS,
        experiments: ExperimentMask::empty(),
        reads: Reads::empty(),
        side: QuerySide::BOTH,
        default_return: DefaultReturn::Float0,
        first_release: None,
    };
}

impl Default for QueryShape {
    /// [`QueryShape::DEFAULT`].
    fn default() -> Self {
        Self::DEFAULT
    }
}

/// The declaration of one query, well-formed by construction ([`QueryDecl::new`]).
///
/// ```
/// use molangx::catalog::{Arity, DeclError, QueryDecl, QueryShape, ReturnType};
///
/// let shape = QueryShape {
///     args: Arity::between(0, 2),
///     returns: ReturnType::BOOL,
///     ..QueryShape::DEFAULT
/// };
/// let decl = QueryDecl::new("query.my_thing", shape)?;
/// assert_eq!(decl.args(), Arity::between(0, 2));
/// assert_eq!(
///     QueryDecl::new("query.My_Thing", QueryShape::DEFAULT),
///     Err(DeclError::Name("query.My_Thing".into()))
/// );
/// # Ok::<(), DeclError>(())
/// ```
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct QueryDecl {
    name: Arc<str>,
    shape: QueryShape,
}

// Disjoint ranges hold distinct versions, so a range's position always fits the `u8`
// implementation index.
const _: () = assert!(MAX_RANGES <= 256);

const PREFIX: &str = "query.";

impl QueryDecl {
    /// The declaration of `name` with `shape`; an error when the name is not canonical, the
    /// default does not fit the return type, the read set has an undefined bit or the first
    /// release has a pre-release tag or build metadata.
    pub fn new(name: &str, shape: QueryShape) -> Result<Self, DeclError> {
        if !is_canonical_name(name, PREFIX) {
            return Err(DeclError::Name(name.into()));
        }
        if let Some(release) = shape
            .first_release
            .as_ref()
            .filter(|release| !release.pre.is_empty() || !release.build.is_empty())
        {
            return Err(DeclError::TaggedRelease(release.clone()));
        }
        if !Reads::all().contains(shape.reads) {
            return Err(DeclError::UndefinedReads(shape.reads));
        }
        if !shape.default_return.fits(shape.returns) {
            return Err(DeclError::DefaultNotReturned {
                default: shape.default_return,
                returns: shape.returns,
            });
        }
        Ok(Self {
            name: Arc::from(name),
            shape,
        })
    }

    /// The full name (`"query.block_state"`).
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The full name, shared rather than copied.
    #[cfg(feature = "vm")]
    pub(crate) fn shared_name(&self) -> Arc<str> {
        Arc::clone(&self.name)
    }

    /// The name after `query.` (`"block_state"`).
    pub fn suffix(&self) -> &str {
        &self.name[PREFIX.len()..]
    }

    /// Everything the declaration declares besides its name.
    pub fn shape(&self) -> &QueryShape {
        &self.shape
    }

    /// The argument counts declared.
    pub fn args(&self) -> Arity {
        self.shape.args
    }

    /// The union of the sets of every version range.
    pub fn sets(&self) -> QuerySetMask {
        self.shape.ranges.sets()
    }

    /// Whether the query resolves in a [`Side::Server`](super::Side::Server) catalogue.
    pub fn on_dedicated_server(&self) -> bool {
        self.shape.side.on_dedicated_server()
    }

    /// The position of the range that contains `version`.
    pub fn implementation_at(&self, version: MolangVersion) -> Option<u8> {
        self.implementation_at_raw(RawVersion(version.as_i16()))
    }

    /// [`QueryDecl::implementation_at`] for a raw version.
    pub fn implementation_at_raw(&self, raw_version: RawVersion) -> Option<u8> {
        self.shape
            .ranges
            .as_slice()
            .iter()
            .position(|range| range.contains_raw(raw_version))
            .map(|position| position as u8)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::version::Experiment;

    fn v(raw: i16) -> MolangVersion {
        MolangVersion::from_i16(raw).unwrap()
    }

    fn range(first: i16, last: i16) -> VersionRange {
        VersionRange::new(v(first), v(last), QuerySetMask::DEFAULT).unwrap()
    }

    #[test]
    fn default_return_as_f32_for_every_variant() {
        for (default, value) in [
            (DefaultReturn::Float0, 0.0f32),
            (DefaultReturn::Float1, 1.0),
            (DefaultReturn::FloatNeg1, -1.0),
            (DefaultReturn::EmptyString, 0.0),
            (DefaultReturn::EmptyActorArray, 0.0),
            (DefaultReturn::StructRgba0, 0.0),
        ] {
            assert_eq!(default.as_f32().to_bits(), value.to_bits(), "{default:?}");
        }
        assert_eq!(DefaultReturn::default(), DefaultReturn::Float0);
        assert_eq!(QuerySide::default(), QuerySide::BOTH);
    }

    #[test]
    fn a_query_side_says_whether_a_dedicated_server_resolves_it() {
        let sides = [
            (QuerySide::BOTH, true, false, "both sides"),
            (
                QuerySide::Both {
                    on_dedicated_server: false,
                },
                false,
                false,
                "both sides, not on a dedicated server",
            ),
            (QuerySide::CLIENT, true, true, "client only"),
            (
                QuerySide::Client {
                    on_dedicated_server: false,
                },
                false,
                true,
                "client only, not on a dedicated server",
            ),
            (QuerySide::Server, true, false, "server only"),
        ];
        for (side, on_dedicated_server, client_only, text) in sides {
            assert_eq!(side.on_dedicated_server(), on_dedicated_server, "{side:?}");
            assert_eq!(side.is_client_only(), client_only, "{side:?}");
            assert_eq!(side.to_string(), text);
        }
    }

    #[test]
    fn a_version_range_is_inclusive_at_both_ends() {
        let r = range(1, 13);
        assert!(
            !r.contains_raw(RawVersion(0))
                && r.contains_raw(RawVersion(1))
                && r.contains_raw(RawVersion(13))
                && !r.contains_raw(RawVersion(14))
        );
        assert!(!r.contains_raw(RawVersion(i16::MIN)) && !r.contains_raw(RawVersion(i16::MAX)));
        assert!(!r.contains(MolangVersion::Invalid) && r.contains(MolangVersion::LATEST));
        assert_eq!(
            (r.first(), r.last(), r.sets()),
            (v(1), v(13), QuerySetMask::DEFAULT)
        );
        assert_eq!(r.in_sets(QuerySetMask::TAGS).sets(), QuerySetMask::TAGS);
        let one = range(8, 8);
        assert_eq!(
            (7..=9)
                .filter(|&raw| one.contains_raw(RawVersion(raw)))
                .collect::<Vec<_>>(),
            [8]
        );
        let invalid = range(-1, 0);
        assert!(invalid.contains(MolangVersion::Invalid) && !invalid.contains(MolangVersion::V1));
    }

    #[test]
    fn a_reversed_version_range_does_not_exist() {
        assert_eq!(VersionRange::new(v(5), v(4), QuerySetMask::DEFAULT), None);
        assert!(VersionRange::new(v(5), v(5), QuerySetMask::empty()).is_some());
        assert_eq!(VersionRange::ALWAYS, range(0, 13));
    }

    fn shape() -> QueryShape {
        QueryShape::DEFAULT
    }

    fn x(shape: QueryShape) -> Result<QueryDecl, DeclError> {
        QueryDecl::new("query.x", shape)
    }

    fn ranges(windows: &[(i16, i16)]) -> Result<VersionRanges, DeclError> {
        VersionRanges::new(windows.iter().map(|&(first, last)| range(first, last)))
    }

    #[test]
    fn only_canonical_names_are_accepted() {
        for good in [
            "query.a",
            "query._",
            "query.is_baby",
            "query.timer_flag_1",
            "query._x9",
        ] {
            assert_eq!(
                QueryDecl::new(good, shape()).map(|d| d.name().to_owned()),
                Ok(good.to_owned())
            );
        }
        for bad in [
            "",
            "query",
            "query.",
            "q.is_baby",
            "QUERY.IS_BABY",
            "query.Is_baby",
            "query.9a",
            "query.a.b",
            "query.a b",
            "math.abs",
            "query.é",
        ] {
            assert_eq!(
                QueryDecl::new(bad, shape()),
                Err(DeclError::Name(bad.into())),
                "{bad:?}"
            );
            let misfit = QueryShape {
                returns: ReturnType::STRING,
                default_return: DefaultReturn::Float1,
                ..shape()
            };
            assert_eq!(
                QueryDecl::new(bad, misfit),
                Err(DeclError::Name(bad.into())),
                "the name is checked first: {bad:?}"
            );
        }
    }

    #[test]
    fn a_first_release_with_a_tag_or_build_metadata_is_rejected() {
        for text in ["1.26.30-beta", "1.26.30+build", "1.26.30-rc.1+b"] {
            let release = Version::parse(text).unwrap();
            let tagged = QueryShape {
                first_release: Some(release.clone()),
                ..shape()
            };
            assert_eq!(x(tagged), Err(DeclError::TaggedRelease(release)), "{text}");
        }
        assert_eq!(
            DeclError::TaggedRelease(Version::parse("1.26.30-beta").unwrap()).to_string(),
            "the first release 1.26.30-beta has a pre-release tag or build metadata"
        );
        let plain = QueryShape {
            first_release: Some(Version::new(1, 26, 30)),
            ..shape()
        };
        assert!(x(plain).is_ok());
    }

    #[test]
    fn the_defaults() {
        let d = x(shape()).unwrap();
        assert_eq!((d.name(), d.suffix()), ("query.x", "x"));
        assert_eq!((d.args(), d.shape().args), (Arity::ANY, Arity::ANY));
        assert_eq!(d.shape().returns, ReturnType::FLOAT);
        assert_eq!(d.shape().ranges.as_slice(), [VersionRange::ALWAYS]);
        assert_eq!(d.sets(), QuerySetMask::DEFAULT);
        assert_eq!(d.shape().experiments, ExperimentMask::empty());
        assert_eq!(
            (d.shape().reads, d.shape().side, d.shape().default_return),
            (Reads::empty(), QuerySide::BOTH, DefaultReturn::Float0)
        );
        assert!(d.on_dedicated_server());
        assert_eq!(d.shape().first_release, None);
        assert_eq!(d.shape(), &QueryShape::DEFAULT);
        assert_eq!(QueryShape::default(), QueryShape::DEFAULT);
    }

    #[test]
    fn every_field_of_the_shape_is_kept() {
        let experiment = Experiment::new(5).unwrap();
        let d = x(QueryShape {
            args: Arity::between(1, 3),
            returns: ReturnType::BOOL,
            ranges: ranges(&[(0, 3), (4, 13)])
                .unwrap()
                .in_sets(QuerySetMask::TAGS),
            experiments: ExperimentMask::empty().with(experiment),
            reads: Reads::ACTOR,
            side: QuerySide::Client {
                on_dedicated_server: false,
            },
            default_return: DefaultReturn::Float1,
            first_release: Some(Version::new(1, 26, 30)),
        })
        .unwrap();
        assert_eq!(d.args(), Arity::between(1, 3));
        assert_eq!(d.shape().returns, ReturnType::BOOL);
        assert_eq!(d.shape().ranges.as_slice().len(), 2);
        assert!(
            d.shape()
                .ranges
                .as_slice()
                .iter()
                .all(|r| r.sets() == QuerySetMask::TAGS)
        );
        assert_eq!(
            d.shape().experiments,
            ExperimentMask::empty().with(experiment)
        );
        assert_eq!(
            (d.shape().reads, d.shape().side, d.shape().default_return),
            (
                Reads::ACTOR,
                QuerySide::Client {
                    on_dedicated_server: false
                },
                DefaultReturn::Float1
            )
        );
        assert!(!d.on_dedicated_server());
        assert_eq!(d.shape().first_release, Some(Version::new(1, 26, 30)));
        assert_eq!(
            (
                d.implementation_at(v(3)),
                d.implementation_at(v(4)),
                d.implementation_at(v(-1))
            ),
            (Some(0), Some(1), None)
        );
        assert_eq!(d.implementation_at_raw(RawVersion(14)), None);
    }

    #[test]
    fn the_default_fits_the_return_type() {
        let with = |returns: ReturnType, default_return: DefaultReturn| {
            x(QueryShape {
                returns,
                default_return,
                ..shape()
            })
        };
        assert_eq!(
            with(ReturnType::STRING, DefaultReturn::EmptyActorArray),
            Err(DeclError::DefaultNotReturned {
                default: DefaultReturn::EmptyActorArray,
                returns: ReturnType::STRING
            })
        );
        assert_eq!(
            with(ReturnType::STRING, DefaultReturn::Float1)
                .err()
                .map(|e| e.to_string()),
            Some("the default Float1 is not a value of the return type {String}".to_owned())
        );
        for (returns, default) in [
            (ReturnType::STRING, DefaultReturn::EmptyString),
            (ReturnType::ACTOR_ARRAY, DefaultReturn::EmptyActorArray),
            (ReturnType::STRUCT, DefaultReturn::StructRgba0),
            (ReturnType::BOOL, DefaultReturn::Float1),
            (ReturnType::FLOAT, DefaultReturn::FloatNeg1),
            (ReturnType::MATRIX, DefaultReturn::Float0),
            (
                ReturnType::STRING.union(ReturnType::ACTOR_ARRAY),
                DefaultReturn::EmptyActorArray,
            ),
        ] {
            assert!(with(returns, default).is_ok(), "{returns:?} {default:?}");
        }
        for default in [
            DefaultReturn::Float1,
            DefaultReturn::FloatNeg1,
            DefaultReturn::EmptyString,
            DefaultReturn::EmptyActorArray,
            DefaultReturn::StructRgba0,
        ] {
            assert!(!default.fits(ReturnType::MATRIX), "{default:?}");
        }
        assert!(DefaultReturn::Float0.fits(ReturnType::ACTOR));
    }

    #[test]
    fn in_sets_puts_every_range_in_the_sets() {
        let two = ranges(&[(0, 3), (4, 13)]).unwrap();
        assert_eq!(two.sets(), QuerySetMask::DEFAULT);
        assert_eq!(
            two.in_sets(QuerySetMask::WORLD_GEN).sets(),
            QuerySetMask::WORLD_GEN
        );
        assert!(
            two.in_sets(QuerySetMask::WORLD_GEN)
                .as_slice()
                .iter()
                .all(|r| r.sets() == QuerySetMask::WORLD_GEN)
        );
        let own =
            VersionRanges::new([range(0, 3), range(4, 13).in_sets(QuerySetMask::TAGS)]).unwrap();
        assert_eq!(
            own.sets(),
            QuerySetMask::DEFAULT | QuerySetMask::TAGS,
            "each range keeps its own"
        );
    }

    #[test]
    fn ill_formed_ranges_are_rejected() {
        assert_eq!(ranges(&[]), Err(DeclError::NoVersionRange));
        assert_eq!(
            ranges(&[(0, 5), (5, 13)]),
            Err(DeclError::OverlappingRanges)
        );
        assert_eq!(
            ranges(&[(6, 13), (0, 5)]),
            Err(DeclError::OverlappingRanges)
        );
        assert_eq!(ranges(&[(0, 5), (0, 5)]), Err(DeclError::OverlappingRanges));
        assert!(ranges(&[(0, 5), (7, 13)]).is_ok(), "a gap is allowed");
        let every = VersionRanges::new((-1..=13).map(|raw| range(raw, raw))).unwrap();
        assert_eq!(every.as_slice().len(), 15);
        assert_eq!(
            x(QueryShape {
                ranges: every,
                ..shape()
            })
            .unwrap()
            .implementation_at(MolangVersion::LATEST),
            Some(14)
        );
    }

    #[test]
    fn version_ranges_compare_hash_and_print_their_ranges_only() {
        use std::collections::hash_map::DefaultHasher;
        let hash = |r: &VersionRanges| {
            let mut h = DefaultHasher::new();
            r.hash(&mut h);
            h.finish()
        };
        let a = VersionRanges::single(range(0, 13));
        let b = VersionRanges::new([range(0, 13)]).unwrap();
        assert_eq!(a, b);
        assert_eq!(hash(&a), hash(&b));
        assert_eq!(
            VersionRanges::ALWAYS,
            VersionRanges::from(VersionRange::ALWAYS)
        );
        assert_eq!(VersionRanges::default(), VersionRanges::ALWAYS);
        assert_ne!(a, ranges(&[(0, 3), (4, 13)]).unwrap());
        assert_eq!(
            format!("{:?}", ranges(&[(0, 3), (4, 13)]).unwrap())
                .matches("VersionRange {")
                .count(),
            2
        );
    }

    #[test]
    fn a_read_set_with_an_undefined_bit_is_rejected() {
        let undefined = Reads::ACTOR | Reads::from_bits_retain(1 << 12);
        assert_eq!(
            x(QueryShape {
                reads: undefined,
                ..shape()
            }),
            Err(DeclError::UndefinedReads(undefined))
        );
        assert_eq!(
            DeclError::UndefinedReads(undefined).to_string(),
            "the read set {Actor, 0x1000} has a bit no `Reads` constant names"
        );
        assert!(
            x(QueryShape {
                reads: Reads::all(),
                ..shape()
            })
            .is_ok()
        );
    }

    #[test]
    fn errors_display() {
        assert_eq!(
            DeclError::NoVersionRange.to_string(),
            "a query needs at least one version range"
        );
        assert!(
            DeclError::Name("q.x".into())
                .to_string()
                .starts_with("`q.x` is not a canonical query name")
        );
    }
}
