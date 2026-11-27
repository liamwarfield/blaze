//! `java.util.Random` with a seed: the same sequence as the JDK's linear congruential generator.

/// `new Random(seed)`.
// Java: java/util/Random.java Random
pub struct JavaRandom {
    seed: u64,
}

impl JavaRandom {
    const MULTIPLIER: u64 = 0x5DEECE66D;
    const ADDEND: u64 = 0xB;
    const MASK: u64 = (1 << 48) - 1;

    /// `new Random(seed)`: `initialScramble(seed)`.
    pub fn new(seed: i64) -> JavaRandom {
        JavaRandom {
            seed: (seed as u64 ^ Self::MULTIPLIER) & Self::MASK,
        }
    }

    /// `next(bits)`.
    fn next(&mut self, bits: u32) -> i32 {
        self.seed = self
            .seed
            .wrapping_mul(Self::MULTIPLIER)
            .wrapping_add(Self::ADDEND)
            & Self::MASK;
        (self.seed >> (48 - bits)) as i64 as i32
    }

    /// `nextInt(bound)`.
    pub fn next_int(&mut self, bound: i32) -> i32 {
        assert!(bound > 0, "bound must be positive");
        let mut r = self.next(31);
        let m = bound - 1;
        if (bound & m) == 0 {
            r = ((i64::from(bound) * i64::from(r)) >> 31) as i32;
        } else {
            let mut u = r;
            loop {
                r = u % bound;
                if u.wrapping_sub(r).wrapping_add(m) >= 0 {
                    break;
                }
                u = self.next(31);
            }
        }
        r
    }

    /// `nextBytes(bytes)`.
    pub fn next_bytes(&mut self, bytes: &mut [u8]) {
        let len = bytes.len();
        let mut i = 0;
        while i < len {
            let mut rnd = self.next(32);
            let mut n = std::cmp::min(len - i, 4);
            while n > 0 {
                bytes[i] = rnd as u8;
                i += 1;
                rnd >>= 8;
                n -= 1;
            }
        }
    }
}

/// `byte[] data = new byte[size]; new Random(seed).nextBytes(data);`
pub fn random_bytes(seed: i64, size: usize) -> Vec<u8> {
    let mut data = vec![0u8; size];
    JavaRandom::new(seed).next_bytes(&mut data);
    data
}
