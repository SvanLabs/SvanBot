//! The port plays as v10.0.2 did: at every recorded spot, seeded as v10 seeded it, it takes the
//! action the v10.0.2 binary took.

mod v10_spots;

use svanbot_core::rng::Rng;
use svanbot_sim::v10::params::Params;
use svanbot_sim::v10::policy::{Action, decide};
use v10_spots::{Recorded, spots};

fn recorded(action: Action) -> Recorded {
    match action {
        Action::Fold => Recorded::Fold,
        Action::Check => Recorded::Check,
        Action::Call => Recorded::Call,
        Action::RaiseTo(to) => Recorded::RaiseTo(to),
        Action::AllIn => Recorded::AllIn,
    }
}

#[test]
fn every_recorded_spot_gets_v10s_action() {
    let params = Params::default();
    let mut misses = Vec::new();
    let all = spots();
    for (i, spot) in all.iter().enumerate() {
        let mut rng = Rng::seeded(spot.seed);
        let got = recorded(decide(&spot.situation, &params, &mut rng));
        if got != spot.action {
            misses.push(format!(
                "spot {i} seed {}: v10 {:?}, port {got:?}",
                spot.seed, spot.action
            ));
        }
    }
    assert!(
        misses.is_empty(),
        "{} of {} spots differ:\n{}",
        misses.len(),
        all.len(),
        misses.join("\n")
    );
}
