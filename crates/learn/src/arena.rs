//! The Arena (V11-15): where a policy's results are proven and shown. A policy plays paired
//! sims, heads up, against each sparring archetype, and the scoreboard lists its win rate over
//! each in mbb/100 with a 95% interval. A run is recorded with its commit and its seed, so every
//! number on the board can be played again.
//!
//! The deals come in **sessions** of [`SESSION`]. One sparring opponent plays a session's hands
//! in order, so the learning archetype learns across them. Sessions do not share anything, so
//! they can run in any order on any number of threads.

use svanbot_core::rules::Action;
use svanbot_core::spot::Spot;
use svanbot_sim::sparring::{Archetype, Sparring};

use crate::paired::{Match, Verdict, duel};

/// Deals in a session.
pub const SESSION: u64 = 50;

/// Plays session `number` against `archetype`: its deals, each from mirrored seats, in order.
/// Returns what the challenger won on each deal, in big blinds a hand.
pub fn session(
    game: &Match,
    archetype: Archetype,
    number: u64,
    challenger: &impl Fn(&Spot, u64) -> Action,
) -> Vec<f64> {
    let opponent = Sparring::new(archetype);
    (number * SESSION..(number + 1) * SESSION)
        .map(|deal| {
            duel(game, deal, challenger, &|spot, seed| {
                opponent.decide(spot, seed, deal)
            })
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

/// The run as JSON, for the dashboard's scoreboard: the commit and seed it can be played again
/// from, and each row.
pub fn json(rows: &[Row], commit: &str, seed: u64) -> String {
    let rows: Vec<String> = rows
        .iter()
        .map(|row| {
            format!(
                r#"{{"opponent":"{}","deals":{},"mbb_per_100":{:?},"ci95":{:?}}}"#,
                escape(&row.opponent),
                row.verdict.deals,
                row.verdict.mbb_per_100,
                row.verdict.ci95
            )
        })
        .collect();
    format!(
        r#"{{"commit":"{}","seed":{seed},"rows":[{}]}}"#,
        escape(commit),
        rows.join(",")
    )
}

/// `text` safe inside a JSON string: the quote, the backslash and control characters escaped.
fn escape(text: &str) -> String {
    text.chars()
        .flat_map(|c| match c {
            '"' => vec!['\\', '"'],
            '\\' => vec!['\\', '\\'],
            c if c.is_control() => format!("\\u{:04x}", c as u32).chars().collect(),
            c => vec![c],
        })
        .collect()
}
