//! Deterministic replay of big live decisions.
//!
//! A decision is fully determined by its situation, the Monte Carlo seed, the live parameters
//! (range model and calibration included), the stats of the players at the table plus the
//! population, and the response network. For big spots (see [`is_big_spot`]) the client stores
//! all of that as a [`ReplayRecord`]; `review replay` re-runs the policy on it and reports whether
//! the result is bit-identical, or — with a changed build or parameters — how the choice moves.
//! Records are kept 14 days (`tasks`).

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use sv10_core::engine::Action;
use sv10_core::model::{ModelStore, PlayerStats};
use sv10_core::nn::Mlp;
use sv10_core::policy::{Decision, Params, decide_with};
use sv10_core::situation::Situation;
use sv10_digest::Sha256;
use sv10_rng::SeedableRng;
use sv10_rng::rngs::SmallRng;
use sv10_store::store as sv10_bot_store;

/// Days of replay records kept.
pub const KEEP_DAYS: i64 = 14;
/// Current replay JSON contract. Version zero is legacy serde input without an explicit field;
/// version 3 added per-opponent corrections; version 4 preserves the hero's table image (#909).
pub const REPLAY_VERSION: u32 = 4;

/// The per-opponent corrections live play had installed for one player (they sit on `ModelStore`,
/// not in `PlayerStats`, so a record without them re-ran every decision against an uncorrected model:
/// replays were not exact and every deep audit graded live choices against a model missing them).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Corrections {
    /// Response-network ratios (0210).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub response_ratio: Option<[f32; 3]>,
    /// Heads-up postflop fold logit offset (0214).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fold_offset: Option<f32>,
    /// River sizing tell (0223).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size_tell: Option<f32>,
}

/// Everything a live decision depended on, plus what it chose.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ReplayRecord {
    /// Version of the captured decision-input contract.
    #[serde(default)]
    pub version: u32,
    /// Seed of the decision's Monte Carlo generator.
    pub seed: u64,
    /// The decision input.
    pub situation: Situation,
    /// Live parameters at decision time.
    pub params: Params,
    /// Stats of every player in the situation that the model knew.
    pub players: BTreeMap<String, PlayerStats>,
    /// Population stats (shrinkage prior).
    pub population: PlayerStats,
    /// Captured per-bot and aggregate table images. None means unknown legacy input; Some(empty)
    /// means capture verified that no image was installed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hero_seen: Option<BTreeMap<String, PlayerStats>>,
    /// SHA-256 (hex, 16 chars) of the response network used, if one was active.
    pub net_digest: Option<String>,
    /// Per-opponent corrections in force for the players in the situation (version 3; absent before).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub corrections: BTreeMap<String, Corrections>,
    /// Chosen action name and raise-to amount.
    pub action: (String, Option<i64>),
    /// Every candidate: action, amount, EV.
    pub candidates: Vec<(String, Option<i64>, f64)>,
}

/// Big spots worth recording: pot of 250 bb or more, a call of 100 bb or more, or an all-in.
pub fn is_big_spot(sit: &Situation, d: &Decision) -> bool {
    let bb = sit.bb.max(1);
    sit.pot >= 250 * bb || sit.call_amount >= 100 * bb || matches!(d.action, Action::AllIn) || d.action_name == "all_in"
}

/// Digest and JSON of a network, as stored in `replay_nets`.
pub fn net_digest(nn: &Mlp) -> Option<(String, String)> {
    let json = serde_json::to_string(nn).ok()?;
    let digest = sv10_digest::hex(Sha256::digest(json.as_bytes()))[..16].to_string();
    Some((digest, json))
}

/// Build the record for a decision just made from exactly these inputs.
pub fn record(sit: &Situation, seed: u64, params: &Params, models: &ModelStore, net_digest: Option<String>, d: &Decision) -> ReplayRecord {
    let players = sit.players.iter().filter_map(|p| models.players.get(&p.name).map(|s| (p.name.clone(), s.clone()))).collect();
    let corrections = sit
        .players
        .iter()
        .filter_map(|p| {
            let c = Corrections {
                response_ratio: models.response_ratios.get(&p.name).copied(),
                fold_offset: models.fold_offsets.get(&p.name).copied(),
                size_tell: models.size_tells.get(&p.name).copied(),
            };
            (c != Corrections::default()).then(|| (p.name.clone(), c))
        })
        .collect();
    ReplayRecord {
        version: REPLAY_VERSION,
        seed,
        situation: sit.clone(),
        params: params.clone(),
        players,
        population: models.population.clone(),
        hero_seen: Some(
            [format!("{}{}", sv10_core::model::HERO_SEEN_ONE, sit.hero().name), sv10_core::model::HERO_SEEN_ALL.into()]
                .into_iter()
                .filter_map(|key| models.hero_seen.get(&key).map(|stats| (key, stats.clone())))
                .collect(),
        ),
        net_digest,
        corrections,
        action: (d.action_name.clone(), d.amount),
        candidates: candidates(d),
    }
}

/// Whether parameters and response-network bytes match the recorded decision inputs. A rerun with
/// different inputs is useful, but it is a what-if and must not be reported as exact replay.
pub fn exact_inputs(rec: &ReplayRecord, params: &Params, nn: Option<&Mlp>) -> bool {
    let params_match = serde_json::to_vec(&rec.params).ok() == serde_json::to_vec(params).ok();
    let digest = nn.and_then(net_digest).map(|(digest, _)| digest);
    params_match && rec.net_digest == digest && (rec.hero_seen.is_some() || params.hero_image <= 0.0)
}

fn candidates(d: &Decision) -> Vec<(String, Option<i64>, f64)> {
    d.candidates.iter().map(|c| (c.action.clone(), c.amount, c.ev)).collect()
}

/// Re-run the policy on a record with `params` (the recorded ones for an exact replay) and `nn`.
pub fn rerun(rec: &ReplayRecord, params: &Params, nn: Option<&Mlp>) -> Decision {
    let pick = |f: &dyn Fn(&Corrections) -> Option<f32>| -> std::sync::Arc<std::collections::HashMap<String, f32>> {
        std::sync::Arc::new(rec.corrections.iter().filter_map(|(n, c)| f(c).map(|v| (n.clone(), v))).collect())
    };
    let models = ModelStore {
        players: rec.players.clone().into_iter().collect(),
        population: rec.population.clone(),
        hero_seen: rec.hero_seen.clone().unwrap_or_default().into_iter().collect(),
        response_ratios: std::sync::Arc::new(
            rec.corrections.iter().filter_map(|(n, c)| c.response_ratio.map(|r| (n.clone(), r))).collect(),
        ),
        fold_offsets: pick(&|c| c.fold_offset),
        size_tells: pick(&|c| c.size_tell),
        ..Default::default()
    };
    let mut rng = SmallRng::seed_from_u64(rec.seed);
    decide_with(&rec.situation, &models, params, nn, &mut rng)
}

/// `raise:640` style label of an action and its amount.
pub fn action_label(action: &str, amount: Option<i64>) -> String {
    match amount {
        Some(a) => format!("{action}:{a}"),
        None => action.to_string(),
    }
}

/// Re-solve a recorded live decision with `deep` parameters (the recorded strategy with a much larger
/// Monte Carlo budget) and measure the live choice against it: the deep EV of the deep search's best
/// action minus the deep EV of the live action (for a raise, the deep candidate of the same kind with
/// the nearest amount), in big blinds, never below zero.
/// Pot size (big blinds) below which the analyst skips a decision by default (`ANALYST_MIN_POT_BB`).
/// 2026-09-23, 24 h of audits: pots under 50 bb took 91% of the analyst's CPU (4.98 of 5.49 h) for
/// 0.005 bb of measured gap per decision; the big spots, where mistakes cost chips, took 9%.
pub const AUDIT_MIN_POT_BB: f64 = 50.0;

/// The share of the pot a call must be for [`worth_auditing`] to audit a spot on its size alone, and
/// the share of `min_pot_bb` it must be at least. One constant for both, because the predicate tests
/// the same fraction twice and [`audit_filter`] renders both numbers from it. A quarter is a power of
/// two, so writing the predicate's old `call * 4.0 >= pot` as `call >= pot * f` admits exactly the
/// records it did: no verdict moved when the constant replaced the literals.
pub const AUDIT_CALL_POT_FRACTION: f64 = 0.25;

/// Whether a recorded decision is worth a deep audit: a pot of at least `min_pot_bb`, or a call of
/// at least a quarter of the pot and of that floor (the spots a wrong answer is expensive in), or any
/// all-in at any pot size. [`audit_filter`] is this predicate in words, and the two share their
/// constants so a line that prints the filter cannot describe one the analyst does not apply (0355).
pub fn worth_auditing(rec: &ReplayRecord, min_pot_bb: f64) -> bool {
    let bb = rec.situation.bb.max(1) as f64;
    let pot = rec.situation.pot as f64 / bb;
    let call = rec.situation.call_amount as f64 / bb;
    pot >= min_pot_bb
        || (call >= pot.max(1.0) * AUDIT_CALL_POT_FRACTION && call >= min_pot_bb * AUDIT_CALL_POT_FRACTION)
        || rec.action.0 == "all_in"
}

/// [`worth_auditing`] in words, for the rows that have to name the filter they are drawn from (0355).
///
/// Rendered from the same constants the predicate tests — the floor it is handed and
/// [`AUDIT_CALL_POT_FRACTION`] — never hand-written. All three disjuncts are load-bearing: a label
/// reading "pot >= 50 bb" would drop the call-and-quarter rule that admits most of the audited
/// preflop raises, and the all-in rule that is the only reason the pooled all-in family is the one
/// class the deep re-solve samples completely (0345).
pub fn audit_filter(min_pot_bb: f64) -> String {
    format!(
        "pot >= {min_pot_bb} bb, or a call of >= {} bb that is at least {:.0}% of the pot, or any all-in",
        min_pot_bb * AUDIT_CALL_POT_FRACTION,
        AUDIT_CALL_POT_FRACTION * 100.0
    )
}

pub fn audit(rec: &ReplayRecord, deep: &Params, nn: Option<&Mlp>, bot: &str, hand_id: &str) -> sv10_bot_store::AuditResult {
    let started = std::time::Instant::now();
    let d = rerun(rec, deep, nn);
    let bb = rec.situation.bb.max(1) as f64;
    let best = d.candidates.iter().max_by(|a, b| a.ev.total_cmp(&b.ev));
    let (live_name, live_amount) = &rec.action;
    let live_ev = d
        .candidates
        .iter()
        .filter(|c| &c.action == live_name)
        .min_by_key(|c| (c.amount.unwrap_or(0) - live_amount.unwrap_or(0)).abs())
        .map(|c| c.ev);
    let gap = match (best, live_ev) {
        (Some(b), Some(l)) => ((b.ev - l) / bb).max(0.0),
        _ => 0.0,
    };
    sv10_bot_store::AuditResult {
        bot: bot.to_string(),
        hand_id: hand_id.to_string(),
        street: rec.situation.street.name().to_string(),
        live_action: action_label(live_name, *live_amount),
        deep_action: best.map_or_else(|| d.action_name.clone(), |b| action_label(&b.action, b.amount)),
        gap_bb: gap,
        pot_bb: rec.situation.pot as f64 / bb,
        deep_ms: started.elapsed().as_secs_f64() * 1000.0,
        samples: deep.samples,
        // The verdict's own provenance: a gap measured on a record without the live inputs (pre-v3)
        // is not comparable with one measured on a record that has them (0316).
        replay_version: Some(rec.version),
    }
}

/// Whether a re-run reproduced the recorded action, amount and every candidate EV exactly.
pub fn identical(rec: &ReplayRecord, d: &Decision) -> bool {
    rec.action == (d.action_name.clone(), d.amount) && rec.candidates == candidates(d)
}

#[cfg(test)]
mod tests {
    use super::*;
    use sv10_core::engine::Hand;
    use sv10_core::model::Counter;

    #[test]
    fn only_expensive_spots_are_worth_a_deep_audit() {
        let mut rng = SmallRng::seed_from_u64(3);
        let names: Vec<String> = ["a", "b", "c"].iter().map(|s| s.to_string()).collect();
        let params = Params { samples: 200, ..Default::default() };
        let models = ModelStore::default();
        let mut at = |raise: i64| {
            let mut hand = Hand::new(&[40_000, 40_000, 40_000], 0, 10, 20, &mut rng);
            hand.apply(Action::RaiseTo(raise)).unwrap();
            let sit = Situation::from_hand(&hand, hand.actor().unwrap(), &names);
            let d = decide_with(&sit, &models, &params, None, &mut SmallRng::seed_from_u64(1));
            record(&sit, 1, &params, &models, None, &d)
        };
        // A min-raise pot (about 4.5 bb) is skipped; facing a 60 bb raise is audited.
        assert!(!worth_auditing(&at(40), AUDIT_MIN_POT_BB));
        assert!(worth_auditing(&at(1_200), AUDIT_MIN_POT_BB));
        // A threshold of 0 audits everything, as before.
        assert!(worth_auditing(&at(40), 0.0));
    }

    /// 0355: the filter is rendered from the predicate's own constants, so a row that names it cannot
    /// describe a filter the analyst does not apply. Every disjunct is exercised at the boundary the
    /// rendered text names — the call floor it prints, and the all-in rule with no size at all.
    #[test]
    fn the_audit_filter_is_rendered_from_the_predicate_it_names() {
        let text = audit_filter(AUDIT_MIN_POT_BB);
        assert!(text.contains(&format!("pot >= {AUDIT_MIN_POT_BB} bb")), "{text}");
        assert!(text.contains(&format!("a call of >= {} bb", AUDIT_MIN_POT_BB * AUDIT_CALL_POT_FRACTION)), "{text}");
        assert!(text.contains(&format!("at least {:.0}% of the pot", AUDIT_CALL_POT_FRACTION * 100.0)), "{text}");
        assert!(text.ends_with("or any all-in"), "{text}");

        // The number the text prints as the call floor is the number the predicate tests: in a pot far
        // under the pot bar, a call of exactly that many big blinds is audited and one below it is not.
        let mut rng = SmallRng::seed_from_u64(3);
        let names: Vec<String> = ["a", "b", "c"].iter().map(|s| s.to_string()).collect();
        let params = Params { samples: 200, ..Default::default() };
        let models = ModelStore::default();
        let mut at = |raise: i64| {
            let mut hand = Hand::new(&[40_000, 40_000, 40_000], 0, 10, 20, &mut rng);
            hand.apply(Action::RaiseTo(raise)).unwrap();
            let sit = Situation::from_hand(&hand, hand.actor().unwrap(), &names);
            let d = decide_with(&sit, &models, &params, None, &mut SmallRng::seed_from_u64(1));
            record(&sit, 1, &params, &models, None, &d)
        };
        let probe = at(40);
        let bb = probe.situation.bb as f64;
        let posted = 40 - probe.situation.call_amount;
        let mut at_call = |call_bb: f64| at((call_bb * bb) as i64 + posted);
        let floor = AUDIT_MIN_POT_BB * AUDIT_CALL_POT_FRACTION;
        assert!(worth_auditing(&at_call(floor), AUDIT_MIN_POT_BB), "a call of exactly the printed floor is audited");
        assert!(!worth_auditing(&at_call(floor - 0.5), AUDIT_MIN_POT_BB), "half a big blind under it is not");

        // The third disjunct carries no size: the same min-raise spot the pot and call rules skip is
        // audited once the action is an all-in, which is why the all-in family is fully sampled (0345).
        assert!(!worth_auditing(&at(40), AUDIT_MIN_POT_BB));
        let mut all_in = at(40);
        all_in.action = ("all_in".into(), Some(40));
        assert!(worth_auditing(&all_in, AUDIT_MIN_POT_BB), "any all-in is worth a deep audit at any pot size");
    }

    #[test]
    fn a_recorded_decision_replays_bit_identically_through_json() {
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
        let params = Params { samples: 400, ..Default::default() };
        let d = decide_with(&sit, &models, &params, None, &mut SmallRng::seed_from_u64(99));
        let rec = record(&sit, 99, &params, &models, None, &d);
        assert_eq!(rec.version, REPLAY_VERSION);
        assert!(exact_inputs(&rec, &rec.params, None));
        let direct = rerun(&rec, &rec.params, None);
        assert!(
            identical(&rec, &direct),
            "without JSON: {:?} vs {:?} / {:?} vs {:?}",
            rec.action,
            (&direct.action_name, direct.amount),
            rec.candidates,
            candidates(&direct)
        );
        let json = serde_json::to_string(&rec).unwrap();
        let back: ReplayRecord = serde_json::from_str(&json).unwrap();
        let again = rerun(&back, &back.params, None);
        assert!(identical(&back, &again), "{:?} vs {:?}", back.action, (again.action_name, again.amount));
        // A different parameter set is a what-if, and is reported as such.
        let tighter = Params { call_margin: back.params.call_margin + 5.0, ..back.params.clone() };
        let what_if = rerun(&back, &tighter, None);
        assert!(what_if.candidates.len() == again.candidates.len());
        assert!(!exact_inputs(&back, &tighter, None));
        // The recorded strategy re-solved with the same budget and seed agrees with itself.
        let same = audit(&back, &back.params, None, "b", "h");
        assert_eq!((same.live_action.as_str(), same.gap_bb), (same.deep_action.as_str(), 0.0));
        assert_eq!((same.street.as_str(), same.samples, same.bot.as_str()), (sit.street.name(), 400, "b"));
        // A deep, parallel re-solve still produces a gap measured against its own best action.
        let deep = Params { samples: 20_000, deal_chunks: 4, ..back.params.clone() };
        let r = audit(&back, &deep, None, "b", "h");
        assert!(r.gap_bb >= 0.0 && r.gap_bb.is_finite());
        assert!(r.pot_bb > 0.0);

        // Version-zero JSON from before authoritative current_bet_to remains readable and exact in
        // ordinary states where the value is reconstructible.
        let mut legacy = serde_json::to_value(&rec).unwrap();
        legacy.as_object_mut().unwrap().remove("version");
        legacy["situation"].as_object_mut().unwrap().remove("current_bet_to");
        let legacy: ReplayRecord = serde_json::from_value(legacy).unwrap();
        assert_eq!(legacy.version, 0);
        let legacy_run = rerun(&legacy, &legacy.params, None);
        assert!(identical(&legacy, &legacy_run));
    }

    #[test]
    fn a_decision_against_corrected_opponents_replays_exactly_and_audits_with_the_corrections() {
        // 0316: the per-opponent corrections sit on ModelStore, and records dropped them, so a
        // corrected decision re-ran (and was audited) against an uncorrected model.
        let mut rng = SmallRng::seed_from_u64(41);
        let mut hand = Hand::new(&[2_000; 2], 0, 10, 20, &mut rng);
        hand.apply(Action::Call).unwrap();
        hand.apply(Action::Check).unwrap();
        let actor = hand.actor().unwrap();
        let names: Vec<String> = (0..2).map(|i| if i == actor { "hero".into() } else { "villain".into() }).collect();
        let sit = Situation::from_hand(&hand, actor, &names);
        let models = ModelStore {
            fold_offsets: std::sync::Arc::new([("villain".to_string(), -1.0f32)].into_iter().collect()),
            size_tells: std::sync::Arc::new([("villain".to_string(), 0.3f32)].into_iter().collect()),
            ..Default::default()
        };
        let params = Params { samples: 400, ..Default::default() };
        let d = decide_with(&sit, &models, &params, None, &mut SmallRng::seed_from_u64(7));
        let rec = record(&sit, 7, &params, &models, None, &d);
        assert_eq!(rec.corrections["villain"], Corrections { fold_offset: Some(-1.0), size_tell: Some(0.3), response_ratio: None });
        let back: ReplayRecord = serde_json::from_str(&serde_json::to_string(&rec).unwrap()).unwrap();
        assert!(identical(&back, &rerun(&back, &back.params, None)), "a corrected decision must replay exactly");
        let same = audit(&back, &back.params, None, "b", "h");
        assert_eq!(same.gap_bb, 0.0, "the audit sees the same model the live decision saw");
    }

    #[test]
    fn a_recorded_hero_image_replays_exactly_through_json() {
        let mut rng = SmallRng::seed_from_u64(41);
        let mut hand = Hand::new(&[2_000; 2], 0, 10, 20, &mut rng);
        hand.apply(Action::Call).unwrap();
        hand.apply(Action::Check).unwrap();
        let actor = hand.actor().unwrap();
        let names = (0..2).map(|i| if i == actor { "hero".into() } else { "villain".into() }).collect::<Vec<_>>();
        let sit = Situation::from_hand(&hand, actor, &names);
        let mut models = ModelStore::default();
        let image = PlayerStats {
            hands: 1000.0,
            vpip: Counter { opp: 1000.0, hit: 900.0 },
            pfr: Counter { opp: 1000.0, hit: 800.0 },
            ..Default::default()
        };
        for key in [format!("{}hero", sv10_core::model::HERO_SEEN_ONE), sv10_core::model::HERO_SEEN_ALL.into()] {
            models.hero_seen.clear();
            models.hero_seen.insert(key, image.clone());
            let params = Params { samples: 400, hero_image: 1.0, ..Default::default() };
            let d = decide_with(&sit, &models, &params, None, &mut SmallRng::seed_from_u64(7));
            let rec = record(&sit, 7, &params, &models, None, &d);
            let back: ReplayRecord = serde_json::from_str(&serde_json::to_string(&rec).unwrap()).unwrap();
            assert!(identical(&back, &rerun(&back, &back.params, None)), "hero image must survive replay capture");
            assert!(exact_inputs(&back, &back.params, None));
            let mut legacy = serde_json::to_value(&back).unwrap();
            legacy.as_object_mut().unwrap().remove("hero_seen");
            legacy["version"] = 3.into();
            let mut legacy: ReplayRecord = serde_json::from_value(legacy).unwrap();
            assert!(!exact_inputs(&legacy, &legacy.params, None), "unknown active image is not exact input");
            legacy.params.hero_image = 0.0;
            assert!(exact_inputs(&legacy, &legacy.params, None), "disabled image is not an input");
        }
    }

    #[test]
    fn response_network_digest_distinguishes_exact_replay_from_what_if() {
        let mut rng = SmallRng::seed_from_u64(17);
        let hand = Hand::new(&[2_000; 3], 0, 10, 20, &mut rng);
        let names = vec!["hero".into(), "a".into(), "b".into()];
        let sit = Situation::from_hand(&hand, hand.actor().unwrap(), &names);
        let params = Params { samples: 100, ..Default::default() };
        let first = Mlp::new(&[sv10_core::features::N_FEATURES, 3], 1);
        let second = Mlp::new(&[sv10_core::features::N_FEATURES, 3], 2);
        let models = ModelStore::default();
        let decision = decide_with(&sit, &models, &params, Some(&first), &mut SmallRng::seed_from_u64(23));
        let digest = net_digest(&first).unwrap().0;
        let rec = record(&sit, 23, &params, &models, Some(digest), &decision);
        assert!(exact_inputs(&rec, &params, Some(&first)));
        assert!(!exact_inputs(&rec, &params, Some(&second)));
        assert!(!exact_inputs(&rec, &params, None));
        let round_trip: ReplayRecord = serde_json::from_str(&serde_json::to_string(&rec).unwrap()).unwrap();
        assert!(identical(&round_trip, &rerun(&round_trip, &round_trip.params, Some(&first))));
    }
}
