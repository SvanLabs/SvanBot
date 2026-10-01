//! Split-fleet model and heartbeat loops.
use super::*;

/// Worker loops of a split fleet (0128): heartbeats for the head dashboard and the operator's
/// desired-mode key. Head processes never run these; the all-in-one fleet never needs them.
/// Reload the head's opponent-model checkpoint every 30 s when it changed (the head saves every
/// 5 minutes after folding every process's hands in, so this is the canonical view).
pub(super) fn spawn_worker_model_refresh(shared: &Arc<Shared>) {
    supervision::spawn("worker model refresh", shared, |refresh| async move {
        let mut last = None;
        loop {
            let s = refresh.clone();
            let prev = last.take();
            last = crate::jobs::blocking("worker model refresh", move || {
                let mut last = prev;
                refresh_models_from_store(&s, &mut last);
                last
            })
            .await
            .flatten();
            tokio::time::sleep(Duration::from_secs(30)).await;
        }
    });
}

pub(super) fn spawn_worker_loops(shared: &Arc<Shared>) {
    supervision::spawn("worker heartbeat", shared, |beats| async move {
        loop {
            for (slot, bot) in beats.config.bots.iter().enumerate() {
                let value = {
                    let b = beats.bots[slot].read();
                    crate::live::wrap_heartbeat(&b, now_secs())
                };
                if let Err(e) = beats.store.put_kv(&crate::live::heartbeat_key(&bot.name), &value.to_string()) {
                    tracing::warn!("heartbeat for {} not stored: {e}", bot.name);
                }
            }
            tokio::time::sleep(Duration::from_secs(5)).await;
        }
    });
    supervision::spawn("worker commands", shared, |wants| async move {
        let mut last: std::collections::HashMap<String, String> = std::collections::HashMap::new();
        loop {
            for (slot, bot) in wants.config.bots.iter().enumerate() {
                let key = crate::live::want_key(&bot.name);
                let want = wants.store.get_kv(&key).ok().flatten().unwrap_or_default();
                if !want.is_empty() && last.get(&bot.name).map(|l| l.as_str()) != Some(want.as_str()) {
                    let cmd = want.clone();
                    wants.update(slot, |b| crate::live::apply_desired(b, &cmd));
                    wants.log(&bot.name, "info", format!("operator command from the head: {cmd}"));
                    last.insert(bot.name.clone(), want);
                }
            }
            tokio::time::sleep(Duration::from_secs(5)).await;
        }
    });
}
/// Head loop of a split fleet (0128): fold every newly stored hand into the canonical models.
/// Workers write hand rows but never save; the regular 5-minute saver persists what the tailer
/// observed, so no observation is lost whichever process saw the table.
pub(super) fn spawn_head_tailer(shared: &Arc<Shared>) {
    supervision::spawn("head tailer", shared, |tail| async move {
        loop {
            let t = tail.clone();
            crate::jobs::blocking("head tailer", move || match fold_new_hands(&t.store, &t.models) {
                Ok(folded) if folded > 0 => tracing::info!("head tailer folded {folded} stored hands into the models"),
                Err(e) => tracing::warn!("head model tail failed: {e}"),
                _ => {}
            })
            .await;
            tokio::time::sleep(Duration::from_secs(30)).await;
        }
    });
}
