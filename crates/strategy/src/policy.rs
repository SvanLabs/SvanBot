//! The policy: how the bot decides at any spot. A heads-up spot after the flop is re-solved
//! ([`crate::resolve`]); the pricer ([`crate::pricer`]) decides every other one.
//!
//! The compute budget comes in from the caller, as the pricer's samples and the re-solve's
//! iterations ([`crate::resolve::iterations_for`]), so the policy never reads a clock.

use svanbot_core::card::CardSet;
use svanbot_core::rules::{Action, Street};
use svanbot_core::spot::Spot;

use crate::action::{ActionModel, BUCKETS, Seen, Situation, masses, range, ranked};
use crate::kind::{BALANCED_EXPLOIT_CAP, PolicyKind};
use crate::params::Params;
use crate::pricer::{self, Budget};
use crate::profile::{ACTIVE, EMPTY, Profile, SeatedStats};
use crate::read::{Class, Read};
pub use crate::resolve::Abstract;
use crate::resolve::resolve;
use crate::style::{STANDARD, Style};

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
/// Each range carries the class and share of the budget its opponent's read earned.
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
    /// The share of the exploit budget the read earned, or `None` when it is held and decides
    /// nothing: the line is then read by the population's model.
    pub trust: Option<f64>,
    /// The evidence behind the read, when the caller kept it, so a decision can cap a pin by
    /// what the read knows at the situation that follows. A held read keeps its evidence here
    /// but decides nothing: `trust` is `None` or zero.
    pub read: Option<Read>,
}

/// What the pricer decided from: our equity against the opponents' ranges, the price of the call
/// it was measured against, and the configured exploit budget when playing by reads.
#[derive(Clone, Debug, PartialEq)]
pub struct PricedTrace {
    pub equity: f64,
    /// `None` when the check was free, since a free check has no price.
    pub price: Option<f64>,
    /// The configured budget when a fitted model or a trusted opponent read was in use.
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
    decide_profile(spot, model, personal, params, budget, iterations, ACTIVE)
}

/// Evaluate the profile first, then use the same re-solve or pricer for unmatched spots.
pub fn decide_profile(
    spot: &Spot,
    model: Option<&ActionModel>,
    personal: &[Personal],
    params: &Params,
    budget: Budget,
    iterations: u32,
    profile: Profile<'_>,
) -> Decision {
    Policy::reference(*params)
        .decide_with_profile(spot, model, personal, budget, iterations, profile)
}

/// A policy's decision logic, style and parameter set.
#[derive(Clone, Copy, Debug)]
pub struct Policy {
    pub kind: PolicyKind,
    pub style: &'static Style,
    pub params: Params,
    continuation_pin: ContinuationPin,
}

/// Which share of the next-street opponent continuation the re-solve pins to its selected model.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ContinuationPin {
    Fixed,
    Zero,
    EvidenceCap,
}

impl Policy {
    pub fn new(kind: PolicyKind, style: &'static Style, params: Params) -> Self {
        Self {
            kind,
            style,
            params,
            continuation_pin: ContinuationPin::Fixed,
        }
    }

    pub fn reference(params: Params) -> Self {
        Self::new(PolicyKind::Solving, &STANDARD, params)
    }

    pub fn name(&self) -> String {
        format!("{}/{}", self.kind.name(), self.style.name)
    }

    /// The exploit budget this policy may spend. `Balanced` caps it low even when its slot was
    /// seeded before the cap existed; every other kind spends the number it was given.
    pub fn exploit(&self) -> f64 {
        if self.kind == PolicyKind::Balanced {
            self.params.exploit.clamp(0.0, BALANCED_EXPLOIT_CAP)
        } else {
            self.params.exploit
        }
    }

    /// Varies the continuation pin for an experiment while leaving the selected range model and
    /// the read-mixing parameters in place.
    #[must_use]
    pub fn with_continuation_pin(mut self, continuation_pin: ContinuationPin) -> Self {
        self.continuation_pin = continuation_pin;
        self
    }

    pub fn decide(
        &self,
        spot: &Spot,
        model: Option<&ActionModel>,
        personal: &[Personal],
        budget: Budget,
        iterations: u32,
        stats: &[SeatedStats],
    ) -> Decision {
        let profile = self.style.profile.with_opponents(stats);
        self.decide_with_profile(spot, model, personal, budget, iterations, profile)
    }

    fn decide_with_profile(
        &self,
        spot: &Spot,
        model: Option<&ActionModel>,
        personal: &[Personal],
        budget: Budget,
        iterations: u32,
        profile: Profile<'_>,
    ) -> Decision {
        // A caller may pass any profile; the Balanced kind still decides without one.
        let profile = if self.kind == PolicyKind::Balanced {
            EMPTY
        } else {
            profile
        };
        if let Some(action) = profile.action(spot, budget.seed) {
            return action.into();
        }

        // Until a fit passes the gate, the baseline reads the opponents' lines. A model present is a
        // model that passed it (`svanbot fit`), so its presence is the gate's answer.
        let gated = model.is_some();
        let baseline = ActionModel::baseline();
        let population = model.unwrap_or(&baseline);
        // Each line's own read, by the seat the spot names it with. Balanced holds every read:
        // its lines are read by the population, as if no read existed.
        let reads: Vec<Option<&Personal>> = if self.kind == PolicyKind::Balanced {
            vec![None; spot.lines.len()]
        } else {
            (0..spot.lines.len())
                .map(|ordinal| {
                    let seat = spot.seats.get(ordinal)?;
                    personal.iter().find(|read| read.seat == *seat)
                })
                .collect()
        };
        let models: Vec<&ActionModel> = reads
            .iter()
            .map(|read| read.map_or(population, |read| &read.model))
            .collect();
        let model = models.first().copied().unwrap_or(population);
        let street = Street::of(spot.board);
        let exploit = self.exploit();
        let pin = match self.continuation_pin {
            ContinuationPin::Fixed => exploit,
            ContinuationPin::Zero => 0.0,
            ContinuationPin::EvidenceCap => {
                let trust = reads
                    .first()
                    .copied()
                    .flatten()
                    .and_then(|read| {
                        let next = next_street(street)?;
                        let damp = match read.class {
                            Class::Fixed | Class::Switching => 1.0,
                            // A learner moves with us: half the evidence it would otherwise earn.
                            Class::Learning => 0.5,
                            Class::Unknown => 0.0,
                        };
                        read.read
                            .as_ref()
                            .map(|evidence| {
                                let budget = read.class.budget();
                                let facing = evidence.trust(
                                    Situation {
                                        street: next,
                                        facing: true,
                                    },
                                    budget,
                                );
                                let open = evidence.trust(
                                    Situation {
                                        street: next,
                                        facing: false,
                                    },
                                    budget,
                                );
                                // The lower supported share: a pin needs evidence for both
                                // answers the opponent can give on the next street.
                                facing.min(open) * damp
                            })
                            .or_else(|| read.trust.map(|trust| trust * damp))
                    })
                    .filter(|trust| trust.is_finite())
                    .unwrap_or(0.0);
                exploit.clamp(0.0, 1.0) * trust.clamp(0.0, 1.0)
            }
        };
        let resolved = match self.kind {
            PolicyKind::Solving => resolve(spot, Some(model), pin, iterations, budget.seed),
            PolicyKind::Balanced | PolicyKind::Pricing => None,
        };
        match resolved {
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
                // Balanced draws among near-best actions and blends the opening boundary;
                // Pricing takes the best, as it always did.
                let (priced, drawn) = match self.kind {
                    PolicyKind::Balanced => {
                        let balanced = pricer::price_lines_balanced(
                            spot,
                            &models,
                            budget,
                            self.style.open_scale,
                            self.params.spread,
                        );
                        let drawn = (!balanced.strategy.is_empty()).then(|| Drawn {
                            strategy: balanced.strategy.clone(),
                            values: balanced.values.clone(),
                            taken: balanced.taken,
                        });
                        (
                            pricer::Decision {
                                action: balanced.action,
                                equity: balanced.equity,
                                price: balanced.price,
                            },
                            drawn,
                        )
                    }
                    _ => (
                        pricer::price_lines_at(spot, &models, budget, self.style.open_scale),
                        None,
                    ),
                };
                Decision {
                    action: priced.action,
                    drawn,
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
                            exploit: (gated
                                || reads
                                    .iter()
                                    .flatten()
                                    .any(|read| read.trust.is_some_and(|trust| trust > 0.0)))
                            .then_some(exploit),
                            ranges: read_ranges(spot, population, &reads, spot.opponents.max(1)),
                        }),
                    }),
                }
            }
        }
    }
}

/// The street after `street`, or `None` on the river, where no continuation is left to pin.
fn next_street(street: Street) -> Option<Street> {
    let at = Street::ALL.iter().position(|&each| each == street)?;
    Street::ALL.get(at + 1).copied()
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
                trust: read.and_then(|read| read.trust),
            }
        })
        .collect()
}
