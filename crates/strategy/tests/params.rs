//! Parameter sets and their digests.

use svanbot_strategy::params::Params;

#[test]
fn equal_parameter_sets_have_one_digest_and_any_change_gives_another() {
    let champion = Params::DEFAULT;
    assert_eq!(champion.digest(), Params::DEFAULT.digest());
    let challenger = Params {
        exploit: 0.25,
        spread: 0.0,
    };
    assert_ne!(champion.digest(), challenger.digest());
    let other = Params {
        exploit: 0.75,
        spread: 0.0,
    };
    assert_ne!(champion.digest(), other.digest());
    assert_ne!(challenger.digest(), other.digest());
    // A digest prints as 16 hex digits and reads back.
    let text = champion.digest().to_string();
    assert_eq!(text.len(), 16);
    assert_eq!(text.parse(), Ok(champion.digest()));
}

#[test]
fn the_default_digest_is_the_one_the_fleet_played_before_the_new_number() {
    // The champion in play and every old champion.json are named by this digest. Adding the
    // spread at its default must not move it.
    assert_eq!(Params::DEFAULT.digest().to_string(), "89d4ce49742c70a9");
}

#[test]
fn the_spread_is_a_number_with_its_own_range_and_digest() {
    let tight = Params {
        exploit: 0.5,
        spread: 0.0,
    };
    let mixed = Params {
        exploit: 0.5,
        spread: 0.1,
    };
    assert_ne!(tight.digest(), mixed.digest());
    assert!(tight.in_range());
    assert!(mixed.in_range());
    assert!(
        !Params {
            exploit: 0.5,
            spread: -0.1
        }
        .in_range()
    );
    assert_eq!(
        mixed.with("spread", 0.2),
        Some(Params {
            exploit: 0.5,
            spread: 0.2
        })
    );
    assert_eq!(mixed.with("missing", 1.0), None);
    assert_eq!(mixed.named(), [("exploit", 0.5), ("spread", 0.1)]);
}
