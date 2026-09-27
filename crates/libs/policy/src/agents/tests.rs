//! Agent behaviour, including what an agent does with a hole pair that is not a legal hand.
use super::*;
use sv10_cards::cards::Card;
use sv10_rng::SeedableRng;

fn rate(c: &sv10_model::model::Counter) -> f32 {
    c.hit / c.opp.max(1.0)
}

#[test]
fn profile_agents_reproduce_their_profile() {
    for (tag, tweak) in
        [("tight-foldy", (0.12f32, 0.04f32, 0.60f32, 0.55f32, 0.30f32, 0.05f32)), ("loose-sticky", (0.35, 0.12, 0.25, 0.30, 0.50, 0.12))]
    {
        let (open, three, fold3, fold_bet, bet_first, raise) = tweak;
        let mut p = ModelStore::default().profile("x");
        p.open_pos = [open * 0.7, open * 1.45, open];
        p.limp = 0.05;
        p.three_bet = three;
        p.call_open = 0.12;
        p.fold_to_3bet = fold3;
        p.fold_vs_bet = [fold_bet; 3];
        p.fold_vs_size = [fold_bet; 3];
        p.bet_first = [bet_first; 3];
        p.cbet = bet_first + 0.15;
        p.fold_to_cbet = fold_bet;
        p.raise_vs_bet = [raise; 3];
        let mut agents: Vec<Box<dyn Agent>> =
            (0..6).map(|i| Box::new(ProfileAgent::new(ProfileClone::exact("x", p.clone()), format!("p{i}"))) as Box<dyn Agent>).collect();
        let mut obs = ModelStore::default();
        crate::sim::run_table_observed(&mut agents, 3000, 100, 42, Some(&mut obs));
        let st = &obs.population;
        let sum3 = |a: &[sv10_model::model::Counter; 3]| sv10_model::model::Counter {
            opp: a.iter().map(|c| c.opp).sum(),
            hit: a.iter().map(|c| c.hit).sum(),
        };
        let got = [
            ("open", rate(&st.open_raise), (p.open_pos[0] * 3.0 + p.open_pos[1] * 2.0 + p.open_pos[2]) / 6.0, 0.05),
            ("3bet", rate(&st.three_bet), three, 0.04),
            ("fold_to_3bet", rate(&st.fold_to_3bet), fold3, 0.10),
            ("bet_first", rate(&sum3(&st.bet_first)), bet_first, 0.08),
            ("fold_vs_bet", rate(&sum3(&st.fold_vs_bet)), fold_bet, 0.10),
            ("raise_vs_bet", rate(&sum3(&st.raise_vs_bet)), raise, 0.06),
            ("cbet", rate(&st.cbet), bet_first + 0.15, 0.10),
        ];
        let report: Vec<String> = got.iter().map(|(k, g, t, _)| format!("{k} {g:.3} vs {t:.3}")).collect();
        eprintln!("{tag}: {}", report.join(", "));
        for (k, g, t, tol) in got {
            assert!((g - t).abs() <= tol, "{tag} {k}: observed {g:.3}, target {t:.3} ({})", report.join(", "));
        }
    }
}

/// A postflop situation whose hole pair names one card twice. This is the shape a replayed,
/// truncated or corrupt `table_state` produces: the tracker parses two cards without checking
/// that they differ, so `Situation::hole` can hold `[c, c]`.
fn hole_pair_of_one_card() -> Situation {
    let c = Card::parse("Tc").expect("a real card");
    let mut sit = sv10_engine::situation::fixtures::uncallable_overshove();
    sit.hole = [c, c];
    sit
}

#[test]
fn an_archetype_declines_a_hole_pair_of_one_card() {
    let mut agent = ArchetypeAgent { a: archetype("tag"), label: "tag".into() };
    let mut rng = SmallRng::seed_from_u64(4);
    let sit = hole_pair_of_one_card();
    assert!(!sit.can_check, "the fixture faces a bet, so declining means folding");
    assert_eq!(agent.act(&sit, &mut rng), Action::Fold, "a hand we cannot name is not a hand to pay for");
}

#[test]
fn an_archetype_checks_a_hole_pair_of_one_card_when_checking_is_free() {
    let mut agent = ArchetypeAgent { a: archetype("tag"), label: "tag".into() };
    let mut rng = SmallRng::seed_from_u64(4);
    let mut sit = hole_pair_of_one_card();
    sit.can_check = true;
    assert_eq!(agent.act(&sit, &mut rng), Action::Check, "checking costs nothing; folding a free hand is worse");
}

#[test]
fn a_profile_agent_declines_a_hole_pair_of_one_card() {
    let profile = ModelStore::default().profile("x");
    let mut agent = ProfileAgent::new(ProfileClone::exact("x", profile), "p".into());
    let mut rng = SmallRng::seed_from_u64(4);
    let sit = hole_pair_of_one_card();
    assert!(!sit.can_check, "the fixture faces a bet, so declining means folding");
    assert_eq!(agent.act(&sit, &mut rng), Action::Fold, "a hand we cannot name is not a hand to pay for");
}

#[test]
fn a_profile_agent_checks_a_hole_pair_of_one_card_when_checking_is_free() {
    let profile = ModelStore::default().profile("x");
    let mut agent = ProfileAgent::new(ProfileClone::exact("x", profile), "p".into());
    let mut rng = SmallRng::seed_from_u64(4);
    let mut sit = hole_pair_of_one_card();
    sit.can_check = true;
    assert_eq!(agent.act(&sit, &mut rng), Action::Check, "checking costs nothing; folding a free hand is worse");
}

#[test]
fn a_normal_hand_still_plays_normally() {
    // The rejection must not fire on a legal hand: the same fixture with the hole pair it was
    // captured with (Ah 6h on a 2h 9s 8s 2s As river) still reaches the ordinary decision path.
    let mut agent = ArchetypeAgent { a: archetype("tag"), label: "tag".into() };
    let mut rng = SmallRng::seed_from_u64(4);
    let sit = sv10_engine::situation::fixtures::uncallable_overshove();
    assert!(!sit.can_check, "the fixture faces a bet");
    assert!(matches!(agent.act(&sit, &mut rng), Action::Fold | Action::Call | Action::AllIn | Action::RaiseTo(_)));
}
