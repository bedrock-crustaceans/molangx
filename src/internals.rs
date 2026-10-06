//! Crate-private items the fuzz crate needs (feature `fuzz`); outside the `SemVer` guarantee.

#[cfg(feature = "compiler")]
pub use crate::catalog::math::MathRef;
#[cfg(feature = "compiler")]
pub use crate::compile::ast::{Name, Node, Payload, QueryRef};
#[cfg(feature = "compiler")]
pub use crate::compile::probe::tokens;

#[cfg(feature = "vm")]
use crate::catalog::QueryCatalog;
#[cfg(feature = "compiler")]
use crate::catalog::{MathCatalog, MathDecl};
#[cfg(feature = "compiler")]
use crate::compile::Expr;
#[cfg(feature = "vm")]
use crate::hash::HashedStr;
#[cfg(feature = "vm")]
use crate::version::MolangVersion;
#[cfg(feature = "vm")]
use crate::vm::{
    EvalCx, Host, MemberStoreCheck, QueryBackend, QueryCx, Subjects, Value, VariableName,
};

/// `stdlib::queries(Side::Client)` extended with the test helper queries, and the test host
/// math functions.
pub mod reference_catalog {
    pub use crate::reference_catalog::{
        ADMISSION, EXPERIMENTAL_TEST, EXPERIMENTS, GET_NAME_TEST, SETS, SUM_TEST, catalog,
    };
    #[cfg(feature = "compiler")]
    pub use crate::reference_catalog::{math, options};
}

/// The host math function `r` refers to in `math`, the catalogue `r` came from.
#[cfg(feature = "compiler")]
pub fn math_decl(math: &MathCatalog, r: MathRef) -> &MathDecl {
    math.decl(r)
}

/// The optimised tree of `expr`; `None` for a rejected expression.
#[cfg(feature = "compiler")]
pub fn tree(expr: &Expr) -> Option<&Node> {
    expr.tree()
}

/// The context of a call to the compile-time resolved `query`, run at `version` over `backend`.
#[cfg(feature = "vm")]
pub fn query_cx<'a, 'w, H: Host>(
    catalog: &'a QueryCatalog,
    query: QueryRef,
    version: MolangVersion,
    backend: &'a mut dyn QueryBackend<'w, H>,
) -> QueryCx<'a, 'w, H> {
    QueryCx::resolved(catalog, query.index, version, query.impl_idx, backend)
}

/// The subjects `->` switches to for the left-side value `target`; `None` when it has none.
#[cfg(feature = "vm")]
pub fn arrow_target<H: Host>(cx: &EvalCx<'_, '_, H>, target: &Value<H>) -> Option<Subjects<H>> {
    cx.arrow_target(target)
}

/// What a `variable.` read sees inside `->`: the subject actor's public snapshot.
#[cfg(feature = "vm")]
pub fn public_variable<'c, H: Host>(
    cx: &'c EvalCx<'_, '_, H>,
    name: VariableName,
) -> Option<&'c Value<H>> {
    cx.public_variable(name)
}

/// The form of `value` an assignment keeps in a variable or temp.
#[cfg(feature = "vm")]
pub fn storable<H: Host>(cx: &EvalCx<'_, '_, H>, value: Value<H>) -> Value<H> {
    cx.storable(value)
}

/// The step-budget cost of storing `value` in a variable or temp, beyond the store instruction.
#[cfg(feature = "vm")]
pub fn store_cost<H: Host>(value: &Value<H>) -> u64 {
    value.store_cost()
}

/// The step-budget cost of storing a member at `path` inside `value`, and the width budget it
/// exceeds by adding a member to a struct that already holds `width_limit` members.
#[cfg(feature = "vm")]
pub fn member_store_check<H: Host>(
    value: &Value<H>,
    path: &[HashedStr],
    width_limit: Option<u32>,
) -> MemberStoreCheck {
    value.member_store_check(path, width_limit)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(feature = "compiler")]
    use crate::compile::{CompileOptions, compile};
    #[cfg(feature = "compiler")]
    use crate::numeric::PostOp;
    #[cfg(feature = "compiler")]
    use crate::ops::ExpressionOp;
    use crate::version::MolangVersion;

    #[cfg(feature = "compiler")]
    #[test]
    fn the_tree_is_the_optimised_tree_and_its_accessors_read_it() {
        let compiled = compile(
            "v.x * 2 + 1",
            &CompileOptions::server(MolangVersion::LATEST),
        );
        let root = tree(compiled.expr().expect("compiles")).expect("a tree");
        assert_eq!(root.op(), ExpressionOp::EntityVariable);
        assert!(matches!(root.value(), Payload::Entity(name) if name.as_str() == "variable.x"));
        assert!(root.children().is_empty());
        assert_eq!(root.post(), PostOp::new(2.0, 1.0));
        let rejected = compile("1 +", &CompileOptions::server(MolangVersion::LATEST));
        assert!(tree(rejected.expr_or_zero().expect("the constant 0")).is_none());
    }

    #[cfg(feature = "compiler")]
    #[test]
    fn the_reference_options_resolve_the_helper_queries() {
        let compiled = compile(
            "q.sum_test(1, 2) + q.experimental_test",
            &reference_catalog::options(13),
        );
        assert!(
            compiled.is_success() && compiled.diagnostics().is_empty(),
            "{:?}",
            compiled.diagnostics()
        );
        assert!(
            reference_catalog::catalog()
                .index_of(reference_catalog::GET_NAME_TEST)
                .is_some()
        );
    }

    #[cfg(feature = "compiler")]
    #[test]
    fn the_reference_options_carry_the_helper_math_functions() {
        let options = reference_catalog::options(13);
        let compiled = compile(
            "math.helper_mix(v.x, 4) + math.helper_sum(1, 2, 3, v.x) + math.helper_noise(1)",
            &options,
        );
        assert!(
            compiled.is_success() && compiled.diagnostics().is_empty(),
            "{:?}",
            compiled.diagnostics()
        );
        let constant = |src: &str| compile(src, &options).expr().and_then(Expr::as_constant);
        assert_eq!(
            (
                constant("math.helper_mix(2, 6)"),
                constant("math.helper_mix(6, 2)")
            ),
            (Some(1.5), Some(2.5))
        );
        assert_eq!(
            constant("math.helper_sum(1, 2, 3, 4, 5, 6, 7, 8)"),
            Some(204.0)
        );
        assert_eq!(
            constant("math.helper_sum(8, 7, 6, 5, 4, 3, 2, 1)"),
            Some(120.0)
        );
        let math = reference_catalog::math();
        let Some(Payload::HostMath(noise)) = tokens("math.helper_noise", &options)
            .ok()
            .and_then(|t| t.first().map(|(_, value)| value.clone()))
        else {
            panic!("a host math token");
        };
        assert_eq!(math_decl(math, noise).name(), "math.helper_noise");
        assert!(math_decl(math, noise).is_volatile());
    }

    #[cfg(feature = "compiler")]
    #[test]
    fn the_lexer_probes_see_the_lowered_tokens() {
        let options = CompileOptions::server(MolangVersion::LATEST);
        let lexed = tokens("V.X + 1", &options).expect("lexes");
        assert_eq!(
            lexed.iter().map(|(op, _)| *op).collect::<Vec<_>>(),
            [
                ExpressionOp::EntityVariable,
                ExpressionOp::Add,
                ExpressionOp::Float
            ]
        );
        assert!(tokens("$", &options).is_err_and(|message| !message.is_empty()));
    }

    #[cfg(feature = "vm")]
    #[test]
    fn the_store_costs_are_the_evaluators() {
        use crate::vm::{NoHost, StructValue};
        assert_eq!(store_cost(&Value::<NoHost>::actor_array([])), 0);
        assert_eq!(store_cost(&Value::<NoHost>::Float(1.0)), 0);
        let one = Value::<NoHost>::structure(StructValue::from([("a", Value::Float(1.0))]));
        let check = member_store_check(&one, &[HashedStr::new("b")], Some(1));
        assert!(check.cost > 0 && check.exceeded_width == Some(1));
    }
}
