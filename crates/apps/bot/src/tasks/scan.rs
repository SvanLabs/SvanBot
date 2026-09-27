//! Findings-scan worker behind the finding loop (0256).

use crate::live::Shared;

/// One pass of the finding loop: scan, merge into what is stored, file and close tickets.
pub fn run_findings_scan(shared: &Shared, root: &std::path::Path) -> (usize, Vec<String>, Vec<String>) {
    let now = chrono::Utc::now().timestamp() as f64;
    let h2h = shared.head_to_head.read().clone();
    let Ok(scan) = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| crate::findings::scan(&shared.store, &h2h, now))) else {
        shared.log("findings", "warn", "the scan panicked; nothing filed");
        return (0, vec![], vec![]);
    };
    // What this pass read and emitted, as one row that is written only when it changes (0356): the
    // calibration table it corrected against is gone by the next learner cycle, so without this a
    // finding cannot be re-derived from its own ticket. A failed write is a warning — the tickets are
    // what the pass is for, and the next pass tries again.
    match shared.store.record_scan_snapshot(&scan.snapshot) {
        Ok(true) => tracing::info!("findings snapshot recorded: this pass's inputs or outputs changed"),
        Ok(false) => {}
        Err(e) => shared.log("findings", "warn", format!("findings snapshot not recorded: {e}")),
    }
    let previous = shared
        .store
        .get_kv(crate::findings::FINDINGS_KEY)
        .ok()
        .flatten()
        .and_then(|j| serde_json::from_str::<serde_json::Value>(&j).ok())
        .unwrap_or(serde_json::Value::Null);
    let merged = crate::findings::merge(&previous, &scan);
    let (filed, closed) = crate::findings::write_tickets(root, &merged, now);
    // Record the ticket each finding now holds, so a later pass can close it.
    let mut stored = merged.clone();
    if let Some(list) = stored.get_mut("findings").and_then(serde_json::Value::as_array_mut) {
        for f in list.iter_mut() {
            let Some(id) = f["id"].as_str() else { continue };
            let suffix = format!("-{}", crate::findings::slug(id));
            if let Some(name) = filed.iter().find(|n| n.ends_with(&suffix)) {
                f["ticket"] = serde_json::json!(name.as_str());
            }
        }
    }
    if let Err(e) = shared.store.put_kv(crate::findings::FINDINGS_KEY, &stored.to_string()) {
        shared.log("findings", "warn", format!("findings not stored: {e}"));
    }
    for name in &filed {
        shared.log(
            "findings",
            "warn",
            format!(
                "filed ticket {name}: {}",
                scan.findings
                    .iter()
                    .find(|f| name.ends_with(&format!("-{}", crate::findings::slug(&f.id))))
                    .map(|f| f.title.clone())
                    .unwrap_or_default()
            ),
        );
    }
    for name in &closed {
        shared.log("findings", "info", format!("closed ticket {name}: the finding cleared (the ticket says why)"));
    }
    (scan.findings.len(), filed, closed)
}
