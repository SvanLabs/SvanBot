//! Per-opponent statistics learned from observed hands, shrunk toward the
//! population aggregate so a handful of samples can't swing decisions.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use sv10_cards::cards::Card;
use sv10_cards::range::Range;
use sv10_engine::engine::{ActionKind, ActionRecord, Street};
use sv10_equity::equity::river_equity_exact;

/// An opportunity/success tally (weighted, so fractional after age decay).
#[derive(Clone, Copy, Default, Debug, Serialize, Deserialize)]
pub struct Counter {
    /// Times the situation arose.
    pub opp: f32,
    /// Times the player took the tracked action.
    pub hit: f32,
}

impl Counter {
    fn add(&mut self, hit: bool) {
        self.opp += 1.0;
        if hit {
            self.hit += 1.0;
        }
    }
    /// Rate shrunk toward `prior` as if `weight` prior opportunities had been seen.
    pub fn rate(&self, prior: f32, weight: f32) -> f32 {
        (self.hit + prior * weight) / (self.opp + weight)
    }
    fn merge(&mut self, o: &Counter, w: f32) {
        self.opp += o.opp * w;
        self.hit += o.hit * w;
    }
}

/// A player's think times in one context (0234): count and summed natural log of milliseconds, so
/// the typical (geometric-mean) think time is `exp(sum_ln / n)`.
#[derive(Clone, Copy, Default, Debug, Serialize, Deserialize)]
pub struct ThinkStat {
    /// Timed actions (weighted).
    pub n: f32,
    /// Sum of `ln(think_ms)`.
    pub sum_ln: f32,
}

impl ThinkStat {
    fn add(&mut self, ms: u32) {
        self.n += 1.0;
        self.sum_ln += (ms.max(1) as f32).ln();
    }
    fn merge(&mut self, o: &ThinkStat, w: f32) {
        self.n += o.n * w;
        self.sum_ln += o.sum_ln * w;
    }
    /// Typical think time in ms, or 0 below `MIN_THINK_SAMPLES` timed actions.
    pub fn typical_ms(&self) -> f32 {
        if self.n >= MIN_THINK_SAMPLES { (self.sum_ln / self.n).exp() } else { 0.0 }
    }
}

/// Timed actions a player needs in a context before their typical think time is used (0234).
pub const MIN_THINK_SAMPLES: f32 = 8.0;

/// Think-time context of an action (0234): 0 after another player's action (the server paces
/// these), 1 first to act after the street opened.
pub fn think_context(rec: &ActionRecord) -> usize {
    rec.street_open as usize
}

/// Bet size relative to the pot before the bet: small < 0.45, medium < 0.85, large.
pub fn size_bucket(frac: f64) -> usize {
    if frac < 0.45 {
        0
    } else if frac < 0.85 {
        1
    } else {
        2
    }
}

/// Raw tallies for one player (or the whole population); shrinkage happens in [`ModelStore::profile`].
#[derive(Clone, Default, Debug, Serialize, Deserialize)]
pub struct PlayerStats {
    /// Hands observed (weighted).
    pub hands: f32,
    /// Voluntarily put chips in preflop (call, raise or all-in), per hand dealt.
    pub vpip: Counter,
    /// Raised preflop, per hand dealt.
    pub pfr: Counter,
    /// Raised when first in (no raise yet), per first-in opportunity.
    pub open_raise: Counter,
    /// Called when first in, per first-in opportunity.
    pub limp: Counter,
    /// Re-raised facing exactly one raise on their first action.
    pub three_bet: Counter,
    /// Flat-called facing exactly one raise on their first action.
    pub call_open: Counter,
    /// Folded as the opener facing a 3-bet.
    pub fold_to_3bet: Counter,
    /// Re-raised as the opener facing a 3-bet.
    pub four_bet: Counter,
    /// Folded as the 3-bettor facing a 4-bet.
    #[serde(default)]
    pub fold_to_4bet: Counter,
    /// Indexed by street 1..=3 as 0..=2.
    pub bet_first: [Counter; 3],
    /// Folded when facing a bet (flop, turn, river).
    pub fold_vs_bet: [Counter; 3],
    /// Raised when facing a bet (flop, turn, river).
    pub raise_vs_bet: [Counter; 3],
    /// Folded facing a small / medium / large bet (see [`size_bucket`]), all postflop streets.
    pub fold_vs_size: [Counter; 3],
    /// Bet the flop as the last preflop raiser when checked to (or first to act).
    pub cbet: Counter,
    /// Folded to a flop bet made by the last preflop raiser.
    pub fold_to_cbet: Counter,
    /// Went to showdown, per flop seen.
    pub wtsd: Counter,
    /// Showdowns after making the last river bet/raise; hit = weak hand.
    pub river_bluff: Counter,
    /// Showdowns reached (weighted).
    pub showdowns: f32,
    /// Position groups: 0 early (EP/MP), 1 late (CO/BTN), 2 blinds.
    #[serde(default)]
    pub vpip_pos: [Counter; 3],
    /// Preflop raise by position group.
    #[serde(default)]
    pub pfr_pos: [Counter; 3],
    /// First-in raise by position group.
    #[serde(default)]
    pub open_pos: [Counter; 3],
    /// Showdowns reached; hit = won (all or part of) the pot.
    #[serde(default)]
    pub won_showdown: Counter,
    /// Think times by context (after an action, first on a new street), from the server's clock (0234).
    #[serde(default)]
    pub think: [ThinkStat; 2],
}

/// Position group used by positional stats: 0 early (EP/MP), 1 late (CO/BTN), 2 blinds.
pub fn position_group(p: sv10_engine::situation::Position) -> usize {
    use sv10_engine::situation::Position::*;
    match p {
        Early | Middle => 0,
        Cutoff | Button => 1,
        SmallBlind | BigBlind => 2,
    }
}

impl PlayerStats {
    /// Add another player's tallies at full weight.
    pub fn merge(&mut self, o: &PlayerStats) {
        self.merge_weighted(o, 1.0);
    }

    /// Scale every decision tally by `f` (recency weighting, 0168); `hands` and `showdowns` keep
    /// counting every hand, so sample-size gates and the dashboard's hand counts are unchanged.
    pub fn decay_counters(&mut self, f: f32) {
        let mut out = PlayerStats::default();
        out.merge_weighted(self, f);
        out.hands = self.hands;
        out.showdowns = self.showdowns;
        *self = out;
    }

    /// Merge with every count scaled by `w` (older evidence counts for less).
    pub fn merge_weighted(&mut self, o: &PlayerStats, w: f32) {
        self.hands += o.hands * w;
        self.showdowns += o.showdowns * w;
        for (a, b) in [
            (&mut self.vpip, &o.vpip),
            (&mut self.pfr, &o.pfr),
            (&mut self.open_raise, &o.open_raise),
            (&mut self.limp, &o.limp),
            (&mut self.three_bet, &o.three_bet),
            (&mut self.call_open, &o.call_open),
            (&mut self.fold_to_3bet, &o.fold_to_3bet),
            (&mut self.four_bet, &o.four_bet),
            (&mut self.fold_to_4bet, &o.fold_to_4bet),
            (&mut self.cbet, &o.cbet),
            (&mut self.fold_to_cbet, &o.fold_to_cbet),
            (&mut self.wtsd, &o.wtsd),
            (&mut self.river_bluff, &o.river_bluff),
        ] {
            a.merge(b, w);
        }
        self.won_showdown.merge(&o.won_showdown, w);
        for (a, b) in self.think.iter_mut().zip(&o.think) {
            a.merge(b, w);
        }
        for i in 0..3 {
            self.vpip_pos[i].merge(&o.vpip_pos[i], w);
            self.pfr_pos[i].merge(&o.pfr_pos[i], w);
            self.open_pos[i].merge(&o.open_pos[i], w);
            self.bet_first[i].merge(&o.bet_first[i], w);
            self.fold_vs_bet[i].merge(&o.fold_vs_bet[i], w);
            self.raise_vs_bet[i].merge(&o.raise_vs_bet[i], w);
            self.fold_vs_size[i].merge(&o.fold_vs_size[i], w);
        }
    }
}

/// A completed hand in the shared format, as seen from one observer's seat.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct HandSummary {
    /// Dealt-in players as (seat, name).
    pub players: Vec<(usize, String)>,
    /// Button seat.
    pub button: usize,
    /// Big blind in chips.
    pub bb: i64,
    /// All actions, blinds excluded.
    pub history: Vec<ActionRecord>,
    /// Board cards dealt (0–5).
    pub board: Vec<Card>,
    /// Hole cards revealed at showdown, by seat.
    pub shown: Vec<(usize, [Card; 2])>,
    /// Stacks at the start of the hand (before blinds), when known.
    #[serde(default)]
    pub stacks: Vec<(usize, i64)>,
}

/// Default rates for a soft bot pool, used until the population aggregate is large.
pub mod defaults {
    /// Default VPIP.
    pub const VPIP: f32 = 0.32;
    /// Default PFR.
    pub const PFR: f32 = 0.18;
    /// Default first-in raise rate.
    pub const OPEN_RAISE: f32 = 0.20;
    /// Default first-in limp rate.
    pub const LIMP: f32 = 0.10;
    /// Default 3-bet rate.
    pub const THREE_BET: f32 = 0.08;
    /// Default flat-call-of-an-open rate.
    pub const CALL_OPEN: f32 = 0.22;
    /// Default fold-to-3-bet rate.
    pub const FOLD_TO_3BET: f32 = 0.50;
    /// Default 4-bet rate.
    pub const FOUR_BET: f32 = 0.08;
    /// Default fold-to-4-bet rate.
    pub const FOLD_TO_4BET: f32 = 0.45;
    /// Default bet-when-checked-to rate (flop, turn, river).
    pub const BET_FIRST: [f32; 3] = [0.38, 0.32, 0.30];
    /// Default fold-to-bet rate (flop, turn, river).
    pub const FOLD_VS_BET: [f32; 3] = [0.45, 0.42, 0.45];
    /// Default raise-vs-bet rate (flop, turn, river).
    pub const RAISE_VS_BET: [f32; 3] = [0.10, 0.08, 0.07];
    /// Default fold rate facing small, medium and large bets.
    pub const FOLD_VS_SIZE: [f32; 3] = [0.36, 0.45, 0.52];
    /// Default continuation-bet rate.
    pub const CBET: f32 = 0.60;
    /// Default fold-to-continuation-bet rate.
    pub const FOLD_TO_CBET: f32 = 0.45;
    /// Default went-to-showdown rate.
    pub const WTSD: f32 = 0.30;
    /// Default share of weak hands behind a river bet or raise at showdown.
    pub const RIVER_BLUFF: f32 = 0.30;
}

/// Rates for one player with shrinkage already applied.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Profile {
    /// Hands observed (weighted).
    pub hands: f32,
    /// VPIP, shrunk.
    pub vpip: f32,
    /// PFR, shrunk.
    pub pfr: f32,
    /// First-in raise rate, shrunk.
    pub open_raise: f32,
    /// First-in limp rate, shrunk.
    pub limp: f32,
    /// 3-bet rate, shrunk.
    pub three_bet: f32,
    /// Flat-call-of-an-open rate, shrunk.
    pub call_open: f32,
    /// Fold-to-3-bet rate, shrunk.
    pub fold_to_3bet: f32,
    /// 4-bet rate, shrunk.
    pub four_bet: f32,
    /// Fold-to-4-bet rate, shrunk.
    pub fold_to_4bet: f32,
    /// Bet-when-checked-to rate by street (flop, turn, river), shrunk.
    pub bet_first: [f32; 3],
    /// Fold-to-bet rate by street, shrunk.
    pub fold_vs_bet: [f32; 3],
    /// Raise-vs-bet rate by street, shrunk.
    pub raise_vs_bet: [f32; 3],
    /// Fold rate facing small / medium / large bets, shrunk.
    pub fold_vs_size: [f32; 3],
    /// Continuation-bet rate, shrunk.
    pub cbet: f32,
    /// Fold-to-continuation-bet rate, shrunk.
    pub fold_to_cbet: f32,
    /// Went-to-showdown rate, shrunk.
    pub wtsd: f32,
    /// Weak-hand share behind river aggression at showdown, shrunk.
    pub river_bluff: f32,
    /// Open-raise rate by position group (early, late, blinds), shrunk toward
    /// the player's overall rate scaled by typical positional widening.
    pub open_pos: [f32; 3],
    /// Voluntarily-in-pot rate by position group, shrunk toward the overall rate scaled by typical
    /// positional widening.
    pub vpip_pos: [f32; 3],
    /// Share of contested showdowns this player won (shrunk to the pool).
    pub won_showdown: f32,
    /// 0 = pure prior, approaching 1 as samples accumulate.
    pub confidence: f32,
    /// Per-class correction (fold, call, raise) of the response network's prediction when this
    /// player faces a bet, from their own residuals against it (0210); 1 = none.
    #[serde(default = "unit_ratio")]
    pub response_ratio: [f32; 3],
    /// Logit offset on the fold estimate of our heads-up postflop bets against this player, from
    /// how often they folded to them against the street-calibrated estimate (0214); 0 = none.
    #[serde(default)]
    pub fold_logit_offset: f32,
    /// How this player's river bet size tracks hand strength against the pool curve (0223): the
    /// value and bluff shares of their river betting range scale by
    /// `(size / SIZE_TELL_REF)^-size_tell`, so a positive tell makes their big bets stronger and
    /// their small ones weaker, a negative tell the reverse; 0 = the pool curve only.
    #[serde(default)]
    pub size_tell: f32,
    /// This player's typical think time in ms after another action and first on a new street
    /// (0234); 0 where too few of their actions were timed.
    #[serde(default)]
    pub think_base: [f32; 2],
}

fn unit_ratio() -> [f32; 3] {
    crate::residual::UNIT_RATIO
}

impl Profile {
    /// Probability of folding to a bet of `frac` pot on `street` (postflop).
    /// The street rate anchors a ~2/3-pot bet; continuing frequency shrinks
    /// with size along a saturating pot-odds curve, and learned size buckets
    /// nudge the result once they have data.
    pub fn fold_to_bet(&self, street: Street, frac: f64) -> f64 {
        let st = street.index().saturating_sub(1).min(2);
        let base_cont = 1.0 - self.fold_vs_bet[st] as f64;
        let curve = |x: f64| ((1.0 + 2.0 * 0.66) / (1.0 + 2.0 * x.max(0.05))).powf(0.55).min(1.6);
        let mut cont = base_cont * curve(frac);
        let b = size_bucket(frac);
        let learned_cont = (1.0 - self.fold_vs_size[b] as f64) * curve(frac) / curve([0.33, 0.66, 1.2][b]);
        cont = 0.6 * cont + 0.4 * learned_cont;
        (1.0 - cont).clamp(0.02, 0.97)
    }
}

/// Every player's tallies plus the population aggregate, with recovery watermarks; checkpointed
/// by the fleet and read by the learner.
#[derive(Clone, Default, Serialize, Deserialize)]
pub struct ModelStore {
    /// Tallies by player name.
    pub players: HashMap<String, PlayerStats>,
    /// Tallies over every observed player (named or anonymous).
    pub population: PlayerStats,
    /// The image opponents form of *us* (0321): our own play, tallied like any player's but kept
    /// apart because [`observe`](ModelStore::observe) skips our seat (we are not our own opponent).
    /// Keyed by our bot name ([`HERO_SEEN_ONE`]) and, in aggregate, by [`HERO_SEEN_ALL`].
    #[serde(default)]
    pub hero_seen: HashMap<String, PlayerStats>,
    /// Highest stored hand row folded into these stats (for exact crash recovery).
    #[serde(default)]
    pub watermark: Option<i64>,
    /// Stat schema; older checkpoints are rebuilt from stored hands on startup.
    #[serde(default)]
    pub schema: u32,
    /// Highest imported server-history row folded into these stats.
    #[serde(default)]
    pub history_watermark: Option<i64>,
    /// Highest corpus row (hands from other sources, not in the server history) folded in.
    #[serde(default)]
    pub corpus_watermark: Option<i64>,
    /// Recency half-life, in a player's own observed hands, of their decision tallies (0 = all-time).
    /// Each observation first decays that player's tallies by `0.5^(1/half_life)`; the population
    /// prior never decays (0168).
    #[serde(default)]
    pub half_life_hands: f32,
    /// Per-player response-network corrections installed from the learner's residual fit (0210);
    /// never checkpointed, reloaded from the store.
    #[serde(skip)]
    pub response_ratios: std::sync::Arc<HashMap<String, [f32; 3]>>,
    /// Per-player fold logit offsets installed from the learner's per-opponent fold calibration
    /// (0214); never checkpointed, reloaded from the store.
    #[serde(skip)]
    pub fold_offsets: std::sync::Arc<HashMap<String, f32>>,
    /// Per-player river sizing tells installed from the learner's per-opponent sizing fit (0223);
    /// never checkpointed, reloaded from the store.
    #[serde(skip)]
    pub size_tells: std::sync::Arc<HashMap<String, f32>>,
}

/// 3: history hands re-imported with full-detail corpus summaries where available.
pub const MODEL_SCHEMA: u32 = 3;

/// Name prefix for players whose identity is unknown; they update population stats only.
pub const ANON_PREFIX: &str = "\u{0}anon";

const W_PRE: f32 = 12.0;
const W_POST: f32 = 10.0;

impl ModelStore {
    /// Update stats for every player in the hand except `hero` (if given).
    pub fn observe(&mut self, hand: &HandSummary, hero: Option<&str>) {
        self.observe_weighted(hand, hero, 1.0);
    }

    /// As `observe`, with every count scaled by `w`. Seats named with the reserved
    /// `ANON_PREFIX` (unknown players in imported histories) feed only the population.
    pub fn observe_weighted(&mut self, hand: &HandSummary, hero: Option<&str>, w: f32) {
        let stats = extract(hand);
        for (seat, name) in &hand.players {
            if Some(name.as_str()) == hero {
                continue;
            }
            if let Some(s) = stats.get(seat) {
                self.population.merge_weighted(s, w);
                if !name.starts_with(ANON_PREFIX) {
                    let player = self.players.entry(name.clone()).or_default();
                    if self.half_life_hands > 0.0 {
                        player.decay_counters(0.5f32.powf(1.0 / self.half_life_hands));
                    }
                    player.merge_weighted(s, w);
                }
            }
        }
    }
}

/// Aggressive action: a bet, raise, or all-in that raised the street's bet level.
pub fn aggressive(rec: &ActionRecord) -> bool {
    match rec.kind {
        ActionKind::Raise => true,
        ActionKind::AllIn => rec.to > rec.bet_before + rec.to_call_before,
        _ => false,
    }
}

mod extract;
mod image;
mod profile;
#[cfg(test)]
mod tests;

use extract::extract;
pub use image::{HERO_SEEN_ALL, HERO_SEEN_ONE};

/// Per-seat tallies of one hand (what `observe` would add), for studies that score predictions
/// before observing.
pub fn hand_stats(hand: &HandSummary) -> HashMap<usize, PlayerStats> {
    extract(hand)
}
