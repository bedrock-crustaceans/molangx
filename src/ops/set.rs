//! [`OpSet`]: the operations a compilation allows.

use core::fmt;

use super::op::{self, ExpressionOp};
use crate::bitmask::set_operators;

/// The operations a compilation allows: a bitset over [`ExpressionOp`] indices.
///
/// A disallowed op is reported as `Expression uses operation {} which is not allowed in this
/// context`, filled with [`ExpressionOp::friendly_name`].
#[derive(Copy, Clone, PartialEq, Eq, Hash)]
pub struct OpSet(u128);

impl OpSet {
    /// No op allowed.
    pub const fn empty() -> Self {
        Self(0)
    }

    /// All 111 ops: the default configuration.
    pub const fn all() -> Self {
        Self((1 << op::COUNT) - 1)
    }

    /// The set from its two words (bit `n` of word `n / 64` = op `n`); bits above 110 are dropped.
    pub const fn from_words([low, high]: [u64; 2]) -> Self {
        Self((((high as u128) << 64) | low as u128) & Self::all().0)
    }

    /// The two words.
    pub const fn words(self) -> [u64; 2] {
        [self.0 as u64, (self.0 >> 64) as u64]
    }

    /// Whether every op of `other` (an [`ExpressionOp`] or an `OpSet`) is allowed.
    pub fn contains(self, other: impl Into<Self>) -> bool {
        let other = other.into();
        self.0 & other.0 == other.0
    }

    /// Whether the sets share an op; `other` is an [`ExpressionOp`] or an `OpSet`.
    pub fn intersects(self, other: impl Into<Self>) -> bool {
        self.0 & other.into().0 != 0
    }

    /// Both sets' ops.
    #[must_use]
    pub const fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }

    /// The ops in both sets.
    #[must_use]
    pub const fn intersection(self, other: Self) -> Self {
        Self(self.0 & other.0)
    }

    /// The ops of `self` not in `other`.
    #[must_use]
    pub const fn difference(self, other: Self) -> Self {
        Self(self.0 & !other.0)
    }

    /// Allows the ops of `other` as well.
    pub const fn insert(&mut self, other: Self) {
        *self = self.union(other);
    }

    /// Disallows the ops of `other`.
    pub const fn remove(&mut self, other: Self) {
        *self = self.difference(other);
    }

    /// The set with `op` allowed as well.
    #[must_use]
    pub const fn with(self, op: ExpressionOp) -> Self {
        Self(self.0 | (1 << op as u8))
    }

    /// The set without `op`.
    #[must_use]
    pub const fn without(self, op: ExpressionOp) -> Self {
        Self(self.0 & !(1 << op as u8))
    }

    /// The set without `=`, for entity property defaults.
    #[must_use]
    pub const fn without_assignments(self) -> Self {
        self.without(ExpressionOp::Assignment)
    }

    /// The set without `=`, `math.random`, `math.random_integer` and volatile host math functions,
    /// for block permutation conditions. `math.die_roll` and `math.die_roll_integer` stay allowed.
    #[must_use]
    pub const fn without_assignments_or_random(self) -> Self {
        self.without_assignments()
            .without(ExpressionOp::Random)
            .without(ExpressionOp::RandomInt)
            .without(ExpressionOp::HostMathVolatile)
    }

    /// Number of allowed ops.
    pub const fn len(self) -> usize {
        self.0.count_ones() as usize
    }

    /// Whether no op is allowed.
    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }

    /// The allowed ops in index order.
    pub fn iter(self) -> impl Iterator<Item = ExpressionOp> {
        op::ALL.into_iter().filter(move |&op| self.contains(op))
    }
}

impl Default for OpSet {
    /// [`OpSet::all`].
    fn default() -> Self {
        Self::all()
    }
}

impl From<ExpressionOp> for OpSet {
    fn from(op: ExpressionOp) -> Self {
        Self::empty().with(op)
    }
}

set_operators!(OpSet);

impl fmt::Debug for OpSet {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_set().entries(self.iter()).finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ops::OpFlags;
    use proptest::prelude::*;

    #[test]
    fn op_set() {
        assert_eq!(OpSet::all().len(), 111);
        assert_eq!(OpSet::default(), OpSet::all());
        assert_eq!(OpSet::all().words(), [u64::MAX, (1 << 47) - 1]);
        assert!(
            ExpressionOp::all()
                .iter()
                .all(|&op| OpSet::all().contains(op))
        );
        assert!(OpSet::empty().is_empty() && OpSet::empty().iter().next().is_none());
        assert_eq!(OpSet::from_words([u64::MAX; 2]), OpSet::all());
        assert_eq!(OpSet::all().iter().count(), 111);

        let property_default = OpSet::all().without_assignments();
        let removed: Vec<u8> = OpSet::all()
            .iter()
            .filter(|&op| !property_default.contains(op))
            .map(ExpressionOp::ordinal)
            .collect();
        assert_eq!(removed, [71]);
        let block_condition = OpSet::all().without_assignments_or_random();
        let removed: Vec<u8> = OpSet::all()
            .iter()
            .filter(|&op| !block_condition.contains(op))
            .map(ExpressionOp::ordinal)
            .collect();
        assert_eq!(removed, [33, 34, 71, 110]);
        assert!(
            block_condition.contains(ExpressionOp::DieRoll)
                && block_condition.contains(ExpressionOp::DieRollInt)
        );
        assert_eq!(block_condition.len(), 107);
        let flagged: Vec<u8> = ExpressionOp::all()
            .iter()
            .filter(|op| op.meta().flags.contains(OpFlags::SIDE_EFFECT))
            .map(|op| op.ordinal())
            .collect();
        assert_eq!(flagged, [33, 34, 71, 110]);
        let only_add = OpSet::empty().with(ExpressionOp::Add);
        assert!(only_add.contains(ExpressionOp::Add) && only_add.len() == 1);
        assert!(only_add.without(ExpressionOp::Add).is_empty());
        assert_eq!(
            OpSet::all().without(ExpressionOp::EaseInOutElastic).len(),
            110
        );
    }

    #[test]
    fn op_set_intersection_difference_insert_and_remove() {
        let (add, mul, last) = (
            ExpressionOp::Add,
            ExpressionOp::Mul,
            ExpressionOp::all()[ExpressionOp::COUNT - 1],
        );
        let a = OpSet::from(add).with(last);
        let b = OpSet::from(mul).with(last);
        assert_eq!(
            (a & b, a.intersection(b)),
            (OpSet::from(last), OpSet::from(last))
        );
        assert_eq!(
            (a - b, a.difference(b)),
            (OpSet::from(add), OpSet::from(add))
        );
        assert_eq!(OpSet::all() - a, OpSet::all().without(add).without(last));
        assert!(a.intersects(add) && !a.intersects(mul) && a.intersects(b));
        let mut set = a;
        set.insert(b);
        assert_eq!(set, a | b);
        set.remove(a);
        assert_eq!(set, OpSet::from(mul));
        set &= a;
        assert!(set.is_empty());
        let mut set = OpSet::all();
        set -= OpSet::all();
        assert!(set.is_empty());
    }

    #[test]
    fn op_set_all_and_empty() {
        assert_eq!(OpSet::all().words(), [u64::MAX, (1 << 47) - 1]);
        assert_eq!(OpSet::empty().words(), [0, 0]);
        assert_eq!(OpSet::all().len(), 111);
        assert_eq!(OpSet::empty().len(), 0);
        assert!(OpSet::empty().is_empty());
        assert!(!OpSet::all().is_empty());
        assert!(!OpSet::empty().with(ExpressionOp::Add).is_empty());
        assert_eq!(OpSet::default(), OpSet::all());
    }

    #[test]
    fn a_set_holding_only_ops_of_the_second_word_is_not_empty() {
        for &op in ExpressionOp::all() {
            let single = OpSet::empty().with(op);
            assert!(!single.is_empty(), "{op:?}");
            assert_eq!(single.len(), 1, "{op:?}");
            assert!(single.without(op).is_empty(), "{op:?}");
        }
        let high = OpSet::from_words([0, u64::MAX]);
        assert!(!high.is_empty());
        assert_eq!(high.len(), 47);
        assert_eq!(OpSet::from_words([u64::MAX, 0]).len(), 64);
    }

    #[test]
    fn op_set_word_boundaries() {
        use ExpressionOp as Op;
        for (op, words) in [
            (Op::Array, [1u64 << 63, 0]),
            (Op::Geometry, [0, 1]),
            (Op::EaseInOutElastic, [0, 1 << 44]),
            (Op::LeftBrace, [1, 0]),
        ] {
            let set = OpSet::empty().with(op);
            assert_eq!(set.words(), words, "{op:?}");
            assert!(set.contains(op));
            assert_eq!(set.len(), 1);
            assert_eq!(set.without(op), OpSet::empty());
            assert_eq!(set.with(op), set, "with twice");
            let all_but = OpSet::all().without(op);
            assert!(!all_but.contains(op));
            assert_eq!(all_but.len(), 110);
        }
        assert_eq!(
            OpSet::empty().without(Op::Add),
            OpSet::empty(),
            "removing an absent op"
        );
    }

    #[test]
    fn from_words_drops_bits_above_the_last_op() {
        assert_eq!(OpSet::from_words([u64::MAX; 2]), OpSet::all());
        assert_eq!(OpSet::from_words([0, 1 << 47]), OpSet::empty());
        assert!(OpSet::from_words([0, 1 << 46]).contains(ExpressionOp::HostMathVolatile));
        assert_eq!(OpSet::from_words([0, 1 << 63]), OpSet::empty());
        assert_eq!(OpSet::from_words([0, 1 << 44]).len(), 1);
        assert!(OpSet::from_words([0, 1 << 44]).contains(ExpressionOp::EaseInOutElastic));
        assert_eq!(OpSet::from_words([5, 6]).words(), [5, 6]);
    }

    #[test]
    fn op_set_iter_is_in_ordinal_order() {
        let ordinals: Vec<u8> = OpSet::all().iter().map(ExpressionOp::ordinal).collect();
        assert_eq!(ordinals, (0..111).collect::<Vec<u8>>());
        assert_eq!(OpSet::empty().iter().next(), None);
        let some = OpSet::empty()
            .with(ExpressionOp::Geometry)
            .with(ExpressionOp::Add);
        assert_eq!(
            some.iter().collect::<Vec<_>>(),
            [ExpressionOp::Add, ExpressionOp::Geometry]
        );
    }

    #[test]
    fn the_side_effect_builders_clear_the_documented_ops() {
        use ExpressionOp as Op;
        let without = OpSet::all().without_assignments();
        assert!(!without.contains(Op::Assignment));
        assert!(without.contains(Op::Random) && without.contains(Op::RandomInt));
        assert_eq!(without.len(), 110);
        let strict = OpSet::all().without_assignments_or_random();
        for op in [
            Op::Assignment,
            Op::Random,
            Op::RandomInt,
            Op::HostMathVolatile,
        ] {
            assert!(!strict.contains(op), "{op:?}");
        }
        assert!(
            strict.contains(Op::DieRoll)
                && strict.contains(Op::DieRollInt)
                && strict.contains(Op::HostMath)
        );
        assert_eq!(strict.len(), 107);
        assert_eq!(
            OpSet::empty().without_assignments_or_random(),
            OpSet::empty()
        );
        assert_eq!(strict.without_assignments_or_random(), strict);
    }

    #[test]
    fn op_set_debug_lists_the_ops() {
        assert_eq!(format!("{:?}", OpSet::empty()), "{}");
        let set = OpSet::empty()
            .with(ExpressionOp::Add)
            .with(ExpressionOp::Abs);
        assert_eq!(format!("{set:?}"), "{Abs, Add}");
    }

    fn any_op_set() -> impl Strategy<Value = OpSet> {
        (any::<u64>(), any::<u64>()).prop_map(|(a, b)| OpSet::from_words([a, b]))
    }

    proptest! {
        #![proptest_config(ProptestConfig { failure_persistence: None, ..ProptestConfig::default() })]

        #[test]
        fn len_counts_the_iterated_ops(set in any_op_set()) {
            prop_assert_eq!(set.iter().count(), set.len());
            prop_assert_eq!(set.len(), (set.words()[0].count_ones() + set.words()[1].count_ones()) as usize);
            prop_assert_eq!(set.is_empty(), set.iter().next().is_none());
            prop_assert_eq!(OpSet::from_words(set.words()), set);
            for op in set.iter() {
                prop_assert!(set.contains(op));
            }
        }

        #[test]
        fn with_then_without_restores_a_set_that_lacked_the_op(set in any_op_set(), index in 0usize..111) {
            let op = ExpressionOp::all()[index];
            let without = set.without(op);
            prop_assert!(!without.contains(op));
            prop_assert!(without.with(op).contains(op));
            prop_assert_eq!(without.with(op).without(op), without);
            prop_assert_eq!(set.with(op).len(), without.len() + 1);
        }

        #[test]
        fn the_side_effect_builders_touch_only_the_documented_ops(set in any_op_set(), with_random in any::<bool>()) {
            let clear = |set: OpSet| if with_random { set.without_assignments_or_random() } else { set.without_assignments() };
            let cleared = clear(set);
            let documented: &[ExpressionOp] = if with_random {
                &[ExpressionOp::Assignment, ExpressionOp::Random, ExpressionOp::RandomInt, ExpressionOp::HostMathVolatile]
            } else {
                &[ExpressionOp::Assignment]
            };
            for &op in ExpressionOp::all() {
                if documented.contains(&op) {
                    prop_assert!(!cleared.contains(op));
                } else {
                    prop_assert_eq!(cleared.contains(op), set.contains(op));
                }
            }
            prop_assert_eq!(clear(cleared), cleared);
        }
    }
}
