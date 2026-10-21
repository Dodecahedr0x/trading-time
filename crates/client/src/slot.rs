//! Slot-time estimate from `getRecentPerformanceSamples`, shared by the maker and keeper. Pure, wasm-safe.

pub const DEFAULT_SLOT_MS: u64 = 400;
pub const MIN_SLOT_MS: u64 = 50;
pub const MAX_SLOT_MS: u64 = 2_000;
pub const SAMPLE_LIMIT: usize = 5;
pub const ESTIMATE_REFRESH_MS: i64 = 60_000;
/// Bound on the slot-time RPC so a hung node never stalls a loop; on timeout the current period is kept.
pub const ESTIMATE_TIMEOUT_MS: u64 = 250;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PerfSample {
    pub num_slots: u64,
    pub period_secs: u64,
}

/// sum(period) / sum(slots), clamped to [MIN_SLOT_MS, MAX_SLOT_MS]; DEFAULT_SLOT_MS when there are no slots.
pub fn estimate_slot_ms(samples: &[PerfSample]) -> u64 {
    let slots: u64 = samples.iter().map(|s| s.num_slots).sum();
    let secs: u64 = samples.iter().map(|s| s.period_secs).sum();
    if slots == 0 {
        return DEFAULT_SLOT_MS;
    }
    (secs.saturating_mul(1_000) / slots).clamp(MIN_SLOT_MS, MAX_SLOT_MS)
}

/// Fewest slots between two readings for a measured slot time to be trusted.
pub const MIN_MEASURED_SLOTS: u64 = 10;
/// First re-estimate after start, so the measured rate replaces the sampled guess quickly.
pub const FIRST_MEASURE_MS: i64 = 5_000;

/// Slot time from the chain's own slot counter between two `(slot, unix_ms)` readings, clamped like
/// `estimate_slot_ms`. Preferred over performance samples, which local validators (Surfpool) synthesise.
pub fn measured_slot_ms(from: (u64, i64), to: (u64, i64)) -> Option<u64> {
    let slots = to.0.checked_sub(from.0)?;
    let ms = to.1 - from.1;
    (slots >= MIN_MEASURED_SLOTS && ms > 0).then(|| (ms as u64 / slots).clamp(MIN_SLOT_MS, MAX_SLOT_MS))
}

pub fn slot_ms_or_default<E>(r: Result<Vec<PerfSample>, E>) -> u64 {
    r.map_or(DEFAULT_SLOT_MS, |s| estimate_slot_ms(&s))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ps(num_slots: u64, period_secs: u64) -> PerfSample {
        PerfSample { num_slots, period_secs }
    }

    #[test]
    fn estimates_clamps_and_defaults() {
        assert_eq!(estimate_slot_ms(&[ps(300, 60)]), 200);
        assert_eq!(estimate_slot_ms(&[ps(150, 60), ps(300, 60)]), 266);
        assert_eq!(estimate_slot_ms(&[ps(60_000, 60)]), MIN_SLOT_MS);
        assert_eq!(estimate_slot_ms(&[ps(10, 60)]), MAX_SLOT_MS);
        assert_eq!(estimate_slot_ms(&[]), DEFAULT_SLOT_MS);
        assert_eq!(slot_ms_or_default(Err("method not found")), 400);
        assert_eq!((ESTIMATE_REFRESH_MS, ESTIMATE_TIMEOUT_MS), (60_000, 250));
        // Surfpool: 25 slots in 10 s while its samples claim 1 slot/s.
        assert_eq!(measured_slot_ms((100, 0), (125, 10_000)), Some(400));
        assert_eq!(measured_slot_ms((100, 0), (105, 10_000)), None);
        assert_eq!(measured_slot_ms((100, 0), (90, 10_000)), None);
        assert_eq!(measured_slot_ms((0, 0), (1_000, 10)), Some(MIN_SLOT_MS));
    }
}
