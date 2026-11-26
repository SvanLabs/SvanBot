//! A bot's own **field**: a clone of the players that bot meets, fitted from the opponent sides of
//! the hands in its own store. The field is one action model over the whole population it has
//! played, so nothing in it names a bot, a seat or an opponent: only aggregate action statistics
//! enter it (`docs/adr/0004-reads-are-per-bot.md`). One bot's field is its own, fitted in its own
//! process or by its own command, never by `svanbot learn`.
//!
//! The field is not gated: it does not decide the bot's play, so it has no incumbent to beat. It
//! stands in for the players only in the sim, as one Arena row recorded apart from the release
//! rows: its name is [`row_name`], and it is not in `arena::Opponent::all()`
//! (`docs/superpowers/specs/2026-11-25-five-brains-design.md`, Phase 3 "Rows").

use svanbot_core::card::CardSet;
use svanbot_core::rng::Rng;
use svanbot_core::rules::{Action, Chips, Street};
use svanbot_core::spot::Spot;
use svanbot_strategy::action::{ActionModel, BUCKETS, Move, Situation, buckets};

use crate::fit::{Observed, fit as em_fit, prepare};

/// The first line of a field written as text.
pub const HEADER: &str = "svanbot field 1";

/// What every field row's name begins with: `field <bot>`.
pub const PREFIX: &str = "field ";

/// A fitted field: the action model the population plays, and how many opponent-hands made it.
#[derive(Clone, Debug, PartialEq)]
pub struct Field {
    model: ActionModel,
    /// Opponent-hands with at least one action, the evidence behind the model.
    sides: usize,
}

impl Field {
    /// Fits the field from the opponent sides of `observed`, by `rounds` rounds of EM from the
    /// uniform model. `alpha` is the Dirichlet prior on each move's count. `None` without one
    /// opponent-hand that could be fitted.
    pub fn fit(
        observed: impl IntoIterator<Item = Observed>,
        rounds: usize,
        alpha: f64,
    ) -> Option<Self> {
        let prepared: Vec<_> = observed
            .into_iter()
            .map(|side| prepare(&side))
            .filter(|side| side.candidates() > 0)
            .collect();
        if prepared.is_empty() {
            return None;
        }
        let sides = prepared.len();
        Some(Self {
            model: em_fit(&prepared, rounds, alpha).model,
            sides,
        })
    }

    /// The fitted action model: how the population plays at each situation and strength bucket.
    pub fn model(&self) -> &ActionModel {
        &self.model
    }

    /// Opponent-hands with at least one action, the evidence behind the field.
    pub fn sides(&self) -> usize {
        self.sides
    }

    /// The field as text: its header, its evidence, then the model document. Apart from the two
    /// fixed headers no line holds anything but numbers, so no identity can be written with it.
    pub fn to_text(&self) -> String {
        format!("{HEADER}\nsides {}\n{}", self.sides, self.model.to_text())
    }

    /// The field `text` holds, or `None` unless it is whole.
    pub fn from_text(text: &str) -> Option<Self> {
        let rest = text.strip_prefix(HEADER)?.strip_prefix('\n')?;
        let (sides, model) = rest.split_once('\n')?;
        Some(Self {
            model: ActionModel::from_text(model)?,
            sides: sides.strip_prefix("sides ")?.parse().ok()?,
        })
    }

    /// The field's action at `spot`, drawing from `seed`: the move its model gives the spot's
    /// situation and the player's strength bucket, made legal and sized by the move.
    pub fn decide(&self, spot: &Spot, seed: u64) -> Action {
        let situation = Situation {
            street: Street::of(spot.board),
            facing: spot.legal.call.is_some(),
        };
        let bucket = bucket(spot);
        let total: f64 = situation
            .moves()
            .iter()
            .map(|&mv| self.model.prob(situation, bucket, mv))
            .sum();
        let mut roll = roll(seed) * total;
        let mut chosen = *situation.moves().last().unwrap_or(&Move::Check);
        for &mv in situation.moves() {
            let weight = self.model.prob(situation, bucket, mv);
            if roll < weight {
                chosen = mv;
                break;
            }
            roll -= weight;
        }
        legal(spot, chosen)
    }
}

/// A uniform draw in `[0, 1)` from `seed`.
fn roll(seed: u64) -> f64 {
    (Rng::stream(seed, 1).next_u64() >> 11) as f64 / (1u64 << 53) as f64
}

/// The strength bucket of the acting player's hole cards at `spot`: where they rank among every
/// pair the board leaves possible. The middle bucket when the cards do not name a pair.
fn bucket(spot: &Spot) -> usize {
    buckets(spot.board, CardSet::EMPTY)
        .into_iter()
        .find(|&(pair, _)| pair == spot.hole)
        .map_or(BUCKETS / 2, |(_, bucket)| bucket)
}

/// `mv` as a legal action at `spot`. A bet or raise adds its share of the pot, and a move the
/// spot does not open falls back to the nearest legal passive action.
fn legal(spot: &Spot, mv: Move) -> Action {
    let passive = if spot.legal.call.is_some() {
        Action::Call
    } else {
        Action::Check
    };
    match mv {
        Move::Fold if spot.legal.call.is_some() => Action::Fold,
        Move::Fold | Move::Check | Move::Call => passive,
        Move::Small | Move::Medium | Move::Large => raise(spot, share(mv)),
    }
}

/// A bet or raise adding `share` of the pot, or the nearest legal action when none is open.
fn raise(spot: &Spot, share: f64) -> Action {
    match (spot.legal.raise, spot.legal.call) {
        (Some(range), _) => {
            let added = (spot.pot as f64 * share) as Chips;
            Action::RaiseTo((spot.committed + added).clamp(range.min, range.max))
        }
        (None, Some(_)) => Action::Call,
        (None, None) => Action::Check,
    }
}

/// The share of the pot a sized move adds.
fn share(mv: Move) -> f64 {
    match mv {
        Move::Small => 0.4,
        Move::Medium => 0.75,
        _ => 1.0,
    }
}

/// The name a bot's field plays under, `field <bot>`, distinct from every release row and from
/// every other bot's field.
pub fn row_name(bot: &str) -> String {
    format!("{PREFIX}{bot}")
}

/// Whether `name` names a field row.
pub fn is_row(name: &str) -> bool {
    name.starts_with(PREFIX)
}
