//! The policy: how the bot decides at any spot. A heads-up spot after the flop is re-solved
//! ([`crate::resolve`]); the pricer ([`crate::pricer`]) decides every other one.
//!
//! The compute budget comes in from the caller, as the pricer's samples and the re-solve's
//! iterations ([`crate::resolve::iterations_for`]), so the policy never reads a clock.

use svanbot_core::card::CardSet;
use svanbot_core::rules::{Action, Street};
use svanbot_core::spot::Spot;

use crate::action::{ActionModel, BUCKETS, Seen, masses, range, ranked};
use crate::params::Params;
use crate::pricer::{self, Budget};
use crate::resolve::{Abstract, resolve};

/// The action to play at `spot`: the re-solve's after `iterations` when it covers the spot, and
/// the pricer's otherwise. Both read the opponents' ranges from their lines, by `model` or, with
/// none, by the baseline model. `params` is the parameter set it plays by.
pub fn decide(
    spot: &Spot,
    model: Option<&ActionModel>,
    params: &Params,
    budget: Budget,
    iterations: u32,
) -> Action {
    decide_drawn(spot, model, params, budget, iterations).action
}

/// A decision, and what it was drawn from when it was drawn.
#[derive(Clone, Debug, PartialEq)]
pub struct Decision {
    pub action: Action,
    /// The re-solve's draw. The pricer draws nothing: it takes its best action every time.
    pub drawn: Option<Drawn>,
    /// What made it: the record the hand's replayer reads. `None` for a decision the caller made
    /// without the policy — a test's fixed action, say — since a trace nobody took is worse than
    /// no trace, and the replayer says which it is holding.
    pub trace: Option<Trace>,
}

impl From<Action> for Decision {
    /// A decision with no trace behind it: the action, and nothing about why.
    fn from(action: Action) -> Self {
        Self {
            action,
            drawn: None,
            trace: None,
        }
    }
}

/// Which of the policy's two ways decided.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Branch {
    /// The rest of the street was re-solved, and an action drawn from its average strategy.
    Solved,
    /// The pricer priced the spot by our equity: before the flop, or in a pot of three or more.
    Priced,
}

/// What a decision was made from, kept with its hand so "why did it do that?" is answered from the
/// record rather than by playing the hand again.
///
/// It carries only what the decision path knows. An opponent's class and an exploit budget spent
/// and left are not among them: nothing computes either while a bot plays (V11-19).
#[derive(Clone, Debug, PartialEq)]
pub struct Trace {
    pub branch: Branch,
    /// The street the decision was made on: the board's length says it.
    pub street: Street,
    /// The re-solve's record, when it decided.
    pub solved: Option<SolvedTrace>,
    /// The pricer's record, when it decided.
    pub priced: Option<PricedTrace>,
}

/// What the re-solve decided from: the actions the root offered, what the last iteration valued
/// each at for our cards, the mix it drew from, and the range it read the villain at.
#[derive(Clone, Debug, PartialEq)]
pub struct SolvedTrace {
    pub actions: Vec<Abstract>,
    pub values: Vec<f64>,
    pub strategy: Vec<f64>,
    pub taken: usize,
    pub ranges: Vec<Ranged>,
}

/// One opponent's range, summarised.
#[derive(Clone, Debug, PartialEq)]
pub struct Ranged {
    /// The seat, counting the seats still in from the button's left, ours included.
    pub seat: usize,
    /// The mass in each strength bucket on the board, weakest first.
    pub masses: Vec<f64>,
}

/// What the pricer decided from: our equity against the opponents' ranges, and the price of the
/// call it was measured against. `None` when the check was free, since a free check has no price.
#[derive(Clone, Debug, PartialEq)]
pub struct PricedTrace {
    pub equity: f64,
    pub price: Option<f64>,
    pub ranges: Vec<Ranged>,
}

/// A draw among actions: the chance each had, what each was valued at in chips, and the one
/// taken. The luck correction of a hand reads it (`learn::probe::corrected`).
#[derive(Clone, Debug, PartialEq)]
pub struct Drawn {
    pub strategy: Vec<f64>,
    pub values: Vec<f64>,
    pub taken: usize,
}

/// [`decide`], with the draw behind the action.
pub fn decide_drawn(
    spot: &Spot,
    model: Option<&ActionModel>,
    params: &Params,
    budget: Budget,
    iterations: u32,
) -> Decision {
    // Until a fit passes the gate, the baseline reads the opponents' lines.
    let baseline = ActionModel::baseline();
    let model = model.unwrap_or(&baseline);
    match resolve(spot, Some(model), params.exploit, iterations, budget.seed) {
        Some(resolved) => Decision {
            action: resolved.action,
            drawn: Some(Drawn {
                strategy: resolved
                    .strategy
                    .iter()
                    .map(|&(_, chance)| chance)
                    .collect(),
                values: resolved.values.clone(),
                taken: resolved.taken,
            }),
            trace: Some(Trace {
                branch: Branch::Solved,
                street: street_of(spot.board),
                solved: Some(SolvedTrace {
                    actions: resolved
                        .strategy
                        .iter()
                        .map(|&(action, _)| action)
                        .collect(),
                    values: resolved.values,
                    strategy: resolved
                        .strategy
                        .iter()
                        .map(|&(_, chance)| chance)
                        .collect(),
                    taken: resolved.taken,
                    ranges: read_ranges(spot, model, 1, spot.board),
                }),
                priced: None,
            }),
        },
        None => {
            let priced = pricer::price(spot, Some(model), budget, params);
            Decision {
                action: priced.action,
                drawn: None,
                trace: Some(Trace {
                    branch: Branch::Priced,
                    street: street_of(spot.board),
                    solved: None,
                    priced: Some(PricedTrace {
                        equity: priced.equity,
                        price: spot
                            .legal
                            .call
                            .map(|call| f64::from(call) / f64::from(spot.pot + call)),
                        ranges: read_ranges(
                            spot,
                            model,
                            spot.opponents.max(1),
                            spot.hole | spot.board,
                        ),
                    }),
                }),
            }
        }
    }
}

/// The street a board of this length is on.
fn street_of(board: CardSet) -> Street {
    match board.len() {
        0 => Street::Preflop,
        3 => Street::Flop,
        4 => Street::Turn,
        _ => Street::River,
    }
}

/// The opponents' ranges from `model` and their lines, summarised by strength bucket: one per
/// opponent, `count` of them, the ones with no line read at the uniform range. `dead` is every
/// card the range must miss.
fn read_ranges(spot: &Spot, model: &ActionModel, count: usize, dead: CardSet) -> Vec<Ranged> {
    let ranked = ranked(spot.board);
    (0..count)
        .map(|seat| Ranged {
            seat,
            masses: match spot.lines.get(seat) {
                Some(line) if !line.is_empty() => {
                    let seen: Vec<Seen> = line.iter().map(Seen::of).collect();
                    masses(&range(model, &seen, dead), &ranked)
                }
                _ => vec![0.0; BUCKETS],
            },
        })
        .collect()
}
