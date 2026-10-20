//! Price sources: seeded synthetic generators and CSV replay.

use rand::{Rng, SeedableRng};
use rand_chacha::ChaCha8Rng;
use rand_distr::StandardNormal;
use std::fmt::Write as _;

/// One OHLC bar; `t` is the bar close time in seconds.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Bar {
    pub t: f64,
    pub open: f64,
    pub high: f64,
    pub low: f64,
    pub close: f64,
}

pub trait PriceFeed {
    fn next_bar(&mut self) -> Option<Bar>;
    /// Volatility per √second actually driving the path, when known.
    fn true_sigma(&self) -> Option<f64> {
        None
    }
    /// Switches to the next volatility regime (no-op without regimes).
    fn shock(&mut self) {}
}

/// Synthetic path models; all sigmas are per √second.
#[derive(Clone, Debug)]
pub enum Model {
    Gbm {
        sigma: f64,
    },
    /// Markov switching between `sigmas`, with `switch_prob` per bar.
    Regime {
        sigmas: Vec<f64>,
        switch_prob: f64,
    },
    /// GBM plus jumps arriving at `lambda` per second with normal log-size.
    Jump {
        sigma: f64,
        lambda: f64,
        jump_sigma: f64,
    },
}

pub struct SynthFeed {
    model: Model,
    rng: ChaCha8Rng,
    price: f64,
    t: f64,
    bar_secs: f64,
    regime: usize,
}

/// Sub-steps per bar; each one's extremes are drawn from the Brownian bridge.
const SUBSTEPS: usize = 20;

impl SynthFeed {
    pub fn new(model: Model, seed: u64, start_price: f64, bar_secs: f64) -> Self {
        Self { model, rng: ChaCha8Rng::seed_from_u64(seed), price: start_price, t: 0.0, bar_secs, regime: 0 }
    }

    fn diffusion_sigma(&self) -> f64 {
        match &self.model {
            Model::Gbm { sigma } | Model::Jump { sigma, .. } => *sigma,
            Model::Regime { sigmas, .. } => sigmas[self.regime],
        }
    }
}

impl PriceFeed for SynthFeed {
    fn next_bar(&mut self) -> Option<Bar> {
        if let Model::Regime { sigmas, switch_prob } = &self.model
            && self.rng.random::<f64>() < *switch_prob
        {
            self.regime = (self.regime + 1) % sigmas.len();
        }
        let sigma = self.diffusion_sigma();
        let dt = self.bar_secs / SUBSTEPS as f64;
        let open = self.price;
        let (mut high, mut low) = (open, open);
        for _ in 0..SUBSTEPS {
            let z: f64 = self.rng.sample(StandardNormal);
            let mut r = -0.5 * sigma * sigma * dt + sigma * dt.sqrt() * z;
            if let Model::Jump { lambda, jump_sigma, .. } = self.model
                && self.rng.random::<f64>() < lambda * dt
            {
                let j: f64 = self.rng.sample(StandardNormal);
                r += jump_sigma * j;
            }
            // Exact bridge extremes in log space: max = (a + b + √((b-a)² - 2s²·ln U)) / 2.
            let var = sigma * sigma * dt;
            let a = self.price.ln();
            let b = a + r;
            let ext = |u: f64| ((r * r - 2.0 * var * (1.0 - u).ln()).sqrt()) / 2.0;
            high = high.max(((a + b) / 2.0 + ext(self.rng.random())).exp());
            low = low.min(((a + b) / 2.0 - ext(self.rng.random())).exp());
            self.price = b.exp();
        }
        self.t += self.bar_secs;
        Some(Bar { t: self.t, open, high, low, close: self.price })
    }

    fn true_sigma(&self) -> Option<f64> {
        Some(match self.model {
            Model::Jump { sigma, lambda, jump_sigma } => (sigma * sigma + lambda * jump_sigma * jump_sigma).sqrt(),
            _ => self.diffusion_sigma(),
        })
    }

    fn shock(&mut self) {
        if let Model::Regime { sigmas, .. } = &self.model {
            self.regime = (self.regime + 1) % sigmas.len();
        }
    }
}

/// Replays bars parsed from CSV rows `ts,open,high,low,close[,...]`.
/// Header lines are skipped; millisecond timestamps (Binance klines) are detected.
pub struct CsvFeed {
    bars: Vec<Bar>,
    idx: usize,
}

impl CsvFeed {
    pub fn parse(text: &str) -> Result<Self, String> {
        let mut bars = Vec::new();
        for (n, line) in text.lines().enumerate() {
            let fields: Vec<&str> = line.split(',').map(str::trim).collect();
            if fields.len() < 5 || fields[0].parse::<f64>().is_err() {
                continue;
            }
            let num = |i: usize| fields[i].parse::<f64>().map_err(|e| format!("line {}: {e}", n + 1));
            let ts = num(0)?;
            bars.push(Bar {
                // Binance spot klines use µs from 2025 and ms before.
                t: if ts > 1e14 {
                    ts / 1e6
                } else if ts > 1e11 {
                    ts / 1e3
                } else {
                    ts
                },
                open: num(1)?,
                high: num(2)?,
                low: num(3)?,
                close: num(4)?,
            });
        }
        if bars.len() < 2 {
            return Err("need at least 2 bars".into());
        }
        Ok(Self { bars, idx: 0 })
    }

    pub fn load(path: &str) -> Result<Self, String> {
        Self::parse(&std::fs::read_to_string(path).map_err(|e| format!("{path}: {e}"))?)
    }
}

impl PriceFeed for CsvFeed {
    fn next_bar(&mut self) -> Option<Bar> {
        let b = self.bars.get(self.idx).copied();
        self.idx += 1;
        b
    }
}

/// Renders bars in the format [`CsvFeed::parse`] reads.
pub fn to_csv(bars: &[Bar]) -> String {
    let mut s = String::from("ts,open,high,low,close\n");
    for b in bars {
        let _ = writeln!(s, "{},{:.6},{:.6},{:.6},{:.6}", b.t, b.open, b.high, b.low, b.close);
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn synth_is_deterministic_and_ohlc_consistent() {
        let mk = || SynthFeed::new(Model::Jump { sigma: 1e-4, lambda: 1e-3, jump_sigma: 0.01 }, 9, 100.0, 60.0);
        let (mut a, mut b) = (mk(), mk());
        for i in 1..=500 {
            let (x, y) = (a.next_bar().unwrap(), b.next_bar().unwrap());
            assert_eq!(x, y);
            assert_eq!(x.t, i as f64 * 60.0);
            assert!(x.low <= x.open.min(x.close) && x.high >= x.open.max(x.close));
        }
    }

    #[test]
    fn regime_shock_switches_sigma() {
        let mut f = SynthFeed::new(Model::Regime { sigmas: vec![1e-4, 3e-4], switch_prob: 0.0 }, 1, 100.0, 60.0);
        assert_eq!(f.true_sigma(), Some(1e-4));
        f.shock();
        assert_eq!(f.true_sigma(), Some(3e-4));
    }

    #[test]
    fn csv_roundtrip_skips_header_and_converts_ms() {
        let text = "open_time,o,h,l,c,volume\n1700000000000,1,2,0.5,1.5,99\n1700000060000,1.5,1.6,1.4,1.55,10\n";
        let mut f = CsvFeed::parse(text).unwrap();
        let b = f.next_bar().unwrap();
        assert_eq!(b, Bar { t: 1_700_000_000.0, open: 1.0, high: 2.0, low: 0.5, close: 1.5 });
        assert_eq!(f.next_bar().unwrap().t, 1_700_000_060.0);
        assert!(f.next_bar().is_none());

        let micros = CsvFeed::parse("1759276800000000,1,1,1,1\n1759276860000000,1,1,1,1\n").unwrap();
        assert_eq!(micros.bars[1].t, 1_759_276_860.0);

        let bars: Vec<Bar> = CsvFeed::parse(&to_csv(&[b, b])).unwrap().bars;
        assert_eq!(bars[0], b);
    }

    #[test]
    fn csv_rejects_too_short() {
        assert!(CsvFeed::parse("ts,o,h,l,c\n1,1,1,1,1\n").is_err());
    }
}
