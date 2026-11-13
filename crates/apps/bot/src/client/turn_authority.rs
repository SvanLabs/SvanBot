//! The bot's turn authority: whether a turn still owes an answer, and the two per-hand recovery
//! budgets. It outlives one connection — the reconnect loop keeps it, as the dashboard state did —
//! so a reconnect cannot re-answer a turn the writer already accepted or reopen a spent budget. The
//! pending missing-state wait is per connection and stays on [`Seat`](super::Seat).

use super::*;

/// One bot's turn bookkeeping, across reconnects.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct TurnAuthority {
    last_hand: Option<String>,
    last_token: Option<String>,
    last_seq: Option<i64>,
    /// Resyncs asked for after an action rejection in the current hand (hand id, count): at most
    /// [`MAX_REJECT_RESYNCS`] per hand, so a persistent rejection cannot loop.
    reject_resyncs: (Option<String>, u32),
    /// Last table/hand whose missing decision state requested recovery: at most one snapshot per
    /// hand, so an incomplete resync cannot loop while the safe action still meets the deadline.
    missing_state: Option<(String, String)>,
}

impl TurnAuthority {
    /// Whether this turn is a duplicate or older than the newest turn already sent for the hand.
    pub(super) fn turn_answered(&self, tracker: &TableTracker, msg: &Value) -> bool {
        let hand_id = msg["hand_id"].as_str().or(tracker.hand_id.as_deref()).unwrap_or("");
        let token = msg["turn_token"].as_str().unwrap_or("");
        let seq = msg["table_seq"].as_i64();
        if self.last_hand.as_deref() != Some(hand_id) {
            return false;
        }
        (!token.is_empty() && self.last_token.as_deref() == Some(token))
            || matches!((seq, self.last_seq), (Some(current), Some(last)) if current < last)
    }

    /// Record the turn an accepted action consumed.
    pub(super) fn answered(&mut self, hand_id: &str, token: &str, seq: Option<i64>) {
        self.last_hand = Some(hand_id.to_string());
        self.last_token = Some(token.to_string());
        if seq.is_some() {
            self.last_seq = seq;
        }
    }

    /// A rejected action did not consume the turn: let it be answered again.
    pub(super) fn rejected(&mut self) {
        self.last_hand = None;
        self.last_token = None;
    }

    /// Count a rejection in this hand and say whether one more resync is within the budget.
    pub(super) fn permit_reject_resync(&mut self, hand: Option<&str>) -> bool {
        if self.reject_resyncs.0.as_deref() != hand {
            self.reject_resyncs = (hand.map(str::to_string), 0);
        }
        self.reject_resyncs.1 += 1;
        self.reject_resyncs.1 <= MAX_REJECT_RESYNCS
    }

    /// The table/hand whose missing-state recovery was already requested, if any.
    pub(super) fn missing_state(&self) -> Option<&(String, String)> {
        self.missing_state.as_ref()
    }

    /// Mark this table/hand's missing-state recovery as requested.
    pub(super) fn claim_missing_state(&mut self, key: (String, String)) {
        self.missing_state = Some(key);
    }

    /// The token of the last accepted action, for tests that assert a failed writer did not consume
    /// the turn.
    #[cfg(test)]
    pub(super) fn last_token(&self) -> Option<&str> {
        self.last_token.as_deref()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn authority_after(hand: &str, token: &str, seq: Option<i64>) -> (TurnAuthority, TableTracker) {
        let mut authority = TurnAuthority::default();
        authority.answered(hand, token, seq);
        let mut tracker = TableTracker::default();
        tracker.hand_id = Some(hand.into());
        (authority, tracker)
    }

    fn turn(hand: &str, token: &str, seq: Option<i64>) -> Value {
        let mut msg = json!({"hand_id": hand, "turn_token": token});
        if let Some(seq) = seq {
            msg["table_seq"] = json!(seq);
        }
        msg
    }

    #[test]
    fn only_a_nonempty_matching_token_counts_as_answered() {
        let (authority, tracker) = authority_after("h1", "t1", Some(10));
        assert!(authority.turn_answered(&tracker, &turn("h1", "t1", Some(10))));
        let (authority, tracker) = authority_after("h1", "t2", Some(12));
        assert!(authority.turn_answered(&tracker, &turn("h1", "old", Some(11))));
        let (authority, tracker) = authority_after("h1", "t1", Some(10));
        assert!(!authority.turn_answered(&tracker, &turn("h1", "t2", Some(11))));
        let (authority, tracker) = authority_after("h1", "t1", Some(10));
        assert!(!authority.turn_answered(&tracker, &turn("h2", "t1", Some(9))));
        let tracker = TableTracker::default();
        assert!(!TurnAuthority::default().turn_answered(&tracker, &turn("h1", "t1", None)));
    }
}
