//! Decision input (`Situation`) from the tracked table and a `your_turn` frame.

use super::*;

impl TableTracker {
    /// Decision input for an action-authority message (`your_turn`).
    pub fn situation(&mut self, msg: &Value, legal: &LegalActions) -> Option<Situation> {
        let hero_seat = msg["seat"].as_u64().map(|s| s as usize).or(self.hero_seat)?;
        self.hero_seat = Some(hero_seat);
        for p in msg["players"].as_array().into_iter().flatten() {
            if let (Some(seat), Some(stack)) = (p["seat"].as_u64(), p["stack"].as_i64())
                && let Some(s) = self.seats.get_mut(&(seat as usize))
            {
                s.stack = stack;
            }
        }
        let board = if msg.get("community_cards").is_some() { cards(&msg["community_cards"]) } else { self.board.clone() };
        let street = match board.len() {
            0 => Street::Preflop,
            3 => Street::Flop,
            4 => Street::Turn,
            _ => Street::River,
        };
        let hole = self.hole?;
        // The hero must be seated in our view of the table, or there is nothing safe to decide.
        if !self.seats.contains_key(&hero_seat) {
            return None;
        }
        let mut history = self.history.clone();
        if street == Street::Preflop {
            // After a resync without replay, rebuild missing preflop raises from the bets on the table.
            let has_aggr = history.iter().any(|r| r.street == Street::Preflop && sv10_model::model::aggressive(r));
            if !has_aggr {
                let mut raisers: Vec<&SeatView> = self.seats.values().filter(|s| s.in_hand && s.bet > self.bb.max(1)).collect();
                raisers.sort_by_key(|s| s.bet);
                // Geometry (0092): only the blinds were in before the first raise (raisers other than
                // the blinds had nothing in), each raise adds its excess over what that seat had posted,
                // and the price a raiser faced is the level above its own post.
                let seats: Vec<usize> = self.seats.keys().copied().collect();
                let post = |seat: usize| match sv10_engine::situation::position_of(&seats, self.dealer, seat) {
                    sv10_engine::situation::Position::SmallBlind => (self.bb / 2).max(1),
                    sv10_engine::situation::Position::BigBlind => self.bb.max(1),
                    _ => 0,
                };
                let raiser_seats: Vec<usize> = raisers.iter().map(|r| r.seat).collect();
                let mut level = self.bb.max(1);
                let mut pot = self
                    .seats
                    .values()
                    .map(|s| if raiser_seats.contains(&s.seat) { post(s.seat).min(s.bet) } else { s.bet })
                    .fold(0i64, |a, b| a.saturating_add(b));
                for r in raisers {
                    if r.bet > level {
                        let had_in = post(r.seat).min(r.bet);
                        history.push(ActionRecord {
                            seat: r.seat,
                            street: Street::Preflop,
                            kind: ActionKind::Raise,
                            to: r.bet,
                            pot_before: pot,
                            to_call_before: level.saturating_sub(had_in),
                            bet_before: had_in,
                            full_raise: true,
                            think_ms: None,
                            street_open: false,
                        });
                        pot = pot.saturating_add(r.bet.saturating_sub(had_in));
                        level = r.bet;
                    }
                }
            }
        }
        let players: Vec<PlayerInfo> = self
            .seats
            .values()
            .filter(|s| s.in_hand || s.seat == hero_seat)
            .map(|s| PlayerInfo {
                seat: s.seat,
                name: s.name.clone(),
                // Anti-corruption: amounts from the wire are never negative in a real game.
                stack: s.stack.max(0),
                bet: s.bet.max(0),
                folded: s.folded && s.seat != hero_seat,
            })
            .collect();
        if players.len() < 2 {
            return None;
        }
        let bets = players.iter().map(|p| p.bet).fold(0i64, |a, b| a.saturating_add(b));
        let (min_raise_to, max_raise_to) = match (legal.raise_min, legal.raise_max) {
            (Some(a), Some(b)) if a > 0 && b > 0 => (Some(a), Some(b)),
            _ => match legal.all_in {
                Some(a) if a > 0 => (Some(a), Some(a)),
                _ => (None, None),
            },
        };
        let hero_bet = players.iter().find(|player| player.seat == hero_seat).map_or(0, |player| player.bet);
        let posted = players.iter().map(|player| player.bet).max().unwrap_or(0);
        let call_amount = legal.call.unwrap_or(0).max(0);
        Some(Situation {
            hero_seat,
            hole,
            board,
            street,
            button: self.dealer,
            bb: self.bb.max(1),
            // A missing or impossible (negative) pot falls back to the tracked pot, then to the bets.
            pot: msg["pot"].as_i64().filter(|p| *p >= 0).unwrap_or(if self.pot >= 0 { self.pot } else { bets }),
            call_amount,
            current_bet_to: Some(posted.max(hero_bet.saturating_add(call_amount))),
            can_check: legal.can_check,
            min_raise_to,
            max_raise_to,
            players,
            history,
        })
    }
}
