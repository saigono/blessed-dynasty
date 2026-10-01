use serde::{Deserialize, Serialize};

/// xoshiro256** (Blackman, Vigna). Platform-independent, state is serializable.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Rng {
    s: [u64; 4],
}

impl Rng {
    /// State is expanded from the seed with splitmix64, as the reference recommends.
    pub fn from_seed(seed: u64) -> Rng {
        let mut x = seed;
        let mut next = || {
            x = x.wrapping_add(0x9e37_79b9_7f4a_7c15);
            let mut z = x;
            z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
            z ^ (z >> 31)
        };
        Rng {
            s: [next(), next(), next(), next()],
        }
    }

    pub fn next_u64(&mut self) -> u64 {
        let s = &mut self.s;
        let result = s[1].wrapping_mul(5).rotate_left(7).wrapping_mul(9);
        let t = s[1] << 17;
        s[2] ^= s[0];
        s[3] ^= s[1];
        s[1] ^= s[2];
        s[0] ^= s[3];
        s[2] ^= t;
        s[3] = s[3].rotate_left(45);
        result
    }

    /// Uniform in `lo..hi`. Panics if `hi <= lo`.
    pub fn range(&mut self, lo: i64, hi: i64) -> i64 {
        assert!(lo < hi, "empty range {lo}..{hi}");
        let span = hi.wrapping_sub(lo) as u64;
        // Rejection sampling: drop the tail that would bias `x % span`.
        let limit = u64::MAX - u64::MAX % span;
        loop {
            let x = self.next_u64();
            if x < limit {
                return lo.wrapping_add((x % span) as i64);
            }
        }
    }

    /// True with probability `num / den`. Panics if `den == 0`.
    pub fn chance(&mut self, num: u32, den: u32) -> bool {
        self.range(0, den as i64) < num as i64
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reference_vector() {
        // Output of the reference C implementation for state {1, 2, 3, 4}.
        let mut r = Rng { s: [1, 2, 3, 4] };
        let got: Vec<u64> = (0..4).map(|_| r.next_u64()).collect();
        assert_eq!(got, [11520, 0, 1509978240, 1215971899390074240]);
    }

    #[test]
    fn same_seed_same_sequence() {
        let seq = |seed| {
            let mut r = Rng::from_seed(seed);
            (0..1000).map(|_| r.next_u64()).collect::<Vec<_>>()
        };
        let a = seq(42);
        assert_eq!(a, seq(42));
        assert_ne!(a, seq(43));
        assert_eq!(
            a[..5],
            [
                1546998764402558742,
                6990951692964543102,
                12544586762248559009,
                17057574109182124193,
                18295552978065317476,
            ]
        );
    }

    #[test]
    fn range_bounds_and_coverage() {
        let mut r = Rng::from_seed(7);
        let mut seen = [0u32; 10];
        for _ in 0..10_000 {
            let v = r.range(0, 10);
            assert!((0..10).contains(&v));
            seen[v as usize] += 1;
        }
        assert!(seen.iter().all(|&n| n > 0), "{seen:?}");
        // Negative bounds and the full i64 span must not overflow.
        for _ in 0..1000 {
            assert!((-5..-2).contains(&r.range(-5, -2)));
        }
        r.range(i64::MIN, i64::MAX);
    }

    #[test]
    fn chance_edges() {
        let mut r = Rng::from_seed(1);
        assert!((0..1000).all(|_| !r.chance(0, 3)));
        assert!((0..1000).all(|_| r.chance(3, 3)));
        let hits = (0..10_000).filter(|_| r.chance(1, 4)).count();
        assert!((2200..2800).contains(&hits), "{hits}");
    }

    #[test]
    fn state_roundtrips_through_ron() {
        let mut r = Rng::from_seed(9);
        r.next_u64();
        let mut back: Rng = ron::from_str(&ron::to_string(&r).unwrap()).unwrap();
        assert_eq!(back.next_u64(), r.next_u64());
    }
}
