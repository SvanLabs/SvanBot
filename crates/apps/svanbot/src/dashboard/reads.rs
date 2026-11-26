//! One bot's own opponent stats, rebuilt from its store for the page: every rate beside its
//! counts, a display interval once the evidence clears the floor, and the floor said out loud.
//!
//! Reads are per bot (`docs/adr/0004-reads-are-per-bot.md`): this opens one bot's own store,
//! read-only, and never crosses an opponent's record to another bot. It sits apart from the
//! fleet's write path, like the hands and trace routes beside it.

use std::collections::BTreeMap;

use serde_json::{Value, json};
use svanbot_core::rules::Street;
use svanbot_runtime::history;
use svanbot_runtime::store::Store;
use svanbot_strategy::action::Situation;
use svanbot_strategy::profile::{MIN_EVIDENCE_HANDS, Rate};

use super::{Config, INTERNAL_ERROR, page};

/// The prior strength the display interval is drawn with: the engine's own shrinkage strength
/// (`read::Trust::DEFAULT.prior`). The prior is the neutral Beta(10, 10); this is presentation,
/// and the engine's estimator does not change.
const PRIOR_STRENGTH: f64 = 20.0;

/// The two-sided 95% normal quantile, for the approximation the page labels as one.
const Z95: f64 = 1.96;

/// How many of an opponent's hands are linked behind its stats.
const HAND_LINKS: usize = 50;

/// A display-only 95% interval for a share: a Beta posterior under the neutral prior at the
/// engine's prior strength, read by its normal approximation. `None` under the evidence floor,
/// where the page shows the floor message instead of a number that would look certain.
fn interval(hits: usize, opportunities: usize) -> Option<[f64; 2]> {
    if opportunities < MIN_EVIDENCE_HANDS {
        return None;
    }
    let half = PRIOR_STRENGTH / 2.0;
    let n = opportunities as f64;
    let mean = (half + hits as f64) / (PRIOR_STRENGTH + n);
    let sd = (mean * (1.0 - mean) / (PRIOR_STRENGTH + n + 1.0)).sqrt();
    Some([(mean - Z95 * sd).max(0.0), (mean + Z95 * sd).min(1.0)])
}

/// One statistic as the page takes it: counts always, a rate and interval only past the floor.
fn stat(rate: Rate) -> Value {
    let (hits, opportunities) = (rate.hits(), rate.opportunities());
    json!({
        "hits": hits,
        "opportunities": opportunities,
        "rate": rate.value(),
        "interval": interval(hits, opportunities),
        "floor": rate.value().is_none(),
    })
}

/// One opponent: its name and class, the four profile stats, the situation counts (a cell thin
/// below the floor is flagged), and the hands behind them for the replayer.
fn opponent_json(bot: &str, opponent: &svanbot_learn::reads::Opponent, hands: &[String]) -> Value {
    let profile = opponent.profile_stats();
    let situations: Vec<Value> = Street::ALL
        .into_iter()
        .flat_map(|street| [false, true].map(|facing| (street, facing)))
        .map(|(street, facing)| {
            let n = opponent.read().n(Situation { street, facing });
            json!({
                "street": street.name(),
                "facing": facing,
                "n": n,
                "sparse": n > 0 && (n as usize) < MIN_EVIDENCE_HANDS,
            })
        })
        .collect();
    let hands: Vec<&String> = hands.iter().rev().take(HAND_LINKS).rev().collect();
    json!({
        "bot": bot,
        "name": opponent.name(),
        "hands": opponent.hands(),
        "class": opponent.class().name(),
        "earned": opponent.earned(),
        "stats": {
            "vpip": stat(profile.vpip),
            "pfr": stat(profile.pfr),
            "fold_to_bet": stat(profile.fold_to_bet),
            "aggression": stat(profile.aggression),
        },
        "situations": situations,
        "hand_ids": hands,
    })
}

/// The JSON answer for `/api/reads/<bot>`. `None` when the name is no bot's; a bot that has
/// played nothing has no store, and no store is an empty list, as the hands route answers.
pub(super) fn reads_json(config: &Config, bot: &str) -> Option<(u16, String)> {
    if !page::is_name(bot) {
        return None;
    }
    let empty = || Some((200, json!({ "bot": bot, "opponents": [] }).to_string()));
    let path = config.store_of(bot);
    let store = match Store::open_reader(&path) {
        Ok(store) => store,
        Err(_) if !path.exists() => return empty(),
        Err(error) => {
            return Some((
                INTERNAL_ERROR,
                json!({ "detail": format!("{bot}'s store cannot be read: {error}") }).to_string(),
            ));
        }
    };
    let hands = match store.hands() {
        Ok(hands) => hands,
        Err(error) => {
            return Some((
                INTERNAL_ERROR,
                json!({ "detail": format!("{bot}'s hands cannot be read: {error}") }).to_string(),
            ));
        }
    };
    let reads = history::reads(&hands, bot);
    // The hands each opponent acted in, in the order they were kept, for the page's links.
    let mut acted: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for hand in &hands {
        for witness in history::witnesses(hand, bot) {
            acted
                .entry(witness.name.to_lowercase())
                .or_default()
                .push(hand.hand_id.clone());
        }
    }
    let opponents: Vec<Value> = reads
        .opponents()
        .map(|opponent| {
            let ids = acted
                .get(&opponent.name().to_lowercase())
                .cloned()
                .unwrap_or_default();
            opponent_json(bot, opponent, &ids)
        })
        .collect();
    Some((
        200,
        json!({ "bot": bot, "opponents": opponents }).to_string(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_interval_waits_for_the_floor_and_stays_a_share() {
        assert_eq!(interval(2, 3), None);
        let [lo, hi] = interval(30, 30).unwrap();
        assert!(0.0 <= lo && lo < hi && hi <= 1.0);
        let [lo, hi] = interval(0, 30).unwrap();
        assert!(0.0 <= lo && lo < hi && hi <= 1.0);
    }
}
