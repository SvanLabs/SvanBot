//! One bot's reads of the opponents it has played: each opponent's actions and how they answered
//! bets, keyed by the venue name the hand named them with. A read is never shared between bots
//! (`docs/adr/0004-reads-are-per-bot.md`).
//!
//! An opponent's class comes from [`classify`] over their hands in order. A switching opponent's
//! read restarts at the change, and an unknown one earns nothing. What a read earns is the class's
//! share of the budget, times `n / (n + half)` for its `n` observations ([`Trust::DEFAULT`]'s
//! half-trust point), the same weight [`personal`](svanbot_strategy::read::personal) tilts by.

use std::collections::BTreeMap;

use svanbot_strategy::action::{ActionModel, Seen, Situation};
use svanbot_strategy::policy::Personal;
use svanbot_strategy::read::{Class, Read, Trust, personal};

use crate::adapt::classify;

/// Whether reads decide in play, live and in the Arena. They stay held until `svanbot paired
/// reads` passes: its whole interval above zero and the exploit gate cleared (V11-35). A change
/// that turns this on is a behaviour change, and its PR quotes that passing line. Held, the bot
/// plays exactly as it would with no reads at all.
pub const IN_PLAY: bool = false;

/// What one hand showed of one opponent.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Witness {
    /// The venue name the hand gave the seat. A hand that names no seat has no witness.
    pub name: String,
    /// The opponent's actions in the hand, oldest first.
    pub actions: Vec<Seen>,
    /// Whether the opponent bet or raised in the hand: the per-hand tally the change point reads.
    pub aggressive: bool,
    /// Whether they folded to one of our bets, when they faced one. The first answer in the hand
    /// stands: the response test asks how they answer us, not how often.
    pub vs_us: Option<bool>,
    /// Whether they folded to someone else's bet, when they faced one.
    pub vs_others: Option<bool>,
}

/// One opponent as this bot has seen them: their hands in order, and the read they give.
#[derive(Clone, Debug, PartialEq)]
pub struct Opponent {
    name: String,
    /// Each hand's actions, oldest first.
    hands: Vec<Vec<Seen>>,
    tallies: Vec<bool>,
    vs_us: Vec<bool>,
    vs_others: Vec<bool>,
    class: Class,
    /// The hand their present strategy began at, for a switching opponent.
    change: Option<usize>,
    read: Read,
    /// Actions in the read: those from the change on.
    actions: u32,
    /// Hands counted into the read so far.
    counted: usize,
}

impl Opponent {
    fn new(name: String) -> Self {
        Self {
            name,
            hands: Vec::new(),
            tallies: Vec::new(),
            vs_us: Vec::new(),
            vs_others: Vec::new(),
            class: Class::Unknown,
            change: None,
            read: Read::default(),
            actions: 0,
            counted: 0,
        }
    }

    fn observe(&mut self, witness: Witness) {
        self.push(witness);
        self.settle();
    }

    /// Adds a hand without classifying: a bulk load classifies once, at the end.
    fn push(&mut self, witness: Witness) {
        self.tallies.push(witness.aggressive);
        self.vs_us.extend(witness.vs_us);
        self.vs_others.extend(witness.vs_others);
        self.hands.push(witness.actions);
    }

    /// Classifies the hands so far, and brings the read up to date with the change point. Each
    /// classification reads the whole series, which is why a bulk load settles only once.
    fn settle(&mut self) {
        let (class, change) = classify(&self.tallies, &self.vs_us, &self.vs_others);
        let change = change.map(|change| change.at);
        if change != self.change {
            // The read restarts where the present strategy began.
            self.read = Read::default();
            self.actions = 0;
            self.counted = change.unwrap_or(0);
        }
        for hand in &self.hands[self.counted..] {
            count(&mut self.read, &mut self.actions, hand);
        }
        self.counted = self.hands.len();
        self.class = class;
        self.change = change;
    }

    /// The venue name.
    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn class(&self) -> Class {
        self.class
    }

    /// The hand their present strategy began at, when they switched.
    pub fn change(&self) -> Option<usize> {
        self.change
    }

    /// Hands seen.
    pub fn hands(&self) -> usize {
        self.hands.len()
    }

    /// Actions in the read.
    pub fn actions(&self) -> u32 {
        self.actions
    }

    /// The read: their moves from the change on.
    pub fn read(&self) -> &Read {
        &self.read
    }

    /// The share of the budget the read has earned over all its actions.
    pub fn earned(&self) -> f64 {
        self.class.budget() * weight(self.actions)
    }

    /// The share of the budget the read has earned at `situation`.
    pub fn trust(&self, situation: Situation) -> f64 {
        self.class.budget() * weight(self.read.n(situation))
    }
}

fn count(read: &mut Read, actions: &mut u32, hand: &[Seen]) {
    for seen in hand {
        read.observe(seen);
        *actions += 1;
    }
}

fn weight(n: u32) -> f64 {
    let n = f64::from(n);
    n / (n + Trust::DEFAULT.half)
}

/// Every opponent this bot has seen, by venue name. A name matches in any case, as the venue's
/// names are compared everywhere else.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Reads {
    opponents: BTreeMap<String, Opponent>,
}

impl Reads {
    /// Takes one hand's witness of one opponent.
    pub fn observe(&mut self, witness: Witness) {
        self.entry(&witness.name).observe(witness);
    }

    /// The reads `witnesses` give, oldest first: the same as observing them one by one, but each
    /// opponent is classified once, at the end, so a store's worth of hands loads in linear time.
    pub fn from_witnesses(witnesses: impl IntoIterator<Item = Witness>) -> Self {
        let mut reads = Self::default();
        for witness in witnesses {
            reads.entry(&witness.name).push(witness);
        }
        for opponent in reads.opponents.values_mut() {
            opponent.settle();
        }
        reads
    }

    fn entry(&mut self, name: &str) -> &mut Opponent {
        self.opponents
            .entry(name.to_lowercase())
            .or_insert_with(|| Opponent::new(name.to_owned()))
    }

    pub fn get(&self, name: &str) -> Option<&Opponent> {
        self.opponents.get(&name.to_lowercase())
    }

    /// Every opponent, by name.
    pub fn opponents(&self) -> impl Iterator<Item = &Opponent> {
        self.opponents.values()
    }

    /// The decision's reads of the opponents `seated` (seat, venue name): each one's model is
    /// `population` tilted toward their read, as far as `exploit` times their class's share allows.
    /// One never seen is unknown, trusted with nothing, and read by `population`.
    pub fn personal(
        &self,
        seated: &[(usize, &str)],
        population: &ActionModel,
        exploit: f64,
    ) -> Vec<Personal> {
        seated
            .iter()
            .map(|&(seat, name)| match self.get(name) {
                Some(opponent) => {
                    let class = opponent.class();
                    let trust = Trust {
                        budget: exploit * class.budget(),
                        ..Trust::DEFAULT
                    };
                    Personal {
                        seat,
                        model: personal(population, opponent.read(), trust),
                        class,
                        trust: Some(opponent.earned()),
                    }
                }
                None => Personal {
                    seat,
                    model: population.clone(),
                    class: Class::Unknown,
                    trust: Some(0.0),
                },
            })
            .collect()
    }

    /// The reads a decision in play takes: [`Reads::personal`] once reads are [`IN_PLAY`], and
    /// [`Reads::held`] until then. Live play and the Arena read by this; the reads' own sim reads
    /// by [`Reads::personal`], since it is what measures whether they may come into play.
    pub fn deciding(
        &self,
        seated: &[(usize, &str)],
        population: &ActionModel,
        exploit: f64,
    ) -> Vec<Personal> {
        if IN_PLAY {
            self.personal(seated, population, exploit)
        } else {
            self.held(seated, population)
        }
    }

    /// The opponents `seated`, each named with their class but read by `population`: reads that
    /// have not cleared their gate are kept and shown, and decide nothing ([`IN_PLAY`]).
    pub fn held(&self, seated: &[(usize, &str)], population: &ActionModel) -> Vec<Personal> {
        seated
            .iter()
            .map(|&(seat, name)| Personal {
                seat,
                model: population.clone(),
                class: self.get(name).map_or(Class::Unknown, Opponent::class),
                trust: None,
            })
            .collect()
    }
}
