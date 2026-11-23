//! The re-solve: a depth-limited, exploitative solve of the rest of this street, heads up after
//! the flop (`docs/research/2026-11/01-decision-system.md`).
//!
//! The villain's range follows from the action model and their line; ours is every live pair, so
//! the strategy found is balanced across our whole range, and only then read for our own cards.
//! The tree ([`tree`]) is solved by Discounted CFR (α 1.5, β 0, γ 2: Brown and Sandholm, 2019) over
//! whole ranges at once. A river ends in a showdown. On the flop and the turn the street's end is
//! valued over sampled run-outs of the board ([`runout`]), so a draw is worth the cards to come,
//! with one more round of betting played out on each by continuation strategies ([`leaf`]).
//!
//! The work is an iteration count passed in, larger for larger pots ([`iterations_for`]): the
//! layer never reads a clock (ADR 0001, ADR 0003). The pricer stays the way to decide in pots of
//! three or more, and before the flop.

mod leaf;
mod runout;
mod showdown;
mod tree;

pub use leaf::{BET, CHOICES, Continuation};
pub use runout::RUNOUTS;
pub use tree::{Abstract, MAX_BETS};

use svanbot_core::card::CardSet;
use svanbot_core::rng::Rng;
use svanbot_core::rules::{Action, Chips, Street};
use svanbot_core::spot::Spot;

use crate::action::{ActionModel, Seen, combos, range};
use leaf::{Choice, Deep, Leaves, Round};
use showdown::Showdown;
use tree::{Node, Tree};

/// The fewest and most iterations a re-solve runs on the river.
pub const ITERATIONS: (u32, u32) = (200, 2_000);

/// An iteration before the river costs about this many on it: its leaves play a round on.
pub const DEEP_COST: u32 = 4;

/// Iterations for a pot of `pot` chips at a big blind of `big_blind`, with `cards` on the board:
/// more for bigger pots, and fewer before the river, so the time spent is about the same.
pub fn iterations_for(pot: Chips, big_blind: Chips, cards: usize) -> u32 {
    let blinds = pot / big_blind.max(1);
    // Counted in chips' width, so a deep pot cannot overflow the count, then capped to a u32.
    let river = (u64::from(ITERATIONS.0) + 20 * blinds).min(u64::from(ITERATIONS.1)) as u32;
    if cards < 5 { river / DEEP_COST } else { river }
}

/// A re-solved decision: the action, the strategy our cards play at the root, what the last
/// iteration valued each of its actions at for our cards, in chips, and which one was drawn.
#[derive(Clone, Debug, PartialEq)]
pub struct Resolved {
    pub action: Action,
    pub strategy: Vec<(Abstract, f64)>,
    pub values: Vec<f64>,
    pub taken: usize,
}

/// Re-solves `spot`, or `None` when it is not heads up after the flop, or our cards are unknown.
/// `exploit` is the exploit budget, 0 to 1: how far the villain is held to `model` past the depth
/// limit. With no model there is nothing to hold them to.
pub fn resolve(
    spot: &Spot,
    model: Option<&ActionModel>,
    exploit: f64,
    iterations: u32,
    seed: u64,
) -> Option<Resolved> {
    if spot.opponents != 1 || spot.board.len() < 3 || spot.hole.len() != 2 {
        return None;
    }
    let pairs = combos(spot.board);
    let me = pairs.iter().position(|&pair| pair == spot.hole)?;
    // Without a fitted model the baseline reads the line: a bet still narrows the range.
    let baseline = ActionModel::baseline();
    let pin = model.map_or(0.0, |_| exploit.clamp(0.0, 1.0));
    let model = model.unwrap_or(&baseline);
    let villain = match spot.lines.first() {
        Some(line) if !line.is_empty() => {
            let seen: Vec<Seen> = line.iter().map(Seen::of).collect();
            let read = range(model, &seen, spot.board);
            let mut weights = vec![0.0; pairs.len()];
            for (pair, weight) in read {
                if let Some(at) = pairs.iter().position(|&p| p == pair) {
                    weights[at] = weight;
                }
            }
            weights
        }
        _ => vec![1.0; pairs.len()],
    };
    let call = spot.legal.call.unwrap_or(0);
    let tree = Tree::new(
        spot.pot.saturating_sub(call),
        call,
        spot.stack,
        spot.in_position,
    );
    let ends = runout::runouts(spot.board, &mut Rng::stream(seed, 1))
        .into_iter()
        .map(|full| Showdown::new(&pairs, full))
        .collect();
    let next = match spot.board.len() {
        3 => Some(Street::Turn),
        4 => Some(Street::River),
        _ => None,
    };
    let deep = next.map(|street| {
        let ours = Continuation::of(&baseline, street).portfolio();
        // The villain's model is one choice among the baseline's, so a villain not held to it
        // can still answer us however extreme the model is.
        let mut theirs = ours;
        theirs.rotate_right(1);
        theirs[0] = Continuation::of(model, street);
        Deep {
            portfolio: [ours, theirs],
            first: usize::from(spot.in_position),
            stack: spot.stack,
            pin,
        }
    });
    let leaves = Leaves::new(ends, deep);
    let mut solver = Solver::new(
        &tree,
        Showdown::new(&pairs, spot.board),
        leaves,
        pairs.len(),
    );
    let reach = [vec![1.0; pairs.len()], villain];
    for t in 1..=iterations.max(1) {
        solver.iterate(&reach, t);
    }
    let Node::Act { actions, .. } = &tree.nodes[0] else {
        return None;
    };
    let strategy: Vec<(Abstract, f64)> =
        actions.iter().copied().zip(solver.average(0, me)).collect();
    let mut rng = Rng::seeded(seed);
    let roll = rng.below(1 << 30) as f64 / f64::from(1u32 << 30);
    let mut sum = 0.0;
    let taken = strategy
        .iter()
        .position(|(_, p)| {
            sum += p;
            roll < sum
        })
        .unwrap_or(strategy.len().checked_sub(1)?);
    // A counterfactual value is weighted by the villain's reach around our cards: divided by
    // it, the value is in chips.
    let around = solver.showdown.compatible(&reach[1])[me];
    let values = solver
        .root
        .iter()
        .map(|value| {
            if around > 0.0 {
                value[me] / around
            } else {
                0.0
            }
        })
        .collect();
    Some(Resolved {
        action: to_action(strategy[taken].0, spot),
        strategy,
        values,
        taken,
    })
}

/// A tree action as a legal action at `spot`: a raise-to is our chips in on this street plus
/// what the action adds, kept inside the raises offered, and a call when no raise is.
fn to_action(action: Abstract, spot: &Spot) -> Action {
    match action {
        Abstract::Fold => Action::Fold,
        Abstract::Check => Action::Check,
        Abstract::Call => Action::Call,
        Abstract::Bet(chips) => match spot.legal.raise {
            Some(range) => Action::RaiseTo((spot.committed + chips).clamp(range.min, range.max)),
            None if spot.legal.call.is_some() => Action::Call,
            None => Action::Check,
        },
    }
}

/// Regrets and strategy sums per acting node, action and pair.
struct Solver<'a> {
    tree: &'a Tree,
    showdown: Showdown,
    leaves: Leaves,
    /// The iteration being run.
    turn: u32,
    /// Each leaf's regrets over the continuations.
    choices: Vec<Choice>,
    regrets: Vec<Vec<Vec<f64>>>,
    sums: Vec<Vec<Vec<f64>>>,
    /// The root's value per action and pair, for its actor, as of the last iteration.
    root: Vec<Vec<f64>>,
}

impl<'a> Solver<'a> {
    fn new(tree: &'a Tree, showdown: Showdown, leaves: Leaves, pairs: usize) -> Self {
        let shape: Vec<Vec<Vec<f64>>> = tree
            .nodes
            .iter()
            .map(|node| match node {
                Node::Act { actions, .. } => vec![vec![0.0; pairs]; actions.len()],
                _ => Vec::new(),
            })
            .collect();
        Self {
            tree,
            showdown,
            leaves,
            turn: 0,
            choices: vec![Choice::default(); tree.nodes.len()],
            regrets: shape.clone(),
            sums: shape,
            root: Vec::new(),
        }
    }

    /// One DCFR iteration: each player traverses once.
    fn iterate(&mut self, reach: &[Vec<f64>; 2], turn: u32) {
        self.turn = turn;
        let t = f64::from(turn);
        let positive = t.powf(1.5) / (t.powf(1.5) + 1.0);
        let weight = (t / (t + 1.0)).powi(2);
        for player in 0..2 {
            self.walk(
                0,
                player,
                &reach[player],
                &reach[1 - player],
                (positive, weight),
            );
        }
    }

    /// The traverser's counterfactual values at `node`, per pair.
    fn walk(
        &mut self,
        node: usize,
        player: usize,
        mine: &[f64],
        theirs: &[f64],
        discount: (f64, f64),
    ) -> Vec<f64> {
        match &self.tree.nodes[node] {
            Node::Fold { folder, added } => {
                let win = if *folder == player {
                    -(added[player] as f64)
                } else {
                    (self.tree.base + added[1 - player]) as f64
                };
                self.showdown
                    .compatible(theirs)
                    .into_iter()
                    .map(|c| win * c)
                    .collect()
            }
            Node::Leaf { added } => {
                let pot = (self.tree.base + added[0] + added[1]) as f64;
                self.leaves.value(
                    player,
                    pot,
                    *added,
                    theirs,
                    &mut self.choices[node],
                    (self.turn, discount.0),
                )
            }
            Node::Act {
                player: actor,
                children,
                ..
            } => {
                let (actor, children) = (*actor, children.clone());
                let strategy = self.current(node);
                let n = mine.len();
                if actor == player {
                    let mut values = Vec::with_capacity(children.len());
                    let mut total = vec![0.0; n];
                    for (a, &child) in children.iter().enumerate() {
                        let reach: Vec<f64> =
                            mine.iter().zip(&strategy[a]).map(|(r, p)| r * p).collect();
                        let value = self.walk(child, player, &reach, theirs, discount);
                        for h in 0..n {
                            total[h] += strategy[a][h] * value[h];
                        }
                        values.push(value);
                    }
                    let (positive, weight) = discount;
                    for (a, value) in values.iter().enumerate() {
                        for h in 0..n {
                            let r = &mut self.regrets[node][a][h];
                            *r *= if *r > 0.0 { positive } else { 0.5 };
                            *r += value[h] - total[h];
                            let s = &mut self.sums[node][a][h];
                            *s = *s * weight + mine[h] * strategy[a][h];
                        }
                    }
                    if node == 0 {
                        self.root = values;
                    }
                    total
                } else {
                    let mut total = vec![0.0; n];
                    for (a, &child) in children.iter().enumerate() {
                        let reach: Vec<f64> = theirs
                            .iter()
                            .zip(&strategy[a])
                            .map(|(r, p)| r * p)
                            .collect();
                        let value = self.walk(child, player, mine, &reach, discount);
                        for h in 0..n {
                            total[h] += value[h];
                        }
                    }
                    total
                }
            }
        }
    }

    /// Regret matching: each pair plays its actions in proportion to their positive regret, or
    /// evenly when none is positive.
    fn current(&self, node: usize) -> Vec<Vec<f64>> {
        let regrets = &self.regrets[node];
        let (actions, n) = (regrets.len(), regrets[0].len());
        let mut strategy = vec![vec![0.0; n]; actions];
        for h in 0..n {
            let total: f64 = regrets.iter().map(|r| r[h].max(0.0)).sum();
            for a in 0..actions {
                strategy[a][h] = if total > 0.0 {
                    regrets[a][h].max(0.0) / total
                } else {
                    1.0 / actions as f64
                };
            }
        }
        strategy
    }

    /// The average strategy pair `h` plays at `node`.
    fn average(&self, node: usize, h: usize) -> Vec<f64> {
        let sums: Vec<f64> = self.sums[node].iter().map(|s| s[h]).collect();
        let total: f64 = sums.iter().sum();
        if total > 0.0 {
            sums.iter().map(|s| s / total).collect()
        } else {
            vec![1.0 / sums.len() as f64; sums.len()]
        }
    }
}

/// The showdown sweep, for tests that check it against a brute-force count.
pub fn showdown_against(pairs: &[CardSet], board: CardSet, reach: &[f64]) -> (Vec<f64>, Vec<f64>) {
    Showdown::new(pairs, board).against(reach)
}

/// One continuation round on the full `board`, for tests that check it against a count pair by
/// pair: each pair's value when it plays `mine` and the other seat, with `reach`, plays `theirs`.
#[allow(clippy::too_many_arguments)]
pub fn continued_against(
    pairs: &[CardSet],
    board: CardSet,
    reach: &[f64],
    mine: Continuation,
    theirs: Continuation,
    first: bool,
    (pot, paid, bet): (f64, f64, f64),
) -> Vec<f64> {
    let end = Showdown::new(pairs, board);
    let round = Round {
        end: &end,
        first,
        pot,
        paid,
        bet,
    };
    let mut values = vec![[0.0; CHOICES]; pairs.len()];
    round.add(
        &[mine; CHOICES],
        &[theirs; CHOICES],
        reach,
        &vec![[1.0 / CHOICES as f64; CHOICES]; pairs.len()],
        &vec![1.0; pairs.len()],
        &mut values,
    );
    values.into_iter().map(|value| value[0]).collect()
}
