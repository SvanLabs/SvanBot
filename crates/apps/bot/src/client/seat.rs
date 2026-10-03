//! The seat lifecycle (0206): whether a leave is in flight and why, whether a lobby join is wanted,
//! and the between-hands choice of whether to move tables. The handler and the session loop send
//! the frames; the state and the order of the reasons live here.

use super::TableQuality;
use crate::config::BotConfig;
use std::time::Duration;
use sv10_venue::tracker::SeatView;

/// Why a `leave_table` was sent.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Leave {
    /// Leave and rejoin at once: table selection, banking, a top-up, or a stuck-table re-queue.
    Rejoin,
    /// Leave and stay off until the operator runs the bot again.
    Pause,
}

/// One session's seat state.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct Seat {
    leave: Option<Leave>,
    /// A lobby join is wanted; the session loop sends it with backoff and clears this.
    pub(super) pending_join: bool,
}

impl Seat {
    /// A leave is in flight (or, for a pause, the bot stays off its table).
    pub(super) fn leaving(&self) -> bool {
        self.leave.is_some()
    }

    /// The leave in flight, if any.
    #[cfg(test)]
    pub(super) fn leave_reason(&self) -> Option<Leave> {
        self.leave
    }

    /// Start a leave unless one is in flight; true when the caller should send `leave_table`.
    pub(super) fn leave(&mut self, why: Leave) -> bool {
        if self.leave.is_some() {
            return false;
        }
        self.leave = Some(why);
        true
    }

    /// Seated at a table: any leave is over.
    pub(super) fn joined(&mut self) {
        self.leave = None;
    }

    /// Off the table (it closed, we left, or the server says we are not seated). Queues a rejoin
    /// unless the bot is pausing; returns true when it stays off (paused).
    pub(super) fn left(&mut self) -> bool {
        if self.leave == Some(Leave::Pause) {
            return true;
        }
        self.leave = None;
        self.pending_join = true;
        false
    }

    /// A season change ends any leave in flight; the new season seats us fresh.
    pub(super) fn season_ended(&mut self) {
        self.leave = None;
        self.pending_join = true;
    }
}

/// A table move chosen between hands.
#[derive(Clone, Debug, PartialEq)]
pub(super) enum Move {
    /// Seeking top-ranked bots: none is seated here (the reason names the table).
    Seek(String),
    /// Tough table with no soft opponent (the table summary).
    Tough(String),
    /// Bank winnings: a very deep stack only adds swing.
    Bank,
    /// Short stack: check the off-table balance and rejoin deeper if it can fund it.
    TopUp,
    /// A same-owner bot is seated at this table (its name): the venue's fair-play rules never allow
    /// it (docs/SPEC-pro.md), so the seat leaves at once.
    Stablemate(String),
}

/// What the between-hands choice looks at.
pub(super) struct HandEnd<'a> {
    /// Table moves are allowed (not near the season end).
    pub moves_ok: bool,
    /// A same-owner bot seated at this table (its name), from [`stablemate_at_table`].
    pub stablemate: Option<String>,
    /// Hands played at this table.
    pub hands_at_table: u32,
    /// `seek_top_rank` from the config (0 = no seeking).
    pub seek_top_rank: i64,
    /// Opponents at the table.
    pub quality: &'a TableQuality,
    /// Time since the last table switch.
    pub since_switch: Option<Duration>,
    /// Time since the last top-up check.
    pub since_topup: Option<Duration>,
    /// Our table stack.
    pub stack: i64,
    /// The big blind.
    pub bb: i64,
    /// `bank_stack_bb` from the config (0 = no banking).
    pub bank_stack_bb: i64,
    /// `bank_until_chips` from the config (0 = no ceiling).
    pub bank_until_chips: i64,
    /// Off-table balance plus table stack, from the last balance on record (`None` = not known yet).
    pub total_chips: Option<i64>,
    /// `max_buy_in` from the config.
    pub max_buy_in: i64,
}

fn within(since: Option<Duration>, secs: u64) -> bool {
    since.is_some_and(|d| d < Duration::from_secs(secs))
}

/// The move to make after this hand, first reason wins: fair play, seek, tough table, bank, top up.
pub(super) fn between_hands(h: &HandEnd) -> Option<Move> {
    // Fair play first, and above the move window: two same-owner bots must never share a table
    // (docs/SPEC-pro.md). This is not a voluntary table move, so the season-end freeze does not
    // suppress it — staying seated together is the rule violation, not leaving.
    if let Some(name) = &h.stablemate {
        return Some(Move::Stablemate(name.clone()));
    }
    if !h.moves_ok {
        return None;
    }
    let q = h.quality;
    let seek = h.seek_top_rank;
    let n = h.hands_at_table;
    if seek > 0 && n >= 15 && n.is_multiple_of(5) && q.top.is_empty() && q.opponents >= 1 && !within(h.since_switch, 600) {
        return Some(Move::Seek(format!("no top-{seek} bot seated ({})", q.summary)));
    }
    if n >= 30
        && n.is_multiple_of(10)
        && (seek == 0 || q.top.is_empty())
        && q.opponents >= 2
        && q.soft == 0
        && q.tough >= 2.max(q.opponents - 1)
        && !within(h.since_switch, 1200)
    {
        return Some(Move::Tough(q.summary.clone()));
    }
    if should_bank(h.stack, h.bb, h.bank_stack_bb) && !past_bank_ceiling(h.total_chips, h.bank_until_chips) {
        return Some(Move::Bank);
    }
    if !within(h.since_topup, 600) && h.stack > 0 && h.stack < h.max_buy_in * 35 / 100 {
        return Some(Move::TopUp);
    }
    None
}

/// Whether a table stack is deep enough to bank (`bank_bb` big blinds or more; 0 disables).
fn should_bank(stack: i64, bb: i64, bank_bb: i64) -> bool {
    bank_bb > 0 && bb > 0 && stack >= bank_bb * bb
}

/// Whether the bot is far enough ahead to stop banking: its total chips have reached the ceiling.
/// An unknown total banks as before, by table stack alone.
fn past_bank_ceiling(total_chips: Option<i64>, until: i64) -> bool {
    until > 0 && total_chips.is_some_and(|t| t >= until)
}

/// Whether a short stack's top-up is worth a rejoin: the off-table balance funds at least twice
/// the stack (capped at the max buy-in) and is at least 1,000 chips.
pub(super) fn top_up_funded(stack: i64, balance: i64, max_buy_in: i64) -> bool {
    (balance + stack).min(max_buy_in) >= stack * 2 && balance >= 1000
}

/// The first same-owner bot seated at this table, other than our own seat, by configured name —
/// the venue's fair-play rules never allow two (docs/SPEC-pro.md), and configured names cannot
/// false-positive.
pub(super) fn stablemate_at_table(
    seats: &std::collections::BTreeMap<usize, SeatView>,
    own_seat: Option<usize>,
    fleet: &[BotConfig],
) -> Option<String> {
    seats
        .iter()
        .filter(|(seat, _)| Some(**seat) != own_seat)
        .map(|(_, view)| &view.name)
        .find(|name| fleet.iter().any(|b| b.name == **name))
        .cloned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn table(soft: usize, tough: usize, opponents: usize, top: bool) -> TableQuality {
        TableQuality { soft, tough, opponents, top: if top { vec![("Top".into(), 3)] } else { vec![] }, summary: "s".into() }
    }

    fn end(q: &TableQuality) -> HandEnd<'_> {
        HandEnd {
            moves_ok: true,
            stablemate: None,
            hands_at_table: 7,
            seek_top_rank: 30,
            quality: q,
            since_switch: None,
            since_topup: None,
            stack: 5_000,
            bb: 20,
            bank_stack_bb: 2_000,
            bank_until_chips: 500_000,
            total_chips: None,
            max_buy_in: 5_000,
        }
    }

    #[test]
    fn a_same_owner_bot_at_the_table_forces_a_leave_even_in_the_freeze() {
        // The venue's fair-play rules: never two same-owner bots at one table (docs/SPEC-pro.md).
        let seats = |entries: &[(usize, &str)]| -> std::collections::BTreeMap<usize, SeatView> {
            entries.iter().map(|&(i, n)| (i, SeatView { seat: i, name: n.into(), ..Default::default() })).collect()
        };
        let fleet: Vec<BotConfig> = ["Alpha", "Beta"].iter().map(|n| BotConfig { name: n.to_string(), api_key: String::new() }).collect();
        assert_eq!(
            stablemate_at_table(&seats(&[(1, "Alpha"), (3, "Beta")]), Some(1), &fleet),
            Some("Beta".to_string()),
            "our own seat is never a stablemate"
        );
        assert_eq!(stablemate_at_table(&seats(&[(1, "Alpha"), (3, "Stranger")]), Some(1), &fleet), None);
        assert_eq!(stablemate_at_table(&seats(&[(0, "Beta")]), None, &fleet), Some("Beta".to_string()));
        let q = table(1, 0, 3, false);
        let frozen = HandEnd { moves_ok: false, stablemate: Some("Beta".into()), ..end(&q) };
        assert_eq!(
            between_hands(&frozen),
            Some(Move::Stablemate("Beta".into())),
            "the season-end freeze does not suppress the fair-play leave"
        );
    }

    #[test]
    fn a_very_deep_stack_is_banked() {
        // SurSvan held 105,098 chips (5,255 bb) when it lost them all in one hand (2026-09-23).
        assert!(should_bank(105_098, 20, 2_000));
        assert!(should_bank(40_000, 20, 2_000));
        assert!(!should_bank(39_999, 20, 2_000));
        assert!(!should_bank(105_098, 20, 0), "0 disables banking");
        assert!(!should_bank(105_098, 0, 2_000), "no blind known yet");
    }

    #[test]
    fn banking_stops_once_the_total_reaches_the_ceiling() {
        let q = table(1, 0, 3, false);
        let deep = |total: Option<i64>| between_hands(&HandEnd { stack: 50_000, total_chips: total, ..end(&q) });
        assert_eq!(deep(Some(120_000)), Some(Move::Bank), "early: bank");
        assert_eq!(deep(Some(499_999)), Some(Move::Bank));
        assert_eq!(deep(Some(500_000)), None, "at the ceiling the deep stack stays on the table");
        assert_eq!(deep(Some(900_000)), None);
        assert_eq!(deep(None), Some(Move::Bank), "an unknown total banks by table stack, as before");
        let no_ceiling = HandEnd { stack: 50_000, total_chips: Some(900_000), bank_until_chips: 0, ..end(&q) };
        assert_eq!(between_hands(&no_ceiling), Some(Move::Bank), "0 = no ceiling");
    }

    #[test]
    fn a_normal_hand_stays_put() {
        let q = table(1, 1, 3, true);
        assert_eq!(between_hands(&end(&q)), None);
    }

    #[test]
    fn seeking_re_queues_every_fifth_hand_from_the_fifteenth_without_a_top_bot() {
        let q = table(1, 0, 2, false);
        let at =
            |n, since: Option<u64>| between_hands(&HandEnd { hands_at_table: n, since_switch: since.map(Duration::from_secs), ..end(&q) });
        assert_eq!(at(15, None), Some(Move::Seek("no top-30 bot seated (s)".into())));
        assert_eq!(at(10, None), None, "too early");
        assert_eq!(at(16, None), None, "only every fifth hand");
        assert_eq!(at(15, Some(599)), None, "switched under ten minutes ago");
        assert!(at(15, Some(600)).is_some());
        let top = table(0, 3, 3, true);
        assert_eq!(between_hands(&HandEnd { hands_at_table: 15, ..end(&top) }), None, "a top bot is seated");
    }

    #[test]
    fn a_tough_table_without_soft_opponents_is_left_every_tenth_hand_from_the_thirtieth() {
        let q = table(0, 3, 3, true);
        let at = |n, seek, since: Option<u64>| {
            between_hands(&HandEnd { hands_at_table: n, seek_top_rank: seek, since_switch: since.map(Duration::from_secs), ..end(&q) })
        };
        assert_eq!(at(30, 0, None), Some(Move::Tough("s".into())));
        assert_eq!(at(30, 30, None), None, "seeking keeps a table with a top bot");
        assert_eq!(at(35, 0, None), None);
        assert_eq!(at(30, 0, Some(1_199)), None, "switched under twenty minutes ago");
        let soft = table(1, 2, 3, false);
        assert_eq!(between_hands(&HandEnd { hands_at_table: 30, seek_top_rank: 0, ..end(&soft) }), None, "a soft opponent keeps us");
    }

    #[test]
    fn reasons_are_taken_in_order_seek_tough_bank_top_up() {
        let empty = table(0, 2, 2, false);
        // Hand 30 qualifies for seeking and a tough table, with a bankable stack: seeking wins.
        let all = HandEnd { hands_at_table: 30, stack: 50_000, ..end(&empty) };
        assert!(matches!(between_hands(&all), Some(Move::Seek(_))));
        let no_seek = HandEnd { seek_top_rank: 0, ..all };
        assert!(matches!(between_hands(&no_seek), Some(Move::Tough(_))));
        let q = table(1, 0, 2, true);
        assert_eq!(between_hands(&HandEnd { stack: 50_000, ..end(&q) }), Some(Move::Bank));
        assert_eq!(between_hands(&HandEnd { stack: 1_000, ..end(&q) }), Some(Move::TopUp));
    }

    #[test]
    fn no_move_near_the_season_end() {
        let q = table(0, 2, 2, false);
        assert_eq!(between_hands(&HandEnd { moves_ok: false, hands_at_table: 30, stack: 50_000, ..end(&q) }), None);
    }

    #[test]
    fn a_short_stack_tops_up_at_most_every_ten_minutes_when_the_balance_funds_it() {
        let q = table(1, 0, 2, true);
        let short = |since: Option<u64>| between_hands(&HandEnd { stack: 1_749, since_topup: since.map(Duration::from_secs), ..end(&q) });
        assert_eq!(short(None), Some(Move::TopUp), "under 35% of the max buy-in");
        assert_eq!(short(Some(599)), None);
        assert_eq!(between_hands(&HandEnd { stack: 1_750, ..end(&q) }), None, "35% is deep enough");
        assert_eq!(between_hands(&HandEnd { stack: 0, ..end(&q) }), None, "busted is the rebuy's job");
        assert!(top_up_funded(1_000, 1_000, 5_000));
        assert!(!top_up_funded(1_000, 999, 5_000), "under 1,000 chips off the table");
        assert!(!top_up_funded(3_000, 2_000, 5_000), "the buy-in cap cannot double the stack");
    }

    #[test]
    fn a_rejoin_leave_queues_a_join_but_a_pause_stays_off() {
        let mut s = Seat::default();
        assert!(s.leave(Leave::Rejoin));
        assert!(!s.leave(Leave::Pause), "one leave at a time");
        assert!(!s.left(), "not paused");
        assert_eq!(s, Seat { leave: None, pending_join: true });
        let mut s = Seat::default();
        s.leave(Leave::Pause);
        assert!(s.left());
        assert!(s.leaving() && !s.pending_join, "paused bots stay off");
        s.joined();
        assert!(!s.leaving());
        s.leave(Leave::Pause);
        s.season_ended();
        assert_eq!(s, Seat { leave: None, pending_join: true }, "a new season seats us fresh");
        let mut s = Seat::default();
        assert!(!s.left(), "an unplanned loss of the table rejoins");
        assert!(s.pending_join);
    }
}
