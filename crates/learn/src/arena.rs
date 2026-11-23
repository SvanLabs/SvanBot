//! The Arena (V11-15): where a policy's results are proven and shown. A policy plays paired
//! sims, heads up, against each sparring archetype, and the scoreboard lists its win rate over
//! each in mbb/100 with a 95% interval. A run is recorded with its commit and its seed, so every
//! number on the board can be played again.
//!
//! The deals come in **bouts** of [`BOUT`] (`GLOSSARY.md`). One sparring opponent plays a bout's
//! hands in order, so the learning archetype learns across them. Bouts do not share anything, so
//! they can run in any order on any number of threads. *A bout is not a session*: a session is a
//! bot's stay at a table (`GLOSSARY.md`), and only live play has those.

use svanbot_sim::sparring::{Archetype, Memory, Sparring};
use svanbot_sim::v10::seat::V10;

use crate::paired::{Challenger, Match, Verdict, duel_seen};
use crate::ring::{SIX_MAX, ring_seen};

mod reader;
pub use reader::{BoutReader, opponent_name};

/// Deals in a bout.
pub const BOUT: u64 = 50;

/// Plays bout `number` against `archetype`: its deals, each from mirrored seats, in order.
/// Returns what the challenger won on each deal, in big blinds a hand.
pub fn bout(
    game: &Match,
    archetype: Archetype,
    number: u64,
    challenger: &impl Challenger,
) -> Vec<f64> {
    bout_against(game, &Sparring::new(archetype), number, challenger)
}

/// [`bout`], against `opponent` as it stands: what it learns stays in it after the bout.
pub fn bout_against(
    game: &Match,
    opponent: &Sparring,
    number: u64,
    challenger: &impl Challenger,
) -> Vec<f64> {
    (number * BOUT..(number + 1) * BOUT)
        .map(|deal| {
            // Both mirrored hands meet the opponent as it was when this deal began: the learning
            // one would otherwise carry the first hand's bets into the second, and the deal's two
            // hands would no longer cancel the cards. What the deal taught it stays for the next.
            let start = opponent.memory();
            duel_seen(
                game,
                deal,
                challenger,
                &|_, spot, _, seed| opponent.decide(spot, seed, deal),
                || opponent.recall(start),
            )
        })
        .collect()
}

/// Plays 6-max bout `number` against five of `archetype`: its deals, each once from every seat
/// ([`ring_seen`]), in order. Each seat has its own opponent, and a learning one watches the
/// challenger's seat, not its own copies. Returns what the challenger won on each deal, in big blinds a hand.
pub fn ring_bout(
    game: &Match,
    archetype: Archetype,
    number: u64,
    challenger: &impl Challenger,
) -> Vec<f64> {
    ring_deals(
        game,
        archetype,
        number * BOUT..(number + 1) * BOUT,
        challenger,
    )
}

/// Plays consecutive six-seat deals with the same sparring opponents.
pub fn ring_deals(
    game: &Match,
    archetype: Archetype,
    numbers: std::ops::Range<u64>,
    challenger: &impl Challenger,
) -> Vec<f64> {
    let opponents: Vec<Sparring> = (0..SIX_MAX).map(|_| Sparring::new(archetype)).collect();
    numbers
        .map(|deal| {
            // Every rotation meets each opponent as it was when the deal began, as in [`bout`].
            let start: Vec<Memory> = opponents.iter().map(Sparring::memory).collect();
            ring_seen(
                game,
                SIX_MAX,
                deal,
                challenger,
                &|seat, spot, _, seed| opponents[seat].decide(spot, seed, deal),
                |us| {
                    for (opponent, &memory) in opponents.iter().zip(&start) {
                        opponent.recall(memory);
                        opponent.watch(us);
                    }
                },
            )
        })
        .collect()
}

/// Plays deal `number` heads up against v10.0.2 ([`V10`]), from mirrored seats. v10 keeps
/// nothing between hands, so a deal stands alone and needs no bout. Returns what the challenger
/// won, in big blinds a hand.
pub fn v10_duel(game: &Match, number: u64, challenger: &impl Challenger) -> f64 {
    let v10 = V10::default();
    duel_seen(
        game,
        number,
        challenger,
        &|seat, spot, seen, seed| v10.decide(seat, spot, seen, seed),
        || {},
    )
}

/// Plays deal `number` at a full table of v10.0.2, once from every seat ([`ring_seen`]).
pub fn v10_ring(game: &Match, number: u64, challenger: &impl Challenger) -> f64 {
    let v10 = V10::default();
    ring_seen(
        game,
        SIX_MAX,
        number,
        challenger,
        &|seat, spot, seen, seed| v10.decide(seat, spot, seen, seed),
        |_| {},
    )
}

/// One line of the scoreboard: an opponent, and the verdict against it.
#[derive(Clone, Debug, PartialEq)]
pub struct Row {
    pub opponent: String,
    pub verdict: Verdict,
}

/// The scoreboard as a table of text.
pub fn scoreboard(rows: &[Row]) -> String {
    let mut board = format!(
        "{:<18}{:>12}{:>10}{:>8}\n",
        "opponent", "mbb/100", "95% CI", "deals"
    );
    for row in rows {
        let verdict = &row.verdict;
        board.push_str(&format!(
            "{:<18}{:>+12.0}{:>10}{:>8}\n",
            row.opponent,
            verdict.mbb_per_100,
            format!("±{:.0}", verdict.ci95),
            verdict.deals
        ));
    }
    board
}
