//! The ship run (V11-40): both leagues are played a heat at a time, every row one more
//! bout a heat, until the bar is decided or the cap runs out. Each heat is recorded, so a run
//! cut short keeps what it played.

use svanbot_runtime::fleet::{ArenaRow, ArenaRun, League};
use svanbot_runtime::ship::{self, Bar};

/// A league's run with no bouts played yet, against v10.0.2, the pricer and the nit.
fn fresh(league: League) -> ArenaRun {
    ArenaRun {
        commit: "4c06087aa11b22c33d44e55f66a77b88c99d00e1".into(),
        champion: Some("00000000000000aa".into()),
        seed: 17,
        league,
        iterations: None,
        rows: ["v10.0.2", "pricer", "nit"]
            .map(|opponent| ArenaRow {
                opponent: opponent.into(),
                deals: 0,
                mbb_per_100: 0.0,
                ci95: 0.0,
                bouts: Vec::new(),
            })
            .into(),
    }
}

/// What a played bout won: `per_bout` for the named opponent, alternately 5 above and below.
fn scripted(
    per_bout: impl Fn(&str) -> (f64, f64),
) -> impl FnMut(&[ArenaRun], &[ship::Bout]) -> Vec<f64> {
    move |runs, bouts| {
        bouts
            .iter()
            .map(|bout| {
                let row = &runs[bout.run].rows[bout.row];
                let (mean, wobble) = per_bout(&row.opponent);
                mean + if bout.number % 2 == 0 {
                    wobble
                } else {
                    -wobble
                }
            })
            .collect()
    }
}

/// Runs both leagues under `play` until it stops, with the clock running out after `heats`
/// heats: where the bar stands, the runs as last recorded, and how many times each was recorded.
fn sample(
    mut play: impl FnMut(&[ArenaRun], &[ship::Bout]) -> Vec<f64>,
    heats: usize,
) -> (Bar, Vec<ArenaRun>, usize) {
    let mut runs = vec![fresh(League::HeadsUp), fresh(League::SixMax)];
    let mut looks = 0;
    let mut recorded = Vec::new();
    let judged = ship::sample(
        &mut runs,
        &mut play,
        &mut || {
            looks += 1;
            looks >= heats
        },
        &mut |run: &ArenaRun| recorded.push(run.clone()),
    );
    let count = recorded.len();
    assert_eq!(
        recorded[count - 2..],
        runs[..],
        "the last heat is the one recorded"
    );
    (judged, runs, count)
}

#[test]
fn a_run_stops_as_soon_as_v10_is_read_below_the_policy_in_both_leagues() {
    let play = scripted(|opponent| match opponent {
        "v10.0.2" => (10.0, 5.0),
        _ => (1.0, 50.0),
    });
    let (judged, runs, recorded) = sample(play, usize::MAX);
    assert_eq!(judged, Bar::Clears);
    // Every row played one bout a heat, and every heat recorded both leagues.
    for run in &runs {
        for row in &run.rows {
            assert_eq!(row.bouts.len(), ship::MIN_BOUTS, "{}", row.opponent);
            assert_eq!(row.deals, ship::MIN_BOUTS * 50);
        }
    }
    assert_eq!(recorded, 2 * ship::MIN_BOUTS);
    // What is recorded is what the Arena stage reads: the release check passes on it.
    let faithful = &mut |_: &ArenaRun, row: &ArenaRow, bout: u64| row.bouts[bout as usize];
    let (commit, champion) = (runs[0].commit.clone(), runs[0].champion.clone().unwrap());
    assert_eq!(ship::arena(&runs, &commit, &champion, faithful), Ok(()));
}

#[test]
fn a_run_stops_as_soon_as_an_opponent_is_read_above_the_policy() {
    let play = scripted(|opponent| match opponent {
        "nit" => (-20.0, 5.0),
        _ => (1.0, 50.0),
    });
    let Bar::Refused(why) = sample(play, usize::MAX).0 else {
        panic!("a regression is refused");
    };
    assert!(
        why.iter()
            .any(|why| why.starts_with("regression: the nit beats the policy at heads-up")),
        "{why:?}"
    );
}

#[test]
fn a_run_still_undecided_when_the_cap_runs_out_is_refused() {
    let play = scripted(|_| (1.0, 50.0));
    let (judged, runs, _) = sample(play, 40);
    let Bar::Refused(why) = judged else {
        panic!("an undecided run is refused at the cap");
    };
    assert_eq!(why[0], "the bar is not decided within the cap: 40 heats");
    assert!(
        why[1..]
            .iter()
            .all(|why| why.contains("is not yet told from zero")),
        "{why:?}"
    );
    assert_eq!(runs[0].rows[0].bouts.len(), 40);
}

#[test]
fn sample_with_stops_at_the_heat_its_judge_decides() {
    let mut runs = [fresh(League::HeadsUp), fresh(League::SixMax)];
    let mut recorded = 0;
    let outcome = ship::sample_with(
        &mut runs,
        &mut scripted(|_| (1., 0.)),
        &mut || false,
        &mut |_| recorded += 1,
        &|runs| {
            if runs[0].rows[0].bouts.len() == 3 {
                Bar::Clears
            } else {
                Bar::Open
            }
        },
    );
    assert_eq!(outcome, Bar::Clears);
    assert_eq!(recorded, 6);
    assert!(
        runs.iter()
            .flat_map(|run| &run.rows)
            .all(|row| row.bouts.len() == 3)
    );
}

#[test]
fn sample_with_refuses_when_the_cap_runs_out_undecided() {
    let mut runs = [fresh(League::HeadsUp), fresh(League::SixMax)];
    let mut looks = 0;
    let outcome = ship::sample_with(
        &mut runs,
        &mut scripted(|_| (1., 0.)),
        &mut || {
            looks += 1;
            looks == 2
        },
        &mut |_| {},
        &|_| Bar::Open,
    );
    assert!(matches!(outcome, Bar::Refused(_)));
    assert!(
        runs.iter()
            .flat_map(|run| &run.rows)
            .all(|row| row.bouts.len() == 2)
    );
}

#[test]
fn sample_with_refuses_an_incomplete_heat_without_recording_it() {
    let mut runs = [fresh(League::HeadsUp), fresh(League::SixMax)];
    let before = runs.clone();
    let outcome = ship::sample_with(
        &mut runs,
        &mut |_, bouts| vec![0.; bouts.len() - 1],
        &mut || false,
        &mut |_| panic!("an incomplete heat must not be recorded"),
        &|_| Bar::Open,
    );
    assert!(matches!(outcome, Bar::Refused(_)));
    assert_eq!(runs, before);
}
