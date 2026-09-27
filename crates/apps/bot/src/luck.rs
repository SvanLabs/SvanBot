//! All-in luck of our stored hands (0213): the luck-adjusted ("all-in EV") net of each hand, from
//! [`sv10_core::allin`], for the dashboard's EV line and `review allin-luck`.

use sv10_core::allin::{AllInLuck, hero_all_in_luck};
use sv10_core::cards::Card;
use sv10_core::model::HandSummary;
use sv10_store::store::HandRow;

/// Runouts averaged per all-in: every flop and turn runout, a sample of preflop ones.
pub const MAX_RUNOUTS: usize = 4_000;

/// A stable seed per hand, so a preflop all-in's sampled EV never changes between reads.
fn seed(hand_id: &str) -> u64 {
    hand_id.bytes().fold(0xcbf2_9ce4_8422_2325u64, |h, b| (h ^ b as u64).wrapping_mul(0x0100_0000_01b3))
}

/// Our hole cards from the stored `hole` column (`AsKd`).
pub(crate) fn hole(text: &str) -> Option<[Card; 2]> {
    let a = Card::parse(text.get(0..2)?)?;
    let b = Card::parse(text.get(2..4)?)?;
    Some([a, b])
}

/// The all-in luck of one stored hand for our seat.
pub fn hand_luck(row: &HandRow) -> AllInLuck {
    let (Some(net), Some(seat)) = (row.net, row.hero_seat) else { return AllInLuck::NotAllIn };
    let Ok(summary) = serde_json::from_str::<HandSummary>(&row.summary) else { return AllInLuck::Unverifiable };
    hero_all_in_luck(&summary, seat as usize, hole(&row.hole), net, MAX_RUNOUTS, seed(&row.hand_id))
}

/// The luck-adjusted net of a stored hand: its all-in EV net when adjusted, else its net.
pub fn ev_net(row: &HandRow) -> Option<f64> {
    row.net.map(|net| net as f64 + hand_luck(row).adjustment())
}

/// Fill the all-in EV net of up to `limit` stored hands that lack one, in one transaction;
/// returns how many hands were read (fewer than `limit`: caught up).
pub fn fill_ev_nets(store: &sv10_store::store::Store, limit: usize) -> anyhow::Result<usize> {
    let rows = store.hands_missing_ev(limit)?;
    let filled: Vec<(String, String, f64)> = rows.iter().filter_map(|r| ev_net(r).map(|e| (r.bot.clone(), r.hand_id.clone(), e))).collect();
    store.set_ev_nets(&filled)?;
    Ok(rows.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_fill_stores_an_ev_net_for_every_hand_with_a_net() {
        let dir = std::env::temp_dir().join(format!("sv10-luck-fill-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let store = sv10_store::store::Store::open(&dir.join("svanbot10.db")).unwrap();
        for i in 0..5 {
            store
                .insert_hand(&HandRow {
                    bot: "A".into(),
                    hand_id: format!("h{i}"),
                    ended_at: format!("2026-09-24T00:00:0{i}Z"),
                    net: Some(10 * i),
                    summary: "{}".into(),
                    ..Default::default()
                })
                .unwrap();
        }
        assert_eq!(fill_ev_nets(&store, 3).unwrap(), 3);
        assert_eq!(fill_ev_nets(&store, 3).unwrap(), 2, "caught up");
        assert_eq!(fill_ev_nets(&store, 3).unwrap(), 0);
        // Unreadable summaries keep their net as the EV net.
        assert_eq!(store.bot_ev_results("A").unwrap()[4].1, Some(40.0));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_stored_row_without_a_result_or_summary_is_left_alone() {
        let row = HandRow { hand_id: "h".into(), net: None, ..Default::default() };
        assert_eq!(hand_luck(&row), AllInLuck::NotAllIn);
        assert_eq!(ev_net(&row), None);
        let row = HandRow { hand_id: "h".into(), net: Some(-40), hero_seat: Some(2), summary: "not json".into(), ..Default::default() };
        assert_eq!(hand_luck(&row), AllInLuck::Unverifiable);
        assert_eq!(ev_net(&row), Some(-40.0));
        assert_eq!(seed("abc"), seed("abc"));
        assert_ne!(seed("abc"), seed("abd"));
        assert_eq!(hole("AhKd"), Some([Card::parse("Ah").unwrap(), Card::parse("Kd").unwrap()]));
        assert_eq!(hole(""), None);
    }
}
