//! Per-player statistics observed in one hand (preflop and postflop opportunities and actions).

use super::*;

pub(super) fn extract(hand: &HandSummary) -> HashMap<usize, PlayerStats> {
    let mut out: HashMap<usize, PlayerStats> = HashMap::new();
    let seats: Vec<usize> = hand.players.iter().map(|(s, _)| *s).collect();
    let group = |seat: usize| position_group(sv10_engine::situation::position_of(&seats, hand.button, seat));
    for (seat, _) in &hand.players {
        out.entry(*seat).or_default().hands = 1.0;
    }
    // Think times by the server's clock (0234), every street.
    for rec in &hand.history {
        if let Some(ms) = rec.think_ms {
            out.entry(rec.seat).or_default().think[think_context(rec)].add(ms);
        }
    }
    // Preflop.
    let mut raises = 0;
    let mut first_raiser: Option<usize> = None;
    let mut last_raiser: Option<usize> = None;
    let mut acted: HashMap<usize, bool> = HashMap::new();
    let mut vpip: HashMap<usize, bool> = HashMap::new();
    let mut pfr: HashMap<usize, bool> = HashMap::new();
    let mut faced_3bet_pending: Option<usize> = None;
    let mut three_bettor: Option<usize> = None;
    let mut faced_4bet_pending: Option<usize> = None;
    let mut saw_flop: Vec<usize> = Vec::new();
    for rec in hand.history.iter().filter(|r| r.street == Street::Preflop) {
        let s = out.entry(rec.seat).or_default();
        let aggr = aggressive(rec);
        let voluntary = matches!(rec.kind, ActionKind::Call | ActionKind::Raise | ActionKind::AllIn);
        if voluntary {
            vpip.insert(rec.seat, true);
        }
        if aggr {
            pfr.insert(rec.seat, true);
        }
        if faced_3bet_pending == Some(rec.seat) {
            s.fold_to_3bet.add(rec.kind == ActionKind::Fold);
            s.four_bet.add(aggr);
            faced_3bet_pending = None;
        }
        if faced_4bet_pending == Some(rec.seat) {
            s.fold_to_4bet.add(rec.kind == ActionKind::Fold);
            faced_4bet_pending = None;
        }
        if !acted.contains_key(&rec.seat) {
            if raises == 0 {
                s.open_pos[group(rec.seat)].add(aggr);
                s.open_raise.add(aggr);
                s.limp.add(!aggr && voluntary);
            } else if raises == 1 && first_raiser != Some(rec.seat) {
                s.three_bet.add(aggr);
                s.call_open.add(!aggr && voluntary);
            }
        }
        acted.insert(rec.seat, true);
        if aggr {
            raises += 1;
            if raises == 1 {
                first_raiser = Some(rec.seat);
            } else if raises == 2 {
                faced_3bet_pending = first_raiser;
                three_bettor = Some(rec.seat);
            } else if raises == 3 {
                // A cold 4-bet before the opener answered the 3-bet: the opener now faces a 4-bet,
                // not a 3-bet, so its next action is not a fold-to-3-bet or 4-bet sample (0096 d).
                if first_raiser != Some(rec.seat) {
                    faced_3bet_pending = None;
                }
                faced_4bet_pending = three_bettor;
            }
            last_raiser = Some(rec.seat);
        }
    }
    for (seat, _) in &hand.players {
        let g = group(*seat);
        let s = out.entry(*seat).or_default();
        s.vpip.add(*vpip.get(seat).unwrap_or(&false));
        s.pfr.add(*pfr.get(seat).unwrap_or(&false));
        s.vpip_pos[g].add(*vpip.get(seat).unwrap_or(&false));
        s.pfr_pos[g].add(*pfr.get(seat).unwrap_or(&false));
    }
    let folded_pre: Vec<usize> =
        hand.history.iter().filter(|r| r.street == Street::Preflop && r.kind == ActionKind::Fold).map(|r| r.seat).collect();
    let reached_flop = hand.history.iter().any(|r| r.street != Street::Preflop) || hand.board.len() >= 3;
    if reached_flop {
        for (seat, _) in &hand.players {
            if !folded_pre.contains(seat) {
                saw_flop.push(*seat);
            }
        }
    }
    // Postflop.
    let mut last_river_aggressor: Option<usize> = None;
    for street in [Street::Flop, Street::Turn, Street::River] {
        let st = street.index() - 1;
        let mut bet_seen = false;
        let mut first_bettor: Option<usize> = None;
        for rec in hand.history.iter().filter(|r| r.street == street) {
            let s = out.entry(rec.seat).or_default();
            let aggr = aggressive(rec);
            if rec.to_call_before == 0 {
                s.bet_first[st].add(aggr);
                if street == Street::Flop && !bet_seen && last_raiser == Some(rec.seat) {
                    s.cbet.add(aggr);
                }
            } else {
                s.fold_vs_bet[st].add(rec.kind == ActionKind::Fold);
                s.raise_vs_bet[st].add(aggr);
                let base = (rec.pot_before - rec.to_call_before).max(1) as f64;
                s.fold_vs_size[size_bucket(rec.to_call_before as f64 / base)].add(rec.kind == ActionKind::Fold);
                if street == Street::Flop && first_bettor.is_some() && first_bettor == last_raiser && first_bettor != Some(rec.seat) {
                    s.fold_to_cbet.add(rec.kind == ActionKind::Fold);
                }
            }
            if aggr {
                if !bet_seen {
                    first_bettor = Some(rec.seat);
                }
                bet_seen = true;
                if street == Street::River {
                    last_river_aggressor = Some(rec.seat);
                }
            }
        }
    }
    let folded: Vec<usize> = hand.history.iter().filter(|r| r.kind == ActionKind::Fold).map(|r| r.seat).collect();
    let showdown = hand.players.iter().filter(|(s, _)| seats.contains(s) && !folded.contains(s)).count() > 1;
    for seat in &saw_flop {
        let s = out.entry(*seat).or_default();
        let went = showdown && !folded.contains(seat);
        s.wtsd.add(went);
        if went {
            s.showdowns += 1.0;
        }
    }
    if showdown && hand.board.len() == 5 {
        let board_mask = hand.board.iter().fold(0u64, |m, c| m | c.bit());
        let values: Vec<(usize, u32)> = hand
            .shown
            .iter()
            .filter(|(s, _)| seats.contains(s) && !folded.contains(s))
            .map(|(s, c)| (*s, sv10_cards::eval::eval(board_mask | c[0].bit() | c[1].bit())))
            .collect();
        if values.len() >= 2
            && let Some(contributions) = crate::flow::reconstructed_contributions(hand)
        {
            for (seat, value) in &values {
                let covered = contributions[seat];
                if covered > 0 {
                    let won = values.iter().all(|(other, rank)| contributions[other] < covered || rank <= value);
                    out.entry(*seat).or_default().won_showdown.add(won);
                }
            }
        }
        if let Some(agg) = last_river_aggressor
            && let Some((_, cards)) = hand.shown.iter().find(|(s, _)| *s == agg)
        {
            let board: [Card; 5] = hand.board.clone().try_into().unwrap();
            let strength = river_equity_exact(*cards, &board, &Range::full());
            out.entry(agg).or_default().river_bluff.add(strength < 0.5);
        }
    }
    out
}
