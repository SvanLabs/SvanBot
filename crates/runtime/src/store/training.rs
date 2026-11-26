use super::{Store, StoreError};
use crate::record::Hand;

impl Store {
    /// Every kept hand, in the order it was kept. Each frame comes back as compact JSON.
    pub fn hands(&self) -> Result<Vec<Hand>, StoreError> {
        self.read_hands("SELECT hand_id, table_id, frames FROM hands ORDER BY rowid")
    }

    /// Every kept hand in play order, using the venue's end time on a downloaded hand.
    pub fn training_hands(&self) -> Result<Vec<Hand>, StoreError> {
        self.read_hands(
            "SELECT hand_id, table_id, frames FROM hands
             ORDER BY COALESCE(
                 CASE WHEN source = 'history'
                      THEN CAST(unixepoch(json_extract(frames, '$[0].ended_at'), 'subsec') * 1000 AS INTEGER)
                 END,
                 recorded_ms
             ), rowid",
        )
    }

    fn read_hands(&self, sql: &str) -> Result<Vec<Hand>, StoreError> {
        let mut query = self.db.prepare(sql)?;
        let rows = query.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        })?;
        rows.map(|row| {
            let (hand_id, table_id, frames) = row?;
            let frames: Vec<serde_json::Value> =
                serde_json::from_str(&frames).map_err(|e| StoreError(e.to_string()))?;
            Ok(Hand {
                hand_id,
                table_id,
                frames: frames.iter().map(ToString::to_string).collect(),
                traced: Vec::new(),
            })
        })
        .collect()
    }
}
