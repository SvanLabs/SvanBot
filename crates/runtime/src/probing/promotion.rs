//! The per-bot promotion guard (V11-77): a bot's own proposal starts its candidate in its slot,
//! and its own standing reference arm decides whether the candidate stays. The guard is between
//! hands; the incumbent is restored by any incomplete or harmful ending.

use svanbot_learn::probe::Verdict;
use svanbot_learn::promotion::{Gate, gate};
use svanbot_strategy::kind::PolicyKind;
use svanbot_strategy::style;

use crate::fleet::{Promotion, Status};

use super::Probing;

impl Probing<'_> {
    /// Takes up a per-bot promotion at open: a guard cut off by the last exit restores the
    /// incumbent and is recorded incomplete; a waiting proposal starts only when the live path is
    /// enabled and no fleet probe owns the canary's hands.
    pub(super) fn open_promotion(&mut self, now_ms: i64) -> Result<Option<String>, String> {
        let Some(promotion) = self.fleet.bot_promotion(&self.bot)? else {
            return Ok(None);
        };
        match promotion.status {
            Status::Guarding => {
                self.restore_incumbent(&promotion)?;
                let note = format!(
                    "{}: the promotion guard of {} was cut off; incumbent {} restored",
                    self.bot,
                    promotion.evidence.challenger.digest(),
                    promotion.evidence.incumbent.digest()
                );
                self.fleet
                    .finish_bot_guard(&self.bot, Status::Incomplete, now_ms, &note)?;
                Ok(Some(note))
            }
            Status::Pending => {
                if let Some(why) = self.stale(&promotion)? {
                    self.fleet.supersede_bot(&self.bot, now_ms, &why)?;
                    return Ok(Some(format!("{}: promotion dropped: {why}", self.bot)));
                }
                if !self.live_promotions {
                    return Ok(Some(format!(
                        "{}: promotion {} pending; live promotion is disabled",
                        self.bot,
                        promotion.evidence.challenger.digest()
                    )));
                }
                if self.running.is_some() {
                    return Ok(None);
                }
                self.start_guard(&promotion, now_ms)
            }
            _ => Ok(None),
        }
    }

    /// One between-hands check of the bot's own promotion. A waiting proposal starts a guard when
    /// the live path is enabled; a running guard is read from the bot's standing orbits.
    pub(super) fn promotion_step(&mut self, now_ms: i64) -> Result<Option<String>, String> {
        let Some(promotion) = self.fleet.bot_promotion(&self.bot)? else {
            return Ok(None);
        };
        match promotion.status {
            Status::Pending => {
                if let Some(why) = self.stale(&promotion)? {
                    self.fleet.supersede_bot(&self.bot, now_ms, &why)?;
                    return Ok(Some(format!("{}: promotion dropped: {why}", self.bot)));
                }
                if !self.live_promotions || self.running.is_some() {
                    return Ok(None);
                }
                // The hand just finished played the incumbent; the guard starts after it.
                self.start_guard(&promotion, now_ms.saturating_add(1))
            }
            Status::Guarding => self.guard(&promotion, now_ms),
            _ => Ok(None),
        }
    }

    fn start_guard(
        &mut self,
        promotion: &Promotion,
        started_ms: i64,
    ) -> Result<Option<String>, String> {
        if !self.fleet.start_bot_guard(&self.bot, started_ms)? {
            return Ok(None);
        }
        if let Err(error) = self
            .fleet
            .set_champion(&self.bot, &promotion.evidence.challenger)
        {
            let _ = self.fleet.finish_bot_guard(
                &self.bot,
                Status::Incomplete,
                started_ms,
                "the slot was not written",
            );
            return Err(error);
        }
        self.ledger.adopt(promotion.evidence.challenger);
        Ok(Some(format!(
            "{}: promotion guard started: {} over incumbent {}",
            self.bot,
            promotion.evidence.challenger.digest(),
            promotion.evidence.incumbent.digest()
        )))
    }

    fn guard(&mut self, promotion: &Promotion, now_ms: i64) -> Result<Option<String>, String> {
        if self.fleet.champion_slot(&self.bot)? != Some(promotion.evidence.challenger) {
            let note = format!(
                "{}: promotion dropped: the slot moved during the guard",
                self.bot
            );
            self.fleet.supersede_bot(&self.bot, now_ms, &note)?;
            return Ok(Some(note));
        }
        if self.policy()? != (promotion.evidence.kind, promotion.evidence.style) {
            self.restore_incumbent(promotion)?;
            let note = format!(
                "{}: promotion incomplete: the assignment changed; incumbent {} restored",
                self.bot,
                promotion.evidence.incumbent.digest()
            );
            self.fleet
                .finish_bot_guard(&self.bot, Status::Incomplete, now_ms, &note)?;
            return Ok(Some(note));
        }
        let Some(started_ms) = promotion.started_ms else {
            self.restore_incumbent(promotion)?;
            let note = format!(
                "{}: promotion incomplete: its guard never started; incumbent restored",
                self.bot
            );
            self.fleet
                .finish_bot_guard(&self.bot, Status::Incomplete, now_ms, &note)?;
            return Ok(Some(note));
        };
        let (verdict, edge, radius) =
            self.standing_guard(started_ms, promotion.guard_hands, promotion.harm_margin)?;
        let note = format!(
            "{}: promotion guard {}: {edge:+.1} bb/100, 95% ±{radius:.1}",
            self.bot,
            verdict_name(verdict)
        );
        match verdict {
            Verdict::Running => Ok(None),
            Verdict::Cleared => {
                self.fleet
                    .finish_bot_guard(&self.bot, Status::Cleared, now_ms, &note)?;
                Ok(Some(format!(
                    "{note}; {} stays in the slot",
                    promotion.evidence.challenger.digest()
                )))
            }
            Verdict::Harmful => {
                self.restore_incumbent(promotion)?;
                self.fleet
                    .finish_bot_guard(&self.bot, Status::Harmful, now_ms, &note)?;
                Ok(Some(format!(
                    "{note}; incumbent {} restored",
                    promotion.evidence.incumbent.digest()
                )))
            }
            _ => {
                self.restore_incumbent(promotion)?;
                self.fleet
                    .finish_bot_guard(&self.bot, Status::Incomplete, now_ms, &note)?;
                Ok(Some(format!(
                    "{note}; incumbent {} restored",
                    promotion.evidence.incumbent.digest()
                )))
            }
        }
    }

    /// Whether a waiting proposal still names this bot's slot and assignment and its evidence is
    /// still admitted.
    fn stale(&self, promotion: &Promotion) -> Result<Option<String>, String> {
        match self.fleet.champion_slot(&self.bot)? {
            Some(slot) if slot == promotion.evidence.incumbent => {}
            Some(_) => return Ok(Some("the slot is no longer the incumbent".into())),
            None => return Ok(Some("the slot is gone".into())),
        }
        if self.policy()? != (promotion.evidence.kind, promotion.evidence.style) {
            return Ok(Some("the assignment changed".into()));
        }
        Ok(match gate(&promotion.evidence) {
            Gate::Admitted => None,
            other => Some(format!("the evidence is no longer admitted: {other:?}")),
        })
    }

    fn policy(&self) -> Result<(PolicyKind, &'static str), String> {
        Ok(self
            .fleet
            .assignment(&self.bot)?
            .map_or((PolicyKind::Solving, style::STANDARD.name), |assignment| {
                (assignment.kind, assignment.style.name)
            }))
    }

    fn restore_incumbent(&mut self, promotion: &Promotion) -> Result<(), String> {
        self.fleet
            .set_champion(&self.bot, &promotion.evidence.incumbent)?;
        self.ledger.adopt(promotion.evidence.incumbent);
        Ok(())
    }
}

fn verdict_name(verdict: Verdict) -> &'static str {
    match verdict {
        Verdict::Running => "running",
        Verdict::Supported => "supported",
        Verdict::Harmful => "harmful",
        Verdict::SimGap => "sim-gap",
        Verdict::Inconclusive => "inconclusive",
        Verdict::Cleared => "cleared",
    }
}
