//! `svanbot learn` (V11-49, V11-76): one bounded step of the unattended search, then exit. A
//! timer runs it at low priority; nobody has to.
//!
//! A run reads the fleet's files and nothing else: the champion, the champion slots, the
//! assignments, a waiting proposal, the canary's running probe, and the search logs. It never
//! opens a store, so no bot's reads reach it (`docs/adr/0004-reads-are-per-bot.md`).
//!
//! With a champion slot, a run is one bot's step ([`bot_step`]): the slotted bot whose last
//! search is oldest rotates in, its assigned kind and style are simulated, and it screens or
//! confirms one challenger against its own champion. A candidate the confirmation puts above the
//! champion waits for that bot's probe (#1222); otherwise the incumbent holds.
//!
//! Without a slot, the fleet champion's search ([`step`]) runs as before: it **waits**, simming
//! nothing, while a proposal waits or a probe runs; **sims** the next step [`search::next`] asks
//! for, one paired sim of a bounded number of deals, and logs it; **proposes** a challenger both
//! its sims put above the champion, as a promotion with the canary's harm guard of
//! [`GUARD_HANDS`] hands, naming the champion it was measured against, and only if the canary has
//! nothing else ([`Fleet::propose_if_free`]); and says the search round this champion is
//! **spent**, until a promotion gives it a new one.
//!
//! Once the reference is frozen (Phase 2's cutover), the fleet-wide search is retired: a run
//! sims nothing, records that the reference is frozen, and a restart does not resume it
//! (`docs/adr/0006-the-reference-is-frozen.md`).
//!
//! Every run notes what it did in the search log, so its progress shows without anyone watching.

use svanbot_learn::paired::Verdict;
use svanbot_learn::probe::Kind;
use svanbot_learn::search::{self, BotNext, Entry, Next, Stage, Step, next_bot};
use svanbot_runtime::fleet::{BotSearch, Candidate, Fleet, Proposal};
use svanbot_strategy::kind::PolicyKind;
use svanbot_strategy::policy::Policy;
use svanbot_strategy::style;

/// The hands of a promotion's harm guard on the canary.
pub const GUARD_HANDS: u64 = 3_000;
/// The loss, in bb/100, a guard counts as harm: it is there to catch gross harm fast.
pub const GUARD_MARGIN: f64 = 20.0;
/// The orbits a probe may take: about 20,000 hands.
pub const PROBE_ORBITS: u64 = 3_334;

/// Runs one bounded step of the unattended search on `fleet` at `now_ms`, each sim `deals` deals
/// by `sim`, and returns what it did, as it noted it in the search log.
pub fn step(
    fleet: &Fleet,
    deals: u64,
    now_ms: i64,
    sim: &dyn Fn(&Step, u64) -> Verdict,
) -> Result<String, String> {
    if let Some(reference) = fleet.reference()? {
        let note = format!(
            "frozen: the reference is {} since {}; the fleet-wide search is stopped",
            reference.digest(),
            reference.cutover_ms
        );
        fleet.record_search(&[], &note, now_ms)?;
        return Ok(note);
    }
    let champion = fleet.champion()?;
    if let Some(probe) = fleet.probe()? {
        let note = format!(
            "waits: the canary is running a probe of {}",
            probe.challenger.digest()
        );
        fleet.record_search(&[], &note, now_ms)?;
        return Ok(note);
    }
    if let Some(waiting) = fleet.waiting()? {
        let note = format!(
            "waits: a proposal of {} waits for the canary",
            waiting.challenger.digest()
        );
        fleet.record_search(&[], &note, now_ms)?;
        return Ok(note);
    }
    let mut log = fleet.search_log()?.entries;
    let mut ran = Vec::new();
    let mut note = None;
    loop {
        match search::next(&champion, &log) {
            Next::Sim(asked) if ran.is_empty() => {
                let verdict = sim(&asked, deals);
                let entry = Entry::Simmed {
                    step: asked,
                    verdict,
                };
                note = Some(format!(
                    "simmed {} ({}) against champion {}: {:+.0} mbb/100, 95% ±{:.0}, over {} deals",
                    asked.challenger.digest(),
                    describe(&asked),
                    champion.digest(),
                    verdict.mbb_per_100,
                    verdict.ci95,
                    verdict.deals,
                ));
                log.push(entry);
                ran.push(entry);
            }
            // One sim a run: the next waits for the next run.
            Next::Sim(_) => break,
            Next::Propose {
                challenger,
                confirmation,
                ..
            } => {
                let proposal = Proposal {
                    challenger,
                    kind: Kind::Guard { hands: GUARD_HANDS },
                    harm_margin: GUARD_MARGIN,
                    // The confirmation's estimate: the first sim was the one that picked it, so
                    // it leans high; the confirmation played deals that had no say in the pick.
                    sim_estimate: Some(confirmation.mbb_per_100 / 1000.0),
                    max_orbits: PROBE_ORBITS,
                    against: Some(champion.digest()),
                };
                note = Some(if fleet.propose_if_free(&proposal)? {
                    ran.push(Entry::Proposed {
                        champion: champion.digest(),
                        challenger,
                    });
                    format!(
                        "proposed {} for the canary's harm guard of {GUARD_HANDS} hands: both sims put it above champion {}",
                        challenger.digest(),
                        champion.digest()
                    )
                } else {
                    format!(
                        "waits: {} is ready to propose, but the canary has a proposal or a probe",
                        challenger.digest()
                    )
                });
                break;
            }
            Next::Exhausted => {
                note.get_or_insert_with(|| {
                    format!(
                        "spent: every step round champion {} is simmed and none is above it",
                        champion.digest()
                    )
                });
                break;
            }
        }
    }
    let note = note.unwrap_or_default();
    fleet.record_search(&ran, &note, now_ms)?;
    Ok(note)
}

/// Runs one bounded step of the per-bot search at `now_ms`: the bot with a champion slot whose
/// last search is oldest (ties by name) gets one screening sim on `screen_deals`, or one
/// confirmation on `confirm_deals` on fresh deals, simulated with the kind and style it is
/// assigned. A candidate the confirmation put above its champion is kept for that bot's own
/// probe; otherwise the incumbent holds. Returns what it did, as the bot's search log noted it.
///
/// A bot with a candidate waiting is not searched again: that evidence is pending for its probe
/// (#1222). A bot with no assignment is searched as the reference policy.
pub fn bot_step(
    fleet: &Fleet,
    screen_deals: u64,
    confirm_deals: u64,
    now_ms: i64,
    sim: &dyn Fn(&Policy, &Policy, &Step, u64) -> Verdict,
) -> Result<String, String> {
    let slots = fleet.champion_slots()?;
    if slots.is_empty() {
        return Ok("no champion slot: nothing to search".into());
    }
    let mut waiting = Vec::new();
    let mut ready = Vec::new();
    for (bot, champion) in slots {
        let progress = fleet.bot_search(&bot, &champion)?;
        if progress.candidate.is_some() {
            waiting.push(bot);
        } else {
            ready.push((bot, champion, progress));
        }
    }
    let Some((bot, champion, progress)) = ready.into_iter().min_by(|left, right| {
        last_at(&left.2)
            .cmp(&last_at(&right.2))
            .then_with(|| left.0.cmp(&right.0))
    }) else {
        return Ok(format!(
            "waits: {} has a candidate for its own probe",
            waiting.join(", ")
        ));
    };
    let (kind, assigned_style) = match fleet.assignment(&bot)? {
        Some(assignment) => (assignment.kind, assignment.style),
        None => (PolicyKind::Solving, &style::STANDARD),
    };
    let policy = |params| Policy::new(kind, assigned_style, params);
    let champion_policy = policy(champion);
    let (entries, note, candidate) = match next_bot(&champion, assigned_style, &progress.entries) {
        BotNext::Screen { challenger, seed } => {
            let step = Step {
                champion: champion.digest(),
                challenger,
                stage: Stage::First,
                seed,
            };
            let verdict = sim(&policy(challenger), &champion_policy, &step, screen_deals);
            (
                vec![Entry::Simmed { step, verdict }],
                format!(
                    "{bot} screened {} ({}) against {}: {:+.0} mbb/100, 95% ±{:.0}, over {} deals",
                    challenger.digest(),
                    describe(&step),
                    champion.digest(),
                    verdict.mbb_per_100,
                    verdict.ci95,
                    verdict.deals
                ),
                None,
            )
        }
        BotNext::Confirm { challenger, seed } => {
            let step = Step {
                champion: champion.digest(),
                challenger,
                stage: Stage::Confirmation,
                seed,
            };
            let verdict = sim(&policy(challenger), &champion_policy, &step, confirm_deals);
            (
                vec![Entry::Simmed { step, verdict }],
                format!(
                    "{bot} confirmed {} ({}) against {}: {:+.0} mbb/100, 95% ±{:.0}, over {} deals",
                    challenger.digest(),
                    describe(&step),
                    champion.digest(),
                    verdict.mbb_per_100,
                    verdict.ci95,
                    verdict.deals
                ),
                None,
            )
        }
        BotNext::Candidate {
            challenger,
            confirmation,
            ..
        } => {
            let candidate = Candidate {
                challenger,
                kind,
                style: assigned_style.name,
                verdict: confirmation,
            };
            (
                Vec::new(),
                format!(
                    "{bot} has candidate {} against {}: confirmation {:+.0} mbb/100, 95% ±{:.0}, over {} deals",
                    challenger.digest(),
                    champion.digest(),
                    confirmation.mbb_per_100,
                    confirmation.ci95,
                    confirmation.deals
                ),
                Some(candidate),
            )
        }
        BotNext::Held => (
            Vec::new(),
            format!(
                "{bot} held: no challenger round {} beat it; the incumbent stays",
                champion.digest()
            ),
            None,
        ),
    };
    fleet.record_bot_search(&bot, &champion, &entries, &note, now_ms, candidate.as_ref())?;
    Ok(note)
}

/// When a bot's search last ran; a bot that has never run is oldest.
fn last_at(search: &BotSearch) -> i64 {
    search.last.as_ref().map_or(i64::MIN, |(at, _)| *at)
}

/// The step in words: what it changes, and which of the challenger's sims it is.
fn describe(step: &Step) -> String {
    let changed: Vec<String> = step
        .challenger
        .named()
        .iter()
        .map(|(name, value)| format!("{name}={value}"))
        .collect();
    let stage = match step.stage {
        Stage::First => "first sim",
        Stage::Confirmation => "confirmation",
    };
    format!("{}, {stage}", changed.join(", "))
}
