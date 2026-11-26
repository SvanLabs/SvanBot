//! Per-bot promotion proposals and their live guard (`promotions.json`, V11-77).
//!
//! A proposal names one bot, the incumbent it was measured against, its kind and style, and the
//! evidence the promotion gate needs: a fresh paired 6-max confirmation and a designated-opponent
//! gauntlet. It waits for that bot alone; competing proposals cannot replace it, and another bot's
//! proposal is its own. A proposal whose slot or assignment changed is stale and is refused.
//!
//! The live guard runs on the bot's own standing reference arm (`Probing::standing_guard`): the
//! challenger is put in the bot's slot, and the bot's own orbits against the frozen reference
//! decide whether it stays. The evidence and the verdict are kept here so a restart can tell a
//! completed guard from one that was cut off.

use serde_json::{Map, Value, json};
use svanbot_learn::paired::Verdict;
use svanbot_learn::promotion::{Evidence, Gate, GauntletRow, gate};
use svanbot_strategy::kind::PolicyKind;
use svanbot_strategy::params::Params;
use svanbot_strategy::style;

use super::{Candidate, Fleet, params_from, params_json};

const PROMOTIONS: &str = "promotions.json";

/// Where a bot's promotion stands.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    /// Evidence passed; waiting for the named bot between hands.
    Pending,
    /// The challenger is in the bot's slot and the standing guard is counting its orbits.
    Guarding,
    /// The guard passed: the challenger stays in the slot.
    Cleared,
    /// The guard found harm: the incumbent is back in the slot.
    Harmful,
    /// The guard was cut off before it could decide: the incumbent is back in the slot.
    Incomplete,
    /// The incumbent or the assignment changed before the guard started: nothing was played.
    Superseded,
}

impl Status {
    pub const fn name(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Guarding => "guarding",
            Self::Cleared => "cleared",
            Self::Harmful => "harmful",
            Self::Incomplete => "incomplete",
            Self::Superseded => "superseded",
        }
    }

    pub fn named(name: &str) -> Option<Self> {
        [
            Self::Pending,
            Self::Guarding,
            Self::Cleared,
            Self::Harmful,
            Self::Incomplete,
            Self::Superseded,
        ]
        .into_iter()
        .find(|status| status.name() == name)
    }

    /// A proposal or guard that still owns the bot's promotion slot.
    pub const fn active(self) -> bool {
        matches!(self, Self::Pending | Self::Guarding)
    }
}

/// One bot's promotion: the evidence it rests on, and how far its guard has got.
#[derive(Clone, Debug, PartialEq)]
pub struct Promotion {
    /// The bot whose slot this may change, by its exact name.
    pub bot: String,
    pub evidence: Evidence,
    /// The standing-arm hands the guard must count before it clears.
    pub guard_hands: u64,
    /// The loss against the frozen reference, in bb/100, past which the guard rolls back.
    pub harm_margin: f64,
    pub status: Status,
    /// When the guard started, in ms since the epoch, and when it was decided.
    pub started_ms: Option<i64>,
    pub decided_ms: Option<i64>,
    /// What the last run said: the gate's reason, or the guard's edge.
    pub note: Option<String>,
}

impl Promotion {
    /// A proposal waiting for its bot, with no guard run yet.
    pub fn new(bot: &str, evidence: Evidence, guard_hands: u64, harm_margin: f64) -> Self {
        Self {
            bot: bot.into(),
            evidence,
            guard_hands,
            harm_margin,
            status: Status::Pending,
            started_ms: None,
            decided_ms: None,
            note: None,
        }
    }

    /// A proposal built from a bot's search candidate and the gauntlet rows run for it. The
    /// candidate is the fresh 6-max confirmation V11-76 keeps; the incumbent is the slot it was
    /// measured against.
    pub fn from_candidate(
        bot: &str,
        incumbent: Params,
        candidate: &Candidate,
        gauntlet: Vec<GauntletRow>,
        guard_hands: u64,
        harm_margin: f64,
    ) -> Self {
        Self::new(
            bot,
            Evidence {
                incumbent,
                challenger: candidate.challenger,
                kind: candidate.kind,
                style: candidate.style,
                fresh: candidate.verdict,
                gauntlet,
            },
            guard_hands,
            harm_margin,
        )
    }
}

impl Fleet {
    /// `bot`'s current promotion, or `None` while it has none.
    pub fn bot_promotion(&self, bot: &str) -> Result<Option<Promotion>, String> {
        match self.promotions()?.get(bot) {
            Some(value) => promotion_from(value, bot)
                .map(Some)
                .map_err(|error| format!("{PROMOTIONS}: {bot}: {error}")),
            None => Ok(None),
        }
    }

    /// Puts `promotion` forward for its bot, if the evidence is admitted and the bot still has
    /// the incumbent and assignment the evidence names. Returns whether it was put forward: a
    /// pending or guarding promotion is never replaced.
    pub fn propose_bot(&self, promotion: &Promotion) -> Result<bool, String> {
        self.refuse_unfrozen_promotion()?;
        if promotion.guard_hands == 0 {
            return Err(format!(
                "{PROMOTIONS}: {}'s guard has no hands",
                promotion.bot
            ));
        }
        if !(promotion.harm_margin.is_finite() && promotion.harm_margin > 0.0) {
            return Err(format!(
                "{PROMOTIONS}: {}'s harm margin is not positive",
                promotion.bot
            ));
        }
        let bot = &promotion.bot;
        match gate(&promotion.evidence) {
            Gate::Admitted => {}
            Gate::Refused(why) => {
                return Err(format!(
                    "{PROMOTIONS}: {bot}'s evidence is refused: {}",
                    why.join("; ")
                ));
            }
            Gate::Incomplete(why) => {
                return Err(format!(
                    "{PROMOTIONS}: {bot}'s evidence is incomplete: {}",
                    why.join("; ")
                ));
            }
        }
        let slot = self
            .champion_slot(bot)?
            .ok_or_else(|| format!("{PROMOTIONS}: {bot} has no champion slot"))?;
        if slot != promotion.evidence.incumbent {
            return Err(format!(
                "{PROMOTIONS}: {bot}'s slot is not the evidence's incumbent"
            ));
        }
        let assignment = self
            .assignment(bot)?
            .map_or((PolicyKind::Solving, style::STANDARD.name), |assignment| {
                (assignment.kind, assignment.style.name)
            });
        if assignment != (promotion.evidence.kind, promotion.evidence.style) {
            return Err(format!(
                "{PROMOTIONS}: {bot}'s assignment is not the evidence's policy"
            ));
        }
        let _lock = self.lock_fleet_file(PROMOTIONS)?;
        let mut bots = self.promotions()?;
        if bots
            .get(bot)
            .and_then(|value| status_from(value).ok())
            .is_some_and(Status::active)
        {
            return Ok(false);
        }
        bots.insert(bot.clone(), promotion_json(promotion));
        self.write(PROMOTIONS, &json!({ "bots": bots }).to_string())?;
        Ok(true)
    }

    /// Moves `bot`'s pending proposal into its live guard. False when it was not pending.
    pub fn start_bot_guard(&self, bot: &str, started_ms: i64) -> Result<bool, String> {
        let _lock = self.lock_fleet_file(PROMOTIONS)?;
        let mut bots = self.promotions()?;
        let Some(value) = bots.get_mut(bot) else {
            return Ok(false);
        };
        let mut promotion =
            promotion_from(value, bot).map_err(|error| format!("{PROMOTIONS}: {bot}: {error}"))?;
        if promotion.status != Status::Pending {
            return Ok(false);
        }
        promotion.status = Status::Guarding;
        promotion.started_ms = Some(started_ms);
        bots.insert(bot.into(), promotion_json(&promotion));
        self.write(PROMOTIONS, &json!({ "bots": bots }).to_string())?;
        Ok(true)
    }

    /// Ends `bot`'s live guard with a terminal status and note. A guard that is not running is
    /// left as it is and reported false.
    pub fn finish_bot_guard(
        &self,
        bot: &str,
        status: Status,
        decided_ms: i64,
        note: &str,
    ) -> Result<bool, String> {
        if status.active() {
            return Err(format!("{PROMOTIONS}: {status:?} is not a finished guard"));
        }
        self.end_active(bot, status, decided_ms, note, true)
    }

    /// Drops an active promotion as stale, whether it was waiting or guarding, without touching
    /// the bot's current slot. The record and the evidence stay for the operator.
    pub fn supersede_bot(&self, bot: &str, decided_ms: i64, note: &str) -> Result<bool, String> {
        self.end_active(bot, Status::Superseded, decided_ms, note, false)
    }

    fn end_active(
        &self,
        bot: &str,
        status: Status,
        decided_ms: i64,
        note: &str,
        guarding_only: bool,
    ) -> Result<bool, String> {
        let _lock = self.lock_fleet_file(PROMOTIONS)?;
        let mut bots = self.promotions()?;
        let Some(value) = bots.get_mut(bot) else {
            return Ok(false);
        };
        let mut promotion =
            promotion_from(value, bot).map_err(|error| format!("{PROMOTIONS}: {bot}: {error}"))?;
        if !promotion.status.active() || (guarding_only && promotion.status != Status::Guarding) {
            return Ok(false);
        }
        promotion.status = status;
        promotion.decided_ms = Some(decided_ms);
        promotion.note = Some(note.into());
        bots.insert(bot.into(), promotion_json(&promotion));
        self.write(PROMOTIONS, &json!({ "bots": bots }).to_string())?;
        Ok(true)
    }

    fn promotions(&self) -> Result<Map<String, Value>, String> {
        self.bots_in(PROMOTIONS)
    }

    /// The frozen reference stops fleet-wide promotions, not a bot's own slot; a per-bot
    /// promotion is allowed, but only when the reference is frozen, so its live control is the
    /// frozen reference every bot shares. Before the freeze the fleet search owns the fleet.
    fn refuse_unfrozen_promotion(&self) -> Result<(), String> {
        if self.reference()?.is_none() {
            return Err(format!(
                "{PROMOTIONS}: the reference is not frozen; a per-bot promotion waits for the freeze"
            ));
        }
        Ok(())
    }
}

fn status_from(value: &Value) -> Result<Status, String> {
    value["status"]
        .as_str()
        .and_then(Status::named)
        .ok_or_else(|| "the promotion's status is unknown".to_owned())
}

fn promotion_from(value: &Value, bot: &str) -> Result<Promotion, String> {
    Ok(Promotion {
        bot: bot.into(),
        evidence: evidence_from(&value["evidence"])?,
        guard_hands: value["guard_hands"]
            .as_u64()
            .ok_or("the promotion lacks guard_hands")?,
        harm_margin: number(value, "harm_margin")?,
        status: status_from(value)?,
        started_ms: value["started_ms"].as_i64(),
        decided_ms: value["decided_ms"].as_i64(),
        note: value["note"].as_str().map(str::to_owned),
    })
}

fn evidence_from(value: &Value) -> Result<Evidence, String> {
    let kind = value["kind"]
        .as_str()
        .and_then(PolicyKind::named)
        .ok_or("the evidence's kind is unknown")?;
    let style = value["style"]
        .as_str()
        .and_then(style::named)
        .ok_or("the evidence's style is unknown")?;
    Ok(Evidence {
        incumbent: params_from(&value["incumbent"])
            .map_err(|error| format!("the evidence's incumbent: {error}"))?,
        challenger: params_from(&value["challenger"])
            .map_err(|error| format!("the evidence's challenger: {error}"))?,
        kind,
        style: style.name,
        fresh: verdict_from(&value["fresh"])?,
        gauntlet: value["gauntlet"]
            .as_array()
            .ok_or("the evidence lacks a gauntlet")?
            .iter()
            .map(|row| {
                Ok(GauntletRow {
                    opponent: row["opponent"]
                        .as_str()
                        .ok_or("a gauntlet row lacks its opponent")?
                        .to_owned(),
                    verdict: verdict_from(row)?,
                })
            })
            .collect::<Result<_, String>>()?,
    })
}

fn promotion_json(promotion: &Promotion) -> Value {
    json!({
        "evidence": evidence_json(&promotion.evidence),
        "guard_hands": promotion.guard_hands,
        "harm_margin": promotion.harm_margin.to_string(),
        "status": promotion.status.name(),
        "started_ms": promotion.started_ms,
        "decided_ms": promotion.decided_ms,
        "note": promotion.note,
    })
}

fn evidence_json(evidence: &Evidence) -> Value {
    json!({
        "incumbent": params_json(&evidence.incumbent),
        "challenger": params_json(&evidence.challenger),
        "kind": evidence.kind.name(),
        "style": evidence.style,
        "fresh": verdict_json(&evidence.fresh),
        "gauntlet": evidence.gauntlet.iter().map(|row| {
            let mut value = verdict_json(&row.verdict);
            value["opponent"] = json!(row.opponent);
            value
        }).collect::<Vec<_>>(),
    })
}

fn verdict_json(verdict: &Verdict) -> Value {
    json!({
        "deals": verdict.deals,
        "mbb_per_100": verdict.mbb_per_100.to_string(),
        "ci95": verdict.ci95.to_string(),
    })
}

fn verdict_from(value: &Value) -> Result<Verdict, String> {
    Ok(Verdict {
        deals: value["deals"].as_u64().ok_or("a verdict lacks deals")? as usize,
        mbb_per_100: number(value, "mbb_per_100")?,
        ci95: number(value, "ci95")?,
    })
}

fn number(value: &Value, key: &str) -> Result<f64, String> {
    value[key]
        .as_str()
        .and_then(|text| text.parse().ok())
        .ok_or_else(|| format!("a promotion lacks {key}"))
}
