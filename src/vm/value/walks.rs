//! The iterative walks over nested structs: `==`, `Debug`, `Drop` and [`DistinctStructs`].

use std::cell::Cell;
use std::collections::HashSet;
use std::fmt::{self, Debug};
use std::sync::Arc;

use smallvec::SmallVec;

use super::members::Members;
use super::{StructValue, Value};
use crate::vm::host::Host;

/// Structural equality: same kind and payload, floats compared with `==`, struct members in any
/// order. Molang's `==` is [`Value::molang_eq`].
///
/// Runs in constant stack and compares each pair of structs once (the same `Arc` is equal without
/// a walk), so a struct that shares members costs its distinct pairs, not its paths.
impl<H: Host> PartialEq for Value<H> {
    fn eq(&self, other: &Self) -> bool {
        let mut pending: SmallVec<[(&Self, &Self); 8]> = SmallVec::new();
        pending.push((self, other));
        all_equal(pending)
    }
}

fn all_equal<'a, H: Host>(mut pending: SmallVec<[(&'a Value<H>, &'a Value<H>); 8]>) -> bool {
    // The pairs of structs compared or queued so far, by address.
    let mut compared: HashSet<(usize, usize)> = HashSet::new();
    while let Some(pair) = pending.pop() {
        let equal = match pair {
            (Value::Float(a), Value::Float(b)) => a == b,
            (Value::Hash(a), Value::Hash(b)) => a == b,
            (Value::Actor(a), Value::Actor(b)) => a == b,
            (Value::Item(a), Value::Item(b)) => a == b,
            (Value::ActorArray(a), Value::ActorArray(b)) => a == b,
            (Value::Struct(a), Value::Struct(b)) => {
                Arc::ptr_eq(a, b)
                    || !compared.insert((Arc::as_ptr(a).addr(), Arc::as_ptr(b).addr()))
                    || queue_members(a, b, &mut pending)
            }
            (Value::Matrix(a), Value::Matrix(b)) => a == b,
            (Value::Resource(a), Value::Resource(b)) => a == b,
            _ => false,
        };
        if !equal {
            return false;
        }
    }
    true
}

/// `false` when the member names differ. Members in the same order (a copy) pair by position,
/// others by name.
fn queue_members<'a, H: Host>(
    a: &'a StructValue<H>,
    b: &'a StructValue<H>,
    pending: &mut SmallVec<[(&'a Value<H>, &'a Value<H>); 8]>,
) -> bool {
    if a.members.len() != b.members.len() {
        return false;
    }
    for (index, (name, value)) in a.members.iter().enumerate() {
        let theirs = match b.members.get(index) {
            Some((their_name, theirs)) if their_name == name => theirs,
            _ => match b.get(*name) {
                Some(theirs) => theirs,
                None => return false,
            },
        };
        pending.push((value, theirs));
    }
    true
}

/// Struct levels `Debug` prints before `Struct(..)`: shallow enough that formatting cannot exhaust
/// the stack.
const DEBUG_DEPTH: u32 = 32;

/// Members one `Debug` prints in all before `..`. Structs share members, so a few hundred distinct
/// structs can have 2^31 paths to their leaves.
const DEBUG_MEMBERS: usize = 4096;

/// `budget` is shared by the whole print.
struct DebugAt<'a, 'b, H: Host> {
    value: &'a Value<H>,
    depth: u32,
    budget: &'b Cell<usize>,
}

impl<H: Host> Debug for DebugAt<'_, '_, H> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.value {
            Value::Float(x) => f.debug_tuple("Float").field(x).finish(),
            Value::Hash(h) => write!(f, "Hash({:#018x})", h.as_u64()),
            Value::Actor(a) => f.debug_tuple("Actor").field(a).finish(),
            Value::Item(i) => f.debug_tuple("Item").field(i).finish(),
            Value::ActorArray(a) => f.debug_tuple("ActorArray").field(a).finish(),
            Value::Struct(_) if self.depth >= DEBUG_DEPTH => f.write_str("Struct(..)"),
            Value::Struct(s) => f
                .debug_tuple("Struct")
                .field(&DebugMembers {
                    members: s,
                    depth: self.depth + 1,
                    budget: self.budget,
                })
                .finish(),
            Value::Matrix(m) => f.debug_tuple("Matrix").field(m).finish(),
            Value::Resource(r) => f.debug_tuple("Resource").field(r).finish(),
        }
    }
}

struct DebugMembers<'a, 'b, H: Host> {
    members: &'a StructValue<H>,
    depth: u32,
    budget: &'b Cell<usize>,
}

impl<H: Host> Debug for DebugMembers<'_, '_, H> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut map = f.debug_map();
        for (name, value) in &self.members.members {
            let left = self.budget.get();
            if left == 0 {
                return map.finish_non_exhaustive();
            }
            self.budget.set(left - 1);
            map.entry(
                &name.as_u64(),
                &DebugAt {
                    value,
                    depth: self.depth,
                    budget: self.budget,
                },
            );
        }
        map.finish()
    }
}

/// Structs nested deeper than 32 levels print as `Struct(..)`, and after 4,096 members in all
/// the rest of each struct prints as `..`, so a value whose structs share members prints in
/// bounded time.
impl<H: Host> Debug for Value<H> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let budget = Cell::new(DEBUG_MEMBERS);
        DebugAt {
            value: self,
            depth: 0,
            budget: &budget,
        }
        .fmt(f)
    }
}

/// The distinct structs a value, or a set of values, holds: each struct once, however many members
/// refer to it.
///
/// Structs share members: `v.a.z = 1; loop(31, { v.b = v.a; v.a.x = v.b; v.a.y = v.b; });` builds
/// 32 structs with 2^31 paths to the innermost one, within every budget of
/// [`EvalLimits`](crate::vm::EvalLimits). A host that saves, sends or measures variables walks
/// them with this iterator (or its own walk deduplicating on [`Arc::as_ptr`] of
/// [`Value::Struct`]) or bounds its walk; a plain recursive walk takes time exponential in the
/// depth.
///
/// ```
/// # #[cfg(feature = "stdlib")]
/// # {
/// use molangx::compile::{CompileOptions, compile};
/// use molangx::version::MolangVersion;
/// use molangx::vm::{StructValue, NoHostEnv, Value, VariableName};
///
/// let (expr, _) = compile(
///     "v.a.z = 1; loop(31, { v.b = v.a; v.a.x = v.b; v.a.y = v.b; });",
///     &CompileOptions::server(MolangVersion::LATEST),
/// )
/// .into_result()
/// .unwrap();
/// let mut env = NoHostEnv::new();
/// expr.eval(&mut env.cx());
/// let a = env.variables.get(VariableName::new("a")).unwrap();
/// assert_eq!(a.distinct_structs().count(), 32);
/// // 31 structs of {z, x, y} and the innermost {z}.
/// assert_eq!(a.distinct_structs().map(StructValue::len).sum::<usize>(), 31 * 3 + 1);
/// # }
/// ```
pub struct DistinctStructs<'a, H: Host> {
    pending: Vec<&'a Value<H>>,
    seen: HashSet<usize>,
}

impl<'a, H: Host> DistinctStructs<'a, H> {
    /// The distinct structs of all `roots`, a struct they share counted once (for a variable map:
    /// `DistinctStructs::of(map.iter().map(|(_, value)| value))`).
    pub fn of(roots: impl IntoIterator<Item = &'a Value<H>>) -> Self {
        Self {
            pending: roots.into_iter().collect(),
            seen: HashSet::new(),
        }
    }
}

impl<'a, H: Host> Iterator for DistinctStructs<'a, H> {
    type Item = &'a StructValue<H>;

    fn next(&mut self) -> Option<Self::Item> {
        while let Some(value) = self.pending.pop() {
            if let Value::Struct(members) = value
                && self.seen.insert(Arc::as_ptr(members).addr())
            {
                self.pending.extend(
                    members
                        .members
                        .iter()
                        .map(|(_, member)| member)
                        .filter(|member| matches!(member, Value::Struct(_))),
                );
                return Some(members);
            }
        }
        None
    }
}

impl<H: Host> Debug for DistinctStructs<'_, H> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DistinctStructs")
            .field("seen", &self.seen.len())
            .field("pending", &self.pending.len())
            .finish()
    }
}

impl<H: Host> Value<H> {
    /// The distinct structs this value holds, itself included; see [`DistinctStructs`] for why a
    /// walk over stored values needs it.
    pub fn distinct_structs(&self) -> DistinctStructs<'_, H> {
        DistinctStructs::of([self])
    }
}

/// Same members in any order, compared as [`Value`]'s `PartialEq` compares them.
impl<H: Host> PartialEq for StructValue<H> {
    fn eq(&self, other: &Self) -> bool {
        let mut pending = SmallVec::new();
        std::ptr::eq(self, other)
            || (queue_members(self, other, &mut pending) && all_equal(pending))
    }
}

/// Structs nested deeper than 32 levels print as `Struct(..)`, and after 4,096 members in all
/// the rest of each struct prints as `..`.
impl<H: Host> Debug for StructValue<H> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let budget = Cell::new(DEBUG_MEMBERS);
        DebugMembers {
            members: self,
            depth: 1,
            budget: &budget,
        }
        .fmt(f)
    }
}

/// Runs in constant stack: an expression can build one level per instruction (`v.a.b = v.a` in a
/// loop).
impl<H: Host> Drop for StructValue<H> {
    fn drop(&mut self) {
        if !self
            .members
            .iter()
            .any(|(_, value)| matches!(value, Value::Struct(_)))
        {
            return;
        }
        let mut pending = Vec::new();
        take_structs(&mut self.members, &mut pending);
        while let Some(shared) = pending.pop() {
            // The last owner empties the struct before it is dropped (its own `drop` then has
            // nothing to do); any other owner only releases its reference.
            if let Some(mut inner) = Arc::into_inner(shared) {
                take_structs(&mut inner.members, &mut pending);
            }
        }
    }
}

/// Moves the struct members out of `members` (leaving float 0 behind) onto `pending`.
fn take_structs<H: Host>(members: &mut Members<H>, pending: &mut Vec<Arc<StructValue<H>>>) {
    for (_, value) in members.iter_mut() {
        if matches!(value, Value::Struct(_))
            && let Value::Struct(inner) = std::mem::take(value)
        {
            pending.push(inner);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use super::*;
    use crate::hash::HashedStr;
    use crate::vm::{
        test_support::TestHost,
        value::{IDENTITY_MATRIX, ResourceRef, test_support::*},
    };

    #[test]
    fn struct_value_equality_is_order_independent() {
        let a = StructValue::<TestHost>::from([("x", 1.0), ("y", 2.0)]);
        let b = StructValue::<TestHost>::from([("y", 2.0), ("x", 1.0)]);
        assert_eq!(a, b);
        assert_ne!(a, StructValue::from([("x", 1.0)]));
        assert_ne!(a, StructValue::from([("x", 1.0), ("y", 3.0)]));
        assert_eq!(StructValue::<TestHost>::rgb(1.0, 2.0, 3.0).iter().len(), 3);
        assert!(StructValue::<TestHost>::default().is_empty());
    }

    #[test]
    fn struct_value_equality_needs_the_same_names_and_values() {
        let xy = M::xy(1.0, 2.0);
        assert_eq!(xy, xy.clone());
        assert_eq!(xy, M::from([("y", 2.0), ("x", 1.0)]));
        assert_ne!(xy, M::new());
        assert_ne!(xy, M::from([("x", 1.0), ("z", 2.0)]));
        assert_ne!(xy, M::xy(1.0, 3.0));
        assert_ne!(xy, M::xyz(1.0, 2.0, 0.0));
        assert_ne!(M::new(), xy);
        assert_eq!(
            M::xyz(1.0, 2.0, 3.0),
            M::from([("z", 3.0), ("y", 2.0), ("x", 1.0)])
        );
    }

    #[test]
    fn struct_value_equality_compares_nested_structs() {
        let a = M::min_and_max([0.0, 1.0, 2.0], [3.0, 4.0, 5.0]);
        assert_eq!(a, M::min_and_max([0.0, 1.0, 2.0], [3.0, 4.0, 5.0]));
        assert_ne!(a, M::min_and_max([0.0, 1.0, 2.0], [3.0, 4.0, 6.0]));
        let reordered = M::from([
            ("max", M::from([("z", 5.0), ("y", 4.0), ("x", 3.0)])),
            ("min", M::xyz(0.0, 1.0, 2.0)),
        ]);
        assert_eq!(a, reordered);
    }

    #[test]
    fn structural_equality() {
        assert_eq!(V::Item(7), V::Item(7));
        assert_ne!(V::Float(1.0), V::Hash(HashedStr::from_u64(1)));
        assert_ne!(V::Float(f32::NAN), V::Float(f32::NAN));
        assert_eq!(V::actor_array([1, 2]), V::actor_array([1, 2]));
        assert_ne!(V::actor_array([1, 2]), V::actor_array([2, 1]));
        assert_eq!(V::default(), V::ZERO);
    }

    #[test]
    fn structural_equality_by_kind() {
        assert_eq!(V::Actor(1), V::Actor(1));
        assert_ne!(V::Actor(1), V::Actor(2));
        assert_ne!(V::Actor(1), V::Item(1));
        assert_ne!(V::Item(1), V::Item(2));
        assert_eq!(V::identity_matrix(), V::identity_matrix());
        let mut other = IDENTITY_MATRIX;
        other[15] = 2.0;
        assert_ne!(V::identity_matrix(), V::Matrix(Arc::new(other)));
        assert_eq!(
            V::Resource(ResourceRef::new("a.b")),
            V::Resource(ResourceRef::new("a.b"))
        );
        assert_ne!(
            V::Resource(ResourceRef::new("a.b")),
            V::Resource(ResourceRef::new("a.c"))
        );
        assert_ne!(V::structure(M::xy(1.0, 2.0)), V::Float(0.0));
        assert_ne!(V::structure(M::new()), V::actor_array([]));
        // The sharing of the `Arc` does not matter.
        assert_eq!(V::structure(M::xy(1.0, 2.0)), V::structure(M::xy(1.0, 2.0)));
        assert_ne!(V::structure(M::xy(1.0, 2.0)), V::structure(M::xy(1.0, 5.0)));
    }

    // A clone shares the `Arc`, so it is equal even with a NaN member.
    #[test]
    fn a_shared_struct_is_equal_to_itself_without_a_walk() {
        let a = V::structure(M::from([("x", f32::NAN)]));
        let shared = a.clone();
        assert_eq!(a, shared);
        // Nested: the same inner struct under two parents.
        let outer_a = V::structure(M::from([("in", a.clone())]));
        let outer_b = V::structure(M::from([("in", shared)]));
        assert_eq!(outer_a, outer_b);
        // Built apart, the NaN members are unequal to each other.
        assert_ne!(a, V::structure(M::from([("x", f32::NAN)])));
        assert_ne!(
            outer_a,
            V::structure(M::from([("in", V::structure(M::from([("x", f32::NAN)])))]))
        );
    }

    #[test]
    fn equality_of_deep_structs_does_not_recurse() {
        on_a_small_stack(|| {
            let a = chain(100_000);
            let b = chain(100_000);
            assert_eq!(a, b);
            assert_eq!(a, a.clone());
            // Differing only in the innermost value.
            let other_leaf = {
                let mut value = V::Float(2.0);
                for _ in 0..100_000 {
                    value = V::structure(M::from([("a", value)]));
                }
                value
            };
            assert_ne!(a, other_leaf);
        });
    }

    #[test]
    fn equality_of_a_shared_dag_compares_each_pair_once() {
        let started = Instant::now();
        let a = doubling(40);
        let rebuilt = doubling(40);
        assert_eq!(a, a.clone());
        assert_eq!(a, rebuilt);
        // A single different leaf, 40 levels down, is found.
        let mut different = doubling(40);
        different.set_member_path(&[h("z")], V::Float(2.0));
        assert_ne!(a, different);
        // So is one that only differs in the depth of the DAG.
        assert_ne!(doubling(39), doubling(40));
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "took {:?}",
            started.elapsed()
        );
    }

    #[test]
    fn equality_of_struct_values_over_a_shared_dag_is_fast_too() {
        let started = Instant::now();
        let (V::Struct(a), V::Struct(b)) = (doubling(40), doubling(40)) else {
            panic!("structs")
        };
        assert_eq!(*a, *b);
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "took {:?}",
            started.elapsed()
        );
    }

    #[test]
    fn debug_of_scalars() {
        assert_eq!(format!("{:?}", V::Float(1.0)), "Float(1.0)");
        assert_eq!(format!("{:?}", V::Actor(3)), "Actor(3)");
        assert_eq!(format!("{:?}", V::Item(4)), "Item(4)");
        assert_eq!(
            format!("{:?}", V::actor_array([1, 2])),
            "ActorArray([1, 2])"
        );
        assert_eq!(format!("{:?}", V::string("a")), "Hash(0xaf63bd4c8601b7be)");
        // The hash is always 16 digits.
        assert_eq!(format!("{:?}", V::string("")), "Hash(0x0000000000000000)");
        assert!(
            format!("{:?}", V::identity_matrix())
                .starts_with("Matrix([1.0, 0.0, 0.0, 0.0, 0.0, 1.0")
        );
        assert!(
            format!("{:?}", V::Resource(ResourceRef::new("texture.default")))
                .starts_with("Resource(")
        );
    }

    #[test]
    fn debug_of_a_struct_lists_members_by_hash() {
        let v = V::structure(M::from([("x", 1.0)]));
        assert_eq!(
            format!("{v:?}"),
            format!("Struct({{{}: Float(1.0)}})", h("x").as_u64())
        );
        assert_eq!(
            format!("{:?}", M::from([("x", 1.0)])),
            format!("{{{}: Float(1.0)}}", h("x").as_u64())
        );
        assert_eq!(format!("{:?}", M::new()), "{}");
    }

    #[test]
    fn debug_abbreviates_a_value_past_32_levels() {
        let text = format!("{:?}", chain(40));
        assert_eq!(text.matches("Struct({").count(), 32);
        assert_eq!(text.matches("Struct(..)").count(), 1);
        // A value of exactly 32 levels prints in full.
        let full = format!("{:?}", chain(32));
        assert_eq!(full.matches("Struct({").count(), 32);
        assert_eq!(full.matches("Struct(..)").count(), 0);
        assert!(full.contains("Float(1.0)"));
        assert!(!text.contains("Float(1.0)"));
    }

    #[test]
    fn debug_of_a_struct_value_counts_its_own_members_as_one_level() {
        let V::Struct(root) = chain(40) else {
            panic!("struct")
        };
        let text = format!("{root:?}");
        // The root is level 1, so 31 nested levels print in full before the abbreviation.
        assert_eq!(text.matches("Struct({").count(), 31);
        assert_eq!(text.matches("Struct(..)").count(), 1);
    }

    #[test]
    fn debug_prints_at_most_4096_members() {
        let mut wide = M::new();
        for i in 0..5000 {
            wide.set(HashedStr::from_u64(i + 1), V::Float(i as f32));
        }
        let text = format!("{wide:?}");
        assert_eq!(text.matches("Float(").count(), 4096);
        assert!(text.ends_with(", ..}"), "{}", &text[text.len() - 20..]);
        let value = format!("{:?}", V::structure(wide));
        assert_eq!(value.matches("Float(").count(), 4096);
        assert!(value.ends_with(", ..})"));
        // Under the budget nothing is abbreviated.
        let mut narrow = M::new();
        for i in 0..4096 {
            narrow.set(HashedStr::from_u64(i + 1), V::Float(0.0));
        }
        let text = format!("{narrow:?}");
        assert_eq!(text.matches("Float(").count(), 4096);
        assert!(!text.contains(".."));
    }

    #[test]
    fn debug_of_a_shared_dag_is_bounded_in_time_and_size() {
        let started = Instant::now();
        let v = doubling(40);
        let text = format!("{v:?}");
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "took {:?}",
            started.elapsed()
        );
        assert!(text.contains(".."));
        assert!(text.len() < 1_000_000, "{} bytes", text.len());
        let V::Struct(root) = &v else {
            panic!("struct")
        };
        let text = format!("{root:?}");
        assert!(text.contains("..") && text.len() < 1_000_000);
        // The budget is shared by the whole print: 4096 members at most, however they nest.
        assert!(text.matches("Float(").count() <= 4096);
    }

    #[test]
    fn clone_compare_debug_and_drop_a_struct_100000_levels_deep_on_a_small_stack() {
        on_a_small_stack(|| {
            let deep = chain(100_000);
            assert_eq!(deep.struct_depth(), 100_000);
            let copy = deep.clone();
            assert_eq!(copy, deep);
            let text = format!("{deep:?}");
            assert_eq!(text.matches("Struct({").count(), 32);
            assert_eq!(text.matches("Struct(..)").count(), 1);
            let V::Struct(root) = &deep else {
                panic!("struct")
            };
            assert!(format!("{root:?}").contains("Struct(..)"));
            // Walking it by path is a loop too.
            let path = vec![h("a"); 99_999];
            assert!(deep.member_path(&path).is_some());
            drop(copy);
            drop(deep);
        });
    }

    #[test]
    fn dropping_a_deep_struct_is_iterative() {
        on_a_small_stack(|| {
            drop(chain(100_000));
            // A struct value dropped directly, not through a value.
            let V::Struct(root) = chain(100_000) else {
                panic!("struct")
            };
            drop(root);
        });
    }

    #[test]
    fn dropping_a_shared_child_releases_it_with_the_last_owner() {
        let child = V::structure(M::xy(1.0, 2.0));
        let weak = Arc::downgrade(arc_of(&child));
        let first = V::structure(M::from([("c", child.clone())]));
        let second = V::structure(M::from([("c", child.clone())]));
        drop(child);
        assert!(weak.upgrade().is_some());
        drop(first);
        assert!(weak.upgrade().is_some(), "the second parent still holds it");
        drop(second);
        assert!(weak.upgrade().is_none());
    }

    #[test]
    fn dropping_a_dag_frees_every_struct() {
        let started = Instant::now();
        let v = doubling(40);
        let weak = Arc::downgrade(arc_of(&v));
        drop(v);
        assert!(weak.upgrade().is_none());
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "took {:?}",
            started.elapsed()
        );
    }

    #[test]
    fn dropping_a_struct_leaves_the_values_of_other_owners_intact() {
        let inner = V::structure(M::xy(1.0, 2.0));
        let outer = V::structure(M::from([("c", inner.clone()), ("n", V::Float(5.0))]));
        drop(outer);
        assert_eq!(inner, V::structure(M::xy(1.0, 2.0)));
    }

    #[test]
    fn distinct_structs_visits_each_struct_once() {
        let v = doubling(31);
        assert_eq!(v.distinct_structs().count(), 32);
        // 31 structs of {z, x, y} and the innermost {z}.
        assert_eq!(
            v.distinct_structs().map(StructValue::len).sum::<usize>(),
            31 * 3 + 1
        );
    }

    #[test]
    fn distinct_structs_of_a_deep_dag_is_fast() {
        let started = Instant::now();
        let v = doubling(40);
        assert_eq!(v.distinct_structs().count(), 41);
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "took {:?}",
            started.elapsed()
        );
    }

    #[test]
    fn distinct_structs_of_a_non_struct_is_empty() {
        for value in [
            V::Float(1.0),
            V::string("a"),
            V::Actor(1),
            V::actor_array([1]),
            V::identity_matrix(),
        ] {
            assert_eq!(value.distinct_structs().count(), 0, "{value:?}");
        }
    }

    #[test]
    fn distinct_structs_includes_the_value_itself_and_the_nested_ones() {
        let v = V::structure(M::from([
            ("one", V::structure(M::xy(1.0, 2.0))),
            ("n", V::Float(1.0)),
        ]));
        let lens: Vec<usize> = v.distinct_structs().map(StructValue::len).collect();
        assert_eq!(lens.len(), 2);
        assert!(lens.contains(&2));
        // The first one yielded is the root.
        assert_eq!(v.distinct_structs().next().map(StructValue::len), Some(2));
    }

    #[test]
    fn distinct_structs_of_several_roots_counts_a_shared_struct_once() {
        let shared = V::structure(M::xy(1.0, 2.0));
        let a = V::structure(M::from([("s", shared.clone())]));
        let b = V::structure(M::from([("s", shared.clone())]));
        // a, b, and the one struct they share.
        assert_eq!(DistinctStructs::of([&a, &b]).count(), 3);
        // The same root twice is one struct.
        assert_eq!(DistinctStructs::of([&shared, &shared]).count(), 1);
        assert_eq!(DistinctStructs::<TestHost>::of([]).count(), 0);
    }

    #[test]
    fn distinct_structs_debug_shows_the_work_left() {
        let v = V::structure(M::from([("one", M::xy(1.0, 2.0))]));
        let mut it = v.distinct_structs();
        assert_eq!(format!("{it:?}"), "DistinctStructs { seen: 0, pending: 1 }");
        it.next();
        assert_eq!(format!("{it:?}"), "DistinctStructs { seen: 1, pending: 1 }");
        it.next();
        assert_eq!(format!("{it:?}"), "DistinctStructs { seen: 2, pending: 0 }");
        assert!(it.next().is_none());
    }

    #[test]
    fn distinct_structs_follows_only_struct_members() {
        let v = V::structure(M::from([
            ("list", V::actor_array(0..1000)),
            ("n", V::Float(1.0)),
        ]));
        assert_eq!(v.distinct_structs().count(), 1);
    }
}
