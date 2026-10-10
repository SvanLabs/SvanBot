//! Opponent hand results with the blind recorded by each hand.

use super::*;

/// A hand with its own positive big blind, if the summary records one.
#[derive(Clone, Debug, PartialEq)]
pub struct PlayerHandWithBlind {
    /// The existing chip result and replay details.
    pub hand: PlayerHand,
    /// Positive integer blind from this hand's summary, or none if absent or invalid.
    pub big_blind: Option<i64>,
}

impl Store {
    /// All hands a player was dealt into, oldest first. An invalid summary can still match the
    /// player's name, but cannot contribute a blind to normalized rates.
    pub fn hands_with_player_with_blinds(&self, player: &str) -> Result<Vec<PlayerHandWithBlind>> {
        // A cheap prefilter on the JSON-escaped name, then the exact test: the name is a seat in the players
        // list. The prefilter alone also matched a card code, the last card of a board (#946). A summary that
        // is not JSON keeps the text match, as before.
        let needle = serde_json::Value::String(player.to_string()).to_string();
        let conn = self.read();
        let mut stmt = conn.prepare(
            "SELECT bot, hand_id, ended_at, net, ev_net, COALESCE(pot, 0), COALESCE(winners, ''),
                    CASE WHEN json_valid(summary) THEN
                        CASE WHEN json_type(summary, '$.bb') = 'integer' AND json_extract(summary, '$.bb') > 0
                            THEN json_extract(summary, '$.bb') END
                    END
             FROM hands WHERE instr(summary, ?1) > 0
               AND CASE WHEN json_valid(summary) THEN EXISTS (
                     SELECT 1 FROM json_each(summary, '$.players') AS seat WHERE json_extract(seat.value, '$[1]') = ?2
                   ) ELSE 1 END
             ORDER BY ended_at",
        )?;
        let rows = stmt
            .query_map(params![needle, player], |r| {
                Ok(PlayerHandWithBlind {
                    hand: PlayerHand {
                        bot: r.get(0)?,
                        hand_id: r.get(1)?,
                        ended_at: r.get(2)?,
                        net: r.get(3)?,
                        ev_net: r.get(4)?,
                        pot: r.get(5)?,
                        winners: r.get(6)?,
                    },
                    big_blind: r.get(7)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }
}
