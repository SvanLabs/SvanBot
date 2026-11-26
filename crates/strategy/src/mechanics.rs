//! Grinder's and Gambler's mechanics (V11-101, spec M3–M5): three-bet ranges, size menus and
//! texture c-bets. Pure rules over a spot and the style; the pricer still fills the equity and
//! the price, and the re-solve is untouched.
//!
//! Every style but `tight` and `loose` carries [`NEUTRAL`](crate::style::Mechanics), so its spots
//! reach the pricer exactly as before.

use svanbot_core::card::{CardSet, Suit};
use svanbot_core::rng::Rng;
use svanbot_core::rules::{Action, Chips, Street};
use svanbot_core::spot::{Act, ActKind, Spot};

use crate::params::Params;
use crate::preflop::{self, Position};
use crate::style::{Mechanics, Sizes};

/// The stream the c-bet's frequency draw comes from: one draw a decision, from the decision's
/// own seed, so the same spot plays the same way on every machine.
const CBET_STREAM: u64 = 0x6362_6574;

/// A board's texture for M5: a flush draw or a run of connected ranks makes it wet.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Texture {
    Dry,
    Wet,
}

/// The texture of `board`: wet when two cards share a suit or three ranks sit within a span of
/// five, dry otherwise. A board of fewer than three cards (no flop yet) is dry.
pub fn texture(board: CardSet) -> Texture {
    let cards: Vec<_> = board.iter().collect();
    if cards.len() < 3 {
        return Texture::Dry;
    }
    let mut suits = [0u8; 4];
    for card in &cards {
        suits[match card.suit() {
            Suit::Clubs => 0,
            Suit::Diamonds => 1,
            Suit::Hearts => 2,
            Suit::Spades => 3,
        }] += 1;
    }
    if suits.iter().any(|&count| count >= 2) {
        return Texture::Wet;
    }
    let mut ranks: Vec<u8> = cards.iter().map(|card| card.rank() as u8).collect();
    ranks.sort_unstable();
    if ranks.windows(3).any(|three| three[2] - three[0] <= 4) {
        return Texture::Wet;
    }
    Texture::Dry
}

/// M3: the preflop three-bet, when the style has a rule and the class is in its range. `None`
/// leaves the spot to the pricer.
///
/// The spot must be the first re-raise: preflop, facing exactly one raise, with a raise open to
/// us. Grinder raises its best `three_bet_value` of classes by rank; Gambler also raises the
/// worst `three_bet_bluff` (polarised).
pub fn three_bet(spot: &Spot, mechanics: &Mechanics, params: &Params) -> Option<Action> {
    let rule = mechanics.three_bet?;
    if !spot.board.is_empty() || spot.legal.call.is_none() {
        return None;
    }
    let raises: usize = spot
        .lines
        .iter()
        .flatten()
        .filter(|act| act.street == Street::Preflop && matches!(act.kind, ActKind::Bet { .. }))
        .count();
    if raises != 1 {
        return None;
    }
    let class = preflop::class_of(spot.hole);
    let position = Position {
        behind: spot.players_behind.unwrap_or(spot.opponents),
        in_position: spot.in_position,
    };
    let rank = preflop::rank_at(class, position);
    let value = params.three_bet_value.clamp(0.0, 1.0);
    let bluff = if rule.polarised {
        params.three_bet_bluff.clamp(0.0, 1.0)
    } else {
        0.0
    };
    if !(rank < value || (bluff > 0.0 && rank > 1.0 - bluff)) {
        return None;
    }
    let to = menu(&crate::pricer::raise_sizes(spot), mechanics.sizes)
        .first()
        .copied()?;
    Some(Action::RaiseTo(to))
}

/// M4: the sizes a style may name from the pricer's own menu, which is ascending and empty when
/// a raise is not open.
pub fn menu(sizes: &[Chips], menu: Sizes) -> Vec<Chips> {
    match sizes {
        [] => Vec::new(),
        [only] => vec![*only],
        many => match menu {
            Sizes::All => many.to_vec(),
            Sizes::Small => vec![many[0]],
            Sizes::Big => vec![many[many.len() - 1]],
        },
    }
}

/// M5: a style's texture c-bet when checked to, at its frequency and size. `None` leaves the
/// spot to the pricer.
pub fn cbet(spot: &Spot, mechanics: &Mechanics, params: &Params, seed: u64) -> Option<Action> {
    let rule = mechanics.cbet?;
    if spot.board.is_empty() || spot.legal.call.is_some() || spot.legal.raise.is_none() {
        return None;
    }
    let wet = texture(spot.board) == Texture::Wet;
    let chance = match (wet, rule.dry, rule.wet) {
        (true, _, true) => params.cbet_wet.clamp(0.0, 1.0),
        (false, true, _) => params.cbet_dry.clamp(0.0, 1.0),
        _ => 0.0,
    };
    if chance <= 0.0 || Rng::stream(seed, CBET_STREAM).below(1_000) as f64 >= chance * 1_000.0 {
        return None;
    }
    let share = if rule.big {
        params.size_big
    } else {
        params.size_small
    }
    .clamp(0.0, 1.0);
    let after_call = spot.pot;
    let to = spot.committed + ((after_call as f64) * share) as Chips;
    let legal = spot.legal.raise?;
    Some(Action::RaiseTo(to.clamp(legal.min, legal.max)))
}

/// The number of preflop raises an opponent's line made, for the tests that read a spot.
pub fn preflop_raises(line: &[Act]) -> usize {
    line.iter()
        .filter(|act| act.street == Street::Preflop && matches!(act.kind, ActKind::Bet { .. }))
        .count()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::style::{LOOSE, STANDARD, TIGHT};
    use svanbot_core::rules::Legal;

    fn params() -> Params {
        Params::DEFAULT
    }

    fn preflop_spot(hole: &str) -> Spot {
        Spot {
            hole: hole.parse().expect("a hole"),
            board: CardSet::EMPTY,
            pot: 90,
            callable_pot: None,
            legal: Legal {
                call: Some(60),
                raise: Some(svanbot_core::rules::RaiseRange {
                    min: 120,
                    max: 2_000,
                }),
            },
            opponents: 1,
            stack: 1_900,
            committed: 10,
            big_blind: 20,
            in_position: true,
            players_behind: Some(1),
            lines: vec![vec![Act {
                street: Street::Preflop,
                board: CardSet::EMPTY,
                facing: false,
                kind: ActKind::Bet { added: 60, pot: 30 },
            }]],
            seats: vec![2],
        }
    }

    fn flop_spot(board: &str) -> Spot {
        Spot {
            hole: "AhKh".parse().expect("a hole"),
            board: board.parse().expect("a board"),
            pot: 100,
            callable_pot: None,
            legal: Legal {
                call: None,
                raise: Some(svanbot_core::rules::RaiseRange {
                    min: 20,
                    max: 2_000,
                }),
            },
            opponents: 1,
            stack: 1_900,
            committed: 0,
            big_blind: 20,
            in_position: true,
            players_behind: Some(1),
            lines: vec![vec![Act {
                street: Street::Flop,
                board: board.parse().expect("a board"),
                facing: false,
                kind: ActKind::Check,
            }]],
            seats: vec![2],
        }
    }

    #[test]
    fn a_flush_draw_or_a_run_of_ranks_is_wet() {
        assert_eq!(texture("Qh7c2d".parse().unwrap()), Texture::Dry);
        assert_eq!(texture("QhJhTh".parse().unwrap()), Texture::Wet);
        assert_eq!(texture("9h8c7d".parse().unwrap()), Texture::Wet);
        assert_eq!(texture("AhKd7c2s".parse().unwrap()), Texture::Dry);
    }

    #[test]
    fn tight_three_bets_its_value_and_never_bluffs() {
        let spot = preflop_spot("AsAc");
        assert!(matches!(
            three_bet(&spot, &TIGHT.mechanics, &params()),
            Some(Action::RaiseTo(_))
        ));
        // 72o is the worst class: the tight rule leaves it to the pricer.
        assert_eq!(
            three_bet(&preflop_spot("7s2c"), &TIGHT.mechanics, &params()),
            None
        );
        // The standard style has no rule at all.
        assert_eq!(three_bet(&spot, &STANDARD.mechanics, &params()), None);
    }

    #[test]
    fn loose_three_bets_a_polarised_range() {
        let tight = TIGHT.mechanics;
        assert!(tight.three_bet.is_some());
        let bluff = Params {
            three_bet_bluff: 0.2,
            ..Params::DEFAULT
        };
        // With a bluff share, the worst class is a three-bet for Gambler.
        assert!(matches!(
            three_bet(&preflop_spot("7s2c"), &LOOSE.mechanics, &bluff),
            Some(Action::RaiseTo(_))
        ));
        // Without one (the default), even the loosest rule leaves it.
        assert_eq!(
            three_bet(&preflop_spot("7s2c"), &LOOSE.mechanics, &params()),
            None
        );
    }

    #[test]
    fn the_size_menu_takes_the_small_or_the_big_end() {
        let sizes = [33, 66, 100];
        assert_eq!(menu(&sizes, Sizes::All), vec![33, 66, 100]);
        assert_eq!(menu(&sizes, Sizes::Small), vec![33]);
        assert_eq!(menu(&sizes, Sizes::Big), vec![100]);
        assert_eq!(menu(&[], Sizes::Big), Vec::<Chips>::new());
    }

    #[test]
    fn grinder_bets_dry_boards_small_and_gambler_wet_ones_big() {
        let always = Params {
            cbet_dry: 1.0,
            cbet_wet: 1.0,
            ..Params::DEFAULT
        };
        let dry = flop_spot("Qh7c2d");
        let wet = flop_spot("QhJhTh");
        assert_eq!(
            cbet(&dry, &TIGHT.mechanics, &always, 7),
            Some(Action::RaiseTo(33)),
            "Grinder bets a dry board for a third of the pot"
        );
        assert_eq!(
            cbet(&wet, &TIGHT.mechanics, &always, 7),
            None,
            "not a wet board"
        );
        assert_eq!(
            cbet(&wet, &LOOSE.mechanics, &always, 7),
            Some(Action::RaiseTo(100)),
            "Gambler bets a wet board for the pot"
        );
        assert_eq!(cbet(&dry, &LOOSE.mechanics, &always, 7), None);
        // A zero frequency never bets, and the standard style has no rule.
        assert_eq!(cbet(&dry, &TIGHT.mechanics, &params(), 7), None);
        assert_eq!(cbet(&dry, &STANDARD.mechanics, &always, 7), None);
    }
}
