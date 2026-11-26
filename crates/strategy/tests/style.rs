use svanbot_strategy::kind::PolicyKind;
use svanbot_strategy::params::Params;
use svanbot_strategy::profile::ACTIVE;
use svanbot_strategy::style::{self, ANCHOR, LOOSE, STANDARD, Style, TIGHT};

#[test]
fn a_kind_and_a_style_are_known_by_the_name_they_print() {
    assert_eq!(PolicyKind::named("solving"), Some(PolicyKind::Solving));
    assert_eq!(PolicyKind::named("pricing"), Some(PolicyKind::Pricing));
    assert_eq!(PolicyKind::named("Solving"), None);
    assert_eq!(PolicyKind::named("nope"), None);
    for kind in PolicyKind::ALL {
        assert_eq!(PolicyKind::named(kind.name()), Some(kind));
    }
    for style in style::ALL {
        assert_eq!(style::named(style.name).unwrap().name, style.name);
        assert_eq!(format!("{style:?}"), style.name);
    }
    assert!(style::named("Standard").is_none());
    assert!(style::named("nope").is_none());
}

#[test]
fn the_standard_style_is_todays_play() {
    assert_eq!(STANDARD.open_scale, 1.0);
    assert_eq!(STANDARD.profile.is_empty(), ACTIVE.is_empty());
}

#[test]
fn the_working_styles_pin_the_values_of_the_design() {
    assert_eq!(TIGHT.open_scale, 0.75);
    assert_eq!(LOOSE.open_scale, 1.25);
}

#[test]
fn every_style_pins_a_width_a_player_could_play() {
    for (index, style) in style::ALL.iter().enumerate() {
        assert!((0.5..=1.5).contains(&style.open_scale));
        assert!(
            style::ALL[..index]
                .iter()
                .all(|other| other.name != style.name)
        );
    }
}

#[test]
fn a_style_seeds_a_slot_with_its_pins_applied() {
    // The width is pinned in the style, not in a parameter set, so seeding moves no number and
    // no digest: the policy reads the width from the style it plays.
    assert_eq!(TIGHT.seed(Params::DEFAULT), Params::DEFAULT);
    assert_eq!(LOOSE.seed(Params::DEFAULT), Params::DEFAULT);
    assert_eq!(
        TIGHT
            .seed(Params {
                exploit: 0.25,
                spread: 0.0,
                ..Params::DEFAULT
            })
            .exploit,
        0.25
    );
}

#[test]
fn a_style_bounds_the_numbers_its_search_may_move_and_pins_the_zero_width_ones() {
    static BOUNDED: Style = Style {
        name: "bounded",
        open_scale: 0.8,
        profile: ACTIVE,
        bounds: &[("exploit", 0.3, 0.6)],
    };
    static PINNED: Style = Style {
        name: "pinned",
        open_scale: 0.8,
        profile: ACTIVE,
        bounds: &[("exploit", 0.4, 0.4)],
    };
    assert!(BOUNDED.allows("exploit", 0.3));
    assert!(BOUNDED.allows("exploit", 0.6));
    assert!(!BOUNDED.allows("exploit", 0.61));
    // A number the style says nothing about is left to the parameter set's own range.
    assert!(BOUNDED.allows("unknown", 99.0));
    // Seeding applies a pin, and a pin is a zero-width bound: the search never moves it.
    assert_eq!(
        BOUNDED.seed(Params {
            exploit: 0.5,
            spread: 0.0,
            ..Params::DEFAULT
        }),
        Params {
            exploit: 0.5,
            spread: 0.0,
            ..Params::DEFAULT
        }
    );
    assert_eq!(
        PINNED
            .seed(Params {
                exploit: 0.5,
                spread: 0.0,
                ..Params::DEFAULT
            })
            .exploit,
        0.4
    );
    assert!(PINNED.allows("exploit", 0.4));
    assert!(!PINNED.allows("exploit", 0.5));
}

#[test]
fn anchor_bounds_its_two_numbers_and_seeds_the_design_values() {
    let seeded = ANCHOR.seed(Params::DEFAULT);
    assert_eq!(seeded.exploit, 0.25);
    assert_eq!(seeded.spread, 0.1);
    assert!(ANCHOR.allows("exploit", 0.25));
    assert!(!ANCHOR.allows("exploit", 0.26));
    assert!(ANCHOR.allows("spread", 0.5));
    assert!(!ANCHOR.allows("spread", 0.51));
    assert!(ANCHOR.profile.is_empty());
}

#[test]
fn a_seeded_style_decides_exactly_as_the_same_style_without_seeding() {
    use svanbot_core::rules::{Legal, RaiseRange};
    use svanbot_core::spot::Spot;
    use svanbot_strategy::kind::PolicyKind;
    use svanbot_strategy::params::Params;
    use svanbot_strategy::policy::Policy;
    use svanbot_strategy::pricer::Budget;

    let spot = Spot {
        hole: "AhKh".parse().unwrap(),
        board: "QhJhTh2c3d".parse().unwrap(),
        pot: 400,
        legal: Legal {
            call: Some(200),
            raise: Some(RaiseRange {
                min: 400,
                max: 2_000,
            }),
        },
        opponents: 1,
        stack: 2_000,
        callable_pot: None,
        committed: 0,
        big_blind: 20,
        in_position: false,
        players_behind: None,
        lines: vec![],
        seats: vec![],
    };
    let budget = Budget {
        samples: 3_200,
        seed: 5,
    };
    let params = Params {
        exploit: 0.25,
        spread: 0.1,
        ..Params::DEFAULT
    };
    for style in style::ALL {
        let seeded = Policy::new(PolicyKind::Solving, style, style.seed(params));
        let raw = Policy::new(PolicyKind::Solving, style, params);
        assert_eq!(
            seeded.decide(&spot, None, &[], budget, 60, &[]),
            raw.decide(&spot, None, &[], budget, 60, &[]),
            "{}",
            style.name
        );
    }
}

#[test]
fn hunter_bounds_the_playbooks_cuts_and_the_other_styles_pin_them() {
    // Hunter, the Exploiting brain, learns the cuts its playbook classifies opponents with,
    // inside its own bounds, and plays no style profile: the playbook is the kind's logic.
    let hunter = style::named("hunter").expect("hunter is a style");
    assert!(hunter.profile.is_empty());
    assert!(hunter.allows("station_vpip", 0.9));
    assert!(!hunter.allows("station_vpip", 0.2));
    assert!(hunter.allows("nit_fold", 0.8));
    assert!(!hunter.allows("nit_fold", 0.2));
    assert!(hunter.allows("exploit", 0.25));
    // Every other style pins the cuts at the first profile's numbers, so seeding one of their
    // slots with a champion that carries Hunter's cuts moves none of their numbers.
    for style in style::ALL.iter().filter(|style| style.name != "hunter") {
        let seeded = style.seed(Params {
            station_vpip: 0.95,
            nit_fold: 0.5,
            ..Params::DEFAULT
        });
        assert_eq!(
            seeded.station_vpip,
            Params::DEFAULT.station_vpip,
            "{}",
            style.name
        );
        assert_eq!(seeded.nit_fold, Params::DEFAULT.nit_fold, "{}", style.name);
    }
    // Hunter keeps the cuts a champion brings inside its bounds.
    let seeded = hunter.seed(Params {
        station_vpip: 0.95,
        nit_fold: 0.5,
        ..Params::DEFAULT
    });
    assert_eq!(seeded.station_vpip, 0.95);
    assert_eq!(seeded.nit_fold, 0.5);
}
