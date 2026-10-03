//! Tests for the drift instrument (0334, 0366), split out of `review_drift.rs` (the
//! 500-line rule).

use super::*;
use crate::{ANALYST_DRIFT_KEY as DRIFT_KEY, PARAMS_KEY};

fn dir(name: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("sv10-drift-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn small_replay() -> ReplayRecord {
    use crate::replay::record;
    use sv10_core::engine::{Action, Hand};
    use sv10_core::model::{Counter, ModelStore, PlayerStats};
    use sv10_core::policy::decide_with;
    use sv10_core::situation::Situation;
    use sv10_rng::SeedableRng;
    use sv10_rng::rngs::SmallRng;
    let mut rng = SmallRng::seed_from_u64(3);
    let mut hand = Hand::new(&[40_000, 40_000, 40_000], 0, 10, 20, &mut rng);
    hand.apply(Action::RaiseTo(60)).unwrap();
    hand.apply(Action::RaiseTo(6_000)).unwrap();
    let actor = hand.actor().unwrap();
    let names: Vec<String> = ["reg", "station", "maniac"].iter().map(|s| s.to_string()).collect();
    let sit = Situation::from_hand(&hand, actor, &names);
    let mut models = ModelStore::default();
    for (name, vpip) in [("reg", 0.22f32), ("station", 0.6), ("maniac", 0.5)] {
        let st = PlayerStats { hands: 300.0, vpip: Counter { opp: 300.0, hit: vpip * 300.0 }, ..Default::default() };
        models.players.insert(name.into(), st);
    }
    let params = Params { samples: 64, ..Default::default() };
    let d = decide_with(&sit, &models, &params, None, &mut SmallRng::seed_from_u64(99));
    record(&sit, 99, &params, &models, None, &d)
}

/// A whole drift check, slice after slice, as the analyst's idle loop runs it.
fn drift_to_the_end(store: &Store, nets: &mut NetCache) {
    let mut run = None;
    while drift_check(store, nets, 1, 64, &mut run, DRIFT_STEP) {}
}

/// Store a row (as an object, so its basis can be set) under the drift kv; an empty `basis`
/// stores the tag-less shape the analyst wrote before 0366.
fn older_row(store: &Store, value: &serde_json::Value, basis: &str) {
    let mut row = value.clone();
    if basis.is_empty() {
        row.as_object_mut().unwrap().remove("basis");
    } else {
        row["basis"] = serde_json::json!(basis);
    }
    store.put_kv(DRIFT_KEY, &row.to_string()).unwrap();
}

/// 0334: a drift check never holds the cores past its slice. With no time budget each call re-solves
/// one replay, and the summary is stored only once every replay of the run is judged.
#[test]
fn a_drift_check_works_in_slices_and_reports_once_complete() {
    let store = Store::open(&dir("drift-slices").join("svanbot10.db")).unwrap();
    let mut nets = NetCache::new();
    let rec = small_replay();
    for h in ["h1", "h2", "h3"] {
        store.insert_replay("b", h, &serde_json::to_string(&rec).unwrap(), None).unwrap();
    }
    let changed = Params { call_margin: 1.0, ..Default::default() };
    store.put_kv(PARAMS_KEY, &serde_json::to_string(&changed).unwrap()).unwrap();
    let mut run = None;
    assert!(drift_check(&store, &mut nets, 1, 64, &mut run, Duration::ZERO), "one of three judged: still under way");
    assert_eq!(run.as_ref().map(|r| r.n), Some(1));
    assert!(store.get_kv(DRIFT_KEY).unwrap().is_none(), "no summary from a partial run");
    assert!(drift_check(&store, &mut nets, 1, 64, &mut run, Duration::ZERO));
    assert!(!drift_check(&store, &mut nets, 1, 64, &mut run, Duration::ZERO), "the third slice completes it");
    let v: serde_json::Value = serde_json::from_str(&store.get_kv(DRIFT_KEY).unwrap().unwrap()).unwrap();
    assert_eq!(v["replays"], 3);
}

#[test]
fn drift_reports_once_per_champion() {
    let store = Store::open(&dir("drift").join("svanbot10.db")).unwrap();
    let mut nets = NetCache::new();
    // No champion yet: no-op, nothing stored.
    drift_to_the_end(&store, &mut nets);
    assert!(store.get_kv(DRIFT_KEY).unwrap().is_none());
    let params = Params::default();
    store.put_kv(PARAMS_KEY, &serde_json::to_string(&params).unwrap()).unwrap();
    // Champion present but no replays: still nothing stored.
    drift_to_the_end(&store, &mut nets);
    assert!(store.get_kv(DRIFT_KEY).unwrap().is_none());
    // One replay, then a changed champion: one summary row stored.
    let rec = small_replay();
    store.insert_replay("b", "h", &serde_json::to_string(&rec).unwrap(), None).unwrap();
    let changed = Params { call_margin: 1.0, ..Default::default() };
    store.put_kv(PARAMS_KEY, &serde_json::to_string(&changed).unwrap()).unwrap();
    drift_to_the_end(&store, &mut nets);
    let raw = store.get_kv(DRIFT_KEY).unwrap().expect("drift summary stored");
    let v: serde_json::Value = serde_json::from_str(&raw).unwrap();
    assert_eq!(v["replays"], 1);
    assert!(v["flip_rate"].as_f64().unwrap().is_finite());
    assert!(v["mean_gap_bb"].as_f64().unwrap().is_finite());
    // The row names the basis it was measured on, the budget, the population and the prices
    // (0366; LESSONS 39: an instrument names its population and its basis wherever it prints).
    assert_eq!(v["basis"], DRIFT_BASIS);
    assert_eq!(v["samples"], 64);
    assert_eq!(v["population"]["version"], REPLAY_VERSION);
    assert!(v["population"]["versions"].as_str().unwrap().starts_with("v3 "), "{v}");
    assert!(v["fits"].as_str().unwrap().contains("rec.params"), "{v}");
    // Same champion again: digest matches, the summary is untouched.
    drift_to_the_end(&store, &mut nets);
    assert_eq!(store.get_kv(DRIFT_KEY).unwrap().unwrap(), raw);
}

/// 0366: the basis is part of what the row *means*, so a row measured on another one is due for a
/// re-check instead of being read as current — the row the live analyst stored before this change
/// carries no basis tag at all, and the digest alone cannot tell it apart from a current one.
#[test]
fn a_row_on_a_superseded_basis_is_due_for_a_re_check() {
    let store = Store::open(&dir("drift-basis").join("svanbot10.db")).unwrap();
    let mut nets = NetCache::new();
    let rec = small_replay();
    store.insert_replay("b", "h", &serde_json::to_string(&rec).unwrap(), None).unwrap();
    let champion = Params { call_margin: 1.0, ..Default::default() };
    store.put_kv(PARAMS_KEY, &serde_json::to_string(&champion).unwrap()).unwrap();
    drift_to_the_end(&store, &mut nets);
    let measured = store.get_kv(DRIFT_KEY).unwrap().expect("drift summary stored");
    let digest = serde_json::from_str::<serde_json::Value>(&measured).unwrap()["digest"].clone();

    // Current champion, current basis: the row stands.
    drift_to_the_end(&store, &mut nets);
    assert_eq!(store.get_kv(DRIFT_KEY).unwrap().unwrap(), measured);

    // The row the live analyst stored before this change carries no basis tag at all while naming
    // this same champion digest, so only the tag can tell it apart from a current one.
    let stored_row: serde_json::Value = serde_json::from_str(&measured).unwrap();
    assert_eq!(stored_row["digest"], digest, "the superseded row is identifiable: same champion, no basis");
    older_row(&store, &stored_row, "");
    drift_to_the_end(&store, &mut nets);
    let after = serde_json::from_str::<serde_json::Value>(&store.get_kv(DRIFT_KEY).unwrap().unwrap()).unwrap();
    assert_eq!(after["basis"], DRIFT_BASIS, "a row with no basis tag is re-measured on the current one");

    // An older tag is due for the same reason, with the champion unmoved.
    older_row(&store, &stored_row, "champion-alone/v0");
    drift_to_the_end(&store, &mut nets);
    let after = serde_json::from_str::<serde_json::Value>(&store.get_kv(DRIFT_KEY).unwrap().unwrap()).unwrap();
    assert_eq!(after["basis"], DRIFT_BASIS, "a row on another basis is re-measured even though the champion did not move");

    // And a champion that moved is still due, as before this change.
    older_row(&store, &stored_row, DRIFT_BASIS);
    store.put_kv(PARAMS_KEY, &serde_json::to_string(&Params { call_margin: 3.0, ..Default::default() }).unwrap()).unwrap();
    drift_to_the_end(&store, &mut nets);
    let moved = serde_json::from_str::<serde_json::Value>(&store.get_kv(DRIFT_KEY).unwrap().unwrap()).unwrap();
    assert_ne!(moved["digest"], digest);
}

/// 0366: the sample is the newest replays *of the current version*. A pre-v3 record carries no
/// per-opponent corrections, so re-solving it compares the live action against a model that saw
/// less than live play did — the disagreement family this check removes from its prices.
#[test]
fn the_sample_is_pinned_to_the_current_replay_version() {
    let store = Store::open(&dir("drift-version").join("svanbot10.db")).unwrap();
    let mut nets = NetCache::new();
    let rec = small_replay();
    let of_version = |v: u32| {
        let mut r = rec.clone();
        r.version = v;
        serde_json::to_string(&r).unwrap()
    };
    store.insert_replay("b", "v0", &of_version(0), None).unwrap();
    store.insert_replay("b", "v2", &of_version(2), None).unwrap();
    store.insert_replay("b", "v3a", &of_version(REPLAY_VERSION), None).unwrap();
    store.insert_replay("b", "v3b", &of_version(REPLAY_VERSION), None).unwrap();
    let ids: Vec<i64> = store.replays(10, None).unwrap().iter().map(|r| r.id).collect();
    let changed = Params { call_margin: 1.0, ..Default::default() };
    store.put_kv(PARAMS_KEY, &serde_json::to_string(&changed).unwrap()).unwrap();
    drift_to_the_end(&store, &mut nets);
    let v: serde_json::Value = serde_json::from_str(&store.get_kv(DRIFT_KEY).unwrap().expect("a row: the sample was not empty")).unwrap();
    // Only the current version is judged, and the row says what it read and what the filter dropped.
    assert_eq!(v["replays"], 2, "the two older-version records are not re-solved");
    assert_eq!(v["population"]["ids"], format!("{}-{}", ids[1], ids[0]));
    assert_eq!(v["population"]["scanned"], 4);
    assert_eq!(v["population"]["other_version"], 2, "the two pre-v3 rows the filter dropped");
    let mix = v["population"]["versions"].as_str().unwrap();
    assert!(mix.contains("v3 2") && mix.contains("v2 1") && mix.contains("v0 1"), "{mix}");

    // A store holding no current-version record stores nothing, so the check stays due and the
    // number is not silently measured on records that cannot be compared.
    let store = Store::open(&dir("drift-version-nothing").join("svanbot10.db")).unwrap();
    store.insert_replay("b", "v0", &of_version(0), None).unwrap();
    store.put_kv(PARAMS_KEY, &serde_json::to_string(&changed).unwrap()).unwrap();
    drift_to_the_end(&store, &mut nets);
    assert!(store.get_kv(DRIFT_KEY).unwrap().is_none(), "no v{REPLAY_VERSION} record: nothing stored");

    // A window that is all current: the filter drops nothing, and the row says zero. Rows read past
    // the 48 are of the right version and are not that number (the live 192-row read is this case).
    let store = Store::open(&dir("drift-version-all").join("svanbot10.db")).unwrap();
    for h in ["a", "b", "c"] {
        store.insert_replay("b", h, &of_version(REPLAY_VERSION), None).unwrap();
    }
    store.put_kv(PARAMS_KEY, &serde_json::to_string(&changed).unwrap()).unwrap();
    drift_to_the_end(&store, &mut nets);
    let v: serde_json::Value = serde_json::from_str(&store.get_kv(DRIFT_KEY).unwrap().unwrap()).unwrap();
    assert_eq!(v["replays"], 3);
    assert_eq!(v["population"]["scanned"], 3);
    assert_eq!(v["population"]["other_version"], 0, "a window of current-version rows drops none to the filter");
}

/// 0366: the re-solve varies the champion's knobs and nothing else. The record's own local set —
/// its self-calibration, here — is what the decision was priced with, so a re-solve that carries
/// it reproduces the live correction and one that does not inherits an unpriced action.
#[test]
fn both_re_solves_run_at_the_records_own_prices() {
    let mut rec = small_replay();
    let replay = rerun(&rec, &rec.params, None);
    assert_eq!(
        (replay.action_name.as_str(), replay.amount),
        (rec.action.0.as_str(), rec.action.1),
        "the record replays exactly, so the two re-solves below differ only in the prices"
    );
    let chosen = replay.chosen.clone();
    let other = replay
        .candidates
        .iter()
        .find(|c| c.action != chosen.action && c.category.is_some())
        .expect("a priced alternative to the action we took")
        .clone();
    // The recorded decision's own price correction, on the action it did not take.
    let category = other.category.clone().unwrap();
    rec.params.ev_bias.insert(category.clone(), 1_000.0);
    // The champion's knobs at the record's own budget, so the bias below is the only difference
    // between the two re-solves and cannot be confused with the deeper search.
    let champion = Params { samples: rec.params.samples, ..Default::default() };

    // Today's champion alone: carrying no live fits, it replays the live action.
    let bare = rerun(&rec, &champion, None);
    assert_eq!((bare.action_name.as_str(), bare.amount), (rec.action.0.as_str(), rec.action.1));

    // The champion at the recorded decision's own prices: the correction moves the action, which
    // is a flip the rate must be able to lay at the champion's door.
    let deep = drift_basis(&champion, &rec, 64, 3);
    assert_eq!(deep.ev_bias[&category], 1_000.0, "the record's own prices are taken");
    assert_eq!(deep.call_margin, champion.call_margin, "the champion's knobs are kept");
    // The budget and the deal decomposition are the analyst's, over both the record's and the
    // champion's (`deal_chunks` is not bit-neutral, so this is not a free choice either way).
    assert_eq!((rec.params.samples, rec.params.deal_chunks), (64, 1));
    assert_eq!((champion.samples, champion.deal_chunks), (64, 1));
    assert_eq!((deep.samples, deep.deal_chunks), (64, 3));
    assert_eq!(drift_basis(&champion, &rec, 99, 5).samples, 99, "and it is the number handed in, not a constant");
    let at_recorded = rerun(&rec, &deep, None);
    assert_ne!(
        (at_recorded.action_name.as_str(), at_recorded.amount),
        (rec.action.0.as_str(), rec.action.1),
        "the record's own prices are part of the basis, not noise the champion is charged for"
    );
}

/// 0366: the row is read on one line that names the basis, the budget, the population and the
/// version mix, and says whether the analyst would store that row now — a superseded row is
/// identifiable and due for a re-check, not a number silently read as current.
#[test]
fn the_drift_row_names_its_basis_and_says_when_it_is_due() {
    let champion = r#"{"call_margin":-0.045}"#;
    let digest = params_digest(champion);
    let current = render(Some(&row(DRIFT_BASIS, &digest)), Some(champion));
    for field in
        [DRIFT_BASIS, "16000000 samples x 8 chunks", "3471", "13:21", "192 rows read", "0 of them not replay v3", "v3 48", "rec.params"]
    {
        assert!(current.contains(field), "{field:?} missing from {current}");
    }
    assert!(current.contains("current: champion"), "{current}");

    // The live row before this change carries no basis tag at all: still readable, still named
    // as lacking one, and due.
    let tagless = render(Some(&row("", &digest)), Some(champion));
    assert!(tagless.contains("superseded basis (none stored)"), "{tagless}");
    assert!(tagless.contains("due for a re-check"), "{tagless}");

    // An older tag, and a champion that moved since: both due, and for the stated reason.
    assert!(render(Some(&row("champion-alone/v0", &digest)), Some(champion)).contains("superseded basis"));
    let moved = render(Some(&row(DRIFT_BASIS, &digest)), Some(r#"{"call_margin":0.0}"#));
    assert!(moved.contains("the champion changed since it was measured"), "{moved}");
    assert!(moved.contains(&digest), "the row's own digest is named: {moved}");

    // Nothing stored, and no champion to compare with.
    assert!(render(None, Some(champion)).contains("no row stored"));
    assert!(render(Some(&row(DRIFT_BASIS, &digest)), None).contains("no champion (params.v1) stored"));
}

/// #723: `--json` is one parseable object carrying the stored row exactly as the analyst wrote it
/// and the same verdict the text line ends with; the text path is unchanged.
#[test]
fn the_json_render_carries_the_row_and_the_same_verdict() {
    let champion = r#"{"call_margin":-0.045}"#;
    let digest = params_digest(champion);
    let store = Store::open(&dir("json").join("t.db")).unwrap();
    store.put_kv(DRIFT_KEY, &row(DRIFT_BASIS, &digest)).unwrap();
    store.put_kv(PARAMS_KEY, champion).unwrap();
    let text = drift(&store, false).unwrap();
    assert!(text.contains("current: champion"), "{text}");
    let json: serde_json::Value = serde_json::from_str(&drift(&store, true).unwrap()).unwrap();
    assert_eq!(json["basis"], DRIFT_BASIS);
    assert_eq!(json["row"]["flips"], 2);
    assert_eq!(json["row"]["population"]["ids"], "3471-3518");
    assert!(json["verdict"].as_str().unwrap().contains("current: champion"), "{json}");
    // A row on a superseded basis: the JSON's `basis` is the row's own — the one the text line
    // names — and the current basis appears only inside the verdict (review of #723).
    store.put_kv(DRIFT_KEY, &row("champion-alone/v0", &digest)).unwrap();
    let text = drift(&store, false).unwrap();
    assert!(text.contains("drift [champion-alone/v0]"), "{text}");
    let json: serde_json::Value = serde_json::from_str(&drift(&store, true).unwrap()).unwrap();
    assert_eq!(json["basis"], "champion-alone/v0");
    assert_eq!(json["row"]["basis"], "champion-alone/v0");
    let verdict = json["verdict"].as_str().unwrap();
    assert!(verdict.contains("superseded basis (\"champion-alone/v0\")"), "{verdict}");
    assert!(verdict.contains(&format!("the analyst measures on {DRIFT_BASIS}")), "{verdict}");
    let none: serde_json::Value = serde_json::from_str(&render_json(None, Some(champion))).unwrap();
    assert!(none["row"].is_null(), "{none}");
    assert!(none["basis"].is_null(), "no row, no basis: {none}");
    assert!(none["verdict"].as_str().unwrap().contains("no row stored"), "{none}");
}

/// A row as the analyst writes it, with `basis`/`digest` overridable so the two ways a row can
/// be superseded — an older basis tag, or a champion that moved — are both exercised.
fn row(basis: &str, digest: &str) -> String {
    serde_json::json!({
        "basis": basis, "digest": digest, "ts": 1.0, "replays": 48, "flips": 2, "flip_rate": 2.0 / 48.0,
        "mean_gap_bb": 0.06, "max_gap_bb": 2.05, "samples": 16_000_000, "deal_chunks": 8,
        "fits": "rec.params (the recorded decision's own prices)",
        "population": {"ids": "3471-3518", "from": "2026-09-27T07:35:08.706432458+00:00",
            "to": "2026-09-27T13:21:00.0+00:00", "scanned": 192, "other_version": 0, "version": 3,
            "versions": "v3 48, v2 90, v0 54"}
    })
    .to_string()
}

#[test]
fn champion_digests_are_stable_and_sensitive() {
    assert_eq!(params_digest("abc"), params_digest("abc"));
    assert_ne!(params_digest("abc"), params_digest("abd"));
}
