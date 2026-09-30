//! A search taken in many stored steps decides what one uninterrupted search decides (0334).

use super::super::run::{self, RESUME_SLICE_SECS, Rate, Run, SLICE_TARGET_SECS};
use super::super::{Ctx, POPULATION_MODELS_KEY};
use super::{Outcome, begin, step};
use crate::search_ledger;
use sv10_core::model::{ModelStore, PlayerStats};
use sv10_store::store::Store;

fn store(tag: &str) -> (std::path::PathBuf, Store) {
    let dir = std::env::temp_dir().join(format!("sv10-search-steps-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let store = Store::open(&dir.join("svanbot10.db")).unwrap();
    let mut models = ModelStore::default();
    for (i, hands) in [60.0, 55.0, 50.0, 45.0, 40.0].into_iter().enumerate() {
        models.players.insert(format!("villain{i}"), PlayerStats { hands, ..Default::default() });
    }
    store.put_kv(POPULATION_MODELS_KEY, &serde_json::to_string(&models).unwrap()).unwrap();
    (dir, store)
}

fn ctx<'a>(dir: &'a std::path::Path, store: &'a Store) -> Ctx<'a> {
    Ctx { store, root: dir, threads: 2, tables: 1, hands: 20, decision_samples: 40 }
}

#[test]
fn live_supported_confirmation_records_the_whole_offered_funnel() {
    use crate::experiment::target::{Source, TARGETS_KEY, Target, TargetQueue};
    use crate::learner::funnel::{self, BARRED, FUNNEL_KEY, PROPOSED};
    use crate::search_ledger::{Ledger, LedgerEntry, transition_key};
    let (dir, store) = store("live-funnel");
    let ctx = ctx(&dir, &store);
    let mut run = begin(&ctx, 0, 0.0, 0).unwrap().unwrap();
    let scope = super::Scope::load(&store);
    let env = super::env(&ctx, &scope, &run, vec![]);
    let mut ledger = Ledger { champion: run.champion_version.clone(), refit_rowid: run.refit_rowid, ..Default::default() };
    let (key, old, new, _) = &env.proposals[0];
    ledger.entries.insert(transition_key(key, *old, *new), LedgerEntry { hands: 20, mean_bb: -0.02, se_bb: 0.0, differing: 1 });
    search_ledger::save(&store, &ledger).unwrap();
    let (key, old, new, params) = &env.proposals[1];
    let target =
        Target::new((&ledger.champion, ledger.refit_rowid), key, *old, *new, params.clone(), LedgerEntry::default(), Source::Ledger);
    let verdicts = crate::experiment::Verdicts::from([(
        target.id.clone(),
        crate::experiment::VerdictRecord {
            verdict: crate::experiment::Verdict::LiveSupported,
            at: 1.0,
            label: target.label(),
            estimate: Default::default(),
        },
    )]);
    let queue =
        TargetQueue { champion: ledger.champion, refit_rowid: ledger.refit_rowid, candidates: vec![target.clone()], ..Default::default() };
    store.put_kv(TARGETS_KEY, &serde_json::to_string(&queue).unwrap()).unwrap();
    store.put_kv(crate::experiment::VERDICTS_KEY, &serde_json::to_string(&verdicts).unwrap()).unwrap();
    store.put_kv(FUNNEL_KEY, "{}").unwrap();
    let stage = super::stages::start_halving(&env, &mut run);
    assert!(matches!(stage, run::Stage::Confirm(c) if c.knob == target.knob));
    let counts = funnel::load(&store).summary(crate::learner::now());
    let count =
        |key: &str| counts["outcomes"].as_array().unwrap().iter().find(|v| v["key"] == key).and_then(|v| v["count"].as_u64()).unwrap_or(0);
    assert_eq!(count(BARRED), 1);
    assert_eq!(count(PROPOSED), env.proposals.len() as u64 - 1);
    std::fs::remove_dir_all(dir).unwrap();
}

/// Run a whole search at `rate` with `cap` seconds of simulation planned for each step's first
/// slice, storing and reloading the run between steps; the outcome, the steps taken and the ledger
/// it left.
fn search(tag: &str, rate: f64, cap: f64) -> (Outcome, u32, search_ledger::Ledger) {
    let (dir, store) = store(tag);
    let ctx = ctx(&dir, &store);
    let mut s = begin(&ctx, 0, 0.0, 0).unwrap().expect("five opponents are enough to search");
    s.rate = Rate(rate);
    run::save(&store, &Run::Search(Box::new(s))).unwrap();
    loop {
        let Some(Run::Search(mut s)) = run::load(&store) else { panic!("the run was stored") };
        let outcome = step(&ctx, &mut s, cap).unwrap();
        if outcome != Outcome::Continue {
            let ledger = search_ledger::load(&store, &s.champion_version, s.refit_rowid);
            let _ = std::fs::remove_dir_all(&dir);
            return (outcome, s.steps, ledger);
        }
        assert!(s.steps < 500, "a search must end");
        run::save(&store, &Run::Search(s)).unwrap();
    }
}

#[test]
fn a_search_in_stored_one_slice_steps_decides_as_one_step_does() {
    let (whole, whole_steps, whole_ledger) = search("whole", 1e9, SLICE_TARGET_SECS);
    // A rate this low plans one table run per slice and one slice per step.
    let (sliced, sliced_steps, sliced_ledger) = search("sliced", 0.02, SLICE_TARGET_SECS);
    // The same search at a rate where the slice cap bites: the conservative first-cap plans 5 runs
    // against 20, so the first batch of round 0 is three candidates instead of five (0343). The
    // slicing differs; what the search decides must not.
    let (capped, _, capped_ledger) = search("capped", 0.5, RESUME_SLICE_SECS);
    assert!(matches!(whole, Outcome::Finished { .. }), "{whole:?}");
    assert_eq!(sliced, whole);
    assert_eq!(capped, whole);
    assert_eq!(whole_steps, 1, "everything fits one step at this rate");
    assert!(sliced_steps > 30, "{sliced_steps} steps: every candidate of r0 is its own step");
    assert_eq!(whole_ledger.entries.keys().collect::<Vec<_>>(), capped_ledger.entries.keys().collect::<Vec<_>>());
    for (k, a) in &whole_ledger.entries {
        let b = &capped_ledger.entries[k];
        assert_eq!((a.hands, a.differing), (b.hands, b.differing), "{k}");
        assert!((a.mean_bb - b.mean_bb).abs() < 1e-9 && (a.se_bb - b.se_bb).abs() < 1e-9, "{k}: {a:?} vs {b:?}");
    }
    assert!(!whole_ledger.entries.is_empty(), "the search measured transitions");
    assert_eq!(whole_ledger.entries.keys().collect::<Vec<_>>(), sliced_ledger.entries.keys().collect::<Vec<_>>());
    for (k, a) in &whole_ledger.entries {
        let b = &sliced_ledger.entries[k];
        assert_eq!((a.hands, a.differing), (b.hands, b.differing), "{k}");
        assert!((a.mean_bb - b.mean_bb).abs() < 1e-9 && (a.se_bb - b.se_bb).abs() < 1e-9, "{k}: {a:?} vs {b:?}");
    }
    // The survivor went through a sliced fresh-deal confirmation and was judged the same way.
    assert!(!whole_ledger.confirm_rejected.is_empty() || whole == Outcome::Finished { promoted: true });
    assert_eq!(whole_ledger.confirm_rejected, sliced_ledger.confirm_rejected);
}

#[test]
fn a_champion_change_between_steps_abandons_the_search() {
    let (dir, store) = store("abandon");
    let ctx = ctx(&dir, &store);
    let mut s = begin(&ctx, 0, 0.0, 0).unwrap().unwrap();
    s.rate = Rate(0.02);
    // One step to get the run under way. Deliberately not asserted to be `Continue`: a machine fast
    // enough to finish the whole gate inside the first slice does that instead, and the outcome then
    // turns on whether the board-strength tables are present (equity::tables::loaded recomputes
    // every board without them), which is not what this test is about. The assertion below holds
    // either way, because `step` compares the champion digest before it does anything else.
    let started = step(&ctx, &mut s, SLICE_TARGET_SECS).unwrap();
    assert!(!matches!(started, Outcome::Abandoned(_)), "nothing has changed yet: {started:?}");
    let moved = sv10_core::policy::Params { open_bb: 3.1, ..Default::default() };
    store.put_kv(crate::PARAMS_KEY, &serde_json::to_string(&moved).unwrap()).unwrap();
    assert_eq!(step(&ctx, &mut s, SLICE_TARGET_SECS).unwrap(), Outcome::Abandoned("the champion's parameters changed"));
    let _ = std::fs::remove_dir_all(&dir);
}
