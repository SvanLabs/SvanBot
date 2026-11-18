//! The Arena (V11-15): where a policy's results are proven and shown. A policy plays paired
//! sims, heads up, against each sparring archetype, and the scoreboard lists its win rate over
//! each in mbb/100 with a 95% interval. A run is recorded with its commit and its seed, so every
//! number on the board can be played again.
//!
//! The deals come in **bouts** of [`BOUT`] (`GLOSSARY.md`). One sparring opponent plays a bout's
//! hands in order, so the learning archetype learns across them. Bouts do not share anything, so
//! they can run in any order on any number of threads. *A bout is not a session*: a session is a
//! bot's stay at a table (`GLOSSARY.md`), and only live play has those.

use svanbot_core::rules::Action;
use svanbot_core::spot::Spot;
use svanbot_sim::sparring::{Archetype, Sparring};

use crate::paired::{Match, Verdict, duel_with};

/// Deals in a bout.
pub const BOUT: u64 = 50;

/// Plays bout `number` against `archetype`: its deals, each from mirrored seats, in order.
/// Returns what the challenger won on each deal, in big blinds a hand.
pub fn bout(
    game: &Match,
    archetype: Archetype,
    number: u64,
    challenger: &impl Fn(&Spot, u64) -> Action,
) -> Vec<f64> {
    let opponent = Sparring::new(archetype);
    (number * BOUT..(number + 1) * BOUT)
        .map(|deal| {
            // Both mirrored hands meet the opponent as it was before this deal: the learning one
            // would otherwise carry the first hand's bets into the second, and the deal's two
            // hands would no longer cancel the cards.
            duel_with(
                game,
                deal,
                challenger,
                &|spot, seed| opponent.decide(spot, seed, deal),
                || opponent.reset(),
            )
        })
        .collect()
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
