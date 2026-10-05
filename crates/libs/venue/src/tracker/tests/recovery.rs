use super::*;

#[test]
fn resync_accepts_old_envelopes_without_reopening_applied_sequences() {
    let mut tracker = TableTracker::default();
    assert!(tracker.accept_seq(&json!({"type": "player_action", "table_seq": 100})));
    for seq in [50, 100] {
        assert!(tracker.accept_seq(&json!({"type": "resync_response", "table_seq": seq})));
        assert_eq!(tracker.last_table_seq, 100);
        assert!(!tracker.accept_seq(&json!({"type": "table_state", "table_seq": 60})));
        assert!(!tracker.accept_seq(&json!({"type": "player_action", "table_seq": 100})));
        assert!(tracker.accept_seq(&json!({"type": "table_state", "table_seq": 100})));
    }
    assert!(tracker.accept_seq(&json!({"type": "resync_response"})));
    assert_eq!(tracker.last_table_seq, 100);
    assert!(tracker.accept_seq(&json!({"type": "resync_response", "table_seq": 120})));
    assert_eq!(tracker.last_table_seq, 120);
    assert!(tracker.accept_seq(&json!({"type": "player_action", "table_seq": 121})));
}

/// The minimum-raise baseline of a new hand is that hand's big blind (#876). It was read before the
/// blinds were assigned, so the first hand after a stakes change judged its opening raise against
/// the previous blind: here a 60 raise over a 100 blind, 40 short of a full raise, read as full.
#[test]
fn the_first_raise_of_a_hand_is_judged_against_that_hands_big_blind() {
    for by_snapshot in [false, true] {
        let mut t = TableTracker::default();
        t.reset_table();
        if by_snapshot {
            t.table_state(&json!({"type": "table_state", "hand_id": "h1", "street": "preflop", "dealer_seat": 0,
                "small_blind": 50, "big_blind": 100, "current_bet": 100, "seats": []}));
        } else {
            t.hand_start(&json!({"type": "hand_start", "hand_id": "h1", "dealer_seat": 0, "seat": 0,
                "blinds": {"small_blind": 50, "big_blind": 100}}));
        }
        assert_eq!(t.last_full_raise, 100, "by_snapshot={by_snapshot}");
    }
}
