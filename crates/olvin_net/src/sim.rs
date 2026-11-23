//! Network simulation: latency, jitter, loss, duplication and reordering as a
//! delay queue. [`Loopback`](crate::Loopback) is built on it and
//! [`Conditioned`](crate::Conditioned) puts the same conditions on any transport.
//!
//! Reliable packets are never lost or duplicated, as on TCP or a QUIC stream:
//! a "lost" one is *delayed* by a retransmission timeout and holds back the
//! ones behind it (head-of-line blocking).

use std::cmp::Reverse;
use std::collections::BinaryHeap;

/// What the simulated link does to packets, one way. A round trip sees it twice.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct NetConditions {
    /// Seconds added to every packet.
    pub latency: f32,
    /// Extra random delay in `0..=jitter` seconds.
    pub jitter: f32,
    /// Probability (0..1) that an unreliable packet is dropped.
    pub loss: f32,
    /// Probability that an unreliable packet arrives twice.
    pub duplicate: f32,
    /// Probability that an unreliable packet is held back and overtaken by later ones.
    pub reorder: f32,
    /// Drops everything (a pulled cable); reliable data too.
    pub blackhole: bool,
}

impl NetConditions {
    /// A perfect link.
    pub const NONE: Self = Self {
        latency: 0.0,
        jitter: 0.0,
        loss: 0.0,
        duplicate: 0.0,
        reorder: 0.0,
        blackhole: false,
    };

    /// One-way latency in milliseconds, nothing else.
    pub fn lag_ms(ms: f32) -> Self {
        Self {
            latency: ms.max(0.0) / 1000.0,
            ..Self::NONE
        }
    }

    pub fn with_jitter_ms(mut self, ms: f32) -> Self {
        self.jitter = ms.max(0.0) / 1000.0;
        self
    }

    /// `percent` of unreliable packets are dropped.
    pub fn with_loss_percent(mut self, percent: f32) -> Self {
        self.loss = (percent / 100.0).clamp(0.0, 1.0);
        self
    }

    pub fn with_duplicate_percent(mut self, percent: f32) -> Self {
        self.duplicate = (percent / 100.0).clamp(0.0, 1.0);
        self
    }

    pub fn with_reorder_percent(mut self, percent: f32) -> Self {
        self.reorder = (percent / 100.0).clamp(0.0, 1.0);
        self
    }

    /// Parses `"lag=80,jitter=20,loss=2,dup=1,reorder=5"` (milliseconds and
    /// percents; `blackhole` alone cuts the link). For `OLVIN_NET_SIM` and settings.
    pub fn parse(text: &str) -> Result<Self, String> {
        let mut c = Self::NONE;
        for part in text.split(',').map(str::trim).filter(|p| !p.is_empty()) {
            if part == "blackhole" {
                c.blackhole = true;
                continue;
            }
            let (key, value) = part
                .split_once('=')
                .ok_or_else(|| format!("`{part}`: expected key=value, e.g. lag=80"))?;
            let n: f32 = value
                .trim()
                .parse()
                .map_err(|_| format!("`{part}`: `{value}` is not a number"))?;
            if !n.is_finite() {
                return Err(format!("`{part}`: not a finite number"));
            }
            c = match key.trim() {
                "lag" | "latency" => Self {
                    latency: n.max(0.0) / 1000.0,
                    ..c
                },
                "jitter" => c.with_jitter_ms(n),
                "loss" => c.with_loss_percent(n),
                "dup" | "duplicate" => c.with_duplicate_percent(n),
                "reorder" => c.with_reorder_percent(n),
                other => {
                    return Err(format!(
                        "unknown key `{other}` (use lag, jitter, loss, dup, reorder, blackhole)"
                    ));
                }
            };
        }
        Ok(c)
    }
}

impl Default for NetConditions {
    fn default() -> Self {
        Self::NONE
    }
}

/// Small deterministic generator (xorshift64*): simulations repeat for a given seed.
#[derive(Clone, Debug)]
pub struct SimRng(u64);

impl SimRng {
    pub fn new(seed: u64) -> Self {
        Self(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1)
    }

    pub fn next_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    /// Uniform in `0..1`.
    pub fn unit(&mut self) -> f32 {
        (self.next_u64() >> 40) as f32 / (1u64 << 24) as f32
    }
}

struct Item<T> {
    at_us: u64,
    seq: u64,
    value: T,
}

impl<T> PartialEq for Item<T> {
    fn eq(&self, other: &Self) -> bool {
        (self.at_us, self.seq) == (other.at_us, other.seq)
    }
}
impl<T> Eq for Item<T> {}
impl<T> PartialOrd for Item<T> {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}
impl<T> Ord for Item<T> {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        (self.at_us, self.seq).cmp(&(other.at_us, other.seq))
    }
}

/// One direction of a simulated link: packets go in with a send time and come
/// out when they are due.
pub struct LinkSim<T> {
    pub conditions: NetConditions,
    rng: SimRng,
    queue: BinaryHeap<Reverse<Item<T>>>,
    seq: u64,
    last_reliable_us: u64,
}

const MAX_QUEUED: usize = 65_536;

impl<T: Clone> LinkSim<T> {
    pub fn new(conditions: NetConditions, seed: u64) -> Self {
        Self {
            conditions,
            rng: SimRng::new(seed),
            queue: BinaryHeap::new(),
            seq: 0,
            last_reliable_us: 0,
        }
    }

    fn put(&mut self, at: f64, value: T) {
        if self.queue.len() >= MAX_QUEUED {
            return;
        }
        self.seq += 1;
        self.queue.push(Reverse(Item {
            at_us: (at.max(0.0) * 1e6) as u64,
            seq: self.seq,
            value,
        }));
    }

    /// Sends `value` at time `now` (seconds).
    pub fn push(&mut self, now: f64, reliable: bool, value: T) {
        let c = self.conditions;
        if c.blackhole {
            return;
        }
        let base = c.latency as f64;
        let jitter = |rng: &mut SimRng| (c.jitter * rng.unit()) as f64;
        if reliable {
            // A "lost" reliable packet costs retransmission timeouts instead.
            let rto = (2.0 * base).max(0.05);
            let mut delay = base + jitter(&mut self.rng);
            let mut tries = 0;
            while tries < 8 && self.rng.unit() < c.loss {
                delay += rto;
                tries += 1;
            }
            let at_us = (((now + delay).max(0.0) * 1e6) as u64).max(self.last_reliable_us);
            self.last_reliable_us = at_us;
            self.put(at_us as f64 / 1e6, value);
            return;
        }
        if self.rng.unit() < c.loss {
            return;
        }
        let mut delay = base + jitter(&mut self.rng);
        if self.rng.unit() < c.reorder {
            delay += base + c.jitter as f64 + 0.02;
        }
        if self.rng.unit() < c.duplicate {
            let again = base + jitter(&mut self.rng) + 0.001;
            self.put(now + again, value.clone());
        }
        self.put(now + delay, value);
    }

    /// Moves everything due at `now` into `out`, earliest first.
    pub fn drain(&mut self, now: f64, out: &mut Vec<T>) {
        let now_us = (now.max(0.0) * 1e6) as u64;
        while let Some(Reverse(top)) = self.queue.peek() {
            if top.at_us > now_us {
                break;
            }
            if let Some(Reverse(item)) = self.queue.pop() {
                out.push(item.value);
            }
        }
    }

    pub fn len(&self) -> usize {
        self.queue.len()
    }

    pub fn is_empty(&self) -> bool {
        self.queue.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(c: NetConditions, reliable: bool, n: u32) -> Vec<u32> {
        let mut sim = LinkSim::new(c, 7);
        for i in 0..n {
            sim.push(i as f64 * 0.01, reliable, i);
        }
        let mut out = Vec::new();
        sim.drain(1_000.0, &mut out);
        out
    }

    #[test]
    fn latency_delays_delivery() {
        let mut sim = LinkSim::new(NetConditions::lag_ms(100.0), 1);
        sim.push(0.0, false, 1u8);
        let mut out = Vec::new();
        sim.drain(0.05, &mut out);
        assert!(out.is_empty());
        sim.drain(0.11, &mut out);
        assert_eq!(out, [1]);
    }

    #[test]
    fn unreliable_loses_and_duplicates_roughly_as_asked() {
        let lossy = run(NetConditions::NONE.with_loss_percent(30.0), false, 2000);
        assert!((1200..1600).contains(&lossy.len()), "{}", lossy.len());
        let dup = run(
            NetConditions::NONE.with_duplicate_percent(50.0),
            false,
            2000,
        );
        assert!((2700..3300).contains(&dup.len()), "{}", dup.len());
    }

    #[test]
    fn reorder_overtakes() {
        let out = run(
            NetConditions::lag_ms(20.0).with_reorder_percent(30.0),
            false,
            500,
        );
        assert_eq!(out.len(), 500);
        assert!(out.windows(2).any(|w| w[0] > w[1]), "nothing was reordered");
    }

    #[test]
    fn reliable_is_complete_and_in_order_under_loss() {
        let c = NetConditions::lag_ms(30.0)
            .with_jitter_ms(40.0)
            .with_loss_percent(40.0)
            .with_duplicate_percent(40.0)
            .with_reorder_percent(40.0);
        let out = run(c, true, 300);
        assert_eq!(out, (0..300).collect::<Vec<_>>());
    }

    #[test]
    fn blackhole_drops_everything() {
        let c = NetConditions {
            blackhole: true,
            ..NetConditions::NONE
        };
        assert!(run(c, true, 10).is_empty());
    }

    #[test]
    fn parse_settings() {
        let c = NetConditions::parse("lag=80, loss=2, jitter=10, dup=1, reorder=5").unwrap();
        assert!((c.latency - 0.08).abs() < 1e-6 && (c.loss - 0.02).abs() < 1e-6);
        assert!(NetConditions::parse("lag").is_err());
        assert!(NetConditions::parse("speed=3").is_err());
        assert!(NetConditions::parse("lag=nan").is_err());
        assert!(NetConditions::parse("blackhole").unwrap().blackhole);
    }
}
