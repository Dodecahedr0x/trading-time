//! Fixed-point math shared by the TradingTime program, simulator, services and app.
//!
//! `Fx` is a signed 128-bit fixed-point number with 48 fractional bits. Prices are
//! ln-time: the natural log of the expected seconds for SOL to move ±band.
#![cfg_attr(not(test), no_std)]

pub type Fx = i128;
pub const FRAC_BITS: u32 = 48;
pub const ONE: Fx = 1 << FRAC_BITS;
pub const LN2: Fx = 195_103_586_505_167;
const SQRT2: Fx = 398_065_729_532_861;
/// Base units per whole USDC.
pub const USDC: i128 = 1_000_000;

#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Side {
    /// Taker exposure increases: buys long time (gains when moves slow down).
    Long = 0,
    /// Taker exposure decreases: buys short time (gains when moves speed up).
    Short = 1,
}

impl Side {
    pub fn from_u8(v: u8) -> Option<Side> {
        match v {
            0 => Some(Side::Long),
            1 => Some(Side::Short),
            _ => None,
        }
    }

    pub fn sign(self) -> i64 {
        match self {
            Side::Long => 1,
            Side::Short => -1,
        }
    }
}

/// `a * b >> shift` with a 256-bit intermediate.
fn mul_shr(a: i128, b: i128, shift: u32) -> Option<i128> {
    let neg = (a < 0) != (b < 0);
    let (ua, ub) = (a.unsigned_abs(), b.unsigned_abs());
    let (a1, a0) = (ua >> 64, ua & u64::MAX as u128);
    let (b1, b0) = (ub >> 64, ub & u64::MAX as u128);
    let lo = a0 * b0;
    let mid1 = a1 * b0;
    let mid2 = a0 * b1;
    let hi = a1 * b1;
    let (mid, mid_carry) = mid1.overflowing_add(mid2);
    let (lo, lo_carry) = lo.overflowing_add(mid << 64);
    let hi = hi + (mid >> 64) + ((mid_carry as u128) << 64) + lo_carry as u128;
    let r = if shift == 0 {
        if hi != 0 {
            return None;
        }
        lo
    } else {
        if hi >> shift != 0 {
            return None;
        }
        (lo >> shift) | (hi << (128 - shift))
    };
    if r > i128::MAX as u128 {
        return None;
    }
    let r = r as i128;
    Some(if neg { -r } else { r })
}

pub fn fx_mul(a: Fx, b: Fx) -> Option<Fx> {
    mul_shr(a, b, FRAC_BITS)
}

pub fn fx_div(a: Fx, b: Fx) -> Option<Fx> {
    if b == 0 {
        return None;
    }
    if a.unsigned_abs() < (1u128 << 78) {
        return Some((a << FRAC_BITS) / b);
    }
    if b.unsigned_abs() >= (1u128 << 78) {
        return a.checked_div(b >> FRAC_BITS);
    }
    let q = a / b;
    let r = a % b;
    q.checked_mul(ONE)?.checked_add((r << FRAC_BITS) / b)
}

pub fn floor_div(a: i128, b: i128) -> i128 {
    a.div_euclid(b)
}

pub fn ceil_div(a: i128, b: i128) -> i128 {
    -(-a).div_euclid(b)
}

pub fn ln_fx(x: Fx) -> Option<Fx> {
    if x <= 0 {
        return None;
    }
    let msb = 127 - x.leading_zeros() as i32;
    let mut k = msb - FRAC_BITS as i32;
    let mut m = if k >= 0 { x >> k } else { x << (-k) };
    if m > SQRT2 {
        m >>= 1;
        k += 1;
    }
    let z = fx_div(m - ONE, m + ONE)?;
    let z2 = fx_mul(z, z)?;
    let mut term = z;
    let mut sum = z;
    let mut n = 1i128;
    while n < 40 {
        term = fx_mul(term, z2)?;
        if term == 0 {
            break;
        }
        sum += term / (2 * n + 1);
        n += 1;
    }
    (k as i128).checked_mul(LN2)?.checked_add(2 * sum)
}

pub fn exp_fx(x: Fx) -> Option<Fx> {
    let k = floor_div(x + LN2 / 2, LN2);
    if k > 77 {
        return None;
    }
    if k < -127 {
        return Some(0);
    }
    let r = x - k * LN2;
    let mut term = ONE;
    let mut sum = ONE;
    let mut n = 1i128;
    while n < 30 {
        term = fx_mul(term, r)? / n;
        if term == 0 {
            break;
        }
        sum += term;
        n += 1;
    }
    Some(if k >= 0 { sum << k } else { sum >> (-k) })
}

/// ln(a / b) for positive integers.
pub fn ln_ratio(a: i64, b: i64) -> Option<Fx> {
    if a <= 0 || b <= 0 {
        return None;
    }
    Some(ln_fx((a as i128) << FRAC_BITS)? - ln_fx((b as i128) << FRAC_BITS)?)
}

pub fn from_f64(x: f64) -> Fx {
    (x * ONE as f64) as Fx
}

pub fn to_f64(x: Fx) -> f64 {
    x as f64 / ONE as f64
}

pub fn from_int(x: i64) -> Fx {
    (x as i128) << FRAC_BITS
}

pub mod index {
    use super::*;

    /// Folds one log return `r` observed over `dt_secs` into a per-second variance EWMA.
    pub fn ewma_update(var: Fx, r: Fx, dt_secs: i64, half_life_secs: u32) -> Option<Fx> {
        if dt_secs <= 0 || half_life_secs == 0 {
            return Some(var);
        }
        let decay = exp_fx(-fx_div(fx_mul(LN2, from_int(dt_secs))?, from_int(half_life_secs as i64))?)?;
        let alpha = ONE - decay;
        let inst = fx_mul(r, r)? / dt_secs as i128;
        var.checked_add(fx_mul(alpha, inst - var)?)
    }

    /// ln of the expected seconds to move ±band: ln(calib · ln(1+band)² / var), clamped.
    pub fn index_ln(var: Fx, calib: Fx, band_bps: u16, min_secs: u32, max_secs: u32) -> Option<Fx> {
        let lo = ln_fx(from_int(min_secs.max(1) as i64))?;
        let hi = ln_fx(from_int(max_secs.max(1) as i64))?;
        if var <= 0 {
            return Some(hi);
        }
        let x = ln_fx(ONE + (band_bps as i128) * ONE / 10_000)?;
        let v = ln_fx(calib)? + 2 * ln_fx(x)? - ln_fx(var)?;
        Some(v.clamp(lo, hi))
    }
}

pub mod position {
    use super::*;

    /// Netted log-quoted position. `cost` = Σ dq · ln_price (base units × Fx).
    #[derive(Clone, Copy, Debug, Default, PartialEq)]
    pub struct Position {
        pub cash: i64,
        pub q: i64,
        pub cost: i128,
        pub funding_snapshot: Fx,
    }

    /// Moves funding owed since the snapshot into cash. Returns the amount moved
    /// (negative = paid). Payers round up.
    pub fn settle_funding(p: &mut Position, funding_index: Fx) -> Option<i64> {
        let owed = (p.q as i128).checked_mul(funding_index.checked_sub(p.funding_snapshot)?)?;
        let moved = -ceil_div(owed, ONE);
        let moved = i64::try_from(moved).ok()?;
        p.cash = p.cash.checked_add(moved)?;
        p.funding_snapshot = funding_index;
        Some(moved)
    }

    pub fn apply_fill(p: &mut Position, dq: i64, ln_price: Fx) -> Option<()> {
        p.q = p.q.checked_add(dq)?;
        p.cost = p.cost.checked_add((dq as i128).checked_mul(ln_price)?)?;
        if p.q == 0 {
            let realized = i64::try_from(floor_div(-p.cost, ONE)).ok()?;
            p.cash = p.cash.checked_add(realized)?;
            p.cost = 0;
        }
        Some(())
    }

    /// Applies a fill whose notional is given as Σ size·ln_price (from a curve route).
    pub fn apply_fill_notional(p: &mut Position, dq: i64, ln_notional: i128) -> Option<()> {
        p.q = p.q.checked_add(dq)?;
        let signed = if dq >= 0 { ln_notional } else { -ln_notional };
        p.cost = p.cost.checked_add(signed)?;
        if p.q == 0 {
            let realized = i64::try_from(floor_div(-p.cost, ONE)).ok()?;
            p.cash = p.cash.checked_add(realized)?;
            p.cost = 0;
        }
        Some(())
    }

    /// Equity in base units at `mark_ln`; funding must already be settled.
    pub fn equity(p: &Position, mark_ln: Fx) -> Option<i64> {
        let upnl = (p.q as i128).checked_mul(mark_ln)?.checked_sub(p.cost)?;
        i64::try_from((p.cash as i128).checked_add(floor_div(upnl, ONE))?).ok()
    }

    pub fn maintenance(q: i64, maintenance_bps: u16) -> i64 {
        ceil_div(q.unsigned_abs() as i128 * maintenance_bps as i128, 10_000) as i64
    }

    pub fn max_exposure(equity: i64, max_leverage_bps: u16) -> i64 {
        (equity.max(0) as i128 * max_leverage_bps as i128 / 10_000) as i64
    }

    /// The ln price at which equity reaches zero.
    pub fn bankruptcy_ln(p: &Position) -> Option<Fx> {
        if p.q == 0 {
            return None;
        }
        let num = p.cost.checked_sub((p.cash as i128).checked_mul(ONE)?)?;
        Some(num / p.q as i128)
    }
}

pub mod funding {
    use super::*;

    /// funding_index += (mark − index) · dt / period (per unit of exposure).
    pub fn accrue(funding_index: Fx, mark_ln: Fx, index_ln: Fx, dt_secs: i64, period_secs: u32) -> Option<Fx> {
        if dt_secs <= 0 || period_secs == 0 {
            return Some(funding_index);
        }
        let delta = (mark_ln - index_ln).checked_mul(dt_secs as i128)? / period_secs as i128;
        funding_index.checked_add(delta)
    }
}

pub mod curve {
    use super::*;

    /// One operator's quote: marginal price fair ± (half_spread + impact · x), centred
    /// against the operator's inventory `q` (base units).
    #[derive(Clone, Copy, Debug, Default, PartialEq)]
    pub struct Curve {
        pub fair_ln: Fx,
        pub half_spread: Fx,
        /// ln per whole USDC of size.
        pub impact_per_usdc: Fx,
        pub inventory_limit: u64,
        pub q: i64,
    }

    #[derive(Clone, Copy, Debug, Default, PartialEq)]
    pub struct Fill {
        pub idx: u8,
        pub size: u64,
        /// Σ size · ln_price over this fill (base units × Fx).
        pub ln_notional: i128,
    }

    #[derive(Clone, Copy, Debug, Default, PartialEq)]
    pub struct Route {
        pub fills: [Fill; 8],
        pub n: u8,
        pub total: u64,
        pub ln_notional: i128,
    }

    impl Route {
        pub fn fills(&self) -> &[Fill] {
            &self.fills[..self.n as usize]
        }
    }

    fn impact_at(c: &Curve, x: u64) -> Option<Fx> {
        c.impact_per_usdc.checked_mul(x as i128).map(|v| v / USDC)
    }

    pub fn center(c: &Curve) -> Option<Fx> {
        c.fair_ln.checked_sub(c.impact_per_usdc.checked_mul(c.q as i128)? / USDC)
    }

    /// Marginal price for the taker at cumulative size `x` on `side`.
    pub fn marginal(c: &Curve, side: Side, x: u64) -> Option<Fx> {
        let off = c.half_spread.checked_add(impact_at(c, x)?)?;
        match side {
            Side::Long => center(c)?.checked_add(off),
            Side::Short => center(c)?.checked_sub(off),
        }
    }

    /// How much the operator can take on for a taker on `side`.
    pub fn room(c: &Curve, side: Side) -> u64 {
        let lim = c.inventory_limit as i128;
        let r = match side {
            Side::Long => lim + c.q as i128,
            Side::Short => lim - c.q as i128,
        };
        r.clamp(0, u64::MAX as i128) as u64
    }

    /// Σ size·price for filling [0, s) on one curve.
    pub fn integral(c: &Curve, side: Side, s: u64) -> Option<i128> {
        let s128 = s as i128;
        let base = match side {
            Side::Long => center(c)?.checked_add(c.half_spread)?,
            Side::Short => center(c)?.checked_sub(c.half_spread)?,
        };
        let quad = c.impact_per_usdc.checked_mul(s128)? / USDC;
        let quad = quad.checked_mul(s128)? / 2;
        let lin = base.checked_mul(s128)?;
        match side {
            Side::Long => lin.checked_add(quad),
            Side::Short => lin.checked_sub(quad),
        }
    }

    const CHUNKS: u64 = 64;

    fn better(side: Side, a: Fx, b: Fx) -> bool {
        match side {
            Side::Long => a < b,
            Side::Short => a > b,
        }
    }

    fn within(side: Side, m: Fx, limit: Fx) -> bool {
        match side {
            Side::Long => m <= limit,
            Side::Short => m >= limit,
        }
    }

    /// Fills `size` across curves by best marginal price, in 1/64 chunks, never at a
    /// marginal price beyond `limit_ln`. At most 8 curves.
    pub fn route(curves: &[Curve], side: Side, size: u64, limit_ln: Fx) -> Option<Route> {
        if curves.len() > 8 {
            return None;
        }
        let n = curves.len();
        let mut x = [0u64; 8];
        let mut done = [false; 8];
        let chunk = (size / CHUNKS).max(1);
        let mut remaining = size;
        while remaining > 0 {
            let mut best: Option<(usize, Fx)> = None;
            for i in 0..n {
                if done[i] {
                    continue;
                }
                if x[i] >= room(&curves[i], side) {
                    done[i] = true;
                    continue;
                }
                let m = marginal(&curves[i], side, x[i])?;
                if !within(side, m, limit_ln) {
                    done[i] = true;
                    continue;
                }
                if best.is_none_or(|(_, bm)| better(side, m, bm)) {
                    best = Some((i, m));
                }
            }
            let Some((i, m)) = best else { break };
            let c = &curves[i];
            let mut amount = chunk.min(remaining).min(room(c, side) - x[i]);
            if c.impact_per_usdc > 0 {
                let headroom = match side {
                    Side::Long => limit_ln - m,
                    Side::Short => m - limit_ln,
                };
                let to_limit = headroom.checked_mul(USDC)? / c.impact_per_usdc;
                if to_limit < amount as i128 {
                    amount = to_limit.max(0) as u64;
                    done[i] = true;
                }
            }
            if amount == 0 {
                done[i] = true;
                continue;
            }
            x[i] += amount;
            remaining -= amount;
        }
        let mut route = Route::default();
        for i in 0..n {
            if x[i] == 0 {
                continue;
            }
            let ln_notional = integral(&curves[i], side, x[i])?;
            route.fills[route.n as usize] = Fill { idx: i as u8, size: x[i], ln_notional };
            route.n += 1;
            route.total += x[i];
            route.ln_notional = route.ln_notional.checked_add(ln_notional)?;
        }
        Some(route)
    }

    pub fn median_center(curves: &[Curve]) -> Option<Fx> {
        if curves.is_empty() || curves.len() > 8 {
            return None;
        }
        let mut c = [0 as Fx; 8];
        let n = curves.len();
        for i in 0..n {
            c[i] = center(&curves[i])?;
        }
        let s = &mut c[..n];
        for i in 1..n {
            let mut j = i;
            while j > 0 && s[j - 1] > s[j] {
                s.swap(j - 1, j);
                j -= 1;
            }
        }
        Some(if n % 2 == 1 { s[n / 2] } else { (s[n / 2 - 1] + s[n / 2]) / 2 })
    }
}

#[cfg(test)]
mod tests {
    use super::curve::*;
    use super::position::*;
    use super::*;

    fn close(a: f64, b: f64, tol: f64) -> bool {
        (a - b).abs() <= tol
    }

    #[test]
    fn mul_div_round_trip_and_wide_values() {
        let a = from_f64(1234.5678);
        let b = from_f64(0.000123);
        assert!(close(to_f64(fx_mul(a, b).unwrap()), 1234.5678 * 0.000123, 1e-10));
        let want = to_f64(a) / to_f64(b);
        assert!(close(to_f64(fx_div(a, b).unwrap()) / want, 1.0, 1e-12));
        let big = from_f64(3.0e10);
        assert!(close(to_f64(fx_mul(big, big).unwrap()) / 9.0e20, 1.0, 1e-12));
        assert_eq!(fx_div(ONE, 0), None);
        assert!(close(to_f64(fx_div(from_f64(-7.0), from_f64(2.0)).unwrap()), -3.5, 1e-12));
    }

    #[test]
    fn ln_matches_f64() {
        for &x in &[1e-9, 1e-6, 0.001, 0.5, std::f64::consts::FRAC_1_SQRT_2, 1.0, 1.0001, std::f64::consts::SQRT_2, 2.0, 3.0, 10.0, 5400.0, 604800.0, 1e15] {
            let fx = from_f64(x);
            let want = to_f64(fx).ln();
            let got = to_f64(ln_fx(fx).unwrap());
            assert!(close(got, want, 1e-12), "ln({x}) = {got}, want {want}");
        }
        assert_eq!(ln_fx(0), None);
        assert_eq!(ln_fx(-ONE), None);
    }

    #[test]
    fn exp_matches_f64_and_inverts_ln() {
        for &x in &[-30.0, -5.0, -0.5, -1e-6, 0.0, 1e-6, 0.3, 1.0, 5.0, 8.6, 13.3, 30.0] {
            let got = to_f64(exp_fx(from_f64(x)).unwrap());
            let want = x.exp();
            assert!(close(got / want, 1.0, 1e-11) || close(got, want, 1e-13), "exp({x}) = {got}, want {want}");
        }
        let v = from_f64(5400.0);
        assert!(close(to_f64(exp_fx(ln_fx(v).unwrap()).unwrap()), 5400.0, 1e-8));
        assert_eq!(exp_fx(from_f64(60.0)), None);
    }

    #[test]
    fn ln_ratio_of_prices() {
        let r = to_f64(ln_ratio(15_012_345_678, 15_000_000_000).unwrap());
        assert!(close(r, (15_012_345_678f64 / 15_000_000_000f64).ln(), 1e-13));
        assert_eq!(ln_ratio(0, 1), None);
    }

    #[test]
    fn index_from_variance_matches_formula() {
        let sigma: f64 = 0.8 / (365.0f64 * 86_400.0).sqrt();
        let var = from_f64(sigma * sigma);
        let got = to_f64(index::index_ln(var, from_f64(1.0), 100, 60, 604_800).unwrap()).exp();
        let want = 1.01f64.ln().powi(2) / (sigma * sigma);
        assert!(close(got / want, 1.0, 1e-6), "{got} vs {want}");
        let clamped = to_f64(index::index_ln(from_f64(1.0), ONE, 100, 60, 604_800).unwrap()).exp();
        assert!(close(clamped, 60.0, 1e-6));
        assert!(close(to_f64(index::index_ln(0, ONE, 100, 60, 604_800).unwrap()).exp(), 604_800.0, 1e-3));
    }

    #[test]
    fn ewma_halves_toward_new_sample_after_one_half_life() {
        let var = from_f64(1e-8);
        let r = from_f64((3e-8f64 * 3600.0).sqrt());
        let v = to_f64(index::ewma_update(var, r, 3600, 3600).unwrap());
        assert!(close(v, 2e-8, 1e-12), "{v}");
        assert_eq!(index::ewma_update(var, r, 0, 3600), Some(var));
    }

    #[test]
    fn fill_round_trip_realizes_log_pnl() {
        let mut p = Position { cash: 10_000_000_000, ..Default::default() };
        let entry = ln_fx(from_int(5400)).unwrap();
        apply_fill(&mut p, 5_000_000_000, entry).unwrap();
        let mark = ln_fx(from_int(10_800)).unwrap();
        let eq = equity(&p, mark).unwrap();
        let want = 10_000_000_000f64 + 5_000_000_000f64 * 2f64.ln();
        assert!(close(eq as f64, want, 2.0), "{eq}");
        apply_fill(&mut p, -5_000_000_000, mark).unwrap();
        assert_eq!(p.q, 0);
        assert_eq!(p.cost, 0);
        assert!(close(p.cash as f64, want, 2.0));
    }

    #[test]
    fn funding_moves_cash_and_payers_round_up() {
        let mut p = Position { cash: 1_000, q: 1_000_000_000, ..Default::default() };
        let f = funding::accrue(0, from_f64(0.02), 0, 86_400, 86_400).unwrap();
        assert!(close(to_f64(f), 0.02, 1e-12));
        let moved = settle_funding(&mut p, f).unwrap();
        assert_eq!(moved, -20_000_000);
        assert_eq!(p.cash, 1_000 - 20_000_000);
        assert_eq!(settle_funding(&mut p, f), Some(0));
        let mut s = Position { q: -1, ..Default::default() };
        assert_eq!(settle_funding(&mut s, from_f64(0.5)), Some(0));
    }

    #[test]
    fn bankruptcy_price_zeroes_equity() {
        let mut p = Position { cash: 1_000_000_000, ..Default::default() };
        apply_fill(&mut p, 1_000_000_000, ln_fx(from_int(5400)).unwrap()).unwrap();
        let b = bankruptcy_ln(&p).unwrap();
        assert!(equity(&p, b).unwrap().abs() <= 1);
        assert!(close(to_f64(b).exp(), 5400.0 / 1f64.exp(), 1e-6));
        assert_eq!(maintenance(-1_000_000, 500), 50_000);
        assert_eq!(max_exposure(-5, 10_000), 0);
        assert_eq!(max_exposure(2_000, 10_000), 2_000);
    }

    fn curve(fair_secs: f64, h: f64, impact_per_10k: f64, limit_usdc: i64, q_usdc: i64) -> Curve {
        Curve {
            fair_ln: from_f64(fair_secs.ln()),
            half_spread: from_f64(h),
            impact_per_usdc: from_f64(impact_per_10k / 10_000.0),
            inventory_limit: (limit_usdc * 1_000_000) as u64,
            q: q_usdc * 1_000_000,
        }
    }

    #[test]
    fn single_curve_matches_closed_form() {
        let c = curve(5400.0, 0.0002, 0.001, 50_000, 0);
        let size = 5_000_000_000u64;
        let r = route(&[c], Side::Long, size, from_f64(100.0)).unwrap();
        assert_eq!(r.total, size);
        let avg = r.ln_notional as f64 / size as f64 / ONE as f64;
        let want = to_f64(c.fair_ln) + to_f64(c.half_spread) + to_f64(c.impact_per_usdc) * 5_000.0 / 2.0;
        assert!(close(avg, want, 1e-12), "{avg} vs {want}");
        let s = route(&[c], Side::Short, size, from_f64(-100.0)).unwrap();
        let avg_s = s.ln_notional as f64 / size as f64 / ONE as f64;
        assert!(close(avg_s, to_f64(c.fair_ln) - to_f64(c.half_spread) - to_f64(c.impact_per_usdc) * 2_500.0, 1e-12));
    }

    #[test]
    fn route_equalizes_marginals_and_respects_room_and_limit() {
        let a = curve(5400.0, 0.0002, 0.001, 50_000, 0);
        let b = curve(5400.0, 0.0002, 0.001, 50_000, 0);
        let r = route(&[a, b], Side::Long, 10_000_000_000, from_f64(100.0)).unwrap();
        assert_eq!(r.n, 2);
        let diff = r.fills[0].size as i64 - r.fills[1].size as i64;
        assert!(diff.abs() <= 10_000_000_000i64 / 64, "{:?}", r.fills());
        let tight = curve(5400.0, 0.0002, 0.001, 1_000, 0);
        let r = route(&[tight], Side::Long, 5_000_000_000, from_f64(100.0)).unwrap();
        assert_eq!(r.total, 1_000_000_000);
        let lim = from_f64(5400f64.ln() + 0.0002 + 0.0005);
        let r = route(&[a], Side::Long, 50_000_000_000, lim).unwrap();
        assert!(close(r.total as f64, 5_000_000_000.0, 1_000.0), "{}", r.total);
        let none = route(&[a], Side::Long, 1_000_000, from_f64(5400f64.ln())).unwrap();
        assert_eq!(none.total, 0);
        assert!(route(&[a; 9], Side::Long, 1, 0).is_none());
    }

    #[test]
    fn inventory_skews_center_and_room() {
        let flat = curve(5400.0, 0.0, 0.001, 50_000, 0);
        let long = curve(5400.0, 0.0, 0.001, 50_000, 20_000);
        let want = -to_f64(long.impact_per_usdc) * 20_000.0;
        assert!(close(to_f64(center(&long).unwrap() - center(&flat).unwrap()), want, 1e-12));
        assert_eq!(room(&long, Side::Long), 70_000_000_000);
        assert_eq!(room(&long, Side::Short), 30_000_000_000);
        assert!(marginal(&long, Side::Long, 0).unwrap() < marginal(&flat, Side::Long, 0).unwrap());
    }

    #[test]
    fn median_of_centers() {
        let cs = [curve(5000.0, 0.0, 0.0, 1, 0), curve(6000.0, 0.0, 0.0, 1, 0), curve(5500.0, 0.0, 0.0, 1, 0)];
        assert!(close(to_f64(median_center(&cs).unwrap()).exp(), 5500.0, 1e-6));
        let even = [cs[0], cs[1]];
        assert!(close(to_f64(median_center(&even).unwrap()), (5000f64.ln() + 6000f64.ln()) / 2.0, 1e-12));
        assert_eq!(median_center(&[]), None);
    }
}
