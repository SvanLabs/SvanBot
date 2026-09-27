//! Fit the range-reconstruction shape constants (`oprange::RangeParams`) to real showdowns.
//!
//! Each showdown hand is one observation: given the player's full action line and their
//! learned profile, the replayed range assigns a probability to every combo, and the combo
//! they actually showed should be likely. The fit maximises the mean log-likelihood of shown
//! combos (coordinate ascent over bounded parameters) on older hands and reports the same
//! score on the newest hands, which decide whether the fit is used.

use crate::model::{HandSummary, ModelStore, Profile};
use crate::oprange::{RangeParams, board_info, replay_ranges_with};
use rayon::prelude::*;
use serde::Serialize;
use sv10_cards::cards::Card;
use sv10_cards::range::combo_index;
use sv10_engine::engine::{ActionKind, Street};
use sv10_engine::situation::{PlayerInfo, Situation};

/// One shown hand prepared for likelihood evaluation: the situation, player, profile, shown combo and board strengths.
pub struct ShowdownSample {
    sit: Situation,
    seat: usize,
    name: String,
    profile: Profile,
    combo: usize,
    /// Board strengths and percentiles on the flop, turn and river.
    boards: [std::sync::Arc<crate::oprange::BoardInfo>; 3],
}

/// One sample per shown hand of a player not in `exclude`, on hands that reached the river.
pub fn samples_from_hand(hand: &HandSummary, models: &ModelStore, exclude: &[String]) -> Vec<ShowdownSample> {
    if hand.board.len() < 5 || hand.shown.is_empty() {
        return Vec::new();
    }
    let folded: Vec<usize> = hand.history.iter().filter(|r| r.kind == ActionKind::Fold).map(|r| r.seat).collect();
    let stacks: std::collections::HashMap<usize, i64> = hand.stacks.iter().copied().collect();
    let players: Vec<PlayerInfo> = hand
        .players
        .iter()
        .map(|(seat, name)| PlayerInfo {
            seat: *seat,
            name: name.clone(),
            stack: stacks.get(seat).copied().unwrap_or(0),
            bet: 0,
            folded: folded.contains(seat),
        })
        .collect();
    let mut out = Vec::new();
    for (seat, cards) in &hand.shown {
        let Some((_, name)) = hand.players.iter().find(|(s, _)| s == seat) else { continue };
        if exclude.contains(name) || folded.contains(seat) {
            continue;
        }
        let sit = Situation {
            hero_seat: *seat,
            hole: *cards,
            board: hand.board.clone(),
            street: Street::River,
            button: hand.button,
            bb: hand.bb,
            pot: 0,
            call_amount: 0,
            current_bet_to: Some(0),
            can_check: true,
            min_raise_to: None,
            max_raise_to: None,
            players: players.clone(),
            history: hand.history.clone(),
        };
        let boards = [3usize, 4, 5].map(|n| board_info(&hand.board[..n]));
        out.push(ShowdownSample {
            sit,
            seat: *seat,
            name: name.clone(),
            profile: models.profile(name),
            combo: combo_index(cards[0], cards[1]),
            boards,
        });
    }
    out
}

fn sample_log_likelihood(s: &ShowdownSample, rp: &RangeParams) -> f64 {
    let dead = s.sit.board.iter().fold(0u64, |m, c: &Card| m | c.bit());
    let provider = |board: &[Card]| s.boards[board.len().clamp(3, 5) - 3].clone();
    let ranges = replay_ranges_with(&s.sit, vec![(s.seat, s.profile.clone())], dead, rp, &provider);
    let Some(r) = ranges.get(&s.seat) else { return (1.0f64 / 1081.0).ln() };
    let total: f64 = r.w.iter().map(|&w| w as f64).sum();
    let p = if total > 0.0 { r.w[s.combo] as f64 / total } else { 0.0 };
    p.max(1e-7).ln()
}

impl ShowdownSample {
    /// The shown player's name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The largest postflop bet or raise the shown player made, as a multiple of the pot before it
    /// (0 when none): splits the held-out likelihood by bet size (0235).
    pub fn max_bet_to_pot(&self) -> f64 {
        self.sit
            .history
            .iter()
            .filter(|r| r.seat == self.seat && r.street != Street::Preflop && crate::model::aggressive(r) && r.pot_before > 0)
            .map(|r| (r.to - r.bet_before) as f64 / r.pot_before as f64)
            .fold(0.0, f64::max)
    }

    /// Log-likelihood of the shown combo under `rp`.
    pub fn log_likelihood(&self, rp: &RangeParams) -> f64 {
        sample_log_likelihood(self, rp)
    }

    /// Whether the shown player bet or raised the river (the spots a sizing tell reads, 0223).
    pub fn bet_the_river(&self) -> bool {
        self.sit.history.iter().any(|r| r.seat == self.seat && r.street == Street::River && crate::model::aggressive(r))
    }

    /// Log-likelihood of the shown combo with the player's sizing tell set to `tell` (0223).
    pub fn log_likelihood_with_tell(&self, rp: &RangeParams, tell: f32) -> f64 {
        if self.profile.size_tell == tell {
            return sample_log_likelihood(self, rp);
        }
        let mut s = ShowdownSample {
            sit: self.sit.clone(),
            seat: self.seat,
            name: String::new(),
            profile: self.profile.clone(),
            combo: self.combo,
            boards: self.boards.clone(),
        };
        s.profile.size_tell = tell;
        sample_log_likelihood(&s, rp)
    }
}

/// The postflop line a showdown player took, most distinctive first: `check-raise` (checked, then
/// raised on the same street), `donk-lead` (bet first into the previous street's last aggressor),
/// `bet-or-raise`, `check-or-call`.
pub fn line_kind(sit: &Situation, seat: usize) -> &'static str {
    let mut kind = "check-or-call";
    let mut prev_aggressor: Option<usize> =
        sit.history.iter().rev().find(|r| r.street == Street::Preflop && crate::model::aggressive(r)).map(|r| r.seat);
    for street in [Street::Flop, Street::Turn, Street::River] {
        let mut checked = false;
        let mut street_aggressor = None;
        for r in sit.history.iter().filter(|r| r.street == street) {
            let aggr = crate::model::aggressive(r);
            if r.seat == seat {
                if r.kind == ActionKind::Check {
                    checked = true;
                } else if aggr && checked && r.to_call_before > 0 {
                    return "check-raise";
                } else if aggr && r.to_call_before == 0 && street_aggressor.is_none() && prev_aggressor.is_some_and(|p| p != seat) {
                    kind = "donk-lead";
                } else if aggr && kind == "check-or-call" {
                    kind = "bet-or-raise";
                }
            }
            if aggr {
                street_aggressor = Some(r.seat);
            }
        }
        if street_aggressor.is_some() {
            prev_aggressor = street_aggressor;
        }
    }
    kind
}

/// Probability the model gives the shown player a stronger river hand than the one shown (ties
/// count half). Calibrated ranges average 0.5; lower means the line was stronger than modelled.
pub fn stronger_share(s: &ShowdownSample, rp: &RangeParams) -> Option<f64> {
    let dead = s.sit.board.iter().fold(0u64, |m, c: &Card| m | c.bit());
    let provider = |board: &[Card]| s.boards[board.len().clamp(3, 5) - 3].clone();
    let ranges = replay_ranges_with(&s.sit, vec![(s.seat, s.profile.clone())], dead, rp, &provider);
    let r = ranges.get(&s.seat)?;
    let strength = &s.boards[2].strength;
    let mine = strength[s.combo];
    let (mut total, mut above) = (0.0f64, 0.0f64);
    for (i, &w) in r.w.iter().enumerate() {
        let w = w as f64;
        total += w;
        if strength[i] > mine {
            above += w;
        } else if strength[i] == mine && i != s.combo {
            above += 0.5 * w;
        }
    }
    (total > 0.0).then(|| above / total)
}

/// Calibration by line: `(line, samples, mean stronger share, standard error)`.
pub fn line_calibration(samples: &[ShowdownSample], rp: &RangeParams) -> Vec<(&'static str, usize, f64, f64)> {
    let values: Vec<(&'static str, f64)> =
        samples.par_iter().filter_map(|s| Some((line_kind(&s.sit, s.seat), stronger_share(s, rp)?))).collect();
    ["check-raise", "donk-lead", "bet-or-raise", "check-or-call"]
        .into_iter()
        .map(|kind| {
            let v: Vec<f64> = values.iter().filter(|(k, _)| *k == kind).map(|(_, x)| *x).collect();
            let n = v.len();
            let mean = v.iter().sum::<f64>() / n.max(1) as f64;
            let var = v.iter().map(|x| (x - mean).powi(2)).sum::<f64>() / (n.max(2) - 1) as f64;
            (kind, n, mean, (var / n.max(1) as f64).sqrt())
        })
        .collect()
}

/// Mean log-likelihood of the shown combos.
pub fn mean_log_likelihood(samples: &[ShowdownSample], rp: &RangeParams) -> f64 {
    if samples.is_empty() {
        return 0.0;
    }
    samples.par_iter().map(|s| sample_log_likelihood(s, rp)).sum::<f64>() / samples.len() as f64
}

/// Result of a range-parameter fit, with before/after likelihoods on training and held-out showdowns.
#[derive(Clone, Debug, Serialize)]
pub struct FitReport {
    /// Fitted parameters.
    pub params: RangeParams,
    /// Samples fitted on (older hands).
    pub train_samples: usize,
    /// Held-out samples (newest hands).
    pub val_samples: usize,
    /// Mean log-likelihood of the starting parameters on the training samples.
    pub default_train_ll: f64,
    /// Mean log-likelihood of the fit on the training samples.
    pub train_ll: f64,
    /// Mean log-likelihood of the starting parameters on held-out samples.
    pub default_val_ll: f64,
    /// Mean log-likelihood of the fit on held-out samples (must beat `default_val_ll` to be used).
    pub val_ll: f64,
    /// Uniform-guess log-likelihood (every live combo equally likely), for scale.
    pub uniform_ll: f64,
    /// Likelihood evaluations spent.
    pub evaluations: usize,
}

/// Coordinate ascent from `start` within each parameter's bounds.
pub fn fit(train: &[ShowdownSample], val: &[ShowdownSample], start: RangeParams, passes: usize) -> FitReport {
    fit_frozen(train, val, start, passes, &[])
}

/// [`fit`] with the named fields held at their `start` values (A/B tests of a new shape term:
/// fit once with it frozen at its neutral value, once free, and compare validation likelihood).
pub fn fit_frozen(train: &[ShowdownSample], val: &[ShowdownSample], start: RangeParams, passes: usize, frozen: &[&str]) -> FitReport {
    let default_train_ll = mean_log_likelihood(train, &start);
    let default_val_ll = mean_log_likelihood(val, &start);
    let mut best = start;
    let mut best_ll = default_train_ll;
    let mut step = 0.25f32;
    let mut evaluations = 2;
    let n = best.clone().fields_mut().len();
    for _ in 0..passes {
        let mut improved = false;
        for k in 0..n {
            for dir in [1.0f32, -1.0] {
                let mut cand = best;
                {
                    let mut fields = cand.fields_mut();
                    let (name, v, lo, hi) = &mut fields[k];
                    if frozen.contains(name) {
                        continue;
                    }
                    let span = *hi - *lo;
                    **v = (**v + dir * step * span * 0.25).clamp(*lo, *hi);
                }
                if cand == best {
                    continue;
                }
                let ll = mean_log_likelihood(train, &cand);
                evaluations += 1;
                if ll > best_ll + 1e-5 {
                    best = cand;
                    best_ll = ll;
                    improved = true;
                    break;
                }
            }
        }
        if !improved {
            step *= 0.5;
            if step < 0.02 {
                break;
            }
        }
    }
    let val_ll = mean_log_likelihood(val, &best);
    FitReport {
        params: best,
        train_samples: train.len(),
        val_samples: val.len(),
        default_train_ll,
        train_ll: best_ll,
        default_val_ll,
        val_ll,
        uniform_ll: (1.0f64 / 1081.0).ln(),
        evaluations,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::HandSummary;
    use sv10_cards::range::combo_index;
    use sv10_engine::engine::ActionRecord;
    use sv10_rng::SeedableRng;
    use sv10_rng::rngs::SmallRng;
    use sv10_rng::seq::SliceRandom;

    fn rec(seat: usize, street: Street, kind: ActionKind, to: i64, pot_before: i64, to_call_before: i64, bet_before: i64) -> ActionRecord {
        ActionRecord {
            seat,
            street,
            kind,
            to,
            pot_before,
            to_call_before,
            bet_before,
            full_raise: kind == ActionKind::Raise,
            think_ms: None,
            street_open: false,
        }
    }

    /// A limped heads-up hand checked to the river, where the villain (seat 1) bets the pot first to
    /// act, timed at `think_ms`, and is called and shown down.
    fn hand(board: &[Card], villain: [Card; 2], hero: [Card; 2], think_ms: u32) -> HandSummary {
        let mut history =
            vec![rec(0, Street::Preflop, ActionKind::Call, 20, 30, 10, 10), rec(1, Street::Preflop, ActionKind::Check, 0, 40, 0, 20)];
        for street in [Street::Flop, Street::Turn] {
            history.push(rec(1, street, ActionKind::Check, 0, 40, 0, 0));
            history.push(rec(0, street, ActionKind::Check, 0, 40, 0, 0));
        }
        let mut bet = rec(1, Street::River, ActionKind::Raise, 40, 40, 0, 0);
        bet.think_ms = Some(think_ms);
        bet.street_open = true;
        history.push(bet);
        history.push(rec(0, Street::River, ActionKind::Call, 40, 80, 40, 0));
        HandSummary {
            players: vec![(0, "hero".into()), (1, "villain".into())],
            button: 0,
            bb: 20,
            history,
            board: board.to_vec(),
            shown: vec![(1, villain), (0, hero)],
            stacks: vec![(0, 2_000), (1, 2_000)],
        }
    }

    /// LESSONS 28 for the timing tell (0234): no stored hand carries think times yet, so the fit's
    /// pass path is exercised here. A villain who bets the river slowly with strong hands and fast
    /// with the rest must yield a positive `think_exp` that raises held-out likelihood, while the
    /// same fit on untimed hands leaves the term at 0.
    #[test]
    fn a_real_timing_tell_is_fitted_and_wins_held_out() {
        let mut rng = SmallRng::seed_from_u64(234);
        let mut deck: Vec<Card> = (0..52u8).map(Card).collect();
        let mut hands = Vec::new();
        for _ in 0..600 {
            deck.shuffle(&mut rng);
            let board = deck[..5].to_vec();
            let (villain, hero) = ([deck[5], deck[6]], [deck[7], deck[8]]);
            let strong = board_info(&board).pct[combo_index(villain[0], villain[1])] < 0.25;
            hands.push(hand(&board, villain, hero, if strong { 2_400 } else { 40 }));
        }
        let mut models = ModelStore::default();
        for h in &hands {
            models.observe(h, Some("hero"));
        }
        assert!(models.profile("villain").think_base[1] > 0.0, "the villain's typical think time is known");
        let exclude = vec!["hero".to_string()];
        let mut samples: Vec<ShowdownSample> = hands.iter().flat_map(|h| samples_from_hand(h, &models, &exclude)).collect();
        assert_eq!(samples.len(), 600);
        let val = samples.split_off(450);
        let mut names = RangeParams::DEFAULT;
        let others: Vec<&str> = names.fields_mut().into_iter().map(|f| f.0).filter(|n| *n != "think_exp").collect();
        let fit = fit_frozen(&samples, &val, RangeParams::DEFAULT, 4, &others);
        assert!(fit.params.think_exp > 0.2, "{:?}", fit.params.think_exp);
        assert!(fit.val_ll > fit.default_val_ll + 0.02, "held-out {} -> {}", fit.default_val_ll, fit.val_ll);

        // Untimed hands: the term has nothing to fit and stays at 0.
        let untimed: Vec<HandSummary> = hands
            .iter()
            .map(|h| HandSummary { history: h.history.iter().map(|r| ActionRecord { think_ms: None, ..r.clone() }).collect(), ..h.clone() })
            .collect();
        let mut models = ModelStore::default();
        for h in &untimed {
            models.observe(h, Some("hero"));
        }
        let mut samples: Vec<ShowdownSample> = untimed.iter().flat_map(|h| samples_from_hand(h, &models, &exclude)).collect();
        let val = samples.split_off(450);
        let fit = fit_frozen(&samples, &val, RangeParams::DEFAULT, 4, &others);
        assert_eq!(fit.params.think_exp, 0.0);
        assert!((fit.val_ll - fit.default_val_ll).abs() < 1e-12);
    }
}
