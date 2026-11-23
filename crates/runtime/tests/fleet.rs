//! The fleet's files: the champion every bot plays by, the proposal the canary takes up, and the
//! Arena's recorded runs.
// A test fleet is a fresh temporary directory.
#![allow(clippy::disallowed_methods, clippy::disallowed_types)]

use std::path::PathBuf;

use svanbot_learn::probe::Kind;
use svanbot_runtime::fleet::{ARENA_HISTORY, ArenaRow, ArenaRun, Fleet, League, Proposal};
use svanbot_strategy::params::Params;

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("svanbot-fleet-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

/// Numbers with no short decimal form: they must come back to the bit.
const CHALLENGER: Params = Params { exploit: 0.1 + 0.2 };

fn proposal() -> Proposal {
    Proposal {
        challenger: CHALLENGER,
        kind: Kind::Guard { hands: 3_000 },
        harm_margin: 20.0,
        sim_estimate: Some(1.0 / 7.0),
        max_orbits: 3_334,
    }
}

#[test]
fn the_champion_is_the_default_until_one_is_promoted_and_comes_back_to_the_bit() {
    let fleet = Fleet::in_dir(scratch("champion"));
    assert_eq!(fleet.champion(), Ok(Params::DEFAULT));
    fleet.promote(&CHALLENGER).unwrap();
    assert_eq!(fleet.champion(), Ok(CHALLENGER));
    assert_eq!(fleet.champion().unwrap().digest(), CHALLENGER.digest());
}

#[test]
fn a_champion_file_whose_numbers_do_not_hash_to_its_digest_is_refused() {
    let dir = scratch("tampered");
    let fleet = Fleet::in_dir(&dir);
    fleet.promote(&CHALLENGER).unwrap();
    let path = dir.join("champion.json");
    let text = std::fs::read_to_string(&path).unwrap();
    std::fs::write(&path, text.replace("0.30000000000000004", "0.3")).unwrap();
    assert!(fleet.champion().is_err());
}

#[test]
fn a_proposal_waits_until_it_is_taken_and_is_taken_once() {
    let fleet = Fleet::in_dir(scratch("proposal"));
    assert_eq!(fleet.take_proposal(), Ok(None));
    fleet.propose(&proposal()).unwrap();
    assert_eq!(fleet.take_proposal(), Ok(Some(proposal())));
    assert_eq!(fleet.take_proposal(), Ok(None));
}

#[test]
fn a_proposal_reads_back_from_its_own_text_and_names_its_hypothesis() {
    let text = proposal().to_json();
    assert_eq!(Proposal::from_json(&text), Ok(proposal()));
    let probe = Proposal {
        kind: Kind::Probe,
        sim_estimate: None,
        ..proposal()
    };
    assert_eq!(Proposal::from_json(&probe.to_json()), Ok(probe));
    let hypothesis = proposal().hypothesis(&Params::DEFAULT);
    assert_eq!(hypothesis.champion, Params::DEFAULT.digest());
    assert_eq!(hypothesis.challenger, CHALLENGER.digest());
    assert_eq!(hypothesis.kind, Kind::Guard { hands: 3_000 });
    assert!(Proposal::from_json("{}").is_err());
}

fn run(commit: &str, seed: u64, mbb: f64) -> ArenaRun {
    ArenaRun {
        commit: commit.into(),
        champion: "00000000000000aa".into(),
        seed,
        league: League::HeadsUp,
        iterations: None,
        rows: vec![ArenaRow {
            opponent: "nit".into(),
            deals: 400,
            mbb_per_100: mbb,
            ci95: 250.0,
            bouts: Vec::new(),
        }],
    }
}

#[test]
fn the_arena_keeps_the_latest_run_and_a_short_history_of_the_ones_before() {
    let fleet = Fleet::in_dir(scratch("arena"));
    assert_eq!(fleet.arena_runs(), Ok(Vec::new()));
    for number in 0..(ARENA_HISTORY as u64 + 3) {
        fleet
            .record_arena(&run(&format!("c{number}"), number, 1.0))
            .unwrap();
    }
    let recorded = fleet.arena_runs().unwrap();
    assert_eq!(recorded.len(), ARENA_HISTORY);
    // Newest first, so the dashboard reads the latest without searching, and the oldest fall off.
    assert_eq!(recorded[0].commit, format!("c{}", ARENA_HISTORY as u64 + 2));
    assert_eq!(recorded[ARENA_HISTORY - 1].commit, "c3");
}

#[test]
fn a_run_reads_back_from_its_own_file_unchanged_enough_to_report() {
    let dir = scratch("arena-roundtrip");
    let fleet = Fleet::in_dir(&dir);
    let run = run("abc1234", 7, 12_345.6 + 0.1 + 0.2);
    fleet.record_arena(&run).unwrap();
    // The last bit of a float is the writer's, not the run's: the run is played again from its
    // commit and seed, so a scoreboard line has to read back as what it printed, not bit for bit.
    let [read] = fleet.arena_runs().unwrap().try_into().unwrap();
    assert_eq!(
        (read.commit, read.champion, read.seed),
        (run.commit.clone(), run.champion.clone(), run.seed)
    );
    assert_eq!(read.rows.len(), 1);
    assert_eq!(read.rows[0].opponent, "nit");
    assert_eq!(read.rows[0].deals, 400);
    assert!((read.rows[0].mbb_per_100 - run.rows[0].mbb_per_100).abs() < 1e-6);
    assert_eq!(read.rows[0].ci95, 250.0);
    // The same run recorded twice is the same run: re-running a commit that did not change the
    // policy replaces its line rather than growing the history.
    fleet.record_arena(&run).unwrap();
    assert_eq!(fleet.arena_runs().unwrap().len(), 1);
}

#[test]
fn a_local_run_is_not_the_same_run_as_a_local_run_that_measured_otherwise() {
    // A local run has no commit to be played again from, so two of them under one champion and
    // seed are not the same run unless every number agrees: otherwise a run of changed code
    // would replace the run before it, and the scoreboard would show one line where there were two.
    let fleet = Fleet::in_dir(scratch("arena-local"));
    fleet.record_arena(&run("local", 7, 1.0)).unwrap();
    fleet.record_arena(&run("local", 7, 2.0)).unwrap();
    assert_eq!(fleet.arena_runs().unwrap().len(), 2);
    // The same local run recorded twice is still one line.
    fleet.record_arena(&run("local", 7, 2.0)).unwrap();
    assert_eq!(fleet.arena_runs().unwrap().len(), 2);
}

#[test]
fn a_run_written_as_text_is_the_run_read_back() {
    // There is one format for a run: `svanbot arena` writes its `--out` file from this text and
    // records the same run here, so the file an operator keeps and the scoreboard cannot disagree
    // on what a run is. The field order is the writer's; the run is the fields and their values.
    let run = run("abc1234", 7, 12_345.6);
    let value: serde_json::Value = serde_json::from_str(&run.as_text()).unwrap();
    assert_eq!(value["commit"], "abc1234");
    assert_eq!(value["champion"], "00000000000000aa");
    assert_eq!(value["seed"], 7);
    assert_eq!(value["rows"][0]["opponent"], "nit");
    assert_eq!(value["rows"][0]["deals"], 400);
    assert_eq!(value["rows"][0]["mbb_per_100"], 12_345.6);
    assert_eq!(value["rows"][0]["ci95"], 250.0);
}

#[test]
fn a_promotion_under_the_same_commit_is_a_run_of_its_own() {
    // The commit says what the code was; the champion says what it played. A promotion between
    // two runs of one commit moves the scoreboard, so the earlier run is not the later one.
    let fleet = Fleet::in_dir(scratch("arena-promoted"));
    let before = run("abc1234", 7, 1.0);
    fleet.record_arena(&before).unwrap();
    let after = ArenaRun {
        champion: "00000000000000bb".into(),
        ..before
    };
    fleet.record_arena(&after).unwrap();
    let runs = fleet.arena_runs().unwrap();
    assert_eq!(runs.len(), 2);
    assert_eq!(runs[0].champion, "00000000000000bb");
}

#[test]
fn a_run_file_that_does_not_parse_is_refused_rather_than_misread() {
    let dir = scratch("arena-broken");
    let fleet = Fleet::in_dir(&dir);
    fleet.record_arena(&run("bad", 1, 1.0)).unwrap();
    std::fs::write(dir.join("arena.json"), "{\"runs\":[{\"commit\":\"x\"}]}").unwrap();
    assert!(fleet.arena_runs().is_err());
}

#[test]
fn a_run_of_numbers_with_no_short_decimal_form_reads_back_to_the_bit() {
    // Real scoreboard numbers come from division, so they almost never have a short decimal
    // form. The file and the scoreboard must not disagree on what a run is, so a number written
    // and read back has to be the same number, bit for bit — otherwise a second recording of the
    // same local run measures differently and becomes a second line.
    let fleet = Fleet::in_dir(scratch("arena-bit-exact"));
    // A real scoreboard line from `svanbot arena 1 5 1`: thirty-one deals split one way, and no
    // short decimal form. serde_json's default decoder rounds it one bit the wrong way.
    let mbb = -984_173.515_151_514_9;
    let run = ArenaRun {
        rows: vec![ArenaRow {
            ..run("local", 7, mbb).rows.into_iter().next().unwrap()
        }],
        ..run("local", 7, mbb)
    };
    fleet.record_arena(&run).unwrap();
    let [read] = fleet.arena_runs().unwrap().try_into().unwrap();
    assert_eq!(read.rows[0].mbb_per_100.to_bits(), mbb.to_bits());
    // The same run recorded twice is still one line.
    fleet.record_arena(&run).unwrap();
    assert_eq!(fleet.arena_runs().unwrap().len(), 1);
}

#[test]
fn recording_a_run_leaves_the_numbers_before_it_alone() {
    // A later run is a later line; it must not rewrite what an earlier line says.
    let fleet = Fleet::in_dir(scratch("arena-stable"));
    let first = run("abc1234", 7, -984_173.515_151_514_9);
    fleet.record_arena(&first).unwrap();
    fleet
        .record_arena(&run("def5678", 7, 2.0 / 7.0 * 100_000.0))
        .unwrap();
    let runs = fleet.arena_runs().unwrap();
    assert_eq!(runs.len(), 2);
    assert_eq!(
        runs[1].rows[0].mbb_per_100.to_bits(),
        first.rows[0].mbb_per_100.to_bits()
    );
}

#[test]
fn a_six_max_run_is_never_the_heads_up_run_of_the_same_name() {
    // The two leagues measure different things, so the same commit, champion and seed in each is
    // two runs, and each reads back as the league it was played in.
    let fleet = Fleet::in_dir(scratch("arena-league"));
    fleet.record_arena(&run("abc1234", 7, 1.0)).unwrap();
    let six = ArenaRun {
        league: League::SixMax,
        iterations: None,
        ..run("abc1234", 7, 1.0)
    };
    fleet.record_arena(&six).unwrap();
    let leagues: Vec<League> = fleet
        .arena_runs()
        .unwrap()
        .iter()
        .map(|r| r.league)
        .collect();
    assert_eq!(leagues, [League::SixMax, League::HeadsUp]);
}

#[test]
fn a_run_stored_before_six_max_reads_as_heads_up() {
    let dir = scratch("arena-old");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("arena.json"),
        r#"{"runs":[{"commit":"abc1234","champion":"00000000000000aa","seed":7,"rows":[]}]}"#,
    )
    .unwrap();
    let [read] = Fleet::in_dir(&dir)
        .arena_runs()
        .unwrap()
        .try_into()
        .unwrap();
    assert_eq!(read.league, League::HeadsUp);
}

#[test]
fn each_league_keeps_its_own_history() {
    // A long 6-max session on the box must not push the heads-up league off the scoreboard.
    let fleet = Fleet::in_dir(scratch("arena-histories"));
    fleet.record_arena(&run("heads", 1, 1.0)).unwrap();
    for number in 0..(ARENA_HISTORY as u64 + 2) {
        let six = ArenaRun {
            league: League::SixMax,
            iterations: None,
            ..run(&format!("six{number}"), number, 1.0)
        };
        fleet.record_arena(&six).unwrap();
    }
    let runs = fleet.arena_runs().unwrap();
    let count = |league| runs.iter().filter(|r| r.league == league).count();
    assert_eq!(count(League::SixMax), ARENA_HISTORY);
    assert_eq!(count(League::HeadsUp), 1);
}

#[test]
fn a_run_keeps_each_bouts_result_and_how_it_was_solved_so_the_gate_can_play_one_again() {
    // The ship bar trusts a run only when a bout it draws plays out as recorded (V11-22),
    // so each row keeps what every bout won, and the run keeps the iterations it re-solved with.
    let fleet = Fleet::in_dir(scratch("arena-bouts"));
    let mut played = run("abc123", 9, 12.0);
    played.iterations = Some(40);
    played.rows[0].bouts = vec![1.5, -0.25, -984_173.515_151_514_9];
    fleet.record_arena(&played).unwrap();
    let [read] = fleet.arena_runs().unwrap().try_into().unwrap();
    assert_eq!(read, played);
    // A run recorded before bouts were kept reads back with none, and budget-sized iterations.
    let old = scratch("arena-old");
    std::fs::create_dir_all(&old).unwrap();
    std::fs::write(
        old.join("arena.json"),
        r#"{"runs":[{"commit":"c","champion":"d","seed":1,"rows":[{"opponent":"nit","deals":50,"mbb_per_100":1.0,"ci95":2.0}]}]}"#,
    )
    .unwrap();
    let [read] = Fleet::in_dir(old).arena_runs().unwrap().try_into().unwrap();
    assert_eq!((read.iterations, read.rows[0].bouts.len()), (None, 0));
}
