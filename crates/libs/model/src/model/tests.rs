use super::*;
use sv10_engine::engine::{Action, Hand};
use sv10_rng::SeedableRng;
use sv10_rng::rngs::SmallRng;

fn summary(h: &Hand) -> HandSummary {
    HandSummary {
        players: (0..h.seats.len()).map(|i| (i, format!("p{i}"))).collect(),
        button: h.button,
        bb: h.bb,
        history: h.history.clone(),
        board: h.board.clone(),
        stacks: vec![],
        shown: if h.showdown() {
            h.seats.iter().enumerate().filter(|(_, s)| !s.folded).map(|(i, s)| (i, s.hole)).collect()
        } else {
            vec![]
        },
    }
}

/// The same hand as seen from named seats (what opponents read us for keys off our name, 0321).
fn named(h: &Hand, names: &[&str]) -> HandSummary {
    let mut s = summary(h);
    s.players = names.iter().enumerate().map(|(i, n)| (i, n.to_string())).collect();
    s
}

/// 0321: our own seat is never an opponent, so `observe` skips it and our play went unmodelled for the
/// fleet's whole life while the pricing read an empty pseudo-player. Our hands now feed the image
/// opponents form of us, per bot and in aggregate.
#[test]
fn our_own_seat_feeds_the_image_and_never_the_opponent_stats() {
    let mut rng = SmallRng::seed_from_u64(21);
    let mut h = Hand::new(&[2_000; 2], 0, 10, 20, &mut rng);
    h.apply(Action::RaiseTo(60)).unwrap(); // our seat opens
    h.apply(Action::Fold).unwrap(); // the opponent folds
    let hand = named(&h, &["svan", "villain"]);
    let mut m = ModelStore::default();
    m.observe_own_hand(&hand, "svan", 1.0);

    assert!(!m.players.contains_key("svan"), "we are not our own opponent");
    assert_eq!(m.players["villain"].hands, 1.0, "the opponent is modelled as before");
    for key in [format!("{HERO_SEEN_ONE}svan"), HERO_SEEN_ALL.to_string()] {
        let image = &m.hero_seen[&key];
        assert_eq!((image.hands, image.vpip.hit, image.pfr.hit), (1.0, 1.0, 1.0), "{key}");
    }
}

/// The image holds our own seat only: with two of our bots at one table each keeps its own play, the
/// aggregate holds both, and a hand our name is not dealt into leaves no image at all.
#[test]
fn the_image_counts_only_our_own_seat() {
    let mut rng = SmallRng::seed_from_u64(22);
    let mut h = Hand::new(&[2_000; 2], 0, 10, 20, &mut rng);
    h.apply(Action::RaiseTo(60)).unwrap(); // A (seat 0) opens
    h.apply(Action::Fold).unwrap(); // B (seat 1) folds
    let hand = named(&h, &["A", "B"]);
    let mut m = ModelStore::default();
    m.observe_own_hand(&hand, "A", 1.0);
    assert_eq!(m.hero_seen[HERO_SEEN_ALL].vpip.hit, 1.0);
    assert!(!m.hero_seen.contains_key(&format!("{HERO_SEEN_ONE}B")), "B's play is not A's image");

    m.observe_own_hand(&hand, "B", 1.0);
    assert_eq!(m.hero_seen[&format!("{HERO_SEEN_ONE}B")].vpip.hit, 0.0, "B folded: its own image is tight");
    assert_eq!(m.hero_seen[&format!("{HERO_SEEN_ONE}A")].vpip.hit, 1.0, "and A's is unchanged");
    assert_eq!(m.hero_seen[HERO_SEEN_ALL].hands, 2.0, "the aggregate holds both seats' play");

    m.observe_image(&named(&h, &["X", "Y"]), "A");
    assert_eq!(m.hero_seen[HERO_SEEN_ALL].hands, 2.0, "a hand we are not dealt into leaves no image");
}

/// The image ages like any player's tallies (0168): the half-life decays our own play too, while
/// `hands` keeps counting every hand so sample-size gates and hand counts read the same as for any
/// player.
#[test]
fn the_image_decays_with_the_player_half_life() {
    let mut rng = SmallRng::seed_from_u64(23);
    let mut h = Hand::new(&[2_000; 2], 0, 10, 20, &mut rng);
    h.apply(Action::RaiseTo(60)).unwrap();
    h.apply(Action::Fold).unwrap();
    let mut m = ModelStore { half_life_hands: 10.0, ..Default::default() };
    m.observe_image(&named(&h, &["svan", "villain"]), "svan");
    m.observe_image(&named(&h, &["svan", "villain"]), "svan");
    let image = &m.hero_seen[&format!("{HERO_SEEN_ONE}svan")];
    let decay = 0.5f32.powf(1.0 / 10.0);
    assert!((image.vpip.opp - (decay + 1.0)).abs() < 1e-4, "the older hand counts for less: {}", image.vpip.opp);
    assert_eq!(image.hands, 2.0, "hands count every hand, as for any player");
}

/// The knob gates and scales the image: nothing observed (or the knob at 0) is exactly the population
/// view the pricing used before 0321, a partial weight carries proportionally less evidence, and a bot
/// with no hands of its own reads as the fleet aggregate.
#[test]
fn the_image_is_gated_and_scaled_by_its_weight() {
    let mut m = ModelStore::default();
    m.population.hands = 400.0;
    for _ in 0..400 {
        m.population.vpip.add(false); // a tight pool
    }
    let mut st = PlayerStats { hands: 100.0, ..Default::default() };
    for _ in 0..100 {
        st.vpip.add(true); // our own play: every hand
    }
    m.hero_seen.insert(format!("{HERO_SEEN_ONE}svan"), st.clone());
    m.hero_seen.insert(HERO_SEEN_ALL.into(), st);

    let pool = m.profile("nobody-at-all");
    let off = m.hero_seen_view("svan", 0.0);
    assert_eq!((off.hands, off.confidence), (0.0, 0.0), "the knob at 0 is no image");
    assert!((off.vpip - pool.vpip).abs() < 1e-6, "and pricing then reads the population, as before 0321");

    let full = m.hero_seen_profile("svan", 1.0).expect("an image exists");
    assert_eq!(full.hands, 100.0);
    assert!(full.vpip > 0.9, "the image is our play, not the pool's: {}", full.vpip);
    let half = m.hero_seen_profile("svan", 0.5).expect("still an image");
    assert_eq!(half.hands, 50.0, "a half-weight image is half the evidence");
    assert!(half.vpip > pool.vpip && half.vpip < full.vpip, "and sits between the pool and our play: {}", half.vpip);
    assert!(half.confidence < full.confidence && half.confidence > pool.confidence);

    assert!(m.hero_seen_profile("other-bot", 1.0).is_some(), "a bot with no hands of its own reads as the fleet");
    assert_eq!(m.hero_seen_view("other-bot", 1.0).vpip, full.vpip);
    m.hero_seen.clear();
    assert!((m.hero_seen_view("svan", 1.0).vpip - pool.vpip).abs() < 1e-6, "nothing observed is the population view");
}

#[test]
fn preflop_open_three_bet_and_fold_to_three_bet() {
    let mut rng = SmallRng::seed_from_u64(5);
    let mut h = Hand::new(&[2000; 4], 0, 10, 20, &mut rng);
    // seat 3 (first) opens, seat 0 3bets, sb folds, bb folds, seat 3 folds.
    h.apply(Action::RaiseTo(60)).unwrap();
    h.apply(Action::RaiseTo(180)).unwrap();
    h.apply(Action::Fold).unwrap();
    h.apply(Action::Fold).unwrap();
    h.apply(Action::Fold).unwrap();
    let mut m = ModelStore::default();
    m.observe(&summary(&h), None);
    let p3 = &m.players["p3"];
    assert_eq!((p3.open_raise.opp, p3.open_raise.hit), (1.0, 1.0));
    assert_eq!((p3.fold_to_3bet.opp, p3.fold_to_3bet.hit), (1.0, 1.0));
    assert_eq!(p3.vpip.hit, 1.0);
    let p0 = &m.players["p0"];
    assert_eq!((p0.three_bet.opp, p0.three_bet.hit), (1.0, 1.0));
    assert_eq!(p0.pfr.hit, 1.0);
    let p1 = &m.players["p1"];
    assert_eq!((p1.three_bet.opp, p1.vpip.opp, p1.vpip.hit), (0.0, 1.0, 0.0));
    assert_eq!(m.population.hands, 4.0);
}

#[test]
fn postflop_fold_to_bet_and_cbet() {
    let mut rng = SmallRng::seed_from_u64(6);
    let mut h = Hand::new(&[2000; 2], 0, 10, 20, &mut rng);
    h.apply(Action::RaiseTo(60)).unwrap(); // button opens
    h.apply(Action::Call).unwrap(); // bb calls
    h.apply(Action::Check).unwrap(); // bb checks flop
    h.apply(Action::RaiseTo(80)).unwrap(); // button cbets 2/3 pot
    h.apply(Action::Fold).unwrap();
    let mut m = ModelStore::default();
    m.observe(&summary(&h), None);
    let btn = &m.players["p0"];
    assert_eq!((btn.cbet.opp, btn.cbet.hit), (1.0, 1.0));
    assert_eq!((btn.bet_first[0].opp, btn.bet_first[0].hit), (1.0, 1.0));
    let bb = &m.players["p1"];
    assert_eq!((bb.bet_first[0].opp, bb.bet_first[0].hit), (1.0, 0.0));
    assert_eq!((bb.fold_vs_bet[0].opp, bb.fold_vs_bet[0].hit), (1.0, 1.0));
    assert_eq!((bb.fold_to_cbet.opp, bb.fold_to_cbet.hit), (1.0, 1.0));
    assert_eq!(bb.fold_vs_size[1].hit, 1.0);
    assert_eq!(bb.wtsd.opp, 1.0);
    assert_eq!(bb.wtsd.hit, 0.0);
}

#[test]
fn shrinkage_moves_with_samples() {
    let mut m = ModelStore::default();
    let fresh = m.profile("nobody");
    assert!((fresh.vpip - defaults::VPIP).abs() < 1e-6);
    let mut st = PlayerStats::default();
    for _ in 0..200 {
        st.vpip.add(true);
    }
    st.hands = 200.0;
    m.players.insert("station".into(), st);
    assert!(m.profile("station").vpip > 0.9);
}

/// A cold 4-bet before the opener answers the 3-bet: the opener's fold is a fold to a 4-bet, not a
/// fold-to-3-bet sample; the 3-bettor facing the 4-bet is a fold-to-4-bet sample (0096 d).
#[test]
fn cold_four_bet_is_not_counted_as_facing_a_three_bet() {
    let mut rng = SmallRng::seed_from_u64(9);
    let mut h = Hand::new(&[5000; 4], 0, 10, 20, &mut rng);
    h.apply(Action::RaiseTo(60)).unwrap(); // seat 3 opens
    h.apply(Action::RaiseTo(180)).unwrap(); // seat 0 3-bets
    h.apply(Action::RaiseTo(500)).unwrap(); // seat 1 (sb) cold 4-bets
    h.apply(Action::Fold).unwrap(); // bb
    h.apply(Action::Fold).unwrap(); // opener
    h.apply(Action::Fold).unwrap(); // 3-bettor
    let mut m = ModelStore::default();
    m.observe(&summary(&h), None);
    let opener = &m.players["p3"];
    assert_eq!((opener.fold_to_3bet.opp, opener.four_bet.opp), (0.0, 0.0));
    let three_bettor = &m.players["p0"];
    assert_eq!((three_bettor.fold_to_4bet.opp, three_bettor.fold_to_4bet.hit), (1.0, 1.0));
}

#[test]
fn named_players_keep_their_own_stats_when_they_swap_seats() {
    let mut rng = SmallRng::seed_from_u64(17);
    let mut first = Hand::new(&[2_000; 2], 0, 10, 20, &mut rng);
    first.apply(Action::RaiseTo(60)).unwrap();
    first.apply(Action::Fold).unwrap();
    let mut first_summary = summary(&first);
    first_summary.players = vec![(0, "alice".into()), (1, "bob".into())];

    let mut second = Hand::new(&[2_000; 2], 1, 10, 20, &mut rng);
    second.apply(Action::RaiseTo(60)).unwrap();
    second.apply(Action::Fold).unwrap();
    let mut second_summary = summary(&second);
    second_summary.players = vec![(0, "bob".into()), (1, "alice".into())];

    let mut models = ModelStore::default();
    models.observe(&first_summary, None);
    models.observe(&second_summary, None);

    let alice = &models.players["alice"];
    let bob = &models.players["bob"];
    assert_eq!((alice.hands, alice.vpip.opp, alice.vpip.hit, alice.pfr.hit), (2.0, 2.0, 2.0, 2.0));
    assert_eq!((bob.hands, bob.vpip.opp, bob.vpip.hit, bob.pfr.hit), (2.0, 2.0, 0.0, 0.0));
}
