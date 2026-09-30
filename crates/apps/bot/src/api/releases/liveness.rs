//! Recover abandoned progress without mistaking a healthy manual release for an orphan.
use super::*;
use std::{fs::File, io::Write, path::Path};

fn process_fields(text: &str) -> Option<(&str, &str)> {
    // The command name can contain spaces or parentheses; fields after its final ')' are fixed.
    let fields: Vec<_> = text.get(text.rfind(')')? + 1..)?.split_whitespace().collect();
    Some((*fields.first()?, *fields.get(19)?))
}

fn owner_alive(owner: &Value) -> bool {
    let Some(pid) = owner["pid"].as_u64() else { return true };
    if let (Some(recorded), Ok(current)) = (owner["boot_id"].as_str(), std::fs::read_to_string("/proc/sys/kernel/random/boot_id"))
        && recorded != current.trim()
    {
        return false;
    }
    match std::fs::read_to_string(format!("/proc/{pid}/stat")) {
        Ok(text) => match process_fields(&text) {
            Some(("Z" | "X", _)) => false,
            Some((_, start)) => owner["process_start"].as_str().is_none_or(|recorded| recorded == start),
            None => true,
        },
        Err(e) => e.kind() != std::io::ErrorKind::NotFound,
    }
}

pub(super) fn update_running(artifacts: &Path) -> bool {
    // A hand-run release has no dashboard marker, but holds this lock throughout its operation.
    match File::open(artifacts.join("release-operation.lock")) {
        Ok(operation) => {
            if operation.try_lock().is_err() {
                return true; // Held, or ownership cannot safely be established.
            }
        }
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => return true,
        Err(_) => {}
    }
    // A manually started updater can spend time fetching before taking the operation lock. New
    // progress writers record that owner too, so it is not mistaken for an abandoned dashboard run.
    let progress_owner = std::fs::read_to_string(artifacts.join("release-progress.json"))
        .ok()
        .and_then(|text| serde_json::from_str::<Value>(&text).ok())
        .filter(|progress| progress["state"] == "running" && progress["pid"].as_u64().is_some());
    let lock = artifacts.join("release.lock");
    let meta = match std::fs::metadata(&lock) {
        Ok(meta) => meta,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return progress_owner.as_ref().is_some_and(owner_alive),
        Err(_) => return true,
    };
    if let Some(owner) = std::fs::read_to_string(&lock).ok().and_then(|text| serde_json::from_str::<Value>(&text).ok())
        && owner["pid"].as_u64().is_some()
    {
        return owner_alive(&owner);
    }
    if let Some(owner) = progress_owner {
        return owner_alive(&owner);
    }
    // Older installs and the short spawn window carry no owner yet; preserve their age fallback.
    meta.modified().ok().and_then(|t| t.elapsed().ok()).is_some_and(|d| d.as_secs() < 2 * 3600)
}

pub(super) fn record_owner(lock: &Path, pid: u32) {
    let process_start = std::fs::read_to_string(format!("/proc/{pid}/stat"))
        .ok()
        .and_then(|text| process_fields(&text).map(|(_, start)| start.to_string()));
    let boot_id = std::fs::read_to_string("/proc/sys/kernel/random/boot_id").ok().map(|s| s.trim().to_string());
    let owner = json!({"started":now_secs(), "pid":pid, "process_start":process_start, "boot_id":boot_id});
    // Open an existing inode: never recreate a marker removed by an updater that finished quickly.
    if let Ok(mut file) = std::fs::OpenOptions::new().write(true).open(lock)
        && file.set_len(0).is_ok()
    {
        let _ = file.write_all(owner.to_string().as_bytes());
    }
}

pub(super) fn reconcile(progress: &Value, running: bool, now: f64) -> Value {
    let mut result = progress.clone();
    let started = progress["started"].as_f64().unwrap_or(now);
    if progress["state"] == "running" && !running && now - started > 5.0 {
        result["state"] = json!("failed");
        result["message"] = json!("The update process is no longer active. Retry the update; see the release log for its last output.");
        // The exact exit time is unknown; freeze at the last recorded activity rather than invent it.
        let ended = progress["updated"].as_f64().unwrap_or(started).max(started).min(now);
        result["updated"] = json!(ended);
        if let Some(stages) = result["stages"].as_array_mut() {
            for stage in stages.iter_mut().filter(|stage| stage["state"] == "running") {
                stage["state"] = json!("failed");
                stage["seconds"] = json!((ended - stage["started"].as_f64().unwrap_or(ended)).max(0.0));
            }
        }
    }
    result
}
