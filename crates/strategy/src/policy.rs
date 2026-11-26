//! The policy: how the bot decides at any spot. A heads-up spot after the flop is re-solved
//! ([`crate::resolve`]); the pricer ([`crate::pricer`]) decides every other one.
//!
//! The compute budget comes in from the caller, as the pricer's samples and the re-solve's
//! iterations ([`crate::resolve::iterations_for`]), so the policy never reads a clock.

use svanbot_core::rules::{Action, Street};
use svanbot_core::spot::Spot;

use crate::action::{ActionModel, BUCKETS, Seen, Situation, masses, range, ranked};
use crate::kind::{BALANCED_EXPLOIT_CAP, PolicyKind};
use crate::params::Params;
use crate::playbook;
use crate::pricer::{self, Budget};
use crate::profile::{ACTIVE, EMPTY, Profile, SeatedStats};
use crate::read::Class;
pub use crate::resolve::Abstract;
use crate::resolve::resolve;
use crate::style::{STANDARD, Style};

mod trace;
pub use trace::{Branch, Decision, Drawn, Personal, PricedTrace, Ranged, ResolvedTrace, Trace};

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
            // The exploiting kind pins the re-solve to the read: its default treatment is the
            // evidence cap, which an experiment may still override.
            continuation_pin: match kind {
                PolicyKind::Exploiting => ContinuationPin::EvidenceCap,
                _ => ContinuationPin::Fixed,
            },
        }
    }

    pub fn reference(params: Params) -> Self {
        Self::new(PolicyKind::Solving, &STANDARD, params)
    }

    pub fn name(&self) -> String {
        format!("{}/{}", self.kind.name(), self.style.name)
    }

    /// Whether a decision by this policy reads the opponents' stats: its style's profile has
    /// rules to apply, or its kind's playbook reads them.
    pub fn uses_stats(&self) -> bool {
        !self.style.profile.is_empty() || self.kind == PolicyKind::Exploiting
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
        // A caller may pass any profile; Balanced decides without one, and the exploiting kind's
        // playbook replaces the style's profile with the rule set its read type chooses.
        let seated = profile.opponents();
        let profile = match self.kind {
            PolicyKind::Balanced | PolicyKind::Exploiting => EMPTY,
            _ => profile,
        };
        let read_action = if self.kind == PolicyKind::Exploiting {
            playbook::decide(&self.params, spot, budget.seed, personal, seated)
        } else {
            profile.action(spot, budget.seed)
        };
        if let Some(action) = read_action {
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
            PolicyKind::Solving | PolicyKind::Exploiting => {
                resolve(spot, Some(model), pin, iterations, budget.seed)
            }
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
