//! The ship bar's Arena stage (V11-22, V11-40): a release is let through only on runs played from
//! the tag's own commit, under the fleet's champion, that play again as recorded, where the policy
//! is read above v10.0.2 in both leagues and below no opponent.

use svanbot_runtime::fleet::{ArenaRow, ArenaRun, League};
use svanbot_runtime::ship;

const COMMIT: &str = "4c06087aa11b22c33d44e55f66a77b88c99d00e1";
const CHAMPION: &str = "00000000000000aa";

/// A row whose numbers are its bouts': `bouts` bouts that each won `per_bout`, with `ci95` around.
fn row(opponent: &str, bouts: usize, per_bout: f64, ci95: f64) -> ArenaRow {
    let deals = bouts * 50;
    ArenaRow {
        opponent: opponent.into(),
        deals,
        mbb_per_100: per_bout * bouts as f64 / deals as f64 * 100_000.0,
        ci95,
        bouts: vec![per_bout; bouts],
    }
}

fn run(league: League, rows: Vec<ArenaRow>) -> ArenaRun {
    ArenaRun {
        commit: COMMIT.into(),
        champion: Some(CHAMPION.into()),
        seed: 17,
        league,
        iterations: None,
        rows,
    }
}

/// A row of `bouts` bouts that won about `per_bout` each, alternately `wobble` above and below,
/// with its win rate their total.
fn wobbling(opponent: &str, bouts: usize, per_bout: f64, wobble: f64) -> ArenaRow {
    let mut row = row(opponent, bouts, per_bout, 0.0);
    for (k, bout) in row.bouts.iter_mut().enumerate() {
        *bout += if k % 2 == 0 { wobble } else { -wobble };
    }
    row.mbb_per_100 = row.bouts.iter().sum::<f64>() / row.deals as f64 * 100_000.0;
    row
}

/// A heads-up run and a 6-max run that clear every clause: v10.0.2 read above zero in both, and
/// the nit, though not told from zero, is no regression.
fn passing() -> Vec<ArenaRun> {
    vec![
        run(
            League::HeadsUp,
            vec![
                wobbling("pricer", 40, 20.0, 5.0),
                wobbling("v10.0.2", 40, 10.0, 5.0),
                wobbling("nit", 40, 1.0, 50.0),
            ],
        ),
        run(
            League::SixMax,
            vec![
                wobbling("pricer", 40, 20.0, 5.0),
                wobbling("v10.0.2", 40, 10.0, 5.0),
            ],
        ),
    ]
}

/// A replay that plays every bout exactly as it was recorded.
fn faithful(_: &ArenaRun, row: &ArenaRow, bout: u64) -> f64 {
    row.bouts[bout as usize]
}

fn failures(runs: &[ArenaRun]) -> Vec<String> {
    ship::arena(runs, COMMIT, CHAMPION, &mut faithful)
        .err()
        .unwrap_or_default()
}

#[test]
fn runs_from_the_tags_commit_that_play_again_and_clear_the_bar_let_the_release_through() {
    assert_eq!(
        ship::arena(&passing(), COMMIT, CHAMPION, &mut faithful),
        Ok(())
    );
}

#[test]
fn a_league_with_no_run_from_the_tags_commit_is_refused() {
    let mut runs = passing();
    runs[1].commit = "local".into();
    assert_eq!(failures(&runs), ["no 6-max run from 4c06087aa11b"]);
}

#[test]
fn a_run_under_another_champion_is_refused() {
    let mut runs = passing();
    runs[0].champion = Some("00000000000000bb".into());
    assert_eq!(
        failures(&runs),
        ["the heads-up run played champion 00000000000000bb, not the fleet's 00000000000000aa"]
    );
}

#[test]
fn a_run_whose_bouts_do_not_play_again_as_recorded_is_refused() {
    let mut played = 0;
    let mut lying = |_: &ArenaRun, row: &ArenaRow, bout: u64| {
        played += 1;
        row.bouts[bout as usize] + 0.001
    };
    let refused = ship::arena(&passing(), COMMIT, CHAMPION, &mut lying).unwrap_err();
    assert_eq!(played, 2 * ship::REPLAYS);
    assert!(
        refused
            .iter()
            .all(|why| why.contains("did not play again as recorded")),
        "{refused:?}"
    );
    assert_eq!(refused.len(), 2 * ship::REPLAYS);
}

#[test]
fn a_row_whose_numbers_are_not_its_bouts_is_refused() {
    let mut runs = passing();
    runs[0].rows[2].mbb_per_100 += 1.0;
    assert_eq!(
        failures(&runs),
        ["the heads-up nit row's win rate is not its bouts' total"]
    );
    let mut runs = passing();
    runs[0].rows[0].bouts.pop();
    assert_eq!(
        failures(&runs),
        ["the heads-up pricer row has 39 bouts for 2000 deals"]
    );
}

/// The one refusal `runs` gets, which must start with `prefix`.
fn refused_with(runs: &[ArenaRun], prefix: &str) {
    let refused = failures(runs);
    assert!(
        refused.len() == 1 && refused[0].starts_with(prefix),
        "{refused:?}"
    );
}

#[test]
fn v10_must_be_read_above_zero_in_both_leagues() {
    for league in [0, 1] {
        let name = ["heads-up", "6-max"][league];
        let mut runs = passing();
        runs[league].rows[1] = wobbling("v10.0.2", 40, 1.0, 50.0);
        refused_with(
            &runs,
            &format!("v10.0.2 in the {name} league is not yet told from zero: +2000 mbb/100 ±"),
        );
        let mut runs = passing();
        runs[league].rows[1] = wobbling("v10.0.2", 40, -10.0, 5.0);
        refused_with(
            &runs,
            &format!("v10.0.2 beats the policy in the {name} league: -20000 mbb/100 ±"),
        );
        let mut runs = passing();
        runs[league].rows.remove(1);
        assert_eq!(
            failures(&runs),
            [format!("the {name} run has no v10.0.2 row")]
        );
    }
}

#[test]
fn a_v10_row_short_of_its_bouts_is_not_yet_read() {
    let mut runs = passing();
    runs[1].rows[1] = wobbling("v10.0.2", ship::MIN_BOUTS - 1, 10.0, 5.0);
    assert_eq!(
        failures(&runs),
        ["v10.0.2 in the 6-max league is not yet read: 29 of the 30 bouts a row needs"]
    );
}

#[test]
fn an_opponent_read_below_zero_in_either_league_is_a_regression() {
    let mut runs = passing();
    runs[1].rows[0] = wobbling("pricer", 40, -20.0, 5.0);
    refused_with(
        &runs,
        "regression: the pricer beats the policy at 6-max, -40000 mbb/100 ±",
    );
}

#[test]
fn the_newest_run_of_a_league_from_the_commit_is_the_one_read() {
    let mut runs = passing();
    let mut older = runs[1].clone();
    older.rows[0] = wobbling("pricer", 40, -20.0, 5.0);
    runs.push(older);
    assert_eq!(failures(&runs), Vec::<String>::new());
}

#[test]
fn a_row_is_read_above_or_below_zero_only_when_its_sequence_leaves_zero_out() {
    // +0.4 big blinds a hand, give or take 0.1 a bout: far from zero at any look.
    assert_eq!(
        ship::reading(&wobbling("nit", 40, 20.0, 5.0)),
        ship::Reading::Above
    );
    assert_eq!(
        ship::reading(&wobbling("nit", 40, -20.0, 5.0)),
        ship::Reading::Below
    );
    // A mean of zero, and a mean well inside its wobble, cannot be told from zero.
    assert_eq!(
        ship::reading(&wobbling("nit", 40, 0.0, 10.0)),
        ship::Reading::Undecided
    );
    assert_eq!(
        ship::reading(&wobbling("nit", 40, 1.0, 50.0)),
        ship::Reading::Undecided
    );
}

#[test]
fn a_row_says_nothing_before_its_spread_is_known() {
    let short = wobbling("nit", ship::MIN_BOUTS - 1, 20.0, 5.0);
    assert_eq!(ship::reading(&short), ship::Reading::Undecided);
    assert_eq!(
        ship::reading(&wobbling("nit", ship::MIN_BOUTS, 20.0, 5.0)),
        ship::Reading::Above
    );
}

#[test]
fn a_release_is_named_by_a_tag_or_a_slot_of_three_numbers() {
    assert_eq!(ship::version("v11.0.0"), Some("11.0.0"));
    assert_eq!(ship::version("11.0.12"), Some("11.0.12"));
    for refused in [
        "",
        "local",
        "v11.0",
        "v11.0.0-rc1",
        "v11..0",
        "vv11.0.0",
        "11.0.0.1",
    ] {
        assert_eq!(ship::version(refused), None, "{refused}");
    }
}

#[test]
fn a_legacy_run_without_a_champion_is_not_release_evidence() {
    let mut runs = passing();
    runs[0].rows.retain(|row| row.opponent != "v10.0.2");
    let mut legacy = runs[0].as_value();
    legacy.as_object_mut().unwrap().remove("champion");
    runs[0] = ArenaRun::from_text(&legacy.to_string()).unwrap();
    assert_eq!(
        failures(&runs),
        [
            "the heads-up run has no recorded champion",
            "the heads-up run has no v10.0.2 row",
        ]
    );
}
