//! Parameter sets and their digests.

use svanbot_strategy::params::Params;

#[test]
fn equal_parameter_sets_have_one_digest_and_any_change_gives_another() {
    let champion = Params::DEFAULT;
    assert_eq!(champion.digest(), Params::DEFAULT.digest());
    let challenger = Params { exploit: 0.25 };
    assert_ne!(champion.digest(), challenger.digest());
    let other = Params { exploit: 0.75 };
    assert_ne!(champion.digest(), other.digest());
    assert_ne!(challenger.digest(), other.digest());
    // A digest prints as 16 hex digits and reads back.
    let text = champion.digest().to_string();
    assert_eq!(text.len(), 16);
    assert_eq!(text.parse(), Ok(champion.digest()));
}
