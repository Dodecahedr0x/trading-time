use super::format::duration;
use crate::types::CandleJson;

#[derive(Clone, Debug, PartialEq)]
pub struct CandleBar {
    pub x: f64,
    pub w: f64,
    pub wick_top: f64,
    pub wick_bottom: f64,
    pub body_top: f64,
    pub body_h: f64,
    /// close >= open: moves got slower (calm colour); else wild.
    pub calm: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Axis {
    pub y: f64,
    pub label: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ChartGeom {
    pub bars: Vec<CandleBar>,
    pub axis: Vec<Axis>,
    /// y of the last close
    pub last_y: Option<f64>,
}

const PAD: f64 = 0.08;
const BODY: f64 = 0.7;

/// Lays candles in a `width`×`height` box (SVG coordinates, y down) on a log-time scale with 8% padding.
/// Up to `ticks` evenly spaced axis labels (format::duration), top to bottom.
pub fn layout(candles: &[CandleJson], width: f64, height: f64, ticks: usize) -> ChartGeom {
    let valid: Vec<&CandleJson> =
        candles.iter().filter(|c| c.low_secs > 0.0 && c.high_secs > 0.0 && c.open_secs > 0.0 && c.close_secs > 0.0).collect();
    if valid.is_empty() {
        return ChartGeom { bars: vec![], axis: vec![], last_y: None };
    }
    let mut lo = valid.iter().map(|c| c.low_secs.min(c.open_secs).min(c.close_secs).ln()).fold(f64::INFINITY, f64::min);
    let mut hi = valid.iter().map(|c| c.high_secs.max(c.open_secs).max(c.close_secs).ln()).fold(f64::NEG_INFINITY, f64::max);
    if hi - lo < 1e-9 {
        lo -= 0.05;
        hi += 0.05;
    }
    let r = hi - lo;
    let (top, bottom) = (hi + PAD * r, lo - PAD * r);
    let y = |secs: f64| (top - secs.ln()) / (top - bottom) * height;
    let slot = width / valid.len() as f64;
    let bars = valid
        .iter()
        .enumerate()
        .map(|(i, c)| {
            let (yo, yc) = (y(c.open_secs), y(c.close_secs));
            CandleBar {
                x: i as f64 * slot + slot * (1.0 - BODY) / 2.0,
                w: slot * BODY,
                wick_top: y(c.high_secs.max(c.open_secs).max(c.close_secs)),
                wick_bottom: y(c.low_secs.min(c.open_secs).min(c.close_secs)),
                body_top: yo.min(yc),
                body_h: (yo - yc).abs().max(1.0),
                calm: c.close_secs >= c.open_secs,
            }
        })
        .collect();
    let axis = match ticks {
        0 => vec![],
        1 => vec![Axis { y: y(hi.exp()), label: duration(hi.exp()) }],
        n => (0..n)
            .map(|k| {
                let ln = hi - r * k as f64 / (n - 1) as f64;
                Axis { y: y(ln.exp()), label: duration(ln.exp()) }
            })
            .collect(),
    };
    ChartGeom { bars, axis, last_y: valid.last().map(|c| y(c.close_secs)) }
}

/// Full price-chart geometry (design "Chart" panel): candles right-aligned on a fixed slot grid,
/// the 7-day index as a path, the current mark line, and time labels along the bottom.
#[derive(Clone, Debug, PartialEq)]
pub struct Plot {
    pub bars: Vec<CandleBar>,
    pub grid: Vec<Axis>,
    /// (x, label)
    pub times: Vec<(f64, String)>,
    /// SVG path "M x y L x y …" of the index, empty when unknown.
    pub index_path: String,
    /// y of the latest close.
    pub now_y: Option<f64>,
    pub now_label: String,
}

/// Candles never get wider than this share of the plot, so a young market doesn't draw giant bodies.
pub const MIN_SLOTS: usize = 48;
const PLOT_BODY: f64 = 0.62;

fn clock(t: i64, iv_secs: i64) -> String {
    let ts = super::format::timestamp(t);
    // "10-20 14:03": dates for daily candles, clock time otherwise.
    if iv_secs >= 86_400 { ts[..5].to_string() } else { ts[6..].to_string() }
}

pub fn plot(candles: &[CandleJson], index: &[(i64, f64)], iv_secs: i64, width: f64, height: f64) -> Plot {
    let valid: Vec<&CandleJson> =
        candles.iter().filter(|c| c.low_secs > 0.0 && c.high_secs > 0.0 && c.open_secs > 0.0 && c.close_secs > 0.0).collect();
    let empty = Plot { bars: vec![], grid: vec![], times: vec![], index_path: String::new(), now_y: None, now_label: String::new() };
    let (Some(first), Some(last)) = (valid.first(), valid.last()) else { return empty };
    let iv = iv_secs.max(1);
    let slots = valid.len().max(MIN_SLOTS);
    let slot = width / slots as f64;
    let offset = (slots - valid.len()) as f64 * slot;
    let t0 = first.t - (slots - valid.len()) as i64 * iv;
    let t_end = last.t + iv;
    let x_of = |t: i64| (t - t0) as f64 / (t_end - t0) as f64 * width;
    // Carry the last index value from before the window in at its left edge, so a slow index still draws.
    let mut idx: Vec<(i64, f64)> = index.iter().copied().filter(|(t, v)| *v > 0.0 && *t >= t0 && *t <= t_end).collect();
    if let Some(&(_, v)) = index.iter().filter(|(t, v)| *v > 0.0 && *t < t0).max_by_key(|(t, _)| *t) {
        idx.insert(0, (t0, v));
    }
    if let (Some(&(_, v)), true) = (idx.last(), !idx.is_empty()) {
        idx.push((t_end, v));
    }

    let mut lo = valid.iter().map(|c| c.low_secs.ln()).fold(f64::INFINITY, f64::min);
    let mut hi = valid.iter().map(|c| c.high_secs.ln()).fold(f64::NEG_INFINITY, f64::max);
    for (_, v) in &idx {
        lo = lo.min(v.ln());
        hi = hi.max(v.ln());
    }
    if hi - lo < 1e-4 {
        lo -= 0.002;
        hi += 0.002;
    }
    let r = hi - lo;
    let (top, bottom) = (hi + PAD * r, lo - PAD * r);
    let y = |secs: f64| (top - secs.ln()) / (top - bottom) * height;
    let bars = valid
        .iter()
        .enumerate()
        .map(|(i, c)| {
            let (yo, yc) = (y(c.open_secs), y(c.close_secs));
            CandleBar {
                x: offset + i as f64 * slot + slot * (1.0 - PLOT_BODY) / 2.0,
                w: (slot * PLOT_BODY).max(1.5),
                wick_top: y(c.high_secs),
                wick_bottom: y(c.low_secs),
                body_top: yo.min(yc),
                body_h: (yo - yc).abs().max(1.0),
                calm: c.close_secs >= c.open_secs,
            }
        })
        .collect();
    let levels: Vec<f64> = (0..4).map(|k| (hi - r * (k as f64 + 0.5) / 4.0).exp()).collect();
    // Tight ranges repeat at minute precision; fall back to h:mm:ss.
    let mut short: Vec<String> = levels.iter().map(|v| duration(*v)).collect();
    short.dedup();
    let fmt: fn(f64) -> String = if short.len() < levels.len() { super::format::hms } else { duration };
    let grid = levels.iter().map(|v| Axis { y: y(*v), label: fmt(*v) }).collect();
    let times = (0..6)
        .map(|k| {
            let t = t0 + (t_end - t0) * (2 * k + 1) / 12;
            let t = t - t.rem_euclid(iv);
            (x_of(t), clock(t, iv))
        })
        .collect();
    let index_path = idx
        .iter()
        .enumerate()
        .map(|(i, (t, v))| format!("{}{:.1} {:.1}", if i == 0 { "M" } else { " L" }, x_of(*t), y(*v)))
        .collect();
    Plot { bars, grid, times, index_path, now_y: Some(y(last.close_secs)), now_label: duration(last.close_secs) }
}

#[cfg(test)]
mod plot_tests {
    use super::*;

    fn c(t: i64, o: f64, h: f64, l: f64, cl: f64) -> CandleJson {
        CandleJson { t, open_secs: o, high_secs: h, low_secs: l, close_secs: cl, volume_usdc: 0.0 }
    }

    #[test]
    fn few_candles_stay_narrow_and_right_aligned() {
        let cs = vec![c(0, 100.0, 110.0, 95.0, 105.0), c(60, 105.0, 106.0, 90.0, 92.0)];
        let p = plot(&cs, &[], 60, 480.0, 300.0);
        let slot = 480.0 / MIN_SLOTS as f64;
        assert!((p.bars[0].w - slot * 0.62).abs() < 1e-9);
        assert!(p.bars[1].x + p.bars[1].w <= 480.0 && p.bars[1].x > 480.0 - slot);
        assert!(p.bars[0].calm && !p.bars[1].calm);
        assert_eq!(p.now_label, duration(92.0));
        assert_eq!((p.grid.len(), p.times.len()), (4, 6));
    }

    #[test]
    fn index_path_and_range_include_the_index() {
        let cs: Vec<_> = (0..60).map(|i| c(i * 60, 100.0, 101.0, 99.0, 100.0)).collect();
        let idx = vec![(0, 150.0), (1_800, 140.0), (99_999, 1.0)];
        let p = plot(&cs, &idx, 60, 600.0, 300.0);
        // Two in-window points plus the carried-forward end point.
        assert!(p.index_path.starts_with("M0.0 ") && p.index_path.matches(" L").count() == 2, "{}", p.index_path);
        // The index (above the candles) sets the top of the range.
        assert!(p.bars.iter().all(|b| b.wick_top > 20.0));
        assert!(plot(&[], &idx, 60, 600.0, 300.0).bars.is_empty());
    }

    #[test]
    fn time_labels_follow_interval() {
        let cs: Vec<_> = (0..10).map(|i| c(1_792_500_000 + i * 86_400, 100.0, 101.0, 99.0, 100.0)).collect();
        assert!(plot(&cs, &[], 86_400, 600.0, 300.0).times[5].1.contains('-'));
        assert!(plot(&cs, &[], 3_600, 600.0, 300.0).times[5].1.contains(':'));
    }
}

#[cfg(test)]
mod plot_more {
    use super::*;

    #[test]
    fn index_before_window_is_carried_in_and_tight_grid_uses_hms() {
        let cs = vec![CandleJson { t: 6_000, open_secs: 5_400.0, high_secs: 5_410.0, low_secs: 5_395.0, close_secs: 5_405.0, volume_usdc: 0.0 }];
        let p = plot(&cs, &[(10, 5_460.0)], 60, 480.0, 300.0);
        assert!(p.index_path.starts_with("M0.0 ") && p.index_path.contains(" L480.0 "), "{}", p.index_path);
        assert!(p.grid.iter().all(|a| a.label.contains(':')), "{:?}", p.grid);
    }
}
