//! [`MathCatalog`]: `math.*` functions a host declares next to the standard ones.

use std::fmt;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

use thiserror::Error;

use super::{Arity, ByName, MAX_DECLS, is_canonical_name};
use crate::ops::ExpressionOp;
use crate::rng::rand_core::Rng;

/// The most arguments a host math function takes.
pub const MAX_MATH_ARGS: u8 = 8;

/// The signature of a pure host math function: the arguments (see [`MathDecl`]).
pub type PureMathFn = dyn Fn(&[f32]) -> f32 + Send + Sync;

/// The signature of a volatile host math function: the evaluation's random source, then the
/// arguments (see [`MathDecl`]). [`rng::sample`](crate::rng::sample) draws a sample as the
/// standard functions do.
pub type VolatileMathFn = dyn Fn(&mut dyn Rng, &[f32]) -> f32 + Send + Sync;

/// How a host math function is evaluated.
#[derive(Clone)]
pub enum MathImpl {
    /// Same arguments, same result: folded when every argument is constant, merged as a term.
    Pure(Arc<PureMathFn>),
    /// Never folded or merged; counts as a side effect; draws from the evaluation's random source.
    Volatile(Arc<VolatileMathFn>),
}

impl fmt::Debug for MathImpl {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Pure(_) => "Pure",
            Self::Volatile(_) => "Volatile",
        })
    }
}

/// One host math function: its name, its argument counts and its implementation.
///
/// - **Arguments.** The slice holds exactly the arguments the call was written with, in source
///   order; the compiler rejects a call whose count is outside the declared [`Arity`].
/// - **Calls.** For a pure function the number of calls is unspecified: equal calls may be merged
///   into one, and a call with constant arguments runs at compile time instead of at evaluation.
///   Its result must depend on the arguments only. A volatile function runs once
///   per call the evaluation reaches.
/// - **Panics** are not caught: one unwinds out of `compile` (while folding) or out of the
///   evaluation.
/// - **Overrides.** A function may take a standard function's name (`math.sin`), but not `math.pi`,
///   a constant. With the catalogue in the options it replaces the standard function wherever the
///   full name appears (`math.sinx` stays the standard function followed by `x`). It is an ordinary
///   host call: folded only when pure, and allowed or denied by [`OpSet`](crate::ops::OpSet) as
///   `HostMath` or `HostMathVolatile`, so a pure override of `math.random` compiles where
///   [`OpSet::without_assignments_or_random`](crate::ops::OpSet::without_assignments_or_random)
///   forbids the standard function. Without the catalogue the standard function is used.
///
/// ```
/// use molangx::catalog::{Arity, MathDecl};
///
/// let twice = MathDecl::pure("math.twice", Arity::exactly(1), |a| a[0] * 2.0)?;
/// assert_eq!(
///     (twice.name(), twice.args(), twice.is_volatile()),
///     ("math.twice", Arity::exactly(1), false)
/// );
/// assert!(MathDecl::pure("math.sin", Arity::exactly(1), |a| a[0] * 2.0).is_ok(), "an override");
/// assert!(MathDecl::pure("math.pi", Arity::exactly(1), |a| a[0]).is_err(), "a constant");
/// # Ok::<(), molangx::catalog::MathError>(())
/// ```
#[derive(Clone, Debug)]
pub struct MathDecl {
    name: Box<str>,
    args: Arity,
    imp: MathImpl,
}

impl MathDecl {
    /// A pure function `name` taking `args` arguments.
    pub fn pure(
        name: &str,
        args: Arity,
        f: impl Fn(&[f32]) -> f32 + Send + Sync + 'static,
    ) -> Result<Self, MathError> {
        Self::new(name, args, MathImpl::Pure(Arc::new(f)))
    }

    /// A volatile function `name` taking `args` arguments, given the evaluation's random source.
    pub fn volatile(
        name: &str,
        args: Arity,
        f: impl Fn(&mut dyn Rng, &[f32]) -> f32 + Send + Sync + 'static,
    ) -> Result<Self, MathError> {
        Self::new(name, args, MathImpl::Volatile(Arc::new(f)))
    }

    fn new(name: &str, args: Arity, imp: MathImpl) -> Result<Self, MathError> {
        if !is_canonical_name(name, "math.") {
            return Err(MathError::Name(name.into()));
        }
        if Some(name) == ExpressionOp::Pi.token() {
            return Err(MathError::Constant(name.into()));
        }
        if args.min() == 0 || args.max().is_none_or(|max| max > MAX_MATH_ARGS) {
            return Err(MathError::Arity {
                name: name.into(),
                args,
            });
        }
        Ok(Self {
            name: name.into(),
            args,
            imp,
        })
    }

    /// The full name (`"math.my_function"`).
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The argument counts: at least 1, at most [`MAX_MATH_ARGS`].
    pub fn args(&self) -> Arity {
        self.args
    }

    /// Whether the function is volatile.
    pub fn is_volatile(&self) -> bool {
        matches!(self.imp, MathImpl::Volatile(_))
    }

    /// The implementation.
    pub fn implementation(&self) -> &MathImpl {
        &self.imp
    }
}

/// A host math function of one catalogue: its position there. Public only through
/// `molangx::internals`.
///
/// Only a lookup in a catalogue makes one, and a compiled expression keeps the catalogue it was
/// compiled with.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub struct MathRef(u16);

impl MathRef {
    /// The position in the catalogue, in declaration order.
    pub fn index(self) -> usize {
        usize::from(self.0)
    }
}

struct Inner {
    decls: Box<[MathDecl]>,
    by_name: ByName,
}

/// An immutable set of host math functions, passed to the compiler in
/// [`CompileOptions::math`](crate::compile::CompileOptions::math).
///
/// Cheap to clone (a shared handle). Compared and hashed by **identity**: clones are equal, two
/// catalogues with equal declarations are not. A compiled expression keeps its catalogue.
///
/// ```
/// use molangx::catalog::{Arity, MathCatalog, MathDecl};
/// use molangx::rng::sample;
///
/// let math = MathCatalog::new([
///     MathDecl::pure("math.twice", Arity::exactly(1), |a| a[0] * 2.0)?,
///     MathDecl::volatile("math.jitter", Arity::exactly(1), |rng, a| a[0] + sample(rng))?,
/// ])?;
/// assert_eq!(math.len(), 2);
/// assert!(math.get("math.jitter").is_some_and(|f| f.is_volatile()));
/// assert_ne!(
///     math,
///     MathCatalog::new([MathDecl::pure("math.twice", Arity::exactly(1), |a| a[0] * 2.0)?])?
/// );
/// # Ok::<(), molangx::catalog::MathError>(())
/// ```
#[derive(Clone)]
pub struct MathCatalog {
    inner: Arc<Inner>,
}

impl MathCatalog {
    /// The catalogue of `decls`, in that order; an error when a name is declared twice or there
    /// are more than 65,536.
    pub fn new(decls: impl IntoIterator<Item = MathDecl>) -> Result<Self, MathError> {
        let decls: Box<[MathDecl]> = decls.into_iter().collect();
        if decls.len() > MAX_DECLS {
            return Err(MathError::Full);
        }
        let by_name = ByName::new(&decls, MathDecl::name);
        if let Some([first, _]) = by_name
            .0
            .windows(2)
            .map(|pair| [pair[0], pair[1]].map(|i| &decls[usize::from(i)]))
            .find(|[a, b]| a.name == b.name)
        {
            return Err(MathError::Duplicate(first.name.clone()));
        }
        Ok(Self {
            inner: Arc::new(Inner { decls, by_name }),
        })
    }

    /// The declaration of the full name `name` (`"math.my_function"`).
    pub fn get(&self, name: &str) -> Option<&MathDecl> {
        self.find(name).map(|found| self.decl(found))
    }

    /// Whether the catalogue declares `name`.
    pub fn contains(&self, name: &str) -> bool {
        self.find(name).is_some()
    }

    /// Every declaration, in declaration order.
    pub fn iter(&self) -> std::slice::Iter<'_, MathDecl> {
        self.inner.decls.iter()
    }

    /// The number of declarations.
    pub fn len(&self) -> usize {
        self.inner.decls.len()
    }

    /// Whether the catalogue declares no function.
    pub fn is_empty(&self) -> bool {
        self.inner.decls.is_empty()
    }

    /// Whether `self` and `other` are the same catalogue; what `==` compares.
    pub fn same(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.inner, &other.inner)
    }

    /// The function named `name`.
    pub(crate) fn find(&self, name: &str) -> Option<MathRef> {
        let Inner { decls, by_name } = &*self.inner;
        by_name
            .find(decls, |decl| decl.name().cmp(name))
            .map(MathRef)
    }

    /// The function `r` refers to; `r` came from this catalogue.
    pub(crate) fn decl(&self, r: MathRef) -> &MathDecl {
        &self.inner.decls[r.index()]
    }
}

impl PartialEq for MathCatalog {
    fn eq(&self, other: &Self) -> bool {
        self.same(other)
    }
}

impl Eq for MathCatalog {}

impl Hash for MathCatalog {
    fn hash<S: Hasher>(&self, state: &mut S) {
        Arc::as_ptr(&self.inner).hash(state);
    }
}

impl fmt::Debug for MathCatalog {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_set()
            .entries(self.iter().map(MathDecl::name))
            .finish()
    }
}

impl<'a> IntoIterator for &'a MathCatalog {
    type Item = &'a MathDecl;
    type IntoIter = std::slice::Iter<'a, MathDecl>;

    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

/// Why a math function was not declared or a math catalogue not built.
#[derive(Error, Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum MathError {
    /// The name is not `math.` followed by a lower-case letter or `_`, then lower-case letters,
    /// digits or `_`.
    #[error(
        "`{0}` is not a canonical math function name: `math.` followed by a lower-case letter or `_`, then lower-case letters, digits or `_`"
    )]
    Name(Box<str>),
    /// The name is `math.pi`, a constant written without parentheses: no function can take its
    /// place.
    #[error("{0} is a constant, not a function, and cannot be declared")]
    Constant(Box<str>),
    /// The argument counts are not within 1 to [`MAX_MATH_ARGS`].
    #[error("{name} must take 1 to {MAX_MATH_ARGS} arguments")]
    Arity {
        /// The declared name.
        name: Box<str>,
        /// The declared argument counts.
        args: Arity,
    },
    /// The name is declared twice.
    #[error("{0} is declared twice")]
    Duplicate(Box<str>),
    /// There are more than 65,536 declarations.
    #[error("a math catalogue holds at most 65,536 functions")]
    Full,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rng::{FixedRng, sample};
    use crate::stdlib::MathFn;

    fn pure(name: &str) -> MathDecl {
        MathDecl::pure(name, Arity::exactly(1), |a| a[0]).unwrap()
    }

    #[test]
    fn a_declaration_keeps_its_name_counts_and_kind() {
        let f = MathDecl::pure("math.f", Arity::between(1, 3), |a| a.iter().sum()).unwrap();
        assert_eq!(
            (f.name(), f.args(), f.is_volatile()),
            ("math.f", Arity::between(1, 3), false)
        );
        let MathImpl::Pure(call) = f.implementation() else {
            panic!("pure")
        };
        assert_eq!(call(&[1.0, 2.0, 3.0]), 6.0);
        let g = MathDecl::volatile("math.g", Arity::exactly(MAX_MATH_ARGS), |rng, a| {
            a[7] + sample(rng)
        })
        .unwrap();
        assert!(g.is_volatile());
        let MathImpl::Volatile(call) = g.implementation() else {
            panic!("volatile")
        };
        let mut half = FixedRng::HALF;
        assert_eq!(
            call(&mut half, &[0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0]),
            1.5
        );
        assert_eq!(format!("{:?}", g.implementation()), "Volatile");
        assert!(format!("{f:?}").contains("Pure"));
    }

    #[test]
    fn a_name_must_be_canonical() {
        for bad in [
            "", "math.", "f", "math.F", "Math.f", "math.1f", "math.f-g", "math.f.g", "query.f",
            "math. f", "m.f",
        ] {
            assert_eq!(
                MathDecl::pure(bad, Arity::exactly(1), |a| a[0]).err(),
                Some(MathError::Name(bad.into())),
                "{bad:?}"
            );
        }
        for good in ["math._", "math.f1", "math.my_function", "math.sinh"] {
            assert!(
                MathDecl::pure(good, Arity::exactly(1), |a| a[0]).is_ok(),
                "{good:?}"
            );
        }
        assert_eq!(
            MathError::Name("math.F".into()).to_string(),
            "`math.F` is not a canonical math function name: `math.` followed by a lower-case letter or `_`, then lower-case letters, digits or `_`"
        );
    }

    #[test]
    fn every_standard_function_name_can_be_declared_but_the_constant() {
        for &standard in MathFn::all() {
            let name = standard.token();
            let declared = MathDecl::volatile(name, Arity::exactly(1), |_, a| a[0]);
            if standard == MathFn::Pi {
                assert_eq!(declared.err(), Some(MathError::Constant(name.into())));
            } else {
                assert!(declared.is_ok(), "{name}");
            }
        }
        assert_eq!(
            MathError::Constant("math.pi".into()).to_string(),
            "math.pi is a constant, not a function, and cannot be declared"
        );
    }

    #[test]
    fn the_argument_counts_are_within_one_to_the_maximum() {
        let bad = [
            Arity::ANY,
            Arity::exactly(0),
            Arity::at_least(1),
            Arity::exactly(MAX_MATH_ARGS + 1),
            Arity::between(0, 2),
            Arity::between(2, 9),
        ];
        for args in bad {
            assert_eq!(
                MathDecl::pure("math.f", args, |a| a[0]).err(),
                Some(MathError::Arity {
                    name: "math.f".into(),
                    args
                }),
                "{args:?}"
            );
        }
        for args in [
            Arity::exactly(1),
            Arity::exactly(MAX_MATH_ARGS),
            Arity::between(1, MAX_MATH_ARGS),
        ] {
            assert!(MathDecl::pure("math.f", args, |a| a[0]).is_ok(), "{args:?}");
        }
        assert_eq!(MAX_MATH_ARGS, 8);
        assert_eq!(
            MathError::Arity {
                name: "math.f".into(),
                args: Arity::ANY
            }
            .to_string(),
            "math.f must take 1 to 8 arguments"
        );
    }

    #[test]
    fn the_host_math_ops_take_what_a_declaration_may_declare() {
        for op in [ExpressionOp::HostMath, ExpressionOp::HostMathVolatile] {
            assert_eq!(
                (op.min_children(), op.max_children()),
                (1, Some(MAX_MATH_ARGS)),
                "{op:?}"
            );
        }
    }

    #[test]
    fn a_catalogue_needs_distinct_names_and_may_be_empty() {
        let empty = MathCatalog::new([]).unwrap();
        assert!(empty.is_empty() && empty.get("math.a").is_none());
        assert_eq!(empty.len(), 0);
        assert_eq!(
            MathCatalog::new([pure("math.a"), pure("math.b"), pure("math.a")]).err(),
            Some(MathError::Duplicate("math.a".into()))
        );
        assert_eq!(
            MathError::Duplicate("math.a".into()).to_string(),
            "math.a is declared twice"
        );
    }

    #[test]
    fn a_catalogue_holds_at_most_65536_functions() {
        let many = |n: usize| (0..n).map(|i| pure(&format!("math.f{i}")));
        assert_eq!(MathCatalog::new(many(65_536)).map(|c| c.len()), Ok(65_536));
        assert_eq!(MathCatalog::new(many(65_537)).err(), Some(MathError::Full));
        assert_eq!(
            MathError::Full.to_string(),
            "a math catalogue holds at most 65,536 functions"
        );
    }

    #[test]
    fn functions_are_found_by_full_name_in_declaration_order() {
        let math = MathCatalog::new([pure("math.zeta"), pure("math.alpha"), pure("math.alphabet")])
            .unwrap();
        assert_eq!(
            math.iter().map(MathDecl::name).collect::<Vec<_>>(),
            ["math.zeta", "math.alpha", "math.alphabet"]
        );
        assert_eq!((&math).into_iter().count(), 3);
        assert_eq!((math.len(), math.is_empty()), (3, false));
        assert_eq!(
            math.get("math.alpha").map(MathDecl::name),
            Some("math.alpha")
        );
        assert_eq!(math.find("math.zeta").map(MathRef::index), Some(0));
        assert_eq!(
            math.find("math.alphabet").map(|r| math.decl(r).name()),
            Some("math.alphabet")
        );
        assert!(math.contains("math.zeta"));
        for miss in [
            "alpha",
            "math.alph",
            "math.alphab",
            "MATH.ALPHA",
            "math.sin",
            "",
        ] {
            assert!(math.get(miss).is_none(), "{miss:?}");
        }
        assert_eq!(
            format!("{math:?}"),
            r#"{"math.zeta", "math.alpha", "math.alphabet"}"#
        );
    }

    #[test]
    fn catalogues_compare_and_hash_by_identity() {
        use std::collections::hash_map::DefaultHasher;
        let hash = |c: &MathCatalog| {
            let mut h = DefaultHasher::new();
            c.hash(&mut h);
            h.finish()
        };
        let a = MathCatalog::new([pure("math.f")]).unwrap();
        let b = MathCatalog::new([pure("math.f")]).unwrap();
        assert_ne!(a, b, "equal contents, different catalogues");
        let clone = a.clone();
        assert_eq!(a, clone);
        assert!(a.same(&clone) && !a.same(&b));
        assert_eq!(hash(&a), hash(&clone));
        assert_ne!(hash(&a), hash(&b));
    }

    #[test]
    fn the_types_are_send_and_sync() {
        fn shared<T: Send + Sync + Clone>() {}
        shared::<MathCatalog>();
        shared::<MathDecl>();
        shared::<MathImpl>();
        shared::<MathRef>();
        shared::<MathError>();
    }
}
