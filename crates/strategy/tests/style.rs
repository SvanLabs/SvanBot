use svanbot_strategy::kind::PolicyKind;
use svanbot_strategy::profile::ACTIVE;
use svanbot_strategy::style::{self, LOOSE, STANDARD, TIGHT};

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
    use svanbot_strategy::params::Params;
    // The width is pinned in the style, not in a parameter set, so seeding moves no number and
    // no digest: the policy reads the width from the style it plays.
    assert_eq!(TIGHT.seed(Params::DEFAULT), Params::DEFAULT);
    assert_eq!(LOOSE.seed(Params::DEFAULT), Params::DEFAULT);
    assert_eq!(TIGHT.seed(Params { exploit: 0.25 }).exploit, 0.25);
}
