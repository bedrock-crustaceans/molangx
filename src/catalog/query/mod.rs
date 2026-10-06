//! The query catalogue: the set of queries an expression may call, as a value.
//!
//! The compiler resolves names against the catalogue of its
//! [`CompileOptions`](crate::compile::CompileOptions); the evaluator's
//! [`QueryTable`](crate::vm::QueryTable) holds one implementation per declaration.

mod decl;
mod resolve;
mod returns_and_reads;
mod sets;

use std::collections::HashSet;
use std::fmt;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

use thiserror::Error;

use super::{ByName, MAX_DECLS};

pub use decl::{
    DeclError, DefaultReturn, QueryDecl, QueryShape, QuerySide, VersionRange, VersionRanges,
};
pub use resolve::{AllowListError, QueryAdmission, QueryAllowList};
pub use returns_and_reads::{Reads, ReturnType};
pub use sets::{ParseQuerySetError, QuerySetMask};

/// Which side a catalogue is for. Exhaustive: it will not grow.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum Side {
    /// A query with [`QueryDecl::on_dedicated_server`] false does not resolve, and calls of
    /// client-only queries are flagged.
    Server,
    /// Every query of the catalogue resolves.
    Client,
}

impl fmt::Display for Side {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Server => "server",
            Self::Client => "client",
        })
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) struct QueryIndex(u16);

impl QueryIndex {
    /// The position in the catalogue, in declaration order.
    pub(crate) const fn index(self) -> usize {
        self.0 as usize
    }
}

struct Inner {
    side: Side,
    decls: Box<[QueryDecl]>,
    by_name: ByName,
}

/// An immutable set of query declarations for one [`Side`].
///
/// Cheap to clone (a shared handle). Compared and hashed by **identity**: clones are equal, two
/// catalogues with equal declarations are not. A compiled expression keeps its catalogue.
///
/// ```
/// # #[cfg(feature = "stdlib")]
/// # {
/// use molangx::catalog::{Arity, QueryDecl, QueryShape, Side};
/// use molangx::stdlib;
///
/// let my_thing = QueryDecl::new(
///     "query.my_thing",
///     QueryShape {
///         args: Arity::between(0, 2),
///         ..QueryShape::DEFAULT
///     },
/// )?;
/// let catalog = stdlib::queries(Side::Client).extended([my_thing])?;
/// assert_eq!(catalog.len(), stdlib::queries(Side::Client).len() + 1);
/// assert_eq!(catalog.get("query.my_thing").map(|d| d.args().max()), Some(Some(2)));
/// assert_ne!(&catalog, stdlib::queries(Side::Client), "a different catalogue");
/// # }
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
#[derive(Clone)]
pub struct QueryCatalog {
    inner: Arc<Inner>,
}

impl QueryCatalog {
    /// The catalogue of `decls` for `side`, in that order; an error when a name is declared twice
    /// or there are more than 65,536 declarations.
    pub fn new(
        side: Side,
        decls: impl IntoIterator<Item = QueryDecl>,
    ) -> Result<Self, CatalogError> {
        let decls: Box<[QueryDecl]> = decls.into_iter().collect();
        if decls.len() > MAX_DECLS {
            return Err(CatalogError::Full);
        }
        let mut names = HashSet::with_capacity(decls.len());
        if let Some(twice) = decls.iter().find(|decl| !names.insert(decl.name())) {
            return Err(CatalogError::Duplicate(twice.name().into()));
        }
        Ok(Self::assemble(side, decls))
    }

    /// The catalogue of `decls`, which hold at most 65,536 distinct names.
    fn assemble(side: Side, decls: Box<[QueryDecl]>) -> Self {
        let by_name = ByName::new(&decls, QueryDecl::name);
        Self {
            inner: Arc::new(Inner {
                side,
                decls,
                by_name,
            }),
        }
    }

    /// A new catalogue for the same side with every declaration of this one, then `decls`; an
    /// error when a name is declared already or twice. [`QueryCatalog::overriding`] replaces
    /// declarations instead.
    pub fn extended(
        &self,
        decls: impl IntoIterator<Item = QueryDecl>,
    ) -> Result<Self, CatalogError> {
        Self::new(self.side(), self.iter().cloned().chain(decls))
    }

    /// A new catalogue for the same side in which each of `decls` replaces this one's declaration
    /// of the same name, at its position; an error when this catalogue does not declare a name
    /// ([`QueryCatalog::extended`] adds names) or `decls` holds one twice.
    ///
    /// ```
    /// # #[cfg(feature = "stdlib")]
    /// # {
    /// use molangx::catalog::{Arity, QueryDecl, QueryShape, ReturnType, Side};
    /// use molangx::stdlib::{self, query};
    ///
    /// let standard = stdlib::queries(Side::Server);
    /// let health = QueryDecl::new(
    ///     query::HEALTH,
    ///     QueryShape {
    ///         args: Arity::exactly(1),
    ///         returns: ReturnType::BOOL,
    ///         ..QueryShape::DEFAULT
    ///     },
    /// )?;
    /// let catalog = standard.overriding([health])?;
    /// assert_eq!(catalog.len(), standard.len());
    /// assert_eq!(catalog.get(query::HEALTH).map(|d| d.shape().returns), Some(ReturnType::BOOL));
    /// # }
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub fn overriding(
        &self,
        decls: impl IntoIterator<Item = QueryDecl>,
    ) -> Result<Self, CatalogError> {
        let mut all: Box<[QueryDecl]> = self.iter().cloned().collect();
        let mut replaced = vec![false; all.len()];
        for decl in decls {
            let index = self
                .index_of(decl.name())
                .ok_or_else(|| CatalogError::Undeclared(decl.name().into()))?
                .index();
            if std::mem::replace(&mut replaced[index], true) {
                return Err(CatalogError::Duplicate(decl.name().into()));
            }
            all[index] = decl;
        }
        Ok(Self::assemble(self.side(), all))
    }

    /// The side the catalogue is for.
    pub fn side(&self) -> Side {
        self.inner.side
    }

    /// The declaration of the full name `name` (`"query.block_state"`). Only the lower-case
    /// `query.` spelling is found.
    pub fn get(&self, name: &str) -> Option<&QueryDecl> {
        self.index_of(name).map(|index| self.decl(index))
    }

    /// The declaration of the name after `query.` (`"block_state"`).
    pub fn get_suffix(&self, suffix: &str) -> Option<&QueryDecl> {
        self.index_of_suffix(suffix).map(|index| self.decl(index))
    }

    /// Whether the catalogue declares the full name `name`.
    pub fn contains(&self, name: &str) -> bool {
        self.index_of(name).is_some()
    }

    /// Every declaration, in declaration order.
    pub fn iter(&self) -> std::slice::Iter<'_, QueryDecl> {
        self.inner.decls.iter()
    }

    /// The number of declarations.
    pub fn len(&self) -> usize {
        self.inner.decls.len()
    }

    /// Whether the catalogue declares nothing.
    pub fn is_empty(&self) -> bool {
        self.inner.decls.is_empty()
    }

    /// Whether `self` and `other` are the same catalogue; what `==` compares.
    pub fn same(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.inner, &other.inner)
    }

    pub(crate) fn index_of(&self, name: &str) -> Option<QueryIndex> {
        let Inner { decls, by_name, .. } = &*self.inner;
        by_name
            .find(decls, |decl| decl.name().cmp(name))
            .map(QueryIndex)
    }

    /// Every name has the `query.` prefix, so the order by name is the order by suffix.
    pub(crate) fn index_of_suffix(&self, suffix: &str) -> Option<QueryIndex> {
        let Inner { decls, by_name, .. } = &*self.inner;
        by_name
            .find(decls, |decl| decl.suffix().cmp(suffix))
            .map(QueryIndex)
    }

    pub(crate) fn decl(&self, index: QueryIndex) -> &QueryDecl {
        &self.inner.decls[index.index()]
    }
}

impl PartialEq for QueryCatalog {
    fn eq(&self, other: &Self) -> bool {
        self.same(other)
    }
}

impl Eq for QueryCatalog {}

impl Hash for QueryCatalog {
    fn hash<S: Hasher>(&self, state: &mut S) {
        Arc::as_ptr(&self.inner).hash(state);
    }
}

impl fmt::Debug for QueryCatalog {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("QueryCatalog")
            .field("side", &self.side())
            .field("len", &self.len())
            .finish_non_exhaustive()
    }
}

impl<'a> IntoIterator for &'a QueryCatalog {
    type Item = &'a QueryDecl;
    type IntoIter = std::slice::Iter<'a, QueryDecl>;

    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

/// Why a catalogue was not built.
#[derive(Error, Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum CatalogError {
    /// The name is declared twice.
    #[error("{0} is declared twice")]
    Duplicate(Box<str>),
    /// There are more than 65,536 declarations.
    #[error("a catalogue holds at most 65,536 queries")]
    Full,
    /// A replacing declaration names a query the catalogue does not declare.
    #[error("{0} is not declared, so it cannot be overridden")]
    Undeclared(Box<str>),
}

/// The index of a standard query in `stdlib::queries(Side::Client)`.
#[cfg(all(test, feature = "compiler", feature = "stdlib"))]
pub(crate) fn stdlib_index(name: &str) -> QueryIndex {
    crate::stdlib::queries(Side::Client)
        .index_of(name)
        .unwrap_or_else(|| panic!("{name} is not a standard query"))
}

#[cfg(test)]
pub(crate) mod test_support {
    use proptest::prelude::*;

    pub(crate) fn config() -> ProptestConfig {
        let cases = std::env::var("PROPTEST_CASES")
            .ok()
            .and_then(|c| c.parse().ok())
            .unwrap_or(512);
        ProptestConfig {
            cases,
            failure_persistence: None,
            ..ProptestConfig::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::{Arity, QueryShape};
    use crate::stdlib::query;

    fn decl(name: &str) -> QueryDecl {
        QueryDecl::new(name, QueryShape::DEFAULT).unwrap()
    }

    #[test]
    fn overriding_replaces_declarations_in_place_in_a_new_catalogue() {
        let base = QueryCatalog::new(
            Side::Client,
            [decl("query.a"), decl("query.b"), decl("query.c")],
        )
        .unwrap();
        let b = QueryDecl::new(
            "query.b",
            QueryShape {
                args: Arity::exactly(2),
                ..QueryShape::DEFAULT
            },
        )
        .unwrap();
        let catalog = base.overriding([b.clone()]).unwrap();
        assert_eq!(
            catalog.iter().map(QueryDecl::name).collect::<Vec<_>>(),
            ["query.a", "query.b", "query.c"]
        );
        assert_eq!(catalog.get("query.b"), Some(&b));
        assert_eq!(
            catalog.index_of("query.b"),
            base.index_of("query.b"),
            "at its position"
        );
        assert_eq!(catalog.get("query.a"), base.get("query.a"));
        assert_eq!(catalog.side(), Side::Client);
        assert!(catalog != base && !catalog.same(&base), "a new identity");
        assert_ne!(
            base.overriding([]).unwrap(),
            base,
            "even with nothing replaced"
        );
        assert_eq!(
            base.overriding([decl("query.d")]).err(),
            Some(CatalogError::Undeclared("query.d".into()))
        );
        assert_eq!(
            base.overriding([b.clone(), b]).err(),
            Some(CatalogError::Duplicate("query.b".into()))
        );
        assert_eq!(
            CatalogError::Undeclared("query.d".into()).to_string(),
            "query.d is not declared, so it cannot be overridden"
        );
    }

    #[test]
    fn a_side_prints_its_name() {
        assert_eq!(
            (Side::Server.to_string(), Side::Client.to_string()),
            ("server".to_owned(), "client".to_owned())
        );
    }

    #[test]
    fn no_declaration_makes_an_empty_catalogue() {
        let catalog = QueryCatalog::new(Side::Server, []).unwrap();
        assert!(catalog.is_empty());
        assert_eq!((catalog.len(), catalog.side()), (0, Side::Server));
        assert!(catalog.get("query.is_baby").is_none());
    }

    #[test]
    fn declarations_are_found_by_name_and_suffix_in_declaration_order() {
        let catalog =
            QueryCatalog::new(Side::Client, [decl("query.zeta"), decl("query.alpha")]).unwrap();
        assert_eq!(
            catalog.iter().map(QueryDecl::name).collect::<Vec<_>>(),
            ["query.zeta", "query.alpha"]
        );
        assert_eq!(
            catalog.get("query.alpha").map(QueryDecl::name),
            Some("query.alpha")
        );
        assert_eq!(
            catalog.get_suffix("zeta").map(QueryDecl::name),
            Some("query.zeta")
        );
        assert!(catalog.contains("query.zeta") && !catalog.contains("query.beta"));
        for bad in ["alpha", "query.alph", "q.alpha", "QUERY.ALPHA", ""] {
            assert!(catalog.get(bad).is_none(), "{bad:?}");
        }
        assert_eq!((&catalog).into_iter().count(), 2);
    }

    #[test]
    fn a_name_declared_twice_is_refused() {
        let other = QueryDecl::new(
            "query.a",
            QueryShape {
                args: Arity::exactly(1),
                ..QueryShape::DEFAULT
            },
        )
        .unwrap();
        assert_eq!(
            QueryCatalog::new(Side::Client, [decl("query.a"), decl("query.b"), other]).err(),
            Some(CatalogError::Duplicate("query.a".into()))
        );
        assert_eq!(
            CatalogError::Duplicate("query.a".into()).to_string(),
            "query.a is declared twice"
        );
    }

    #[test]
    fn a_catalogue_holds_at_most_65536_declarations() {
        let many = |n: usize| (0..n).map(|i| decl(&format!("query.q{i}")));
        assert_eq!(
            QueryCatalog::new(Side::Client, many(65_536)).map(|c| c.len()),
            Ok(65_536)
        );
        assert_eq!(
            QueryCatalog::new(Side::Client, many(65_537)).err(),
            Some(CatalogError::Full)
        );
    }

    #[test]
    fn extending_keeps_the_declarations_and_the_side_and_makes_a_new_catalogue() {
        let base = crate::stdlib::queries(Side::Server);
        let extended = base.extended([decl("query.my_thing")]).unwrap();
        assert_eq!(extended.side(), Side::Server);
        assert_eq!(extended.len(), base.len() + 1);
        assert!(base.iter().zip(extended.iter()).all(|(a, b)| a == b));
        assert!(extended.contains("query.my_thing") && !base.contains("query.my_thing"));
        assert_eq!(
            base.extended([decl(query::IS_BABY)]).err(),
            Some(CatalogError::Duplicate(query::IS_BABY.into())),
            "a standard query name cannot be declared again"
        );
        assert_ne!(&extended, base);
        assert_ne!(
            base.extended([]).unwrap(),
            *base,
            "a new identity even with nothing added"
        );
    }

    #[test]
    fn catalogues_compare_and_hash_by_identity() {
        use std::collections::hash_map::DefaultHasher;
        let hash = |c: &QueryCatalog| {
            let mut h = DefaultHasher::new();
            c.hash(&mut h);
            h.finish()
        };
        let a = QueryCatalog::new(Side::Client, []).unwrap();
        let b = QueryCatalog::new(Side::Client, []).unwrap();
        assert_ne!(a, b, "equal contents, different catalogues");
        let clone = a.clone();
        assert_eq!(a, clone);
        assert!(a.same(&clone) && !a.same(&b));
        assert_eq!(hash(&a), hash(&clone));
        assert!(
            std::ptr::eq(
                crate::stdlib::queries(Side::Client),
                crate::stdlib::queries(Side::Client)
            ),
            "built once"
        );
        assert_ne!(
            crate::stdlib::queries(Side::Client),
            crate::stdlib::queries(Side::Server)
        );
        assert_eq!(
            format!("{a:?}"),
            "QueryCatalog { side: Client, len: 0, .. }"
        );
    }

    #[test]
    fn a_catalogue_is_send_and_sync() {
        fn shared<T: Send + Sync + Clone>() {}
        shared::<QueryCatalog>();
    }
}
