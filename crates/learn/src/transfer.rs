//! Gene transfer (V11-80, spec Phase 2 item 5): a candidate that cleared one bot's search,
//! offered to another bot's search.
//!
//! An offer is the donor's parameter set, the numbers only. It carries no verdict, no seed and no
//! read of an opponent (`docs/adr/0004-reads-are-per-bot.md`): the receiver screens and confirms
//! it against its own champion, on its own deals, with its own log. A transfer may only be a
//! candidate inside the receiver's style: a number the style pins cannot move, and one it bounds
//! must arrive inside the bound. Offers for the same receiver candidate are kept once.

use svanbot_strategy::params::Params;
use svanbot_strategy::style::Style;

/// A move another bot's search found, offered to this bot.
#[derive(Clone, Debug, PartialEq)]
pub struct Offer {
    /// The donor's candidate numbers. No evidence travels with them.
    pub challenger: Params,
}

/// Whether `candidate` is a move for a bot whose incumbent is `incumbent` and whose style is
/// `style`: different from the incumbent, inside the parameter set's own range, and inside every
/// pin and bound the style names.
pub fn allowed(candidate: &Params, incumbent: &Params, style: &Style) -> bool {
    candidate != incumbent
        && candidate.in_range()
        && candidate
            .named()
            .iter()
            .all(|(name, value)| style.allows(name, *value))
}

/// The offer `candidate` makes to `incumbent`'s bot under `style`, or `None` when the move is no
/// move at all. The candidate is brought inside the style first (its numbers outside the style's
/// bounds clamped into them), so a Grinder's move can be tried by a Gambler at the Gambler's end
/// of the same numbers.
pub fn offer(candidate: Params, incumbent: &Params, style: &Style) -> Option<Offer> {
    let challenger = style.seed(candidate);
    allowed(&challenger, incumbent, style).then_some(Offer { challenger })
}

/// Adds `offer` to `offers` unless its receiver candidate is already there. Returns whether it
/// was added.
pub fn push(offers: &mut Vec<Offer>, offer: Offer) -> bool {
    if offers
        .iter()
        .any(|held| held.challenger == offer.challenger)
    {
        return false;
    }
    offers.push(offer);
    true
}
