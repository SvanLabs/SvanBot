//! The page's JSON views of one bot's store: its newest traced hands and one hand's decisions.
//!
//! They open the store read-only. A name no bot can have is a 404, a store that is there and
//! cannot be read is a 500, and a bot that has played nothing is an empty list.

use serde_json::{Value, json};
use svanbot_runtime::store::Store;
use svanbot_strategy::policy::{Branch, Ranged, Trace};

use super::{Config, INTERNAL_ERROR, page};

/// The newest traced hands of one bot, newest first — what the replayer's list opens.
///
/// A bot with no store yet has played nothing, which is an empty list. A name no bot can have,
/// or a store that cannot be read, is not an empty list: one is a 404 and the other a 500, since
/// a page that answered "this bot has no hands" to every question would hide a mistyped name.
pub(super) fn hands_json(config: &Config, bot: &str) -> Option<(u16, String)> {
    if !page::is_name(bot) {
        return None;
    }
    let path = config.store_of(bot);
    let store = match Store::open_reader(&path) {
        Ok(store) => store,
        Err(_) if !path.exists() => {
            return Some((200, json!({ "bot": bot, "hands": [] }).to_string()));
        }
        Err(error) => {
            return Some((
                INTERNAL_ERROR,
                json!({ "detail": format!("{bot}'s store cannot be read: {error}") }).to_string(),
            ));
        }
    };
    let hands = match store.traced_hands() {
        Ok(hands) => hands,
        Err(error) => {
            return Some((
                INTERNAL_ERROR,
                json!({ "detail": format!("{bot}'s hands cannot be read: {error}") }).to_string(),
            ));
        }
    };
    let hands: Vec<Value> = hands
        .into_iter()
        .map(|hand| {
            json!({
                "hand_id": hand.hand_id,
                "table_id": hand.table_id,
                "recorded_ms": hand.recorded_ms,
                "decisions": hand.decisions,
                "quality": hand
                    .quality
                    .as_deref()
                    .and_then(|text| serde_json::from_str::<Value>(text).ok()),
            })
        })
        .collect();
    Some((200, json!({ "bot": bot, "hands": hands }).to_string()))
}

/// One hand's decisions, oldest first. `None` when the store does not hold the hand at all; a hand
/// it holds with no decisions is `"traced": false`, with the note the page reads, because a hand
/// played before the trace existed is not a hand that made no decisions.
///
/// A store that is there and cannot be read is neither: it is `INTERNAL_ERROR`, the same as the
/// hands list, so a corrupt store does not read as a hand that was never played.
pub(super) fn trace_json(config: &Config, route: &str) -> Option<(u16, String)> {
    let (bot, hand_id) = route.split_once('/')?;
    if !page::is_name(bot) || !page::is_name(hand_id) {
        return None;
    }
    let path = config.store_of(bot);
    let store = match Store::open_reader(&path) {
        Ok(store) => store,
        // A bot that has played nothing has no store, and no store holds no hands: the same answer
        // the hands list gives, so the two routes agree about a bot that has just started.
        Err(_) if !path.exists() => return None,
        Err(error) => {
            return Some((
                INTERNAL_ERROR,
                json!({ "detail": format!("{bot}'s store cannot be read: {error}") }).to_string(),
            ));
        }
    };
    // The store opens lazily, so a file that is not a database fails here rather than at
    // `open_reader`: a read error on a store that is there is a 500, not a hand nobody played.
    let holds = match store.holds(hand_id) {
        Ok(holds) => holds,
        Err(error) => {
            return Some((
                INTERNAL_ERROR,
                json!({ "detail": format!("{bot}'s store cannot be read: {error}") }).to_string(),
            ));
        }
    };
    if !holds {
        return None;
    }
    let traces = match store.traces(hand_id) {
        Ok(traces) => traces,
        // A decision the codec cannot read is not a hand with no trace: the hands list counts
        // that decision, so answering "no trace" here would show the page an empty hand the
        // store says it holds decisions for.
        Err(error) => {
            return Some((
                INTERNAL_ERROR,
                json!({ "detail": format!("{bot}'s trace cannot be read: {error}") }).to_string(),
            ));
        }
    };
    match traces {
        Some(traces) => {
            let decisions: Vec<Value> = traces.iter().map(trace_value).collect();
            Some((
                200,
                json!({ "bot": bot, "hand_id": hand_id, "traced": true, "decisions": decisions })
                    .to_string(),
            ))
        }
        None => Some((
            200,
            json!({
                "bot": bot,
                "hand_id": hand_id,
                "traced": false,
                "decisions": [],
                "note": "no trace — this hand was played before traces were recorded, or it has been pruned",
            })
            .to_string(),
        )),
    }
}

/// The trace as the page reads it. The street and branch are named where they are defined
/// (`Street::name`, `Branch::name`), so the page and the store cannot spell them apart.
fn trace_value(trace: &Trace) -> Value {
    let mut value = json!({
        "street": trace.street.name(),
        "branch": Branch::name(trace.branch),
        "hole": trace.hole.to_string(),
        "board": trace.board.to_string(),
    });
    if let Some(resolved) = &trace.resolved {
        let actions: Vec<String> = resolved
            .actions
            .iter()
            .map(|action| action.named())
            .collect();
        value["values"] = json!(resolved.values);
        value["strategy"] = json!(resolved.strategy);
        value["taken"] = json!(resolved.taken);
        value["actions"] = json!(actions);
        value["ranges"] = json!(ranges_value(&resolved.ranges));
    }
    if let Some(priced) = &trace.priced {
        value["equity"] = json!(priced.equity);
        value["price"] = json!(priced.price);
        value["exploit"] = json!(priced.exploit);
        value["ranges"] = json!(ranges_value(&priced.ranges));
    }
    value
}

/// Each range's seat, as the trace keeps it. A seat the spot did not carry is `null`: the page
/// says the seat is unknown rather than numbering the opponents itself, which would put a label
/// on a player the table view does not show.
fn ranges_value(ranges: &[Ranged]) -> Vec<Value> {
    ranges
        .iter()
        .map(|ranged| {
            // The class and trust are `null` when the population's model read the opponent.
            json!({
                "seat": ranged.seat,
                "masses": ranged.masses,
                "class": ranged.class.map(|class| class.name()),
                "trust": ranged.trust,
            })
        })
        .collect()
}
