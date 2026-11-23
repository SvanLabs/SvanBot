//! The trace's spelling: one record of a decision, as the `decisions` table holds it.
//!
//! Written by hand rather than through a serialiser. `strategy` carries none, and the record is a
//! handful of numbers and one word, which is not worth a crate the operator has to approve.

use svanbot_core::card::CardSet;
use svanbot_core::rules::Street;
use svanbot_strategy::policy::{Abstract, Branch, PricedTrace, Ranged, ResolvedTrace, Trace};
use svanbot_strategy::read::Class;

use rusqlite::{Transaction, params};

use super::{Store, StoreError};

use crate::record::Hand;

/// The newest hands whose decisions are kept, per bot. Past this the oldest fall off: at about
/// thirteen decisions a hand that is some 100 MB, inside the disk budget `tools/disk.ts` reports.
pub const TRACED_HANDS: usize = 2_000;

/// One hand in the list the replayer opens a trace from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TracedHand {
    pub hand_id: String,
    pub table_id: String,
    pub recorded_ms: i64,
    /// How many decisions in it carry a trace.
    pub decisions: usize,
}

/// The trace as JSON. Written by hand: `strategy` carries no serialiser, and the record is a
/// handful of numbers and a few words, which is not worth a crate the operator has to approve.
///
/// A number that is not finite has no JSON spelling (`inf`, `NaN`), and the column's
/// `json_valid` refuses the row, which would lose the whole hand with it. A value that is not
/// finite loses its own entry only, and says so with `null`: a recorded hole in the values is
/// better than a decision the bot played and the store never kept.
pub(super) fn trace_json(trace: &Trace) -> String {
    let mut fields: Vec<String> = vec![
        format!(r#""branch":"{}""#, trace.branch.name()),
        format!(r#""hole":"{}""#, trace.hole),
        format!(r#""board":"{}""#, trace.board),
    ];
    if let Some(resolved) = &trace.resolved {
        let actions: Vec<String> = resolved
            .actions
            .iter()
            .map(|action| format!(r#""{}""#, action.named()))
            .collect();
        fields.push(format!(
            r#""values":{},"strategy":{},"taken":{},"actions":[{}],"ranges":{}"#,
            numbers(&resolved.values),
            numbers(&resolved.strategy),
            resolved.taken,
            actions.join(","),
            ranges_json(&resolved.ranges)
        ));
    }
    if let Some(priced) = &trace.priced {
        fields.push(format!(
            r#""equity":{},"price":{},"exploit":{},"ranges":{}"#,
            number(priced.equity),
            optional(priced.price),
            optional(priced.exploit),
            ranges_json(&priced.ranges)
        ));
    }
    format!("{{{}}}", fields.join(","))
}

/// A float as JSON, or `null` when it has no JSON spelling.
fn number(value: f64) -> String {
    if value.is_finite() {
        value.to_string()
    } else {
        "null".to_owned()
    }
}

/// A float that may be absent, as JSON.
fn optional(value: Option<f64>) -> String {
    value.map_or_else(|| "null".to_owned(), number)
}

fn numbers(values: &[f64]) -> String {
    let items: Vec<String> = values.iter().copied().map(number).collect();
    format!("[{}]", items.join(","))
}

fn ranges_json(ranges: &[Ranged]) -> String {
    let items: Vec<String> = ranges
        .iter()
        .map(|ranged| {
            // A seat the spot did not carry is `null`, not 0: zero is a seat at the table, and a
            // trace that guessed one would put a wrong label on the page.
            let seat = match ranged.seat {
                Some(seat) => seat.to_string(),
                None => "null".to_owned(),
            };
            // The class is there when the opponent has a read, and the trust only when that read
            // decided the range: a held read names its class and spends nothing.
            let class = ranged
                .class
                .map(|class| format!(r#","class":"{}""#, class.name()))
                .unwrap_or_default();
            let trust = ranged
                .trust
                .map(|trust| format!(r#","trust":{}"#, number(trust)))
                .unwrap_or_default();
            let read = format!("{class}{trust}");
            format!(
                r#"{{"seat":{seat},"masses":{}{read}}}"#,
                numbers(&ranged.masses)
            )
        })
        .collect();
    format!("[{}]", items.join(","))
}

/// The trace back from [`trace_json`], with the street and branch the row keeps as columns.
///
/// A value the schema forbids is an error, not a default: a row whose `branch` or `street` the
/// `CHECK`s do not allow is a store that disagrees with this code, and reading it as some other
/// branch would put a wrong answer on the page.
pub(super) fn trace_from(street: &str, branch: &str, json: &str) -> Result<Trace, StoreError> {
    let value: serde_json::Value =
        serde_json::from_str(json).map_err(|e| StoreError(format!("a trace is not JSON: {e}")))?;
    let branch =
        Branch::named(branch).ok_or_else(|| StoreError(format!("no branch is called {branch}")))?;
    let street =
        Street::named(street).ok_or_else(|| StoreError(format!("no street is called {street}")))?;
    let text = |name: &str| -> Result<CardSet, StoreError> {
        value[name]
            .as_str()
            .ok_or_else(|| StoreError(format!("a trace has no {name}")))?
            .parse()
            .map_err(|e| StoreError(format!("a trace's {name} is not cards: {e}")))
    };
    let ranges = |value: &serde_json::Value| -> Vec<Ranged> {
        value["ranges"]
            .as_array()
            .map(|rows| {
                rows.iter()
                    .map(|row| Ranged {
                        // A trace written before the seat was carried has no `seat`, and reads
                        // back as unknown rather than as seat 0.
                        seat: row["seat"]
                            .as_u64()
                            .and_then(|seat| usize::try_from(seat).ok()),
                        masses: floats(&row["masses"]),
                        // A trace written before reads were in play has neither, and reads back
                        // as the population's.
                        class: row["class"].as_str().and_then(Class::named),
                        trust: row["trust"].as_f64(),
                    })
                    .collect()
            })
            .unwrap_or_default()
    };
    let (resolved, priced) = match branch {
        Branch::Resolved => (
            Some(ResolvedTrace {
                values: floats(&value["values"]),
                strategy: floats(&value["strategy"]),
                taken: usize::try_from(value["taken"].as_u64().unwrap_or(0)).unwrap_or(0),
                actions: actions(&value["actions"])?,
                ranges: ranges(&value),
            }),
            None,
        ),
        Branch::Priced => (
            None,
            Some(PricedTrace {
                equity: value["equity"].as_f64().unwrap_or(0.0),
                price: value["price"].as_f64(),
                exploit: value["exploit"].as_f64(),
                ranges: ranges(&value),
            }),
        ),
    };
    Ok(Trace {
        branch,
        street,
        hole: text("hole")?,
        board: text("board")?,
        resolved,
        priced,
    })
}

/// The floats [`trace_json`] wrote, in order. A `null` keeps its place as `NaN` rather than
/// dropping out: `values`, `strategy` and `actions` are read by index together, so a value that
/// vanished would put every later value against the wrong action.
fn floats(value: &serde_json::Value) -> Vec<f64> {
    value
        .as_array()
        .map(|items| {
            items
                .iter()
                .map(|item| item.as_f64().unwrap_or(f64::NAN))
                .collect()
        })
        .unwrap_or_default()
}

/// The names [`trace_json`] wrote for a re-solve's actions, in order.
///
/// An action this code does not know is an error, like an unknown branch: dropping it would shift
/// every later action against the value and the mix that belong to a different one.
fn actions(value: &serde_json::Value) -> Result<Vec<Abstract>, StoreError> {
    let Some(items) = value.as_array() else {
        return Ok(Vec::new());
    };
    items
        .iter()
        .map(|item| {
            let name = item
                .as_str()
                .ok_or_else(|| StoreError("an action is not a name".to_owned()))?;
            abstract_named(name).ok_or_else(|| StoreError(format!("no action is called {name}")))
        })
        .collect()
}

/// The action called `name`, or `None` when no action is.
fn abstract_named(name: &str) -> Option<Abstract> {
    match name {
        "fold" => Some(Abstract::Fold),
        "check" => Some(Abstract::Check),
        "call" => Some(Abstract::Call),
        other => Some(Abstract::Bet(other.strip_prefix("bet ")?.parse().ok()?)),
    }
}

impl Store {
    /// The trace of each decision in `hand_id`, oldest first. `None` when the hand carries no
    /// trace at all: a hand played before the trace existed is not a hand that made no decisions,
    /// and the replayer says which it is holding.
    pub fn traces(&self, hand_id: &str) -> Result<Option<Vec<Trace>>, StoreError> {
        let mut statement = self.db.prepare(
            "SELECT street, branch, trace FROM decisions WHERE hand_id = ?1 ORDER BY seq",
        )?;
        let rows = statement.query_map(params![hand_id], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        })?;
        let mut traces = Vec::new();
        for row in rows {
            let (street, branch, json) = row?;
            traces.push(trace_from(&street, &branch, &json)?);
        }
        Ok((!traces.is_empty()).then_some(traces))
    }

    /// Whether `hand_id` is in this store at all, traced or not. The replayer needs the difference
    /// between a hand it holds with no trace and a hand it never held.
    pub fn holds(&self, hand_id: &str) -> Result<bool, StoreError> {
        let found: i64 = self.db.query_row(
            "SELECT COUNT(*) FROM hands WHERE hand_id = ?1",
            params![hand_id],
            |row| row.get(0),
        )?;
        Ok(found > 0)
    }

    /// The newest hands whose decisions are kept, newest first — what the replayer's list opens.
    pub fn traced_hands(&self) -> Result<Vec<TracedHand>, StoreError> {
        let mut statement = self.db.prepare(
            "SELECT h.hand_id, h.table_id, h.recorded_ms, COUNT(d.seq)
             FROM hands h JOIN decisions d ON d.hand_id = h.hand_id
             GROUP BY h.hand_id
             ORDER BY h.recorded_ms DESC, h.rowid DESC
             LIMIT ?1",
        )?;
        let rows = statement.query_map(
            params![i64::try_from(TRACED_HANDS).unwrap_or(i64::MAX)],
            |row| {
                Ok(TracedHand {
                    hand_id: row.get(0)?,
                    table_id: row.get(1)?,
                    recorded_ms: row.get(2)?,
                    decisions: usize::try_from(row.get::<_, i64>(3)?).unwrap_or(0),
                })
            },
        )?;
        rows.map(|row| row.map_err(StoreError::from)).collect()
    }
}

/// Writes a hand's decisions in the hand's own transaction, then enforces the cap.
///
/// The hand and its traces are one write: a trace has no meaning without its hand, and the
/// recorder drops an open hand when the connection breaks, so a decision written at decision
/// time can outlive a hand the store never kept.
pub(super) fn keep(tx: &Transaction<'_>, hand: &Hand) -> Result<(), StoreError> {
    for (seq, trace) in hand.traced.iter().enumerate() {
        tx.execute(
            "INSERT INTO decisions (hand_id, seq, street, branch, trace)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                hand.hand_id,
                i64::try_from(seq).unwrap_or(i64::MAX),
                trace.street.name(),
                trace.branch.name(),
                trace_json(trace)
            ],
        )?;
    }
    // The cap is checked before it is enforced: a trim on every hand reads and rewrites the
    // newest 2,000 for nothing, and a store under the cap has nothing to trim.
    let traced: i64 = tx.query_row(
        "SELECT COUNT(*) FROM (SELECT hand_id FROM decisions GROUP BY hand_id)",
        [],
        |row| row.get(0),
    )?;
    if traced > i64::try_from(TRACED_HANDS).unwrap_or(i64::MAX) {
        trim(tx)?;
    }
    Ok(())
}

/// `trim` under the cap: keeps the newest TRACED_HANDS traced hands and deletes the decisions of
/// everything older. Only decisions are deleted; the hands themselves stay.
fn trim(tx: &Transaction<'_>) -> Result<(), StoreError> {
    tx.execute(
        "DELETE FROM decisions WHERE hand_id NOT IN (
            SELECT hand_id FROM (
                SELECT h.hand_id FROM hands h JOIN decisions d ON d.hand_id = h.hand_id
                GROUP BY h.hand_id
                ORDER BY h.recorded_ms DESC, h.rowid DESC
                LIMIT ?1
            )
        )",
        params![i64::try_from(TRACED_HANDS).unwrap_or(i64::MAX)],
    )?;
    Ok(())
}
