//! The random draws of the designs and of the bootstrap: xoshiro256++ of Blackman and Vigna,
//! seeded through SplitMix64.
//!
//! The stream of a seed is a part of the contract of the crate: the points of a design and the
//! resamples of an interval follow from the seed and the arguments alone, on every target, so
//! a record of the seed recreates them. The generator is small and written out here rather than
//! taken from a crate, so no update of a dependency can change a stream.

/// The coefficients of `x^0` to `x^255` of the characteristic polynomial of the linear
/// transition of the state of xoshiro256 over GF(2), the bit `i % 64` of the word `i / 64` the
/// coefficient of `x^i`; the polynomial is of degree 256 (`x^256` is not stored) and primitive.
/// Found by Berlekamp and Massey from a sequence of the generator, and held by the test which
/// derives the jump polynomials of 2^128 and 2^192 draws of Blackman and Vigna from it.
const CHARACTERISTIC: [u64; 4] = [
    0x9d11_6f2b_b0f0_f001,
    0x0280_002b_cefd_1a5e,
    0x04b4_edcf_2625_9f85,
    0x0003_c03c_3f3e_cb19,
];

/// The most steps [`Stream::advance`] draws one after the other: fewer than the 256 draws and
/// the polynomials of a jump cost.
const STEPPED: u128 = 1024;

/// The product of two polynomials of degree below 256 over GF(2) modulo [`CHARACTERISTIC`]:
/// the product without carries, reduced from its highest coefficient down.
fn multiply(a: &[u64; 4], b: &[u64; 4]) -> [u64; 4] {
    let mut product = [0u64; 8];
    for (word, &bits) in a.iter().enumerate() {
        for bit in 0..64 {
            if bits >> bit & 1 == 1 {
                add_shifted(&mut product, b, 64 * word + bit);
            }
        }
    }
    for degree in (256..512).rev() {
        if product[degree / 64] >> (degree % 64) & 1 == 1 {
            // x^degree = x^(degree - 256) (p(x) - x^256): the term cleared and the lower
            // coefficients of p added
            product[degree / 64] ^= 1 << (degree % 64);
            add_shifted(&mut product, &CHARACTERISTIC, degree - 256);
        }
    }
    [product[0], product[1], product[2], product[3]]
}

/// Adds the polynomial `q` times `x^shift` to `sum`, `shift` below 256.
fn add_shifted(sum: &mut [u64; 8], q: &[u64; 4], shift: usize) {
    let (words, bits) = (shift / 64, shift % 64);
    for (i, &x) in q.iter().enumerate() {
        sum[words + i] ^= x << bits;
        if bits > 0 {
            sum[words + i + 1] ^= x >> (64 - bits);
        }
    }
}

/// The polynomial `q` times `x` modulo [`CHARACTERISTIC`].
fn times_x(q: &[u64; 4]) -> [u64; 4] {
    let carry = q[3] >> 63;
    let mut shifted = [
        q[0] << 1,
        q[1] << 1 | q[0] >> 63,
        q[2] << 1 | q[1] >> 63,
        q[3] << 1 | q[2] >> 63,
    ];
    if carry == 1 {
        for (s, &c) in shifted.iter_mut().zip(&CHARACTERISTIC) {
            *s ^= c;
        }
    }
    shifted
}

/// `x^n` modulo [`CHARACTERISTIC`], squared and multiplied by `x` from the highest bit of `n`.
fn power_of_x(n: u128) -> [u64; 4] {
    let mut power = [1, 0, 0, 0];
    for bit in (0..128 - n.leading_zeros()).rev() {
        power = multiply(&power, &power);
        if n >> bit & 1 == 1 {
            power = times_x(&power);
        }
    }
    power
}

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

    /// Advances the stream by `steps` draws, to the state `steps` calls of
    /// [`next_u64`](Self::next_u64) leave, in a time logarithmic in `steps`.
    ///
    /// The transition of the state of xoshiro256++ is linear over GF(2), a 256 x 256 matrix
    /// `T` of bits, so `steps` draws are `T^steps`. By Cayley and Hamilton `T^steps` is the
    /// polynomial `x^steps mod p(x)` of `T`, with `p` the characteristic polynomial of `T`
    /// ([`CHARACTERISTIC`]); the remainder is found by squaring and multiplying modulo `p`,
    /// and applied to the state as the jump functions of Blackman and Vigna apply theirs: the
    /// sum of the states after the draws `i` of the coefficients of `x^i` which are one. A few
    /// steps are drawn one after the other, which gives the same state.
    pub fn advance(&mut self, steps: u128) {
        if steps <= STEPPED {
            for _ in 0..steps {
                self.next_u64();
            }
        } else {
            self.apply(&power_of_x(steps));
        }
    }

    /// The state of the polynomial `q` of the transition, `q(T) s`: the sum of the states
    /// after the draws `i` of the coefficients of `x^i` of `q` which are one, after 256 draws.
    fn apply(&mut self, q: &[u64; 4]) {
        let mut sum = [0u64; 4];
        for word in q {
            for bit in 0..64 {
                if word >> bit & 1 == 1 {
                    for (s, &x) in sum.iter_mut().zip(&self.state) {
                        *s ^= x;
                    }
                }
                self.next_u64();
            }
        }
        self.state = sum;
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
    use super::{Stream, multiply, power_of_x};

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
    fn the_characteristic_polynomial_gives_the_jumps_of_the_authors() {
        // JUMP (2^128 draws) and LONG_JUMP (2^192 draws) of the reference implementation of
        // xoshiro256++, which are x^(2^128) and x^(2^192) modulo the characteristic polynomial
        let jump = multiply(&power_of_x(1 << 127), &power_of_x(1 << 127));
        assert_eq!(
            jump,
            [
                0x180e_c6d3_3cfd_0aba,
                0xd5a6_1266_f0c9_392c,
                0xa958_2618_e03f_c9aa,
                0x39ab_dc45_29b1_661c
            ]
        );
        // x^(2^192), the jump squared 64 times
        let long_jump = (0..64).fold(jump, |q, _| multiply(&q, &q));
        assert_eq!(
            long_jump,
            [
                0x76e1_5d3e_fefd_cbbf,
                0xc500_4e44_1c52_2fb3,
                0x7771_0069_854e_e241,
                0x3910_9bb0_2acb_e635
            ]
        );
    }

    #[test]
    fn an_advance_is_the_state_of_as_many_draws() {
        for seed in [0, 1, 42] {
            for steps in [
                0u128, 1, 7, 255, 256, 257, 1000, 1023, 1024, 1025, 4096, 65_537, 999_983,
            ] {
                let mut stepped = Stream::new(seed);
                for _ in 0..steps {
                    stepped.next_u64();
                }
                let mut advanced = Stream::new(seed);
                advanced.advance(steps);
                assert_eq!(advanced.state, stepped.state, "{seed} {steps}");
                // the polynomial of the jump, also where the advance draws one after the other
                let mut jumped = Stream::new(seed);
                jumped.apply(&power_of_x(steps));
                assert_eq!(jumped.state, stepped.state, "{seed} {steps}");
                assert_eq!(advanced.next_u64(), stepped.next_u64());
            }
        }
    }

    #[test]
    fn advances_add_up() {
        // 2^100 + 3 draws as 2^100 draws and three more, and as two advances
        let mut whole = Stream::new(9);
        whole.advance((1 << 100) + 3);
        let mut parts = Stream::new(9);
        parts.advance(1 << 100);
        for _ in 0..3 {
            parts.next_u64();
        }
        assert_eq!(whole.state, parts.state);
        let mut twice = Stream::new(9);
        twice.advance(1 << 99);
        twice.advance((1 << 99) + 3);
        assert_eq!(whole.state, twice.state);
        let mut largest = Stream::new(9);
        largest.advance(u128::MAX);
        assert_ne!(largest.state, [0; 4]);
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
