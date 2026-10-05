//! Corpus import: replay exports into opponent models (0260).

use super::db::HistoryDb;
use crate::live::Shared;
use anyhow::Result;
use sv10_core::history::{RawHand, attribute_names, to_summary};
use sv10_core::model::{ANON_PREFIX, HandSummary, ModelStore};

/// Recency weight: evidence halves every 30 days, never below a quarter.
fn age_weight(started_at: &str, now: chrono::DateTime<chrono::Utc>) -> f32 {
    let Ok(t) = chrono::DateTime::parse_from_rfc3339(started_at) else { return 0.25 };
    let days = (now - t.with_timezone(&chrono::Utc)).num_seconds().max(0) as f64 / 86400.0;
    0.5f64.powf(days / 30.0).max(0.25) as f32
}

/// What one import pass did. Counts describe the pass; `merged` says whether they reached the models.
#[derive(Debug, Default)]
pub struct ImportReport {
    /// Hands folded into the delta model.
    pub hands: usize,
    pub named_seats: usize,
    pub anon_seats: usize,
    /// Hands already recorded live (not double-counted).
    pub skipped_live: usize,
    /// Parsed hands that could not be turned into a summary.
    pub failed: usize,
    /// Rows in range whose stored JSON no longer parses (an export format change shows up here).
    pub unparseable: usize,
    /// False when a model reset during the pass discarded the delta (the next pass re-imports).
    pub merged: bool,
}

impl ImportReport {
    /// Whether unreadable rows dominate the pass: likely a server format change, not noise.
    pub fn format_alarm(&self) -> bool {
        let unreadable = self.unparseable + self.failed;
        unreadable >= 20 && unreadable * 2 > unreadable + self.hands + self.skipped_live
    }
}

/// Replay every downloaded hand above the model's history watermark into a delta model, then
/// merge it into the live models under a short lock.
pub fn import(shared: &Shared, db: &HistoryDb) -> Result<ImportReport> {
    let from = shared.models.read().history_watermark.unwrap_or(0);
    let upto = db.max_id();
    let mut report = ImportReport::default();
    if upto <= from {
        return Ok(report);
    }
    let fleet: Vec<String> = shared.config.bots.iter().map(|b| b.name.clone()).collect();
    let now = chrono::Utc::now();
    let mut delta = ModelStore::default();
    for table in db.tables_after(from, upto)? {
        let rows = db.table_hands(&table)?;
        let mut parsed: Vec<(i64, String, RawHand, Option<String>)> = Vec::with_capacity(rows.len());
        for (id, bot, j, full) in rows {
            match serde_json::from_str::<RawHand>(&j) {
                Ok(r) => parsed.push((id, bot, r, full)),
                // Older rows of the table were counted by an earlier pass.
                Err(_) if id > from && id <= upto => report.unparseable += 1,
                Err(_) => {}
            }
        }
        let refs: Vec<&RawHand> = parsed.iter().map(|p| &p.2).collect();
        let names = attribute_names(&refs, 40);
        let mut summaries = Vec::new();
        for ((id, bot, raw, full), seat_names) in parsed.iter().zip(names) {
            if *id <= from || *id > upto {
                continue;
            }
            if shared.store.hand(bot, &raw.hand_id).ok().flatten().is_some() {
                report.skipped_live += 1;
                continue;
            }
            if let Some(mut summary) = full.as_deref().and_then(|j| serde_json::from_str::<HandSummary>(j).ok()) {
                observe_corpus_hand(&mut delta, &mut summary, bot, &fleet, &raw.started_at, now, &mut report);
                continue;
            }
            let mut seat_names = seat_names;
            seat_names.insert(raw.seat, bot.clone());
            // Winner names can be our other bots only by coincidence of rename; never model them.
            seat_names.retain(|s, n| *s == raw.seat || !fleet.contains(n));
            let tid: String = raw.table_id.chars().take(8).collect();
            let anon = |seat: usize| format!("{ANON_PREFIX}:{tid}:{seat}");
            let Some(summary) = to_summary(raw, &seat_names, &anon) else {
                report.failed += 1;
                continue;
            };
            for (_, n) in &summary.players {
                if n.starts_with(ANON_PREFIX) {
                    report.anon_seats += 1
                } else if n != bot {
                    report.named_seats += 1
                }
            }
            let w = age_weight(&raw.started_at, now);
            delta.observe_weighted(&summary, Some(bot), w);
            delta.observe_image_weighted(&summary, bot, w);
            if let Ok(j) = serde_json::to_string(&summary) {
                summaries.push((*id, j));
            }
            report.hands += 1;
        }
        db.set_summaries(&summaries)?;
    }
    let corpus_from = shared.models.read().corpus_watermark.unwrap_or(0);
    let corpus_upto = db.max_corpus_id();
    for (_, bot, started_at, j) in db.corpus_only_after(corpus_from, corpus_upto)? {
        match serde_json::from_str::<HandSummary>(&j) {
            Ok(mut summary) => observe_corpus_hand(&mut delta, &mut summary, &bot, &fleet, &started_at, now, &mut report),
            Err(_) => report.unparseable += 1,
        }
    }
    let mut models = shared.models.write();
    if models.history_watermark.unwrap_or(0) != from || models.corpus_watermark.unwrap_or(0) != corpus_from {
        // A schema rebuild reset the models meanwhile; the next pass re-imports from scratch.
        return Ok(report);
    }
    models.corpus_watermark = Some(corpus_upto);
    models.population.merge(&delta.population);
    for (name, st) in delta.players {
        models.players.entry(name).or_default().merge(&st);
    }
    // The imported hands carry our own play too, so the image of us grows with them (0321).
    for (name, st) in delta.hero_seen {
        models.hero_seen.entry(name).or_default().merge(&st);
    }
    models.history_watermark = Some(upto);
    report.merged = true;
    Ok(report)
}

/// Fold a full-detail corpus hand into the delta model. Our other bots at the table become
/// anonymous (population only), as in the export path.
fn observe_corpus_hand(
    delta: &mut ModelStore,
    summary: &mut HandSummary,
    bot: &str,
    fleet: &[String],
    started_at: &str,
    now: chrono::DateTime<chrono::Utc>,
    report: &mut ImportReport,
) {
    for (seat, name) in summary.players.iter_mut() {
        if name != bot && fleet.contains(name) {
            *name = format!("{ANON_PREFIX}:own:{seat}");
        }
    }
    for (_, n) in &summary.players {
        if n.starts_with(ANON_PREFIX) {
            report.anon_seats += 1
        } else if n != bot {
            report.named_seats += 1
        }
    }
    let w = age_weight(started_at, now);
    delta.observe_weighted(summary, Some(bot), w);
    delta.observe_image_weighted(summary, bot, w);
    report.hands += 1;
}

/// Live hands joined mid-hand after a reconnect have no starting stacks, so their net is unknown;
/// fill it from the server's profit (matches every tracked net). Returns how many were filled.
pub fn fill_missing_nets(store: &sv10_store::store::Store, db: &HistoryDb) -> Result<usize> {
    let mut filled = 0;
    for (bot, hand_id) in store.hands_missing_net()? {
        if let Some(profit) = db.profit(&bot, &hand_id)
            && store.fill_net(&bot, &hand_id, profit)?
        {
            filled += 1;
        }
    }
    Ok(filled)
}
