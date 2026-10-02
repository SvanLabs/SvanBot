//! Alias-aware history reads for one fleet seat. Store rows retain the name used in play.
use super::*;
use sv10_store::store::{HandRow, PlayerHandWithBlind};

pub(super) fn recent_seat_hands(s: &Shared, name: &str, limit: usize, light: bool) -> Result<Vec<HandRow>, ApiError> {
    let mut rows = Vec::new();
    for alias in s.names_of(&s.current_name(name)) {
        rows.extend(store_read(
            "recent hands",
            if light { s.store.recent_hands_light(&alias, limit) } else { s.store.recent_hands(&alias, limit) },
        )?);
    }
    rows.sort_by(|a, b| b.ended_at.cmp(&a.ended_at));
    let mut seen = std::collections::HashSet::new();
    rows.retain(|r| seen.insert(r.hand_id.clone()));
    rows.truncate(limit);
    Ok(rows)
}

pub(super) fn seat_hand(s: &Shared, name: &str, id: &str) -> Result<Option<HandRow>, ApiError> {
    for alias in s.names_of(&s.current_name(name)) {
        if let Some(row) = store_read("hand", s.store.hand(&alias, id))? {
            return Ok(Some(row));
        }
    }
    Ok(None)
}

pub(super) fn player_hands(s: &Shared, name: &str) -> Result<Vec<PlayerHandWithBlind>, ApiError> {
    let mut rows = Vec::new();
    for alias in s.names_of(&s.current_name(name)) {
        rows.extend(store_read("hands with player", s.store.hands_with_player_with_blinds(&alias))?);
    }
    rows.sort_by(|a, b| a.hand.ended_at.cmp(&b.hand.ended_at));
    let mut seen = std::collections::HashSet::new();
    rows.retain(|r| seen.insert((r.hand.bot.clone(), r.hand.hand_id.clone())));
    Ok(rows)
}
