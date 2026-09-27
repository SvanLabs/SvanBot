//! A repeatable six-max benchmark (0279).
//!
//! Every strength number the project owned was heads-up — `sim paired`, the promotion gate's
//! fresh-deal confirmation, the speed-only rule's reproducibility check — while the fleet plays
//! six-handed, and the population that faces a challenger moves every season. A measurement that
//! moves when the population moves cannot settle an argument, and a heads-up number cannot see a
//! *seat*, which is exactly what a per-position preflop change is about (0277).
//!
//! So: one champion and one challenger over identical deals against a **frozen** opponent pool,
//! with the result broken down by the two things a six-max change moves — the position the hero
//! holds, and how the hand ended. The pool is a fixed archetype mix, so the benchmark is the same
//! benchmark on any machine in any season, with no store and no live population to drift.
//!
//! Both arms are played from one deck stream, so the per-hand difference is paired: the same cards,
//! the same opponents, the same luck-reduced runout, only the policy differs.

use crate::agents::{Agent, Archetype, PolicyAgent};
use crate::policy::Params;
use crate::sim::PairedResult;
use std::collections::BTreeMap;
use sv10_engine::engine::{Action, Hand};
use sv10_engine::situation::{Position, Situation, position_of};
use sv10_model::model::{HandSummary, ModelStore};
use sv10_rng::SeedableRng;
use sv10_rng::rngs::SmallRng;

/// Seats at the benchmark table. Six, because that is what the fleet plays.
pub const SEATS: usize = 6;

/// The frozen pool: the built-in styles, one each, so the table spans loose to tight, aggressive to
/// nit. Named rather than sampled — a frozen pool that is regenerated from live statistics is not
/// frozen.
pub const FROZEN_POOL: [&str; 5] = ["station", "maniac", "nit", "tag", "lag"];

/// How a hand ended for us. The same classes `review` reports on stored hands, so a benchmark
/// number and a live number mean the same thing.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    /// We were paid.
    Won,
    /// We lost at showdown.
    LostAtShowdown,
    /// We invested and then gave the pot up.
    FoldedAfterInvesting,
}

impl Outcome {
    /// The report's key for this class.
    pub fn label(self) -> &'static str {
        match self {
            Outcome::Won => "won",
            Outcome::LostAtShowdown => "lost_at_showdown",
            Outcome::FoldedAfterInvesting => "folded_after_investing",
        }
    }
}

/// The outcome class of the hero's seat in a finished hand, from the engine's own record.
pub fn outcome(net: i64, invested: i64, folded: bool, shown: bool) -> Outcome {
    if net > 0 {
        Outcome::Won
    } else if shown {
        Outcome::LostAtShowdown
    } else if folded && invested > 0 {
        Outcome::FoldedAfterInvesting
    } else {
        Outcome::LostAtShowdown
    }
}

/// One bucket of the breakdown: both arms over the same hands, so it pairs like the headline.
#[derive(Clone, Debug, Default)]
struct Bucket {
    champion: Vec<f64>,
    challenger: Vec<f64>,
}

impl Bucket {
    /// Add a hand, keeping the two arms aligned.
    fn push(&mut self, champion: f64, challenger: f64) {
        self.champion.push(champion);
        self.challenger.push(challenger);
    }

    /// The paired difference in big blinds per hand, and the standard error of it.
    fn paired(&self) -> PairedResult {
        let diffs: Vec<f64> = self.champion.iter().zip(&self.challenger).map(|(c, x)| (x - c) / 20.0).collect();
        let n = diffs.len() as f64;
        if diffs.is_empty() {
            return PairedResult { hands: 0, mean_bb: 0.0, se_bb: f64::INFINITY, differing: 0 };
        }
        let mean = diffs.iter().sum::<f64>() / n;
        let var = (diffs.iter().map(|d| (d - mean) * (d - mean)).sum::<f64>() / n).max(0.0);
        PairedResult {
            hands: diffs.len() as u64,
            mean_bb: mean,
            se_bb: (var / n).sqrt(),
            differing: diffs.iter().filter(|d| **d != 0.0).count() as u64,
        }
    }
}

/// A benchmark run.
#[derive(Clone, Debug, serde::Serialize)]
pub struct Bench {
    /// Hands each arm played.
    pub hands: u64,
    /// The five opponents' names, so a run names what it measured against.
    pub pool: Vec<String>,
    /// Challenger minus champion, in big blinds per hand, with its 95% interval.
    pub paired: PairedResult,
    /// The champion's own bb/100 overall.
    pub champion_bb100: f64,
    /// The challenger's own bb/100 overall.
    pub challenger_bb100: f64,
    /// Per position class: our bb/100 in each arm and the paired difference.
    pub by_position: BTreeMap<String, PositionRead>,
    /// Per outcome class: the same.
    pub by_outcome: BTreeMap<String, PositionRead>,
}

/// One breakdown row: the level in both arms, and the paired difference.
#[derive(Clone, Debug, Default, serde::Serialize)]
pub struct PositionRead {
    /// Hands in the bucket.
    pub hands: u64,
    /// Champion's bb/100 in it.
    pub champion_bb100: f64,
    /// Champion's 95% interval around that level. A bucket's *level* is a noisy estimate — at
    /// 2,000 hands a position is worth tens of bb/100 of standard error — and the paired interval
    /// says nothing about it, because it is exactly zero whenever both arms are the same policy.
    /// Without this column a reader sees `+119.5` next to `+0.000..+0.000` and concludes the seat
    /// is worth 120 bb/100. It is not pinned down at all.
    pub champion_95: (f64, f64),
    /// Challenger's bb/100 in it.
    pub challenger_bb100: f64,
    /// The same interval for the challenger.
    pub challenger_95: (f64, f64),
    /// The paired difference and its interval.
    pub paired: PairedResult,
}

/// Mean of `nets` in bb/100 as `(point, low, high)`, 95%. Chips become bb/100 by dividing by the
/// 20 chips in a big blind and scaling to a hundred hands — a positive factor, so the order of the
/// interval survives it.
fn level95(nets: &[f64]) -> (f64, f64, f64) {
    let n = nets.len() as f64;
    if n == 0.0 {
        return (0.0, 0.0, 0.0);
    }
    let mean = nets.iter().sum::<f64>() / n;
    let var = (nets.iter().map(|x| (x - mean) * (x - mean)).sum::<f64>() / n).max(0.0);
    let half = 1.96 * (var / n).sqrt() * 5.0;
    (mean * 5.0, mean * 5.0 - half, mean * 5.0 + half)
}

/// One breakdown row: the level in each arm and the paired difference, in bb/100.
fn read(bucket: &Bucket) -> PositionRead {
    let (champion, c_lo, c_hi) = level95(&bucket.champion);
    let (challenger, x_lo, x_hi) = level95(&bucket.challenger);
    PositionRead {
        hands: bucket.champion.len() as u64,
        champion_bb100: champion,
        champion_95: (c_lo, c_hi),
        challenger_bb100: challenger,
        challenger_95: (x_lo, x_hi),
        paired: bucket.paired(),
    }
}

/// The frozen pool: the built-in styles, one agent each. Nothing here is built from a live model,
/// a stored opponent or a season, which is what "frozen" has to mean for a benchmark to be worth
/// anything a year from now.
pub fn frozen_pool() -> Vec<Archetype> {
    FROZEN_POOL.iter().map(|kind| crate::agents::archetype(kind)).collect()
}

/// Play one arm: `hands` hands against `pool`, returning each hand's luck-reduced net for the hero
/// (seat 0) with the position and outcome class it was played in.
fn arm(params: &Params, models: &ModelStore, pool: &[Archetype], hands: usize, stack_bb: i64, seed: u64) -> Vec<HandRead> {
    let names: Vec<String> = std::iter::once("hero".to_string()).chain(pool.iter().map(|p| p.name.clone())).collect();
    let mut hero = PolicyAgent { label: "hero".into(), models: models.clone(), params: params.clone(), learn: false, nn: None };
    let mut seats: Vec<Box<dyn Agent>> = vec![];
    for (i, style) in pool.iter().enumerate() {
        seats.push(crate::agents::OpponentSpec::agent(style, i + 1));
        let _ = &names;
    }
    let mut deal_rng = SmallRng::seed_from_u64(seed);
    let mut out = Vec::with_capacity(hands);
    for h in 0..hands {
        let stacks = vec![stack_bb * 20; SEATS];
        let mut hand = Hand::new(&stacks, h % SEATS, 10, 20, &mut deal_rng);
        // Decision randomness, one stream per hand: identical in both arms, so a difference is the
        // policy and not the coin.
        let mut rng = SmallRng::seed_from_u64(seed ^ 0x9E37_79B9_7F4A_7C15 ^ (h as u64).wrapping_mul(0xD1B5_4A32_D192_ED03));
        let mut guard = 0;
        while let Some(seat) = hand.actor() {
            let sit = Situation::from_hand(&hand, seat, &names);
            let action = if seat == 0 { hero.act(&sit, &mut rng) } else { seats[seat - 1].act(&sit, &mut rng) };
            if hand.apply(action).is_err() {
                hand.apply(if hand.legal().can_check { Action::Check } else { Action::Fold }).unwrap();
            }
            guard += 1;
            assert!(guard < 500, "runaway hand");
        }
        let net_bb = hand.net().first().copied().unwrap_or(0);
        let net = net_bb as f64;
        let hero_state = &hand.seats[0];
        let summary = HandSummary {
            players: names.iter().cloned().enumerate().collect(),
            button: hand.button,
            bb: 20,
            history: hand.history.clone(),
            board: hand.board.clone(),
            stacks: hand.seats.iter().enumerate().map(|(i, s)| (i, s.start_stack)).collect(),
            shown: if hand.showdown() {
                hand.seats.iter().enumerate().filter(|(_, s)| !s.folded).map(|(i, s)| (i, s.hole)).collect()
            } else {
                vec![]
            },
        };
        hero.observe(&summary);
        for agent in seats.iter_mut() {
            agent.observe(&summary);
        }
        out.push(HandRead {
            net,
            position: position_of(&(0..SEATS).collect::<Vec<_>>(), hand.button, 0),
            outcome: outcome(net_bb, hero_state.invested, hero_state.folded, hand.showdown()),
        });
    }
    out
}

/// The position class as the benchmark's key: `small_blind`, `early`, and so on.
pub fn position_name(p: Position) -> String {
    match p {
        Position::SmallBlind => "small_blind",
        Position::BigBlind => "big_blind",
        Position::Button => "button",
        Position::Cutoff => "cutoff",
        Position::Middle => "middle",
        Position::Early => "early",
    }
    .to_string()
}

/// What one hand of the benchmark produced.
#[derive(Clone, Debug, serde::Serialize)]
pub struct HandRead {
    /// Luck-reduced net for the hero, in chips.
    pub net: f64,
    /// The position the hero held.
    pub position: Position,
    /// How the hand ended.
    pub outcome: Outcome,
}

/// Run the benchmark. `seed` fixes the deck stream, the button rotation and the decision stream, so
/// two runs with the same arguments measure the same thing.
pub fn six_max_paired(champion: &Params, challenger: &Params, models: &ModelStore, hands: usize, stack_bb: i64, seed: u64) -> Bench {
    let pool = frozen_pool();
    let champ = arm(champion, models, &pool, hands, stack_bb, seed);
    let chal = arm(challenger, models, &pool, hands, stack_bb, seed);
    let mut overall = Bucket::default();
    let mut positions: BTreeMap<String, Bucket> = BTreeMap::new();
    let mut outcomes: BTreeMap<Outcome, Bucket> = BTreeMap::new();
    for (c, x) in champ.iter().zip(&chal) {
        overall.push(c.net, x.net);
        positions.entry(position_name(c.position)).or_default().push(c.net, x.net);
        outcomes.entry(c.outcome).or_default().push(c.net, x.net);
    }
    let n = champ.len().max(1) as f64;
    Bench {
        hands: champ.len() as u64,
        pool: pool.iter().map(|p| p.name.clone()).collect(),
        paired: overall.paired(),
        champion_bb100: champ.iter().map(|h| h.net).sum::<f64>() / 20.0 / n * 100.0,
        challenger_bb100: chal.iter().map(|h| h.net).sum::<f64>() / 20.0 / n * 100.0,
        by_position: positions.iter().map(|(p, b)| (p.clone(), read(b))).collect(),
        by_outcome: outcomes.iter().map(|(o, b)| (o.label().to_string(), read(b))).collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn params() -> Params {
        Params { samples: 150, ..Default::default() }
    }

    /// The two arms must see the same cards, or the comparison is noise: the same seed gives
    /// the same hands in both, and the decks are the table's own.
    #[test]
    fn both_arms_see_identical_deals() {
        let models = ModelStore::default();
        let p = params();
        let pool = frozen_pool();
        let a = arm(&p, &models, &pool, 300, 100, 7);
        let b = arm(&p, &models, &pool, 300, 100, 7);
        let loose = arm(&Params { call_margin: 0.30, ..p.clone() }, &models, &pool, 300, 100, 7);
        assert_eq!(a.len(), 300);
        assert!(a.iter().zip(&b).all(|(x, y)| x.position == y.position && x.outcome == y.outcome), "identical policy, identical hands");
        // A different policy must change the results, or the arms are not actually playing it.
        let differs = a.iter().zip(&loose).filter(|(x, y)| x.net != y.net).count();
        assert!(differs > 0, "a 0.30 call margin must change the play over 300 hands");
    }

    /// The benchmark is reproducible: same arguments, same numbers, on any machine.
    #[test]
    fn a_run_is_reproducible_from_its_seed() {
        let models = ModelStore::default();
        let p = params();
        let one = six_max_paired(&p, &Params { call_margin: 0.02, ..p.clone() }, &models, 200, 100, 11);
        let two = six_max_paired(&p, &Params { call_margin: 0.02, ..p.clone() }, &models, 200, 100, 11);
        assert_eq!(one.hands, two.hands);
        assert_eq!(one.paired.mean_bb, two.paired.mean_bb);
        assert_eq!(one.paired.se_bb, two.paired.se_bb);
        assert_eq!(one.by_position.keys().collect::<Vec<_>>(), two.by_position.keys().collect::<Vec<_>>());
    }

    /// The breakdown the ticket is for: a seat is visible, and every position is played.
    #[test]
    fn every_position_is_played_and_reported() {
        let models = ModelStore::default();
        let p = params();
        let bench = six_max_paired(&p, &p, &models, 600, 100, 3);
        assert_eq!(bench.hands, 600);
        for seat in ["small_blind", "big_blind", "early", "middle", "cutoff", "button"] {
            assert!(bench.by_position.contains_key(seat), "{seat} missing from {:?}", bench.by_position.keys().collect::<Vec<_>>());
            assert!(bench.by_position[seat].hands > 0);
        }
        assert!(bench.by_outcome.contains_key("won"), "{:?}", bench.by_outcome.keys().collect::<Vec<_>>());
        assert!(bench.pool.len() == 5);
        // Same policy in both arms: the paired difference must be exactly zero.
        assert_eq!(bench.paired.mean_bb, 0.0);
        assert_eq!(bench.paired.differing, 0, "an identical policy cannot differ on a hand");
    }

    /// The interval on a bucket's *own level*. An identical-arm run reports a paired difference of
    /// exactly zero, so `+0.000..+0.000` next to `+119.5` is how the button came to look worth 120
    /// bb/100 when across three seeds it read +119.5, −15.7 and +46.1. The paired column says
    /// nothing about the level; this one does.
    #[test]
    fn a_bucket_reports_an_interval_on_its_own_level() {
        let models = ModelStore::default();
        let p = params();
        let bench = six_max_paired(&p, &p, &models, 1_200, 100, 5);
        for (seat, row) in &bench.by_position {
            let (lo, hi) = row.champion_95;
            assert!(lo.is_finite() && hi.is_finite(), "{seat}: no interval on the level");
            assert!(lo <= row.champion_bb100 && row.champion_bb100 <= hi, "{seat}: the point sits outside its own interval");
            // The floor that matters: 200 hands of a seat is worth tens of bb/100 of standard
            // error, and a reader who is shown a point estimate alone will believe a swing that
            // this interval says is noise.
            assert!(hi - lo > 1.0, "{seat}: {lo:+.1}..{hi:+.1} is too tight to be an estimate of a seat");
        }
    }

    /// `level95` is the estimator, tested on a population whose variance is known — which is the
    /// only way to pin the 1/sqrt(n) law, because a real bucket's variance is itself a random
    /// quantity: 20 hands that all check have almost none, and would report a spuriously tight
    /// interval that no amount of arithmetic can call wrong.
    #[test]
    fn the_level_interval_shrinks_as_the_root_of_the_hands() {
        // Chips, mean 0, alternating ±1000: variance 1e6, sd 1000, so bb/100 sd = 5000.
        let pop: Vec<f64> = (0..2_000).map(|i| if i % 2 == 0 { 1_000.0 } else { -1_000.0 }).collect();
        let width = |n: usize| {
            let (point, lo, hi) = level95(&pop[..n]);
            assert!(lo <= point && point <= hi, "the point must sit inside its own interval");
            hi - lo
        };
        // 1/sqrt(n): 16x the hands is a quarter of the width.
        let ratio = width(125) / width(2_000);
        assert!((3.5..4.5).contains(&ratio), "16x the hands gave {ratio:.2}x the width, not 4x");
        // And the width is the sample sd scaled to bb/100, times 1.96, on each side.
        let expected = 2.0 * 1.96 * 1000.0 / (125.0f64).sqrt() * 5.0;
        assert!((width(125) - expected).abs() < 1.0, "expected {expected:.1}, got {:.1}", width(125));
        assert_eq!(level95(&[]), (0.0, 0.0, 0.0), "an empty bucket has no estimate and no interval");
    }

    /// The pool is frozen: the built-in styles by name, nothing derived from a live population.
    #[test]
    fn the_pool_is_the_frozen_archetype_mix() {
        let pool = frozen_pool();
        assert_eq!(pool.len(), 5);
        assert_eq!(pool.iter().map(|a| a.name.clone()).collect::<Vec<_>>(), FROZEN_POOL.to_vec());
        assert!(frozen_pool().iter().any(|a| a.vpip < 0.2), "the nit is still a nit");
        assert!(frozen_pool().iter().any(|a| a.vpip > 0.6), "and the station still calls anything");
    }
}
