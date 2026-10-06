//! The exact comparison of two values, two variable maps and two fuzz environments, NaN sign and
//! payload included.

use super::{FuzzEnv, FuzzHost};
use molangx::vm::namespace::Namespace;
use molangx::vm::{Host, TempMap, Value, VariableMap};
use std::collections::HashSet;
use std::iter;
use std::sync::Arc;

/// Whether two values are the same, floats compared by bits and structs member by member.
pub fn same_value<H: Host>(a: &Value<H>, b: &Value<H>) -> bool {
    // Iterative: a value can nest far deeper than the tree that built it. Each pair of structs is
    // compared once: structs share members, so the paths through a value can be exponential in
    // its depth.
    let mut pending = vec![(a, b)];
    let mut compared = HashSet::new();
    while let Some((a, b)) = pending.pop() {
        let same = match (a, b) {
            (Value::Float(x), Value::Float(y)) => x.to_bits() == y.to_bits(),
            (Value::Struct(x), Value::Struct(y)) => {
                if Arc::ptr_eq(x, y)
                    || !compared.insert((Arc::as_ptr(x).addr(), Arc::as_ptr(y).addr()))
                {
                    continue;
                }
                if x.len() != y.len() {
                    return false;
                }
                for (name, value) in x.iter() {
                    let Some(other) = y.get(name) else {
                        return false;
                    };
                    pending.push((value, other));
                }
                true
            }
            _ => a == b,
        };
        if !same {
            return false;
        }
    }
    true
}

/// Whether two maps hold the same names with the same values (see [`same_value`]).
pub fn same_entries<H: Host, N: Namespace>(a: &VariableMap<H, N>, b: &VariableMap<H, N>) -> bool {
    a.len() == b.len()
        && a.iter()
            .all(|(name, value)| b.get(name).is_some_and(|other| same_value(value, other)))
}

/// Whether two optional temp maps are both absent or hold the same entries.
pub fn same_temps<H: Host>(a: Option<&TempMap<H>>, b: Option<&TempMap<H>>) -> bool {
    match (a, b) {
        (Some(x), Some(y)) => same_entries(x, y),
        (x, y) => x.is_none() && y.is_none(),
    }
}

/// How two variable maps differ, or `None`: each variable's latest value (see [`same_value`]), its
/// access (evaluation sets it only for a new slot), and its public snapshot (`None` for a private
/// slot).
///
/// A slot declared without a value is not listed by [`VariableMap::iter`]; it is found through the
/// access of the listed names and `any_public`.
pub(super) fn map_difference(
    a: &VariableMap<FuzzHost>,
    b: &VariableMap<FuzzHost>,
) -> Option<String> {
    if !same_entries(a, b) {
        return Some(format!("{a:?} vs {b:?}"));
    }
    // The same names now hold the same values; their access and snapshots, in the order of the
    // names.
    let mut names: Vec<_> = a.iter().map(|(name, _)| name).collect();
    names.sort_by_cached_key(|name| format!("{name:?}"));
    for name in names {
        if a.access(name) != b.access(name) {
            return Some(format!(
                "access of {name:?}: {:?} vs {:?}",
                a.access(name),
                b.access(name)
            ));
        }
        let same_snapshot = match (a.get_public(name), b.get_public(name)) {
            (Some(x), Some(y)) => same_value(x, y),
            (x, y) => x.is_none() && y.is_none(),
        };
        if !same_snapshot {
            return Some(format!(
                "public snapshot of {name:?}: {:?} vs {:?}",
                a.get_public(name),
                b.get_public(name)
            ));
        }
    }
    (a.any_public() != b.any_public()).then(|| {
        format!(
            "has public variables: {} vs {}",
            a.any_public(),
            b.any_public()
        )
    })
}

/// Why the state two runs left behind differs (variables with their access and public snapshots,
/// temps, messages, random source), or `None`.
///
/// Not part of the state, since an evaluation only reads it: the context, the world, the subject,
/// `this`, the queries and the limits.
pub fn state_difference(a: &FuzzEnv, b: &FuzzEnv) -> Option<String> {
    let mut why = Vec::new();
    let maps =
        iter::once((&a.vars.local, &b.vars.local)).chain(a.vars.actors.iter().zip(&b.vars.actors));
    for (i, (x, y)) in maps.enumerate() {
        if let Some(detail) = map_difference(x, y) {
            why.push(format!("variable map {i}: {detail}"));
        }
    }
    if !same_temps(a.temps.as_ref(), b.temps.as_ref()) {
        why.push(format!("temps: {:?} vs {:?}", a.temps, b.temps));
    }
    if a.sink.messages != b.sink.messages {
        why.push(format!(
            "messages: {:?} vs {:?}",
            a.sink.messages, b.sink.messages
        ));
    }
    if a.rng != b.rng {
        why.push(format!("random source: {:?} vs {:?}", a.rng, b.rng));
    }
    (!why.is_empty()).then(|| why.join("; "))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::generator::env::{
        ACTORS, FuzzActor, FuzzRng, Subject, TempLifetime, test_support::*,
    };
    use molangx::hash::HashedStr;
    use molangx::rng::{FixedRng, Xorshift128, sample};
    use molangx::vm::{Access, EvalLimits, StructValue, TempName};
    use std::thread;

    fn chain(depth: usize, leaf: f32) -> V {
        let mut value = Value::Float(leaf);
        for _ in 0..depth {
            value = Value::structure(StructValue::from([("n", value)]));
        }
        value
    }

    fn shared_levels(levels: usize, leaf: f32) -> V {
        let mut value = Value::Float(leaf);
        for _ in 0..levels {
            value = Value::structure(StructValue::from([("a", value.clone()), ("b", value)]));
        }
        value
    }

    fn same_map(a: &VariableMap<FuzzHost>, b: &VariableMap<FuzzHost>) -> bool {
        map_difference(a, b).is_none()
    }

    fn map_of(entries: &[(&str, V)]) -> VariableMap<FuzzHost> {
        let mut map = VariableMap::new();
        for (name, value) in entries {
            map.set(var(name), value.clone());
        }
        map
    }

    #[test]
    fn floats_are_the_same_when_their_bits_are() {
        assert!(same_value(&float(1.5), &float(1.5)));
        assert!(!same_value(&float(1.5), &float(1.25)));
        assert!(
            !same_value(&float(0.0), &float(-0.0)),
            "the sign of zero is a difference"
        );
        assert!(same_value(&float(-0.0), &float(-0.0)));
        assert!(same_value(&float(f32::INFINITY), &float(f32::INFINITY)));
        assert!(!same_value(
            &float(f32::INFINITY),
            &float(f32::NEG_INFINITY)
        ));
        assert!(!same_value(&float(f32::MAX), &float(f32::INFINITY)));
        // One ulp apart is a difference.
        assert!(!same_value(
            &float(1.0),
            &float(f32::from_bits(1.0f32.to_bits() + 1))
        ));
    }

    #[test]
    fn a_nan_is_the_same_only_as_a_nan_of_the_same_sign_and_payload() {
        let payloads = [
            f32::NAN,
            f32::from_bits(0x7fc0_0001),
            f32::from_bits(0xffc0_0000),
            f32::from_bits(0x7f80_0001),
        ];
        for (i, a) in payloads.into_iter().enumerate() {
            for (j, b) in payloads.into_iter().enumerate() {
                assert_eq!(
                    same_value(&float(a), &float(b)),
                    i == j,
                    "{:#x} / {:#x}",
                    a.to_bits(),
                    b.to_bits()
                );
            }
            for number in [0.0, -0.0, 1.0, f32::INFINITY, f32::NEG_INFINITY] {
                assert!(!same_value(&float(a), &float(number)));
                assert!(!same_value(&float(number), &float(a)));
            }
        }
    }

    #[test]
    fn values_of_different_kinds_are_never_the_same() {
        let hash = Value::<FuzzHost>::string("moo");
        let values = [
            float(0.0),
            float(f32::from_bits(HashedStr::new("moo").as_u64() as u32)),
            hash.clone(),
            Value::Actor(FuzzActor::Handle(1)),
            Value::Actor(FuzzActor::Id(1)),
            Value::actor_array([FuzzActor::Handle(1)]),
            Value::structure(StructValue::new()),
            Value::structure(StructValue::from([("x", 1.0)])),
        ];
        for (i, a) in values.iter().enumerate() {
            for (j, b) in values.iter().enumerate() {
                assert_eq!(same_value(a, b), i == j, "{a:?} / {b:?}");
            }
        }
    }

    #[test]
    fn strings_actors_and_arrays_compare_exactly() {
        assert!(same_value(&V::string("moo"), &Value::string("moo")));
        assert!(!same_value(&V::string("moo"), &Value::string("Moo")));
        assert!(same_value(
            &V::Actor(FuzzActor::Handle(2)),
            &Value::Actor(FuzzActor::Handle(2))
        ));
        assert!(
            !same_value(
                &V::Actor(FuzzActor::Handle(2)),
                &Value::Actor(FuzzActor::Id(2))
            ),
            "a direct handle is not an id"
        );
        assert!(!same_value(
            &V::Actor(FuzzActor::Handle(2)),
            &Value::Actor(FuzzActor::Handle(3))
        ));
        let a = V::actor_array([FuzzActor::Handle(1), FuzzActor::Handle(2)]);
        assert!(same_value(
            &a,
            &Value::actor_array([FuzzActor::Handle(1), FuzzActor::Handle(2)])
        ));
        assert!(
            !same_value(
                &a,
                &Value::actor_array([FuzzActor::Handle(2), FuzzActor::Handle(1)])
            ),
            "an array is ordered"
        );
        assert!(!same_value(&a, &Value::actor_array([FuzzActor::Handle(1)])));
        assert!(!same_value(
            &V::actor_array([]),
            &Value::actor_array([FuzzActor::Handle(0)])
        ));
    }

    #[test]
    fn structs_compare_member_by_member_in_any_order() {
        let ab = V::structure(StructValue::from([("a", 1.0), ("b", 2.0)]));
        let ba = Value::structure(StructValue::from([("b", 2.0), ("a", 1.0)]));
        assert!(same_value(&ab, &ba));
        assert!(same_value(&ba, &ab));
        // A different value, a different name, a different count.
        assert!(!same_value(
            &ab,
            &Value::structure(StructValue::from([("a", 1.0), ("b", 3.0)]))
        ));
        assert!(!same_value(
            &ab,
            &Value::structure(StructValue::from([("a", 1.0), ("c", 2.0)]))
        ));
        assert!(!same_value(
            &Value::structure(StructValue::from([("a", 1.0), ("c", 2.0)])),
            &ab
        ));
        assert!(!same_value(
            &ab,
            &Value::structure(StructValue::from([("a", 1.0)]))
        ));
        assert!(!same_value(
            &Value::structure(StructValue::from([("a", 1.0)])),
            &ab
        ));
        assert!(!same_value(
            &ab,
            &Value::structure(StructValue::from([("a", 1.0), ("b", 2.0), ("c", 3.0)]))
        ));
        assert!(same_value(
            &V::structure(StructValue::new()),
            &Value::structure(StructValue::new())
        ));
    }

    #[test]
    fn structs_compare_through_nested_levels() {
        let make = |leaf: f32, other: f32| {
            V::structure(StructValue::from([
                (
                    "deep",
                    Value::structure(StructValue::from([
                        ("leaf", Value::Float(leaf)),
                        (
                            "tail",
                            Value::structure(StructValue::from([("end", other)])),
                        ),
                    ])),
                ),
                ("flat", Value::Float(1.0)),
            ]))
        };
        assert!(same_value(&make(1.0, 2.0), &make(1.0, 2.0)));
        assert!(same_value(&make(f32::NAN, 2.0), &make(f32::NAN, 2.0)));
        assert!(
            !same_value(
                &make(f32::NAN, 2.0),
                &make(f32::from_bits(0x7fc0_0001), 2.0)
            ),
            "a nested NaN payload"
        );
        assert!(!same_value(&make(1.0, 2.0), &make(1.5, 2.0)));
        assert!(
            !same_value(&make(1.0, 2.0), &make(1.0, 2.5)),
            "a difference three levels down"
        );
        assert!(
            !same_value(&make(0.0, 2.0), &make(-0.0, 2.0)),
            "a nested sign of zero"
        );
        // A float where a struct is expected, at depth.
        let float_at_depth = Value::structure(StructValue::from([("deep", 1.0), ("flat", 1.0)]));
        assert!(!same_value(&make(1.0, 2.0), &float_at_depth));
    }

    #[test]
    fn a_struct_is_the_same_as_its_own_copy_even_when_it_holds_a_nan() {
        let nan = V::structure(StructValue::from([("n", f32::NAN)]));
        assert!(
            same_value(&nan, &nan.clone()),
            "a shared struct is the same struct"
        );
        assert!(same_value(
            &nan,
            &Value::structure(StructValue::from([("n", f32::NAN)]))
        ));
        assert_ne!(
            nan,
            Value::structure(StructValue::from([("n", f32::NAN)])),
            "plain equality would say no"
        );
    }

    #[test]
    fn a_very_deep_chain_is_compared_without_recursion() {
        // On a thread too small to recurse that deep; the drop of the chains is iterative too.
        thread::Builder::new()
            .stack_size(256 * 1024)
            .spawn(|| {
                let (a, b, c) = (chain(50_000, 1.0), chain(50_000, 1.0), chain(50_000, 2.0));
                assert!(same_value(&a, &b));
                assert!(!same_value(&a, &c), "the difference is 50,000 levels down");
                assert!(!same_value(&a, &chain(49_999, 1.0)), "one level short");
                assert!(same_value(
                    &chain(50_000, f32::NAN),
                    &chain(50_000, f32::NAN)
                ));
                assert!(!same_value(
                    &chain(50_000, f32::NAN),
                    &chain(50_000, f32::from_bits(0x7fc0_0001))
                ));
            })
            .expect("a thread")
            .join()
            .expect("no overflow");
    }

    #[test]
    fn a_value_that_shares_its_members_is_compared_once_per_pair() {
        // Thirty-two levels, each holding the level below twice: 2^32 paths, 33 distinct pairs.
        let (a, b) = (shared_levels(32, 1.0), shared_levels(32, 1.0));
        assert!(same_value(&a, &b));
        assert!(!same_value(&a, &shared_levels(32, 2.0)));
        assert!(same_value(
            &shared_levels(32, f32::NAN),
            &shared_levels(32, f32::NAN)
        ));
        assert!(!same_value(
            &shared_levels(32, f32::NAN),
            &shared_levels(32, -f32::NAN)
        ));
    }

    /// A value to build in two ways: a tree of the kinds a variable holds.
    #[derive(Clone, Debug)]
    enum Tree {
        Float(u32),
        Hash(u8),
        Handle(u8),
        Id(u8),
        Array(Vec<u8>),
        Struct(Vec<(u8, Tree)>),
    }

    impl Tree {
        fn build(&self) -> V {
            match self {
                Self::Float(bits) => Value::Float(f32::from_bits(*bits)),
                Self::Hash(n) => Value::string(&format!("s{n}")),
                Self::Handle(n) => Value::Actor(FuzzActor::Handle(*n)),
                Self::Id(n) => Value::Actor(FuzzActor::Id(*n)),
                Self::Array(items) => {
                    Value::actor_array(items.iter().map(|n| FuzzActor::Handle(*n)))
                }
                Self::Struct(members) => {
                    let mut array = StructValue::new();
                    for (name, tree) in members {
                        array.set(HashedStr::new(&format!("m{name}")), tree.build());
                    }
                    Value::structure(array)
                }
            }
        }

        /// The same tree with the later of two members of one name replacing the earlier, as
        /// building does.
        fn deduplicated(&self) -> Self {
            match self {
                Self::Struct(members) => {
                    let mut kept: Vec<(u8, Tree)> = Vec::new();
                    for (name, tree) in members {
                        let tree = tree.deduplicated();
                        match kept.iter_mut().find(|(n, _)| n == name) {
                            Some(slot) => slot.1 = tree,
                            None => kept.push((*name, tree)),
                        }
                    }
                    Self::Struct(kept)
                }
                other => other.clone(),
            }
        }

        /// The same value with every list of members reversed.
        fn reversed(&self) -> Self {
            match self {
                Self::Struct(members) => Self::Struct(
                    members
                        .iter()
                        .rev()
                        .map(|(n, t)| (*n, t.reversed()))
                        .collect(),
                ),
                other => other.clone(),
            }
        }

        /// What `same_value` must say, written separately: structure by structure, floats by bits,
        /// a struct by its members once duplicate names are resolved as building does.
        fn same(&self, other: &Self) -> bool {
            match (self.deduplicated(), other.deduplicated()) {
                (Self::Float(a), Self::Float(b)) => a == b,
                (Self::Hash(a), Self::Hash(b))
                | (Self::Handle(a), Self::Handle(b))
                | (Self::Id(a), Self::Id(b)) => a == b,
                (Self::Array(a), Self::Array(b)) => a == b,
                (Self::Struct(a), Self::Struct(b)) => {
                    a.len() == b.len()
                        && a.iter().all(|(name, x)| {
                            b.iter()
                                .find(|(n, _)| n == name)
                                .is_some_and(|(_, y)| x.same(y))
                        })
                }
                _ => false,
            }
        }
    }

    fn tree() -> impl proptest::strategy::Strategy<Value = Tree> {
        use proptest::prelude::*;
        let leaf = prop_oneof![
            prop::sample::select(vec![
                0.0f32.to_bits(),
                (-0.0f32).to_bits(),
                1.0f32.to_bits(),
                f32::NAN.to_bits(),
                0x7fc0_0001,
                0xffc0_0000,
                f32::INFINITY.to_bits()
            ])
            .prop_map(Tree::Float),
            (0u8..3).prop_map(Tree::Hash),
            (0u8..3).prop_map(Tree::Handle),
            (0u8..3).prop_map(Tree::Id),
            prop::collection::vec(0u8..3, 0..3).prop_map(Tree::Array),
        ];
        leaf.prop_recursive(3, 12, 3, |inner| {
            prop::collection::vec((0u8..3, inner), 0..3).prop_map(Tree::Struct)
        })
    }

    proptest::proptest! {
        #![proptest_config(proptest::prelude::ProptestConfig::with_cases(256))]

        #[test]
        fn same_value_agrees_with_an_independent_comparison(a in tree(), b in tree()) {
            proptest::prop_assert_eq!(same_value(&a.build(), &b.build()), a.same(&b), "{:?} / {:?}", a, b);
        }

        #[test]
        fn same_value_is_reflexive_and_symmetric(a in tree(), b in tree()) {
            let (x, y) = (a.build(), b.build());
            proptest::prop_assert!(same_value(&x, &x));
            proptest::prop_assert!(same_value(&x, &a.build()));
            proptest::prop_assert_eq!(same_value(&x, &y), same_value(&y, &x));
        }

        #[test]
        fn a_struct_is_the_same_whatever_the_order_of_its_members(a in tree()) {
            let a = a.deduplicated();
            proptest::prop_assert!(same_value(&a.build(), &a.reversed().build()));
        }

        #[test]
        fn a_float_that_differs_in_one_bit_is_never_the_same(bits in proptest::prelude::any::<u32>(), bit in 0u32..32) {
            let (a, b) = (f32::from_bits(bits), f32::from_bits(bits ^ (1 << bit)));
            proptest::prop_assert!(!same_value(&float(a), &float(b)));
        }
    }

    #[test]
    fn maps_are_the_same_with_the_same_names_and_values() {
        let a = map_of(&[("x", float(1.0)), ("s", Value::string("moo"))]);
        let b = map_of(&[("s", Value::string("moo")), ("x", float(1.0))]);
        assert!(same_map(&a, &b), "the order of insertion does not matter");
        assert!(same_map(&VariableMap::new(), &VariableMap::new()));
    }

    #[test]
    fn maps_with_a_different_length_are_different_in_both_directions() {
        let small = map_of(&[("x", float(1.0))]);
        let big = map_of(&[("x", float(1.0)), ("y", float(2.0))]);
        assert!(!same_map(&small, &big));
        assert!(!same_map(&big, &small));
        assert!(!same_map(&small, &VariableMap::new()));
        assert!(!same_map(&VariableMap::new(), &small));
    }

    #[test]
    fn maps_with_a_different_name_or_value_are_different() {
        let a = map_of(&[("x", float(1.0)), ("y", float(2.0))]);
        assert!(
            !same_map(&a, &map_of(&[("x", float(1.0)), ("z", float(2.0))])),
            "same length, a different name"
        );
        assert!(!same_map(
            &a,
            &map_of(&[("x", float(1.0)), ("y", float(2.5))])
        ));
        assert!(
            !same_map(
                &a,
                &map_of(&[("x", float(1.0)), ("y", Value::string("moo"))])
            ),
            "a different kind"
        );
        assert!(!same_map(
            &map_of(&[("x", float(0.0))]),
            &map_of(&[("x", float(-0.0))])
        ));
    }

    #[test]
    fn maps_with_nan_values_compare_their_bits() {
        let a = map_of(&[("n", float(f32::NAN))]);
        assert!(same_map(&a, &map_of(&[("n", float(f32::NAN))])));
        assert!(!same_map(
            &a,
            &map_of(&[("n", float(f32::from_bits(0x7fc0_0001)))])
        ));
        assert!(!same_map(&a, &map_of(&[("n", float(-f32::NAN))])));
        assert!(!same_map(&a, &map_of(&[("n", float(0.0))])));
    }

    #[test]
    fn maps_compare_structs_by_member() {
        let a = map_of(&[(
            "st",
            Value::structure(StructValue::from([("a", 1.0), ("b", 2.0)])),
        )]);
        assert!(same_map(
            &a,
            &map_of(&[(
                "st",
                Value::structure(StructValue::from([("b", 2.0), ("a", 1.0)]))
            )])
        ));
        assert!(!same_map(
            &a,
            &map_of(&[(
                "st",
                Value::structure(StructValue::from([("a", 1.0), ("b", 3.0)]))
            )])
        ));
    }

    #[test]
    fn maps_with_a_different_access_are_different() {
        // Evaluation sets the access of a new slot and keeps that of an existing one, so a map that
        // differs only in it is a map one evaluator wrote differently.
        let a = map_of(&[("x", float(1.0)), ("y", float(2.0))]);
        let mut b = a.clone();
        assert_eq!(map_difference(&a, &b), None);
        b.set_access(var("y"), Access::Public);
        let expected = format!("access of {:?}: Some(Private) vs Some(Public)", var("y"));
        assert_eq!(map_difference(&a, &b), Some(expected));
        assert!(!same_map(&a, &b) && !same_map(&b, &a));
        // The value alone is the same, so plain equality of the values does not see it.
        assert!(same_value(
            a.get(var("y")).expect("y"),
            b.get(var("y")).expect("y")
        ));
    }

    #[test]
    fn maps_with_a_different_public_snapshot_are_different() {
        let mut a = map_of(&[("x", float(1.0))]);
        a.set_public(var("x"), float(1.0));
        let mut b = a.clone();
        assert!(same_map(&a, &b), "no snapshot on either side");
        a.refresh_snapshots();
        let why = map_difference(&a, &b).expect("a snapshot against none");
        assert!(
            why.starts_with("public snapshot of ") && why.ends_with(": Some(Float(1.0)) vs None"),
            "{why}"
        );
        b.refresh_snapshots();
        assert!(same_map(&a, &b));
        // Both snapshots are compared as values: a NaN of another payload or a zero of the other
        // sign is a difference.
        let snapshot = |value: f32, snapshot: f32| {
            let mut map = map_of(&[("x", float(value))]);
            map.set_public(var("x"), float(snapshot));
            map.refresh_snapshots();
            map.set_public(var("x"), float(value));
            map
        };
        assert!(same_map(
            &snapshot(f32::NAN, f32::NAN),
            &snapshot(f32::NAN, f32::NAN)
        ));
        assert!(!same_map(
            &snapshot(f32::NAN, f32::NAN),
            &snapshot(f32::NAN, f32::from_bits(0x7fc0_0001))
        ));
        assert!(
            !same_map(&snapshot(0.0, 0.0), &snapshot(0.0, -0.0)),
            "0.0 against -0.0 in the snapshot"
        );
        assert!(!same_map(&snapshot(0.0, 1.0), &snapshot(0.0, 2.0)));
    }

    #[test]
    fn a_stale_snapshot_is_the_snapshot_not_the_value() {
        // The latest values agree and so do the access; the snapshots hold what the values were at
        // the update.
        let mut a = map_of(&[]);
        let mut b = map_of(&[]);
        a.set_public(var("x"), float(1.0));
        b.set_public(var("x"), float(1.0));
        a.refresh_snapshots();
        a.set_public(var("x"), float(2.0));
        b.set_public(var("x"), float(2.0));
        b.refresh_snapshots();
        assert!(same_value(
            a.get(var("x")).expect("x"),
            b.get(var("x")).expect("x")
        ));
        assert!(map_difference(&a, &b).is_some_and(|why| why.starts_with("public snapshot of ")));
    }

    #[test]
    fn a_declared_public_slot_without_a_value_is_seen_through_the_public_count() {
        // `iter` and `len` do not list a slot that only has an access; `any_public` does count it.
        let a = map_of(&[("x", float(1.0))]);
        let mut b = a.clone();
        b.set_access(var("ghost"), Access::Public);
        assert_eq!(b.len(), 1, "the declared slot has no value");
        assert_eq!(
            map_difference(&a, &b).as_deref(),
            Some("has public variables: false vs true")
        );
        // A declared private one cannot be told apart: it holds nothing and evaluation never
        // declares one.
        let mut c = a.clone();
        c.set_access(var("ghost"), Access::Private);
        assert!(same_map(&a, &c));
    }

    #[test]
    fn the_attributes_of_a_struct_member_are_its_name_and_value_alone() {
        // A member holds no flag of its own: two structs that were built in another order, or whose
        // cached depth was read on one side only, are the same.
        let one = Value::structure(StructValue::from([
            ("a", Value::Float(1.0)),
            ("b", Value::structure(StructValue::from([("c", 2.0)]))),
        ]));
        let other = Value::structure(StructValue::from([
            ("b", Value::structure(StructValue::from([("c", 2.0)]))),
            ("a", Value::Float(1.0)),
        ]));
        if let Value::Struct(members) = &one {
            assert_eq!(
                members.depth(),
                2,
                "the depth cache is filled on this side only"
            );
        }
        assert!(same_value(&one, &other));
        assert!(same_map(&map_of(&[("s", one)]), &map_of(&[("s", other)])));
    }

    #[test]
    fn equal_states_have_no_difference() {
        let a = env();
        let mut b = a.clone();
        assert_eq!(state_difference(&a, &b), None);
        // Things evaluation only reads are not part of the state.
        b.this = 99.0;
        b.world.alive = 0;
        b.subject = None;
        b.limits.total_steps = Some(1);
        assert_eq!(state_difference(&a, &b), None);
    }

    #[test]
    fn a_changed_message_is_reported_with_both_lists() {
        let a = env();
        let mut b = a.clone();
        b.sink.messages.push("oops".to_owned());
        let why = state_difference(&a, &b).expect("a difference");
        assert_eq!(why, r#"messages: [] vs ["oops"]"#);
        assert_eq!(
            state_difference(&b, &a).as_deref(),
            Some(r#"messages: ["oops"] vs []"#)
        );
        // Same count, different text; and the same text in another order.
        let (mut c, mut d) = (a.clone(), a.clone());
        c.sink.messages = vec!["one".to_owned(), "two".to_owned()];
        d.sink.messages = vec!["one".to_owned(), "tow".to_owned()];
        assert!(state_difference(&c, &d).is_some_and(|why| why.starts_with("messages:")));
        d.sink.messages = vec!["two".to_owned(), "one".to_owned()];
        assert!(state_difference(&c, &d).is_some());
    }

    #[test]
    fn a_changed_random_source_is_reported() {
        let a = xorshift_env();
        let mut b = a.clone();
        sample(&mut b.rng);
        let why = state_difference(&a, &b).expect("a difference");
        assert!(why.starts_with("random source: Xorshift("), "{why}");
        // A forced sample is compared by bits, and a kind change is a difference.
        let fixed = env();
        let mut other = fixed.clone();
        other.rng = FuzzRng::Fixed(QUARTER);
        assert_eq!(
            state_difference(&fixed, &other).as_deref(),
            Some("random source: Fixed(FixedRng(1073741824)) vs Fixed(FixedRng(536870912))")
        );
        other.rng = FuzzRng::Fixed(FixedRng::HALF);
        assert_eq!(state_difference(&fixed, &other), None);
        assert!(
            state_difference(&fixed, &xorshift_env())
                .is_some_and(|why| why.starts_with("random source:"))
        );
    }

    #[test]
    fn a_draw_count_difference_is_a_difference_of_the_random_state() {
        let a = xorshift_env();
        let (mut one, mut two) = (a.clone(), a.clone());
        sample(&mut one.rng);
        sample(&mut two.rng);
        sample(&mut two.rng);
        assert_eq!(state_difference(&one, &one.clone()), None);
        assert!(state_difference(&one, &two).is_some());
        assert!(state_difference(&a, &one).is_some());
    }

    #[test]
    fn temps_of_a_different_kind_or_value_are_reported() {
        let ours = env();
        let mut persistent = FuzzEnv::new(
            Subject::Actor,
            TempLifetime::Persistent,
            EvalLimits::DEFAULT,
            FuzzRng::Fixed(FixedRng::HALF),
        );
        let why = state_difference(&ours, &persistent).expect("None against Some");
        assert!(why.starts_with("temps: None vs Some("), "{why}");
        assert!(
            state_difference(&persistent, &ours).is_some_and(|why| why.starts_with("temps: Some("))
        );
        let mut other = persistent.clone();
        assert_eq!(
            state_difference(&persistent, &other),
            None,
            "two empty temp maps"
        );
        persistent
            .temps
            .as_mut()
            .expect("temps")
            .set(TempName::new("t"), float(1.0));
        assert!(state_difference(&persistent, &other).is_some_and(|why| why.starts_with("temps:")));
        other
            .temps
            .as_mut()
            .expect("temps")
            .set(TempName::new("t"), float(1.0));
        assert_eq!(state_difference(&persistent, &other), None);
        other
            .temps
            .as_mut()
            .expect("temps")
            .set(TempName::new("t"), float(2.0));
        assert!(state_difference(&persistent, &other).is_some());
        // Temps are compared as maps: the bits of a NaN count.
        persistent
            .temps
            .as_mut()
            .expect("temps")
            .set(TempName::new("t"), float(f32::NAN));
        other
            .temps
            .as_mut()
            .expect("temps")
            .set(TempName::new("t"), float(f32::NAN));
        assert_eq!(state_difference(&persistent, &other), None);
        other
            .temps
            .as_mut()
            .expect("temps")
            .set(TempName::new("t"), float(f32::from_bits(0x7fc0_0001)));
        assert!(state_difference(&persistent, &other).is_some());
    }

    #[test]
    fn a_changed_variable_names_the_map_it_is_in() {
        let a = env();
        // The detached map is number 0 and actor n is number n + 1.
        let mut local = a.clone();
        local.vars.local.set(var("x"), float(99.0));
        let why = state_difference(&a, &local).expect("a difference");
        assert!(why.starts_with("variable map 0: "), "{why}");
        assert!(!why.contains(';'), "only one difference: {why}");
        for slot in 0..ACTORS {
            let mut changed = a.clone();
            changed.vars.actors[slot].set(var("zzz"), float(1.0));
            let why = state_difference(&a, &changed).unwrap_or_else(|| panic!("actor {slot}"));
            assert!(
                why.starts_with(&format!("variable map {}: ", slot + 1)),
                "{why}"
            );
        }
    }

    #[test]
    fn a_removed_variable_an_added_one_and_a_changed_kind_are_all_reported() {
        let a = env();
        let mut removed = a.clone();
        removed.vars.actors[1].remove(var("x"));
        assert!(
            state_difference(&a, &removed).is_some_and(|why| why.starts_with("variable map 2: "))
        );
        assert!(
            state_difference(&removed, &a).is_some_and(|why| why.starts_with("variable map 2: "))
        );
        let mut kind = a.clone();
        kind.vars.actors[1].set(var("x"), Value::string("1.5"));
        assert!(state_difference(&a, &kind).is_some());
        let mut structure = a.clone();
        structure
            .vars
            .local
            .set(var("st"), Value::structure(StructValue::from([("x", 1.0)])));
        assert!(
            state_difference(&a, &structure).is_some(),
            "a struct with a member fewer"
        );
        let mut sign = a.clone();
        sign.vars.local.set(var("a"), float(-0.0));
        assert!(state_difference(&a, &sign).is_some(), "0.0 against -0.0");
    }

    #[test]
    fn a_nan_that_changes_payload_or_sign_or_becomes_a_number_is_a_difference() {
        let a = env();
        let mut payload = a.clone();
        payload
            .vars
            .local
            .set(var("n"), float(f32::from_bits(0x7fc0_1234)));
        assert!(state_difference(&a, &payload).is_some());
        let mut sign = a.clone();
        let n = a.vars.local.get(var("n")).expect("n").as_f32();
        sign.vars.local.set(var("n"), float(-n));
        assert!(state_difference(&a, &sign).is_some());
        let mut number = a.clone();
        number.vars.local.set(var("n"), float(0.0));
        assert!(state_difference(&a, &number).is_some());
    }

    #[test]
    fn several_differences_are_joined_in_the_order_variables_temps_messages_random() {
        let mut a = FuzzEnv::new(
            Subject::Actor,
            TempLifetime::Persistent,
            EvalLimits::DEFAULT,
            FuzzRng::Xorshift(Xorshift128::new()),
        );
        let mut b = a.clone();
        b.vars.local.set(var("x"), float(0.0));
        b.vars.actors[3].set(var("x"), float(0.0));
        b.temps
            .as_mut()
            .expect("temps")
            .set(TempName::new("t"), float(1.0));
        b.sink.messages.push("m".to_owned());
        sample(&mut b.rng);
        let why = state_difference(&a, &b).expect("differences");
        let parts: Vec<&str> = why.split("; ").collect();
        assert_eq!(parts.len(), 5, "{why}");
        assert!(parts[0].starts_with("variable map 0: "), "{why}");
        assert!(parts[1].starts_with("variable map 4: "), "{why}");
        assert!(parts[2].starts_with("temps: "), "{why}");
        assert!(parts[3].starts_with("messages: "), "{why}");
        assert!(parts[4].starts_with("random source: "), "{why}");
        // Reversed arguments say the same things about the same places.
        let reversed = state_difference(&b, &a).expect("differences");
        assert_eq!(reversed.split("; ").count(), 5);
        a.temps
            .as_mut()
            .expect("temps")
            .set(TempName::new("t"), float(1.0));
        assert_eq!(
            state_difference(&a, &b)
                .expect("differences")
                .split("; ")
                .count(),
            4,
            "the temps now agree"
        );
    }

    proptest::proptest! {
        #![proptest_config(proptest::prelude::ProptestConfig::with_cases(128))]

        #[test]
        fn a_real_difference_in_any_variable_is_never_reported_as_equal(slot in 0usize..=ACTORS, name in 0usize..8, replacement in tree()) {
            let names = ["x", "y", "n", "s", "st", "e", "arr", "a"];
            let a = env();
            let mut b = a.clone();
            let map = if slot == 0 { &mut b.vars.local } else { &mut b.vars.actors[slot - 1] };
            let before = map.get(var(names[name])).cloned();
            let new = replacement.build();
            map.set(var(names[name]), new.clone());
            let really_different = before.as_ref().is_none_or(|old| !Tree::same_as_value(old, &new));
            let verdict = state_difference(&a, &b);
            proptest::prop_assert_eq!(verdict.is_some(), really_different, "slot {} name {} before {:?} after {:?}", slot, names[name], before, new);
        }
    }

    impl Tree {
        /// Whether two built values are the same, by the plain comparison except that floats are by
        /// bits. Independent of `same_value`: written on `Value`.
        fn same_as_value(a: &V, b: &V) -> bool {
            match (a, b) {
                (Value::Float(x), Value::Float(y)) => x.to_bits() == y.to_bits(),
                (Value::Struct(x), Value::Struct(y)) => {
                    x.len() == y.len()
                        && x.iter()
                            .all(|(name, v)| y.get(name).is_some_and(|w| Self::same_as_value(v, w)))
                }
                _ => a == b,
            }
        }
    }
}
