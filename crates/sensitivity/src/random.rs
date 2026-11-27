//! The random draws of the designs and of the bootstrap: xoshiro256++ of Blackman and Vigna,
//! seeded through SplitMix64.
//!
//! The stream of a seed is a part of the contract of the crate: the points of a design and the
//! resamples of an interval follow from the seed and the arguments alone, on every target, so
//! a record of the seed recreates them. The generator is small and written out here rather than
//! taken from a crate, so no update of a dependency can change a stream.

/// A stream of random numbers of a seed.
#[derive(Debug, Clone)]
pub struct Stream {
    state: [u64; 4],
}

impl Stream {
    /// The stream of `seed`: the state of xoshiro256++ is four outputs of SplitMix64 started at
    /// the seed, the seeding its authors recommend, so every seed, 0 included, gives a state
    /// which is not zero.
    #[must_use]
    pub fn new(seed: u64) -> Self {
        let mut x = seed;
        let mut next = || {
            x = x.wrapping_add(0x9e37_79b9_7f4a_7c15);
            let mut z = x;
            z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
            z ^ (z >> 31)
        };
        Self {
            state: [next(), next(), next(), next()],
        }
    }

    /// The next 64 random bits.
    pub fn next_u64(&mut self) -> u64 {
        let s = &mut self.state;
        let result = s[0].wrapping_add(s[3]).rotate_left(23).wrapping_add(s[0]);
        let t = s[1] << 17;
        s[2] ^= s[0];
        s[3] ^= s[1];
        s[1] ^= s[2];
        s[0] ^= s[3];
        s[2] ^= t;
        s[3] = s[3].rotate_left(45);
        result
    }

    /// A uniform double of `[0, 1)`: the upper 53 bits of the next draw times `2^-53`.
    pub fn uniform(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 * (1.0 / (1u64 << 53) as f64)
    }

    /// A uniform integer of `[0, n)`, without bias: the multiplication of Lemire with its
    /// rejection of the draws which would favour the lower integers.
    ///
    /// # Panics
    ///
    /// If `n` is zero.
    pub fn below(&mut self, n: usize) -> usize {
        assert!(n > 0, "a draw below zero");
        let n = n as u64;
        let mut product = u128::from(self.next_u64()) * u128::from(n);
        if (product as u64) < n {
            let threshold = n.wrapping_neg() % n;
            while (product as u64) < threshold {
                product = u128::from(self.next_u64()) * u128::from(n);
            }
        }
        (product >> 64) as usize
    }

    /// The lower `bits` bits of the next draw, `bits` at most 64.
    pub fn bits(&mut self, bits: u32) -> u64 {
        let draw = self.next_u64();
        if bits >= 64 {
            draw
        } else {
            draw & ((1u64 << bits) - 1)
        }
    }

    /// Shuffle a slice in place, Fisher and Yates from its end.
    pub fn shuffle<T>(&mut self, items: &mut [T]) {
        for i in (1..items.len()).rev() {
            let j = self.below(i + 1);
            items.swap(i, j);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Stream;

    #[test]
    fn the_stream_of_a_seed_is_fixed() {
        // the reference outputs of xoshiro256++ for the state of SplitMix64 at 0, so the stream
        // of a seed never changes with an edit of the generator
        let mut stream = Stream::new(0);
        let first: Vec<u64> = (0..3).map(|_| stream.next_u64()).collect();
        assert_eq!(
            first,
            [0x53175d61490b23df, 0x61da6f3dc380d507, 0x5c0fdf91ec9a7bfc]
        );
    }

    #[test]
    fn a_draw_below_n_is_uniform() {
        let mut stream = Stream::new(7);
        let mut counts = [0usize; 5];
        for _ in 0..50_000 {
            counts[stream.below(5)] += 1;
        }
        for count in counts {
            assert!((9_500..10_500).contains(&count), "{counts:?}");
        }
    }

    #[test]
    fn a_uniform_is_in_the_unit_interval() {
        let mut stream = Stream::new(3);
        let draws: Vec<f64> = (0..10_000).map(|_| stream.uniform()).collect();
        assert!(draws.iter().all(|&u| (0.0..1.0).contains(&u)));
        let mean = draws.iter().sum::<f64>() / draws.len() as f64;
        assert!((mean - 0.5).abs() < 0.01);
    }

    #[test]
    fn a_shuffle_is_a_permutation() {
        let mut stream = Stream::new(11);
        let mut items: Vec<usize> = (0..20).collect();
        stream.shuffle(&mut items);
        let mut sorted = items.clone();
        sorted.sort_unstable();
        assert_eq!(sorted, (0..20).collect::<Vec<_>>());
        assert_ne!(items, sorted);
    }
}
