//! The trace's spelling: one record of a decision, as the `decisions` table holds it.
//!
//! Written by hand rather than through a serialiser. `strategy` carries none, and the record is a
//! handful of numbers and one word, which is not worth a crate the operator has to approve.

use svanbot_core::rules::Street;
use svanbot_strategy::policy::{Branch, PricedTrace, Ranged, SolvedTrace, Trace};

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

pub(super) fn street_name(street: Street) -> &'static str {
    match street {
        Street::Preflop => "preflop",
        Street::Flop => "flop",
        Street::Turn => "turn",
        Street::River => "river",
    }
}

fn street_named(name: &str) -> Street {
    match name {
        "flop" => Street::Flop,
        "turn" => Street::Turn,
        "river" => Street::River,
        _ => Street::Preflop,
    }
}

pub(super) fn branch_name(branch: Branch) -> &'static str {
    match branch {
        Branch::Solved => "solved",
        Branch::Priced => "priced",
    }
}

/// The trace as JSON. Written by hand: `strategy` carries no serialiser, and the record is a
/// handful of numbers and one word, which is not worth a crate the operator has to approve.
pub(super) fn trace_json(trace: &Trace) -> String {
    let mut fields: Vec<String> = vec![format!(r#""branch":"{}""#, branch_name(trace.branch))];
    if let Some(solved) = &trace.solved {
        fields.push(format!(
            r#""values":{},"strategy":{},"taken":{},"ranges":{}"#,
            numbers(&solved.values),
            numbers(&solved.strategy),
            solved.taken,
            ranges_json(&solved.ranges)
        ));
    }
    if let Some(priced) = &trace.priced {
        let price = match priced.price {
            Some(price) => price.to_string(),
            None => "null".into(),
        };
        fields.push(format!(
            r#""equity":{},"price":{},"ranges":{}"#,
            priced.equity,
            price,
            ranges_json(&priced.ranges)
        ));
    }
    format!("{{{}}}", fields.join(","))
}

fn numbers(values: &[f64]) -> String {
    let items: Vec<String> = values.iter().map(ToString::to_string).collect();
    format!("[{}]", items.join(","))
}

fn ranges_json(ranges: &[Ranged]) -> String {
    let items: Vec<String> = ranges
        .iter()
        .map(|ranged| {
            format!(
                r#"{{"seat":{},"masses":{}}}"#,
                ranged.seat,
                numbers(&ranged.masses)
            )
        })
        .collect();
    format!("[{}]", items.join(","))
}

/// The trace back from [`trace_json`], with the street and branch the row keeps as columns.
pub(super) fn trace_from(street: &str, branch: &str, json: &str) -> Result<Trace, StoreError> {
    let value: serde_json::Value =
        serde_json::from_str(json).map_err(|e| StoreError(e.to_string()))?;
    let branch = match branch {
        "solved" => Branch::Solved,
        _ => Branch::Priced,
    };
    let ranges = || -> Vec<Ranged> {
        value["ranges"]
            .as_array()
            .map(|rows| {
                rows.iter()
                    .map(|row| Ranged {
                        seat: usize::try_from(row["seat"].as_u64().unwrap_or(0)).unwrap_or(0),
                        masses: floats(&row["masses"]),
                    })
                    .collect()
            })
            .unwrap_or_default()
    };
    let (solved, priced) = match branch {
        Branch::Solved => (
            Some(SolvedTrace {
                // The actions are the tree's own names, and the page reads the branch: a stored
                // trace says what each action was worth, and the values are in the same order.
                actions: Vec::new(),
                values: floats(&value["values"]),
                strategy: floats(&value["strategy"]),
                taken: usize::try_from(value["taken"].as_u64().unwrap_or(0)).unwrap_or(0),
                ranges: ranges(),
            }),
            None,
        ),
        Branch::Priced => (
            None,
            Some(PricedTrace {
                equity: value["equity"].as_f64().unwrap_or(0.0),
                price: value["price"].as_f64(),
                ranges: ranges(),
            }),
        ),
    };
    Ok(Trace {
        branch,
        street: street_named(street),
        solved,
        priced,
    })
}

fn floats(value: &serde_json::Value) -> Vec<f64> {
    value
        .as_array()
        .map(|items| items.iter().filter_map(serde_json::Value::as_f64).collect())
        .unwrap_or_default()
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
                street_name(trace.street),
                branch_name(trace.branch),
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
