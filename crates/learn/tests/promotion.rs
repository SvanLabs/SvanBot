//! The per-bot promotion gate (V11-77): a fresh 6-max improvement plus a gauntlet row for every
//! designated opponent, before a live guard may start.

use svanbot_learn::paired::Verdict;
use svanbot_learn::promotion::{DESIGNATED, Evidence, GAUNTLET_MARGIN, Gate, GauntletRow, gate};
use svanbot_strategy::kind::PolicyKind;
use svanbot_strategy::params::Params;
use svanbot_strategy::style::TIGHT;

fn verdict(mbb: f64, ci95: f64) -> Verdict {
    Verdict {
        deals: 1_600,
        mbb_per_100: mbb,
        ci95,
    }
}

fn rows(lower: f64) -> Vec<GauntletRow> {
    DESIGNATED
        .into_iter()
        .map(|opponent| GauntletRow {
            opponent: opponent.into(),
            verdict: verdict(lower + 100.0, 100.0),
        })
        .collect()
}

fn evidence(fresh: Verdict, gauntlet: Vec<GauntletRow>) -> Evidence {
    Evidence {
        incumbent: Params {
            exploit: 0.4,
            ..Params::DEFAULT
        },
        challenger: Params {
            exploit: 0.32,
            ..Params::DEFAULT
        },
        kind: PolicyKind::Solving,
        style: TIGHT.name,
        fresh,
        gauntlet,
    }
}

#[test]
fn a_fresh_edge_above_zero_with_every_designated_row_clear_is_admitted() {
    assert_eq!(
        gate(&evidence(verdict(300.0, 100.0), rows(0.0))),
        Gate::Admitted
    );
}

#[test]
fn a_fresh_edge_that_does_not_exceed_zero_is_refused() {
    let at_zero = gate(&evidence(verdict(100.0, 100.0), rows(0.0)));
    assert!(matches!(at_zero, Gate::Refused(ref why) if why[0].contains("fresh")));
    let below = gate(&evidence(verdict(-100.0, 100.0), rows(0.0)));
    assert!(matches!(below, Gate::Refused(_)));
}

#[test]
fn a_designated_row_is_refused_only_when_its_whole_interval_can_lose_more_than_five_bb() {
    let at_bar = gate(&evidence(verdict(300.0, 100.0), rows(-GAUNTLET_MARGIN)));
    assert_eq!(
        at_bar,
        Gate::Admitted,
        "the lower bound is exactly -5 bb/100"
    );
    let past = gate(&evidence(
        verdict(300.0, 100.0),
        rows(-GAUNTLET_MARGIN - 100.0),
    ));
    assert!(
        matches!(past, Gate::Refused(_)),
        "the lower bound is below -5 bb/100"
    );
}

#[test]
fn a_missing_or_undecided_row_is_incomplete_and_holds_the_incumbent() {
    let mut missing = rows(0.0);
    missing.retain(|row| row.opponent != "nit");
    assert!(matches!(
        gate(&evidence(verdict(300.0, 100.0), missing)),
        Gate::Incomplete(_)
    ));

    let mut undecided = rows(0.0);
    if let Some(row) = undecided.iter_mut().find(|row| row.opponent == "maniac") {
        row.verdict = Verdict {
            deals: 0,
            mbb_per_100: 0.0,
            ci95: f64::INFINITY,
        };
    }
    assert!(matches!(
        gate(&evidence(verdict(300.0, 100.0), undecided)),
        Gate::Incomplete(_)
    ));
    // An undecided fresh confirmation is incomplete too.
    assert!(matches!(
        gate(&evidence(
            Verdict {
                deals: 0,
                mbb_per_100: 0.0,
                ci95: f64::INFINITY,
            },
            rows(0.0)
        )),
        Gate::Incomplete(_)
    ));
}

#[test]
fn a_complete_refusal_is_not_hidden_by_another_incomplete_row() {
    // The candidate is over the fresh bar, but one row is clearly bad and another missing. The
    // incomplete row wins: no promotion either way, and the reason names the missing one.
    let mut gauntlet = rows(-GAUNTLET_MARGIN - 100.0);
    gauntlet.retain(|row| row.opponent != "nit");
    let reading = gate(&evidence(verdict(300.0, 100.0), gauntlet));
    assert!(matches!(reading, Gate::Incomplete(ref why) if why[0].contains("nit")));
}
