//! Run statistics.

use crate::agents::AgentKind;

/// How well the model price, quoted at each band reset, predicted the realized
/// hitting time. Compared as a ratio of means: hitting times are right-skewed,
/// so log errors are biased even for a perfect model.
#[derive(Clone, Debug, Default)]
pub struct Calibration {
    pub n: u64,
    predicted: f64,
    realized: f64,
}

impl Calibration {
    pub fn record(&mut self, predicted: f64, realized: f64) {
        self.n += 1;
        self.predicted += predicted;
        self.realized += realized;
    }

    /// Σ realized / Σ predicted: above 1 means the model quotes too short.
    pub fn ratio(&self) -> f64 {
        if self.predicted > 0.0 { self.realized / self.predicted } else { 1.0 }
    }
}

#[derive(Clone, Debug)]
pub struct KindPnl {
    pub kind: AgentKind,
    pub agents: usize,
    /// Wallet plus open-position equity, minus starting wallet.
    pub pnl: f64,
}

#[derive(Clone, Debug)]
pub struct Summary {
    pub steps: u64,
    pub sim_days: f64,
    pub hits: u64,
    pub calibration: Calibration,
    pub lp_start: f64,
    pub lp_equity: f64,
    pub fees: f64,
    pub bad_debt: f64,
    pub liquidations: u64,
    /// Positions reduced by auto-deleveraging.
    pub adl: u64,
    pub trades: u64,
    /// Peak |long OI − short OI| / starting LP liquidity seen during the run.
    pub max_skew_ratio: f64,
    /// LP PnL from carrying net skew through mark moves: −Σ skew·Δln(mark).
    /// Exact for log-quoted positions; an approximation for linear ones.
    pub skew_pnl: f64,
    /// Mean |ln(target / mark)|: the half-spread worst-of pricing charges.
    pub mean_gap: f64,
    /// Fraction of bars where that gap exceeds 5%.
    pub wide_gap_frac: f64,
    pub by_kind: Vec<KindPnl>,
}

impl std::fmt::Display for Summary {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        writeln!(f, "steps          {} ({:.1} days)", self.steps, self.sim_days)?;
        writeln!(f, "band hits      {}", self.hits)?;
        writeln!(f, "calibration    realized/predicted {:.3} (n={})", self.calibration.ratio(), self.calibration.n)?;
        let lp = self.lp_equity - self.lp_start;
        writeln!(f, "LP PnL         {:+.2} ({:+.2}%)", lp, 100.0 * lp / self.lp_start)?;
        writeln!(f, "fees           {:.2}", self.fees)?;
        writeln!(f, "bad debt       {:.2}", self.bad_debt)?;
        writeln!(f, "trades         {}", self.trades)?;
        writeln!(f, "liquidations   {}", self.liquidations)?;
        writeln!(f, "ADL reductions {}", self.adl)?;
        writeln!(f, "max skew/LP0   {:.3}", self.max_skew_ratio)?;
        writeln!(f, "LP skew PnL    {:+.2}", self.skew_pnl)?;
        writeln!(
            f,
            "worst-of gap   mean {:.2}%  >5% on {:.1}% of bars",
            100.0 * self.mean_gap,
            100.0 * self.wide_gap_frac
        )?;
        writeln!(f, "trader PnL by kind:")?;
        for k in &self.by_kind {
            writeln!(f, "  {:<10} n={:<3} {:+.2}", format!("{:?}", k.kind), k.agents, k.pnl)?;
        }
        Ok(())
    }
}
