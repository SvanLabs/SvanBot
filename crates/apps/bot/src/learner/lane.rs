//! Which champion a search is about (ADR 0002): the shared one, or one bot's own lineage. A lane
//! keeps its own parameters, lineage list, promotion record and rejection ledger under keys of its
//! own; until it first promotes it reads the shared champion, so every lineage starts as a copy.

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sv10_core::policy::Params;
use sv10_store::store::Store;

/// The shared champion (`None`) or one bot's lineage.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Lane(pub Option<String>);

impl Lane {
    pub fn bot(name: &str) -> Self {
        Lane(Some(name.to_string()))
    }

    pub fn is_shared(&self) -> bool {
        self.0.is_none()
    }

    /// The lanes to search in turn: one per name in `LEARNER_LINEAGES` (comma separated), or only
    /// the shared champion when it is unset (today's behaviour).
    pub fn rotation(env: Option<&str>) -> Vec<Lane> {
        let bots: Vec<Lane> = env.unwrap_or_default().split(',').map(str::trim).filter(|b| !b.is_empty()).map(Lane::bot).collect();
        if bots.is_empty() { vec![Lane::default()] } else { bots }
    }

    /// A store key private to this lane; the shared lane keeps the original key.
    pub fn key(&self, shared: &str) -> String {
        match &self.0 {
            None => shared.to_string(),
            Some(bot) => format!("{shared}.slot.{bot}"),
        }
    }

    /// Where this lane's promoted parameters live (`params.slot.<bot>` is what live play reads).
    pub fn params_key(&self) -> String {
        self.0.as_deref().map_or_else(|| crate::PARAMS_KEY.to_string(), crate::slot_params_key)
    }

    /// Version name of this lane's `n`th champion.
    pub fn version(&self, n: usize) -> String {
        format!("{}-ev-{n}", self.0.as_deref().unwrap_or("sv10"))
    }

    /// The lane's champion: its own parameters, else the shared champion's.
    pub fn params(&self, store: &Store) -> Params {
        let read = |key: String| store.get_kv(&key).ok().flatten().and_then(|s| serde_json::from_str(&s).ok());
        read(self.params_key()).or_else(|| read(crate::PARAMS_KEY.to_string())).unwrap_or_default()
    }

    /// The lane's promoted versions, oldest first: its own list, else a copy of the shared one.
    pub fn lineage(&self, store: &Store) -> Vec<String> {
        let read = |key: String| store.get_kv(&key).ok().flatten().and_then(|s| serde_json::from_str(&s).ok());
        read(self.key(crate::LEARNER_LINEAGE_KEY))
            .or_else(|| read(crate::LEARNER_LINEAGE_KEY.to_string()))
            .unwrap_or_else(|| vec!["sv10-ev-1".to_string()])
    }
}

/// A lineage's latest gene transfer, adopted (`.slot.<bot>` suffix per lane).
pub const LAST_TRANSFER_KEY: &str = "learner.last-transfer.v1";

/// The dashboard's rows, one per bot, once any bot has a lineage of its own; empty while the shared
/// champion plays them all.
pub fn dashboard(store: &Store, bots: &[String]) -> Value {
    let own = |b: &String| store.get_kv(&Lane::bot(b).params_key()).ok().flatten().is_some();
    if !bots.iter().any(own) {
        return json!([]);
    }
    let kv = |key: String| store.get_kv(&key).ok().flatten().and_then(|s| serde_json::from_str::<Value>(&s).ok());
    bots.iter()
        .map(|b| {
            let lane = Lane::bot(b);
            let lineage = lane.lineage(store);
            let prefix = format!("{b}-ev-");
            json!({"bot": b, "own": own(b), "version": lineage.last(), "promotions": lineage.iter().filter(|v| v.starts_with(&prefix)).count(),
                "last_transfer": kv(lane.key(LAST_TRANSFER_KEY)), "last_refresh": kv(lane.key(super::tournament::LAST_REFRESH_KEY))})
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_lane_starts_as_a_copy_and_keeps_its_own_keys() {
        let shared = crate::live::Shared::for_test("lane-copy", &["A"]);
        let store = &shared.store;
        store.put_kv(crate::LEARNER_LINEAGE_KEY, r#"["sv10-ev-1","sv10-ev-2"]"#).unwrap();
        store.put_kv(crate::PARAMS_KEY, &serde_json::to_string(&Params { call_margin: 0.5, ..Default::default() }).unwrap()).unwrap();
        let a = Lane::bot("A");
        assert_eq!(a.params(store).call_margin, 0.5);
        assert_eq!(a.lineage(store), ["sv10-ev-1", "sv10-ev-2"]);
        store.put_kv(&a.params_key(), &serde_json::to_string(&Params { call_margin: 0.9, ..Default::default() }).unwrap()).unwrap();
        assert_eq!(a.params(store).call_margin, 0.9);
        assert_eq!(Lane::default().params(store).call_margin, 0.5, "the shared champion is untouched");
        assert_eq!(Lane::default().key("k"), "k");
        assert_eq!(a.key("k"), "k.slot.A");
        assert_eq!(a.version(3), "A-ev-3");
        let rows = dashboard(store, &["A".into(), "B".into()]);
        assert_eq!((&rows[0]["own"], &rows[1]["own"]), (&json!(true), &json!(false)));
        assert_eq!((&rows[1]["version"], &rows[1]["promotions"]), (&json!("sv10-ev-2"), &json!(0)));
        assert_eq!(Lane::rotation(None), [Lane::default()]);
        assert_eq!(Lane::rotation(Some("A, B,")), [Lane::bot("A"), Lane::bot("B")]);
    }
}
