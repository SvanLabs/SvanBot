use svanbot_core::rules::Blinds;
use svanbot_learn::paired::Match;
use svanbot_learn::play::{player, profile_stats, rival_duel, rival_ring_at};
use svanbot_learn::reads::Reads;
use svanbot_strategy::{
    kind::PolicyKind, params::Params, policy::Policy, profile::Profile, style::Style,
};

const GAME: Match = Match {
    blinds: Blinds { small: 10, big: 20 },
    stack: 2000,
    seed: 7,
};

#[test]
fn a_policy_playing_itself_round_a_table_breaks_even() {
    let play = player(Policy::reference(Params::DEFAULT), Some(2));
    for deal in 0..2 {
        assert!(rival_ring_at(&GAME, 3, deal, &play, &play).abs() < 1e-9);
    }
}

#[test]
fn a_heads_up_stablemate_deal_plays_the_same_twice() {
    let play = player(Policy::reference(Params::DEFAULT), Some(2));
    let first = rival_duel(&GAME, 0, &play, &play);
    assert!(first.is_finite());
    assert_eq!(
        first.to_bits(),
        rival_duel(&GAME, 0, &play, &play).to_bits()
    );
}

#[test]
fn a_style_with_rules_collects_stats_per_seat() {
    let policy = Policy::reference(Params::DEFAULT);
    let stats = profile_stats(&policy, &Reads::default(), &[(1, "x"), (3, "y")]);
    assert_eq!(
        stats.iter().map(|stats| stats.seat).collect::<Vec<_>>(),
        [1, 3]
    );
}

#[test]
fn a_style_with_no_rules_collects_none() {
    static BARE: Style = Style {
        name: "bare",
        open_scale: 1.0,
        profile: Profile::new(&[]),
    };
    let policy = Policy::new(PolicyKind::Solving, &BARE, Params::DEFAULT);
    assert!(profile_stats(&policy, &Reads::default(), &[(1, "x")]).is_empty());
}
