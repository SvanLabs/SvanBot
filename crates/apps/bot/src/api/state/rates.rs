//! Chip totals and blind-normalized rates for the dashboard's stored hand results.

use super::*;

/// Net chips, win rate and the running curve over stored results. Chip totals include every
/// settled hand; a rate needs that hand's own positive blind, never the current table's stake.
pub(super) fn summarize(rows: &[ResultRow]) -> (Value, Vec<Value>) {
    let (mut priced, mut sum, mut sq) = (0f64, 0f64, 0f64);
    let (mut ev_sum, mut ev_sq) = (0f64, 0f64);
    let (mut total, mut ev_total, mut showdown, mut other) = (0i64, 0f64, 0i64, 0i64);
    let mut series = Vec::new();
    let step = (rows.len() / 150).max(1);
    for (i, row) in rows.iter().enumerate() {
        let Some(net) = row.net else { continue };
        let ev = row.ev_net.unwrap_or(net as f64);
        total += net;
        ev_total += ev;
        if row.showdown {
            showdown += net;
        } else {
            other += net;
        }
        if let Some(bb) = row.big_blind.filter(|bb| *bb > 0) {
            let net_bb = net as f64 / bb as f64;
            let ev_bb = ev / bb as f64;
            priced += 1.0;
            sum += net_bb;
            sq += net_bb * net_bb;
            ev_sum += ev_bb;
            ev_sq += ev_bb * ev_bb;
        }
        if i % step == 0 || i + 1 == rows.len() {
            series.push(json!({"hand": i + 1, "total": total, "showdown": showdown, "other": other, "ev": ev_total.round()}));
        }
    }
    let rate = |sum: f64, sq: f64| {
        if priced >= 2.0 {
            let mean = sum / priced;
            let var = (sq / priced - mean * mean).max(0.0);
            (Some(mean * 100.0), Some(1.96 * (var / priced).sqrt() * 100.0))
        } else {
            (None, None)
        }
    };
    let (bb100, conf) = rate(sum, sq);
    let (ev_bb100, ev_conf) = rate(ev_sum, ev_sq);
    (
        json!({"hands": rows.len(), "priced_hands": priced as i64, "net_chips": total, "bb100": bb100, "confidence": conf,
            "ev_net_chips": ev_total.round(), "ev_bb100": ev_bb100, "ev_confidence": ev_conf, "luck_chips": (total as f64 - ev_total).round()}),
        series,
    )
}
