//! `math.die_roll` and `math.die_roll_integer`, one call or one roll at a time.

use super::clamp_sample;
use crate::numeric::{PostOp, arith};
use crate::rng::{rand_core::Rng, sample};
use crate::stdlib::math::arch::{self, Sort, integer_reach, sorted_bounds};

/// The number of rolls of `math.die_roll(n, …)`: `n` truncated, at least 0.
///
/// There is no cap below 2^31. An `n` of 2^31 or more is 2^31 − 1 rolls on `Arm64` and none on
/// `X86_64`.
#[inline]
pub fn die_roll_count(n: f32) -> u32 {
    arith::to_int(n).max(0) as u32
}

/// A `math.die_roll` / `math.die_roll_integer` in progress, for a caller that must bound the work:
///
/// ```
/// use molangx::numeric::PostOp;
/// use molangx::rng::{Xorshift128, sample};
/// use molangx::stdlib::math::DieRoll;
///
/// let mut rng = Xorshift128::new();
/// let mut roll = DieRoll::new(3.0, 1.0, 6.0);
/// while roll.remaining() > 0 {
///     // … charge one step, stop early if the budget is spent …
///     roll.roll(sample(&mut rng));
/// }
/// let sum = roll.finish(PostOp::IDENTITY);
/// assert!((3.0..=18.0).contains(&sum));
/// ```
///
/// [`die_roll`] and [`die_roll_integer`] run this loop, so both ways give the same bits.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct DieRoll {
    lo: f32,
    hi: f32,
    /// `Some` for the integer operator.
    reach: Option<f32>,
    sum: f32,
    remaining: u32,
}

impl DieRoll {
    /// Starts `math.die_roll(n, a, b)`: [`die_roll_count`] rolls over the floored, sorted bounds.
    #[inline]
    pub fn new(n: f32, a: f32, b: f32) -> Self {
        let (lo, hi) = sorted_bounds(Sort::RunTime, a.floor(), b.floor());
        Self {
            lo,
            hi,
            reach: None,
            sum: 0.0,
            remaining: die_roll_count(n),
        }
    }

    /// Starts `math.die_roll_integer(n, a, b)`.
    #[inline]
    pub fn new_integer(n: f32, a: f32, b: f32) -> Self {
        let mut roll = Self::new(n, a, b);
        roll.reach = Some(integer_reach(roll.lo, roll.hi));
        roll
    }

    /// The number of rolls still to make.
    #[inline]
    pub fn remaining(&self) -> u32 {
        self.remaining
    }

    /// Makes one roll with a raw `sample`; does nothing once no roll remains.
    ///
    /// A continuous roll computes `sum + (r·hi + (1 − r)·lo)` on `X86_64` and
    /// `r·(hi − lo) + (sum + lo)` rounded once on `Arm64`; an integer roll adds the draw of
    /// [`random_integer`](super::random_integer).
    #[inline]
    pub fn roll(&mut self, sample: f32) {
        if self.remaining == 0 {
            return;
        }
        self.remaining -= 1;
        let r = clamp_sample(sample);
        let (lo, hi) = (self.lo, self.hi);
        self.sum = match self.reach {
            Some(reach) => arch::integer_roll(self.sum, lo, hi, reach, r),
            None => arch::roll(self.sum, lo, hi, r),
        };
    }

    /// The sum of the rolls made so far (0 for none), with the node's post-op.
    #[inline]
    pub fn finish(self, post: PostOp) -> f32 {
        post.apply(self.sum)
    }
}

#[inline]
fn roll_all<R: Rng + ?Sized>(
    mut roll: DieRoll,
    max_rolls: u32,
    rng: &mut R,
    post: PostOp,
) -> Option<f32> {
    if roll.remaining() > max_rolls {
        return None;
    }
    while roll.remaining() > 0 {
        roll.roll(sample(rng));
    }
    Some(roll.finish(post))
}

/// `math.die_roll(n, a, b)`: the sum of [`die_roll_count`] continuous draws over the floored,
/// sorted bounds, one sample per roll.
///
/// `None`, without drawing, when the count exceeds `max_rolls` (`u32::MAX` for no limit; a count
/// near 2^31 takes seconds).
#[inline]
pub fn die_roll<R: Rng + ?Sized>(
    n: f32,
    a: f32,
    b: f32,
    max_rolls: u32,
    rng: &mut R,
    post: PostOp,
) -> Option<f32> {
    roll_all(DieRoll::new(n, a, b), max_rolls, rng, post)
}

/// `math.die_roll_integer(n, a, b)`: the sum of [`die_roll_count`] integer draws over the floored,
/// sorted bounds, one sample per roll.
///
/// `None`, without drawing, when the count exceeds `max_rolls`.
#[inline]
pub fn die_roll_integer<R: Rng + ?Sized>(
    n: f32,
    a: f32,
    b: f32,
    max_rolls: u32,
    rng: &mut R,
    post: PostOp,
) -> Option<f32> {
    roll_all(DieRoll::new_integer(n, a, b), max_rolls, rng, post)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::numeric::test_support::*;
    use crate::rng::{FixedRng, sample};
    use crate::stdlib::math::arch::integer_draw;
    use proptest::prelude::*;

    const NO_LIMIT: u32 = u32::MAX;

    /// Plays back the words of `samples` in a loop and counts its draws.
    struct Scripted {
        samples: Vec<f32>,
        draws: usize,
    }

    fn scripted(samples: &[f32]) -> Scripted {
        Scripted {
            samples: samples.to_vec(),
            draws: 0,
        }
    }

    /// A `math.die_roll_integer` roll when `integer`, else a `math.die_roll` one.
    fn new_roll(integer: bool, n: f32, a: f32, b: f32) -> DieRoll {
        if integer {
            DieRoll::new_integer(n, a, b)
        } else {
            DieRoll::new(n, a, b)
        }
    }

    impl rand_core::TryRng for Scripted {
        type Error = rand_core::Infallible;

        fn try_next_u32(&mut self) -> Result<u32, Self::Error> {
            let sample = self.samples[self.draws % self.samples.len()];
            self.draws += 1;
            Ok(FixedRng::from_sample(sample).expect("a word's sample").0)
        }

        fn try_next_u64(&mut self) -> Result<u64, Self::Error> {
            unreachable!("a sample is one next_u32")
        }

        fn try_fill_bytes(&mut self, _: &mut [u8]) -> Result<(), Self::Error> {
            unreachable!("a sample is one next_u32")
        }
    }

    #[test]
    fn die_roll_count_truncates_and_floors_at_zero() {
        for (n, count) in [
            (0.0_f32, 0),
            (-0.0, 0),
            (0.9, 0),
            (1.0, 1),
            (1.9, 1),
            (3.9, 3),
            (100.0, 100),
            (-0.5, 0),
            (-1.0, 0),
            (-3.9, 0),
            (-INF, 0),
            (NAN, 0),
        ] {
            assert_eq!(die_roll_count(n), count, "die_roll_count({n})");
        }
        assert_eq!(die_roll_count(2_147_483_520.0), 2_147_483_520);
    }

    #[test]
    fn die_roll_count_beyond_the_int_range_differs_between_the_architectures() {
        for n in [2_147_483_648.0_f32, 1.0e10, f32::MAX, INF] {
            assert_eq!(die_roll_count(n), per_arch(0, i32::MAX as u32), "{n}");
        }
    }

    #[test]
    fn die_roll_new_floors_and_sorts_the_bounds() {
        let roll = DieRoll::new(3.9, 6.9, 1.9);
        assert_eq!((roll.lo, roll.hi), (1.0, 6.0));
        assert_eq!(roll.remaining(), 3);
        assert_eq!(roll.reach, None);
        assert_bits(roll.sum, 0.0);
        let roll = DieRoll::new(1.0, -1.5, -3.5);
        assert_eq!((roll.lo, roll.hi), (-4.0, -2.0));
    }

    #[test]
    fn die_roll_new_integer_precomputes_the_reach() {
        let roll = DieRoll::new_integer(2.0, 1.0, 6.0);
        assert_eq!(roll.reach, Some(integer_reach(1.0, 6.0)));
        assert_eq!((roll.lo, roll.hi, roll.remaining()), (1.0, 6.0, 2));
        assert_eq!(DieRoll::new(2.0, 1.0, 6.0).reach, None);
    }

    #[test]
    fn die_roll_with_no_rolls_is_zero_then_the_post_op() {
        for n in [0.0, 0.9, -1.0, NAN, -INF] {
            let roll = DieRoll::new(n, 1.0, 6.0);
            assert_eq!(roll.remaining(), 0);
            assert_bits(roll.finish(ID), 0.0);
            assert_bits(roll.finish(AFFINE), 1.0);
            assert_bits(
                DieRoll::new_integer(n, 1.0, 6.0).finish(PostOp::new(2.0, 1.5)),
                1.5,
            );
        }
    }

    #[test]
    fn die_roll_roll_counts_down_and_accumulates() {
        let mut roll = DieRoll::new(3.0, 1.0, 6.0);
        roll.roll(0.0);
        assert_eq!((roll.remaining(), roll.sum), (2, 1.0));
        roll.roll(1.0);
        assert_eq!((roll.remaining(), roll.sum), (1, 7.0));
        roll.roll(0.5);
        assert_eq!((roll.remaining(), roll.sum), (0, 10.5));
        assert_bits(roll.finish(ID), 10.5);
        assert_bits(roll.finish(AFFINE), 22.0);
    }

    #[test]
    fn die_roll_roll_clamps_the_sample() {
        let mut roll = DieRoll::new(4.0, 1.0, 6.0);
        roll.roll(9.0);
        roll.roll(-9.0);
        roll.roll(NAN);
        roll.roll(INF);
        assert_bits(roll.finish(ID), 6.0 + 1.0 + 1.0 + 6.0);
    }

    #[test]
    fn die_roll_roll_does_nothing_once_the_count_is_spent() {
        for integer in [false, true] {
            let mut roll = new_roll(integer, 1.0, 1.0, 6.0);
            roll.roll(0.5);
            let spent = roll;
            roll.roll(1.0);
            roll.roll(0.0);
            assert_eq!(roll, spent);
            assert_eq!(roll.remaining(), 0);
        }
    }

    #[test]
    fn a_continuous_roll_adds_the_architectures_interpolation() {
        let (lo, hi, r, sum) = (1.0_f32, 6.0_f32, third(), 2.5_f32);
        let mut roll = DieRoll::new(1.0, lo, hi);
        roll.sum = sum;
        roll.roll(r);
        assert_bits(
            roll.sum,
            per_arch(
                sum + (r * hi + (1.0 - r) * lo),
                r.mul_add(hi - lo, sum + lo),
            ),
        );
    }

    #[test]
    fn an_integer_roll_adds_the_clamped_integer_draw() {
        let mut roll = DieRoll::new_integer(3.0, 1.0, 6.0);
        let reach = roll.reach.unwrap();
        let mut expected = 0.0_f32;
        for sample in [0.0_f32, 0.5, 1.0] {
            roll.roll(sample);
            expected += integer_draw(1.0, 6.0, reach, sample);
            assert_bits(roll.sum, expected);
        }
        assert!(roll.sum >= 3.0 && roll.sum <= 18.0);
    }

    #[test]
    fn die_roll_is_copy_and_comparable() {
        let roll = DieRoll::new(3.0, 1.0, 6.0);
        let mut copy = roll;
        assert_eq!(roll, copy);
        copy.roll(0.5);
        assert_ne!(roll, copy);
        assert!(format!("{roll:?}").starts_with("DieRoll"));
    }

    #[test]
    fn die_roll_sums_one_sample_per_roll() {
        for (sample, real, integer) in [(0.0, 3.0, 3.0), (0.5, 10.5, 9.0), (1.0, 18.0, 18.0)] {
            let mut rng = scripted(&[sample]);
            assert_eq!(die_roll(3.0, 1.0, 6.0, NO_LIMIT, &mut rng, ID), Some(real));
            assert_eq!(rng.draws, 3);
            assert_eq!(
                die_roll_integer(3.0, 1.0, 6.0, NO_LIMIT, &mut rng, ID),
                Some(integer)
            );
            assert_eq!(rng.draws, 6);
        }
    }

    #[test]
    fn die_roll_with_a_constant_source_and_the_post_op() {
        let (mut zero, mut one, mut half) = (FixedRng::ZERO, FixedRng::ONE, FixedRng::HALF);
        assert_eq!(
            die_roll(2.0, 1.0, 6.0, NO_LIMIT, &mut zero, AFFINE),
            Some(5.0)
        );
        assert_eq!(
            die_roll(2.0, 1.0, 6.0, NO_LIMIT, &mut one, AFFINE),
            Some(25.0)
        );
        assert_eq!(
            die_roll_integer(2.0, 1.0, 6.0, NO_LIMIT, &mut one, AFFINE),
            Some(25.0)
        );
        assert_eq!(die_roll(1.0, 5.0, 5.0, NO_LIMIT, &mut half, ID), Some(5.0));
    }

    #[test]
    fn die_roll_truncates_the_count_and_floors_the_bounds() {
        let mut rng = scripted(&[1.0]);
        assert_eq!(die_roll(2.9, 1.9, 6.9, NO_LIMIT, &mut rng, ID), Some(12.0));
        assert_eq!(rng.draws, 2);
        let mut rng = scripted(&[0.0]);
        assert_eq!(die_roll(1.0, -1.5, 3.5, NO_LIMIT, &mut rng, ID), Some(-2.0));
    }

    #[test]
    fn die_roll_with_fewer_than_one_roll_draws_nothing() {
        let mut rng = scripted(&[0.5]);
        for n in [0.9, 0.0, -1.0, NAN, -INF] {
            assert_eq!(die_roll(n, 1.0, 6.0, NO_LIMIT, &mut rng, ID), Some(0.0));
            assert_eq!(
                die_roll_integer(n, 1.0, 6.0, 0, &mut rng, PostOp::new(2.0, 1.5)),
                Some(1.5)
            );
        }
        assert_eq!(rng.draws, 0);
    }

    #[test]
    fn die_roll_inverted_bounds_equal_sorted_bounds() {
        let samples = [0.1, 0.9, 0.4];
        let mut a = scripted(&samples);
        let mut b = scripted(&samples);
        let x = die_roll(3.0, 6.0, 1.0, NO_LIMIT, &mut a, ID).unwrap();
        let y = die_roll(3.0, 1.0, 6.0, NO_LIMIT, &mut b, ID).unwrap();
        assert_bits(x, y);
        let x = die_roll_integer(3.0, 6.0, 1.0, NO_LIMIT, &mut a, ID).unwrap();
        let y = die_roll_integer(3.0, 1.0, 6.0, NO_LIMIT, &mut b, ID).unwrap();
        assert_bits(x, y);
    }

    #[test]
    fn a_count_above_the_limit_returns_none_and_draws_nothing() {
        let mut rng = scripted(&[0.5]);
        assert_eq!(die_roll(5.0, 1.0, 6.0, 4, &mut rng, ID), None);
        assert_eq!(die_roll_integer(5.0, 1.0, 6.0, 4, &mut rng, ID), None);
        assert_eq!(
            die_roll_integer(2_000_000_000.0, 1.0, 6.0, 1_024, &mut rng, AFFINE),
            None
        );
        assert_eq!(die_roll(1.0, 1.0, 6.0, 0, &mut rng, ID), None);
        assert_eq!(rng.draws, 0);
        assert_eq!(die_roll(5.0, 1.0, 6.0, 5, &mut rng, ID), Some(17.5));
        assert_eq!(rng.draws, 5);
    }

    #[test]
    fn an_empty_roll_is_allowed_even_with_a_zero_limit() {
        let mut rng = scripted(&[0.5]);
        assert_eq!(die_roll(0.0, 1.0, 6.0, 0, &mut rng, AFFINE), Some(1.0));
        assert_eq!(rng.draws, 0);
    }

    #[test]
    fn a_huge_count_is_cut_off_by_the_limit_on_arm64_and_empty_on_x86_64() {
        let mut rng = scripted(&[0.5]);
        assert_eq!(
            die_roll(1.0e10, 1.0, 6.0, 1_000_000, &mut rng, ID),
            per_arch(Some(0.0), None)
        );
        assert_eq!(
            die_roll_integer(INF, 1.0, 6.0, 1_000_000, &mut rng, ID),
            per_arch(Some(0.0), None)
        );
        assert_eq!(rng.draws, 0);
    }

    #[test]
    fn die_roll_accepts_an_unsized_source() {
        let mut rng = FixedRng::ONE;
        let dynamic: &mut dyn rand_core::Rng = &mut rng;
        assert_eq!(die_roll(3.0, 1.0, 6.0, NO_LIMIT, dynamic, ID), Some(18.0));
    }

    #[test]
    fn roll_all_runs_a_prepared_roll_to_completion() {
        let mut rng = scripted(&[0.0, 1.0]);
        let roll = DieRoll::new(4.0, 1.0, 6.0);
        assert_eq!(roll_all(roll, 4, &mut rng, ID), Some(14.0));
        assert_eq!(rng.draws, 4);
        assert_eq!(roll_all(roll, 3, &mut rng, ID), None);
        assert_eq!(rng.draws, 4);
        let mut partial = roll;
        partial.roll(1.0);
        assert_eq!(
            roll_all(partial, 3, &mut rng, ID),
            Some(6.0 + 1.0 + 6.0 + 1.0)
        );
    }

    proptest! {
        #[test]
        fn stepping_a_die_roll_gives_the_same_bits_as_the_one_call_form(
            n in 0.0_f32..12.0,
            a in -50.0_f32..50.0,
            b in -50.0_f32..50.0,
            words in proptest::collection::vec(any::<u32>(), 1..13),
            integer in any::<bool>(),
            scale in -4.0_f32..4.0,
            offset in -4.0_f32..4.0,
        ) {
            let post = PostOp::new(scale, offset);
            let samples: Vec<f32> = words.into_iter().map(|word| sample(&mut FixedRng(word))).collect();
            let mut rng = scripted(&samples);
            let whole = if integer {
                die_roll_integer(n, a, b, NO_LIMIT, &mut rng, post)
            } else {
                die_roll(n, a, b, NO_LIMIT, &mut rng, post)
            };
            let mut roll = new_roll(integer, n, a, b);
            let count = roll.remaining();
            prop_assert_eq!(rng.draws, count as usize);
            let mut made = 0;
            while roll.remaining() > 0 {
                roll.roll(samples[made % samples.len()]);
                made += 1;
            }
            prop_assert_eq!(made, count as usize);
            prop_assert_eq!(Some(roll.finish(post).to_bits()), whole.map(f32::to_bits));
        }

        #[test]
        fn every_integer_roll_lies_between_the_floored_bounds(
            n in 1.0_f32..8.0,
            a in -20.0_f32..20.0,
            b in -20.0_f32..20.0,
            word in any::<u32>(),
        ) {
            let (lo, hi) = (a.min(b).floor(), a.max(b).floor());
            let count = n.trunc();
            let mut rng = FixedRng(word);
            let sum = die_roll_integer(n, a, b, NO_LIMIT, &mut rng, ID).unwrap();
            prop_assert!(sum >= lo * count && sum <= hi * count, "{} not in [{}, {}]", sum, lo * count, hi * count);
        }

        #[test]
        fn a_raw_sample_rolls_as_its_clamped_sample(
            n in 0.0_f32..4.0,
            a in -50.0_f32..50.0,
            b in -50.0_f32..50.0,
            raw in proptest::num::f32::ANY,
            integer in any::<bool>(),
        ) {
            let start = new_roll(integer, n, a, b);
            let (mut given, mut clamped) = (start, start);
            while given.remaining() > 0 {
                given.roll(raw);
                clamped.roll(clamp_sample(raw));
            }
            prop_assert_eq!(given.finish(ID).to_bits(), clamped.finish(ID).to_bits());
        }
    }

    #[test]
    fn die_rolls_draw_once_per_roll() {
        for (sample, real, integer) in [(0.0, 3.0, 3.0), (0.5, 10.5, 9.0), (1.0, 18.0, 18.0)] {
            let mut rng = scripted(&[sample]);
            assert_eq!(die_roll(3.0, 1.0, 6.0, NO_LIMIT, &mut rng, ID), Some(real));
            assert_eq!(rng.draws, 3);
            assert_eq!(
                die_roll(3.0, 6.0, 1.0, NO_LIMIT, &mut rng, ID),
                Some(real),
                "inverted bounds"
            );
            assert_eq!(rng.draws, 6);
            assert_eq!(
                die_roll_integer(3.0, 1.0, 6.0, NO_LIMIT, &mut rng, ID),
                Some(integer)
            );
            assert_eq!(rng.draws, 9);
        }
        let mut rng = scripted(&[1.0]);
        assert_eq!(die_roll(2.9, 1.9, 6.9, NO_LIMIT, &mut rng, ID), Some(12.0));
        assert_eq!(rng.draws, 2);
        for n in [0.9, 0.0, -1.0, NAN] {
            assert_eq!(die_roll(n, 1.0, 6.0, NO_LIMIT, &mut rng, ID), Some(0.0));
            assert_eq!(
                die_roll_integer(n, 1.0, 6.0, 0, &mut rng, PostOp::new(2.0, 1.5)),
                Some(1.5)
            );
        }
        assert_eq!(rng.draws, 2);
        let mut rng = scripted(&[0.0, 1.0]);
        assert_eq!(die_roll(2.0, 1.0, 6.0, NO_LIMIT, &mut rng, ID), Some(7.0));
        assert_eq!(
            die_roll_integer(2.0, 1.0, 6.0, NO_LIMIT, &mut rng, PostOp::new(2.0, 1.0)),
            Some(15.0)
        );
        assert_eq!(die_roll_count(3.9), 3);
        assert_eq!(die_roll_count(-3.9), 0);
        assert_eq!(die_roll_count(NAN), 0);
        assert_eq!(die_roll_count(1.0e10), per_arch(0, i32::MAX as u32));
    }

    #[test]
    fn die_rolls_are_bounded_by_the_caller() {
        let mut rng = scripted(&[0.5]);
        assert_eq!(die_roll(5.0, 1.0, 6.0, 4, &mut rng, ID), None);
        assert_eq!(
            die_roll_integer(2_000_000_000.0, 1.0, 6.0, 1_024, &mut rng, ID),
            None
        );
        assert_eq!(rng.draws, 0);
        assert_eq!(die_roll(5.0, 1.0, 6.0, 5, &mut rng, ID), Some(17.5));
        assert_eq!(rng.draws, 5);

        let samples = [0.1, 0.9, 0.333_333_34, 0.5, 1.0, 0.0, 0.777];
        let post = PostOp::new(1.5, -0.25);
        for integer in [false, true] {
            let mut rng = scripted(&samples);
            let whole = if integer {
                die_roll_integer(7.0, -2.5, 9.25, NO_LIMIT, &mut rng, post)
            } else {
                die_roll(7.0, -2.5, 9.25, NO_LIMIT, &mut rng, post)
            };
            let mut roll = new_roll(integer, 7.0, -2.5, 9.25);
            assert_eq!(roll.remaining(), 7);
            for (made, sample) in samples.iter().enumerate() {
                roll.roll(*sample);
                assert_eq!(roll.remaining() as usize, 6 - made);
            }
            let before = roll;
            roll.roll(0.5);
            assert_eq!(roll, before);
            assert_eq!(Some(roll.finish(post).to_bits()), whole.map(f32::to_bits));
        }
        let mut roll = DieRoll::new(100.0, 1.0, 6.0);
        roll.roll(0.0);
        roll.roll(1.0);
        assert_eq!(roll.remaining(), 98);
        assert_eq!(roll.finish(ID), 7.0);
        assert_eq!(DieRoll::new(0.5, 1.0, 6.0).remaining(), 0);
        assert_eq!(
            DieRoll::new_integer(NAN, 1.0, 6.0).finish(PostOp::new(2.0, 1.5)),
            1.5
        );
    }
}
