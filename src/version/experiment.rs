//! [`Experiment`] and [`ExperimentMask`]: the experiment toggles content may be compiled with.

use core::fmt;

use crate::bitmask::set_operators;

/// One experiment toggle, numbered `0..64` by the host.
///
/// No standard query and no operator is experiment-gated; a host's
/// [`QueryDecl`](crate::catalog::QueryDecl) can be.
#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Experiment(u8);

impl Experiment {
    /// The experiment with number `id`, if it fits the mask (`id < 64`).
    pub const fn new(id: u8) -> Option<Self> {
        if id < 64 { Some(Self(id)) } else { None }
    }

    /// The experiment's number.
    pub const fn id(self) -> u8 {
        self.0
    }
}

/// The experiments a compilation has enabled.
#[derive(Copy, Clone, Default, PartialEq, Eq, Hash)]
pub struct ExperimentMask(u64);

impl ExperimentMask {
    /// No experiment enabled.
    pub const fn empty() -> Self {
        Self(0)
    }

    /// Every experiment enabled.
    pub const fn all() -> Self {
        Self(u64::MAX)
    }

    /// The mask with `experiment` enabled as well.
    #[must_use]
    pub const fn with(self, experiment: Experiment) -> Self {
        Self(self.0 | 1 << experiment.0)
    }

    /// The mask with `experiment` disabled.
    #[must_use]
    pub const fn without(self, experiment: Experiment) -> Self {
        Self(self.0 & !(1 << experiment.0))
    }

    /// Whether every experiment of `other` (an [`Experiment`] or an `ExperimentMask`) is enabled.
    pub fn contains(self, other: impl Into<Self>) -> bool {
        let other = other.into();
        self.0 & other.0 == other.0
    }

    /// Both masks' experiments.
    #[must_use]
    pub const fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }

    /// The experiments in both masks.
    #[must_use]
    pub const fn intersection(self, other: Self) -> Self {
        Self(self.0 & other.0)
    }

    /// The experiments of `self` not in `other`.
    #[must_use]
    pub const fn difference(self, other: Self) -> Self {
        Self(self.0 & !other.0)
    }

    /// Enables the experiments of `other`.
    pub const fn insert(&mut self, other: Self) {
        *self = self.union(other);
    }

    /// Disables the experiments of `other`.
    pub const fn remove(&mut self, other: Self) {
        *self = self.difference(other);
    }

    /// Whether the masks share an experiment; `other` is an [`Experiment`] or an
    /// `ExperimentMask`.
    pub fn intersects(self, other: impl Into<Self>) -> bool {
        self.0 & other.into().0 != 0
    }

    /// Whether no experiment is enabled.
    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }

    /// The raw bits (bit `n` = experiment `n`).
    pub const fn bits(self) -> u64 {
        self.0
    }
}

impl From<Experiment> for ExperimentMask {
    fn from(experiment: Experiment) -> Self {
        Self::empty().with(experiment)
    }
}

impl FromIterator<Experiment> for ExperimentMask {
    fn from_iter<I: IntoIterator<Item = Experiment>>(experiments: I) -> Self {
        experiments.into_iter().fold(Self::empty(), Self::with)
    }
}

set_operators!(ExperimentMask);

impl fmt::Debug for ExperimentMask {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_set()
            .entries((0..64u8).filter(|&i| self.0 & (1 << i) != 0))
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn experiment_ids_are_below_sixty_four() {
        for id in 0..64 {
            assert_eq!(Experiment::new(id).map(Experiment::id), Some(id));
        }
        for id in [64, 65, 100, 255] {
            assert_eq!(Experiment::new(id), None, "{id}");
        }
    }

    #[test]
    fn experiments_order_by_number() {
        assert!(Experiment::new(1).unwrap() < Experiment::new(2).unwrap());
        assert!(Experiment::new(63).unwrap() > Experiment::new(0).unwrap());
    }

    #[test]
    fn experiment_mask_with_sets_exactly_one_bit() {
        assert_eq!(ExperimentMask::empty().bits(), 0);
        assert_eq!(ExperimentMask::default(), ExperimentMask::empty());
        for id in 0..64u8 {
            let mask = ExperimentMask::empty().with(Experiment::new(id).unwrap());
            assert_eq!(mask.bits(), 1u64 << id);
            for other in 0..64u8 {
                assert_eq!(
                    mask.contains(Experiment::new(other).unwrap()),
                    other == id,
                    "{id} {other}"
                );
            }
        }
    }

    #[test]
    fn experiment_mask_with_is_idempotent_and_order_free() {
        let (a, b) = (Experiment::new(3).unwrap(), Experiment::new(40).unwrap());
        assert_eq!(
            ExperimentMask::empty().with(a).with(a),
            ExperimentMask::empty().with(a)
        );
        assert_eq!(
            ExperimentMask::empty().with(a).with(b),
            ExperimentMask::empty().with(b).with(a)
        );
        assert_eq!(
            ExperimentMask::empty().with(a).with(b).bits(),
            (1 << 3) | (1 << 40)
        );
    }

    #[test]
    fn experiment_mask_all_enables_each_experiment() {
        let each: ExperimentMask = (0..64).filter_map(Experiment::new).collect();
        assert_eq!(ExperimentMask::all(), each);
    }

    #[test]
    fn experiment_mask_intersection_difference_insert_and_remove() {
        let (a, b, c) = (
            Experiment::new(1).unwrap(),
            Experiment::new(2).unwrap(),
            Experiment::new(63).unwrap(),
        );
        let ab: ExperimentMask = [a, b].into_iter().collect();
        let bc: ExperimentMask = [b, c].into_iter().collect();
        assert_eq!((ab & bc, ab.intersection(bc)), (b.into(), b.into()));
        assert_eq!((ab - bc, ab.difference(bc)), (a.into(), a.into()));
        assert_eq!(ab.without(a), b.into());
        assert_eq!(ab.without(c), ab);
        let mut mask = ab;
        mask.insert(bc);
        assert_eq!(mask, ab | bc);
        mask.remove(ab);
        assert_eq!(mask, c.into());
        mask &= ab;
        assert!(mask.is_empty());
        let mut mask = ExperimentMask::all();
        mask -= ab;
        assert!(!mask.contains(a) && mask.contains(c));
    }

    #[test]
    fn contains_needs_every_experiment() {
        let (a, b, c) = (
            Experiment::new(1).unwrap(),
            Experiment::new(2).unwrap(),
            Experiment::new(3).unwrap(),
        );
        let mask: ExperimentMask = [a, b].into_iter().collect();
        assert_eq!(mask, ExperimentMask::empty().with(a).with(b));
        assert_eq!(mask, ExperimentMask::from(a) | ExperimentMask::from(b));
        assert!(mask.contains(ExperimentMask::empty()));
        assert!(ExperimentMask::empty().contains(ExperimentMask::empty()));
        assert!(mask.contains(a));
        assert!(mask.contains(mask));
        assert!(mask.contains([b, a, b].into_iter().collect::<ExperimentMask>()));
        assert!(!mask.contains([a, b, c].into_iter().collect::<ExperimentMask>()));
        assert!(!mask.contains(c));
        assert!(mask.intersects(a) && !mask.intersects(c));
        assert!(!ExperimentMask::empty().contains(Experiment::new(5).unwrap()));
        let last = Experiment::new(63).unwrap();
        assert!(!ExperimentMask::empty().contains(last));
        assert!(ExperimentMask::from(last).contains(last));
        assert!(!ExperimentMask::empty().contains(mask));
    }

    #[test]
    fn experiment_mask_debug_lists_ascending_numbers() {
        assert_eq!(format!("{:?}", ExperimentMask::empty()), "{}");
        let mask = ExperimentMask::empty()
            .with(Experiment::new(63).unwrap())
            .with(Experiment::new(5).unwrap());
        assert_eq!(format!("{mask:?}"), "{5, 63}");
        assert_eq!(
            format!(
                "{:?}",
                ExperimentMask::empty().with(Experiment::new(0).unwrap())
            ),
            "{0}"
        );
    }
}
