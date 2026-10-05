//! One hourly axis for stored fleet results and the operational changes around them (0290).
//! The main request reads indexed hand aggregates; exact hand rows load only for the selected hour.
use super::*;
use chrono::{DateTime, Utc};
use std::collections::{BTreeMap, HashMap, HashSet};

const HOUR: i64 = 3600;
const MAX_HOURS: i64 = 21 * 24;

fn hour_start(ts: i64) -> i64 {
    ts.div_euclid(HOUR) * HOUR
}

fn stamp(ts: i64) -> String {
    DateTime::<Utc>::from_timestamp(ts, 0).expect("timeline date is in range").to_rfc3339()
}

fn fleet_names(s: &Shared) -> Vec<String> {
    let mut seen = HashSet::new();
    s.bots.iter().flat_map(|bot| s.names_of(&bot.read().name)).filter(|name| seen.insert(name.clone())).collect()
}

fn mark(kind: &str, id: String, ts: f64, title: String, detail: String, source: Value) -> Value {
    json!({"kind": kind, "id": id, "ts": ts, "title": title, "detail": detail, "source": source})
}

fn release_marks(text: &str, start: i64, end: i64) -> Vec<Value> {
    text.lines()
        .enumerate()
        .filter_map(|(line_number, line)| {
            let at = line.split_whitespace().next().map(parse_ts)?;
            if at < start as f64 || at >= end as f64 {
                return None;
            }
            let (commit, subject) = parse_release_line(line)?;
            let short: String = commit.chars().take(7).collect();
            Some(mark(
                "release",
                format!("release:{line_number}:{commit}"),
                at,
                format!("Release {short}"),
                subject.clone(),
                json!({"line": line_number + 1, "commit": commit, "subject": subject, "recorded_at": line.split_whitespace().next()}),
            ))
        })
        .collect()
}

fn operation_marks(text: &str, start: i64, end: i64) -> Result<Vec<Value>, String> {
    let mut marks = Vec::new();
    for (line_number, line) in text.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let record: Value = serde_json::from_str(line).map_err(|e| format!("line {}: {e}", line_number + 1))?;
        let ts = record["ts"].as_str().map(parse_ts).unwrap_or(0.0);
        let title = record["title"].as_str().ok_or_else(|| format!("line {}: title missing", line_number + 1))?;
        let detail = record["detail"].as_str().ok_or_else(|| format!("line {}: detail missing", line_number + 1))?;
        if ts <= 0.0 {
            return Err(format!("line {}: timestamp invalid", line_number + 1));
        }
        if ts >= start as f64 && ts < end as f64 {
            marks.push(mark(
                "operation",
                format!("operation:{}", line_number + 1),
                ts,
                title.to_string(),
                detail.to_string(),
                json!({"ledger_line": line_number + 1, "record": record}),
            ));
        }
    }
    Ok(marks)
}

pub(super) async fn timeline(State(s): State<Arc<Shared>>) -> Response {
    off_runtime(move || timeline_blocking(&s)).await.into_response()
}

fn timeline_blocking(s: &Shared) -> Result<Response, ApiError> {
    let now = now_secs().floor() as i64;
    let end = hour_start(now) + HOUR;
    let season = s.season();
    let requested = season.as_ref().map(|v| hour_start(v.started_at as i64)).unwrap_or(end - 14 * 24 * HOUR);
    let start = requested.max(end - MAX_HOURS * HOUR);
    let names = fleet_names(s);
    let rows = store_read("timeline hourly hands", s.store.timeline_hours(&names, &stamp(start), &stamp(end)))?;
    let by_hour: BTreeMap<i64, sv10_store::store::TimelineHour> = rows.into_iter().map(|row| (row.ts, row)).collect();
    let hours: Vec<sv10_store::store::TimelineHour> = (start..end)
        .step_by(HOUR as usize)
        .map(|ts| by_hour.get(&ts).cloned().unwrap_or_else(|| sv10_store::store::TimelineHour { ts, ..Default::default() }))
        .collect();

    let mut marks = Vec::new();
    if let Some(season) = &season
        && season.started_at >= start as f64
        && season.started_at < end as f64
    {
        marks.push(mark(
            "season",
            format!("season:{}", season.id.as_deref().unwrap_or("current")),
            season.started_at,
            format!("Season {} began", season.number.map(|n| n.to_string()).unwrap_or_else(|| "current".into())),
            "Season scope starts here; chip totals before this point belong to another contest.".into(),
            json!({"season_id": season.id, "number": season.number, "started_at": season.started_at}),
        ));
    }
    let release_text = std::fs::read_to_string(s.config.artifacts.join("releases.log"));
    if let Ok(text) = &release_text {
        marks.extend(release_marks(text, start, end));
    }
    let operation_text = std::fs::read_to_string(s.config.root.join("docs/operations-timeline.jsonl"));
    if let Ok(text) = &operation_text {
        marks.extend(operation_marks(text, start, end).map_err(|e| server_error("operations timeline unreadable", e))?);
    }
    let experiments = store_read("timeline promotions", s.store.get_kv(crate::LEARNER_EXPERIMENTS_KEY))?;
    if let Some(raw) = experiments {
        let records: Value = serde_json::from_str(&raw).map_err(|e| server_error("promotion history unreadable", e))?;
        for row in records.as_array().into_iter().flatten() {
            let Some(ts) = row["ts"].as_f64() else { continue };
            if row["status"] != "promoted" || ts < start as f64 || ts >= end as f64 {
                continue;
            }
            let id = row["id"].as_str().unwrap_or("promotion");
            marks.push(mark("promotion", format!("promotion:{id}"), ts, "Champion promoted".into(), id.to_string(), row.clone()));
        }
    }
    for event in store_read("timeline events", s.store.timeline_events(&stamp(start), &stamp(end)))? {
        let ts = parse_ts(&event.ts);
        if ts <= 0.0 {
            continue;
        }
        let kind = if event.level == "error" { "error" } else { "warning" };
        let title = if event.level == "error" { "Error" } else { "Warning" };
        marks.push(mark(kind, format!("event:{}", event.id), ts, title.into(), event.message.clone(), json!(event)));
    }
    marks.sort_by(|a, b| a["ts"].as_f64().unwrap_or(0.0).total_cmp(&b["ts"].as_f64().unwrap_or(0.0)));
    Ok(Json(json!({
        "scope": season_scope(season.as_ref()), "from": start, "until": end,
        "window_limited": start > requested, "hours": hours, "marks": marks,
        "release_log_available": release_text.is_ok(),
        "operations_ledger_available": operation_text.is_ok(), "updated": now_secs(),
    }))
    .into_response())
}

pub(super) async fn timeline_hour(State(s): State<Arc<Shared>>, Path(hour): Path<i64>) -> Response {
    off_runtime(move || timeline_hour_blocking(&s, hour)).await.into_response()
}

fn timeline_hour_blocking(s: &Shared, hour: i64) -> Result<Response, ApiError> {
    let now = now_secs().floor() as i64;
    let end = hour_start(now) + HOUR;
    let season_start = s.season().map(|v| hour_start(v.started_at as i64)).unwrap_or(end - 14 * 24 * HOUR);
    if hour != hour_start(hour) || hour < season_start.max(end - MAX_HOURS * HOUR) || hour >= end {
        return Err((StatusCode::BAD_REQUEST, Json(json!({"detail": "Hour outside the available timeline"}))).into_response().into());
    }
    let rows = store_read("timeline hour hands", s.store.timeline_hands(&fleet_names(s), &stamp(hour), &stamp(hour + HOUR)))?;
    let aliases: HashMap<String, (usize, String)> = s
        .bots
        .iter()
        .flat_map(|bot| {
            let b = bot.read();
            let (slot, display_bot) = (b.slot, b.name.clone());
            s.names_of(&display_bot).into_iter().map(move |name| (name, (slot, display_bot.clone())))
        })
        .collect();
    let hands: Vec<Value> = rows
        .into_iter()
        .map(|row| {
            let (slot, display_bot) = aliases.get(&row.bot).cloned().map_or((None, row.bot.clone()), |(slot, name)| (Some(slot), name));
            json!({"slot": slot, "bot": display_bot, "stored_bot": row.bot, "hand_id": row.hand_id,
            "ts": parse_ts(&row.ts), "net": row.net, "ev_net": row.ev_net})
        })
        .collect();
    Ok(Json(json!({"hour": hour, "hands": hands})).into_response())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn release_marks_keep_the_exact_log_line_for_drill_down() {
        let lines = "2026-09-22T09:00:00Z abc1234 Install five-minute keepalive\n\
                     2026-09-23T09:00:00Z def5678 Next release\n";
        let start = parse_ts("2026-09-22T00:00:00Z") as i64;
        let marks = release_marks(lines, start, start + 24 * HOUR);
        assert_eq!(marks.len(), 1);
        assert_eq!(marks[0]["source"]["line"], 1);
        assert_eq!(marks[0]["source"]["commit"], "abc1234");
        assert_eq!(marks[0]["detail"], "Install five-minute keepalive");
    }

    #[test]
    fn operation_marks_keep_the_dated_source_record() {
        let line =
            r#"{"ts":"2026-09-22T19:21:00Z","title":"Five-minute keepalive enabled","detail":"Operator enabled timer","ticket":"0145"}"#;
        let start = parse_ts("2026-09-22T00:00:00Z") as i64;
        let marks = operation_marks(line, start, start + 24 * HOUR).unwrap();
        assert_eq!(marks.len(), 1);
        assert_eq!(marks[0]["source"]["ledger_line"], 1);
        assert_eq!(marks[0]["source"]["record"]["ticket"], "0145");
        assert_eq!(marks[0]["title"], "Five-minute keepalive enabled");
        assert!(operation_marks("{bad json", start, start + HOUR).is_err());
    }
}
