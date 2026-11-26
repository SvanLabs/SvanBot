//! The exploiter's side of one round-robin pass (V11-100): one bounded search step against a
//! bot's champion, and the bout in which a bot meets the exploiter found for it.

use svanbot_learn::arena::{BOUT, BoutReader};
use svanbot_learn::exploiter;
use svanbot_learn::paired::{Match, Verdict};
use svanbot_learn::play;
use svanbot_learn::reads::Reads;
use svanbot_learn::search::{BotNext, Entry, Stage, Step};
use svanbot_runtime::fleet::League;
use svanbot_runtime::round_robin::ExploiterRecord;
use svanbot_strategy::kind::PolicyKind;
use svanbot_strategy::policy::Policy;

use super::{EXPLOITER_CONFIRM_DEALS, EXPLOITER_SCREEN_DEALS, Gauntlet, Search};

/// One bounded step of the exploiter search against `gauntlet`'s champion: at most one screening
/// or confirmation sim, then whatever the search concludes without another sim. A candidate a
/// confirmation put above the champion is recorded as the brain's exploitability, which ends the
/// search for this champion.
pub(super) fn step_exploiter(gauntlet: &mut Gauntlet, search: &mut Search<'_>) {
    let mut next = exploiter::next(&gauntlet.champion, &gauntlet.exploiter_log);
    match next {
        BotNext::Screen { challenger, seed } => {
            let step = Step {
                champion: gauntlet.champion.digest(),
                challenger,
                stage: Stage::First,
                seed,
            };
            let verdict = search(gauntlet, &step, EXPLOITER_SCREEN_DEALS);
            gauntlet.exploiter_log.push(Entry::Simmed { step, verdict });
            next = exploiter::next(&gauntlet.champion, &gauntlet.exploiter_log);
        }
        BotNext::Confirm { challenger, seed } => {
            let step = Step {
                champion: gauntlet.champion.digest(),
                challenger,
                stage: Stage::Confirmation,
                seed,
            };
            let verdict = search(gauntlet, &step, EXPLOITER_CONFIRM_DEALS);
            gauntlet.exploiter_log.push(Entry::Simmed { step, verdict });
            next = exploiter::next(&gauntlet.champion, &gauntlet.exploiter_log);
        }
        _ => {}
    }
    if let BotNext::Candidate {
        challenger,
        confirmation,
        ..
    } = next
    {
        gauntlet.exploiter = Some(ExploiterRecord {
            params: challenger,
            champion: gauntlet.champion.digest(),
            win_rate: confirmation.mbb_per_100,
            radius: confirmation.ci95,
            deals: confirmation.deals as u64,
        });
    }
}

/// Plays bout `number` of `league` against `rival`, in the bot's seat and the rival's own policy.
pub(super) fn play_rival(
    game: &Match,
    league: League,
    policy: &Policy,
    rival: Policy,
    number: u64,
    fixed: Option<u64>,
) -> Vec<f64> {
    let seats = league.seats();
    let challenger = BoutReader::new(play::reading(*policy, fixed, seats, Reads::deciding, None))
        .with_profile_stats(policy.uses_stats());
    let rival = play::player(rival, fixed);
    let deals = number * BOUT..(number + 1) * BOUT;
    deals
        .map(|deal| match league {
            League::HeadsUp => play::rival_duel(game, deal, &challenger, &rival),
            _ => play::rival_ring_at(game, seats, deal, &challenger, &rival),
        })
        .collect()
}

/// How the pass sims one exploiter step: the challenger is the `Exploiting` kind in the hunter's
/// style, and the champion is the bot's own policy, heads up on the step's seed.
pub(super) fn exploiter_sim(
    gauntlet: &Gauntlet,
    step: &Step,
    deals: u64,
    game: &Match,
    fixed: Option<u64>,
) -> Verdict {
    let challenger = play::player(
        Policy::new(PolicyKind::Exploiting, exploiter::style(), step.challenger),
        fixed,
    );
    let champion = play::player(gauntlet.policy, fixed);
    let game = Match {
        seed: step.seed,
        ..*game
    };
    crate::paired::sim(&game, deals, &challenger, &champion)
}
