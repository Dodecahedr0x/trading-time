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
