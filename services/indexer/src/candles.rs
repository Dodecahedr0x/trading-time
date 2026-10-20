use serde::Serialize;

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Candle {
    pub t: i64,
    pub open_secs: f64,
    pub high_secs: f64,
    pub low_secs: f64,
    pub close_secs: f64,
    pub volume_usdc: f64,
}

pub fn interval_secs(s: &str) -> Option<i64> {
    match s {
        "1h" => Some(3_600),
        "4h" => Some(14_400),
        "1d" => Some(86_400),
        _ => None,
    }
}

/// Buckets ln-time `points` (ascending by ts) into OHLC candles in seconds. Empty buckets
/// between the first point and `now` repeat the previous close; returns the last `limit`.
pub fn build(points: &[(i64, f64)], trades: &[(i64, u64)], interval: i64, limit: usize, now: i64) -> Vec<Candle> {
    let Some(&(first_ts, _)) = points.first() else { return Vec::new() };
    let bucket = |ts: i64| ts.div_euclid(interval) * interval;
    let last = bucket(now.max(points.last().map(|p| p.0).unwrap_or(now)));
    let start = bucket(first_ts).max(last - interval * (limit as i64 - 1));
    let mut out: Vec<Candle> = Vec::new();
    let mut prev_close = None;
    let mut i = 0;
    while i < points.len() && bucket(points[i].0) < start {
        prev_close = Some(points[i].1.exp());
        i += 1;
    }
    let mut t = start;
    while t <= last {
        let mut c: Option<Candle> = prev_close.map(|p| Candle {
            t,
            open_secs: p,
            high_secs: p,
            low_secs: p,
            close_secs: p,
            volume_usdc: 0.0,
        });
        while i < points.len() && bucket(points[i].0) == t {
            let v = points[i].1.exp();
            c = Some(match c {
                None => Candle { t, open_secs: v, high_secs: v, low_secs: v, close_secs: v, volume_usdc: 0.0 },
                Some(mut c) => {
                    c.high_secs = c.high_secs.max(v);
                    c.low_secs = c.low_secs.min(v);
                    c.close_secs = v;
                    c
                }
            });
            i += 1;
        }
        if let Some(mut c) = c {
            c.volume_usdc = trades.iter().filter(|(ts, _)| bucket(*ts) == t).map(|(_, s)| *s as f64 / 1e6).sum();
            prev_close = Some(c.close_secs);
            out.push(c);
        }
        t += interval;
    }
    let skip = out.len().saturating_sub(limit);
    out.split_off(skip)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-6 * b.abs().max(1.0)
    }

    #[test]
    fn buckets_ohlc_volume_and_forward_fills() {
        let ln = |s: f64| s.ln();
        let pts = vec![(3_600, ln(100.0)), (3_700, ln(300.0)), (4_000, ln(50.0)), (7_100, ln(200.0)), (14_500, ln(80.0))];
        let trades = vec![(3_650, 2_000_000), (3_700, 1_000_000), (14_500, 500_000)];
        let c = build(&pts, &trades, 3_600, 500, 14_500);
        assert_eq!(c.iter().map(|c| c.t).collect::<Vec<_>>(), vec![3_600, 7_200, 10_800, 14_400]);
        assert!(close(c[0].open_secs, 100.0) && close(c[0].high_secs, 300.0));
        assert!(close(c[0].low_secs, 50.0) && close(c[0].close_secs, 200.0));
        assert!(close(c[0].volume_usdc, 3.0));
        assert!(close(c[1].open_secs, 200.0) && close(c[1].close_secs, 200.0));
        assert!(close(c[2].open_secs, c[1].close_secs) && c[2].volume_usdc == 0.0);
        assert!(close(c[3].close_secs, 80.0) && close(c[3].volume_usdc, 0.5));

        let last2 = build(&pts, &trades, 3_600, 2, 14_500);
        assert_eq!(last2.len(), 2);
        assert_eq!(last2[1].t, 14_400);
        assert!(close(last2[0].open_secs, c[2].open_secs));

        let day = build(&pts, &trades, 86_400, 10, 14_500);
        assert_eq!(day.len(), 1);
        assert!(close(day[0].high_secs, 300.0) && close(day[0].close_secs, 80.0));
        assert!(build(&[], &[], 3_600, 10, 0).is_empty());
        assert_eq!(interval_secs("4h"), Some(14_400));
        assert_eq!(interval_secs("5m"), None);
    }
}
