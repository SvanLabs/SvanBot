use super::*;

#[test]
fn straight_texture_counts_distinct_ranks_and_the_wheel() {
    for (names, expected) in
        [(["Ah", "Kd", "Qc", "Js", "2h"], 0.8), (["Ah", "2d", "3c", "4s", "5h"], 1.0), (["Th", "Jd", "Qc", "Ks", "2h"], 0.8)]
    {
        let board = names.map(|name| Card::parse(name).unwrap());
        assert_eq!(texture(&board)[2], expected);
    }
}

#[test]
fn corrected_layout_preserves_every_legacy_input_and_has_its_own_width() {
    let profile = ModelStore::default().profile("x");
    for (names, old, new, wheel) in [(["Ah", "Kd", "Qc", "Js", "2h"], 1.0, 0.8, 0.4), (["Ah", "2d", "3c", "4s", "5h"], 0.8, 1.0, 1.0)] {
        let board = names.map(|name| Card::parse(name).unwrap());
        let ctx = ResponseContext {
            street: Street::River,
            to_call: 20,
            pot_before: 100,
            stack_behind: 1000,
            bb: 20,
            active_players: 2,
            in_position: true,
            preflop_raises: 1,
            was_preflop_aggressor: false,
            facing_cbet: false,
            prior_postflop_calls: 2,
            board: &board,
            profile: &profile,
        };
        let a = features_for(&ctx, ResponseFeatureSet::Incumbent37);
        let b = features_for(&ctx, ResponseFeatureSet::PriorStreetCalls38);
        let c = features_for(&ctx, ResponseFeatureSet::StraightTexture39);
        assert_eq!((a.len(), b.len(), c.len()), (37, 38, 39));
        assert_eq!(&b[..37], &a);
        assert_eq!(a[15], old);
        assert_eq!(c[15], new);
        assert_eq!(c[38], wheel);
        for i in 0..38 {
            if i != 15 {
                assert_eq!(b[i].to_bits(), c[i].to_bits());
            }
        }
        assert_eq!(ResponseFeatureSet::for_inputs(39), Some(ResponseFeatureSet::StraightTexture39));
    }
}

#[test]
fn straight_connectivity_matches_distinct_window_masks_for_every_rank_subset() {
    let windows: Vec<u16> = (0..=8).map(|lo| 0b11111 << lo).chain([0b1000000001111]).collect();
    for mask in 0..8192u16 {
        let board: Vec<Card> = (0..13).filter(|rank| mask & (1 << rank) != 0).map(Card).collect();
        let best = windows.iter().map(|window| (mask & window).count_ones()).max().unwrap();
        assert_eq!(texture(&board)[2], best as f32 / 5.0);
    }
}
