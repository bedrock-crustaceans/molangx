//! [`ProcessRng`]: one random sequence shared by every evaluation.

use std::sync::{Mutex, MutexGuard, PoisonError};

use crate::rng::{
    Xorshift128,
    rand_core::{Infallible, TryRng},
};

static PROCESS_XORSHIFT: Mutex<Xorshift128> = Mutex::new(Xorshift128::new());

/// A handle to one xorshift128 for the whole process, from the standard seeds and never reseeded.
///
/// Pass `&mut ProcessRng` as [`EvalCx::rng`](super::EvalCx::rng). Concurrent evaluations
/// interleave their draws; each `next_u32`, `next_u64` or `fill_bytes` takes the lock once, so the
/// state is never torn.
///
/// ```
/// use molangx::rng::{Xorshift128, sample};
/// use molangx::vm::ProcessRng;
///
/// ProcessRng::reset();
/// let mut reference = Xorshift128::new();
/// assert_eq!(sample(&mut ProcessRng), sample(&mut reference));
/// assert_eq!(sample(&mut ProcessRng), sample(&mut reference));
/// ```
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub struct ProcessRng;

impl ProcessRng {
    /// Puts the process-wide generator back to the standard seeds.
    pub fn reset() {
        *locked() = Xorshift128::new();
    }
}

fn locked() -> MutexGuard<'static, Xorshift128> {
    PROCESS_XORSHIFT
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
}

impl TryRng for ProcessRng {
    type Error = Infallible;

    fn try_next_u32(&mut self) -> Result<u32, Infallible> {
        locked().try_next_u32()
    }

    fn try_next_u64(&mut self) -> Result<u64, Infallible> {
        locked().try_next_u64()
    }

    fn try_fill_bytes(&mut self, dst: &mut [u8]) -> Result<(), Infallible> {
        locked().try_fill_bytes(dst)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rng::{rand_core::Rng, sample};

    /// The generator is process-wide, so these tests must not run concurrently.
    static LOCK: Mutex<()> = Mutex::new(());

    fn exclusive() -> MutexGuard<'static, ()> {
        let guard = LOCK.lock().unwrap_or_else(PoisonError::into_inner);
        ProcessRng::reset();
        guard
    }

    fn reference(n: usize) -> Vec<f32> {
        let mut rng = Xorshift128::new();
        (0..n).map(|_| sample(&mut rng)).collect()
    }

    #[test]
    fn the_first_samples_are_those_of_a_freshly_seeded_generator() {
        let _guard = exclusive();
        let mut rng = ProcessRng;
        let got: Vec<u32> = (0..5).map(|_| sample(&mut rng).to_bits()).collect();
        let want: Vec<u32> = reference(5).into_iter().map(f32::to_bits).collect();
        assert_eq!(got, want);
    }

    #[test]
    fn reset_restarts_the_sequence() {
        let _guard = exclusive();
        let first = sample(&mut ProcessRng);
        for _ in 0..3 {
            sample(&mut ProcessRng);
        }
        assert_ne!(sample(&mut ProcessRng).to_bits(), first.to_bits());
        ProcessRng::reset();
        assert_eq!(sample(&mut ProcessRng).to_bits(), first.to_bits());
        assert_eq!(first.to_bits(), reference(1)[0].to_bits());
    }

    #[test]
    fn every_value_shares_the_one_generator() {
        let _guard = exclusive();
        let (mut a, mut b) = (ProcessRng, ProcessRng);
        let got = [
            sample(&mut a),
            sample(&mut b),
            sample(&mut a),
            sample(&mut b),
            sample(&mut a),
        ];
        let want = reference(5);
        for (got, want) in got.iter().zip(&want) {
            assert_eq!(got.to_bits(), want.to_bits());
        }
    }

    #[test]
    fn samples_stay_in_the_unit_interval() {
        let _guard = exclusive();
        for _ in 0..10_000 {
            let sample = sample(&mut ProcessRng);
            assert!((0.0..=1.0).contains(&sample), "{sample}");
        }
    }

    #[test]
    fn concurrent_draws_hand_out_each_sample_exactly_once() {
        let _guard = exclusive();
        let threads: Vec<_> = (0..4)
            .map(|_| {
                std::thread::spawn(|| {
                    (0..1000)
                        .map(|_| sample(&mut ProcessRng).to_bits())
                        .collect::<Vec<u32>>()
                })
            })
            .collect();
        let mut drawn: Vec<u32> = threads
            .into_iter()
            .flat_map(|t| t.join().unwrap())
            .collect();
        drawn.sort_unstable();
        let mut want: Vec<u32> = reference(4000).into_iter().map(f32::to_bits).collect();
        want.sort_unstable();
        assert_eq!(drawn, want);
    }

    #[test]
    fn wide_draws_and_bytes_are_those_of_the_one_generator() {
        let _guard = exclusive();
        let mut reference = Xorshift128::new();
        assert_eq!(ProcessRng.next_u64(), reference.next_u64());
        let (mut got, mut want) = ([0_u8; 7], [0_u8; 7]);
        ProcessRng.fill_bytes(&mut got);
        reference.fill_bytes(&mut want);
        assert_eq!(got, want);
        assert_eq!(ProcessRng.next_u32(), reference.next_u32());
    }

    #[test]
    fn a_poisoned_lock_is_recovered() {
        let _guard = exclusive();
        let poisoner = std::thread::spawn(|| {
            let _held = PROCESS_XORSHIFT.lock().unwrap();
            panic!("poisons the generator's lock on purpose");
        });
        assert!(poisoner.join().is_err());
        assert!(PROCESS_XORSHIFT.is_poisoned());
        ProcessRng::reset();
        assert_eq!(sample(&mut ProcessRng).to_bits(), reference(1)[0].to_bits());
        assert_eq!(sample(&mut ProcessRng).to_bits(), reference(2)[1].to_bits());
    }

    #[test]
    fn the_handle_is_a_zero_sized_copy_with_equality() {
        let a = ProcessRng;
        let b = a;
        assert_eq!(a, b);
        assert_eq!(size_of::<ProcessRng>(), 0);
        assert_eq!(format!("{a:?}"), "ProcessRng");
    }
}
