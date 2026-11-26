//! Refresh (V11-81, spec Phase 2 item 6): a lineage dominated in the round-robin selects the
//! best eligible champion of its own style from its archive.

use svanbot_learn::archive::{Archive, Archived};
use svanbot_learn::paired::Verdict;
use svanbot_learn::probe::Verdict as Guard;
use svanbot_learn::refresh::{Matchup, Outcome, Reason, Row, refresh};
use svanbot_strategy::params::Params;

fn params(exploit: f64) -> Params {
    Params {
        exploit,
        spread: 0.0,
        ..Params::DEFAULT
    }
}

fn archived(exploit: f64, style: &'static str, edge: f64) -> Archived {
    Archived {
        params: params(exploit),
        style,
        promotion: vec![edge; 40],
    }
}

/// The other lineage's edge over the row: `edge` mbb/100 with a ±100 interval.
fn won(edge: f64) -> Verdict {
    Verdict {
        deals: 100,
        mbb_per_100: edge,
        ci95: 100.0,
    }
}

fn row(exploit: f64, style: &'static str, matchups: Vec<Matchup>) -> Row {
    Row {
        bot: "Svanar".into(),
        style,
        champion: params(exploit),
        matchups,
    }
}

#[test]
fn a_dominated_lineage_selects_its_best_same_style_archived_champion() {
    let mut archive = Archive::new();
    // The other style is stronger, and so is the incumbent itself: neither is eligible.
    archive.record(archived(0.2, "loose", 4_000.0));
    archive.record(archived(0.3, "tight", 1_000.0));
    archive.record(archived(0.4, "tight", 3_000.0));
    archive.record(archived(0.5, "tight", 5_000.0));
    let standing = row(
        0.5,
        "tight",
        vec![Matchup {
            name: "Svanby".into(),
            verdict: won(2_000.0),
        }],
    );

    let plan = refresh(&standing, &archive);

    let Outcome::Selected(selected) = &plan.outcome else {
        panic!("expected a refresh, found {:?}", plan.outcome);
    };
    assert_eq!(selected.params, params(0.4));
    assert_eq!(plan.selected(), Some(params(0.4).digest()));
    assert_eq!(plan.evidence.len(), 1);
    assert_eq!(plan.evidence[0].name, "Svanby");
    assert_eq!(plan.incumbent, params(0.5).digest());
}

#[test]
fn a_lineage_no_other_leads_it_keeps_its_incumbent() {
    let standing = row(
        0.5,
        "tight",
        vec![
            // Losing, and a tie that the interval does not separate from zero.
            Matchup {
                name: "Svanby".into(),
                verdict: won(-2_000.0),
            },
            Matchup {
                name: "Svanism".into(),
                verdict: Verdict {
                    deals: 100,
                    mbb_per_100: 300.0,
                    ci95: 500.0,
                },
            },
        ],
    );
    let plan = refresh(&standing, &Archive::new());
    assert_eq!(plan.outcome, Outcome::NoChange(Reason::NotDominated));
    assert_eq!(plan.selected(), None);
    assert!(plan.evidence.is_empty());
}

#[test]
fn a_champion_of_another_style_is_not_eligible() {
    let mut archive = Archive::new();
    archive.record(archived(0.2, "loose", 4_000.0));
    let standing = row(
        0.5,
        "tight",
        vec![Matchup {
            name: "Svanby".into(),
            verdict: won(2_000.0),
        }],
    );
    let plan = refresh(&standing, &archive);
    assert_eq!(plan.outcome, Outcome::NoChange(Reason::NoEligibleChampion));
    assert_eq!(plan.selected(), None);
    assert_eq!(plan.evidence.len(), 1, "the domination is still recorded");
}

#[test]
fn the_incumbent_stays_when_no_archived_champion_is_eligible() {
    let mut archive = Archive::new();
    // The only same-style champion is the incumbent itself.
    archive.record(archived(0.5, "tight", 5_000.0));
    // And this one's promotion never established an edge.
    archive.record(Archived {
        params: params(0.3),
        style: "tight",
        promotion: vec![],
    });
    let standing = row(
        0.5,
        "tight",
        vec![Matchup {
            name: "Svanby".into(),
            verdict: won(2_000.0),
        }],
    );
    let plan = refresh(&standing, &archive);
    assert_eq!(plan.outcome, Outcome::NoChange(Reason::NoEligibleChampion));
    assert_eq!(plan.evidence.len(), 1);
}

#[test]
fn the_evidence_records_every_dominating_matchup_strongest_first() {
    let standing = row(
        0.5,
        "tight",
        vec![
            Matchup {
                name: "Svanby".into(),
                verdict: won(1_000.0),
            },
            Matchup {
                name: "Svanar".into(),
                verdict: won(2_000.0),
            },
            Matchup {
                name: "Svanism".into(),
                verdict: won(500.0),
            },
            Matchup {
                name: "Svanbot".into(),
                verdict: won(-1_000.0),
            },
        ],
    );
    let mut archive = Archive::new();
    archive.record(archived(0.3, "tight", 1_000.0));
    let plan = refresh(&standing, &archive);
    let names: Vec<&str> = plan
        .evidence
        .iter()
        .map(|matchup| matchup.name.as_str())
        .collect();
    assert_eq!(names, ["Svanar", "Svanby", "Svanism"]);
    assert_eq!(plan, refresh(&standing, &archive));
}

#[test]
fn a_refresh_only_yields_its_champion_through_a_cleared_guard() {
    let mut archive = Archive::new();
    archive.record(archived(0.4, "tight", 3_000.0));
    let standing = row(
        0.5,
        "tight",
        vec![Matchup {
            name: "Svanby".into(),
            verdict: won(2_000.0),
        }],
    );
    let plan = refresh(&standing, &archive);
    for guard in [
        Guard::Running,
        Guard::Supported,
        Guard::Harmful,
        Guard::SimGap,
        Guard::Inconclusive,
    ] {
        assert_eq!(plan.through_guard(guard), None, "{guard:?}");
    }
    assert_eq!(plan.through_guard(Guard::Cleared), Some(params(0.4)));

    let held = refresh(
        &row(
            0.5,
            "tight",
            vec![Matchup {
                name: "Svanby".into(),
                verdict: won(-2_000.0),
            }],
        ),
        &archive,
    );
    assert_eq!(held.through_guard(Guard::Cleared), None);
}

#[test]
fn an_evidence_tie_selects_the_newer_champion() {
    let mut archive = Archive::new();
    archive.record(archived(0.3, "tight", 2_000.0));
    archive.record(archived(0.4, "tight", 2_000.0));
    let standing = row(
        0.5,
        "tight",
        vec![Matchup {
            name: "Svanby".into(),
            verdict: won(2_000.0),
        }],
    );
    let plan = refresh(&standing, &archive);
    assert_eq!(plan.selected(), Some(params(0.4).digest()));
}
