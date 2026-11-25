//! The latest live evidence for a probe that had a paired sim estimate.

use std::fs;
use std::io::ErrorKind;

use serde_json::{Value, json};
use svanbot_learn::calibration::Point;

use super::Fleet;

const CALIBRATION: &str = "calibration.json";
const KEPT: usize = 50;

struct Entry {
    probe: u64,
    point: Point,
    verdict: String,
    ended_ms: i64,
}

impl Entry {
    fn from_json(value: &Value) -> Result<Self, String> {
        let invalid = |key: &str| format!("{CALIBRATION}: invalid {key}");
        let number = |key: &str| value[key].as_f64().ok_or_else(|| invalid(key));
        let point = Point {
            sim: number("sim")?,
            live: number("live")?,
            radius: number("radius")?,
        };
        if !point.is_valid() {
            return Err(invalid("point"));
        }
        Ok(Self {
            probe: value["probe"].as_u64().ok_or_else(|| invalid("probe"))?,
            point,
            verdict: value["verdict"]
                .as_str()
                .ok_or_else(|| invalid("verdict"))?
                .into(),
            ended_ms: value["ended_ms"]
                .as_i64()
                .ok_or_else(|| invalid("ended_ms"))?,
        })
    }

    fn as_json(&self) -> Value {
        json!({
            "probe": self.probe,
            "sim": self.point.sim,
            "live": self.point.live,
            "radius": self.point.radius,
            "verdict": self.verdict,
            "ended_ms": self.ended_ms,
        })
    }
}

impl Fleet {
    pub fn record_calibration(
        &self,
        probe: u64,
        point: &Point,
        verdict: &str,
        ended_ms: i64,
    ) -> Result<(), String> {
        if !point.is_valid() {
            return Err(format!("{CALIBRATION}: invalid point"));
        }
        let _lock = self.lock_fleet_file(CALIBRATION)?;
        let mut entries = self.calibration_entries()?;
        if entries
            .iter()
            .any(|entry| entry.probe == probe && entry.ended_ms > ended_ms)
        {
            return Ok(());
        }
        entries.retain(|entry| entry.probe != probe);
        entries.insert(
            0,
            Entry {
                probe,
                point: *point,
                verdict: verdict.into(),
                ended_ms,
            },
        );
        entries.sort_by_key(|entry| std::cmp::Reverse(entry.ended_ms));
        entries.truncate(KEPT);
        let entries: Vec<_> = entries.iter().map(Entry::as_json).collect();
        self.write(CALIBRATION, &json!({ "points": entries }).to_string())
    }

    pub fn calibration(&self) -> Result<Vec<Point>, String> {
        Ok(self
            .calibration_entries()?
            .into_iter()
            .map(|entry| entry.point)
            .collect())
    }

    fn calibration_entries(&self) -> Result<Vec<Entry>, String> {
        let text = match fs::read_to_string(self.dir.join(CALIBRATION)) {
            Ok(text) => text,
            Err(error) if error.kind() == ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => return Err(format!("{CALIBRATION}: {error}")),
        };
        let value: Value =
            serde_json::from_str(&text).map_err(|error| format!("{CALIBRATION}: {error}"))?;
        value["points"]
            .as_array()
            .ok_or_else(|| format!("{CALIBRATION}: expected a points list"))?
            .iter()
            .map(Entry::from_json)
            .collect()
    }
}
