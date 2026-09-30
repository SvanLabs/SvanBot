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
