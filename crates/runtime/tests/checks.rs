#![allow(clippy::disallowed_methods, clippy::disallowed_types)]

use svanbot_runtime::fleet::{Check, Fleet};
use svanbot_strategy::{kind::PolicyKind, params::Params};

#[test]
fn a_check_is_kept_newest_first_one_a_policy_and_read_back_whole() {
    let dir = std::env::temp_dir().join(format!("svanbot-checks-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let fleet = Fleet::in_dir(dir);
    let mut tight = Check {
        kind: PolicyKind::Solving,
        style: "tight".into(),
        reference: Params::DEFAULT.digest(),
        seed: 7,
        iterations: Some(20),
        bouts: 60,
        admitted: true,
        heads_up: "admitted".into(),
        six_max: "admitted".into(),
        checked_ms: 1,
    };
    assert!(fleet.checks().unwrap().is_empty());
    fleet.record_check(&tight).unwrap();
    let loose = Check {
        style: "loose".into(),
        checked_ms: 3,
        ..tight.clone()
    };
    fleet.record_check(&loose).unwrap();
    tight.checked_ms = 2;
    tight.admitted = false;
    fleet.record_check(&tight).unwrap();
    assert_eq!(fleet.checks().unwrap(), [loose, tight]);
}
