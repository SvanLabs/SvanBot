//! What one observed hand contributes to a profile's opponent stats.

use svanbot_core::rules::Street;
use svanbot_core::spot::{Act, ActKind};
use svanbot_strategy::profile::OpponentStats;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Observation {
    preflop: bool,
    vpip: bool,
    pfr: bool,
    folds: usize,
    faced_bets: usize,
    aggressive: usize,
    non_checks: usize,
}

impl Observation {
    /// Forced blinds do not appear as actions. A preflop check does not put money in,
    /// even when the action model counts it as a call.
    pub fn of(line: &[Act]) -> Self {
        let mut hand = Self::default();
        for act in line {
            if act.street == Street::Preflop {
                hand.preflop = true;
                hand.vpip |= matches!(act.kind, ActKind::Call | ActKind::Bet { .. });
                hand.pfr |= matches!(act.kind, ActKind::Bet { .. });
            } else if act.kind != ActKind::Check {
                hand.non_checks += 1;
                hand.aggressive += usize::from(matches!(act.kind, ActKind::Bet { .. }));
                if act.facing {
                    hand.faced_bets += 1;
                    hand.folds += usize::from(act.kind == ActKind::Fold);
                }
            }
        }
        hand
    }

    pub(crate) fn add_to(self, stats: &mut OpponentStats) {
        let preflop = usize::from(self.preflop);
        stats.vpip.observe(usize::from(self.vpip), preflop);
        stats.pfr.observe(usize::from(self.pfr), preflop);
        stats.fold_to_bet.observe(self.folds, self.faced_bets);
        stats.aggression.observe(self.aggressive, self.non_checks);
    }
}
