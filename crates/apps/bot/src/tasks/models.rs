//! Model checkpoint reload, hand folding and crash recovery (0256).

use crate::MODELS_KEY;
use crate::live::Shared;
use anyhow::Result;
use sv10_core::model::ModelStore;
use sv10_store::store;

/// Replace the in-memory models with the stored checkpoint when it differs from `last`; returns
/// whether it reloaded.
pub fn refresh_models_from_store(shared: &Shared, last: &mut Option<String>) -> bool {
    let Some(json) = shared.store.get_kv(MODELS_KEY).ok().flatten() else { return false };
    if last.as_deref() == Some(json.as_str()) {
        return false;
    }
    match serde_json::from_str::<ModelStore>(&json) {
        Ok(mut models) => {
            let mut live = shared.models.write();
            // Per-opponent fits are not checkpointed; keep the ones installed (0218).
            crate::playerfits::PlayerFits::of(&live).apply(&mut models);
            *live = models;
            *last = Some(json);
            true
        }
        Err(e) => {
            tracing::warn!("head model checkpoint unreadable: {e}");
            false
        }
    }
}

/// Fold every hand stored past the models' watermark into `models` — every seat but ours into the
/// opponent stats, our seat into the fleet's image of us (0321) — advancing the watermark past
/// unparseable rows; returns hands folded. The head tailer and startup recovery share this.
pub fn fold_new_hands(store: &store::Store, models: &parking_lot::RwLock<ModelStore>) -> Result<usize> {
    let from = models.read().watermark.unwrap_or(0);
    let (mut folded, mut unreadable) = (0, 0);
    let mut m = models.write();
    for (rowid, bot, summary) in store.hands_after(from)? {
        match serde_json::from_str::<sv10_core::model::HandSummary>(&summary) {
            Ok(h) => {
                m.observe_own_hand(&h, &bot, 1.0);
                folded += 1;
            }
            // Skipped for good (the watermark moves past it), so counted and logged (LESSONS 20).
            Err(e) => {
                unreadable += 1;
                if unreadable == 1 {
                    tracing::warn!("stored hand row {rowid} ({bot}) has an unreadable summary, not folded into the models: {e}");
                }
            }
        }
        m.watermark = Some(rowid);
    }
    if unreadable > 1 {
        tracing::warn!("{unreadable} stored hands had unreadable summaries and were not folded into the models");
    }
    Ok(folded)
}

/// Fold every stored hand the last model checkpoint missed back into the models,
/// so a crash between checkpoints never loses observations.
pub fn recover_models(store: &store::Store, models: &mut ModelStore) -> Result<()> {
    if models.schema < sv10_core::model::MODEL_SCHEMA {
        // New statistics were added: rebuild everything from the stored hand history.
        let rows = store.hands_after(0)?;
        let mut rebuilt =
            ModelStore { schema: sv10_core::model::MODEL_SCHEMA, half_life_hands: models.half_life_hands, ..Default::default() };
        for (rowid, bot, summary) in &rows {
            if let Ok(h) = serde_json::from_str::<sv10_core::model::HandSummary>(summary) {
                rebuilt.observe_own_hand(&h, bot, 1.0);
            }
            rebuilt.watermark = Some(*rowid);
        }
        tracing::info!("rebuilt opponent models (schema {}) from {} stored hands", rebuilt.schema, rows.len());
        *models = rebuilt;
        return Ok(());
    }
    match models.watermark {
        None if !models.players.is_empty() => {
            // Checkpoint from before watermarks existed: it is current as of now.
            models.watermark = Some(store.max_hand_rowid()?);
        }
        _ => {
            let tmp = parking_lot::RwLock::new(std::mem::take(models));
            let replayed = fold_new_hands(store, &tmp)?;
            *models = tmp.into_inner();
            if replayed > 0 {
                tracing::info!("recovered {replayed} hands missing from the model checkpoint");
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use sv10_store::store::HandRow;

    #[test]
    fn rebuilding_an_old_checkpoint_keeps_the_configured_recency() {
        use sv10_core::model::{MODEL_SCHEMA, ModelStore};
        use sv10_store::store::Store;
        let d = std::env::temp_dir().join(format!("sv10-rebuild-recency-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        let store = Store::open(&d.join("svanbot10.db")).unwrap();
        let mut models = ModelStore { schema: MODEL_SCHEMA - 1, half_life_hands: crate::OPPONENT_HALF_LIFE_HANDS, ..Default::default() };
        super::recover_models(&store, &mut models).unwrap();
        assert_eq!(models.schema, MODEL_SCHEMA);
        assert_eq!(models.half_life_hands, crate::OPPONENT_HALF_LIFE_HANDS);
    }

    #[test]
    fn fold_new_hands_advances_the_watermark_past_bad_rows() {
        use parking_lot::RwLock;
        use sv10_core::model::ModelStore;
        use sv10_store::store::Store;
        let d = std::env::temp_dir().join(format!("sv10-fold-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        let store = Store::open(&d.join("svanbot10.db")).unwrap();
        let row = |hand_id: &str, summary: &str| {
            store
                .insert_hand(&HandRow {
                    bot: "b".into(),
                    hand_id: hand_id.into(),
                    table_id: "t".into(),
                    ended_at: "2026-09-17T00:00:00Z".into(),
                    hero_seat: None,
                    hole: String::new(),
                    board: String::new(),
                    pot: 60,
                    net: Some(10),
                    winners: "b".into(),
                    summary: summary.into(),
                    showdown: false,
                })
                .unwrap()
        };
        let good = serde_json::json!({"players": [[0, "a"], [1, "b"]], "button": 0, "bb": 20, "history": [], "board": [], "shown": []});
        row("h1", &good.to_string());
        row("h2", "not json");
        let models = RwLock::new(ModelStore::default());
        assert_eq!(super::fold_new_hands(&store, &models).unwrap(), 1);
        assert_eq!(models.read().watermark, Some(2));
        // The folded row is bot b's hand: the tailer feeds the image of our own play as well as the
        // opponent stats, or the pricing would read us as an unknown player (0321).
        let hands = models.read().hero_seen[&format!("{}b", sv10_core::model::HERO_SEEN_ONE)].hands;
        assert_eq!(hands, 1.0);
        // Nothing new: a second fold is a no-op.
        assert_eq!(super::fold_new_hands(&store, &models).unwrap(), 0);
    }
}
