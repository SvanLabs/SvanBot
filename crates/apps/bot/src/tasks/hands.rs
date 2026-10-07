//! Unstored-hand retry queue: bounded, watermark-safe (0256).

use crate::live::Shared;
use sv10_store::store;

/// Retry rounds (30 s apart) before a hand is given up: 20 minutes of failures means the store
/// itself is broken, and the operator needs one clear error rather than a warning every 30 s.
pub const MAX_HAND_RETRIES: u32 = 40;
/// Most hands held for retry; the oldest are dropped first.
pub const MAX_UNSTORED_HANDS: usize = 2_000;

/// Apply the retry limits: drop hands that used up their retries and the oldest beyond the cap.
/// Returns the queue to keep and how many were dropped.
pub fn bound_unstored(queue: Vec<(store::HandRow, u32)>) -> (Vec<(store::HandRow, u32)>, usize) {
    let before = queue.len();
    let mut keep: Vec<_> = queue.into_iter().filter(|(_, attempts)| *attempts < MAX_HAND_RETRIES).collect();
    if keep.len() > MAX_UNSTORED_HANDS {
        keep.drain(..keep.len() - MAX_UNSTORED_HANDS);
    }
    let dropped = before - keep.len();
    (keep, dropped)
}

/// Put the retried hands back on the queue, keeping whatever the frame loop queued while this round
/// was inserting. The drain and the write share one lock: taking the queue, working on it unlocked
/// and assigning the result back drops anything pushed in between — a hand that then exists nowhere
/// (0247). Returns the hands still queued and how many were given up.
pub(super) fn requeue(shared: &Shared, mut keep: Vec<(store::HandRow, u32)>) -> (usize, usize) {
    let mut queue = shared.unstored_hands.lock();
    // Hands queued during the round go after the retried ones, so the oldest stay first.
    keep.append(&mut queue);
    let (keep, dropped) = bound_unstored(keep);
    let left = keep.len();
    *queue = keep;
    (left, dropped)
}

/// One last try at the hands that could not be stored, before the process exits. The queue lives in
/// memory only: left as it was, an exit dropped them without a word although the models saved on the
/// same exit already count them (#897). What still cannot be stored is named, so the gap is on record.
pub fn flush_unstored_hands(shared: &Shared) {
    if shared.unstored_hands.lock().is_empty() {
        return;
    }
    let (stored, _) = retry_unstored_hands(shared);
    let lost: Vec<String> = shared.unstored_hands.lock().iter().map(|(row, _)| format!("{} {}", row.bot, row.hand_id)).collect();
    if lost.is_empty() {
        tracing::info!("stored {stored} queued hand(s) before exit");
    } else {
        tracing::error!("exiting with {} hand(s) not stored (stored {stored}): {}", lost.len(), lost.join(", "));
    }
}

/// Insert queued hands; returns (stored, still queued). A stored hand was already observed by the
/// models when it finished, so its rowid only advances the watermark: a restart then does not
/// replay it into the models a second time. Retries are bounded (LESSONS 30): see
/// [`MAX_HAND_RETRIES`] and [`MAX_UNSTORED_HANDS`].
pub fn retry_unstored_hands(shared: &Shared) -> (usize, usize) {
    let pending = std::mem::take(&mut *shared.unstored_hands.lock());
    let mut stored = 0;
    let mut left = Vec::new();
    for (row, attempts) in pending {
        match shared.store.insert_hand(&row) {
            Ok(rowid) => {
                let mut models = shared.models.write();
                models.watermark = Some(models.watermark.unwrap_or(0).max(rowid));
                stored += 1;
            }
            Err(_) => left.push((row, attempts + 1)),
        }
    }
    let (n_left, dropped) = requeue(shared, left);
    if stored > 0 || n_left > 0 {
        shared.log(
            "store",
            if n_left > 0 { "warn" } else { "info" },
            format!("retried unstored hands: {stored} stored, {n_left} still queued"),
        );
    }
    if dropped > 0 {
        shared.log(
            "store",
            "error",
            format!("gave up on {dropped} hand(s) after {MAX_HAND_RETRIES} failed retries or a full retry queue; they are not stored"),
        );
    }
    (stored, n_left)
}
