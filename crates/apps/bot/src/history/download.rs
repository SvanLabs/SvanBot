//! Server export download and backfill loop (0260).

use super::db::{HistoryDb, STATUS_KEY, open};
use super::import::{fill_missing_nets, import};
use super::{PAGE, REQUEST_GAP};
use crate::live::Shared;
use anyhow::Result;
use serde_json::{Value, json};
use std::time::Duration;

async fn fetch_page(http: &reqwest::Client, shared: &Shared, key: &str, offset: i64) -> Option<Value> {
    let url = format!("{}/me/hand-history?limit={PAGE}&offset={offset}", shared.config.rest_base);
    for attempt in 0..4u64 {
        match http.get(&url).bearer_auth(key).send().await {
            Ok(r) if r.status().is_success() => return r.json().await.ok(),
            Ok(r) if r.status().as_u16() == 429 => tokio::time::sleep(Duration::from_secs(60 * (attempt + 1))).await,
            Ok(r) if r.status().is_server_error() => return None,
            _ => tokio::time::sleep(Duration::from_secs(10)).await,
        }
    }
    None
}

/// Minimum spacing between attempts at a page that has been failing (deep pages time out on
/// the server depending on its load, so they are retried in stages rather than abandoned).
const DEEP_RETRY: Duration = Duration::from_secs(600);

/// The server exports only the newest `cap` hands per bot; pages past it can never succeed.
/// A `done` marker below the current total with no cap in force means the backfill stopped at
/// the old cap (Free tier) and must resume now that the key is Pro.
pub(super) fn beyond_export_cap(frontier: i64, cap: i64) -> bool {
    cap > 0 && frontier >= cap
}

pub(super) fn resume_capped_backfill(done: bool, frontier: i64, total: i64, cap: i64) -> bool {
    cap == 0 && done && total > 0 && frontier < total
}

fn now_secs() -> i64 {
    chrono::Utc::now().timestamp()
}

fn recent_enough(h: &Value) -> bool {
    // Leave the last few minutes to the live tracker, which records them in full.
    h["ended_at"]
        .as_str()
        .and_then(|t| chrono::DateTime::parse_from_rfc3339(t).ok())
        .map(|t| (chrono::Utc::now() - t.with_timezone(&chrono::Utc)).num_minutes() >= 10)
        .unwrap_or(true)
}

pub(super) async fn store_page(db: &std::sync::Arc<HistoryDb>, bot: &str, page: &Value) -> Result<(usize, usize, i64)> {
    let got = page["hands"].as_array().map(|a| a.len()).unwrap_or(0) as i64;
    let hands: Vec<Value> = page["hands"].as_array().cloned().unwrap_or_default().into_iter().filter(recent_enough).collect();
    let (db, name) = (db.clone(), bot.to_string());
    let (new, known) = tokio::task::spawn_blocking(move || db.insert_page(&name, &hands))
        .await
        .map_err(|e| anyhow::anyhow!("insert_page task join failed: {e}"))?
        .map_err(|e| anyhow::anyhow!("insert_page failed: {e}"))?;
    Ok((new, known, got))
}

/// Background task, in three stages per bot every pass:
/// 1. **Newest** — read from offset 0 until a page contains hands we already have.
/// 2. **Backfill** — continue from the deepest offset reached. The frontier is shifted by the
///    hands played since it was saved (new hands push old ones deeper), so nothing is skipped or
///    re-read. A failing deep page is retried at most every ten minutes, indefinitely, because
///    whether the server manages to serve it depends on its load. A bot whose key reports
///    `pro_tier` ignores the export cap here (Pro exports are unlimited); the cap only binds
///    Free keys.
/// 3. **Past seasons** (Pro keys only) — [`crate::seasons::step`] advances each ended season a
///    bounded number of pages per pass, oldest season first, so every season the key ever played
///    downloads slowly in the background with no operator action beyond the key in `.env`.
///    The active season is left to stages 1–2; ended seasons are frozen server-side, so one read
///    to the end never needs repeating, and the hourly season-list refresh picks up new ones.
pub async fn run(shared: std::sync::Arc<Shared>) {
    tokio::time::sleep(Duration::from_secs(60)).await;
    let db = match open(&shared) {
        Ok(db) => std::sync::Arc::new(db),
        Err(e) => {
            shared.log("history", "error", format!("history database unavailable: {e}"));
            return;
        }
    };
    let Ok(http) = reqwest::Client::builder().user_agent(crate::USER_AGENT).timeout(Duration::from_secs(90)).build() else { return };
    let mut seasons: Vec<crate::seasons::Season> = Vec::new();
    let mut seasons_fetched_at = 0i64;
    loop {
        if now_secs() - seasons_fetched_at >= 3600 {
            match crate::seasons::fetch_seasons(&http, &shared.config.rest_base).await {
                Ok(list) => {
                    seasons_fetched_at = now_secs();
                    seasons = list;
                    shared.log("history", "info", format!("season list refreshed ({} seasons)", seasons.len()));
                }
                Err(e) => shared.log("history", "warn", format!("season list refresh failed: {e:#}")),
            }
        }
        let mut totals = serde_json::Map::new();
        for bot in shared.config.bots.clone() {
            let key = |k: &str| format!("{k}.{}", bot.name);
            let meta_i64 = |k: &str| db.meta(&key(k)).and_then(|v| v.parse::<i64>().ok());
            let mut total = meta_i64("total").unwrap_or(0);
            // Pro keys export without limit, so the cap binds Free keys only. Checked every
            // pass: cheap (one request per bot) and a lapsed Pro falls back to the cap itself.
            let pro = crate::seasons::fetch_pro_tier(&http, &shared.config.rest_base, &bot.api_key).await;
            let cap = if pro { 0 } else { shared.config.export_cap };

            // Stage 1: newest hands (only once a backfill exists; a first backfill starts at 0).
            let mut offset = 0;
            while meta_i64("offset").is_some() {
                let started = std::time::Instant::now();
                let Some(page) = fetch_page(&http, &shared, &bot.api_key, offset).await else { break };
                let server_total = page["total_hands"].as_i64().unwrap_or(total);
                let page_hands = page["hands"].as_array().map(|a| a.len()).unwrap_or(0);
                let (new, known, got) = match store_page(&db, &bot.name, &page).await {
                    Ok(r) => r,
                    Err(e) => {
                        shared.log(
                            "history",
                            "warn",
                            format!(
                                "{}: failed to store {} hands at offset {} — not advancing frontier, will retry next pass: {e}",
                                bot.name, page_hands, offset
                            ),
                        );
                        break;
                    }
                };
                offset += got;
                if got < PAGE || known > 0 || new == 0 {
                    // Caught up with what we already hold; move the backfill frontier by the
                    // hands that arrived since it was saved.
                    if let Some(frontier) = meta_i64("offset")
                        && total > 0
                        && server_total > total
                    {
                        db.set_meta(&key("offset"), &(frontier + server_total - total).to_string());
                    }
                    total = server_total;
                    db.set_meta(&key("total"), &total.to_string());
                    break;
                }
                tokio::time::sleep(REQUEST_GAP.saturating_sub(started.elapsed())).await;
            }

            // Stage 2: backfill toward the oldest hand.
            if resume_capped_backfill(db.meta(&key("done")).is_some(), meta_i64("offset").unwrap_or(0), total, cap) {
                db.clear_meta(&key("done"));
                shared.log(
                    "history",
                    "info",
                    format!(
                        "{}: Pro unlocks the full export; resuming the backfill at hand {} of {total}",
                        bot.name,
                        meta_i64("offset").unwrap_or(0)
                    ),
                );
            }
            let complete = db.meta(&key("done")).is_some();
            let next_try = meta_i64("next_try").unwrap_or(0);
            if !complete && now_secs() >= next_try {
                let mut frontier = meta_i64("offset").unwrap_or(0);
                loop {
                    if total > 0 && frontier >= total {
                        db.set_meta(&key("done"), "1");
                        shared.log("history", "info", format!("{}: full history downloaded ({total} hands)", bot.name));
                        break;
                    }
                    if beyond_export_cap(frontier, cap) {
                        db.set_meta(&key("done"), "1");
                        shared.log(
                            "history",
                            "info",
                            format!("{}: history downloaded to the server's export cap ({} of {total} hands)", bot.name, frontier),
                        );
                        break;
                    }
                    let started = std::time::Instant::now();
                    let Some(page) = fetch_page(&http, &shared, &bot.api_key, frontier).await else {
                        let fails = meta_i64(&format!("fails_at_{frontier}")).unwrap_or(0) + 1;
                        db.set_meta(&key(&format!("fails_at_{frontier}")), &fails.to_string());
                        db.set_meta(&key("next_try"), &(now_secs() + DEEP_RETRY.as_secs() as i64).to_string());
                        if fails == 3 {
                            shared.log(
                                "history",
                                "warn",
                                format!(
                                    "{}: the server keeps timing out at hand {frontier} of {total}; retrying every 10 minutes",
                                    bot.name
                                ),
                            );
                        }
                        break;
                    };
                    let server_total = page["total_hands"].as_i64().unwrap_or(total);
                    let page_hands = page["hands"].as_array().map(|a| a.len()).unwrap_or(0);
                    let (_, _, got) = match store_page(&db, &bot.name, &page).await {
                        Ok(r) => r,
                        Err(e) => {
                            shared.log(
                                "history",
                                "warn",
                                format!(
                                    "{}: failed to store {} hands at offset {} — not advancing frontier, will retry next pass: {e}",
                                    bot.name, page_hands, frontier
                                ),
                            );
                            break;
                        }
                    };
                    // Hands played during the backfill shift this page; skip past them next time.
                    frontier += got + (server_total - total).max(0);
                    total = server_total;
                    db.set_meta(&key("total"), &total.to_string());
                    db.set_meta(&key("offset"), &frontier.to_string());
                    if frontier % 5_000 < PAGE {
                        shared.log("history", "info", format!("{}: downloaded back to hand {frontier} of {total}", bot.name));
                    }
                    if got < PAGE {
                        db.set_meta(&key("done"), "1");
                        shared.log("history", "info", format!("{}: full history downloaded ({total} hands)", bot.name));
                        break;
                    }
                    tokio::time::sleep(REQUEST_GAP.saturating_sub(started.elapsed())).await;
                }
            }
            // Stage 3: past seasons, a few pages per pass so the download trickles in the
            // background. The export filter needs Pro; Free keys keep stages 1–2 only.
            let mut season_summary = Value::Null;
            if pro && !seasons.is_empty() {
                match crate::seasons::step(&db, &http, &shared.config.rest_base, &bot, &seasons, 2, now_secs()).await {
                    Ok(rep) => {
                        for (n, hands) in &rep.finished {
                            shared.log("history", "info", format!("{}: season {n} fully downloaded ({hands} hands stored)", bot.name));
                        }
                        for (n, at) in &rep.exhausted {
                            shared.log(
                                "history",
                                "info",
                                format!(
                                    "{}: season {n} gave up at the server's export ceiling (hand {at}) after {} retries without a new hand",
                                    bot.name,
                                    crate::seasons::MAX_CEILING_RETRIES
                                ),
                            );
                        }
                        for (n, at) in &rep.ceilings {
                            shared.log(
                                "history",
                                "info",
                                format!("{}: season {n} reached the server's timeout point at hand {at}; retrying hourly", bot.name),
                            );
                        }
                        season_summary = serde_json::to_value(&rep.summary).unwrap_or(Value::Null);
                    }
                    Err(e) => shared.log("history", "warn", format!("{}: season backfill failed: {e:#}", bot.name)),
                }
            }
            totals.insert(
                bot.name.clone(),
                json!({"server_total": total, "backfilled": db.meta(&key("done")).is_some(), "frontier": meta_i64("offset"), "pro": pro, "seasons": season_summary}),
            );
        }
        let (s, d) = (shared.clone(), db.clone());
        let report = tokio::task::spawn_blocking(move || {
            match fill_missing_nets(&s.store, &d) {
                Ok(n) if n > 0 => s.log("history", "info", format!("filled {n} live hand results from server histories")),
                Err(e) => s.log("history", "warn", format!("filling live hand results failed: {e}")),
                _ => {}
            }
            import(&s, &d)
        })
        .await;
        match report {
            Ok(Ok(r)) => {
                if r.format_alarm() {
                    shared.log(
                        "history",
                        "error",
                        format!(
                            "history import: {} unparseable and {} unconvertible rows vs {} imported; the export format may have changed",
                            r.unparseable, r.failed, r.hands
                        ),
                    );
                }
                if !r.merged && r.hands > 0 {
                    shared.log(
                        "history",
                        "warn",
                        format!("discarded {} imported hands: the models were reset mid-pass; re-importing next pass", r.hands),
                    );
                } else if r.hands > 0 || r.unparseable > 0 || r.failed > 0 {
                    shared.log(
                        "history",
                        "info",
                        format!(
                            "imported {} past hands into the opponent models ({} named seats, {} population-only, {} already live, {} unconvertible, {} unparseable)",
                            r.hands, r.named_seats, r.anon_seats, r.skipped_live, r.failed, r.unparseable
                        ),
                    );
                }
            }
            Ok(Err(e)) => shared.log("history", "error", format!("history import failed: {e}")),
            Err(e) => shared.log("history", "error", format!("history import task panicked: {e}")),
        }
        let status = json!({"downloaded": db.count(), "bots": totals, "imported_through": shared.models.read().history_watermark, "updated": now_secs()});
        if let Err(e) = shared.store.put_kv(STATUS_KEY, &status.to_string()) {
            tracing::warn!("history status not stored (the dashboard shows the previous one): {e}");
        }
        let all_done = shared.config.bots.iter().all(|b| db.meta(&format!("done.{}", b.name)).is_some());
        tokio::time::sleep(Duration::from_secs(if all_done { 3600 } else { 60 })).await;
    }
}
