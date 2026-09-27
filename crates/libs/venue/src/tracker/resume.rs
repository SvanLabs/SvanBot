//! Resuming a hand across a process restart (0315): the hand in progress is saved when the process
//! exits mid-hand, and the next process settles it from the resync replay's `hand_result`.

use super::TableTracker;
use sv10_cards::cards::Card;

/// A hand in progress, saved when the process exits mid-hand, so the process that starts next can
/// finish it from the resync replay's `hand_result` (0315: a hot swap mid-hand lost the hand's row).
#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct OpenHand {
    /// Server hand id.
    pub hand_id: String,
    /// Table the hand is played at.
    pub table_id: Option<String>,
    /// Our seat.
    pub hero_seat: Option<usize>,
    /// Button seat.
    pub dealer: usize,
    /// Big blind.
    pub bb: i64,
    /// Every seat's stack at the start of the hand.
    pub start_stacks: Vec<(usize, i64)>,
    /// Seats and names dealt in.
    pub dealt: Vec<(usize, String)>,
    /// Our hole cards, when dealt.
    pub hole: Option<String>,
}

impl TableTracker {
    /// The hand in progress, if any, in the form a later process can resume.
    pub fn open_hand(&self) -> Option<OpenHand> {
        let hand_id = self.hand_id.clone()?;
        let mut start_stacks: Vec<(usize, i64)> = self.start_stacks.iter().map(|(s, v)| (*s, *v)).collect();
        start_stacks.sort();
        Some(OpenHand {
            hand_id,
            table_id: self.table_id.clone(),
            hero_seat: self.hero_seat,
            dealer: self.dealer,
            bb: self.bb,
            start_stacks,
            dealt: self.dealt.clone(),
            hole: self.hole.map(|h| format!("{}{}", h[0], h[1])),
        })
    }

    /// Resume a hand saved by an earlier process (before its replayed `hand_result` is applied).
    pub fn resume_hand(&mut self, h: &OpenHand) {
        if self.hand_id.as_deref() != Some(h.hand_id.as_str()) {
            self.begin_hand(&h.hand_id, h.dealer);
        }
        if self.table_id.is_none() {
            self.table_id = h.table_id.clone();
        }
        self.hero_seat = self.hero_seat.or(h.hero_seat);
        if h.bb > 0 {
            self.bb = h.bb;
        }
        for (seat, stack) in &h.start_stacks {
            self.start_stacks.entry(*seat).or_insert(*stack);
        }
        if self.dealt.is_empty() {
            self.dealt = h.dealt.clone();
        }
        if self.hole.is_none()
            && let Some(cards) = &h.hole
            && cards.len() == 4
            && let (Some(a), Some(b)) = (Card::parse(&cards[..2]), Card::parse(&cards[2..]))
        {
            self.hole = Some([a, b]);
        }
    }

    /// Whether this tracker knows the start of `hand_id` well enough to settle it.
    pub fn knows_hand(&self, hand_id: &str) -> bool {
        self.hand_id.as_deref() == Some(hand_id) && self.hero_seat.is_some_and(|h| self.start_stacks.contains_key(&h))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A tracker mid-hand on seat 2 holding AsKd, both stacks 2,000.
    fn mid_hand() -> TableTracker {
        let mut t = TableTracker::default();
        t.reset_table();
        t.table_id = Some("t".into());
        t.begin_hand("h9", 4);
        t.hero_seat = Some(2);
        t.hole = Some([Card::parse("As").unwrap(), Card::parse("Kd").unwrap()]);
        t.start_stacks.insert(2, 2000);
        t.start_stacks.insert(4, 2000);
        t.dealt = vec![(2, "A".into()), (4, "B".into())];
        t
    }

    /// The restart round trip: what the saving process held is what the resuming tracker needs to
    /// settle the hand from its replayed result.
    #[test]
    fn a_hand_saved_at_exit_resumes_with_our_seat_stacks_and_cards() {
        let open = mid_hand().open_hand().expect("a hand in progress is saved");
        let mut next = TableTracker::default();
        next.reset_table();
        assert!(!next.knows_hand("h9"), "a cold process does not know the hand yet");
        next.resume_hand(&open);
        assert!(next.knows_hand("h9"));
        assert_eq!(next.hero_seat, Some(2));
        assert_eq!((next.dealer, next.bb, next.table_id.as_deref()), (4, 20, Some("t")));
        assert_eq!(next.hole, Some([Card::parse("As").unwrap(), Card::parse("Kd").unwrap()]));
        assert_eq!(next.start_stacks.get(&2), Some(&2000));
        assert_eq!(next.start_stacks.get(&4), Some(&2000));
        assert_eq!(next.dealt, vec![(2, "A".into()), (4, "B".into())]);
        assert!(next.history.is_empty(), "a resumed hand settles from the replay, it does not invent actions");
    }

    #[test]
    fn a_table_without_a_hand_saves_nothing() {
        let mut t = TableTracker::default();
        t.reset_table();
        assert!(t.open_hand().is_none());
    }

    /// A process that already restored the hand from the resync snapshot keeps what it has: the
    /// live stream's values are fresher than the saved ones, and the hand is not begun twice.
    #[test]
    fn resuming_keeps_what_this_process_already_knows() {
        let open = mid_hand().open_hand().unwrap();
        let mut live = TableTracker::default();
        live.reset_table();
        live.begin_hand("h9", 0);
        live.hero_seat = Some(2);
        live.start_stacks.insert(2, 2100);
        live.resume_hand(&open);
        assert_eq!(live.dealer, 0, "the current process's button stands");
        assert_eq!(live.start_stacks.get(&2), Some(&2100), "and its own start stack");
        assert_eq!(live.start_stacks.get(&4), Some(&2000), "a seat it lacks is filled from the save");
        assert_eq!(live.hole, Some([Card::parse("As").unwrap(), Card::parse("Kd").unwrap()]), "cards it lacks are filled too");
    }

    /// A saved hand whose cards do not parse resumes without them rather than panicking.
    #[test]
    fn a_saved_hole_that_does_not_parse_is_ignored() {
        let mut open = mid_hand().open_hand().unwrap();
        open.hole = Some("zz".into());
        let mut next = TableTracker::default();
        next.reset_table();
        next.resume_hand(&open);
        assert_eq!(next.hole, None);
        assert!(next.knows_hand("h9"), "settling needs the stack, not the cards");
    }

    /// `knows_hand` asks for our seat's start stack: without it the net cannot be computed, and the
    /// hand must not be settled.
    #[test]
    fn knows_hand_requires_our_seat_start_stack() {
        let mut t = TableTracker::default();
        t.reset_table();
        t.begin_hand("h9", 4);
        t.hero_seat = Some(2);
        assert!(!t.knows_hand("h9"), "no start stack for seat 2");
        t.start_stacks.insert(2, 2000);
        assert!(t.knows_hand("h9"));
        assert!(!t.knows_hand("h10"), "and only for the hand it is playing");
    }
}
