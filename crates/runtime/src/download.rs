//! The season's hand history, downloaded into one bot's store (V11-13).
//!
//! It reads the season's export, `GET /api/portfolio/bots/{agent_id}/hand-history/export` with
//! `format=json` (V11-38), with the bot's own key and the id `GET /api/me` gives it, [`PAGE`]
//! hands a page, through a [`Rest`] that honours `Retry-After`. The export is the site's analytics
//! download: this season's hands only, newest first, the same fields as `/me/hand-history`. That
//! older endpoint walks every season the bot ever played, 200 hands a page at about ten seconds
//! each, where the export gives this season in a few pages.
//!
//! Every run reads the season from its newest hand: a new hand moves every offset by one, and a
//! new season starts them again, so no offset from an earlier run names the same hand. The store
//! still records how far each run got. Each hand is kept as the venue sent it, as
//! `Source::History`, and a hand the store already holds, from live play or an earlier run, keeps
//! its first copy.
//!
//! A page is a list, or an object holding the list under `hands`, `items`, `data`, `results` or
//! `history`. A hand is named by its `hand_id`, or else its `id`.

use std::time::Duration;

use serde_json::Value;

use crate::limit::Limiter;
use crate::record::Hand;
use crate::rest::{Rest, RestError};
use crate::store::{Source, Store, StoreError};

/// The profile, which names the bot's `agent_id`.
pub const ME: &str = "/me";

/// The season's export of the bot `agent_id`, as JSON.
pub fn export_path(agent_id: &str) -> String {
    format!("/portfolio/bots/{agent_id}/hand-history/export?format=json")
}

/// Hands per page: more than a season holds, so a season is one request. The export gives an
/// empty reply from offset 10 000 on (measured on 2026-11-23), so a season of more than 10 000
/// hands cannot be paged; from offset 0 it gives any number. An empty reply is an error here,
/// never the end, so a season cut short is never taken for a whole one.
pub const PAGE: u32 = 50_000;

/// How long the one request may take: a season of 14 000 hands is 30 MB and about 75 seconds.
pub const TIMEOUT: Duration = Duration::from_secs(600);

/// The largest export read: a season of 14 000 hands is 30 MB, so this holds about 100 000.
pub const MAX_BODY: u64 = 256 * 1024 * 1024;

/// The endpoint's limit per key: 30 requests a minute, with a second's margin on the window.
pub fn limiter() -> Limiter {
    Limiter::new(30, Duration::from_secs(61))
}

/// What a download did.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Summary {
    pub pages: u32,
    /// Hands new to the store.
    pub added: u32,
    /// Hands the store already held.
    pub known: u32,
    /// Items without a hand id, skipped.
    pub skipped: u32,
    /// Where the next download starts.
    pub offset: u64,
}

/// Why a download stopped early. What it stored so far is kept, with its offset.
#[derive(Debug)]
pub enum DownloadError {
    Rest(RestError),
    Store(StoreError),
    /// The profile named no `agent_id`, so there is no export to read.
    NoAgentId,
}

impl std::fmt::Display for DownloadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Rest(error) => write!(f, "{error}"),
            Self::Store(error) => write!(f, "{error}"),
            Self::NoAgentId => write!(f, "the profile at {ME} names no agent_id"),
        }
    }
}

impl std::error::Error for DownloadError {}

impl From<RestError> for DownloadError {
    fn from(error: RestError) -> Self {
        Self::Rest(error)
    }
}

impl From<StoreError> for DownloadError {
    fn from(error: StoreError) -> Self {
        Self::Store(error)
    }
}

/// Downloads this season's hand history into `store`, from its newest hand. `now_ms` stamps each
/// record.
pub fn download(
    rest: &mut Rest,
    store: &Store,
    mut now_ms: impl FnMut() -> i64,
) -> Result<Summary, DownloadError> {
    let profile = rest.get(ME)?;
    let agent_id = text(&profile, &["agent_id"]).ok_or(DownloadError::NoAgentId)?;
    let mut summary = Summary::default();
    let mut pager = rest.pages(&export_path(&agent_id), PAGE, items);
    while let Some(page) = pager.next_page()? {
        summary.pages += 1;
        for item in &page {
            let Some(hand_id) = text(item, &["hand_id", "id"]) else {
                summary.skipped += 1;
                continue;
            };
            let hand = Hand {
                hand_id,
                table_id: text(item, &["table_id"]).unwrap_or_default(),
                frames: vec![item.to_string()],
                // A downloaded hand was played elsewhere, on a build that kept no trace.
                traced: Vec::new(),
            };
            if store.append_hand(&hand, Source::History, now_ms())? {
                summary.added += 1;
            } else {
                summary.known += 1;
            }
        }
        summary.offset = pager.offset();
        store.record_download(summary.offset, now_ms())?;
    }
    Ok(summary)
}

/// The list of hands in a page.
fn items(page: &Value) -> Option<Vec<Value>> {
    if let Some(list) = page.as_array() {
        return Some(list.clone());
    }
    ["hands", "items", "data", "results", "history"]
        .iter()
        .find_map(|key| page.get(key).and_then(Value::as_array))
        .cloned()
}

/// The first of `keys` that holds text or a number, as text.
fn text(item: &Value, keys: &[&str]) -> Option<String> {
    keys.iter().find_map(|key| match item.get(key)? {
        Value::String(text) => Some(text.clone()),
        Value::Number(number) => Some(number.to_string()),
        _ => None,
    })
}
