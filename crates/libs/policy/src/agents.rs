//! Agents for simulation: the policy under test and parametric archetypes
//! modeled on the kinds of bots in the OpenPoker pool (calling stations,
//! maniacs, nits, TAG/LAG regulars, template-style rocks).

use crate::policy::Params;
use sv10_cards::range::combo_index;
use sv10_engine::engine::{Action, Street};
use sv10_engine::situation::Situation;
use sv10_equity::preflop;
use sv10_model::model::{HandSummary, ModelStore, aggressive};
use sv10_model::oprange::board_strengths;
use sv10_rng::RngExt;
use sv10_rng::rngs::SmallRng;

/// A seat in a simulated table.
pub trait Agent: Send {
    /// Label the table tallies results under.
    fn name(&self) -> &str;
    /// Choose an action for `sit` (must be legal).
    fn act(&mut self, sit: &Situation, rng: &mut SmallRng) -> Action;
    /// See a finished hand (learning agents update their models).
    fn observe(&mut self, _hand: &HandSummary) {}
}

/// A parametric rule-based opponent style.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct Archetype {
    /// Style or clone name.
    pub name: String,
    /// Target share of hands played preflop.
    pub vpip: f64,
    /// Target share of hands raised preflop.
    pub pfr: f64,
    /// Postflop bet/raise tendency with decent hands.
    pub aggression: f64,
    /// Chance to bet or raise without a decent hand when checked to.
    pub bluff: f64,
    /// Added to pot odds when deciding to call (negative = calls light).
    pub call_margin: f64,
    /// Bet size as a pot fraction postflop; preflop opens are `2.5 + sizing` big blinds.
    pub sizing: f64,
}

/// A named style from [`ARCHETYPES`]. Panics on an unknown name.
pub fn archetype(kind: &str) -> Archetype {
    let a = |vpip, pfr, aggression, bluff, call_margin, sizing| Archetype {
        name: kind.to_string(),
        vpip,
        pfr,
        aggression,
        bluff,
        call_margin,
        sizing,
    };
    match kind {
        "station" => a(0.55, 0.06, 0.15, 0.02, -0.18, 0.5),
        "maniac" => a(0.65, 0.45, 0.70, 0.35, -0.06, 1.0),
        "nit" => a(0.13, 0.10, 0.40, 0.03, 0.12, 0.6),
        "tag" => a(0.22, 0.18, 0.50, 0.12, 0.02, 0.66),
        "lag" => a(0.32, 0.26, 0.60, 0.20, 0.0, 0.75),
        "rock" => a(0.17, 0.08, 0.30, 0.05, 0.08, 0.5),
        _ => panic!("unknown archetype {kind}"),
    }
}

/// Every built-in style name.
pub const ARCHETYPES: [&str; 6] = ["station", "maniac", "nit", "tag", "lag", "rock"];

/// Something paired evaluation can seat as an opponent.
pub trait OpponentSpec: Sync {
    /// A fresh agent for seat index `i` (labelled with the opponent's name when it has one).
    fn agent(&self, i: usize) -> Box<dyn Agent>;
}

impl OpponentSpec for Archetype {
    fn agent(&self, i: usize) -> Box<dyn Agent> {
        let label = if self.name.is_empty() { format!("opp{i}") } else { self.name.clone() };
        Box::new(ArchetypeAgent { a: self.clone(), label })
    }
}

impl OpponentSpec for ProfileClone {
    fn agent(&self, i: usize) -> Box<dyn Agent> {
        let label = if self.name.is_empty() { format!("opp{i}") } else { self.name.clone() };
        Box::new(ProfileAgent::new(self.clone(), label))
    }
}

/// An agent that plays an [`Archetype`].
pub struct ArchetypeAgent {
    /// Its style parameters.
    pub a: Archetype,
    /// Tally label.
    pub label: String,
}

fn raise_or_call(sit: &Situation, to: i64) -> Action {
    match (sit.min_raise_to, sit.max_raise_to) {
        (Some(lo), Some(hi)) => {
            let t = to.clamp(lo, hi);
            if t == hi { Action::AllIn } else { Action::RaiseTo(t) }
        }
        _ => {
            if sit.can_check {
                Action::Check
            } else {
                Action::Call
            }
        }
    }
}

impl Agent for ArchetypeAgent {
    fn name(&self) -> &str {
        &self.label
    }
    fn act(&mut self, sit: &Situation, rng: &mut SmallRng) -> Action {
        let a = &self.a;
        let pot = sit.pot as f64;
        let call = sit.call_amount as f64;
        let passive = if sit.can_check { Action::Check } else { Action::Fold };
        if sit.street == Street::Preflop {
            let pct = preflop::percentile(sit.hole[0], sit.hole[1]) as f64;
            let raises = sit.history.iter().filter(|r| r.street == Street::Preflop && aggressive(r)).count();
            let cur = sit.current_bet();
            return match raises {
                0 => {
                    if pct < a.pfr {
                        raise_or_call(sit, (sit.bb as f64 * (2.5 + a.sizing)) as i64)
                    } else if pct < a.vpip {
                        if sit.can_check { Action::Check } else { Action::Call }
                    } else {
                        passive
                    }
                }
                1 => {
                    if pct < a.pfr * 0.3 {
                        raise_or_call(sit, cur * 3)
                    } else if pct < a.vpip * 0.65 {
                        Action::Call
                    } else {
                        passive
                    }
                }
                _ => {
                    if pct < a.pfr * 0.08 {
                        Action::AllIn
                    } else if pct < a.vpip * 0.25 {
                        Action::Call
                    } else {
                        passive
                    }
                }
            };
        }
        let s = board_strengths(&sit.board)[combo_index(sit.hole[0], sit.hole[1])] as f64;
        if sit.can_check {
            let strong = s > 0.8 - a.aggression * 0.25;
            if (strong && rng.random::<f64>() < 0.4 + a.aggression * 0.6) || rng.random::<f64>() < a.bluff {
                return raise_or_call(sit, (pot * a.sizing).max(sit.bb as f64) as i64);
            }
            return Action::Check;
        }
        let po = call / (pot + call);
        if s > 0.9 && rng.random::<f64>() < a.aggression {
            return raise_or_call(sit, sit.current_bet() * 3);
        }
        if rng.random::<f64>() < a.bluff * 0.25 {
            return raise_or_call(sit, sit.current_bet() * 3);
        }
        // Equity vs random overstates vs a betting range; strength threshold blends pot odds.
        if s > (po + 0.25 + a.call_margin).clamp(0.05, 0.95) { Action::Call } else { Action::Fold }
    }
}

/// An agent that plays the live decision policy.
pub struct PolicyAgent {
    /// Tally label.
    pub label: String,
    /// Opponent models it decides with.
    pub models: ModelStore,
    /// Strategy parameters (champion or challenger).
    pub params: Params,
    /// Whether observed hands update `models`.
    pub learn: bool,
    /// Neural response model, when active.
    pub nn: Option<std::sync::Arc<sv10_nn::nn::Mlp>>,
}

impl Agent for PolicyAgent {
    fn name(&self) -> &str {
        &self.label
    }
    fn act(&mut self, sit: &Situation, rng: &mut SmallRng) -> Action {
        crate::policy::decide_with(sit, &self.models, &self.params, self.nn.as_deref(), rng).action
    }
    fn observe(&mut self, hand: &HandSummary) {
        if self.learn {
            let me = self.label.clone();
            self.models.observe(hand, Some(&me));
        }
    }
}

/// Observable rates a clone is calibrated to match.
#[derive(Clone, Debug)]
pub struct TargetRates {
    /// Share of hands played preflop.
    pub vpip: f64,
    /// Share of hands raised preflop.
    pub pfr: f64,
    /// Bet-when-checked-to rate, averaged over streets.
    pub bet_first: f64,
    /// Fold-to-bet rate, averaged over streets.
    pub fold_vs_bet: f64,
}

impl TargetRates {
    /// Targets from a live player's shrunk profile.
    pub fn from_profile(p: &sv10_model::model::Profile) -> TargetRates {
        TargetRates {
            vpip: p.vpip as f64,
            pfr: p.pfr as f64,
            bet_first: (p.bet_first.iter().sum::<f32>() / 3.0) as f64,
            fold_vs_bet: (p.fold_vs_bet.iter().sum::<f32>() / 3.0) as f64,
        }
    }
}

/// Fit an archetype whose simulated behaviour matches `target`, by a few rounds
/// of proportional correction on a table of six copies.
pub fn fit_clone(name: &str, target: &TargetRates, seed: u64) -> Archetype {
    let mut a = Archetype {
        name: name.to_string(),
        vpip: target.vpip.clamp(0.05, 0.95),
        pfr: target.pfr.clamp(0.01, 0.9),
        aggression: (target.bet_first * 1.3).clamp(0.05, 0.95),
        bluff: (target.bet_first * 0.25).clamp(0.0, 0.5),
        call_margin: ((target.fold_vs_bet - 0.40) * 0.8).clamp(-0.3, 0.3),
        sizing: 0.66,
    };
    for round in 0..4 {
        let mut agents: Vec<Box<dyn Agent>> =
            (0..6).map(|i| Box::new(ArchetypeAgent { a: a.clone(), label: format!("c{i}") }) as Box<dyn Agent>).collect();
        let mut obs = ModelStore::default();
        crate::sim::run_table_observed(&mut agents, 700, 100, seed + round, Some(&mut obs));
        let st = &obs.population;
        let rate = |c: &sv10_model::model::Counter, fallback: f64| if c.opp > 20.0 { (c.hit / c.opp) as f64 } else { fallback };
        let m_vpip = rate(&st.vpip, a.vpip);
        let m_pfr = rate(&st.pfr, a.pfr);
        let bf =
            sv10_model::model::Counter { opp: st.bet_first.iter().map(|c| c.opp).sum(), hit: st.bet_first.iter().map(|c| c.hit).sum() };
        let fb =
            sv10_model::model::Counter { opp: st.fold_vs_bet.iter().map(|c| c.opp).sum(), hit: st.fold_vs_bet.iter().map(|c| c.hit).sum() };
        let m_bet = rate(&bf, target.bet_first);
        let m_fold = rate(&fb, target.fold_vs_bet);
        a.vpip = (a.vpip * (target.vpip / m_vpip.max(0.02)).powf(0.8)).clamp(0.03, 0.98);
        a.pfr = (a.pfr * (target.pfr / m_pfr.max(0.01)).powf(0.8)).clamp(0.005, 0.95);
        a.pfr = a.pfr.min(a.vpip);
        a.aggression = (a.aggression + (target.bet_first - m_bet) * 1.5).clamp(0.02, 0.98);
        a.bluff = (a.aggression * 0.25).clamp(0.0, 0.5);
        a.call_margin = (a.call_margin + (target.fold_vs_bet - m_fold) * 0.9).clamp(-0.45, 0.45);
    }
    a
}

/// A simulated opponent that plays a full learned [`Profile`](sv10_model::model::Profile): positional
/// opens and limps, 3-bet / call / fold-to-3-bet / 4-bet / fold-to-4-bet, c-bets, per-street
/// bet-first, size-aware folds, raises versus bets and the river bluff share. Hands are ranked
/// within the agent's own preflop range, so observed frequencies track the profile (0099).
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct ProfileClone {
    /// The opponent's name (the policy looks its model up by it).
    pub name: String,
    /// Rates the clone plays, already perturbed by [`ProfileClone::sampled`] when built for the learner.
    pub profile: sv10_model::model::Profile,
    /// Open size in big blinds.
    pub open_bb: f64,
    /// Postflop bet size as a pot fraction.
    pub bet_size: f64,
}

impl ProfileClone {
    /// A clone that plays `profile` exactly.
    pub fn exact(name: &str, profile: sv10_model::model::Profile) -> ProfileClone {
        ProfileClone { name: name.to_string(), profile, open_bb: 2.75, bet_size: 0.62 }
    }

    /// A clone whose rates are drawn around `profile` with the sampling error of the evidence in
    /// `stats` (logit-normal), so the policy's model of it is realistic rather than an oracle.
    pub fn sampled(
        name: &str,
        profile: sv10_model::model::Profile,
        stats: Option<&sv10_model::model::PlayerStats>,
        rng: &mut SmallRng,
    ) -> ProfileClone {
        use rand_distr_free::normal;
        let mut p = profile;
        let jit = |rate: &mut f32, opp: f32, rng: &mut SmallRng| {
            let r = (*rate as f64).clamp(0.005, 0.995);
            let n = opp as f64 + 8.0;
            let sd = (1.0 / (n * r * (1.0 - r))).sqrt().min(1.5);
            let logit = (r / (1.0 - r)).ln() + sd * normal(rng);
            *rate = (1.0 / (1.0 + (-logit).exp())) as f32;
        };
        let empty = sv10_model::model::PlayerStats::default();
        let s = stats.unwrap_or(&empty);
        jit(&mut p.limp, s.limp.opp, rng);
        jit(&mut p.three_bet, s.three_bet.opp, rng);
        jit(&mut p.call_open, s.call_open.opp, rng);
        jit(&mut p.fold_to_3bet, s.fold_to_3bet.opp, rng);
        jit(&mut p.four_bet, s.four_bet.opp, rng);
        jit(&mut p.fold_to_4bet, s.fold_to_4bet.opp, rng);
        jit(&mut p.cbet, s.cbet.opp, rng);
        jit(&mut p.fold_to_cbet, s.fold_to_cbet.opp, rng);
        jit(&mut p.river_bluff, s.river_bluff.opp, rng);
        for i in 0..3 {
            jit(&mut p.open_pos[i], s.open_pos[i].opp, rng);
            jit(&mut p.vpip_pos[i], s.vpip_pos[i].opp, rng);
            jit(&mut p.bet_first[i], s.bet_first[i].opp, rng);
            jit(&mut p.fold_vs_bet[i], s.fold_vs_bet[i].opp, rng);
            jit(&mut p.raise_vs_bet[i], s.raise_vs_bet[i].opp, rng);
            jit(&mut p.fold_vs_size[i], s.fold_vs_size[i].opp, rng);
        }
        let open_bb = 2.25 + rng.random::<f64>() * 1.25;
        let bet_size = 0.4 + rng.random::<f64>() * 0.55;
        ProfileClone { name: name.to_string(), profile: p, open_bb, bet_size }
    }
}

/// Simulated clones of the `max` most-observed opponents with at least `min_hands` hands, weighted
/// by hands, each drawn with its own sampling error from `seed`.
pub fn live_pool(models: &ModelStore, min_hands: f32, max: usize, seed: u64) -> Vec<(ProfileClone, f64)> {
    use sv10_rng::SeedableRng;
    let mut pool: Vec<(&String, &sv10_model::model::PlayerStats)> = models.players.iter().filter(|(_, s)| s.hands >= min_hands).collect();
    pool.sort_by(|a, b| b.1.hands.total_cmp(&a.1.hands).then_with(|| a.0.cmp(b.0)));
    pool.truncate(max);
    pool.iter()
        .enumerate()
        .map(|(i, (name, st))| {
            let mut rng = SmallRng::seed_from_u64(seed.wrapping_mul(0x9E37_79B9).wrapping_add(i as u64));
            (ProfileClone::sampled(name, models.profile(name), Some(st), &mut rng), st.hands as f64)
        })
        .collect()
}

/// Standard-normal draws without an extra dependency.
mod rand_distr_free {
    use sv10_rng::RngExt;
    use sv10_rng::rngs::SmallRng;
    /// One N(0, 1) sample (Box–Muller).
    pub fn normal(rng: &mut SmallRng) -> f64 {
        let u1 = rng.random::<f64>().max(1e-12);
        let u2 = rng.random::<f64>();
        (-2.0 * u1.ln()).sqrt() * (std::f64::consts::TAU * u2).cos()
    }
}

/// An agent that plays a [`ProfileClone`].
pub struct ProfileAgent {
    /// Rates and sizes.
    pub c: ProfileClone,
    /// Tally label.
    pub label: String,
    /// Upper preflop percentile of the range the agent continued with this hand (1 = any two).
    range_top: f32,
}

impl ProfileAgent {
    /// An agent labelled `label` playing `c`.
    pub fn new(c: ProfileClone, label: String) -> ProfileAgent {
        ProfileAgent { c, label, range_top: 1.0 }
    }

    fn preflop(&mut self, sit: &Situation, rng: &mut SmallRng) -> Action {
        let p = &self.c.profile;
        let me = sit.hero_seat;
        let bb = sit.bb as f64;
        let pct = preflop::percentile(sit.hole[0], sit.hole[1]);
        let g = sv10_model::model::position_group(sit.position_of(me));
        let pre: Vec<_> = sit.history.iter().filter(|r| r.street == Street::Preflop).collect();
        let raisers: Vec<usize> = pre.iter().filter(|r| aggressive(r)).map(|r| r.seat).collect();
        let raises = raisers.len();
        let cur = sit.current_bet();
        let passive = if sit.can_check { Action::Check } else { Action::Fold };
        let keep = |top: f32, a: Action, this: &mut f32| {
            *this = top.clamp(0.0, 1.0);
            a
        };
        let mut top = self.range_top;
        let action = match raises {
            0 => {
                if sit.can_check {
                    // Big blind after limps: raise the top, check the rest.
                    let iso = p.open_pos[g] * 0.5;
                    if pct < iso {
                        keep(iso, raise_or_call(sit, (bb * (self.c.open_bb + 1.0)) as i64), &mut top)
                    } else {
                        keep(1.0, Action::Check, &mut top)
                    }
                } else {
                    let open = p.open_pos[g];
                    let limped = pre.iter().any(|r| r.kind == sv10_engine::engine::ActionKind::Call);
                    let flat = if limped { (p.vpip_pos[g] - open).max(0.0) } else { p.limp };
                    let size = self.c.open_bb + if limped { pre.len() as f64 * 0.5 } else { 0.0 };
                    if pct < open {
                        keep(open, raise_or_call(sit, (bb * size) as i64), &mut top)
                    } else if pct < open + flat {
                        keep(open + flat, Action::Call, &mut top)
                    } else {
                        keep(0.0, passive, &mut top)
                    }
                }
            }
            1 => {
                if raisers[0] == me {
                    keep(top, Action::Call, &mut top)
                } else {
                    let three = p.three_bet;
                    let call = three + p.call_open;
                    if pct < three {
                        keep(three, raise_or_call(sit, cur * 3), &mut top)
                    } else if pct < call {
                        keep(call, Action::Call, &mut top)
                    } else {
                        keep(0.0, passive, &mut top)
                    }
                }
            }
            2 => {
                let width = self.range_top.max(0.01);
                if raisers[0] == me {
                    // Opener facing a 3-bet: rank within the opening range.
                    let rel = pct / width;
                    if rel < p.four_bet {
                        keep(width * p.four_bet, raise_or_call(sit, (cur as f64 * 2.3) as i64), &mut top)
                    } else if rel < 1.0 - p.fold_to_3bet {
                        keep(width * (1.0 - p.fold_to_3bet), Action::Call, &mut top)
                    } else {
                        keep(0.0, Action::Fold, &mut top)
                    }
                } else if pct < p.three_bet * 0.25 {
                    keep(p.three_bet * 0.25, raise_or_call(sit, (cur as f64 * 2.3) as i64), &mut top)
                } else if pct < p.call_open * 0.25 {
                    keep(p.call_open * 0.25, Action::Call, &mut top)
                } else {
                    keep(0.0, Action::Fold, &mut top)
                }
            }
            _ => {
                let width = self.range_top.max(0.005);
                let rel = pct / width;
                if raisers.len() >= 2 && raisers[1] == me {
                    // 3-bettor facing a 4-bet.
                    let cont = 1.0 - p.fold_to_4bet;
                    if rel < cont * 0.35 {
                        keep(width * cont * 0.35, Action::AllIn, &mut top)
                    } else if rel < cont {
                        keep(width * cont, Action::Call, &mut top)
                    } else {
                        keep(0.0, Action::Fold, &mut top)
                    }
                } else if raisers.contains(&me) && rel < 0.35 {
                    keep(width * 0.35, Action::Call, &mut top)
                } else if pct < 0.015 {
                    keep(0.015, Action::Call, &mut top)
                } else {
                    keep(0.0, Action::Fold, &mut top)
                }
            }
        };
        self.range_top = top;
        let _ = rng;
        action
    }

    /// Hand strength within the agent's own continuing range on this board (1 = strongest).
    fn strength(&self, sit: &Situation) -> f32 {
        let info = sv10_model::oprange::board_info(&sit.board);
        let table = &preflop::table().combo_percentile;
        let mut range = sv10_cards::range::Range::empty();
        let top = self.range_top.max(0.02);
        for i in 0..sv10_cards::range::NUM_COMBOS {
            if info.strength[i] > 0.0 && table[i] <= top {
                range.w[i] = 1.0;
            }
        }
        let me = combo_index(sit.hole[0], sit.hole[1]);
        range.w[me] = 1.0;
        1.0 - sv10_model::oprange::range_percentiles(&range, &info.strength)[me]
    }
}

impl Agent for ProfileAgent {
    fn name(&self) -> &str {
        &self.label
    }
    fn act(&mut self, sit: &Situation, rng: &mut SmallRng) -> Action {
        if sit.street == Street::Preflop {
            return self.preflop(sit, rng);
        }
        let p = &self.c.profile;
        let st = sit.street.index() - 1;
        let s = self.strength(sit) as f64;
        let me = sit.hero_seat;
        let pot = sit.pot as f64;
        let pf_aggressor = sit.history.iter().rfind(|r| r.street == Street::Preflop && aggressive(r)).map(|r| r.seat);
        let street_aggr: Vec<usize> = sit.history.iter().filter(|r| r.street == sit.street && aggressive(r)).map(|r| r.seat).collect();
        let bluff_share = if sit.street == Street::River { p.river_bluff as f64 } else { 0.35 };
        if sit.can_check {
            let rate = if sit.street == Street::Flop && pf_aggressor == Some(me) { p.cbet } else { p.bet_first[st] } as f64;
            let value = rate * (1.0 - bluff_share);
            let bluff = rate * bluff_share;
            let bet = s > 1.0 - value || (s < 0.5 && rng.random::<f64>() < bluff / 0.5);
            if bet {
                let size = self.c.bet_size * (0.75 + 0.5 * rng.random::<f64>());
                return raise_or_call(sit, sit.current_bet() + (pot * size).max(sit.bb as f64) as i64);
            }
            return Action::Check;
        }
        let call = sit.call_amount as f64;
        let frac = call / (pot - call).max(1.0);
        let mut fold = p.fold_to_bet(sit.street, frac);
        if sit.street == Street::Flop && street_aggr.len() == 1 && pf_aggressor == street_aggr.first().copied() {
            fold = 0.5 * fold + 0.5 * p.fold_to_cbet as f64;
        }
        if street_aggr.contains(&me) {
            fold = (fold + 0.12).min(0.95);
        }
        let raise = p.raise_vs_bet[st] as f64;
        if s > 1.0 - raise || (s < 0.3 && rng.random::<f64>() < raise * 0.25 / 0.3) {
            return raise_or_call(sit, sit.current_bet() * 3);
        }
        if s > fold { Action::Call } else { Action::Fold }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rate(c: &sv10_model::model::Counter) -> f32 {
        c.hit / c.opp.max(1.0)
    }

    #[test]
    fn profile_agents_reproduce_their_profile() {
        for (tag, tweak) in [
            ("tight-foldy", (0.12f32, 0.04f32, 0.60f32, 0.55f32, 0.30f32, 0.05f32)),
            ("loose-sticky", (0.35, 0.12, 0.25, 0.30, 0.50, 0.12)),
        ] {
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
            let mut agents: Vec<Box<dyn Agent>> = (0..6)
                .map(|i| Box::new(ProfileAgent::new(ProfileClone::exact("x", p.clone()), format!("p{i}"))) as Box<dyn Agent>)
                .collect();
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
}
