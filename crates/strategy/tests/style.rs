use svanbot_strategy::kind::PolicyKind;
use svanbot_strategy::params::Params;
use svanbot_strategy::profile::ACTIVE;
use svanbot_strategy::style::{self, LOOSE, STANDARD, Style, TIGHT};

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
    assert_eq!(TIGHT.seed(Params { exploit: 0.25 }).exploit, 0.25);
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
        BOUNDED.seed(Params { exploit: 0.5 }),
        Params { exploit: 0.5 }
    );
    assert_eq!(PINNED.seed(Params { exploit: 0.5 }).exploit, 0.4);
    assert!(PINNED.allows("exploit", 0.4));
    assert!(!PINNED.allows("exploit", 0.5));
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
    let params = Params { exploit: 0.35 };
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
