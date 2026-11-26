//! The decision's record: what was taken, and what it was taken from. A replayer answers
//! "why did it do that?" from this rather than by playing the hand again.

use svanbot_core::card::CardSet;
use svanbot_core::rules::{Action, Street};

use crate::action::ActionModel;
use crate::read::{Class, Read};
use crate::resolve::Abstract;

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
