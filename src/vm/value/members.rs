//! [`StructValue`], the struct value, and reading and writing members through a [`Value`].

use std::slice;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

use smallvec::SmallVec;

use super::Value;
use crate::hash::HashedStr;
use crate::vm::name::hash_lowered;
use crate::vm::{DuplicateMember, EvalLimits, host::Host};

/// A struct value: a short list of uniquely named members.
///
/// Members are keyed by the [`HashedStr`] of the member name **without its leading dot**, in
/// lower case (`v.a.b` → `HashedStr::new("b")`); build keys with [`StructValue::key`]. Two struct
/// values are equal when they hold the same members, in any order.
pub struct StructValue<H: Host> {
    pub(super) members: Members<H>,
    /// The cached [`StructValue::depth`], or [`UNKNOWN_DEPTH`]. Every `&mut self` method resets
    /// it; a nested struct changes only through `&mut` access to every struct above it (or by
    /// being copied), so the cache is never stale.
    depth: AtomicU32,
}

/// The members of a [`StructValue`], in insertion order.
pub(super) type Members<H> = SmallVec<[(HashedStr, Value<H>); 4]>;

/// No depth is cached; a struct is at least 1 deep.
const UNKNOWN_DEPTH: u32 = 0;

impl<H: Host> StructValue<H> {
    /// An empty struct.
    pub fn new() -> Self {
        Self {
            members: SmallVec::new(),
            depth: AtomicU32::new(UNKNOWN_DEPTH),
        }
    }

    fn changed(&mut self) {
        *self.depth.get_mut() = UNKNOWN_DEPTH;
    }

    /// How many struct levels this struct nests: 1 when no member is a struct, else one more
    /// than its deepest struct member.
    ///
    /// Computed without recursion and cached until the struct next changes.
    pub fn depth(&self) -> u32 {
        struct Visit<'a, H: Host> {
            node: &'a StructValue<H>,
            members: slice::Iter<'a, (HashedStr, Value<H>)>,
            depth: u32,
        }
        fn visit<H: Host>(node: &StructValue<H>) -> Visit<'_, H> {
            Visit {
                node,
                members: node.members.iter(),
                depth: 1,
            }
        }
        if let Some(depth) = self.cached_depth() {
            return depth;
        }
        let mut stack = vec![visit(self)];
        // The root finishes last.
        let mut finished = 1;
        while let Some(top) = stack.last_mut() {
            match top.members.next() {
                Some((_, Value::Struct(child))) => match child.cached_depth() {
                    Some(known) => top.depth = top.depth.max(known.saturating_add(1)),
                    None => stack.push(visit(child)),
                },
                Some(_) => {}
                None => {
                    finished = top.depth;
                    top.node.depth.store(finished, Ordering::Relaxed);
                    stack.pop();
                    if let Some(parent) = stack.last_mut() {
                        parent.depth = parent.depth.max(finished.saturating_add(1));
                    }
                }
            }
        }
        finished
    }

    fn cached_depth(&self) -> Option<u32> {
        match self.depth.load(Ordering::Relaxed) {
            UNKNOWN_DEPTH => None,
            depth => Some(depth),
        }
    }

    /// Number of members.
    pub fn len(&self) -> usize {
        self.members.len()
    }

    /// Whether the struct has no member.
    pub fn is_empty(&self) -> bool {
        self.members.is_empty()
    }

    /// The members in insertion order.
    pub fn iter(&self) -> impl ExactSizeIterator<Item = (HashedStr, &Value<H>)> {
        self.members.iter().map(|(name, value)| (*name, value))
    }

    /// The member `name`.
    pub fn get(&self, name: HashedStr) -> Option<&Value<H>> {
        self.members
            .iter()
            .find(|(n, _)| *n == name)
            .map(|(_, v)| v)
    }

    /// The member `name`, mutably.
    pub fn get_mut(&mut self, name: HashedStr) -> Option<&mut Value<H>> {
        self.changed();
        self.members
            .iter_mut()
            .find(|(n, _)| *n == name)
            .map(|(_, v)| v)
    }

    /// Adds a new member; a name already present is an error and leaves the struct unchanged.
    pub fn add(
        &mut self,
        name: HashedStr,
        value: impl Into<Value<H>>,
    ) -> Result<(), DuplicateMember> {
        if self.get(name).is_some() {
            return Err(DuplicateMember { name });
        }
        self.changed();
        self.members.push((name, value.into()));
        Ok(())
    }

    /// Sets a member, returning the value it replaces.
    pub fn set(&mut self, name: HashedStr, value: impl Into<Value<H>>) -> Option<Value<H>> {
        let value = value.into();
        if let Some(slot) = self.get_mut(name) {
            return Some(std::mem::replace(slot, value));
        }
        self.members.push((name, value));
        None
    }

    /// The key of the member `name`: its [`HashedStr`] with ASCII `A`–`Z` lowered and the text
    /// ending at the first NUL byte ([`Name`](crate::vm::Name) keys do the same). A member keyed
    /// with upper-case letters (`HashedStr::new("Speed")`) is unreachable from an expression.
    ///
    /// ```
    /// use molangx::hash::HashedStr;
    /// use molangx::vm::{StructValue, NoHost};
    ///
    /// assert_eq!(StructValue::<NoHost>::key("Speed"), HashedStr::new("speed"));
    /// ```
    pub const fn key(name: &str) -> HashedStr {
        hash_lowered("", name.as_bytes())
    }

    /// The member `name`, inserting `default()` first when it is absent.
    pub fn get_or_insert_with(
        &mut self,
        name: HashedStr,
        default: impl FnOnce() -> Value<H>,
    ) -> &mut Value<H> {
        self.changed();
        let index = if let Some(index) = self.members.iter().position(|(n, _)| *n == name) {
            index
        } else {
            self.members.push((name, default()));
            self.members.len() - 1
        };
        &mut self.members[index].1
    }

    /// `{x, y}`.
    pub fn xy(x: f32, y: f32) -> Self {
        Self::from([("x", x), ("y", y)])
    }

    /// `{x, y, z}`.
    pub fn xyz(x: f32, y: f32, z: f32) -> Self {
        Self::from([("x", x), ("y", y), ("z", z)])
    }

    /// `{u, v}`.
    pub fn uv(u: f32, v: f32) -> Self {
        Self::from([("u", u), ("v", v)])
    }

    /// `{r, g, b}`.
    pub fn rgb(r: f32, g: f32, b: f32) -> Self {
        Self::from([("r", r), ("g", g), ("b", b)])
    }

    /// `{r, g, b, a}` (`query.spellcolor`).
    pub fn rgba(r: f32, g: f32, b: f32, a: f32) -> Self {
        Self::from([("r", r), ("g", g), ("b", b), ("a", a)])
    }

    /// `{min: {x, y, z}, max: {x, y, z}}` (`query.bone_aabb`).
    pub fn min_and_max(min: [f32; 3], max: [f32; 3]) -> Self {
        Self::from([("min", Self::xyz_of(min)), ("max", Self::xyz_of(max))])
    }

    /// `{t: {x, y, z}, r: {x, y, z}, s: {x, y, z}}` (`query.bone_orientation_trs`).
    pub fn trs(translation: [f32; 3], rotation: [f32; 3], scale: [f32; 3]) -> Self {
        Self::from([
            ("t", Self::xyz_of(translation)),
            ("r", Self::xyz_of(rotation)),
            ("s", Self::xyz_of(scale)),
        ])
    }

    fn xyz_of([x, y, z]: [f32; 3]) -> Self {
        Self::xyz(x, y, z)
    }
}

impl<H: Host> Default for StructValue<H> {
    fn default() -> Self {
        Self::new()
    }
}

impl<H: Host> Clone for StructValue<H> {
    fn clone(&self) -> Self {
        Self {
            members: self.members.clone(),
            depth: AtomicU32::new(self.depth.load(Ordering::Relaxed)),
        }
    }
}

impl<'n, H: Host, V: Into<Value<H>>> FromIterator<(&'n str, V)> for StructValue<H> {
    /// Members keyed by [`StructValue::key`]; a later value of a name replaces an earlier one.
    fn from_iter<I: IntoIterator<Item = (&'n str, V)>>(members: I) -> Self {
        let mut s = Self::new();
        for (name, value) in members {
            s.set(Self::key(name), value.into());
        }
        s
    }
}

impl<'n, H: Host, V: Into<Value<H>>, const LEN: usize> From<[(&'n str, V); LEN]>
    for StructValue<H>
{
    fn from(members: [(&'n str, V); LEN]) -> Self {
        members.into_iter().collect()
    }
}

/// What [`Value::member_store_check`] finds out about a member store before it is made.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct MemberStoreCheck {
    /// The steps the store costs.
    pub cost: u64,
    /// The width budget a struct on the path would exceed by gaining a member.
    pub exceeded_width: Option<u32>,
}

impl<H: Host> Value<H> {
    /// The member `name` of a struct value (`None` for a missing member or a non-struct).
    pub fn member(&self, name: HashedStr) -> Option<&Value<H>> {
        self.as_struct()?.get(name)
    }

    /// The value at the end of a member chain (`v.a.b.c` → `[b, c]` on the value of `v.a`).
    pub fn member_path(&self, path: &[HashedStr]) -> Option<&Value<H>> {
        path.iter()
            .try_fold(self, |value, name| value.member(*name))
    }

    /// Writes `value` at the end of a member chain (`v.a.b = …`), creating intermediate structs
    /// and replacing any non-struct on the way. An empty `path` replaces `self`.
    ///
    /// A shared struct on the way is copied once (`Arc` copy-on-write). Runs in constant stack.
    pub fn set_member_path(&mut self, path: &[HashedStr], value: Value<H>) {
        let mut cursor = self;
        for name in path {
            cursor = cursor
                .make_struct()
                .get_or_insert_with(*name, Self::default);
        }
        *cursor = value;
    }

    /// The step cost of [`Value::set_member_path`] on this value, and whether it would add a
    /// member to a struct already holding `width_limit` members
    /// ([`EvalLimits::struct_members`]).
    ///
    /// Every struct on the path costs its member count plus
    /// [`EvalLimits::STRUCT_COPY_STEPS`] whether or not
    /// it is shared, so the cost depends only on the values; a level the write creates costs the
    /// constant alone.
    pub(crate) fn member_store_check(
        &self,
        path: &[HashedStr],
        width_limit: Option<u32>,
    ) -> MemberStoreCheck {
        let mut check = MemberStoreCheck {
            cost: 0,
            exceeded_width: None,
        };
        let mut cursor = Some(self);
        for name in path {
            let (len, next) = match cursor {
                Some(Self::Struct(members)) => (members.len(), members.get(*name)),
                _ => (0, None),
            };
            check.cost = check
                .cost
                .saturating_add(EvalLimits::STRUCT_COPY_STEPS)
                .saturating_add(len as u64);
            if next.is_none()
                && let Some(limit) = width_limit
                && len >= limit as usize
            {
                check.exceeded_width = Some(limit);
            }
            cursor = next;
        }
        check
    }

    /// Replaces a non-struct by an empty struct first.
    fn make_struct(&mut self) -> &mut StructValue<H> {
        if !matches!(self, Self::Struct(_)) {
            *self = Self::structure(StructValue::new());
        }
        match self {
            Self::Struct(members) => Arc::make_mut(members),
            _ => unreachable!("a struct was just stored"),
        }
    }

    /// How many struct levels the value nests: 0 for a non-struct, else [`StructValue::depth`]
    /// (`v.a.b.c = 1` makes `v.a` 2 deep). Checked against
    /// [`EvalLimits::struct_depth`] after every member store.
    pub fn struct_depth(&self) -> u32 {
        match self {
            Self::Struct(members) => members.depth(),
            _ => 0,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::time::{Duration, Instant};

    use super::*;
    use crate::vm::{EvalLimits, test_support::TestHost, value::test_support::*};

    #[test]
    fn member_lookup() {
        let v = V::structure(StructValue::xyz(1.0, 2.0, 3.0));
        assert_eq!(v.member(h("y")), Some(&V::Float(2.0)));
        assert_eq!(v.member(h("w")), None);
        assert_eq!(V::Float(1.0).member(h("x")), None);
        let trs = V::structure(StructValue::trs(
            [1.0, 2.0, 3.0],
            [4.0, 5.0, 6.0],
            [7.0, 8.0, 9.0],
        ));
        assert_eq!(trs.member_path(&[h("r"), h("x")]), Some(&V::Float(4.0)));
        assert_eq!(trs.member_path(&[h("s"), h("z")]), Some(&V::Float(9.0)));
        assert_eq!(trs.member_path(&[h("r"), h("x"), h("deeper")]), None);
        assert_eq!(trs.member_path(&[]), Some(&trs));
        let aabb = StructValue::<TestHost>::min_and_max([0.0, 1.0, 2.0], [3.0, 4.0, 5.0]);
        assert_eq!(
            aabb.get(h("max")).and_then(|m| m.member(h("y"))),
            Some(&V::Float(4.0))
        );
    }

    #[test]
    fn member_path_through_a_non_struct_finds_nothing() {
        let v = V::structure(M::from([("a", 1.0)]));
        assert_eq!(v.member_path(&[h("a")]), Some(&V::Float(1.0)));
        assert_eq!(v.member_path(&[h("a"), h("b")]), None);
        assert_eq!(v.member_path(&[h("missing"), h("b")]), None);
        assert_eq!(V::Float(1.0).member_path(&[]), Some(&V::Float(1.0)));
        assert_eq!(V::Float(1.0).member_path(&[h("a")]), None);
    }

    #[test]
    fn duplicate_member_names_are_an_error() {
        let mut s = StructValue::<TestHost>::new();
        assert!(s.add(h("x"), V::Float(1.0)).is_ok());
        let err = s.add(h("x"), V::Float(2.0)).unwrap_err();
        assert_eq!(err.name, h("x"));
        assert!(
            err.to_string()
                .starts_with("molangx: a struct already has a member named '")
        );
        assert!(
            err.to_string()
                .ends_with(&format!("{:#018x}'", h("x").as_u64()))
        );
        assert_eq!(s.get(h("x")), Some(&V::Float(1.0)));
        assert_eq!(s.len(), 1);
        // An assignment replaces instead.
        s.set(h("x"), V::Float(3.0));
        assert_eq!(s.get(h("x")), Some(&V::Float(3.0)));
        assert_eq!(s.len(), 1);
    }

    #[test]
    fn add_appends_new_names_in_order_and_set_replaces_in_place() {
        let mut s = M::new();
        s.add(h("b"), V::Float(2.0)).unwrap();
        s.add(h("a"), V::Float(1.0)).unwrap();
        s.set(h("c"), V::Float(3.0));
        s.set(h("b"), V::Float(20.0));
        let members: Vec<_> = s
            .iter()
            .map(|(name, value)| (name, value.as_f32()))
            .collect();
        assert_eq!(members, [(h("b"), 20.0), (h("a"), 1.0), (h("c"), 3.0)]);
    }

    #[test]
    fn get_and_get_mut_find_members_by_name() {
        let mut s = M::xy(1.0, 2.0);
        assert_eq!(s.get(h("x")), Some(&V::Float(1.0)));
        assert_eq!(s.get(h("z")), None);
        *s.get_mut(h("y")).unwrap() = V::Float(5.0);
        assert_eq!(s.get(h("y")), Some(&V::Float(5.0)));
        assert!(s.get_mut(h("z")).is_none());
        assert_eq!(s.len(), 2);
    }

    #[test]
    fn get_or_insert_with_runs_the_closure_only_for_an_absent_name() {
        let mut s = M::xy(1.0, 2.0);
        let calls = Cell::new(0);
        let present = s.get_or_insert_with(h("x"), || {
            calls.set(calls.get() + 1);
            V::Float(9.0)
        });
        assert_eq!(*present, V::Float(1.0));
        assert_eq!(calls.get(), 0);
        let new = s.get_or_insert_with(h("z"), || {
            calls.set(calls.get() + 1);
            V::Float(3.0)
        });
        assert_eq!(*new, V::Float(3.0));
        *new = V::Float(4.0);
        assert_eq!(calls.get(), 1);
        assert_eq!(s.get(h("z")), Some(&V::Float(4.0)));
        assert_eq!(s.len(), 3);
    }

    #[test]
    fn a_repeated_name_replaces_the_earlier_value_instead_of_appending() {
        let s = M::from([("x", 1.0), ("x", 2.0)]);
        assert_eq!(s.len(), 1);
        assert_eq!(s.get(h("x")), Some(&V::Float(2.0)));
    }

    #[test]
    fn collect_keys_the_members_like_from() {
        let s: M = [("a", 1.0f32), ("B", 2.0)].into_iter().collect();
        assert_eq!(s, M::from([("a", 1.0), ("b", 2.0)]));
        assert_eq!(
            s.iter().map(|(name, _)| name).collect::<Vec<_>>(),
            [h("a"), h("b")]
        );
    }

    #[test]
    fn from_lowercases_the_name() {
        let s = M::from([("Speed", 2.0)]);
        assert_eq!(s.get(h("speed")), Some(&V::Float(2.0)));
        assert_eq!(s.get(h("Speed")), None);
    }

    #[test]
    fn key_lowers_ascii_and_ends_at_the_first_nul() {
        assert_eq!(M::key("Speed"), h("speed"));
        assert_eq!(M::key("SPEED"), h("speed"));
        assert_eq!(M::key("a\0b"), h("a"));
        assert_eq!(M::key(""), HashedStr::EMPTY);
        assert_eq!(M::key("\0abc"), HashedStr::EMPTY);
        // Only ASCII is lowered: a multi-byte character stays as it is.
        assert_eq!(M::key("ÄB"), h("Äb"));
        assert_ne!(M::key("ÄB"), M::key("äb"));
    }

    #[test]
    fn iter_lists_members_in_insertion_order() {
        let s = M::from([("c", 3.0), ("a", 1.0), ("b", 2.0)]);
        let iter = s.iter();
        assert_eq!(iter.len(), 3);
        let names: Vec<_> = iter.map(|(name, _)| name).collect();
        assert_eq!(names, [h("c"), h("a"), h("b")]);
        assert_eq!(s.len(), 3);
        assert!(!s.is_empty());
        assert!(M::new().is_empty());
        assert_eq!(M::new().iter().len(), 0);
    }

    #[test]
    fn builders_have_the_member_names_of_the_query_results() {
        let read = |s: &M, names: &[&str]| -> f32 {
            let path: Vec<HashedStr> = names.iter().map(|n| h(n)).collect();
            V::Struct(Arc::new(s.clone()))
                .member_path(&path)
                .unwrap_or_else(|| panic!("{names:?}"))
                .as_f32()
        };
        let xy = M::xy(1.0, 2.0);
        assert_eq!(
            xy.iter().map(|(n, _)| n).collect::<Vec<_>>(),
            [h("x"), h("y")]
        );
        assert_eq!((read(&xy, &["x"]), read(&xy, &["y"])), (1.0, 2.0));

        let xyz = M::xyz(1.0, 2.0, 3.0);
        assert_eq!(
            xyz.iter().map(|(n, _)| n).collect::<Vec<_>>(),
            [h("x"), h("y"), h("z")]
        );
        assert_eq!(
            (read(&xyz, &["x"]), read(&xyz, &["y"]), read(&xyz, &["z"])),
            (1.0, 2.0, 3.0)
        );

        let uv = M::uv(0.25, 0.75);
        assert_eq!(
            uv.iter().map(|(n, _)| n).collect::<Vec<_>>(),
            [h("u"), h("v")]
        );
        assert_eq!((read(&uv, &["u"]), read(&uv, &["v"])), (0.25, 0.75));

        let rgb = M::rgb(0.1, 0.2, 0.3);
        assert_eq!(
            rgb.iter().map(|(n, _)| n).collect::<Vec<_>>(),
            [h("r"), h("g"), h("b")]
        );
        assert_eq!(
            (read(&rgb, &["r"]), read(&rgb, &["g"]), read(&rgb, &["b"])),
            (0.1, 0.2, 0.3)
        );

        let rgba = M::rgba(0.1, 0.2, 0.3, 0.4);
        assert_eq!(
            rgba.iter().map(|(n, _)| n).collect::<Vec<_>>(),
            [h("r"), h("g"), h("b"), h("a")]
        );
        assert_eq!(read(&rgba, &["a"]), 0.4);

        let aabb = M::min_and_max([1.0, 2.0, 3.0], [4.0, 5.0, 6.0]);
        assert_eq!(
            aabb.iter().map(|(n, _)| n).collect::<Vec<_>>(),
            [h("min"), h("max")]
        );
        assert_eq!(
            (read(&aabb, &["min", "x"]), read(&aabb, &["min", "z"])),
            (1.0, 3.0)
        );
        assert_eq!(
            (read(&aabb, &["max", "x"]), read(&aabb, &["max", "z"])),
            (4.0, 6.0)
        );

        let trs = M::trs([1.0, 2.0, 3.0], [4.0, 5.0, 6.0], [7.0, 8.0, 9.0]);
        assert_eq!(
            trs.iter().map(|(n, _)| n).collect::<Vec<_>>(),
            [h("t"), h("r"), h("s")]
        );
        for (group, base) in [("t", 1.0), ("r", 4.0), ("s", 7.0)] {
            assert_eq!(
                (
                    read(&trs, &[group, "x"]),
                    read(&trs, &[group, "y"]),
                    read(&trs, &[group, "z"])
                ),
                (base, base + 1.0, base + 2.0),
                "{group}"
            );
        }
    }

    #[test]
    fn depth_of_the_builders() {
        assert_eq!(M::new().depth(), 1);
        assert_eq!(M::xy(1.0, 2.0).depth(), 1);
        assert_eq!(M::min_and_max([0.0; 3], [1.0; 3]).depth(), 2);
        assert_eq!(M::trs([0.0; 3], [0.0; 3], [0.0; 3]).depth(), 2);
    }

    #[test]
    fn struct_values_are_default_empty() {
        let s = M::default();
        assert!(s.is_empty());
        assert_eq!(s.len(), 0);
        assert_eq!(s, M::new());
    }

    #[test]
    fn struct_copies_share_until_written() {
        let original = V::structure(StructValue::xy(1.0, 2.0));
        let mut copy = original.clone();
        let (V::Struct(a), V::Struct(b)) = (&original, &copy) else {
            panic!("structs")
        };
        assert!(Arc::ptr_eq(a, b));
        copy.set_member_path(&[h("x")], V::Float(9.0));
        assert_eq!(original.member(h("x")), Some(&V::Float(1.0)));
        assert_eq!(copy.member(h("x")), Some(&V::Float(9.0)));
        assert_eq!(copy.member(h("y")), Some(&V::Float(2.0)));
    }

    #[test]
    fn a_uniquely_owned_struct_is_edited_in_place() {
        let mut v = V::structure(M::xy(1.0, 2.0));
        let before = Arc::as_ptr(arc_of(&v));
        v.set_member_path(&[h("x")], V::Float(9.0));
        v.set_member_path(&[h("z")], V::Float(3.0));
        assert_eq!(Arc::as_ptr(arc_of(&v)), before);
        assert_eq!(Arc::strong_count(arc_of(&v)), 1);
        assert_eq!(v.member(h("z")), Some(&V::Float(3.0)));
    }

    #[test]
    fn member_writes_create_intermediate_structs() {
        let mut v = V::ZERO;
        v.set_member_path(&[h("a"), h("b"), h("c")], V::Float(1.0));
        assert_eq!(
            v.member_path(&[h("a"), h("b"), h("c")]),
            Some(&V::Float(1.0))
        );
        v.set_member_path(&[h("a"), h("d")], V::Float(2.0));
        assert_eq!(
            v.member_path(&[h("a"), h("b"), h("c")]),
            Some(&V::Float(1.0))
        );
        assert_eq!(v.member_path(&[h("a"), h("d")]), Some(&V::Float(2.0)));
        // `v.x.x = 1; v.x.y = 2;`
        let mut x = V::ZERO;
        x.set_member_path(&[h("x")], V::Float(1.0));
        x.set_member_path(&[h("y")], V::Float(2.0));
        assert_eq!(x, V::structure(StructValue::xy(1.0, 2.0)));
        // A non-struct on the way is replaced by a struct.
        let mut y = V::structure(StructValue::from([("a", 5.0)]));
        y.set_member_path(&[h("a"), h("b")], V::Float(6.0));
        assert_eq!(y.member_path(&[h("a"), h("b")]), Some(&V::Float(6.0)));
        // An empty path replaces the value itself.
        y.set_member_path(&[], V::Float(7.0));
        assert_eq!(y, V::Float(7.0));
    }

    #[test]
    fn a_member_path_creates_a_chain_of_the_path_length() {
        let mut v = V::ZERO;
        v.set_member_path(&[h("a"), h("b"), h("c")], V::Float(1.0));
        assert_eq!(v.struct_depth(), 3);
        let first = v.as_struct().expect("struct");
        assert_eq!(first.len(), 1);
        // A sibling keeps the chain and does not deepen it.
        v.set_member_path(&[h("a"), h("d")], V::Float(2.0));
        assert_eq!(v.struct_depth(), 3);
        assert_eq!(
            v.member_path(&[h("a"), h("b"), h("c")]),
            Some(&V::Float(1.0))
        );
    }

    #[test]
    fn writing_through_a_non_struct_root_replaces_it_with_a_struct() {
        for root in [
            V::Float(5.0),
            V::string("moo"),
            V::Actor(1),
            V::Item(2),
            V::actor_array([1]),
            V::identity_matrix(),
        ] {
            let mut v = root;
            v.set_member_path(&[h("a")], V::Float(1.0));
            assert_eq!(v, V::structure(M::from([("a", 1.0)])));
        }
    }

    #[test]
    fn a_non_struct_member_on_the_path_is_replaced_and_its_siblings_kept() {
        let mut v = V::structure(M::from([("a", 5.0), ("keep", 7.0)]));
        v.set_member_path(&[h("a"), h("b"), h("c")], V::Float(6.0));
        assert_eq!(
            v.member_path(&[h("a"), h("b"), h("c")]),
            Some(&V::Float(6.0))
        );
        assert_eq!(v.member(h("keep")), Some(&V::Float(7.0)));
    }

    #[test]
    fn an_empty_path_replaces_the_whole_value() {
        let mut v = V::structure(M::xy(1.0, 2.0));
        v.set_member_path(&[], V::string("moo"));
        assert_eq!(v, V::string("moo"));
    }

    #[test]
    fn writing_a_struct_member_shares_the_stored_struct() {
        let inner = V::structure(M::xy(1.0, 2.0));
        let mut outer = V::ZERO;
        outer.set_member_path(&[h("p")], inner.clone());
        let stored = outer.member(h("p")).expect("member");
        assert!(Arc::ptr_eq(arc_of(stored), arc_of(&inner)));
    }

    #[test]
    fn a_write_through_a_shared_level_copies_only_that_level() {
        let mut original = V::ZERO;
        original.set_member_path(&[h("a"), h("b")], V::Float(1.0));
        original.set_member_path(&[h("keep"), h("c")], V::Float(2.0));
        let mut copy = original.clone();
        copy.set_member_path(&[h("a"), h("b")], V::Float(9.0));
        assert_eq!(
            original.member_path(&[h("a"), h("b")]),
            Some(&V::Float(1.0))
        );
        assert_eq!(copy.member_path(&[h("a"), h("b")]), Some(&V::Float(9.0)));
        // The level the write did not touch is still the same struct in both.
        assert!(Arc::ptr_eq(
            arc_of(original.member(h("keep")).unwrap()),
            arc_of(copy.member(h("keep")).unwrap())
        ));
    }

    #[test]
    fn struct_depth_counts_levels() {
        assert_eq!(V::Float(1.0).struct_depth(), 0);
        assert_eq!(V::actor_array([1]).struct_depth(), 0);
        assert_eq!(V::structure(M::new()).struct_depth(), 1);
        assert_eq!(V::structure(M::xy(1.0, 2.0)).struct_depth(), 1);
        assert_eq!(chain(3).struct_depth(), 3);
        // The deepest branch decides, however many shallow ones there are.
        let mixed = V::structure(M::from([
            ("shallow", V::Float(1.0)),
            ("one", V::structure(M::xy(1.0, 2.0))),
            ("deep", chain(4)),
        ]));
        assert_eq!(mixed.struct_depth(), 5);
        // Members that are not structs add no level.
        assert_eq!(
            V::structure(M::from([("list", V::actor_array([1, 2]))])).struct_depth(),
            1
        );
    }

    #[test]
    fn depth_is_cached_and_every_mutator_invalidates_it() {
        let mut m = M::from([("a", M::xy(1.0, 2.0))]);
        assert_eq!(m.cached_depth(), None);
        assert_eq!(m.depth(), 2);
        assert_eq!(m.cached_depth(), Some(2));
        // The cache of the nested struct was filled on the way.
        let child = m
            .get(h("a"))
            .and_then(V::as_struct)
            .expect("struct")
            .cached_depth();
        assert_eq!(child, Some(1));
        // A clone carries the cached depth.
        assert_eq!(m.clone().cached_depth(), Some(2));

        m.set(h("b"), V::Float(1.0));
        assert_eq!(m.cached_depth(), None);
        assert_eq!(m.depth(), 2);

        m.add(h("c"), V::Float(1.0)).unwrap();
        assert_eq!(m.cached_depth(), None);
        assert_eq!(m.depth(), 2);

        assert!(m.get_mut(h("b")).is_some());
        assert_eq!(m.cached_depth(), None);
        assert_eq!(m.depth(), 2);

        m.get_or_insert_with(h("d"), V::default);
        assert_eq!(m.cached_depth(), None);
        assert_eq!(m.depth(), 2);

        m.set(h("e"), V::Float(1.0));
        assert_eq!(m.cached_depth(), None);
        assert_eq!(m.depth(), 2);
    }

    #[test]
    fn a_refused_add_leaves_the_depth_cache_alone() {
        let mut m = M::xy(1.0, 2.0);
        assert_eq!(m.depth(), 1);
        assert!(m.add(h("x"), V::Float(5.0)).is_err());
        assert_eq!(m.cached_depth(), Some(1));
    }

    #[test]
    fn depth_recomputes_after_a_deeper_write_through_the_value() {
        let mut v = chain(2);
        assert_eq!(v.struct_depth(), 2);
        // Caches the depth at every level.
        assert_eq!(v.struct_depth(), 2);
        v.set_member_path(&[h("a"), h("a"), h("c"), h("d")], V::Float(1.0));
        assert_eq!(v.struct_depth(), 4);
    }

    #[test]
    fn depth_recomputes_after_a_write_to_a_shared_struct() {
        let mut v = chain(2);
        let copy = v.clone();
        assert_eq!(copy.struct_depth(), 2);
        v.set_member_path(&[h("a"), h("a"), h("b")], V::Float(1.0));
        assert_eq!(v.struct_depth(), 3);
        assert_eq!(copy.struct_depth(), 2);
    }

    #[test]
    fn depth_is_what_the_depth_cap_compares_after_a_write() {
        let cap = EvalLimits::DEFAULT_STRUCT_DEPTH as usize;
        let path: Vec<HashedStr> = (0..=cap).map(|i| h(&format!("m{i}"))).collect();
        let mut v = V::ZERO;
        v.set_member_path(&path[..cap], V::Float(1.0));
        assert_eq!(v.struct_depth(), cap as u32);
        v.set_member_path(&path, V::Float(1.0));
        assert_eq!(v.struct_depth(), cap as u32 + 1);
    }

    #[test]
    fn depth_of_a_shared_dag_visits_each_struct_once() {
        let started = Instant::now();
        let v = doubling(40);
        // 40 rounds make 41 structs with 2^40 paths to the innermost.
        assert_eq!(v.struct_depth(), 41);
        assert_eq!(v.struct_depth(), 41);
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "took {:?}",
            started.elapsed()
        );
    }

    #[test]
    fn depth_of_a_very_deep_struct_is_computed_without_recursion() {
        on_a_small_stack(|| {
            let v = chain(100_000);
            assert_eq!(v.struct_depth(), 100_000);
            // The cache answers the second time.
            assert_eq!(v.struct_depth(), 100_000);
        });
    }

    #[test]
    fn member_store_check_charges_a_level_each_and_a_member_per_stored_one() {
        let step = EvalLimits::STRUCT_COPY_STEPS;
        assert_eq!(step, 4);
        // A path through nothing: each level it creates costs the constant alone.
        assert_eq!(
            V::ZERO.member_store_check(&[h("a"), h("b")], None),
            MemberStoreCheck {
                cost: 2 * step,
                exceeded_width: None
            }
        );
        assert_eq!(
            V::ZERO.member_store_check(&[], None),
            MemberStoreCheck {
                cost: 0,
                exceeded_width: None
            }
        );
        // An existing struct costs its member count on top.
        let xyz = V::structure(M::xyz(1.0, 2.0, 3.0));
        assert_eq!(
            xyz.member_store_check(&[h("w")], None),
            MemberStoreCheck {
                cost: step + 3,
                exceeded_width: None
            }
        );
        assert_eq!(
            xyz.member_store_check(&[h("x")], None),
            MemberStoreCheck {
                cost: step + 3,
                exceeded_width: None
            }
        );
        // Through a member that is not a struct: the level after it is created.
        assert_eq!(
            xyz.member_store_check(&[h("x"), h("deeper")], None),
            MemberStoreCheck {
                cost: step + 3 + step,
                exceeded_width: None
            }
        );
        // Each struct on the path pays its own width.
        let nested = V::structure(M::from([("p", M::xyz(1.0, 2.0, 3.0))]));
        assert_eq!(
            nested.member_store_check(&[h("p"), h("x")], None),
            MemberStoreCheck {
                cost: step + 1 + step + 3,
                exceeded_width: None
            }
        );
    }

    #[test]
    fn member_store_check_costs_the_same_whether_or_not_the_struct_is_shared() {
        let v = V::structure(M::xyz(1.0, 2.0, 3.0));
        let alone = v.member_store_check(&[h("x")], None);
        let shared = v.clone();
        assert_eq!(v.member_store_check(&[h("x")], None), alone);
        assert_eq!(shared.member_store_check(&[h("x")], None), alone);
    }

    #[test]
    fn member_store_check_refuses_width_only_when_adding_a_member() {
        let xyz = V::structure(M::xyz(1.0, 2.0, 3.0));
        // Adding a fourth member to a struct that holds 3.
        assert_eq!(
            xyz.member_store_check(&[h("w")], Some(3)).exceeded_width,
            Some(3)
        );
        assert_eq!(
            xyz.member_store_check(&[h("w")], Some(4)).exceeded_width,
            None
        );
        // Storing into an existing member is never refused, however wide the struct.
        assert_eq!(
            xyz.member_store_check(&[h("x")], Some(3)).exceeded_width,
            None
        );
        assert_eq!(
            xyz.member_store_check(&[h("x")], Some(1)).exceeded_width,
            None
        );
        // No limit: never too wide.
        assert_eq!(xyz.member_store_check(&[h("w")], None).exceeded_width, None);
        // A struct the write creates has no members yet; a zero limit refuses even that.
        assert_eq!(
            V::ZERO
                .member_store_check(&[h("a")], Some(0))
                .exceeded_width,
            Some(0)
        );
        assert_eq!(
            V::ZERO
                .member_store_check(&[h("a")], Some(1))
                .exceeded_width,
            None
        );
    }

    #[test]
    fn member_store_check_reports_the_width_of_any_level_and_still_costs_all_of_them() {
        let step = EvalLimits::STRUCT_COPY_STEPS;
        let xyz = V::structure(M::xyz(1.0, 2.0, 3.0));
        // Too wide at the first level: the cost of the level below it is still counted.
        assert_eq!(
            xyz.member_store_check(&[h("w"), h("q")], Some(3)),
            MemberStoreCheck {
                cost: step + 3 + step,
                exceeded_width: Some(3)
            }
        );
        // Too wide at a deeper level only.
        let nested = V::structure(M::from([("p", M::xyz(1.0, 2.0, 3.0))]));
        assert_eq!(
            nested.member_store_check(&[h("p"), h("w")], Some(3)),
            MemberStoreCheck {
                cost: step + 1 + step + 3,
                exceeded_width: Some(3)
            }
        );
        assert_eq!(
            nested
                .member_store_check(&[h("p"), h("x")], Some(3))
                .exceeded_width,
            None
        );
    }
}
