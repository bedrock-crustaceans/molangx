//! What a script may call and how a host declares it: the queries of a [`QueryCatalog`] and the
//! host's own `math.*` functions in a `MathCatalog` (feature `compiler`).
//!
#![cfg_attr(
    feature = "stdlib",
    doc = "A query catalogue is a value: [`stdlib::queries`](crate::stdlib::queries) holds the standard library's queries and a host"
)]
#![cfg_attr(
    not(feature = "stdlib"),
    doc = "A query catalogue is a value: `stdlib::queries` holds the standard library's queries and a host"
)]
//! extends it with its own. A name with two version ranges is told apart by its implementation
//! index. Math functions are operators, not queries: no query set or version range applies to them.
//! The library ships declarations only; it never implements a query.

use core::cmp::Ordering;

mod arity;
#[cfg(feature = "compiler")]
pub(crate) mod math;
mod query;

pub use arity::{Arity, EmptyArity};
#[cfg(feature = "compiler")]
pub(crate) use math::MathRef;
#[cfg(feature = "compiler")]
#[cfg_attr(docsrs, doc(cfg(feature = "compiler")))]
pub use math::{
    MAX_MATH_ARGS, MathCatalog, MathDecl, MathError, MathImpl, PureMathFn, VolatileMathFn,
};
#[cfg(feature = "compiler")]
pub(crate) use query::QueryIndex;
#[cfg(all(test, feature = "compiler", feature = "stdlib"))]
pub(crate) use query::stdlib_index;
pub use query::{
    AllowListError, CatalogError, DeclError, DefaultReturn, ParseQuerySetError, QueryAdmission,
    QueryAllowList, QueryCatalog, QueryDecl, QuerySetMask, QueryShape, QuerySide, Reads,
    ReturnType, Side, VersionRange, VersionRanges,
};

/// The most declarations a catalogue holds: a position fits a `u16`.
const MAX_DECLS: usize = 1 << 16;

/// Positions into a catalogue's declarations, sorted by name, for lookup by binary search.
struct ByName(Box<[u16]>);

impl ByName {
    /// The order of `decls`, which number at most [`MAX_DECLS`].
    fn new<T>(decls: &[T], name: impl Fn(&T) -> &str) -> Self {
        let mut order: Box<[u16]> = (0..decls.len())
            .map(|i| u16::try_from(i).unwrap_or(u16::MAX))
            .collect();
        order.sort_unstable_by(|&a, &b| {
            name(&decls[usize::from(a)]).cmp(name(&decls[usize::from(b)]))
        });
        Self(order)
    }

    /// The position of the declaration `probe` finds equal; `decls` are those `self` orders.
    fn find<T>(&self, decls: &[T], probe: impl Fn(&T) -> Ordering) -> Option<u16> {
        let found = self
            .0
            .binary_search_by(|&i| probe(&decls[usize::from(i)]))
            .ok()?;
        Some(self.0[found])
    }
}

/// Whether `name` is `prefix` followed by a lower-case letter or `_`, then lower-case letters,
/// digits or `_`: the names an expression can spell after lowering.
fn is_canonical_name(name: &str, prefix: &str) -> bool {
    let Some(suffix) = name.strip_prefix(prefix) else {
        return false;
    };
    let mut bytes = suffix.bytes();
    bytes
        .next()
        .is_some_and(|b| b.is_ascii_lowercase() || b == b'_')
        && bytes.all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::stdlib::query;

    #[test]
    fn query_re_exports_are_the_catalogue_items() {
        let catalog: &'static QueryCatalog = crate::stdlib::queries(Side::Client);
        let decl: &QueryDecl = catalog.get(query::IS_BABY).unwrap();
        assert_eq!(decl.name(), "query.is_baby");
        let _: QueryAdmission = QueryAdmission::Sets(QuerySetMask::DEFAULT);
        let _: QueryShape = QueryShape {
            args: Arity::ANY,
            ..QueryShape::DEFAULT
        };
        let _: Result<VersionRanges, DeclError> = VersionRanges::new([VersionRange::ALWAYS]);
        let _: Option<CatalogError> = None;
        let _: Option<DeclError> = None;
    }

    #[test]
    fn value_type_re_exports_are_usable() {
        assert_eq!(QuerySide::default(), QuerySide::BOTH);
        assert_eq!(DefaultReturn::Float0.as_f32().to_bits(), 0.0_f32.to_bits());
        assert!(ReturnType::NUMBER.contains(ReturnType::BOOL));
        assert!(Reads::empty().is_empty());
        assert_eq!(VersionRange::ALWAYS.sets(), QuerySetMask::DEFAULT);
    }
}
