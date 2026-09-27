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

pub fn replay(store: &Store, args: &[String]) -> Result<()> {
    use crate::replay::{ReplayRecord, exact_inputs, identical, rerun};
    let current = args.iter().any(|a| a == "--current");
    let id = args.iter().find_map(|a| a.strip_prefix("id=")).and_then(|v| v.parse().ok());
    let n = args.iter().find_map(|a| a.parse::<usize>().ok()).unwrap_or(10);
    let live: Option<Params> = store.get_kv(crate::PARAMS_KEY)?.and_then(|j| serde_json::from_str(&j).ok());
    let source = Source::of(current, live.is_some());
    // What the footer names: one `Params` built the same way as every row's, since the words do not
    // depend on the record (a test holds `label` to that).
    let what = source.label(&Params::default(), &source.params(&Params::default(), live.as_ref()));
    let (mut same, mut moved, mut total) = (0, 0, 0);
    for row in store.replays(n, id)? {
        let (rid, ts, bot, hand, json) = (row.id, crate::local_time(&row.ts), row.bot, row.hand_id, row.record);
        let rec: ReplayRecord = match serde_json::from_str(&json) {
            Ok(r) => r,
            Err(e) => {
                println!("#{rid} unreadable: {e}");
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
        let exact = identical(&rec, &d);
        if exact {
            same += 1;
        }
        let changed = rec.action != (d.action_name.clone(), d.amount);
        if changed {
            moved += 1;
        }
        let sit = &rec.situation;
        println!(
            "#{rid} {ts} {bot} hand {hand} {} pot {} bb, call {} bb: recorded {} {:?} -> replay {} {:?} {}",
            sit.street.name(),
            sit.pot / sit.bb.max(1),
            sit.call_amount / sit.bb.max(1),
            rec.action.0,
            rec.action.1,
            d.action_name,
            d.amount,
            if !inputs_exact {
                "what-if inputs"
            } else if exact {
                "identical"
            } else if changed {
                "CHANGED"
            } else {
                "same action, EVs differ"
            }
        );
    }
    println!("{total} replayed ({what}): {same} bit-identical, {moved} with a different action");
    if !current && same < total {
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
}
