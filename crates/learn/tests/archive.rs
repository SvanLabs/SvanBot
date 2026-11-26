//! The archive and the regression rule (V11-96, spec Phase 3).

use svanbot_learn::archive::{
    Archive, Archived, Archives, Bouts, EXPLOITABILITY_BOUND, Exploitability, KEEP, Regression,
    regression,
};
use svanbot_strategy::params::{Digest, Params};

fn digest(exploit: f64) -> Digest {
    Params {
        exploit,
        spread: 0.0,
    }
    .digest()
}

fn archived(exploit: f64) -> Archived {
    Archived {
        params: Params {
            exploit,
            spread: 0.0,
        },
        style: "tight",
        promotion: Vec::new(),
    }
}

/// A candidate's bout differences against a champion: `mbb` mbb/100 on every bout.
fn bouts(exploit: f64, mbb: f64) -> Bouts {
    Bouts {
        champion: digest(exploit),
        differences: vec![mbb; 40],
    }
}

/// An archive holding one champion for each exploit value, oldest first.
fn archive_of(exploits: &[f64]) -> Archive {
    let mut archive = Archive::new();
    for &exploit in exploits {
        archive.record(archived(exploit));
    }
    archive
}

#[test]
fn a_sixth_promotion_evicts_the_oldest_and_the_archive_holds_five() {
    let mut archive = Archive::new();
    for step in 1..=6 {
        archive.record(archived(0.1 * f64::from(step)));
    }
    assert_eq!(archive.len(), KEEP);
    let held: Vec<Digest> = archive
        .champions()
        .map(|champion| champion.digest())
        .collect();
    let newest_first = (2..=6).rev().map(|step| digest(0.1 * f64::from(step)));
    assert_eq!(held, newest_first.collect::<Vec<_>>());
    assert!(!held.contains(&digest(0.1)));
}

#[test]
fn a_champion_promoted_again_moves_to_the_newest() {
    let mut archive = Archive::new();
    for exploit in [0.1, 0.2, 0.3, 0.4, 0.5] {
        archive.record(archived(exploit));
    }
    archive.record(archived(0.3));

    assert_eq!(archive.len(), KEEP);
    let held: Vec<Digest> = archive
        .champions()
        .map(|champion| champion.digest())
        .collect();
    let expected = [0.3, 0.5, 0.4, 0.2, 0.1].map(digest);
    assert_eq!(held, expected);
}

#[test]
fn each_bot_keeps_its_own_five() {
    let mut archives = Archives::new();
    for step in 1..=6 {
        archives.record("anchor", archived(0.1 * f64::from(step)));
    }
    archives.record("hunter", archived(0.9));

    let anchor = archives.get("anchor").expect("anchor has an archive");
    let hunter = archives.get("hunter").expect("hunter has an archive");
    assert_eq!(anchor.len(), KEEP);
    assert_eq!(hunter.len(), 1);
    assert_eq!(hunter.champions().next().unwrap().digest(), digest(0.9));
    assert!(
        !anchor
            .champions()
            .any(|champion| champion.digest() == digest(0.9)),
        "a champion of one bot never reaches a stablemate's archive"
    );
    assert!(archives.get("grinder").is_none());
}

#[test]
fn a_candidate_that_loses_to_an_archived_champion_is_refused() {
    let champions = [0.1, 0.2, 0.3, 0.4, 0.5];
    let archive = archive_of(&champions);
    let mut evidence: Vec<Bouts> = champions.map(|exploit| bouts(exploit, 2_000.0)).to_vec();
    // The candidate beats four of the five and loses to the third.
    evidence[2] = bouts(0.3, -2_000.0);

    assert_eq!(
        regression(&archive, &evidence),
        Regression::Refused {
            champion: digest(0.3)
        }
    );
}

#[test]
fn a_candidate_that_beats_all_five_is_admitted() {
    let champions = [0.1, 0.2, 0.3, 0.4, 0.5];
    let archive = archive_of(&champions);
    let evidence: Vec<Bouts> = champions
        .iter()
        .map(|&exploit| bouts(exploit, 2_000.0))
        .collect();
    assert_eq!(regression(&archive, &evidence), Regression::Admitted);
}

#[test]
fn a_tie_with_an_archived_champion_holds_the_incumbent() {
    let champions = [0.1, 0.2, 0.3, 0.4, 0.5];
    let archive = archive_of(&champions);
    let mut evidence: Vec<Bouts> = champions
        .iter()
        .map(|&exploit| bouts(exploit, 2_000.0))
        .collect();
    evidence[4] = bouts(0.5, 0.0);

    assert_eq!(regression(&archive, &evidence), Regression::Unresolved);
}

#[test]
fn a_wide_interval_around_zero_holds_the_incumbent() {
    let champions = [0.1, 0.2, 0.3, 0.4, 0.5];
    let archive = archive_of(&champions);
    let mut evidence: Vec<Bouts> = champions
        .iter()
        .map(|&exploit| bouts(exploit, 2_000.0))
        .collect();
    evidence[1] = Bouts {
        champion: digest(0.2),
        differences: [1_000.0, -1_000.0].repeat(20),
    };

    assert_eq!(regression(&archive, &evidence), Regression::Unresolved);
}

#[test]
fn missing_evidence_holds_the_incumbent() {
    let champions = [0.1, 0.2, 0.3, 0.4, 0.5];
    let archive = archive_of(&champions);
    let mut evidence: Vec<Bouts> = champions
        .iter()
        .map(|&exploit| bouts(exploit, 2_000.0))
        .collect();
    evidence.pop();

    assert_eq!(regression(&archive, &evidence), Regression::Unresolved);
}

#[test]
fn too_few_bouts_hold_the_incumbent() {
    let champions = [0.1, 0.2, 0.3, 0.4, 0.5];
    let archive = archive_of(&champions);
    let mut evidence: Vec<Bouts> = champions
        .iter()
        .map(|&exploit| bouts(exploit, 2_000.0))
        .collect();
    evidence[0] = Bouts {
        champion: digest(0.1),
        differences: vec![-2_000.0; 10],
    };

    assert_eq!(regression(&archive, &evidence), Regression::Unresolved);
}

#[test]
fn an_empty_archive_admits_the_first_champion() {
    let champions = [0.1, 0.2];
    let evidence: Vec<Bouts> = champions
        .iter()
        .map(|&exploit| bouts(exploit, 2_000.0))
        .collect();

    assert_eq!(regression(&Archive::new(), &evidence), Regression::Admitted);
}

#[test]
fn the_exploitability_bound_is_five_bb_per_100() {
    assert_eq!(EXPLOITABILITY_BOUND, 5_000.0);
    assert!(Exploitability::Unmeasured.within_bound());
    assert!(Exploitability::Measured(5_000.0).within_bound());
    assert!(!Exploitability::Measured(5_000.1).within_bound());
}
