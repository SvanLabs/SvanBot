//! `svanbot field`: a bot's own field, fitted from that bot's store in this process, kept beside
//! the store, and played as one Arena row recorded apart from the release rows.
//!
//! The field is a clone of the players the bot meets: one action model fitted on the opponent
//! sides of the hands in its own store, with no name, seat or hand in it
//! (`svanbot_learn::field`). It is not gated, because it does not decide the bot's play; it is a
//! sim opponent. The learner never opens a store, so this per-bot command (or the bot's own
//! process) is what builds it (`docs/adr/0004-reads-are-per-bot.md`;
//! `docs/superpowers/specs/2026-11-25-five-brains-design.md`, Phase 3 "Rows").
//!
//! `svanbot field [league] <bouts> [seed] [iterations] [out.json]` fits the field from
//! `SVANBOT_STORE`, keeps it whole as `<store>.field`, reads it back whole, and plays one row
//! against the reference policy. The row is named `field <bot>`, goes to the fleet's
//! `policy-runs.json`, and never to `arena.json`, which the ship bar reads.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use svanbot_learn::arena::Row;
use svanbot_learn::field::{Field, row_name};
use svanbot_learn::paired::verdict;
use svanbot_runtime::fleet::{Fleet, League};
use svanbot_runtime::history::observed_for_fit;
use svanbot_runtime::store::Store;
use svanbot_strategy::policy::Policy;

use crate::{arena, deciders, open_store, store_path};

/// Rounds of EM a field's fit runs.
const ROUNDS: usize = 25;

/// The bot's own name, as the fleet's env files write it: only the characters a fleet name uses,
/// so a store or field path can never leave the store directory.
fn bot_name() -> Result<String, String> {
    let me = std::env::var("SVANBOT_NAME").map_err(|_| "set SVANBOT_NAME".to_string())?;
    if me.is_empty() || me.len() > 32 || !me.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
    {
        return Err("SVANBOT_NAME is not a bot name".into());
    }
    Ok(me)
}

/// The Dirichlet prior on each move's count.
const ALPHA: f64 = 1.0;

/// Where a bot's field is kept: beside its store, under the bot's own name.
pub(crate) fn field_path(store: &Path) -> PathBuf {
    store.with_extension("field")
}

/// The field this bot's store makes: every opponent side of every stored hand, aggregated into
/// one identity-free model. `None` without an opponent-hand that could be fitted.
pub(crate) fn from_store(store: &Store) -> Result<Option<Field>, String> {
    let hands = store.hands_for_fit().map_err(|error| error.to_string())?;
    let observed = hands
        .iter()
        .enumerate()
        .flat_map(|(number, hand)| observed_for_fit(hand, number as u64).0);
    Ok(Field::fit(observed, ROUNDS, ALPHA))
}

/// Keeps `field` at `path` all or not at all: beside it, then renamed over it, so a reader sees
/// the old field whole or the new one whole.
pub(crate) fn keep_field(path: &Path, field: &Field) -> Result<(), String> {
    let partial = path.with_extension("field.partial");
    std::fs::write(&partial, field.to_text())
        .and_then(|()| std::fs::rename(&partial, path))
        .map_err(|error| format!("the field was not kept: {error}"))
}

/// The field kept at `path`, if one reads whole. A half-written or unreadable file is no field.
pub(crate) fn load_field(path: &Path) -> Option<Field> {
    Field::from_text(&std::fs::read_to_string(path).ok()?)
}

/// The run's champion line: the reference policy and the field it met. It names the bot's own
/// line in `policy-runs.json`, so two bots' field runs are two lines, not one replacing another.
fn run_name(policy: &Policy, me: &str) -> String {
    format!("{}@{}", policy.name(), row_name(me))
}

pub(crate) fn run(args: &[String]) -> ExitCode {
    let (league, args) = match args.first().and_then(|name| League::named(name)) {
        Some(league) => (league, &args[1..]),
        None => (League::HeadsUp, args),
    };
    let number = |at: usize| args.get(at).map(|text| text.parse::<u64>());
    let (Some(Ok(bouts @ 1..)), Ok(seed), Ok(fixed)) =
        (number(0), number(1).transpose(), number(2).transpose())
    else {
        eprintln!(
            "svanbot field: usage: svanbot field [3-handed|4-handed|5-handed|6-max] <bouts> [seed] [iterations] [out.json]"
        );
        return ExitCode::from(2);
    };
    let me = match bot_name() {
        Ok(me) => me,
        Err(why) => {
            eprintln!("svanbot field: {why}");
            return ExitCode::from(2);
        }
    };
    let store = match open_store(&me) {
        Ok(store) => store,
        Err(why) => {
            eprintln!("svanbot field: {why}");
            return ExitCode::from(2);
        }
    };
    let field = match from_store(&store) {
        Ok(Some(field)) => field,
        Ok(None) => {
            println!("svanbot field: the store has no opponent-hand to fit a field from");
            return ExitCode::SUCCESS;
        }
        Err(why) => {
            eprintln!("svanbot field: {why}");
            return ExitCode::from(1);
        }
    };
    let Ok(path) = store_path(&me).map(|store| field_path(&store)) else {
        eprintln!("svanbot field: set SVANBOT_STORE or HOME for the store");
        return ExitCode::from(2);
    };
    if let Err(why) = keep_field(&path, &field) {
        eprintln!("svanbot field: the field was not kept: {why}");
        return ExitCode::from(1);
    }
    // Read back whole, as a bot reads it: the row plays what the file holds, not the fit in hand.
    let Some(field) = load_field(&path) else {
        eprintln!("svanbot field: the kept field did not read back whole");
        return ExitCode::from(1);
    };
    println!(
        "svanbot field: field from {} opponent-hands, kept",
        field.sides()
    );
    let fleet = Fleet::in_dir(crate::fleet_dir());
    let champion = match fleet.champion() {
        Ok(champion) => champion,
        Err(why) => {
            eprintln!("svanbot field: the champion file is refused: {why}");
            return ExitCode::from(2);
        }
    };
    let policy = Policy::reference(champion);
    let game = deciders::game(seed.unwrap_or(1));
    let played: Vec<Vec<f64>> = (0..bouts)
        .map(|number| arena::play_field_bout(&game, league, &field, number, &policy, fixed))
        .collect();
    let sums: Vec<f64> = played.iter().map(|deals| deals.iter().sum()).collect();
    let results: Vec<f64> = played.into_iter().flatten().collect();
    let row = Row {
        opponent: row_name(&me),
        verdict: verdict(&results),
    };
    let run = arena::report(
        league,
        run_name(&policy, &me),
        game.seed,
        fixed,
        vec![(row, sums)],
    );
    if let Err(error) = fleet.record_policy_arena(&run) {
        eprintln!("svanbot field: the row was not recorded: {error}");
        return ExitCode::from(1);
    }
    if let Some(out) = args.get(3)
        && let Err(error) = std::fs::write(out, run.as_text() + "\n")
    {
        eprintln!("svanbot field: the output file was not written: {error}");
        return ExitCode::from(1);
    }
    ExitCode::SUCCESS
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use svanbot_runtime::fleet::{ArenaRow, ArenaRun};
    use svanbot_runtime::record::Hand;
    use svanbot_runtime::store::Source;
    use svanbot_strategy::params::Params;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("svanbot-field-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// One opponent's hand at a seat: raises preflop and the flop when `aggressive`, and folds
    /// preflop otherwise. The store names the seat, never the opponent.
    fn hand(id: &str, aggressive: bool) -> Hand {
        let actions = if aggressive {
            json!([
                {"seat": 1, "sequence_in_hand": 1, "street": "preflop", "action": "raise", "amount": 60},
                {"seat": 0, "sequence_in_hand": 2, "street": "preflop", "action": "call", "amount": 0},
                {"seat": 0, "sequence_in_hand": 3, "street": "flop", "action": "check", "amount": 0},
                {"seat": 1, "sequence_in_hand": 4, "street": "flop", "action": "raise", "amount": 60}
            ])
        } else {
            // Hero raises first (dealer posts the small blind heads up), so the opponent folds
            // to a bet rather than checking a free one.
            json!([
                {"seat": 0, "sequence_in_hand": 1, "street": "preflop", "action": "raise", "amount": 60},
                {"seat": 1, "sequence_in_hand": 2, "street": "preflop", "action": "fold", "amount": 0}
            ])
        };
        Hand {
            hand_id: id.into(),
            table_id: "t-1".into(),
            traced: Vec::new(),
            frames: vec![
                json!({
                    "hand_id": id, "seat": 0, "dealer_seat": 0,
                    "small_blind": 10, "big_blind": 20, "num_players": 2,
                    "hole_cards": ["3c", "3d"], "board": ["Ks", "9d", "4c"],
                    "shown_cards": {}, "pot_total": if aggressive { 120 } else { 80 },
                    "all_actions": actions
                })
                .to_string(),
            ],
        }
    }

    fn stored(dir: &Path, bot: &str, aggressive: bool, count: usize) -> (PathBuf, Store) {
        let path = dir.join(format!("{bot}.db"));
        let store = Store::open(&path, bot).unwrap();
        for at in 0..count {
            store
                .append_hand(
                    &hand(&format!("{bot}-{at}"), aggressive),
                    Source::History,
                    at as i64 * 1_000,
                )
                .unwrap();
        }
        (path, store)
    }

    #[test]
    fn a_field_is_fitted_from_the_bots_own_store_and_kept_whole_beside_it() {
        let dir = scratch("own-store");
        let (path, store) = stored(&dir, "Alpha", true, 40);
        let field = from_store(&store)
            .unwrap()
            .expect("forty opponent-hands make a field");
        assert_eq!(field.sides(), 40);

        let kept = field_path(&path);
        assert_eq!(kept, path.with_extension("field"));
        keep_field(&kept, &field).unwrap();
        assert_eq!(load_field(&kept), Some(field.clone()));
        assert!(
            !dir.join("Alpha.field.partial").exists(),
            "the temporary file was renamed over the target"
        );
        // The field was written with no bot, seat or opponent in it.
        let text = field.to_text();
        for identity in ["Alpha", "Beta", "seat", "t-1", "3c"] {
            assert!(
                !text.contains(identity),
                "{identity} crossed into the field"
            );
        }
    }

    #[test]
    fn one_bots_field_is_not_the_other_bots_and_its_store_is_refused_to_them() {
        let dir = scratch("not-visible");
        let (alpha_path, alpha) = stored(&dir, "Alpha", true, 40);
        let (beta_path, beta) = stored(&dir, "Beta", false, 40);
        let alpha_field = from_store(&alpha).unwrap().expect("Alpha's field");
        let beta_field = from_store(&beta).unwrap().expect("Beta's field");
        assert_ne!(alpha_field.model(), beta_field.model());

        // Beta cannot open Alpha's store, so it cannot fit from Alpha's hands.
        assert!(Store::open(&alpha_path, "Beta").is_err());

        // Nor does Beta read Alpha's kept field: each bot's path names its own store.
        assert_ne!(field_path(&alpha_path), field_path(&beta_path));
        keep_field(&field_path(&alpha_path), &alpha_field).unwrap();
        assert_eq!(load_field(&field_path(&beta_path)), None);
    }

    #[test]
    fn a_bots_field_row_is_recorded_apart_and_keeps_its_own_line() {
        let dir = scratch("recorded-apart");
        let fleet = Fleet::in_dir(dir.join("fleet"));
        let field_run = |bot: &str| ArenaRun {
            commit: "abc1234".into(),
            champion: Some(run_name(&Policy::reference(Params::DEFAULT), bot)),
            seed: 7,
            league: League::HeadsUp,
            iterations: Some(10),
            rows: vec![ArenaRow {
                opponent: row_name(bot),
                deals: 50,
                mbb_per_100: 100.0,
                ci95: 50.0,
                bouts: vec![0.5],
            }],
        };
        fleet.record_policy_arena(&field_run("Alpha")).unwrap();
        fleet.record_policy_arena(&field_run("Beta")).unwrap();

        assert!(
            fleet.arena_runs().unwrap().is_empty(),
            "the ship bar's arena holds no field row"
        );
        let runs = fleet.policy_arena_runs().unwrap();
        assert_eq!(runs.len(), 2, "each bot keeps its own field line");
        assert_eq!(runs[0].rows[0].opponent, row_name("Beta"));
        assert_eq!(runs[1].rows[0].opponent, row_name("Alpha"));
        assert!(svanbot_learn::play::apart(&runs[0].rows[0].opponent));
    }
}
