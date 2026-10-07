//! Pro per-season hand-history backfill into `history.db`.
//!
//! `/me/hand-history` pages by offset, and the server times out (HTTP 500 after ~10 s) once the
//! offset passes ~15,000 hands. So the plain downloader never reaches most past-season hands of a
//! bot with a long history. The analytics page's Export (openpoker.ai/lab/analytics) uses
//! `/me/hand-history/export?format=json&season_id=<id>`, which Pro unlocks for every season. That
//! restarts the offset in each season, so the newest ~12,000–15,000 hands of every season can be
//! reached. The endpoint accepts `limit` and `offset` but no date or order filter. It is not in the
//! llms-full.txt spec (revision 2026-09-02).
//!
//! The fleet's history task ([`crate::history::run`]) calls [`step`] for every configured bot key, a
//! bounded number of pages per pass, so every ended season downloads slowly in the background with
//! no operator action. Progress is kept per bot and season in `history.db` meta
//! (`season:<bot>:<season_id>`). A season read to its end is never fetched again. A season that hit
//! the server's timeout ceiling is retried hourly from where it stopped, because the ceiling moves
//! with load. Rows land in `raw` like the plain downloader's, so they reach the opponent models.

use crate::config::BotConfig;
use crate::history::HistoryDb;
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::time::Duration;

/// Spacing between season-export requests. The export endpoint tripped HTTP 429 at the plain
/// downloader's 2.6 s pace (2026-09-15: ~10 requests/minute while the fleet's own downloader
/// shares the key quota), so seasons trickle at one request per 6 s; 429s still back off 60 s+.
pub(crate) const EXPORT_GAP: Duration = Duration::from_secs(6);

/// Page sizes tried in order when a page times out.
const LIMITS: [i64; 3] = [200, 100, 50];

/// One season from `GET /season/list`.
#[derive(Clone, Debug, PartialEq)]
pub struct Season {
    /// Season number (1, 2, …).
    pub number: i64,
    /// Season id used by the export filter.
    pub id: String,
    /// Still being played: new hands shift offsets, so it is never marked done.
    pub active: bool,
}

/// Parse the `/season/list` response, oldest season first.
pub fn parse_seasons(v: &Value) -> Vec<Season> {
    let list = v.as_array().or_else(|| v["seasons"].as_array()).cloned().unwrap_or_default();
    let mut out: Vec<Season> = list
        .iter()
        .filter_map(|s| {
            Some(Season {
                number: s["season_number"].as_i64()?,
                id: s["season_id"].as_str().or_else(|| s["id"].as_str())?.to_string(),
                active: s["status"].as_str() == Some("active"),
            })
        })
        .collect();
    out.sort_by_key(|s| s.number);
    out
}

/// Download progress for one bot in one season.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct SeasonProgress {
    /// Next offset to request.
    pub offset: i64,
    /// Read to the end (ended seasons only).
    pub done: bool,
    /// Offset at which every page size timed out on the last attempt.
    pub ceiling: Option<i64>,
    /// Hands newly stored by this downloader, over all runs.
    pub new_hands: i64,
    /// Unix time before which a season at its ceiling is not retried.
    #[serde(default)]
    pub retry_at: i64,
    /// Consecutive retries that reached the ceiling without storing a hand. Reset by any page.
    #[serde(default)]
    pub ceiling_retries: i64,
}

/// Outcome of one request.
#[derive(Clone, Debug, PartialEq)]
pub enum Page {
    /// A page of `got` hands, `new` of them not stored before.
    Hands { got: i64, new: i64 },
    /// Every page size timed out at this offset.
    TimedOut,
}

/// What to do after a page.
#[derive(Clone, Debug, PartialEq)]
pub enum Next {
    /// Request the next offset.
    Continue,
    /// The season is exhausted.
    End,
    /// The server's timeout ceiling was reached.
    Ceiling,
    /// The ceiling held for [`MAX_CEILING_RETRIES`] retries; the season is given up on.
    Exhausted,
}

/// Retries at an unmoved ceiling before a season is given up on. The ceiling moves with server
/// load, so a few retries are worth it; a ceiling that survives this many spaced retries is
/// structural, and further passes only re-request pages that time out (2026-09-20: four seasons
/// had been retried hourly for days at ~16,600 hands without storing one new hand).
pub const MAX_CEILING_RETRIES: i64 = 8;

impl SeasonProgress {
    /// Fold one page result into the progress.
    pub fn apply(&mut self, page: Page, limit: i64, active: bool) -> Next {
        match page {
            Page::Hands { got, new } => {
                self.offset += got;
                self.new_hands += new;
                self.ceiling = None;
                self.retry_at = 0;
                self.ceiling_retries = 0;
                if got < limit {
                    self.done = !active;
                    Next::End
                } else {
                    Next::Continue
                }
            }
            Page::TimedOut => {
                self.ceiling = Some(self.offset);
                self.ceiling_retries += 1;
                if self.ceiling_retries >= MAX_CEILING_RETRIES {
                    self.done = !active;
                    return Next::Exhausted;
                }
                Next::Ceiling
            }
        }
    }
}

fn meta_key(bot: &str, season: &Season) -> String {
    format!("season:{bot}:{}", season.id)
}

/// Seasons from `GET /season/list`, oldest first.
pub async fn fetch_seasons(http: &reqwest::Client, rest_base: &str) -> Result<Vec<Season>> {
    let v: Value = http.get(format!("{rest_base}/season/list")).send().await?.error_for_status()?.json().await?;
    Ok(parse_seasons(&v))
}

/// Whether this bot key has Pro (`GET /season/me` → `pro_tier`). Pro unlocks the unlimited
/// full hand-history export, so a Pro bot's plain backfill ignores the export cap and its
/// ended seasons download. Fails closed (`false`), so a transient error never lifts the cap.
pub async fn fetch_pro_tier(http: &reqwest::Client, rest_base: &str, key: &str) -> bool {
    let Ok(r) = http.get(format!("{rest_base}/season/me")).bearer_auth(key).send().await else { return false };
    let Ok(v): Result<Value, _> = r.json().await else { return false };
    is_pro(&v)
}

/// Read the `pro_tier` flag out of a `/season/me` response.
pub fn is_pro(v: &Value) -> bool {
    v["pro_tier"].as_bool().unwrap_or(false)
}

/// Fetch one export page, shrinking the page size on server timeouts. Returns the hands and the
/// limit that worked, or `None` when every size timed out. A pass that was only rate-limited or
/// could not connect is an error, not a timeout: counted as a ceiling, a long outage used up the
/// retries and marked an ended season done with its hands still on the server.
async fn fetch(http: &reqwest::Client, rest_base: &str, key: &str, season: &Season, offset: i64) -> Result<Option<(Vec<Value>, i64)>> {
    let mut timed_out = false;
    for limit in LIMITS {
        let url = format!("{rest_base}/me/hand-history/export?format=json&season_id={}&limit={limit}&offset={offset}", season.id);
        for attempt in 0..3u64 {
            tokio::time::sleep(EXPORT_GAP).await;
            let r = match http.get(&url).bearer_auth(key).send().await {
                Ok(r) => r,
                Err(_) => {
                    tokio::time::sleep(Duration::from_secs(10)).await;
                    continue;
                }
            };
            let status = r.status();
            if status.is_success() {
                let v: Value = r.json().await.context("export page is not JSON")?;
                // A body with no list of hands is not an empty page: read as one, it ended the season.
                let hands = v.as_array().or_else(|| v["hands"].as_array()).cloned().context("export page holds no list of hands")?;
                return Ok(Some((hands, limit)));
            }
            match status.as_u16() {
                429 => tokio::time::sleep(Duration::from_secs(60 * (attempt + 1))).await,
                401 | 403 => anyhow::bail!("export refused ({status}): past seasons need Pro"),
                s if s >= 500 => {
                    timed_out = true;
                    break;
                }
                _ => anyhow::bail!("export failed with {status}"),
            }
        }
    }
    anyhow::ensure!(timed_out, "export of season {} got no answer (rate limit or network); it is tried again on the next pass", season.id);
    Ok(None)
}

/// A ceiling is retried after this long (the server's timeout point moves with its load).
pub const CEILING_RETRY_SECS: i64 = 3600;

/// Per-bot summary of the season backfill after a step.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct BotSeasons {
    /// Ended seasons read to their end.
    pub complete: usize,
    /// Ended seasons.
    pub ended: usize,
    /// Hands stored by the season backfill over all runs.
    pub new_hands: i64,
    /// Seasons waiting to retry at the server's timeout ceiling: (season number, offset).
    pub ceilings: Vec<(i64, i64)>,
    /// Ended seasons given up on at an unmoving ceiling.
    pub exhausted: usize,
}

/// What one step did.
#[derive(Clone, Debug, Default)]
pub struct StepReport {
    /// Requests' pages processed.
    pub pages: usize,
    /// Hands newly stored in this step.
    pub new_hands: i64,
    /// Seasons finished in this step: (season number, hands stored for it overall).
    pub finished: Vec<(i64, i64)>,
    /// Seasons that reached the timeout ceiling in this step: (season number, offset).
    pub ceilings: Vec<(i64, i64)>,
    /// Seasons given up on at an unmoving ceiling in this step: (season number, offset).
    pub exhausted: Vec<(i64, i64)>,
    /// State after the step.
    pub summary: BotSeasons,
}

/// Advance one bot's backfill of ended seasons by at most `budget` pages, oldest pending season
/// first. The active season is left to the plain downloader (its offsets shift as hands arrive).
pub async fn step(
    db: &HistoryDb,
    http: &reqwest::Client,
    rest_base: &str,
    bot: &BotConfig,
    seasons: &[Season],
    budget: usize,
    now: i64,
) -> Result<StepReport> {
    let mut report = StepReport::default();
    for season in seasons.iter().filter(|s| !s.active) {
        let key = meta_key(&bot.name, season);
        let mut progress: SeasonProgress = db.meta(&key).and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default();
        while !progress.done && progress.retry_at <= now && report.pages < budget {
            let page = match fetch(http, rest_base, &bot.api_key, season, progress.offset).await? {
                Some((hands, limit)) => {
                    let (new, _) = db.insert_page(&bot.name, &hands)?;
                    report.new_hands += new as i64;
                    (Page::Hands { got: hands.len() as i64, new: new as i64 }, limit)
                }
                None => (Page::TimedOut, LIMITS[0]),
            };
            report.pages += 1;
            match progress.apply(page.0, page.1, false) {
                Next::Continue => {}
                Next::End => report.finished.push((season.number, progress.new_hands)),
                Next::Ceiling => {
                    // Spaced further apart each time, so a ceiling that is not going to move
                    // stops costing requests long before it is given up on.
                    progress.retry_at = now + CEILING_RETRY_SECS * progress.ceiling_retries.max(1);
                    report.ceilings.push((season.number, progress.offset));
                }
                Next::Exhausted => report.exhausted.push((season.number, progress.offset)),
            }
            db.set_meta(&key, &serde_json::to_string(&progress)?);
        }
        report.summary.ended += 1;
        report.summary.new_hands += progress.new_hands;
        if progress.done {
            report.summary.complete += 1;
            if progress.ceiling.is_some() {
                report.summary.exhausted += 1;
            }
        } else if let Some(c) = progress.ceiling {
            report.summary.ceilings.push((season.number, c));
        }
    }
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn seasons_parse_oldest_first_with_active_flag() {
        let v = json!([
            {"season_number": 12, "season_id": "b", "status": "active"},
            {"season_number": 11, "season_id": "a", "status": "ended"},
            {"season_number": null, "season_id": "x"}
        ]);
        let s = parse_seasons(&v);
        assert_eq!(s, vec![Season { number: 11, id: "a".into(), active: false }, Season { number: 12, id: "b".into(), active: true }]);
    }

    #[test]
    fn pro_flag_reads_season_me() {
        assert!(is_pro(&json!({"pro_tier": true, "hands_played": 12286})));
        assert!(!is_pro(&json!({"pro_tier": false})));
        assert!(!is_pro(&json!({"detail": "error"})), "fail closed");
    }

    #[test]
    fn progress_ends_on_a_short_page_and_resumes_after_a_ceiling() {
        let mut p = SeasonProgress::default();
        assert_eq!(p.apply(Page::Hands { got: 200, new: 150 }, 200, false), Next::Continue);
        assert_eq!(p.apply(Page::TimedOut, 200, false), Next::Ceiling);
        assert_eq!((p.offset, p.ceiling, p.done), (200, Some(200), false));
        assert_eq!(p.apply(Page::Hands { got: 50, new: 50 }, 100, false), Next::End);
        assert_eq!((p.offset, p.ceiling, p.done, p.new_hands), (250, None, true, 200));
        let mut active = SeasonProgress::default();
        assert_eq!(active.apply(Page::Hands { got: 3, new: 3 }, 200, true), Next::End);
        assert!(!active.done, "an active season is never complete");
    }

    #[test]
    fn an_unmoving_ceiling_is_given_up_on_and_a_page_forgives_it() {
        let mut p = SeasonProgress::default();
        for _ in 0..MAX_CEILING_RETRIES - 1 {
            assert_eq!(p.apply(Page::TimedOut, 200, false), Next::Ceiling);
        }
        assert!(!p.done);
        assert_eq!(p.apply(Page::TimedOut, 200, false), Next::Exhausted);
        assert_eq!((p.done, p.ceiling), (true, Some(0)));

        // A ceiling that moves resets the count, so a loaded server never costs a season.
        let mut q = SeasonProgress::default();
        for _ in 0..MAX_CEILING_RETRIES - 1 {
            assert_eq!(q.apply(Page::TimedOut, 200, false), Next::Ceiling);
        }
        assert_eq!(q.apply(Page::Hands { got: 200, new: 200 }, 200, false), Next::Continue);
        assert_eq!(q.ceiling_retries, 0);
        assert_eq!(q.apply(Page::TimedOut, 200, false), Next::Ceiling);

        // An active season is never marked done, however long its ceiling holds.
        let mut a = SeasonProgress::default();
        for _ in 0..MAX_CEILING_RETRIES {
            a.apply(Page::TimedOut, 200, true);
        }
        assert!(!a.done);
    }
}
