use pinocchio::{AccountView, ProgramResult};
use tradingtime_interface::{
    CountData, PodI128, TtError,
    events::{self, FundingEvent, IndexEvent},
    pyth,
};
use tradingtime_math::{funding, index, ln_ratio};

use crate::{
    payload,
    trade::{distinct_accounts, load_pairs, refresh_mark},
    util::*,
};

fn rd8(d: &[u8], at: usize) -> [u8; 8] {
    let mut b = [0u8; 8];
    b.copy_from_slice(&d[at..at + 8]);
    b
}

pub fn crank_index(a: &[AccountView]) -> ProgramResult {
    let [market_acc, feed] = accounts::<2>(a)?;
    let m = market_mut(market_acc, true)?;
    ensure(key_eq(feed, &m.pyth_feed), TtError::InvalidAccount)?;
    ensure(feed.owned_by(&PYTH_RECEIVER), TtError::InvalidOwner)?;
    // SAFETY: read-only view of a foreign account for the duration of this instruction.
    let d = unsafe { core::slice::from_raw_parts(feed.data_ptr(), feed.data_len()) };
    let valid = d.len() >= pyth::LEN
        && d[..8] == pyth::DISCRIMINATOR
        && d[pyth::VERIFICATION_TAG] == pyth::VERIFICATION_FULL
        && d[pyth::FEED_ID..pyth::FEED_ID + 32] == tradingtime_interface::PYTH_SOL_USD_FEED_ID;
    ensure(valid, TtError::PythInvalid)?;
    let price = i64::from_le_bytes(rd8(d, pyth::PRICE));
    let mut e = [0u8; 4];
    e.copy_from_slice(&d[pyth::EXPONENT..pyth::EXPONENT + 4]);
    let expo = i32::from_le_bytes(e);
    let publish = i64::from_le_bytes(rd8(d, pyth::PUBLISH_TIME));
    ensure(price > 0, TtError::PythInvalid)?;
    ensure(publish > m.last_publish_time, TtError::StaleOrSamePrice)?;
    let (_, ts) = now()?;
    ensure(ts - publish <= m.params.pyth_max_staleness_secs as i64, TtError::PythStale)?;

    if m.last_publish_time > 0 && m.last_pyth_price > 0 && m.last_pyth_expo == expo {
        let p = m.params;
        let r = ovf(ln_ratio(price, m.last_pyth_price))?;
        let dt = publish - m.last_publish_time;
        let var = ovf(index::ewma_update(m.var_ewma.get(), r, dt, p.index_half_life_secs))?;
        m.var_ewma = PodI128::new(var);
        let idx = ovf(index::index_ln(var, p.calib.get(), p.band_bps, p.index_min_secs, p.index_max_secs))?;
        m.index_ln = PodI128::new(idx);
        m.index_updated_ts = ts;
    }
    m.last_pyth_price = price;
    m.last_pyth_expo = expo;
    m.last_publish_time = publish;
    emit(
        events::TAG_INDEX,
        &IndexEvent {
            index_ln: m.index_ln,
            var_ewma: m.var_ewma,
            pyth_price: price,
            pyth_expo: expo,
            _pad: 0,
            publish_time: publish,
            ts,
        },
    );
    Ok(())
}

pub fn crank_funding(a: &[AccountView]) -> ProgramResult {
    let [market_acc] = accounts::<1>(a)?;
    let m = market_mut(market_acc, true)?;
    let (_, ts) = now()?;
    let dt = ts - m.funding_updated_ts;
    if dt <= 0 {
        return Ok(());
    }
    let mark = m.effective_mark(ts);
    let idx = m.index_ln.get();
    let fi = ovf(funding::accrue(m.funding_index.get(), mark, idx, dt, m.params.funding_period_secs))?;
    m.funding_index = PodI128::new(fi);
    m.funding_updated_ts = ts;
    emit(
        events::TAG_FUNDING,
        &FundingEvent { funding_index: m.funding_index, mark_ln: PodI128::new(mark), index_ln: m.index_ln, ts },
    );
    Ok(())
}

pub fn crank_mark(a: &[AccountView], data: &[u8]) -> ProgramResult {
    let d: CountData = payload(data)?;
    let n = d.n_ops as usize;
    ensure(n <= tradingtime_interface::MAX_OPERATORS_PER_IX, TtError::TooManyAccounts)?;
    let [market_acc] = accounts::<1>(a)?;
    let pair_accs = a.get(1..1 + 2 * n).ok_or(err(TtError::InvalidInstruction))?;
    distinct_accounts(&[market_acc], pair_accs)?;
    let m = market_mut(market_acc, true)?;
    let (slot, ts) = now()?;
    let p = load_pairs(pair_accs, n, m, ts, false)?;
    ensure(refresh_mark(&p, m, slot, ts)?, TtError::NoFreshCurves)
}
