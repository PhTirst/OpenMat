use std::time::{SystemTime, UNIX_EPOCH};

const STATE_WORDS: usize = 624;
const MATRIX_A: u32 = 0x9908_b0df;
const UPPER_MASK: u32 = 0x8000_0000;
const LOWER_MASK: u32 = 0x7fff_ffff;

/// Snapshot of one interpreter's deterministic random stream.
#[derive(Clone, Debug, PartialEq)]
pub struct RandomSnapshot {
    /// User-visible seed supplied at the last reset.
    pub seed: u32,
    /// MT19937 state words followed by the current one-based state cursor.
    pub state: Vec<u32>,
}

/// Narrow session random-number service injected into built-in invocations.
pub trait RandomService {
    /// Draws one uniform value from the open interval `[0, 1)`.
    fn next_uniform(&mut self) -> f64;
    /// Draws one standard-normal value.
    fn next_normal(&mut self) -> f64;
    /// Resets the stream to a deterministic seed.
    fn reseed(&mut self, seed: u32);
    /// Resets the stream from host entropy.
    fn shuffle(&mut self);
    /// Captures the current deterministic stream state.
    fn snapshot(&self) -> RandomSnapshot;
}

/// MATLAB-style per-interpreter `twister` stream.
#[derive(Clone, Debug)]
pub struct RandomSession {
    seed: u32,
    state: [u32; STATE_WORDS],
    index: usize,
    spare_normal: Option<f64>,
}

impl RandomSession {
    /// Creates the R2022b-compatible default twister stream.
    #[must_use]
    pub fn new() -> Self {
        let mut session = Self {
            seed: 0,
            state: [0; STATE_WORDS],
            index: STATE_WORDS,
            spare_normal: None,
        };
        session.reseed(0);
        session
    }

    fn next_u32(&mut self) -> u32 {
        if self.index >= STATE_WORDS {
            self.twist();
        }
        let mut value = self.state[self.index];
        self.index += 1;
        value ^= value >> 11;
        value ^= (value << 7) & 0x9d2c_5680;
        value ^= (value << 15) & 0xefc6_0000;
        value ^= value >> 18;
        value
    }

    fn twist(&mut self) {
        for index in 0..STATE_WORDS {
            let combined = (self.state[index] & UPPER_MASK)
                | (self.state[(index + 1) % STATE_WORDS] & LOWER_MASK);
            let mut value = self.state[(index + 397) % STATE_WORDS] ^ (combined >> 1);
            if !combined.is_multiple_of(2) {
                value ^= MATRIX_A;
            }
            self.state[index] = value;
        }
        self.index = 0;
    }
}

impl Default for RandomSession {
    fn default() -> Self {
        Self::new()
    }
}

impl RandomService for RandomSession {
    #[allow(clippy::cast_precision_loss)]
    fn next_uniform(&mut self) -> f64 {
        let high = u64::from(self.next_u32() >> 5);
        let low = u64::from(self.next_u32() >> 6);
        ((high << 26) + low) as f64 * (1.0 / 9_007_199_254_740_992.0)
    }

    fn next_normal(&mut self) -> f64 {
        if let Some(value) = self.spare_normal.take() {
            return value;
        }
        let radius = (-2.0 * (1.0 - self.next_uniform()).ln()).sqrt();
        let angle = std::f64::consts::TAU * self.next_uniform();
        self.spare_normal = Some(radius * angle.sin());
        radius * angle.cos()
    }

    fn reseed(&mut self, seed: u32) {
        self.seed = seed;
        self.state[0] = if seed == 0 { 5489 } else { seed };
        for index in 1..STATE_WORDS {
            let previous = self.state[index - 1];
            self.state[index] = 1_812_433_253_u32
                .wrapping_mul(previous ^ (previous >> 30))
                .wrapping_add(u32::try_from(index).expect("MT19937 index fits u32"));
        }
        self.index = STATE_WORDS;
        self.spare_normal = None;
    }

    fn shuffle(&mut self) {
        let duration = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default();
        let seconds = duration.as_secs().to_le_bytes();
        let folded_seconds = u32::from_le_bytes([seconds[0], seconds[1], seconds[2], seconds[3]])
            ^ u32::from_le_bytes([seconds[4], seconds[5], seconds[6], seconds[7]]);
        let seed = duration.subsec_nanos() ^ folded_seconds.rotate_left(13);
        self.reseed(seed);
    }

    fn snapshot(&self) -> RandomSnapshot {
        let mut state = Vec::with_capacity(STATE_WORDS + 1);
        state.extend_from_slice(&self.state);
        state.push(u32::try_from(self.index + 1).unwrap_or(u32::MAX));
        RandomSnapshot {
            seed: self.seed,
            state,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[allow(clippy::float_cmp)]
    fn default_uniform_prefix_matches_matlab_twister() {
        let mut random = RandomSession::new();
        let values = [
            random.next_uniform(),
            random.next_uniform(),
            random.next_uniform(),
        ];
        assert_eq!(
            values,
            [
                0.814_723_686_393_178_9,
                0.905_791_937_075_619_2,
                0.126_986_816_293_506_06,
            ]
        );
    }

    #[test]
    fn reseeding_replays_uniform_and_normal_draws() {
        let mut random = RandomSession::new();
        random.reseed(7);
        let first = (random.next_uniform(), random.next_normal());
        random.reseed(7);
        assert_eq!(first, (random.next_uniform(), random.next_normal()));
    }
}
