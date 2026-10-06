//! What content targets: the Molang version ([`MolangVersion`], the rules keyed on it, the raw
//! number documents carry as [`RawVersion`], and the map [`VERSION_THRESHOLDS`] from a pack's
//! [`EngineVersion`]), the game release (a [`semver::Version`] without a pre-release tag or build
//! metadata, such as [`QueryShape::first_release`](crate::catalog::QueryShape::first_release)) and
//! the enabled experiments ([`ExperimentMask`]).
//!
//! ```
//! use molangx::version::{EngineVersion, MolangVersion};
//!
//! assert_eq!(MolangVersion::from_engine_version_str("1.18.10"), MolangVersion::V5);
//! // No engine version maps to 3: 1.17.40 maps to 4.
//! assert_eq!(MolangVersion::from_engine_version_str("1.17.40"), MolangVersion::V4);
//! // A pre-release sorts below its release.
//! assert_eq!(MolangVersion::from_engine_version_str("1.21.100-beta"), MolangVersion::V12);
//! // `"*"` maps to the newest version, an invalid engine version to `Invalid`.
//! let any = MolangVersion::from(&EngineVersion::Any);
//! assert_eq!(any, MolangVersion::LATEST);
//! assert_eq!(MolangVersion::from_engine_version_str("1.21"), MolangVersion::Invalid);
//! ```

mod engine;
mod experiment;

use std::fmt;

use thiserror::Error;

/// The `semver` crate, whose `Version` is a game release and an engine version. A new major
/// version of it is a breaking change of this crate.
pub use semver;

pub use engine::EngineVersion;
pub use experiment::{Experiment, ExperimentMask};

/// One entry of [`VERSION_THRESHOLDS`]: the first engine version of a [`MolangVersion`].
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub struct VersionThreshold {
    /// The first `min_engine_version` of `version`.
    pub engine_version: EngineVersion,
    /// The version this entry introduces.
    pub version: MolangVersion,
}

/// The first engine version of each Molang version from 1 to 13: entry `k` is where version
/// `k + 1` starts.
///
/// Version 3 shares 1.17.40 with 4 so the list stays dense; no engine version maps to 3.
pub static VERSION_THRESHOLDS: [VersionThreshold; 13] = [
    entry(1, 17, 0, MolangVersion::V1),
    entry(1, 17, 30, MolangVersion::V2),
    entry(1, 17, 40, MolangVersion::V3),
    entry(1, 17, 40, MolangVersion::V4),
    entry(1, 18, 10, MolangVersion::V5),
    entry(1, 18, 20, MolangVersion::V6),
    entry(1, 19, 60, MolangVersion::V7),
    entry(1, 20, 0, MolangVersion::V8),
    entry(1, 20, 10, MolangVersion::V9),
    entry(1, 20, 40, MolangVersion::V10),
    entry(1, 20, 50, MolangVersion::V11),
    entry(1, 20, 70, MolangVersion::V12),
    entry(1, 21, 100, MolangVersion::V13),
];

const fn entry(major: u64, minor: u64, patch: u64, version: MolangVersion) -> VersionThreshold {
    VersionThreshold {
        engine_version: EngineVersion::Version(semver::Version::new(major, minor, patch)),
        version,
    }
}

/// A version of the Molang language: the rules an expression is parsed and its queries resolved
/// with.
///
/// Every rule compares the version signed, so [`MolangVersion::Invalid`] (−1) takes the version-0
/// branch of every rule; it also resolves no standard query.
#[repr(i16)]
#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[non_exhaustive]
pub enum MolangVersion {
    /// −1: an invalid or missing `min_engine_version`. Parses with the rules of 0 and resolves no
    /// standard query.
    Invalid = -1,
    /// 0: below engine version 1.17.0. Left-associative `?:`, the old precedence of comparison and
    /// logical operators, and no error for a misused string, an unexpected operator or the empty
    /// expression.
    V0 = 0,
    /// 1: engine version 1.17.0; the parser rules are those of 0.
    V1 = 1,
    /// 2: engine version 1.17.30; the fix to `query.item_remaining_use_duration` takes effect.
    V2 = 2,
    /// 3: a string misused in arithmetic (`'text' + 1`) is an error. Reached only through a raw
    /// version 3; no engine version maps here.
    V3 = 3,
    /// 4: engine version 1.17.40. An unexpected operator (`1+(2 3)`) and the empty expression are
    /// errors.
    V4 = 4,
    /// 5: engine version 1.18.10. `a ? b : c ? d : e` is `a ? b : (c ? d : e)`.
    V5 = 5,
    /// 6: engine version 1.18.20. `&&` binds tighter than `||`, relational operators tighter than
    /// equality.
    V6 = 6,
    /// 7: engine version 1.19.60. Dividing by a negative value works: `5 / v.h` with `v.h = -1` is
    /// −5, not 5.
    V7 = 7,
    /// 8: engine version 1.20.0; the fix to `query.cape_flap_amount` takes effect.
    V8 = 8,
    /// 9: engine version 1.20.10. `query.block_property` is renamed to `query.block_state`; the old
    /// names still resolve.
    V9 = 9,
    /// 10: engine version 1.20.40. `query.block_property` and `query.has_block_property` no longer
    /// resolve.
    V10 = 10,
    /// 11: engine version 1.20.50. `query.is_scenting`, `query.is_rising`,
    /// `query.is_feeling_happy` and `query.dash_cooldown_progress` no longer resolve.
    V11 = 11,
    /// 12: engine version 1.20.70; leaf blocks count as supporting for the
    /// `query.surface_particle_*` queries.
    V12 = 12,
    /// 13: engine version 1.21.100. `query.is_carrying_block` gets a second version range.
    V13 = 13,
}

impl MolangVersion {
    /// The newest version.
    pub const LATEST: Self = Self::V13;

    /// The version number.
    pub const fn as_i16(self) -> i16 {
        self as i16
    }

    /// The version with this number, if it is one of −1..=13.
    pub const fn from_i16(raw: i16) -> Option<Self> {
        Some(match raw {
            -1 => Self::Invalid,
            0 => Self::V0,
            1 => Self::V1,
            2 => Self::V2,
            3 => Self::V3,
            4 => Self::V4,
            5 => Self::V5,
            6 => Self::V6,
            7 => Self::V7,
            8 => Self::V8,
            9 => Self::V9,
            10 => Self::V10,
            11 => Self::V11,
            12 => Self::V12,
            13 => Self::V13,
            _ => return None,
        })
    }

    /// [`EngineVersion`]'s `FromStr` then `MolangVersion::from`; a string that does not parse
    /// gives [`MolangVersion::Invalid`].
    ///
    /// ```
    /// use molangx::version::MolangVersion;
    ///
    /// assert_eq!(MolangVersion::from_engine_version_str("1.20.50"), MolangVersion::V11);
    /// assert_eq!(MolangVersion::from_engine_version_str("*"), MolangVersion::LATEST);
    /// assert_eq!(MolangVersion::from_engine_version_str("1.020.0"), MolangVersion::Invalid);
    /// ```
    pub fn from_engine_version_str(text: &str) -> Self {
        text.parse::<EngineVersion>()
            .map_or(Self::Invalid, |version| Self::from(&version))
    }

    /// The first engine version of this version ([`VERSION_THRESHOLDS`] entry `v − 1`); `None` for
    /// [`MolangVersion::V0`] and [`MolangVersion::Invalid`].
    ///
    /// Mapping it back can give a later version: 3 → 1.17.40 → 4.
    pub const fn first_engine_version(self) -> Option<&'static EngineVersion> {
        let raw = self as i16;
        if raw < 1 {
            None
        } else {
            Some(&VERSION_THRESHOLDS[(raw - 1) as usize].engine_version)
        }
    }

    /// From version 3: misuse of a string in arithmetic (`'text' + 1`) is an error.
    pub const fn reports_expression_errors(self) -> bool {
        self as i16 >= 3
    }

    /// From version 4: unexpected operators (`1+(2 3)`) and the empty expression are errors.
    pub const fn reports_unexpected_operators(self) -> bool {
        self as i16 >= 4
    }

    /// From version 5: `a ? b : c ? d : e` groups to the right.
    pub const fn right_assoc_ternary(self) -> bool {
        self as i16 >= 5
    }

    /// From version 6: `&&` binds tighter than `||`, relational tighter than equality.
    pub const fn c_like_logic_precedence(self) -> bool {
        self as i16 >= 6
    }

    /// From version 7: the division guard tests the signed divisor instead of its magnitude.
    pub const fn signed_division_fix(self) -> bool {
        self as i16 >= 7
    }
}

impl fmt::Display for MolangVersion {
    /// The version number.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.as_i16(), f)
    }
}

impl From<MolangVersion> for i16 {
    fn from(version: MolangVersion) -> Self {
        version.as_i16()
    }
}

impl From<&EngineVersion> for MolangVersion {
    /// The version of a pack's engine version: the index of the first [`VERSION_THRESHOLDS`]
    /// entry it is less than, or 13, so 3 is never produced. A pre-release sorts below its
    /// release; `"*"` gives 13.
    fn from(engine_version: &EngineVersion) -> Self {
        // An index, not an entry's version: entries 2 and 3 share 1.17.40, so it is never 3.
        VERSION_THRESHOLDS
            .iter()
            .position(|entry| engine_version.is_less_than(&entry.engine_version))
            .and_then(|i| Self::from_i16(i as i16))
            .unwrap_or(Self::LATEST)
    }
}

impl TryFrom<i16> for MolangVersion {
    type Error = UnknownVersion;

    fn try_from(raw: i16) -> Result<Self, UnknownVersion> {
        Self::try_from(RawVersion(raw))
    }
}

impl TryFrom<RawVersion> for MolangVersion {
    type Error = UnknownVersion;

    fn try_from(raw: RawVersion) -> Result<Self, UnknownVersion> {
        Self::from_i16(raw.0).ok_or_else(|| UnknownVersion::new(raw))
    }
}

/// The version number a document carries, unchecked: any `i16`.
///
/// Query resolution compares it unclamped (`14` resolves no query); the parser rules use
/// [`RawVersion::effective`] of it (`14` parses like 13).
///
/// ```
/// use molangx::version::{MolangVersion, RawVersion};
///
/// assert_eq!(RawVersion::from(MolangVersion::V5), RawVersion(5));
/// assert_eq!(RawVersion(99).effective(), MolangVersion::LATEST);
/// assert_eq!(MolangVersion::try_from(RawVersion(99)).ok(), None);
/// assert_eq!(RawVersion(-7).to_string(), "-7");
/// ```
#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[cfg_attr(feature = "facet", derive(facet::Facet), facet(transparent))]
pub struct RawVersion(pub i16);

impl RawVersion {
    /// The version the parser and evaluator rules use: above 13 acts as 13, any negative value as
    /// [`MolangVersion::Invalid`]. [`MolangVersion::try_from`] gives the defined version with this
    /// number instead.
    ///
    /// For the rules only. Query resolution
    /// ([`QueryDecl::resolve`](crate::catalog::QueryDecl::resolve)) compares the raw value
    /// unclamped, so a raw 14 gates like 13 but resolves no query.
    pub const fn effective(self) -> MolangVersion {
        if self.0 < 0 {
            return MolangVersion::Invalid;
        }
        match MolangVersion::from_i16(self.0) {
            Some(v) => v,
            None => MolangVersion::LATEST,
        }
    }
}

impl From<MolangVersion> for RawVersion {
    fn from(version: MolangVersion) -> Self {
        Self(version.as_i16())
    }
}

impl From<i16> for RawVersion {
    fn from(raw: i16) -> Self {
        Self(raw)
    }
}

impl From<RawVersion> for i16 {
    fn from(raw: RawVersion) -> Self {
        raw.0
    }
}

impl fmt::Display for RawVersion {
    /// The version number.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.0, f)
    }
}

/// A version number outside the defined Molang versions (−1..=13): the error of
/// [`MolangVersion`]'s `TryFrom` conversions.
#[derive(Error, Copy, Clone, Debug, PartialEq, Eq, Hash)]
#[error("{0} is not a Molang version (-1..=13)")]
pub struct UnknownVersion(RawVersion);

impl UnknownVersion {
    fn new(raw: RawVersion) -> Self {
        debug_assert!(
            MolangVersion::from_i16(raw.0).is_none(),
            "{raw:?} is a version"
        );
        Self(raw)
    }

    /// The number that is not a Molang version.
    pub const fn raw(self) -> RawVersion {
        self.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ev(major: u64, minor: u64, patch: u64) -> EngineVersion {
        EngineVersion::Version(semver::Version::new(major, minor, patch))
    }

    fn ev_pre(major: u64, minor: u64, patch: u64) -> EngineVersion {
        EngineVersion::Version(semver::Version {
            pre: semver::Prerelease::new("pre").unwrap(),
            ..semver::Version::new(major, minor, patch)
        })
    }

    fn parse(text: &str) -> Option<EngineVersion> {
        text.parse().ok()
    }

    fn map(major: u64, minor: u64, patch: u64) -> i16 {
        MolangVersion::from(&ev(major, minor, patch)).as_i16()
    }

    fn map_pre(major: u64, minor: u64, patch: u64) -> i16 {
        MolangVersion::from(&ev_pre(major, minor, patch)).as_i16()
    }

    #[test]
    fn current_releases_map_to_the_latest_version() {
        for current in [ev(1, 26, 0), ev(1, 26, 45)] {
            assert_eq!(MolangVersion::from(&current), MolangVersion::LATEST);
        }
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
            assert_eq!(v as i16, raw);
            assert_eq!(MolangVersion::from_i16(raw), Some(v));
        }
        assert_eq!(MolangVersion::from_i16(-2), None);
        assert_eq!(MolangVersion::from_i16(14), None);
        assert_eq!(size_of::<MolangVersion>(), size_of::<i16>());
    }

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
            assert_eq!(entry.engine_version, ev(major, minor, patch), "entry {i}");
            assert_eq!(entry.version.as_i16(), i as i16 + 1, "entry {i}");
        }
    }

    #[test]
    fn every_band_maps_to_its_version() {
        type Triple = (u64, u64, u64);
        // (first version of the band, last version inside it, expected version)
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
        assert_eq!(map(1, 9, 0), 0);
        assert_eq!(map(1, 100, 0), 13);
        assert_eq!(map(0, 99, 99), 0);
        assert_eq!(map(1, 20, 5), 8);
        assert_eq!(map(1, 20, 100), 12);
    }

    #[test]
    fn pre_release_sorts_below_its_release() {
        assert_eq!(map_pre(1, 21, 100), 12);
        assert_eq!(map(1, 21, 100), 13);
        assert_eq!(map_pre(1, 17, 0), 0);
        assert_eq!(map_pre(1, 17, 40), 2);
        assert_eq!(map_pre(1, 19, 60), 6);
        assert_eq!(map_pre(1, 21, 101), 13);
        assert_eq!(map_pre(1, 17, 1), 1);
    }

    #[test]
    fn build_metadata_is_not_part_of_the_comparison() {
        let molang = |text| MolangVersion::from_engine_version_str(text).as_i16();
        assert_eq!(molang("1.21.100+build"), 13);
        assert_eq!(molang("1.21.100-beta+build"), 12);
        assert_eq!(molang("1.17.40+zzz"), 4);
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
            assert_eq!(RawVersion(raw).effective().as_i16(), raw);
        }
        for raw in [14, 15, 100, i16::MAX] {
            assert_eq!(RawVersion(raw).effective(), MolangVersion::LATEST);
        }
        for raw in [-2, -100, i16::MIN] {
            assert_eq!(RawVersion(raw).effective(), MolangVersion::Invalid);
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

    fn molang(text: &str) -> i16 {
        parse(text)
            .as_ref()
            .map_or(MolangVersion::Invalid, MolangVersion::from)
            .as_i16()
    }

    #[test]
    fn release_strings() {
        assert_eq!(parse("1.21.100"), Some(ev(1, 21, 100)));
        assert_eq!(parse("65535.65535.65535"), Some(ev(65535, 65535, 65535)));
        assert_eq!(parse("1.020.0"), None);
        assert_eq!(molang("1.16.0"), 0);
        assert_eq!(molang("1.17.39"), 2);
        assert_eq!(molang("1.17.40"), 4);
        assert_eq!(molang("1.21.100"), 13);
    }

    #[test]
    fn pre_release_and_build_metadata() {
        for text in [
            "1.21.100-beta",
            "1.21.100-beta.1",
            "1.21.100-rc-2",
            "1.21.100-beta+b.5",
        ] {
            assert_eq!(molang(text), 12, "{text:?}");
        }
        assert_eq!(molang("1.21.100+build.5"), 13);
        assert_eq!(molang("1.21.100+abc"), 13);
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

    #[test]
    fn version_map_is_dense_and_sorted() {
        assert_eq!(VERSION_THRESHOLDS.len(), 13);
        for (k, entry) in VERSION_THRESHOLDS.iter().enumerate() {
            assert_eq!(usize::try_from(entry.version.as_i16()).unwrap(), k + 1);
            let EngineVersion::Version(version) = &entry.engine_version else {
                panic!("entry {k} is the any-version");
            };
            assert!(version.pre.is_empty() && version.build.is_empty());
        }
        for pair in VERSION_THRESHOLDS.windows(2) {
            assert!(!pair[1].engine_version.is_less_than(&pair[0].engine_version));
        }
        assert_eq!(
            VERSION_THRESHOLDS[2].engine_version,
            VERSION_THRESHOLDS[3].engine_version
        );
        for (k, pair) in VERSION_THRESHOLDS.windows(2).enumerate() {
            if k != 2 {
                assert!(
                    pair[0].engine_version.is_less_than(&pair[1].engine_version),
                    "entries {k} and {}",
                    k + 1
                );
            }
        }
    }

    #[test]
    fn entry_builds_a_release_entry() {
        let built = entry(1, 2, 3, MolangVersion::V1);
        assert_eq!(built.engine_version, ev(1, 2, 3));
        assert_eq!(built.version, MolangVersion::V1);
    }

    #[test]
    fn from_engine_version_at_and_below_every_row() {
        for (k, entry) in VERSION_THRESHOLDS.iter().enumerate() {
            let v = &entry.engine_version;
            // Entries 2 and 3 share 1.17.40.
            let expected = if k == 2 { 4 } else { k as i16 + 1 };
            assert_eq!(MolangVersion::from(v).as_i16(), expected, "row {k}");
            let EngineVersion::Version(version) = v else {
                panic!("row {k} is the any-version");
            };
            let (major, minor, patch) = (version.major, version.minor, version.patch);
            let below = if patch > 0 {
                ev(major, minor, patch - 1)
            } else {
                ev_pre(major, minor, patch)
            };
            let previous = MolangVersion::from(&below).as_i16();
            let expected_below = match k {
                0 => 0,
                3 => 2,
                _ => k as i16,
            };
            assert_eq!(previous, expected_below, "below row {k}");
        }
    }

    #[test]
    fn from_engine_version_special_inputs() {
        assert_eq!(
            MolangVersion::from(&EngineVersion::Any),
            MolangVersion::LATEST
        );
        assert_eq!(MolangVersion::from(&ev_pre(1, 21, 100)).as_i16(), 12);
        assert_eq!(MolangVersion::from(&ev(65535, 0, 0)).as_i16(), 13);
        assert_eq!(MolangVersion::from(&ev(0, 0, 0)).as_i16(), 0);
        assert_eq!(MolangVersion::from(&ev(1, 16, 0)), MolangVersion::V0);
        assert_eq!(MolangVersion::from(&ev_pre(1, 17, 30)), MolangVersion::V1);
        assert_eq!(MolangVersion::from(&ev(1, 17, 30)), MolangVersion::V2);
    }

    #[test]
    fn from_engine_version_never_returns_three_on_a_grid() {
        for major in 0..=2 {
            for minor in 0..30 {
                for patch in 0..200 {
                    for pre in [false, true] {
                        let v = if pre {
                            ev_pre(major, minor, patch)
                        } else {
                            ev(major, minor, patch)
                        };
                        assert_ne!(MolangVersion::from(&v), MolangVersion::V3, "{v:?}");
                    }
                }
            }
        }
    }

    proptest::proptest! {
        #![proptest_config(proptest::prelude::ProptestConfig { failure_persistence: None, ..Default::default() })]

        #[test]
        fn from_engine_version_is_monotone(
            a in (0u64..3, 0u64..30, 0u64..120, proptest::bool::ANY),
            b in (0u64..3, 0u64..30, 0u64..120, proptest::bool::ANY),
        ) {
            let make = |(major, minor, patch, pre): (u64, u64, u64, bool)| {
                if pre { ev_pre(major, minor, patch) } else { ev(major, minor, patch) }
            };
            let (a, b) = (make(a), make(b));
            if !b.is_less_than(&a) {
                proptest::prop_assert!(
                    MolangVersion::from(&a) <= MolangVersion::from(&b)
                );
            }
        }
    }

    #[test]
    fn from_i16_covers_exactly_minus_one_to_thirteen() {
        let mut count = 0;
        for raw in i16::MIN..=i16::MAX {
            match MolangVersion::from_i16(raw) {
                Some(v) => {
                    assert!((-1..=13).contains(&raw), "{raw}");
                    assert_eq!(v.as_i16(), raw);
                    count += 1;
                }
                None => assert!(!(-1..=13).contains(&raw), "{raw}"),
            }
        }
        assert_eq!(count, 15);
    }

    #[test]
    fn effective_at_the_extremes() {
        assert_eq!(RawVersion(-1).effective(), MolangVersion::Invalid);
        assert_eq!(RawVersion(i16::MIN).effective(), MolangVersion::Invalid);
        assert_eq!(RawVersion(-7).effective(), MolangVersion::Invalid);
        assert_eq!(RawVersion(0).effective(), MolangVersion::V0);
        assert_eq!(RawVersion(13).effective(), MolangVersion::LATEST);
        assert_eq!(RawVersion(14).effective(), MolangVersion::LATEST);
        assert_eq!(RawVersion(i16::MAX).effective(), MolangVersion::LATEST);
    }

    #[test]
    fn versions_order_by_their_raw_value() {
        let all: Vec<MolangVersion> = (-1..=13)
            .map(|raw| MolangVersion::from_i16(raw).unwrap())
            .collect();
        for pair in all.windows(2) {
            assert!(pair[0] < pair[1]);
        }
        assert_eq!(all.first(), Some(&MolangVersion::Invalid));
        assert_eq!(all.last(), Some(&MolangVersion::LATEST));
    }

    #[test]
    fn gates_are_const_fns() {
        const A: bool = MolangVersion::V1.reports_expression_errors();
        const B: bool = MolangVersion::LATEST.signed_division_fix();
        const C: MolangVersion = RawVersion(99).effective();
        const D: Option<MolangVersion> = MolangVersion::from_i16(5);
        assert_eq!((A, B), (false, true));
        assert_eq!(C, MolangVersion::LATEST);
        assert_eq!(D, Some(MolangVersion::V5));
    }

    #[test]
    fn gate_boundaries_at_the_adjacent_versions() {
        use MolangVersion as V;
        assert!(!V::V2.reports_expression_errors());
        assert!(V::V3.reports_expression_errors());
        assert!(!V::V3.reports_unexpected_operators());
        assert!(V::V4.reports_unexpected_operators());
        assert!(!V::V4.right_assoc_ternary());
        assert!(V::V5.right_assoc_ternary());
        assert!(!V::V5.c_like_logic_precedence());
        assert!(V::V6.c_like_logic_precedence());
        assert!(!V::V6.signed_division_fix());
        assert!(V::V7.signed_division_fix());
    }

    #[test]
    fn raw_versions_convert_both_ways() {
        for raw in [i16::MIN, -2, -1, 0, 13, 14, i16::MAX] {
            assert_eq!(RawVersion::from(raw), RawVersion(raw));
            assert_eq!(i16::from(RawVersion(raw)), raw);
            assert_eq!(RawVersion(raw).to_string(), raw.to_string());
        }
        for version in (-1..=13).filter_map(MolangVersion::from_i16) {
            assert_eq!(RawVersion::from(version), RawVersion(version.as_i16()));
            assert_eq!(i16::from(version), version.as_i16());
            assert_eq!(
                MolangVersion::try_from(RawVersion::from(version)),
                Ok(version)
            );
            assert_eq!(version.to_string(), version.as_i16().to_string());
        }
    }

    #[test]
    fn try_from_accepts_exactly_the_defined_versions() {
        for raw in [i16::MIN, -2, 14, i16::MAX] {
            assert_eq!(
                MolangVersion::try_from(raw).map_err(UnknownVersion::raw),
                Err(RawVersion(raw))
            );
            assert_eq!(
                MolangVersion::try_from(RawVersion(raw)).map_err(UnknownVersion::raw),
                Err(RawVersion(raw))
            );
            assert!(MolangVersion::try_from(RawVersion(raw)).is_err());
        }
        for raw in -1..=13 {
            assert_eq!(
                MolangVersion::try_from(raw).map(MolangVersion::as_i16),
                Ok(raw)
            );
            assert_eq!(
                MolangVersion::try_from(RawVersion(raw)).ok(),
                MolangVersion::from_i16(raw)
            );
        }
        assert_eq!(
            MolangVersion::try_from(14).unwrap_err().to_string(),
            "14 is not a Molang version (-1..=13)"
        );
    }

    #[test]
    fn from_engine_version_str_parses_then_maps() {
        for text in [
            "1.16.0",
            "1.17.40",
            "1.21.100-beta",
            "1.21.100",
            "*",
            "",
            "1.020.0",
            "1.21",
        ] {
            assert_eq!(
                MolangVersion::from_engine_version_str(text),
                parse(text)
                    .as_ref()
                    .map_or(MolangVersion::Invalid, MolangVersion::from),
                "{text:?}"
            );
        }
        assert_eq!(
            MolangVersion::from_engine_version_str("1.18.10"),
            MolangVersion::V5
        );
        assert_eq!(
            MolangVersion::from_engine_version_str("x"),
            MolangVersion::Invalid
        );
    }
}
