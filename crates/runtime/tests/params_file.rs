//! Parameter sets on disk: the new mixing spread is optional in an old champion file.
#![allow(clippy::disallowed_methods, clippy::disallowed_types)]

use std::path::{Path, PathBuf};

use svanbot_runtime::fleet::Fleet;
use svanbot_strategy::params::Params;

fn scratch(name: &str) -> PathBuf {
    // Cargo's own scratch root for this test binary, fixed at compile time: no environment read,
    // and nothing a caller could point somewhere else.
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("svanbot-params-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn an_old_champion_file_without_the_spread_reads_with_the_default() {
    let dir = scratch("old");
    // The digest an old file wrote: FNV-1a over exploit's name and bits alone.
    std::fs::write(
        dir.join("champion.json"),
        r#"{"digest":"19a6ec23fd7f5a08","numbers":{"exploit":"0.3"}}"#,
    )
    .unwrap();
    let fleet = Fleet::in_dir(&dir);
    assert_eq!(
        fleet.champion(),
        Ok(Params {
            exploit: 0.3,
            spread: Params::DEFAULT.spread,
            ..Params::DEFAULT
        })
    );
}

#[test]
fn a_new_champion_file_writes_the_spread_and_reads_back() {
    let dir = scratch("new");
    let fleet = Fleet::in_dir(&dir);
    let params = Params {
        exploit: 0.25,
        spread: 0.1,
        ..Params::DEFAULT
    };
    fleet.promote(&params).unwrap();
    assert_eq!(fleet.champion(), Ok(params));
    let text = std::fs::read_to_string(dir.join("champion.json")).unwrap();
    assert!(
        text.contains("spread"),
        "the champion file lacks the spread"
    );
}

#[test]
fn a_champion_carrying_hunters_cuts_reads_back() {
    let dir = scratch("cuts");
    let fleet = Fleet::in_dir(&dir);
    let params = Params {
        station_vpip: 0.9,
        nit_fold: 0.6,
        ..Params::DEFAULT
    };
    fleet.promote(&params).unwrap();
    assert_eq!(fleet.champion(), Ok(params));
    let text = std::fs::read_to_string(dir.join("champion.json")).unwrap();
    assert!(
        text.contains("station_vpip"),
        "the champion file lacks the station cut"
    );
    assert!(
        text.contains("nit_fold"),
        "the champion file lacks the nit cut"
    );
}
