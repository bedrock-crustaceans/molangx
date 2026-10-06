//! [`EngineVersion`]: a pack engine version, read from its textual and JSON forms.

use std::cmp::Ordering;
use std::fmt;
use std::str::FromStr;

use semver::Version;

/// A pack engine version (`min_engine_version`, a document `format_version`): the any-version
/// `"*"` or a semantic version.
///
/// `==` and `Hash` compare every part, the pre-release tag's text and build metadata included.
/// The version map orders by [`EngineVersion::is_less_than`], which ignores both texts.
///
/// ```
/// use molangx::version::EngineVersion;
///
/// let beta: EngineVersion = "1.21.100-beta".parse()?;
/// assert!(beta.is_less_than(&"1.21.100".parse()?));
/// assert_eq!(EngineVersion::Any.to_string(), "*");
/// # Ok::<(), molangx::version::semver::Error>(())
/// ```
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum EngineVersion {
    /// The any-version `"*"`: never less than any version, so it maps to the newest
    /// [`MolangVersion`](super::MolangVersion).
    Any,
    /// A semantic version, with or without a pre-release tag and build metadata.
    Version(Version),
}

impl EngineVersion {
    /// Reads the array form of a manifest's `min_engine_version`, `[1, 20, 0]`: exactly three whole
    /// numbers in `0..2⁶⁴`.
    ///
    /// ```
    /// use molangx::version::{EngineVersion, semver::Version};
    ///
    /// assert_eq!(
    ///     EngineVersion::from_json_array(&[1.0, 20.0, 0.0]),
    ///     Some(EngineVersion::Version(Version::new(1, 20, 0)))
    /// );
    /// assert_eq!(EngineVersion::from_json_array(&[1.0, 20.0]), None);
    /// assert_eq!(EngineVersion::from_json_array(&[1.0, 20.5, 0.0]), None);
    /// ```
    pub fn from_json_array(numbers: &[f64]) -> Option<Self> {
        // 2⁶⁴ is the first `f64` above `u64::MAX`.
        const LIMIT: f64 = 18_446_744_073_709_551_616.0;
        let component = |x: f64| -> Option<u64> {
            (x.fract() == 0.0 && (0.0..LIMIT).contains(&x)).then_some(x as u64)
        };
        match *numbers {
            [major, minor, patch] => Some(Self::Version(Version::new(
                component(major)?,
                component(minor)?,
                component(patch)?,
            ))),
            _ => None,
        }
    }

    /// The order the version map uses: the any-version is never less; otherwise the numeric
    /// triples compare and, on a tie, a version with a pre-release tag is less than one without.
    ///
    /// The tag's text and build metadata take no part, so this is not `SemVer` precedence. An
    /// any-version on the right compares as 0.0.0 without a tag.
    pub fn is_less_than(&self, other: &Self) -> bool {
        let Self::Version(this) = self else {
            return false;
        };
        let (triple, pre_release) = match other {
            Self::Any => ((0, 0, 0), false),
            Self::Version(other) => (
                (other.major, other.minor, other.patch),
                !other.pre.is_empty(),
            ),
        };
        match (this.major, this.minor, this.patch).cmp(&triple) {
            Ordering::Equal => !this.pre.is_empty() && !pre_release,
            order => order.is_lt(),
        }
    }
}

impl fmt::Display for EngineVersion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Any => f.pad("*"),
            Self::Version(version) => fmt::Display::fmt(version, f),
        }
    }
}

impl FromStr for EngineVersion {
    type Err = semver::Error;

    /// `"*"` exactly, or what [`Version::from_str`] reads.
    fn from_str(text: &str) -> Result<Self, semver::Error> {
        if text == "*" {
            Ok(Self::Any)
        } else {
            text.parse().map(Self::Version)
        }
    }
}

#[cfg(test)]
mod tests {
    use semver::Prerelease;

    use super::*;
    use crate::version::MolangVersion;

    fn v(major: u64, minor: u64, patch: u64) -> EngineVersion {
        EngineVersion::Version(Version::new(major, minor, patch))
    }

    fn pre(major: u64, minor: u64, patch: u64, tag: &str) -> EngineVersion {
        EngineVersion::Version(Version {
            pre: Prerelease::new(tag).unwrap(),
            ..Version::new(major, minor, patch)
        })
    }

    fn parse(text: &str) -> Option<EngineVersion> {
        text.parse().ok()
    }

    /// Engine-version strings and the Molang version each maps to; −1 is rejected.
    const HAND_PICKED: &[(&str, i16)] = &[
        ("1.21.100", 13),
        ("1.21.100-beta", 12),
        ("1.21.100-beta+build.5", 12),
        ("1.21.100+build.5", 13),
        ("1.20.0", 8),
        ("1.19.60-beta", 6),
        ("1.19.50", 6),
        ("1.2.3", 0),
        ("0.0.0", 0),
        ("1.16.0", 0),
        ("1.17.39", 2),
        ("1.17.40", 4),
        ("1.17.40-rc", 2),
        ("*", 13),
        ("1.2.3-a-b", 0),
        ("1.2.3+a-b", 0),
        ("1.19.60-0", 6),
        ("1.19.60-0a", 6),
        ("1.19.60-01a", 6),
        ("1.19.60-0+01", 6),
        ("1.19.60+01", 7),
        ("1.19.60-A-Z.1.2", 6),
        ("65535.65535.65535", 13),
        // Components are `u64`.
        ("65536.0.0", 13),
        ("1.65536.0", 13),
        ("1.2.65536", 0),
        ("1.20.99999", 12),
        ("18446744073709551615.0.0", 13),
        ("18446744073709551616.0.0", -1),
        ("1.2.99999999999999999999", -1),
        // No empty identifier in the build metadata.
        ("1.21.100+a..b", -1),
        ("1.21.100+.a", -1),
        ("1.21.100+a.", -1),
        ("1.21.100-beta+a..b", -1),
        ("", -1),
        (" ", -1),
        ("1", -1),
        ("1.21", -1),
        ("1.21.100.0", -1),
        (" 1.21.100", -1),
        ("1.21.100 ", -1),
        ("\t1.2.3", -1),
        ("1.2.3\n", -1),
        ("1..100", -1),
        (".1.2", -1),
        ("1.2.", -1),
        ("-1.2.3", -1),
        ("+1.2.3", -1),
        ("1.2.-3", -1),
        ("1.+2.3", -1),
        ("1.2.3-", -1),
        ("1.2.3+", -1),
        ("1.2.3-beta+", -1),
        ("1.2.3-be ta", -1),
        ("1.2.3-a_b", -1),
        ("1.2.3+a_b", -1),
        ("1.2.3+a+b", -1),
        ("1.2.3-é", -1),
        ("1.2.3+é", -1),
        ("a.b.c", -1),
        ("1.x.0", -1),
        ("1.2.3a", -1),
        ("1,2,3", -1),
        ("v1.2.3", -1),
        ("=1.2.3", -1),
        ("0x1.0.0", -1),
        ("1_0.0.0", -1),
        ("01.20.0", -1),
        ("1.020.0", -1),
        ("1.2.03", -1),
        ("00.0.0", -1),
        ("1.19.60-01", -1),
        ("1.2.3-01", -1),
        ("1.19.60-beta.01", -1),
        ("1.19.60-00", -1),
        ("1.19.60-a..b", -1),
        ("1.19.60-.a", -1),
        ("1.19.60-a.", -1),
        ("**", -1),
        (" *", -1),
        ("* ", -1),
        ("*.0.0", -1),
        ("*-beta", -1),
        ("*+x", -1),
        ("1.*.0", -1),
    ];

    #[test]
    fn hand_picked_strings_map_as_listed() {
        for &(text, expected) in HAND_PICKED {
            assert_eq!(
                MolangVersion::from_engine_version_str(text).as_i16(),
                expected,
                "{text:?}"
            );
            assert_eq!(parse(text).is_some(), expected != -1, "{text:?}");
        }
    }

    #[test]
    fn parse_keeps_the_tag_and_the_build_metadata() {
        let parsed = parse("1.21.100-beta.1+build.5").unwrap();
        let EngineVersion::Version(version) = &parsed else {
            panic!("a version");
        };
        assert_eq!(version.pre.as_str(), "beta.1");
        assert_eq!(version.build.as_str(), "build.5");
        assert_eq!(parsed.to_string(), "1.21.100-beta.1+build.5");
        assert_ne!(parse("1.2.3-alpha"), parse("1.2.3-beta"));
        assert_eq!(parse("*"), Some(EngineVersion::Any));
    }

    #[test]
    fn from_str_reports_the_semver_error() {
        assert!("1.21".parse::<EngineVersion>().is_err());
        assert_eq!(
            "1.21".parse::<EngineVersion>().unwrap_err().to_string(),
            Version::parse("1.21").unwrap_err().to_string()
        );
        assert!("**".parse::<EngineVersion>().is_err());
    }

    #[test]
    fn is_less_than_orders_triples_numerically() {
        assert!(v(1, 9, 0).is_less_than(&v(1, 10, 0)));
        assert!(!v(1, 10, 0).is_less_than(&v(1, 9, 0)));
        assert!(!v(2, 0, 0).is_less_than(&v(1, 99, 99)));
        assert!(v(1, 99, 99).is_less_than(&v(2, 0, 0)));
        assert!(v(1, 2, 3).is_less_than(&v(1, 2, 4)));
        assert!(!v(1, 2, 4).is_less_than(&v(1, 2, 3)));
        assert!(!v(1, 2, 3).is_less_than(&v(1, 2, 3)));
        assert!(v(65535, 0, 0).is_less_than(&v(65536, 0, 0)));
    }

    #[test]
    fn is_less_than_a_lower_triple_wins_over_the_tag() {
        assert!(v(1, 2, 3).is_less_than(&pre(1, 2, 4, "beta")));
        assert!(!pre(1, 2, 4, "beta").is_less_than(&v(1, 2, 3)));
        assert!(pre(1, 2, 3, "beta").is_less_than(&v(1, 2, 4)));
        assert!(!v(1, 2, 4).is_less_than(&pre(1, 2, 3, "beta")));
    }

    #[test]
    fn is_less_than_ignores_the_tag_text_and_the_build_metadata() {
        let alpha = pre(1, 21, 100, "alpha");
        let beta = pre(1, 21, 100, "beta");
        assert!(alpha.is_less_than(&v(1, 21, 100)));
        assert!(!v(1, 21, 100).is_less_than(&alpha));
        assert!(!alpha.is_less_than(&beta));
        assert!(!beta.is_less_than(&alpha));
        let built = parse("1.21.100+zzz").unwrap();
        let other_build = parse("1.21.100+aaa").unwrap();
        assert!(!built.is_less_than(&other_build));
        assert!(!other_build.is_less_than(&built));
        assert!(!v(1, 21, 100).is_less_than(&built));
        assert!(parse("1.21.100-a+zzz").unwrap().is_less_than(&built));
    }

    #[test]
    fn is_less_than_any_is_never_less_and_other_any_is_zero() {
        assert!(!EngineVersion::Any.is_less_than(&v(0, 0, 0)));
        assert!(!EngineVersion::Any.is_less_than(&v(u64::MAX, u64::MAX, u64::MAX)));
        assert!(!EngineVersion::Any.is_less_than(&EngineVersion::Any));
        assert!(!v(0, 0, 0).is_less_than(&EngineVersion::Any));
        assert!(!v(0, 0, 1).is_less_than(&EngineVersion::Any));
        assert!(pre(0, 0, 0, "a").is_less_than(&EngineVersion::Any));
    }

    #[test]
    fn from_json_array_reads_three_whole_numbers() {
        assert_eq!(
            EngineVersion::from_json_array(&[1.0, 20.0, 0.0]),
            Some(v(1, 20, 0))
        );
        assert_eq!(
            EngineVersion::from_json_array(&[-0.0, 0.0, 0.0]),
            Some(v(0, 0, 0))
        );
        assert_eq!(
            EngineVersion::from_json_array(&[65536.0, 0.0, 0.0]),
            Some(v(65536, 0, 0))
        );
        // The largest `f64` below 2⁶⁴.
        let top = 18_446_744_073_709_549_568.0;
        assert_eq!(
            EngineVersion::from_json_array(&[top, 0.0, 0.0]),
            Some(v(18_446_744_073_709_549_568, 0, 0))
        );
    }

    #[test]
    fn from_json_array_rejects_components_outside_u64_or_not_whole() {
        for bad in [
            18_446_744_073_709_551_616.0,
            -1.0,
            1.5,
            0.5,
            f64::NAN,
            f64::INFINITY,
            f64::NEG_INFINITY,
            1e300,
            -1e-300,
        ] {
            for position in 0..3 {
                let mut array = [1.0, 2.0, 3.0];
                array[position] = bad;
                assert_eq!(EngineVersion::from_json_array(&array), None, "{array:?}");
            }
        }
    }

    #[test]
    fn from_json_array_needs_exactly_three_numbers() {
        assert_eq!(EngineVersion::from_json_array(&[]), None);
        assert_eq!(EngineVersion::from_json_array(&[1.0]), None);
        assert_eq!(EngineVersion::from_json_array(&[1.0, 2.0]), None);
        assert_eq!(EngineVersion::from_json_array(&[1.0, 2.0, 3.0, 4.0]), None);
    }

    #[test]
    fn display_prints_the_version_as_written_or_a_star() {
        for text in ["1.21.100", "1.21.100-beta.1", "1.2.3+a-b", "0.0.0", "*"] {
            assert_eq!(parse(text).unwrap().to_string(), text);
        }
    }

    #[test]
    fn display_honours_width_and_alignment() {
        assert_eq!(format!("{:>6}", EngineVersion::Any), "     *");
        assert_eq!(format!("{:-<3}|", EngineVersion::Any), "*--|");
        assert_eq!(format!("{:>8}", v(1, 2, 3)), "   1.2.3");
    }
}
