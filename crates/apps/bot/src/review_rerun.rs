//! `review replay`: re-run recorded big decisions bit for bit (0257).

use crate::livefits::LiveFits;
use anyhow::Result;
use sv10_core::policy::Params;
use sv10_store::store::Store;

/// Where one re-solve's parameters come from, and — from the same value — the words the footer
/// prints for them (0359).
///
/// `--current` is *not* today's live parameters, and the old footer said it was. What it re-solves
/// with is the champion's strategy knobs (`params.v1`) on the recorded decision's own state: its
/// budget, deal decomposition and live-fitted set ([`Params::adopt_recorded_local`]) — so the only
/// thing it varies against the recorded action is the knob set it exists to test (0358). Live play
/// additionally has the range from `RANGE_PARAMS_KEY` and the fits of the moment installed, so a
/// what-if is not live play and must not read as one — LESSONS 39: an instrument names its
/// population and its basis wherever it prints. [`Source::label`] derives the words from the very
/// `Params` the re-solve is given, so the two cannot drift apart again.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Source {
    /// The recorded decision's own parameters: what the action was priced with.
    Recorded,
    /// `--current`, the store holding a champion: the champion's knobs on the record's own state.
    Champion,
    /// `--current`, the store holding no champion: the re-solve falls back to the record, and the
    /// label says so instead of naming a parameter set that was never loaded.
    NoChampion,
}

impl Source {
    /// The source a run is on: `--current` and whether the store holds a champion to replay with.
    fn of(current: bool, champion: bool) -> Source {
        match (current, champion) {
            (false, _) => Source::Recorded,
            (true, true) => Source::Champion,
            (true, false) => Source::NoChampion,
        }
    }

    /// The parameters one re-solve runs with. `recorded` is the decision's own set, `champion` the
    /// store's `params.v1` (`None` when it holds none).
    fn params(self, recorded: &Params, champion: Option<&Params>) -> Params {
        match (self, champion) {
            (Source::Champion, Some(p)) => {
                let mut params = p.clone();
                params.adopt_recorded_local(recorded);
                params
            }
            _ => recorded.clone(),
        }
    }

    /// The footer's words for `params`, which must be the [`Source::params`] this source just built:
    /// the base names the source and the budget, and the price basis is read off `params` rather
    /// than asserted ([`price_basis`]), so a `Params` that gains or loses one of the record's inputs
    /// is a test failure, not a silent mislabel (0359; LESSONS 39).
    ///
    /// The words depend on the source and the two parameter sets only — never on the rest of the
    /// record — so the footer can state them once, before any row is read.
    fn label(self, recorded: &Params, params: &Params) -> String {
        match self {
            Source::Recorded => "the recorded parameters".to_string(),
            Source::Champion => format!(
                "today's champion knobs (params.v1) on the record's own budget — {} — and nothing else",
                price_basis(recorded, params)
            ),
            Source::NoChampion => "the recorded parameters; the store holds no champion (params.v1) to replay with".to_string(),
        }
    }
}

/// The record's price basis in the footer's words, read off `params` rather than asserted: the parts
/// of [`Params::adopt_recorded_local`] the re-solve really carries, and — the case this exists for —
/// the part it does not, named (0359). Removing the adoption, or adding a source under it, moves
/// these words with the parameters instead of leaving the footer claiming an input nobody installed.
fn price_basis(recorded: &Params, params: &Params) -> &'static str {
    let prices = params.ev_bias == recorded.ev_bias && params.ev_bias_pot_cap == recorded.ev_bias_pot_cap && params.range == recorded.range;
    let fits = LiveFits::of(params) == LiveFits::of(recorded);
    match (prices, fits) {
        (true, true) => "at the record's prices: its self-calibration, range and live fits",
        (true, false) => "at the record's prices without its live fits: its self-calibration and range only",
        (false, true) => "at the record's live fits only, not its self-calibration and range",
        (false, false) => "at none of the record's prices: neither its self-calibration and range nor its live fits",
    }
}

/// One run's outcome: the text the CLI prints (the default), whether it was a `--current` what-if,
/// and the counts the exit code reads.
pub struct Report {
    /// The text rows and footer, or — `--json` (#723) — one JSON object.
    pub out: String,
    /// The run re-solved today's champion knobs rather than the recorded parameters.
    pub current: bool,
    /// Rows that replayed bit-identically.
    pub same: usize,
    /// Rows replayed.
    pub total: usize,
}

/// Build the report for `args`; `json` prints one object instead of the text rows.
pub fn report(store: &Store, args: &[String], json: bool) -> Result<Report> {
    use crate::replay::{ReplayRecord, exact_inputs, identical, rerun};
    use std::fmt::Write as _;
    let current = args.iter().any(|a| a == "--current");
    let id = args.iter().find_map(|a| a.strip_prefix("id=")).and_then(|v| v.parse().ok());
    let n = args.iter().find_map(|a| a.parse::<usize>().ok()).unwrap_or(10);
    let live: Option<Params> = store.get_kv(crate::PARAMS_KEY)?.and_then(|j| serde_json::from_str(&j).ok());
    let source = Source::of(current, live.is_some());
    // What the footer names: one `Params` built the same way as every row's, since the words do not
    // depend on the record (a test holds `label` to that).
    let what = source.label(&Params::default(), &source.params(&Params::default(), live.as_ref()));
    let (mut same, mut moved, mut total) = (0, 0, 0);
    let mut out = String::new();
    let mut rows: Vec<serde_json::Value> = Vec::new();
    for row in store.replays(n, id)? {
        let (rid, ts, bot, hand, record) = (row.id, crate::local_time(&row.ts), row.bot, row.hand_id, row.record);
        let rec: ReplayRecord = match serde_json::from_str(&record) {
            Ok(r) => r,
            Err(e) => {
                // stdout is exactly one JSON object in JSON mode, so the note goes to stderr; in text
                // mode it stays in the buffer, in position among the rows (it is a row that could not
                // be read).
                if json {
                    eprintln!("#{rid} unreadable: {e}");
                } else {
                    let _ = writeln!(out, "#{rid} unreadable: {e}");
                }
                continue;
            }
        };
        let nn = match &rec.net_digest {
            Some(d) => store.replay_net(d)?.and_then(|j| serde_json::from_str::<sv10_core::nn::Mlp>(&j).ok()),
            None => None,
        };
        let params = source.params(&rec.params, live.as_ref());
        let d = rerun(&rec, &params, nn.as_ref());
        let inputs_exact = exact_inputs(&rec, &params, nn.as_ref());
        total += 1;
        let exact = inputs_exact && identical(&rec, &d);
        if exact {
            same += 1;
        }
        let changed = rec.action != (d.action_name.clone(), d.amount);
        if changed {
            moved += 1;
        }
        let sit = &rec.situation;
        let status = if !inputs_exact {
            "what-if inputs"
        } else if exact {
            "identical"
        } else if changed {
            "CHANGED"
        } else {
            "same action, EVs differ"
        };
        if json {
            rows.push(serde_json::json!({
                "id": rid, "ts": ts, "bot": bot, "hand": hand, "street": sit.street.name(),
                "pot_bb": sit.pot / sit.bb.max(1), "call_bb": sit.call_amount / sit.bb.max(1),
                "recorded": {"action": rec.action.0, "amount": rec.action.1},
                "replay": {"action": d.action_name, "amount": d.amount},
                "identical": exact, "changed": changed, "inputs_exact": inputs_exact, "status": status,
            }));
            continue;
        }
        let _ = writeln!(
            out,
            "#{rid} {ts} {bot} hand {hand} {} pot {} bb, call {} bb: recorded {} {:?} -> replay {} {:?} {}",
            sit.street.name(),
            sit.pot / sit.bb.max(1),
            sit.call_amount / sit.bb.max(1),
            rec.action.0,
            rec.action.1,
            d.action_name,
            d.amount,
            status
        );
    }
    if json {
        // The footer's numbers and the rows above them, one object.
        out = serde_json::json!({
            "basis": what, "current": current, "total": total, "identical": same, "moved": moved, "rows": rows,
        })
        .to_string();
    } else {
        let _ = writeln!(out, "{total} replayed ({what}): {same} bit-identical, {moved} with a different action");
    }
    Ok(Report { out, current, same, total })
}

pub fn replay(store: &Store, args: &[String]) -> Result<()> {
    let json = args.iter().any(|a| a == "--json");
    let r = report(store, args, json)?;
    print!("{}", r.out);
    if !r.current && r.same < r.total {
        std::process::exit(1);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A recorded decision's parameters: every field of the local set ([`Params::adopt_recorded_local`])
    /// distinct from the champion's defaults, so one the re-solve forgets to adopt shows up.
    fn recorded() -> Params {
        Params {
            samples: 900,
            deal_chunks: 8,
            ev_bias: [("river:call".to_string(), 1.5)].into_iter().collect(),
            ev_bias_pot_cap: [("river:call".to_string(), 0.2)].into_iter().collect(),
            range: sv10_core::oprange::RangeParams { bluff_min: 0.11, ..Default::default() },
            fold_logit_shift: [0.2, 0.0, -0.7],
            preflop_fold_logit_shift: -1.4,
            river_jam_call_shift: 0.07,
            deep_call_shift: 0.15,
            overbet_call_shift: 0.3,
            overbet_call_slope: 0.06,
            ..Default::default()
        }
    }

    /// Today's champion as the store holds it: promoted knobs, and the local set at its defaults.
    fn champion() -> Params {
        Params { call_margin: 0.05, three_bet_ip: 3.2, ..Default::default() }
    }

    /// The `Params` fields two sets differ on, by their serialized names.
    fn differing(a: &Params, b: &Params) -> Vec<String> {
        let a = serde_json::to_value(a).unwrap();
        let b = serde_json::to_value(b).unwrap();
        let (a, b) = (a.as_object().unwrap(), b.as_object().unwrap());
        let mut keys: Vec<String> = a.keys().chain(b.keys()).filter(|k| a.get(*k) != b.get(*k)).cloned().collect();
        keys.sort();
        keys.dedup();
        keys
    }

    /// 0359: the footer said "current live parameters" over a mix that kept none of the six live fits
    /// live play had installed. Under 0358's convention the re-solve takes the record's local set and
    /// the label names the basis the parameters really have; this test holds the words against the
    /// `Params` the same source built, so the next edit that adds or drops an input fails here
    /// instead of shipping a footer that describes a parameter set nobody installed.
    #[test]
    fn every_source_states_the_basis_its_parameters_have() {
        let (rec, champ) = (recorded(), champion());

        // `--current`: the champion's knobs on the record's own budget and fitted set — those fields
        // and no others. This is the census the footer's words are held to.
        let current = Source::Champion.params(&rec, Some(&champ));
        assert_eq!(
            differing(&current, &champ),
            [
                "deal_chunks",
                "deep_call_shift",
                "ev_bias",
                "ev_bias_pot_cap",
                "fold_logit_shift",
                "overbet_call_shift",
                "overbet_call_slope",
                "preflop_fold_logit_shift",
                "range",
                "river_jam_call_shift",
                "samples",
            ],
            "the re-solve must take the record's own state and change nothing else"
        );
        assert_eq!(current.call_margin, champ.call_margin, "the knob set under test is the champion's");
        let what = Source::Champion.label(&rec, &current);
        assert_eq!(
            what,
            "today's champion knobs (params.v1) on the record's own budget — at the record's prices: its \
             self-calibration, range and live fits — and nothing else"
        );
        assert!(!what.contains("current live parameters"), "0359: the footer claimed a basis it did not have: {what}");

        // The price claim is measured, not written down. The old defect as a fixture — the record's
        // self-calibration and range with the champion's fits — makes the words say what is missing
        // rather than repeat the claim, and the live-fits-only and nothing-at-all states too.
        let no_fits = Params { fold_logit_shift: champ.fold_logit_shift, ..current.clone() };
        assert_eq!(price_basis(&rec, &no_fits), "at the record's prices without its live fits: its self-calibration and range only");
        let only_fits = Params { ev_bias: champ.ev_bias.clone(), range: champ.range, ..current.clone() };
        assert_eq!(price_basis(&rec, &only_fits), "at the record's live fits only, not its self-calibration and range");
        let neither = Params { fold_logit_shift: champ.fold_logit_shift, ev_bias: champ.ev_bias.clone(), ..current };
        assert!(price_basis(&rec, &neither).starts_with("at none of the record's prices"), "{}", price_basis(&rec, &neither));

        // The record's own parameters, and the `--current` fallback when the store holds no
        // champion: both are the recorded set, and neither is labelled as a champion's.
        for source in [Source::Recorded, Source::NoChampion] {
            let p = source.params(&rec, None);
            assert_eq!(serde_json::to_value(&p).unwrap(), serde_json::to_value(&rec).unwrap(), "{source:?} is not the record");
        }
        assert_eq!(Source::Recorded.label(&rec, &rec), "the recorded parameters");
        let fallback = Source::NoChampion.label(&rec, &rec);
        assert!(fallback.contains("no champion"), "the fallback must say why it is not current: {fallback}");
    }

    /// Which source a run picks, including the case 0359 found unlabelled: `--current` with nothing
    /// in `params.v1` re-solved the recorded parameters while the footer called them current.
    #[test]
    fn current_without_a_champion_is_not_a_champion_replay() {
        assert_eq!(Source::of(false, true), Source::Recorded);
        assert_eq!(Source::of(false, false), Source::Recorded);
        assert_eq!(Source::of(true, true), Source::Champion);
        assert_eq!(Source::of(true, false), Source::NoChampion);
    }

    /// The footer states the basis once, before any row is read, so the words must not depend on
    /// which record they are asked about. If a later change makes one row's parameters differ in
    /// kind from another's, this fails: move the footer into the loop and name the mix, rather than
    /// printing one row's basis over all of them.
    #[test]
    fn the_words_are_the_same_for_every_record_in_a_run() {
        let champ = champion();
        let other = Params { deal_chunks: 4, fold_logit_shift: [0.5, 0.5, 0.5], ..recorded() };
        for source in [Source::Recorded, Source::Champion, Source::NoChampion] {
            let live = matches!(source, Source::Champion).then_some(&champ);
            let one = recorded();
            let first = source.label(&one, &source.params(&one, live));
            let second = source.label(&other, &source.params(&other, live));
            assert_eq!(first, second, "{source:?} names the record it was handed");
        }
    }

    /// #723: `--json` is one parseable object over the same rows and footer numbers, the text path is
    /// what a script read before the flag existed, and an unreadable record's note keeps its place
    /// among the rows instead of jumping to the top (review of #723).
    #[test]
    fn the_replay_json_parses_and_the_text_path_is_unchanged() {
        let dir = std::env::temp_dir().join(format!("sv10-replay-json-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let store = Store::open(&dir.join("t.db")).unwrap();
        let text = report(&store, &[], false).unwrap();
        assert_eq!(text.out, "0 replayed (the recorded parameters): 0 bit-identical, 0 with a different action\n");
        assert!(!text.current && text.total == 0, "nothing replayed, and the exit code reads this");
        let json: serde_json::Value = serde_json::from_str(&report(&store, &["--json".into()], true).unwrap().out).unwrap();
        assert_eq!(json["basis"], "the recorded parameters");
        assert_eq!((json["total"].as_u64(), json["identical"].as_u64(), json["moved"].as_u64()), (Some(0), Some(0), Some(0)));
        assert_eq!(json["current"], false);
        assert_eq!(json["rows"].as_array().map(Vec::len), Some(0));

        // A seeded store: the record replays bit-identically, and an unreadable record older than it
        // (lower id, so read second) must leave its note after the first row, where the row it stands
        // for would have printed — not ahead of every row.
        store.insert_replay("A", "h1", "not json", None).unwrap();
        store.insert_replay("A", "h2", &serde_json::to_string(&small_replay()).unwrap(), None).unwrap();
        let text = report(&store, &[], false).unwrap();
        let row = text.out.find("#2 ").expect("the readable row");
        let note = text.out.find("#1 unreadable").expect("the note for the unreadable record");
        let foot = text.out.find("1 replayed").expect("the footer");
        assert!(row < note && note < foot, "the note stays in position among the rows: {}", text.out);
        assert!(text.total == 1 && text.same == 1, "one readable row, replayed exactly");

        let json: serde_json::Value = serde_json::from_str(&report(&store, &["--json".into()], true).unwrap().out).unwrap();
        assert_eq!((json["total"].as_u64(), json["identical"].as_u64(), json["moved"].as_u64()), (Some(1), Some(1), Some(0)));
        let rows = json["rows"].as_array().expect("the readable row's object");
        assert_eq!(rows.len(), 1, "only the readable record is a row");
        assert_eq!((rows[0]["id"].as_i64(), rows[0]["bot"].as_str(), rows[0]["hand"].as_str()), (Some(2), Some("A"), Some("h2")));
        assert_eq!(rows[0]["status"], "identical");
        assert_eq!(rows[0]["identical"], true);
        assert!(!json.to_string().contains("unreadable"), "the note is not part of the object: {json}");
        // Equal outputs cannot prove identity when an active input was never captured.
        let mut legacy = small_replay();
        legacy.params.hero_image = 0.25;
        legacy.hero_seen = None;
        legacy.version = 3;
        store.insert_replay("A", "legacy", &serde_json::to_string(&legacy).unwrap(), None).unwrap();
        let report = report(&store, &["1".into()], true).unwrap();
        let json: serde_json::Value = serde_json::from_str(&report.out).unwrap();
        assert_eq!((report.total, report.same), (1, 0));
        assert_eq!(json["rows"][0]["inputs_exact"], false);
        assert_eq!(json["rows"][0]["status"], "what-if inputs");
    }

    /// One replay record of a real decision, built the way the live client records one: `report`
    /// replays it bit-identically, so a seeded row has a known status.
    fn small_replay() -> crate::replay::ReplayRecord {
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
}
