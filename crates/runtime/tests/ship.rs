//! The ship bar's Arena stage (V11-22): a release is let through only on runs played from the
//! tag's own commit, under the fleet's champion, that play again as recorded, that beat v10.0.2
//! heads up over a million hands, and where no opponent beats the policy.

use svanbot_runtime::fleet::{ArenaRow, ArenaRun, League};
use svanbot_runtime::ship::{self, V10_DEALS};

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
        champion: CHAMPION.into(),
        seed: 17,
        league,
        iterations: None,
        rows,
    }
}

/// A heads-up run and a 6-max run that clear every clause.
fn passing() -> Vec<ArenaRun> {
    vec![
        run(
            League::HeadsUp,
            vec![
                row("pricer", 20, 0.5, 300.0),
                row("v10.0.2", V10_DEALS / 50, 0.25, 100.0),
                row("nit", 20, 0.1, 300.0),
            ],
        ),
        run(
            League::SixMax,
            vec![
                row("pricer", 20, 0.5, 300.0),
                row("v10.0.2", 20, -2.0, 9_000.0),
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
    runs[0].champion = "00000000000000bb".into();
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
        ["the heads-up pricer row has 19 bouts for 1000 deals"]
    );
}

#[test]
fn v10_must_be_beaten_heads_up_over_a_million_hands_with_the_interval_above_zero() {
    let mut runs = passing();
    runs[0].rows[1] = row("v10.0.2", V10_DEALS / 50 - 1, 0.25, 100.0);
    assert_eq!(
        failures(&runs),
        ["v10.0.2 heads up: 499950 deals, short of 500000"]
    );
    let mut runs = passing();
    runs[0].rows[1].ci95 = 600.0;
    assert_eq!(
        failures(&runs),
        ["v10.0.2 heads up: +500 mbb/100 ±600, the interval is not above zero"]
    );
    let mut runs = passing();
    runs[0].rows.remove(1);
    assert_eq!(failures(&runs), ["the heads-up run has no v10.0.2 row"]);
}

#[test]
fn an_opponent_beating_the_policy_in_either_league_is_a_regression() {
    let mut runs = passing();
    runs[1].rows[0] = row("pricer", 20, -0.5, 300.0);
    assert_eq!(
        failures(&runs),
        ["regression: the pricer beats the policy at 6-max, -1000 mbb/100 ±300"]
    );
    // The 6-max v10.0.2 row is shown, never required: losing it is not a regression.
    let mut runs = passing();
    runs[1].rows[1] = row("v10.0.2", 20, -2.0, 100.0);
    assert_eq!(failures(&runs), Vec::<String>::new());
}

#[test]
fn the_newest_run_of_a_league_from_the_commit_is_the_one_read() {
    let mut runs = passing();
    let mut older = runs[1].clone();
    older.rows[0] = row("pricer", 20, -0.5, 300.0);
    runs.push(older);
    assert_eq!(failures(&runs), Vec::<String>::new());
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
