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
use crate::read::Class;
pub use crate::resolve::Abstract;
use crate::resolve::resolve;

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
    /// The rest of the street was re-solved, and an action drawn from its average strategy. The
    /// glossary calls the work the re-solve, so the record says `resolved`, never `solved`.
    Resolved,
    /// The pricer priced the spot by our equity: before the flop, or in a pot of three or more.
    Priced,
}

impl Branch {
    /// The branch's name, as the store keeps it and the page shows it.
    pub fn name(self) -> &'static str {
        match self {
            Self::Resolved => "resolved",
            Self::Priced => "priced",
        }
    }

    /// The branch called `name`, or `None` when no branch is.
    pub fn named(name: &str) -> Option<Self> {
        [Self::Resolved, Self::Priced]
            .into_iter()
            .find(|branch| branch.name() == name)
    }
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
    /// Our two hole cards, and the board as it stood: a replayer cannot say what the bot held
    /// otherwise, and the ranges below are read on this board.
    pub hole: CardSet,
    pub board: CardSet,
    /// The re-solve's record, when it decided.
    pub resolved: Option<ResolvedTrace>,
    /// The pricer's record, when it decided.
    pub priced: Option<PricedTrace>,
}

/// What the re-solve decided from: what the last iteration valued each action at for our cards,
/// the mix it drew from, and the range it read the villain at. `actions` names each entry of
/// `values` and `strategy`, in order, so "action 2" reads as the call or bet it was.
#[derive(Clone, Debug, PartialEq)]
pub struct ResolvedTrace {
    pub values: Vec<f64>,
    pub strategy: Vec<f64>,
    pub taken: usize,
    pub actions: Vec<Abstract>,
    pub ranges: Vec<Ranged>,
}

/// One opponent's range, summarised.
#[derive(Clone, Debug, PartialEq)]
pub struct Ranged {
    /// The opponent's seat at the table, as the spot carried it, so a reader can match the range
    /// to a seat in the table view. `None` when the spot named no seats: a sim's own table, or a
    /// caller that did not follow the hand. The ordinal (the line's index in `Spot::lines`) is
    /// never written here in its place — a seat guessed from a position is a wrong label that
    /// reads as a right one.
    pub seat: Option<usize>,
    /// The mass in each strength bucket on the board, weakest first.
    pub masses: Vec<f64>,
    /// The opponent's adaptivity class, when their own read decided the range; `None` when the
    /// population's model read them.
    pub class: Option<Class>,
    /// The share of the exploit budget their read had earned, with the class.
    pub trust: Option<f64>,
}

/// One opponent's own read, as the decision takes it: the model their line is read by, and the
/// class and trust it carries for the trace.
#[derive(Clone, Debug, PartialEq)]
pub struct Personal {
    /// The seat, as [`Spot::seats`] names it.
    pub seat: usize,
    /// The population's model tilted toward their read ([`crate::read::personal`]).
    pub model: ActionModel,
    pub class: Class,
    pub trust: f64,
}

/// What the pricer decided from: our equity against the opponents' ranges, the price of the call
/// it was measured against, and how far the exploit gate let the reads move it.
#[derive(Clone, Debug, PartialEq)]
pub struct PricedTrace {
    pub equity: f64,
    /// `None` when the check was free, since a free check has no price.
    pub price: Option<f64>,
    /// The exploit budget the decision read by, when a fitted model passed the gate; `None` when
    /// the baseline decided, since the baseline exploits nothing.
    pub exploit: Option<f64>,
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
    decide_read(spot, model, &[], params, budget, iterations)
}

/// [`decide_drawn`] with each opponent in `personal` read by their own model: the re-solve's
/// villain and each of the pricer's lines. An opponent with no read is read by `model`.
pub fn decide_read(
    spot: &Spot,
    model: Option<&ActionModel>,
    personal: &[Personal],
    params: &Params,
    budget: Budget,
    iterations: u32,
) -> Decision {
    // Until a fit passes the gate, the baseline reads the opponents' lines. A model present is a
    // model that passed it (`svanbot fit`), so its presence is the gate's answer.
    let gated = model.is_some();
    let baseline = ActionModel::baseline();
    let population = model.unwrap_or(&baseline);
    // Each line's own read, by the seat the spot names it with.
    let reads: Vec<Option<&Personal>> = (0..spot.lines.len())
        .map(|ordinal| {
            let seat = spot.seats.get(ordinal)?;
            personal.iter().find(|read| read.seat == *seat)
        })
        .collect();
    let models: Vec<&ActionModel> = reads
        .iter()
        .map(|read| read.map_or(population, |read| &read.model))
        .collect();
    let model = models.first().copied().unwrap_or(population);
    let street = Street::of(spot.board);
    match resolve(spot, Some(model), params.exploit, iterations, budget.seed) {
        Some(resolved) => {
            let strategy: Vec<f64> = resolved
                .strategy
                .iter()
                .map(|&(_, chance)| chance)
                .collect();
            Decision {
                action: resolved.action,
                drawn: Some(Drawn {
                    strategy: strategy.clone(),
                    values: resolved.values.clone(),
                    taken: resolved.taken,
                }),
                trace: Some(Trace {
                    branch: Branch::Resolved,
                    street,
                    hole: spot.hole,
                    board: spot.board,
                    resolved: Some(ResolvedTrace {
                        values: resolved.values,
                        strategy,
                        taken: resolved.taken,
                        actions: resolved
                            .strategy
                            .into_iter()
                            .map(|(action, _)| action)
                            .collect(),
                        ranges: read_ranges(spot, population, &reads, 1),
                    }),
                    priced: None,
                }),
            }
        }
        None => {
            let priced = pricer::price_lines(spot, &models, budget, params);
            Decision {
                action: priced.action,
                drawn: None,
                trace: Some(Trace {
                    branch: Branch::Priced,
                    street,
                    hole: spot.hole,
                    board: spot.board,
                    resolved: None,
                    priced: Some(PricedTrace {
                        equity: priced.equity,
                        // What the pricer compared against, not a second reading of the spot.
                        price: priced.price,
                        exploit: gated.then_some(params.exploit),
                        ranges: read_ranges(spot, population, &reads, spot.opponents.max(1)),
                    }),
                }),
            }
        }
    }
}

/// The opponents' ranges from `model` and their lines, summarised by strength bucket: one per
/// opponent, `count` of them, the ones with no line read at the uniform range. `dead` is every
/// card the range must miss. Each range names its opponent's seat when the spot carries one.
fn read_ranges(
    spot: &Spot,
    population: &ActionModel,
    reads: &[Option<&Personal>],
    count: usize,
) -> Vec<Ranged> {
    let ranked = ranked(spot.board);
    let dead = spot.hole | spot.board;
    (0..count)
        .map(|ordinal| {
            let read = reads.get(ordinal).copied().flatten();
            let model = read.map_or(population, |read| &read.model);
            Ranged {
                seat: spot.seats.get(ordinal).copied(),
                masses: match spot.lines.get(ordinal) {
                    Some(line) if !line.is_empty() => {
                        let seen: Vec<Seen> = line.iter().map(Seen::of).collect();
                        masses(&range(model, &seen, dead), &ranked)
                    }
                    // No line is no information: every pair equally likely, which is uniform
                    // over the buckets too. An all-zero row would read as "nothing is possible".
                    _ => vec![1.0 / BUCKETS as f64; BUCKETS],
                },
                class: read.map(|read| read.class),
                trust: read.map(|read| read.trust),
            }
        })
        .collect()
}
