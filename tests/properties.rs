//! Property tests (proptest). `PROPTEST_CASES` overrides each module's case count.

#![cfg(feature = "stdlib")]

mod common;

use molangx::catalog::{QuerySetMask, Side};
use proptest::prelude::*;

/// `PROPTEST_CASES` cases, else `default_cases`. No failure file: an integration test has no
/// `lib.rs` / `main.rs` for proptest to put it beside.
fn config(default_cases: u32) -> ProptestConfig {
    let cases = std::env::var("PROPTEST_CASES")
        .ok()
        .and_then(|c| c.parse().ok())
        .unwrap_or(default_cases);
    ProptestConfig {
        cases,
        failure_persistence: None,
        ..ProptestConfig::default()
    }
}

/// An index into the client's queries.
fn query() -> impl Strategy<Value = usize> {
    0..molangx::stdlib::queries(Side::Client).len()
}

fn raw_version() -> impl Strategy<Value = i16> {
    prop_oneof![3 => -3i16..=16, 1 => any::<i16>()]
}

fn sets() -> impl Strategy<Value = QuerySetMask> {
    prop::sample::select(vec![
        QuerySetMask::empty(),
        QuerySetMask::DEFAULT,
        QuerySetMask::TAGS,
        QuerySetMask::WORLD_GEN,
        QuerySetMask::DEFAULT.union(QuerySetMask::TAGS),
        QuerySetMask::BUILTIN,
    ])
}

#[cfg(feature = "vm")]
mod division_guard {
    //! The two division guards differ only for a negative run-time divisor, which the unsigned one
    //! (version ≤ 6) replaces by `|d|`. A literal divisor folds into a signed multiplication at
    //! every version.

    use crate::common::{compile_support::compile_expr, nan_or_same_bits};
    use molangx::compile::CompileOptions;
    use molangx::numeric::{self, PostOp};
    use molangx::version::{MolangVersion, RawVersion};
    use molangx::vm::{NoHostEnv, Value, VariableName};
    use proptest::prelude::*;

    /// Floats with the guard's edges, signed zeros, NaN and infinities well represented.
    fn float() -> impl Strategy<Value = f32> {
        prop_oneof![
            3 => any::<f32>(),
            2 => -1e6f32..1e6f32,
            2 => prop::sample::select(vec![
                0.0f32,
                -0.0,
                f32::EPSILON,
                -f32::EPSILON,
                f32::EPSILON * 0.5,
                -f32::EPSILON * 0.5,
                f32::from_bits(f32::EPSILON.to_bits() - 1),
                -f32::from_bits(f32::EPSILON.to_bits() - 1),
                1.0,
                -1.0,
                -2.5,
                f32::NAN,
                -f32::NAN,
                f32::INFINITY,
                f32::NEG_INFINITY,
                f32::MIN_POSITIVE,
                -f32::MIN_POSITIVE,
                f32::MAX,
                f32::MIN,
            ]),
        ]
    }

    /// −1 and 0–6 use `DivGuard { signed: false }`, 7–13 `DivGuard { signed: true }`.
    fn old_version() -> impl Strategy<Value = MolangVersion> {
        (-1i16..=6).prop_map(|raw| RawVersion(raw).effective())
    }

    fn new_version() -> impl Strategy<Value = MolangVersion> {
        (7i16..=13).prop_map(|raw| RawVersion(raw).effective())
    }

    fn run(source: &str, version: MolangVersion, a: f32, d: f32) -> f32 {
        let expr = compile_expr(source, &CompileOptions::server(version));
        let mut env = NoHostEnv::new();
        env.variables.set(VariableName::new("a"), Value::Float(a));
        env.variables.set(VariableName::new("d"), Value::Float(d));
        let value = expr.eval_f32(&mut env.cx());
        assert!(env.sink.is_empty(), "{source}: {:?}", env.sink.messages());
        value
    }

    fn negative_and_divides(d: f32) -> bool {
        d < 0.0 && numeric::div_guard(true, d).is_some()
    }

    proptest! {
        #![proptest_config(super::config(256))]

        #[test]
        fn guards_differ_only_for_negative_divisors(d in float()) {
            let abs = numeric::div_guard(false, d);
            let signed = numeric::div_guard(true, d);
            // Both fire on exactly the same divisors.
            prop_assert_eq!(abs.is_none(), signed.is_none());
            if let (Some(abs), Some(signed)) = (abs, signed) {
                // `|d|` clears the sign of a NaN divisor.
                if d < 0.0 {
                    prop_assert!(nan_or_same_bits(abs, -d) && nan_or_same_bits(signed, d), "{} {} {}", d, abs, signed);
                } else {
                    prop_assert!(nan_or_same_bits(abs, signed), "{} {} {}", d, abs, signed);
                }
            }
        }

        #[test]
        fn dynamic_division_differs_only_for_negative_divisors(
            old in old_version(),
            new in new_version(),
            a in float(),
            d in float(),
        ) {
            for (source, post) in [("v.a / v.d", PostOp::IDENTITY), ("(v.a / v.d) * 3 - 1", PostOp::new(3.0, -1.0))] {
                let before = run(source, old, a, d);
                let after = run(source, new, a, d);
                if negative_and_divides(d) {
                    // The old guard divides by |d|, the new one by d.
                    prop_assert!(nan_or_same_bits(before, numeric::div(a, -d, post)), "{source} v{old:?}: {before}");
                    prop_assert!(nan_or_same_bits(after, numeric::div(a, d, post)), "{source} v{new:?}: {after}");
                } else {
                    prop_assert!(nan_or_same_bits(before, after), "{source} with a = {a}, d = {d}: v{old:?} {before}, v{new:?} {after}");
                }
            }
        }

        /// A literal divisor folds into a signed multiplication at every version.
        #[test]
        fn literal_divisors_do_not_differ(
            old in old_version(),
            new in new_version(),
            a in float(),
            divisor in prop::sample::select(vec!["2", "0.5", "3", "0", "0.0000001", "1000"]),
            negative in any::<bool>(),
        ) {
            let source = if negative { format!("v.a / -{divisor}") } else { format!("v.a / {divisor}") };
            let before = run(&source, old, a, 0.0);
            let after = run(&source, new, a, 0.0);
            prop_assert!(nan_or_same_bits(before, after), "{source} with a = {a}: v{old:?} {before}, v{new:?} {after}");
        }
    }
}

#[cfg(feature = "vm")]
mod folding {
    //! Constant folding changes a result only in the classes the `numeric` module docs list: each
    //! generated expression folds with literals and is evaluated with every literal behind its own
    //! variable, and both must give the same bits. A kept constant, a factor, term or divisor of the
    //! generated shapes, stays a constant in the run-time form, so that it folds into its operand's
    //! post-op there.
    //!
    //! When the two forms differ, the tree is checked bottom-up: a node whose forms differ, its
    //! operands giving the same bits in both, must make one of these differences exactly, and is
    //! then replaced by a literal of its run-time value, so that the rest of the tree is still
    //! checked.
    //! - A negation, or a kept factor or term: zeros of different sign.
    //! - A negation of a NaN `n`: the fold gives `−n`, the run `n`.
    //! - A kept NaN term `c` left of a NaN `x`: the fold gives `c`, the run `x`.
    //! - `math.sign` under kept factors and terms: the fold is that of the node with the call
    //!   replaced by −1, the run the negated fold of the node with the call replaced by 1.
    //! - A division the guard stops under kept factors and terms: the fold is that of the node with
    //!   the division replaced by 0, the run +0.
    //! - A kept infinite factor: the fold is the constant product, the run NaN.
    //! - A kept divisor `d` of `x`: the fold is `x / d`, the run `x · (1/d)`; for `|d| < ε` the
    //!   fold is 0 and the run `x · 0`.
    //! - `Arm64` only: a kept factor or term that the run applies inside its operand's instruction:
    //!   with that constant behind a variable too, the run gives the folded bits.
    //! - `X86_64` only: a NaN divisor: the fold gives +0, the run the quotient.
    //! - `math.mod` with a zero divisor: the fold gives NaN, the run +0.
    //! - `math.mod` with a −0 remainder: the fold gives −0, the run +0.
    //!
    //! A sum directly inside a sum, or under a kept factor in one, is not generated: the literal
    //! form folds the inner sum first while the hidden form flattens it into three terms.
    //!
    //! `PROPTEST_RNG_SEED` (printed at the start) and `PROPTEST_CASES` replay a failure.

    use crate::common::per_arch;
    use molangx::compile::{CompileOptions, compile};
    use molangx::numeric::{self, ARCH, Arch, PostOp, arith};
    use molangx::stdlib::MATH_META;
    use molangx::version::MolangVersion;
    use molangx::vm::{NoHostEnv, Value, VariableName};
    use proptest::prelude::*;

    #[derive(Clone, Debug)]
    enum E {
        /// A literal: its value and the text that lexes to it.
        Lit(f32, String),
        Pi,
        Bin(&'static str, Box<E>, Box<E>),
        Neg(Box<E>),
        Not(Box<E>),
        Call(&'static str, Vec<E>),
        Cond(Box<E>, Box<E>, Box<E>),
        /// A constant that stays one in the run-time form: its value and its text.
        Kept(f32, String),
    }

    fn options() -> CompileOptions {
        CompileOptions::server(MolangVersion::LATEST)
    }

    fn fold(source: &str) -> Option<f32> {
        compile(source, &options()).expr().cloned()?.as_constant()
    }

    /// A text the lexer reads back as exactly `value`; negative values in parentheses, since a
    /// negated literal folds exactly.
    fn literal_text(value: f32) -> Option<String> {
        let magnitude = value.abs();
        let candidates = [
            format!("{magnitude:e}"),
            format!("{magnitude}"),
            format!("{magnitude:.8e}"),
            format!("{magnitude:.9e}"),
        ];
        let text = candidates
            .into_iter()
            .find(|text| fold(text).is_some_and(|read| read.to_bits() == magnitude.to_bits()))?;
        Some(if value.is_sign_negative() {
            format!("(-{text})")
        } else {
            text
        })
    }

    impl E {
        /// The source text; literals as numbers (`hide = false`) or as `v.kN` reads.
        fn render(&self, hide: bool, next: &mut usize, out: &mut Vec<f32>) -> String {
            match self {
                E::Lit(value, text) => {
                    out.push(*value);
                    *next += 1;
                    if hide {
                        format!("v.k{}", *next - 1)
                    } else {
                        text.clone()
                    }
                }
                // Hidden too, so that nothing folds in the hidden form (a constant divisor would).
                E::Pi if hide => E::Lit(fold("math.pi").unwrap_or(f32::NAN), String::new())
                    .render(hide, next, out),
                E::Pi => "math.pi".to_owned(),
                E::Bin(op, a, b) => format!(
                    "({} {op} {})",
                    a.render(hide, next, out),
                    b.render(hide, next, out)
                ),
                E::Neg(a) => format!("(-{})", a.render(hide, next, out)),
                E::Not(a) => format!("(!{})", a.render(hide, next, out)),
                E::Call(f, args) => {
                    let args: Vec<String> =
                        args.iter().map(|a| a.render(hide, next, out)).collect();
                    format!("{f}({})", args.join(", "))
                }
                E::Cond(c, a, b) => format!(
                    "({} ? {} : {})",
                    c.render(hide, next, out),
                    a.render(hide, next, out),
                    b.render(hide, next, out)
                ),
                E::Kept(_, text) => text.clone(),
            }
        }

        /// Whether a `+` / `-` has an operand that is itself a sum, possibly under unary minus or a
        /// kept factor.
        fn has_nested_sum(&self) -> bool {
            fn is_sum(e: &E) -> bool {
                match e {
                    E::Bin("+" | "-", ..) => true,
                    E::Neg(a) => is_sum(a),
                    E::Bin("*", a, b) => match (&**a, &**b) {
                        (E::Kept(..), other) | (other, E::Kept(..)) => is_sum(other),
                        _ => false,
                    },
                    _ => false,
                }
            }
            let nested_here = match self {
                E::Bin("+" | "-", a, b) => is_sum(a) || is_sum(b),
                _ => false,
            };
            nested_here
                || match self {
                    E::Lit(..) | E::Pi | E::Kept(..) => false,
                    E::Bin(_, a, b) => a.has_nested_sum() || b.has_nested_sum(),
                    E::Neg(a) | E::Not(a) => a.has_nested_sum(),
                    E::Call(_, args) => args.iter().any(E::has_nested_sum),
                    E::Cond(c, a, b) => {
                        c.has_nested_sum() || a.has_nested_sum() || b.has_nested_sum()
                    }
                }
        }

        fn source(&self) -> String {
            self.render(false, &mut 0, &mut Vec::new())
        }

        /// The case with each node whose two forms differ as listed replaced, bottom-up, by a
        /// literal of its run-time value, so that the rest of the tree is still checked; an error
        /// for the first node that differs otherwise.
        fn resolved(&self) -> Result<E, TestCaseError> {
            let node = self.with_children(E::resolved)?;
            let (folded, run) = forms(&node)?;
            if same(folded, run) {
                return Ok(node);
            }
            if node.own_difference(folded, run).is_none() {
                return Err(TestCaseError::fail(format!(
                    "{} folds to {folded} ({:#010x}), runs to {run} ({:#010x}); with the literals \
                     behind variables: {}",
                    node.source(),
                    folded.to_bits(),
                    run.to_bits(),
                    node.render(true, &mut 0, &mut Vec::new())
                )));
            }
            constant_text(run)
                .map(|text| E::Lit(run, text))
                .ok_or_else(|| {
                    TestCaseError::fail(format!("no text folds to {:#010x}", run.to_bits()))
                })
        }

        /// The node with `f` applied to each operand.
        fn with_children(
            &self,
            f: impl Fn(&E) -> Result<E, TestCaseError>,
        ) -> Result<E, TestCaseError> {
            let boxed = |e: &E| f(e).map(Box::new);
            Ok(match self {
                E::Lit(..) | E::Pi | E::Kept(..) => self.clone(),
                E::Bin(op, a, b) => E::Bin(op, boxed(a)?, boxed(b)?),
                E::Neg(a) => E::Neg(boxed(a)?),
                E::Not(a) => E::Not(boxed(a)?),
                E::Call(name, args) => {
                    E::Call(name, args.iter().map(&f).collect::<Result<_, _>>()?)
                }
                E::Cond(c, a, b) => E::Cond(boxed(c)?, boxed(a)?, boxed(b)?),
            })
        }

        /// The listed difference that this node makes itself, its operands giving the same bits in
        /// both forms, if any.
        fn own_difference(&self, folded: f32, run: f32) -> Option<&'static str> {
            self.division_difference(folded, run)
                .or_else(|| self.post_op_difference(folded, run))
        }

        /// A literal divisor, a NaN divisor on `X86_64`, and `math.mod`'s zero divisor and −0
        /// remainder.
        fn division_difference(&self, folded: f32, run: f32) -> Option<&'static str> {
            match self {
                E::Bin("/", a, d) if matches!(**d, E::Kept(..)) => {
                    literal_divisor(value(a), value(d), folded, run).then_some(
                        "a literal divisor: a multiplication by its reciprocal at run time",
                    )
                }
                E::Bin("/", a, d)
                    if ARCH == Arch::X86_64
                        && value(d).is_nan()
                        && folded.to_bits() == 0
                        && same(run, numeric::div(value(a), value(d), PostOp::IDENTITY)) =>
                {
                    Some("x86-64: a NaN divisor folds to 0 and divides at run time")
                }
                E::Call("math.mod", args)
                    if value(&args[1]) == 0.0 && folded.is_nan() && run.to_bits() == 0 =>
                {
                    Some("math.mod: a zero divisor folds to NaN and runs to 0")
                }
                E::Call("math.mod", _)
                    if folded.to_bits() == (-0.0f32).to_bits() && run.to_bits() == 0 =>
                {
                    Some("math.mod: a −0 remainder folds to −0 and runs to +0")
                }
                _ => None,
            }
        }

        /// A difference that a negation or a kept operand of this node makes.
        fn post_op_difference(&self, folded: f32, run: f32) -> Option<&'static str> {
            let operands = match self {
                E::Neg(a) => vec![&**a],
                E::Bin("+" | "-" | "*", a, b) => vec![&**a, &**b],
                _ => return None,
            };
            let kept = operands.iter().any(|e| matches!(e, E::Kept(..)));
            // The operand a negation applies to.
            let negated = match self {
                E::Neg(a) | E::Bin("-", _, a) => Some(value(a)),
                _ => None,
            };
            let sign_bit_only = folded.to_bits() ^ run.to_bits() == 0x8000_0000;
            if sign_bit_only && folded == 0.0 && (negated.is_some() || kept) {
                Some("zeros of different sign: a negation or a kept constant")
            } else if negated.is_some_and(|n| n.is_nan() && same(folded, -n) && same(run, n)) {
                Some("NaNs of different sign: a negation")
            } else if self.left_term_nan_wins(folded, run) {
                Some("two NaNs: the fold takes the kept left term's, the run the other")
            } else {
                self.carrier_difference(folded, run)
                    .or_else(|| self.kept_difference(kept, folded, run))
            }
        }

        /// A kept left term and the other operand both NaN: the fold gives the term, the run the
        /// operand.
        fn left_term_nan_wins(&self, folded: f32, run: f32) -> bool {
            match self {
                E::Bin("+" | "-", c, x) if matches!(**c, E::Kept(..)) => {
                    let (c, x) = (value(c), value(x));
                    c.is_nan() && x.is_nan() && same(folded, c) && same(run, x)
                }
                _ => false,
            }
        }

        /// A difference that the node carrying this node's post-op makes, the fold being that of
        /// the same node with the carrier's folded value: `math.sign`'s rule or the guard's 0.
        fn carrier_difference(&self, folded: f32, run: f32) -> Option<&'static str> {
            let folds_as = |text: &str| {
                fold(&self.with_carrier_replaced(&lit(text)).source())
                    .is_some_and(|value| same(value, folded))
            };
            match self.carrier()? {
                E::Call("math.sign", _)
                    if folds_as("(-1)")
                        && fold(&self.with_carrier_replaced(&lit("1")).source())
                            .is_some_and(|one| same(run, -one)) =>
                {
                    Some("math.sign: the run gives −(S + O)")
                }
                E::Bin("/", _, d) if run.to_bits() == 0 && guarded(value(d)) && folds_as("0") => {
                    Some("a guarded division: 0 at run time without the post-op")
                }
                _ => None,
            }
        }

        /// A difference that a kept operand of this node makes: an infinite factor, or on `Arm64`
        /// a constant applied inside the instruction.
        fn kept_difference(&self, kept: bool, folded: f32, run: f32) -> Option<&'static str> {
            if self.is_kept_infinite_factor() && run.is_nan() && same(folded, self.product()) {
                Some("a kept infinite factor: the NaN offset at run time, the product folded")
            } else if ARCH == Arch::Arm64
                && kept
                && forms(&self.with_kept_operands_hidden()).is_ok_and(|(_, r)| same(r, folded))
            {
                Some("arm64: a kept constant applied inside its operand's instruction")
            } else {
                None
            }
        }

        /// The node whose post-op this node's negations and kept constants build at run time.
        fn carrier(&self) -> Option<&E> {
            match self {
                E::Neg(a) => a.carrier().or(Some(a)),
                E::Bin("+" | "-" | "*", a, b) => match (&**a, &**b) {
                    (E::Kept(..), other) | (other, E::Kept(..)) => other.carrier().or(Some(other)),
                    _ => None,
                },
                _ => None,
            }
        }

        /// The node with its [`carrier`](Self::carrier) replaced by `by`.
        fn with_carrier_replaced(&self, by: &E) -> E {
            let inner = |e: &E| {
                Box::new(match e.carrier() {
                    Some(_) => e.with_carrier_replaced(by),
                    None => by.clone(),
                })
            };
            match self {
                E::Neg(a) => E::Neg(inner(a)),
                E::Bin(op, a, b) if matches!(**a, E::Kept(..)) => E::Bin(op, a.clone(), inner(b)),
                E::Bin(op, a, b) => E::Bin(op, inner(a), b.clone()),
                other => other.clone(),
            }
        }

        /// The constant product of a `*` node's folded operands.
        fn product(&self) -> f32 {
            match self {
                E::Bin("*", a, b) => arith::mul(value(a), value(b)),
                _ => f32::NAN,
            }
        }

        fn is_kept_infinite_factor(&self) -> bool {
            match self {
                E::Bin("*", a, b) => [a, b]
                    .iter()
                    .any(|e| matches!(***e, E::Kept(value, _) if value.is_infinite())),
                _ => false,
            }
        }

        /// The node with its kept operands as literals, so that they are hidden too.
        fn with_kept_operands_hidden(&self) -> E {
            let hidden = |e: &E| match e {
                E::Kept(value, text) => E::Lit(*value, text.clone()),
                other => other.clone(),
            };
            match self {
                E::Bin(op, a, b) => E::Bin(op, Box::new(hidden(a)), Box::new(hidden(b))),
                other => other.clone(),
            }
        }
    }

    /// Finite literals; small integers, halves and signed zeros are where the operators' special
    /// cases are.
    fn literal() -> impl Strategy<Value = E> {
        let value = prop_oneof![
            4 => any::<f32>().prop_filter("finite", |x| x.is_finite()),
            2 => (-8i32..=8).prop_map(|i| i as f32 * 0.5),
            1 => prop::sample::select(vec![0.0f32, -0.0, 1.0, -1.0, 100.0, 1e-7, 1.0e30, 360.0, 180.0, -90.0]),
            // The extremes: near overflow (products reach infinity), denormals, and arguments that
            // make `acos` / `asin` / `sqrt` / `ln` / `pow` give a NaN of either sign.
            1 => prop::sample::select(vec![3.4e38f32, -3.4e38, 1e38, 1e-38, 1e-44, -1e-44, 1.5e27, -1.2313328e37, 16_777_216.0, 2.0, -2.0]),
        ];
        value.prop_filter_map("no literal form reads back as this value", |value| {
            literal_text(value).map(|text| E::Lit(value, text))
        })
    }

    /// The `math.*` functions that fold (all but the random ones), with their arities.
    fn functions() -> Vec<(&'static str, usize)> {
        MATH_META
            .iter()
            .filter(|meta| {
                !meta.token.contains("random")
                    && !meta.token.contains("die_roll")
                    && meta.min_args > 0
            })
            .map(|meta| (meta.token, usize::from(meta.min_args)))
            .collect()
    }

    fn expression() -> impl Strategy<Value = E> {
        let leaf = prop_oneof![10 => literal(), 1 => Just(E::Pi)];
        leaf.prop_recursive(4, 40, 3, |inner| {
            let boxed = inner.clone().prop_map(Box::new);
            // Half of the calls come from the functions where NaN, infinity and signs matter most.
            let sensitive: Vec<_> = functions()
                .into_iter()
                .filter(|(f, _)| {
                    [
                        "math.copy_sign",
                        "math.min",
                        "math.max",
                        "math.mod",
                        "math.sqrt",
                        "math.acos",
                        "math.asin",
                        "math.ln",
                        "math.pow",
                        "math.atan2",
                        "math.clamp",
                    ]
                    .contains(f)
                })
                .collect();
            let call = prop_oneof![1 => prop::sample::select(functions()), 1 => prop::sample::select(sensitive)]
                .prop_flat_map(move |(f, arity)| prop::collection::vec(inner.clone(), arity).prop_map(move |args| E::Call(f, args)));
            prop_oneof![
                2 => (prop::sample::select(vec!["+", "-", "*", "/", "<", "<=", ">", ">=", "==", "!=", "&&", "||"]), boxed.clone(), boxed.clone()).prop_map(|(op, a, b)| E::Bin(op, a, b)),
                3 => call,
                1 => boxed.clone().prop_map(E::Neg),
                1 => boxed.clone().prop_map(E::Not),
                1 => (boxed.clone(), boxed.clone(), boxed).prop_map(|(c, a, b)| E::Cond(c, a, b)),
            ]
        })
        .prop_filter("a nested sum is two different trees", |e| !e.has_nested_sum())
    }

    /// A constant from one of `texts`, kept.
    fn kept_text(texts: &[&'static str]) -> impl Strategy<Value = E> {
        prop::sample::select(texts.to_vec())
            .prop_map(|text| E::Kept(fold(text).unwrap_or(f32::NAN), text.to_owned()))
    }

    /// An expression, or one under a kept factor and / or term: on a product, on `math.sign`, on
    /// any expression, and an infinite factor; a kept term on the left, NaN too; a kept divisor.
    fn shaped() -> impl Strategy<Value = E> {
        let kept = || {
            literal().prop_map(|e| match e {
                E::Lit(value, text) => E::Kept(value, text),
                other => other,
            })
        };
        let product = (expression(), expression()).prop_map(|(a, b)| bin("*", a, b));
        let sign = expression().prop_map(|a| E::Call("math.sign", vec![a]));
        let factor = prop_oneof![3 => kept(), 1 => kept_text(&NON_FINITE[4..])];
        let left = prop_oneof![1 => kept(), 1 => kept_text(&NON_FINITE[..4])];
        let term = prop::sample::select(vec!["+", "-"]);
        prop_oneof![
            4 => expression(),
            1 => (product, term.clone(), kept()).prop_map(|(p, op, c)| bin(op, p, c)),
            1 => (sign, factor.clone(), term.clone(), kept())
                .prop_map(|(s, m, op, c)| bin(op, bin("*", s, m), c)),
            1 => (expression(), factor.clone(), term.clone(), kept())
                .prop_map(|(x, m, op, c)| bin(op, bin("*", x, m), c)),
            1 => (expression(), term.clone(), kept()).prop_map(|(x, op, c)| bin(op, x, c)),
            1 => (left, term, expression()).prop_map(|(c, op, x)| bin(op, c, x)),
            1 => (expression(), kept()).prop_map(|(x, d)| bin("/", x, d)),
            1 => (expression(), factor).prop_map(|(x, m)| bin("*", x, m)),
        ]
        .prop_filter("a nested sum is two different trees", |e| {
            !e.has_nested_sum()
        })
    }

    fn bin(op: &'static str, a: E, b: E) -> E {
        E::Bin(op, Box::new(a), Box::new(b))
    }

    fn same(a: f32, b: f32) -> bool {
        a.to_bits() == b.to_bits()
    }

    /// The constant `e` folds to.
    fn value(e: &E) -> f32 {
        fold(&e.source()).unwrap_or(f32::NAN)
    }

    /// Texts of the non-finite constants.
    const NON_FINITE: [&str; 6] = [
        "math.sqrt(-1)",
        "(-math.sqrt(-1))",
        "math.ln(-1)",
        "(-math.ln(-1))",
        "math.exp(1000)",
        "(-math.exp(1000))",
    ];

    /// A text that folds to exactly `value`.
    fn constant_text(value: f32) -> Option<String> {
        if value.is_finite() {
            return literal_text(value).or_else(|| scaled_text(value));
        }
        NON_FINITE
            .into_iter()
            .find(|text| fold(text).is_some_and(|read| same(read, value)))
            .map(str::to_owned)
    }

    /// A finite `value` as an integer times a power of two, both exact.
    fn scaled_text(value: f32) -> Option<String> {
        let bits = value.abs().to_bits();
        let (exponent, fraction) = (bits >> 23, bits & 0x7f_ffff);
        let (integer, power) = match exponent {
            0 => (fraction, -149),
            _ => (fraction | 0x80_0000, exponent as i32 - 150),
        };
        let sign = if value.is_sign_negative() { "-" } else { "" };
        let text = format!("({sign}{integer} * math.pow(2, {power}))");
        fold(&text)
            .is_some_and(|read| same(read, value))
            .then_some(text)
    }

    /// `x / d` with a literal `d`: the fold divides and the run multiplies by `1/d`, or the fold
    /// gives 0 and the run multiplies by 0 when `|d| < ε`.
    fn literal_divisor(x: f32, d: f32, folded: f32, run: f32) -> bool {
        let divides = d.abs() >= f32::EPSILON;
        let quotient = if divides { x / d } else { 0.0 };
        let reciprocal = if divides { 1.0 / d } else { 0.0 };
        same(folded, quotient) && same(run, arith::mul(x, reciprocal))
    }

    /// Whether the division guard stops a division by `divisor`.
    fn guarded(divisor: f32) -> bool {
        divisor.abs() < f32::EPSILON || (ARCH == Arch::Arm64 && divisor.is_nan())
    }

    /// The folded constant and the run-time result, which must not fold.
    fn outcome(e: &E) -> Result<(f32, f32), TestCaseError> {
        let hidden_source = e.render(true, &mut 0, &mut Vec::new());
        let hidden = compile(&hidden_source, &options());
        prop_assert!(
            hidden
                .expr()
                .is_none_or(|expr| expr.as_constant().is_none()),
            "{hidden_source} folded"
        );
        forms(e)
    }

    /// The folded constant and the run-time result.
    fn forms(e: &E) -> Result<(f32, f32), TestCaseError> {
        let options = options();
        let folded_source = e.source();
        let mut hidden_values = Vec::new();
        let hidden_source = e.render(true, &mut 0, &mut hidden_values);

        let folded = compile(&folded_source, &options);
        let folded_expr = folded.expr().cloned().ok_or_else(|| {
            TestCaseError::fail(format!("{folded_source}: {:?}", folded.diagnostics()))
        })?;
        let constant = folded_expr
            .as_constant()
            .ok_or_else(|| TestCaseError::fail(format!("{folded_source} did not fold")))?;

        let hidden = compile(&hidden_source, &options);
        let hidden_expr = hidden.expr().cloned().ok_or_else(|| {
            TestCaseError::fail(format!("{hidden_source}: {:?}", hidden.diagnostics()))
        })?;
        let mut env = NoHostEnv::new();
        for (i, value) in hidden_values.iter().enumerate() {
            env.variables
                .set(VariableName::new(&format!("k{i}")), Value::Float(*value));
        }
        let run = hidden_expr.eval_f32(&mut env.cx());
        prop_assert!(
            env.sink.is_empty(),
            "{hidden_source}: {:?}",
            env.sink.messages()
        );
        Ok((constant, run))
    }

    fn check(e: &E) -> Result<(), TestCaseError> {
        let (constant, run) = outcome(e)?;
        if same(constant, run) {
            return Ok(());
        }
        e.resolved().map(drop)
    }

    /// The seed is printed first so that a failed test's output carries it.
    fn config() -> ProptestConfig {
        let config = super::config(3_000);
        let seed = std::env::var("PROPTEST_RNG_SEED")
            .ok()
            .and_then(|s| s.parse::<u64>().ok())
            .unwrap_or_else(|| {
                let nanos = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map_or(0, |d| d.as_nanos());
                (nanos as u64) ^ (u64::from(std::process::id()) << 32)
            });
        println!(
            "folding property: PROPTEST_RNG_SEED={seed} PROPTEST_CASES={}",
            config.cases
        );
        ProptestConfig {
            max_global_rejects: 100_000_000,
            rng_seed: proptest::test_runner::RngSeed::Fixed(seed),
            ..config
        }
    }

    proptest! {
        #![proptest_config(config())]

        #[test]
        fn folding_matches_run_time(e in shaped()) {
            check(&e)?;
        }
    }

    fn lit(text: &str) -> E {
        E::Lit(fold(text).expect("a literal folds"), text.to_owned())
    }

    /// The fold negates a constant by flipping its sign bit; the run-time post-op `0 − x·1` leaves
    /// a NaN's sign bit alone. Only `math.copy_sign` shows the difference:
    ///
    /// - `math.acos(1.563555e27)` is a positive NaN: folded, `copy_sign(0, …)` gives −0, at run
    ///   time +0;
    /// - `math.sqrt(-1)` is the invalid-operation NaN, negative on `X86_64`: folded, `copy_sign(1,
    ///   …)` gives 1, at run time −1; positive on `Arm64`: −1 and 1;
    /// - `0e0 - math.sqrt(-1.2313328e37)` folds to a NaN of the other sign than the run.
    #[test]
    fn a_negated_nan_keeps_its_sign_only_when_folded() {
        let negated_acos = E::Neg(Box::new(E::Call("math.acos", vec![lit("1.563555e27")])));
        let acos_case = E::Call("math.copy_sign", vec![lit("0e0"), negated_acos]);
        let negated_sqrt = E::Neg(Box::new(E::Call("math.sqrt", vec![lit("(-1)")])));
        let sqrt_case = E::Call("math.copy_sign", vec![lit("1"), negated_sqrt]);
        let subtracted = E::Bin(
            "-",
            Box::new(lit("0e0")),
            Box::new(E::Call("math.sqrt", vec![lit("(-1.2313328e37)")])),
        );
        let subtracted_case = E::Call("math.copy_sign", vec![lit("0e0"), subtracted]);
        // The NaN reaches the negation through another call: `atan2` of a NaN is that NaN, sign
        // kept.
        let through_a_call = E::Neg(Box::new(E::Call(
            "math.atan2",
            vec![
                lit("0e0"),
                E::Call("math.sqrt", vec![lit("(-2.8403985e34)")]),
            ],
        )));
        let through_a_call_case = E::Call("math.copy_sign", vec![lit("0e0"), through_a_call]);
        let (sqrt_signs, zero_signs) = per_arch(
            ((1.0f32, -1.0f32), (0x0000_0000, 0x8000_0000)),
            ((-1.0, 1.0), (0x8000_0000, 0x0000_0000)),
        );
        let (folded, run) = outcome(&acos_case).expect("compiles and runs");
        assert_eq!(
            (folded.to_bits(), run.to_bits()),
            (0x8000_0000, 0x0000_0000),
            "{}",
            acos_case.source()
        );
        assert!(acos_case.resolved().is_ok(), "the property excuses it");

        let (folded, run) = outcome(&sqrt_case).expect("compiles and runs");
        assert_eq!(
            (folded.to_bits(), run.to_bits()),
            (sqrt_signs.0.to_bits(), sqrt_signs.1.to_bits()),
            "{}",
            sqrt_case.source()
        );
        assert!(sqrt_case.resolved().is_ok(), "the property excuses it");

        for case in [&subtracted_case, &through_a_call_case] {
            let (folded, run) = outcome(case).expect("compiles and runs");
            assert_eq!(
                (folded.to_bits(), run.to_bits()),
                zero_signs,
                "{}",
                case.source()
            );
            assert!(case.resolved().is_ok(), "the property excuses it");
        }
    }

    /// An excused node is replaced by its run-time value and the rest of the tree is still
    /// checked: a literal that reads back as another value stands for a wrong fold beside it.
    #[test]
    fn a_wrong_fold_beside_an_excused_node_is_reported() {
        let excused = E::Call("math.mod", vec![lit("3"), lit("0")]);
        let right = bin("+", excused.clone(), lit("2"));
        let wrong = bin("+", excused, E::Lit(1.0, "2".to_owned()));
        assert!(right.resolved().is_ok(), "{}", right.source());
        let error = wrong.resolved().expect_err("the sibling's wrong fold");
        assert!(
            error
                .to_string()
                .contains("folds to 2 (0x40000000), runs to 1"),
            "{error}"
        );
    }

    #[test]
    fn literal_forms_round_trip() {
        for value in [
            0.0f32,
            -0.0,
            1.0,
            0.1,
            1e-7,
            3.402_823_5e38,
            1.0e-38,
            -2.5,
            16_777_217.0,
            0.333_333_34,
        ] {
            if let Some(text) = literal_text(value) {
                let read = fold(&text).expect("folds");
                assert_eq!(read.to_bits(), value.to_bits(), "{text}");
            }
        }
        assert!(
            literal_text(0.1).is_some()
                && literal_text(1e-7).is_some()
                && literal_text(-0.0).is_some()
        );
    }
}

#[cfg(feature = "compiler")]
mod query_ranges {
    //! The compiler resolves `query.<name>` exactly when the declaration's `resolve` does.

    use super::{config, query, raw_version, sets};
    use molangx::catalog::{QueryAdmission, Side};
    use molangx::compile::{CompileOptions, compile};
    use molangx::version::{ExperimentMask, RawVersion};
    use proptest::prelude::*;

    proptest! {
        #![proptest_config(config(512))]

        /// Client side, so that no name is withheld for the server.
        #[test]
        fn the_compiler_resolves_exactly_when_resolve_does(index in query(), raw in raw_version(), sets in sets()) {
            let catalog = molangx::stdlib::queries(Side::Client);
            let decl = catalog.iter().nth(index).expect("a declaration");
            let options = CompileOptions { admission: QueryAdmission::Sets(sets), ..CompileOptions::from_raw_version(catalog.clone(), RawVersion(raw)) };
            let resolves = decl.resolve(RawVersion(raw), &QueryAdmission::Sets(sets), ExperimentMask::empty()).is_some();
            let compiled = compile(decl.name(), &options);
            prop_assert_eq!(compiled.is_success(), resolves, "{} at {}: {:?}", decl.name(), raw, compiled.diagnostics());
        }
    }
}

mod resolution_ranges {
    //! The catalogue's version ranges: each implementation serves one contiguous range, and the raw
    //! and `MolangVersion` paths agree.

    use molangx::catalog::{QueryAdmission, QueryAllowList, QueryCatalog, QuerySetMask, Side};
    use molangx::version::{Experiment, ExperimentMask, RawVersion};

    use molangx::version::MolangVersion;

    use super::{config, query, raw_version, sets};
    use proptest::prelude::*;

    fn catalog() -> &'static QueryCatalog {
        molangx::stdlib::queries(Side::Client)
    }

    /// Exhaustive over `i16`.
    #[test]
    fn each_implementation_is_one_contiguous_range() {
        for decl in catalog() {
            let mut runs: Vec<(u8, i16, i16)> = Vec::new();
            let mut current: Option<(u8, i16, i16)> = None;
            for raw in i16::MIN..=i16::MAX {
                let serving: Vec<u8> = (0..)
                    .zip(decl.shape().ranges.as_slice())
                    .filter(|(_, r)| r.contains_raw(RawVersion(raw)))
                    .map(|(k, _)| k)
                    .collect();
                assert!(
                    serving.len() <= 1,
                    "{}: ranges overlap at {raw}: {serving:?}",
                    decl.name()
                );
                match (current, serving.first()) {
                    (Some((k, first, _)), Some(&now)) if k == now => {
                        current = Some((k, first, raw))
                    }
                    (previous, now) => {
                        runs.extend(previous);
                        current = now.map(|&k| (k, raw, raw));
                    }
                }
            }
            runs.extend(current);
            let mut seen = std::collections::HashSet::new();
            for (k, first, last) in &runs {
                assert!(
                    seen.insert(*k),
                    "{}: implementation {k} serves more than one range: {runs:?}",
                    decl.name()
                );
                assert_eq!(decl.implementation_at_raw(RawVersion(*first)), Some(*k));
                assert_eq!(decl.implementation_at_raw(RawVersion(*last)), Some(*k));
            }
            assert_eq!(
                runs.len(),
                decl.shape().ranges.as_slice().len(),
                "{}: {runs:?} vs {:?}",
                decl.name(),
                decl.shape().ranges.as_slice()
            );
        }
    }

    fn experiments() -> impl Strategy<Value = ExperimentMask> {
        prop::sample::select(vec![
            ExperimentMask::empty(),
            ExperimentMask::empty().with(Experiment::new(63).expect("experiment 63")),
        ])
    }

    proptest! {
        #![proptest_config(config(512))]

        /// With sets or with a list; a resolved window holds the raw version.
        #[test]
        fn raw_and_enum_resolution_agree(
            index in query(),
            raw in raw_version(),
            sets in sets(),
            experiments in experiments(),
            listed in prop::collection::vec(query(), 1..3),
            include_self in any::<bool>(),
        ) {
            let decl = catalog().iter().nth(index).expect("a declaration");
            let mut names: Vec<&str> = listed.iter().map(|&i| catalog().iter().nth(i).expect("a declaration").name()).collect();
            if include_self {
                names.push(decl.name());
            }
            let list = QueryAllowList::new(catalog(), &names).expect("declared names");
            for admission in [QueryAdmission::Sets(sets), QueryAdmission::Only(list)] {
                let by_raw = decl.resolve(RawVersion(raw), &admission, experiments);
                prop_assert_eq!(by_raw, decl.implementation_at_raw(RawVersion(raw)).filter(|_| admission.admits(decl)));
                if let Some(version) = MolangVersion::from_i16(raw) {
                    prop_assert_eq!(decl.implementation_at(version), decl.implementation_at_raw(RawVersion(raw)));
                }
                if let Some(k) = by_raw {
                    let range = decl.shape().ranges.as_slice()[usize::from(k)];
                    prop_assert!(range.first().as_i16() <= raw && raw <= range.last().as_i16());
                }
            }
        }

        /// Although a raw version above 13 gates like 13; inside −1…13 the effective version
        /// resolves like the raw one.
        #[test]
        fn raw_versions_outside_the_enum_resolve_nothing(index in query(), raw in any::<i16>()) {
            let decl = catalog().iter().nth(index).expect("a declaration");
            let by_raw = decl.resolve(RawVersion(raw), &QueryAdmission::Sets(QuerySetMask::BUILTIN), ExperimentMask::empty());
            if (-1..=13).contains(&raw) {
                prop_assert_eq!(by_raw, decl.resolve(RawVersion(RawVersion(raw).effective().as_i16()), &QueryAdmission::Sets(QuerySetMask::BUILTIN), ExperimentMask::empty()));
            } else {
                prop_assert!(by_raw.is_none(), "{} resolves at raw version {raw}", decl.name());
            }
        }
    }
}
