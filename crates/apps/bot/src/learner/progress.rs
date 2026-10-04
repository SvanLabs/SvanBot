//! Durable strategy progress, independent of the bounded recent candidate list.
use serde_json::{Value, json};
use sv10_store::store::Store;

/// Latest promotion record, committed together with champion parameters and lineage.
pub const LAST_PROMOTION_KEY: &str = "learner.last-promotion.v1";

/// Successful champion replacements, excluding the initial policy.
pub fn promotion_count(store: &Store) -> usize {
    super::load_lineage(store).len().saturating_sub(1)
}

/// Older installations retain their lineage's real write time, explicitly labelled because
/// legacy records have no retained confirmation interval.
pub fn latest(store: &Store) -> Value {
    let lineage = super::load_lineage(store);
    let version = lineage.last().map(String::as_str).unwrap_or_default();
    if let Some(mut record) = store.get_kv(LAST_PROMOTION_KEY).ok().flatten().and_then(|j| serde_json::from_str::<Value>(&j).ok())
        && record["status"] == "promoted"
        && record["challenger"] == version
        && record["ts"].as_f64().is_some()
    {
        record["timestamp_basis"] = json!("promotion record");
        return record;
    }
    if lineage.len() > 1 {
        let ts = store
            .kv_updated(crate::LEARNER_LINEAGE_KEY)
            .ok()
            .flatten()
            .and_then(|t| chrono::DateTime::parse_from_rfc3339(&t).ok())
            .map(|t| t.timestamp());
        return json!({"status":"promoted", "challenger":version, "ts":ts, "timestamp_basis":"lineage write"});
    }
    Value::Null
}

/// A failed candidate-history write cannot erase an installed promotion, and a failed
/// installation cannot claim one happened: all authoritative records share one transaction.
pub(super) fn install(store: &Store, lane: &super::lane::Lane, params: &str, lineage: &str, promotion: &Value) -> anyhow::Result<()> {
    let promotion = serde_json::to_string(promotion)?;
    store.put_kv_batch(&[
        (&lane.params_key(), params),
        (&lane.key(crate::LEARNER_LINEAGE_KEY), lineage),
        (&lane.key(LAST_PROMOTION_KEY), &promotion),
    ])
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn installed_promotion_outlives_recent_rejections() {
        let shared = crate::live::Shared::for_test("promotion-install", &["A"]);
        let record = json!({"status":"promoted", "challenger":"sv10-ev-2", "ts":1_700_000_000.0, "lower_95":0.02});
        install(&shared.store, &super::super::lane::Lane::default(), "{}", r#"["sv10-ev-1","sv10-ev-2"]"#, &record).unwrap();
        for _ in 0..45 {
            super::super::push_experiment(&shared.store, json!({"status":"rejected"}));
        }
        assert_eq!(latest(&shared.store)["ts"], record["ts"]);
        assert_eq!(latest(&shared.store)["lower_95"], record["lower_95"]);
        assert_eq!(latest(&shared.store)["timestamp_basis"], "promotion record");
        assert_eq!(promotion_count(&shared.store), 1);
        assert_eq!(shared.store.get_kv(crate::PARAMS_KEY).unwrap().as_deref(), Some("{}"));
    }

    #[test]
    fn a_lane_promotion_writes_its_own_keys_and_leaves_the_shared_champion() {
        let shared = crate::live::Shared::for_test("promotion-lane", &["A"]);
        let lane = super::super::lane::Lane::bot("A");
        install(&shared.store, &lane, r#"{"call_margin":0.7}"#, r#"["sv10-ev-1","A-ev-2"]"#, &json!({"status":"promoted"})).unwrap();
        assert_eq!(shared.store.get_kv(&crate::slot_params_key("A")).unwrap().as_deref(), Some(r#"{"call_margin":0.7}"#));
        assert_eq!(shared.store.get_kv(crate::PARAMS_KEY).unwrap(), None);
        assert_eq!(shared.store.get_kv(crate::LEARNER_LINEAGE_KEY).unwrap(), None);
        assert_eq!(lane.lineage(&shared.store).last().unwrap(), "A-ev-2");
    }
}
