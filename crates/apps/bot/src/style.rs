//! One style classifier for every surface that names an opponent's play.
//!
//! The dashboard's scout view and the `monitor`'s OPPONENTS line read the same `models.v1` stats and
//! must never call one player two things (0246: VPIP 34 read "tight" in one place and "loose" in the
//! other when the monitor carried its own copy of the thresholds).

use sv10_core::model::{Counter, PlayerStats};

/// A counter's rate, or `None` with no observations to take it over.
pub fn rate(c: &Counter) -> Option<f32> {
    if c.opp >= 1.0 { Some(c.hit / c.opp) } else { None }
}

/// The three per-street counters of one kind summed.
pub fn sum3(cs: &[Counter; 3]) -> Counter {
    Counter { opp: cs.iter().map(|c| c.opp).sum(), hit: cs.iter().map(|c| c.hit).sum() }
}

/// Postflop aggression frequency: bets and raises over bets, raises and calls.
pub fn aggression_counter(st: &PlayerStats) -> Counter {
    let aggr: f32 = st.bet_first.iter().map(|c| c.hit).sum::<f32>() + st.raise_vs_bet.iter().map(|c| c.hit).sum::<f32>();
    let calls: f32 = (0..3).map(|i| (st.fold_vs_bet[i].opp - st.fold_vs_bet[i].hit - st.raise_vs_bet[i].hit).max(0.0)).sum();
    Counter { opp: aggr + calls, hit: aggr }
}

/// Style label and the advice that goes with it, from a player's observed stats.
pub fn style_of(st: &PlayerStats) -> (String, String) {
    if st.hands < 20.0 {
        return ("Sampling".into(), format!("{} hands observed; decisions still lean on the population prior.", st.hands as i64));
    }
    let vpip = rate(&st.vpip).unwrap_or(0.3);
    let pfr = rate(&st.pfr).unwrap_or(0.15);
    let fold = if sum3(&st.fold_vs_bet).opp >= 12.0 { rate(&sum3(&st.fold_vs_bet)) } else { None };
    let afq = {
        let c = aggression_counter(st);
        if c.opp >= 15.0 { rate(&c) } else { None }
    };
    let wtsd = if st.wtsd.opp >= 12.0 { rate(&st.wtsd) } else { None };
    let passive_gap = vpip - pfr;
    let (style, mut advice): (&str, String) = if vpip >= 0.45 && pfr >= 0.28 && afq.unwrap_or(0.5) >= 0.45 {
        ("Maniac", "Plays most hands aggressively. Call down lighter, trap strong hands, avoid thin bluffs.".into())
    } else if vpip >= 0.38 && passive_gap >= 0.2 && fold.map(|f| f <= 0.35).unwrap_or(true) {
        ("Calling station", "Enters loose and calls. Value bet thinner and larger; bluff almost never.".into())
    } else if fold.map(|f| f >= 0.6).unwrap_or(false) {
        ("Overfolder", "Gives up to bets too often. Bet more often with weak hands; smaller bluffs work.".into())
    } else if fold.map(|f| f <= 0.2).unwrap_or(false) {
        ("Sticky", "Rarely folds once invested. Stop bluffing; bet strong hands for value every street.".into())
    } else if vpip <= 0.16 && pfr <= 0.12 {
        ("Nit", "Very tight. Steal blinds relentlessly; respect their raises and fold marginal hands.".into())
    } else if vpip <= 0.26 && pfr >= 0.16 && afq.unwrap_or(0.4) >= 0.4 {
        ("Tight-aggressive", "Solid regular. Stay close to baseline; avoid bloating pots out of position.".into())
    } else if vpip > 0.26 && pfr >= 0.2 {
        ("Loose-aggressive", "Wide and aggressive. Defend wider in position and 3-bet their opens for value.".into())
    } else if passive_gap >= 0.15 {
        ("Loose-passive", "Limps and calls. Isolate their limps and value bet; their raises mean strength.".into())
    } else {
        ("Balanced", "No large leak yet; stick close to the baseline.".into())
    };
    if let (Some(w), Some(won)) = (wtsd, rate(&st.won_showdown).filter(|_| st.won_showdown.opp >= 10.0)) {
        advice.push_str(&format!(" Goes to showdown {:.0}% of flops seen and wins {:.0}% there.", w * 100.0, won * 100.0));
    }
    (style.to_string(), advice)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stats(hands: f32, vpip: f32, pfr: f32, fold: f32) -> PlayerStats {
        let c = |rate: f32, n: f32| Counter { opp: n, hit: rate * n };
        PlayerStats {
            hands,
            vpip: c(vpip, hands),
            pfr: c(pfr, hands),
            fold_vs_bet: [c(fold, 12.0), Counter::default(), Counter::default()],
            ..Default::default()
        }
    }

    #[test]
    fn labels_match_the_thresholds_the_monitor_printed_before_the_port() {
        assert_eq!(style_of(&stats(5.0, 0.9, 0.9, 0.0)).0, "Sampling");
        assert_eq!(style_of(&stats(200.0, 0.55, 0.30, 0.2)).0, "Maniac");
        assert_eq!(style_of(&stats(200.0, 0.50, 0.10, 0.2)).0, "Calling station");
        assert_eq!(style_of(&stats(200.0, 0.30, 0.20, 0.7)).0, "Overfolder");
        assert_eq!(style_of(&stats(200.0, 0.30, 0.20, 0.1)).0, "Sticky");
        assert_eq!(style_of(&stats(200.0, 0.10, 0.08, 0.45)).0, "Nit");
        assert_eq!(style_of(&stats(200.0, 0.24, 0.19, 0.45)).0, "Tight-aggressive");
        assert_eq!(style_of(&stats(200.0, 0.34, 0.24, 0.45)).0, "Loose-aggressive");
        assert_eq!(style_of(&stats(200.0, 0.30, 0.10, 0.45)).0, "Loose-passive");
        assert_eq!(style_of(&stats(200.0, 0.28, 0.14, 0.45)).0, "Balanced");
    }
}
