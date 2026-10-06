//! The random source of the `math.random*` / `math.die_roll*` operators: any [`rand_core::Rng`].
//!
//! The generator is a value the embedding owns, so evaluation is reproducible and
//! thread-independent. Each sample is one `next_u32`, mapped by [`sample`]; `rand_core` is
//! re-exported here, so a host needs no dependency of its own.

use rand_core::{Infallible, SeedableRng, TryRng, utils};

/// The `rand_core` 0.10 crate, whose `Rng` is the random source; a new major version of it is a
/// breaking change here.
pub use rand_core;

/// Draws one `next_u32` `u` and returns `(u & 0x7fff_ffff) · 2^-31`, rounded to nearest, which can
/// be exactly 1.0.
///
/// ```
/// use molangx::rng::{FixedRng, Xorshift128, sample};
///
/// assert_eq!(sample(&mut FixedRng::HALF), 0.5);
/// assert_eq!(sample(&mut Xorshift128::new()), sample(&mut Xorshift128::new()));
/// ```
#[inline]
pub fn sample<R: rand_core::Rng + ?Sized>(rng: &mut R) -> f32 {
    (rng.next_u32() & 0x7fff_ffff) as f32 * (1.0 / 2_147_483_648.0)
}

/// Marsaglia's xorshift128.
///
/// Every [`Xorshift128::new`] (and [`Default`], and so every `vm` `HostEnv::new` and
/// `NoHostEnv::new`) starts from the same seeds and draws the same sequence. Give each environment
/// its own seed with [`SeedableRng::seed_from_u64`] (the trait must be in scope), or share one
/// source (`vm::ProcessRng`). [`Xorshift128::with_state`] is the `const` constructor, for a
/// `static`.
///
/// `next_u32` is one step; `next_u64` is two steps, the first in the low half; `fill_bytes` takes
/// one step per 4 bytes, little-endian, discarding the unused bytes of the last.
///
/// ```
/// use molangx::rng::{Xorshift128, rand_core::SeedableRng, sample};
///
/// assert_eq!(sample(&mut Xorshift128::new()), sample(&mut Xorshift128::new()));
/// assert_ne!(
///     sample(&mut Xorshift128::seed_from_u64(1)),
///     sample(&mut Xorshift128::seed_from_u64(2))
/// );
/// ```
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Xorshift128 {
    x: u32,
    y: u32,
    z: u32,
    w: u32,
}

impl Xorshift128 {
    /// The generator's standard seeds.
    pub const STANDARD_SEED: [u32; 4] = [123_456_789, 362_436_069, 521_288_629, 88_675_123];

    /// A generator from the standard seeds.
    #[inline]
    pub const fn new() -> Self {
        Self::with_state(Self::STANDARD_SEED)
    }

    /// A generator with the state `[x, y, z, w]`.
    ///
    /// An all-zero state, a fixed point of xorshift, is replaced by the standard seeds.
    #[inline]
    pub const fn with_state(state: [u32; 4]) -> Self {
        let [x, y, z, w] = match state {
            [0, 0, 0, 0] => Self::STANDARD_SEED,
            state => state,
        };
        Self { x, y, z, w }
    }

    /// The current state `[x, y, z, w]`.
    #[inline]
    pub const fn state(&self) -> [u32; 4] {
        [self.x, self.y, self.z, self.w]
    }

    /// Advances the state and returns the new `w`.
    #[inline]
    const fn step(&mut self) -> u32 {
        let t = self.x ^ (self.x << 11);
        self.x = self.y;
        self.y = self.z;
        self.z = self.w;
        self.w = self.w ^ (self.w >> 19) ^ t ^ (t >> 8);
        self.w
    }
}

impl Default for Xorshift128 {
    #[inline]
    fn default() -> Self {
        Self::new()
    }
}

impl TryRng for Xorshift128 {
    type Error = Infallible;

    #[inline]
    fn try_next_u32(&mut self) -> Result<u32, Infallible> {
        Ok(self.step())
    }

    #[inline]
    fn try_next_u64(&mut self) -> Result<u64, Infallible> {
        utils::next_u64_via_u32(self)
    }

    #[inline]
    fn try_fill_bytes(&mut self, dst: &mut [u8]) -> Result<(), Infallible> {
        utils::fill_bytes_via_next_word(dst, || self.try_next_u32())
    }
}

impl SeedableRng for Xorshift128 {
    /// The state `[x, y, z, w]` as little-endian words.
    type Seed = [u8; 16];

    /// [`Xorshift128::with_state`] of the seed's words.
    #[inline]
    fn from_seed(seed: [u8; 16]) -> Self {
        Self::with_state(utils::read_words(&seed))
    }

    /// The state from two `SplitMix64` outputs of `state`, low word first, so nearby seeds give
    /// unrelated sequences.
    #[inline]
    fn seed_from_u64(state: u64) -> Self {
        let (a, state) = split_mix_64(state);
        let (b, _) = split_mix_64(state);
        Self::with_state([a as u32, (a >> 32) as u32, b as u32, (b >> 32) as u32])
    }

    /// [`SeedableRng::seed_from_u64`] of one `next_u64` of `rng`. Filling the 16-byte seed from a
    /// `Xorshift128` would copy it: four steps leave its state equal to their outputs.
    #[inline]
    fn from_rng<R: rand_core::Rng + ?Sized>(rng: &mut R) -> Self {
        Self::seed_from_u64(rng.next_u64())
    }

    /// [`SeedableRng::from_rng`] for a fallible source.
    #[inline]
    fn try_from_rng<R: TryRng + ?Sized>(rng: &mut R) -> Result<Self, R::Error> {
        rng.try_next_u64().map(Self::seed_from_u64)
    }
}

/// One step of `SplitMix64`: the output and the advanced state.
const fn split_mix_64(state: u64) -> (u64, u64) {
    let state = state.wrapping_add(0x9e37_79b9_7f4a_7c15);
    let mut z = state;
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    (z ^ (z >> 31), state)
}

/// A generator whose every `next_u32` is the same word, so every [`sample`] is the same.
///
/// `next_u64` is the word in both halves; `fill_bytes` repeats its little-endian bytes.
///
/// ```
/// use molangx::rng::{FixedRng, sample};
///
/// assert_eq!(sample(&mut FixedRng::from_sample(0.25).unwrap()), 0.25);
/// assert_eq!(FixedRng::from_sample(1e-10), None, "not k · 2^-31");
/// ```
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub struct FixedRng(pub u32);

impl FixedRng {
    /// Always the sample 0: every random operator returns its lower bound.
    pub const ZERO: Self = Self(0);
    /// Always the sample 0.5.
    pub const HALF: Self = Self(1 << 30);
    /// Always the sample 1: every random operator returns its upper bound.
    pub const ONE: Self = Self(0x7fff_ffff);

    /// The generator whose [`sample`] is `sample`; `None` when no word maps to it: NaN, -0,
    /// outside `[0, 1]`, or not a multiple of 2^-31.
    pub const fn from_sample(sample: f32) -> Option<Self> {
        if sample.is_nan() || sample.is_sign_negative() || sample > 1.0 {
            return None;
        }
        if sample == 1.0 {
            return Some(Self::ONE);
        }
        let scaled = sample * 2_147_483_648.0;
        let word = scaled as u32;
        if word as f32 == scaled {
            Some(Self(word))
        } else {
            None
        }
    }
}

impl TryRng for FixedRng {
    type Error = Infallible;

    #[inline]
    fn try_next_u32(&mut self) -> Result<u32, Infallible> {
        Ok(self.0)
    }

    #[inline]
    fn try_next_u64(&mut self) -> Result<u64, Infallible> {
        utils::next_u64_via_u32(self)
    }

    #[inline]
    fn try_fill_bytes(&mut self, dst: &mut [u8]) -> Result<(), Infallible> {
        utils::fill_bytes_via_next_word(dst, || self.try_next_u32())
    }
}

#[cfg(test)]
mod tests {
    use rand_core::Rng;

    use super::*;

    /// A generator whose next `next_u32` is `target`: with `x = 0` the update is
    /// `w' = w ^ (w >> 19)`, which `w = target ^ (target >> 19)` inverts.
    fn producing(target: u32) -> Xorshift128 {
        Xorshift128::with_state([0, 1, 2, target ^ (target >> 19)])
    }

    /// Plays back its words in order and counts the calls of each method.
    #[derive(Default)]
    struct Counted {
        words: Vec<u32>,
        u32_calls: usize,
        other_calls: usize,
    }

    impl TryRng for Counted {
        type Error = Infallible;

        fn try_next_u32(&mut self) -> Result<u32, Infallible> {
            self.u32_calls += 1;
            Ok(self.words[self.u32_calls - 1])
        }

        fn try_next_u64(&mut self) -> Result<u64, Infallible> {
            self.other_calls += 1;
            Ok(0)
        }

        fn try_fill_bytes(&mut self, _: &mut [u8]) -> Result<(), Infallible> {
            self.other_calls += 1;
            Ok(())
        }
    }

    #[test]
    fn standard_seeds() {
        assert_eq!(
            Xorshift128::STANDARD_SEED,
            [123_456_789, 362_436_069, 521_288_629, 88_675_123]
        );
        assert_eq!(Xorshift128::new().state(), Xorshift128::STANDARD_SEED);
        assert_eq!(Xorshift128::default(), Xorshift128::new());
    }

    #[test]
    fn first_outputs_from_the_standard_seeds() {
        let mut raw = Xorshift128::new();
        let words: Vec<u32> = (0..6).map(|_| raw.next_u32()).collect();
        assert_eq!(
            words,
            [
                3_701_687_786,
                458_299_110,
                2_500_872_618,
                3_633_119_408,
                516_391_518,
                2_377_269_574
            ]
        );

        let mut rng = Xorshift128::new();
        let samples: Vec<u32> = (0..6).map(|_| sample(&mut rng).to_bits()).collect();
        assert_eq!(
            samples,
            [
                0x3f39_468c,
                0x3e5a_88b7,
                0x3e28_824d,
                0x3f31_1a01,
                0x3e76_3c13,
                0x3ddb_2414
            ]
        );
    }

    #[test]
    fn state_after_each_of_the_first_steps() {
        let mut rng = Xorshift128::new();
        rng.next_u32();
        assert_eq!(
            rng.state(),
            [362_436_069, 521_288_629, 88_675_123, 3_701_687_786]
        );
        rng.next_u32();
        assert_eq!(
            rng.state(),
            [521_288_629, 88_675_123, 3_701_687_786, 458_299_110]
        );
        rng.next_u32();
        assert_eq!(
            rng.state(),
            [88_675_123, 3_701_687_786, 458_299_110, 2_500_872_618]
        );
    }

    #[test]
    fn next_u32_returns_the_new_w() {
        let mut rng = Xorshift128::new();
        for _ in 0..16 {
            let out = rng.next_u32();
            assert_eq!(out, rng.state()[3]);
        }
    }

    #[test]
    fn next_u64_is_two_steps_low_half_first() {
        let mut words = Xorshift128::new();
        let mut wide = Xorshift128::new();
        for _ in 0..8 {
            let (low, high) = (words.next_u32(), words.next_u32());
            assert_eq!(wide.next_u64(), u64::from(high) << 32 | u64::from(low));
        }
        assert_eq!(words, wide);
        assert_eq!(
            Xorshift128::new().next_u64(),
            1_968_379_692_937_594_346,
            "458_299_110 · 2^32 + 3_701_687_786"
        );
    }

    #[test]
    fn fill_bytes_is_one_step_per_four_bytes_little_endian() {
        let mut words = Xorshift128::new();
        let expected: Vec<u8> = (0..3)
            .flat_map(|_| words.next_u32().to_le_bytes())
            .take(10)
            .collect();
        let mut bytes = Xorshift128::new();
        let mut filled = [0_u8; 10];
        bytes.fill_bytes(&mut filled);
        assert_eq!(filled[..], expected[..]);
        assert_eq!(bytes, words, "the last partial word is a whole step");
        assert_eq!(filled[..4], 3_701_687_786_u32.to_le_bytes());
    }

    #[test]
    fn sample_is_the_masked_output_times_two_to_the_minus_31() {
        let mut words = Xorshift128::new();
        let mut samples = Xorshift128::new();
        for _ in 0..64 {
            let w = words.next_u32();
            let expected = (w & 0x7fff_ffff) as f32 * 2.0_f32.powi(-31);
            assert_eq!(sample(&mut samples).to_bits(), expected.to_bits());
        }
        assert_eq!(words, samples);
    }

    #[test]
    fn a_sample_is_one_next_u32_and_nothing_else() {
        let mut source = Counted {
            words: vec![0, 1 << 30, 0x7fff_ffff, 0x8000_0000, u32::MAX, 0x1234_5678],
            ..Counted::default()
        };
        let samples: Vec<f32> = (0..6).map(|_| sample(&mut source)).collect();
        assert_eq!(
            samples,
            [
                0.0,
                0.5,
                1.0,
                0.0,
                1.0,
                0x1234_5678 as f32 / 2_147_483_648.0
            ]
        );
        assert_eq!((source.u32_calls, source.other_calls), (6, 0));
    }

    #[test]
    fn a_dynamic_or_borrowed_source_is_a_source() {
        let mut rng = Xorshift128::new();
        let first = sample(&mut &mut rng);
        let dynamic: &mut dyn Rng = &mut rng;
        let second = sample(dynamic);
        let mut reference = Xorshift128::new();
        assert_eq!(
            (first, second),
            (sample(&mut reference), sample(&mut reference))
        );
    }

    #[test]
    fn the_top_bit_of_the_output_is_ignored() {
        assert_eq!(
            sample(&mut producing(0x8000_0000)).to_bits(),
            0.0_f32.to_bits()
        );
        assert_eq!(
            sample(&mut producing(0x8000_0001)),
            sample(&mut producing(1))
        );
        assert_eq!(
            sample(&mut producing(0xffff_ffff)),
            sample(&mut producing(0x7fff_ffff))
        );
    }

    #[test]
    fn extreme_outputs_map_to_the_ends_of_the_unit_interval() {
        assert_eq!(producing(0).next_u32(), 0);
        assert_eq!(sample(&mut producing(0)).to_bits(), 0.0_f32.to_bits());
        assert_eq!(sample(&mut producing(1)).to_bits(), 0x3000_0000);
        // Spacing below 2^31 is 128: 2^31 − 65 rounds down to 1 − 2^-24, the tie at 2^31 − 64
        // rounds to even, which is 2^31.
        assert_eq!(sample(&mut producing(0x7fff_ffbf)).to_bits(), 0x3f7f_ffff);
        assert_eq!(sample(&mut producing(0x7fff_ffc0)), 1.0);
        assert_eq!(sample(&mut producing(0x7fff_ffff)), 1.0);
    }

    #[test]
    fn samples_stay_in_the_closed_unit_interval() {
        let mut rng = Xorshift128::new();
        for _ in 0..100_000 {
            let sample = sample(&mut rng);
            assert!((0.0..=1.0).contains(&sample), "{sample}");
        }
    }

    #[test]
    fn a_sample_can_round_to_exactly_one() {
        // 0xffffe000 steps to 0xffffffff, whose low 31 bits round up to 2^31.
        let mut rng = Xorshift128::with_state([0, 1, 2, 0xffff_e000]);
        assert_eq!(rng.clone().next_u32(), 0xffff_ffff);
        assert_eq!(sample(&mut rng), 1.0);
    }

    #[test]
    fn state_and_seed_constructors() {
        let state = [1, 2, 3, 4];
        assert_eq!(Xorshift128::with_state(state).state(), state);
        assert_eq!(Xorshift128::with_state([0; 4]), Xorshift128::new());

        let (mut a, mut b, mut c) = (
            Xorshift128::seed_from_u64(1),
            Xorshift128::seed_from_u64(1),
            Xorshift128::seed_from_u64(2),
        );
        assert_ne!(a.state(), [0; 4]);
        assert_ne!(Xorshift128::seed_from_u64(0).state(), [0; 4]);
        let (xs, ys, zs): (Vec<u32>, Vec<u32>, Vec<u32>) = (
            (0..8).map(|_| a.next_u32()).collect(),
            (0..8).map(|_| b.next_u32()).collect(),
            (0..8).map(|_| c.next_u32()).collect(),
        );
        assert_eq!(xs, ys);
        assert_ne!(xs, zs);
    }

    #[test]
    fn from_seed_reads_little_endian_words() {
        let bytes: Vec<u8> = [1_u32, 2, 3, 0x0405_0607]
            .iter()
            .flat_map(|word| word.to_le_bytes())
            .collect();
        let seed: [u8; 16] = bytes.try_into().expect("16 bytes");
        assert_eq!(Xorshift128::from_seed(seed).state(), [1, 2, 3, 0x0405_0607]);
        assert_eq!(
            Xorshift128::from_seed([0; 16]),
            Xorshift128::new(),
            "the all-zero fallback"
        );
    }

    #[test]
    fn a_state_with_one_nonzero_word_is_kept() {
        for i in 0..4 {
            let mut state = [0_u32; 4];
            state[i] = 1;
            assert_eq!(Xorshift128::with_state(state).state(), state, "word {i}");
        }
    }

    #[test]
    fn the_all_zero_fallback_matches_a_new_generator_draw_for_draw() {
        let mut fallback = Xorshift128::with_state([0; 4]);
        let mut fresh = Xorshift128::new();
        for _ in 0..8 {
            assert_eq!(fallback.next_u32(), fresh.next_u32());
        }
    }

    #[test]
    fn seed_from_u64_fills_the_state_from_split_mix_64() {
        assert_eq!(
            Xorshift128::seed_from_u64(0).state(),
            [2_065_550_767, 3_793_791_033, 2_713_282_036, 1_853_398_634]
        );
        assert_eq!(
            Xorshift128::seed_from_u64(1).state(),
            [2_298_633_409, 2_433_363_436, 1_703_865_447, 3_203_108_257]
        );
        assert_eq!(
            Xorshift128::seed_from_u64(2).state(),
            [479_680_206, 2_539_140_574, 201_072_194, 3_217_573_392]
        );
    }

    #[test]
    fn a_fork_differs_from_its_parent() {
        let mut parent = Xorshift128::new();
        let mut child = parent.fork();
        assert_ne!(child, parent);
        assert_ne!(sample(&mut child), sample(&mut parent));
        let mut parent = Xorshift128::seed_from_u64(7);
        let mut child = parent.try_fork().expect("infallible");
        assert_ne!(child, parent);
        assert_ne!(
            (0..4).map(|_| child.next_u32()).collect::<Vec<_>>(),
            (0..4).map(|_| parent.next_u32()).collect::<Vec<_>>()
        );
        let mut source = Xorshift128::new();
        let expected = Xorshift128::seed_from_u64(Xorshift128::new().next_u64());
        assert_eq!(Xorshift128::from_rng(&mut source), expected);
    }

    #[test]
    fn split_mix_64_returns_the_output_and_the_advanced_state() {
        let (out, next) = split_mix_64(0);
        assert_eq!(next, 0x9e37_79b9_7f4a_7c15);
        assert_eq!(out & 0xffff_ffff, 2_065_550_767);
        assert_eq!(out >> 32, 3_793_791_033);
        let (_, wrapped) = split_mix_64(u64::MAX);
        assert_eq!(wrapped, 0x9e37_79b9_7f4a_7c14);
    }

    #[test]
    fn nearby_seeds_give_unrelated_first_samples() {
        let firsts: Vec<u32> = (0..16_u64)
            .map(|seed| sample(&mut Xorshift128::seed_from_u64(seed)).to_bits())
            .collect();
        let mut sorted = firsts.clone();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted.len(), firsts.len());
    }

    #[test]
    fn a_clone_continues_independently() {
        let mut a = Xorshift128::new();
        a.next_u32();
        let mut b = a.clone();
        assert_eq!(a, b);
        assert_eq!(a.next_u32(), b.next_u32());
        a.next_u32();
        assert_ne!(a, b);
    }

    #[test]
    fn fixed_sources() {
        for (mut source, expected) in [
            (FixedRng::ZERO, 0.0),
            (FixedRng::HALF, 0.5),
            (FixedRng::ONE, 1.0),
        ] {
            assert_eq!(sample(&mut source), expected);
            assert_eq!(FixedRng::from_sample(expected), Some(source));
        }
        let mut fixed = FixedRng(0x1234_5678);
        assert_eq!(
            (fixed.next_u32(), fixed.next_u32()),
            (0x1234_5678, 0x1234_5678)
        );
        assert_eq!(fixed.next_u64(), 0x1234_5678_1234_5678);
        let mut bytes = [0_u8; 6];
        fixed.fill_bytes(&mut bytes);
        assert_eq!(bytes, [0x78, 0x56, 0x34, 0x12, 0x78, 0x56]);
        assert_eq!(fixed, FixedRng(0x1234_5678), "drawing leaves it as it was");
    }

    #[test]
    fn from_sample_takes_exactly_the_samples_a_word_maps_to() {
        for value in [
            0.25,
            0.3,
            0.6,
            1.0 / 3.0,
            0.999_999_94,
            2.0_f32.powi(-8),
            2.0_f32.powi(-31),
            3.0 * 2.0_f32.powi(-31),
        ] {
            let mut fixed = FixedRng::from_sample(value).unwrap_or_else(|| panic!("{value}"));
            assert_eq!(sample(&mut fixed).to_bits(), value.to_bits(), "{value}");
        }
        for value in [
            -0.0,
            -1.0,
            1.000_000_1,
            2.0,
            f32::NAN,
            f32::INFINITY,
            f32::NEG_INFINITY,
            1e-10,
            2.0_f32.powi(-32),
            1.5 * 2.0_f32.powi(-31),
        ] {
            assert_eq!(FixedRng::from_sample(value), None, "{value}");
        }
        assert_eq!(FixedRng::from_sample(0.0), Some(FixedRng::ZERO));
    }

    #[test]
    fn every_word_round_trips_through_from_sample() {
        let mut rng = Xorshift128::new();
        for _ in 0..10_000 {
            let mut fixed = FixedRng(rng.next_u32());
            let value = sample(&mut fixed);
            assert_eq!(
                FixedRng::from_sample(value).map(|mut f| sample(&mut f).to_bits()),
                Some(value.to_bits())
            );
        }
    }
}
