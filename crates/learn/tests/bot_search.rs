//! The per-bot search (V11-76): screen every challenger round the bot's champion, then spend a
//! longer confirmation on the promising ones.

use svanbot_learn::paired::Verdict;
use svanbot_learn::search::{BotNext, Entry, Stage, Step, next_bot};
use svanbot_strategy::params::Params;
use svanbot_strategy::profile::ACTIVE;
use svanbot_strategy::style::{self, Style};

const CHAMPION: Params = Params {
    exploit: 0.5,
    spread: 0.0,
    ..Params::DEFAULT
};

fn at(exploit: f64) -> Params {
    Params {
        exploit,
        spread: 0.0,
        ..Params::DEFAULT
    }
}

fn found(deals: usize, mbb: f64) -> Verdict {
    Verdict {
        deals,
        mbb_per_100: mbb,
        ci95: 100.0,
    }
}

fn ran(challenger: Params, stage: Stage, seed: u64, mbb: f64) -> Entry {
    Entry::Simmed {
        step: Step {
            champion: CHAMPION.digest(),
            challenger,
            stage,
            seed,
        },
        verdict: found(200, mbb),
    }
}

fn screen(next: BotNext) -> (Params, u64) {
    match next {
        BotNext::Screen { challenger, seed } => (challenger, seed),
        other => panic!("expected a screen, got {other:?}"),
    }
}

fn confirm(next: BotNext) -> (Params, u64) {
    match next {
        BotNext::Confirm { challenger, seed } => (challenger, seed),
        other => panic!("expected a confirmation, got {other:?}"),
    }
}

#[test]
fn every_challenger_is_screened_on_the_same_deals_before_a_confirmation() {
    let style = &style::STANDARD;
    let mut log = Vec::new();
    let mut seen = Vec::new();
    let mut seeds = Vec::new();
    while let BotNext::Screen { challenger, seed } = next_bot(&CHAMPION, style, &log) {
        seen.push(challenger);
        seeds.push(seed);
        log.push(ran(
            challenger,
            Stage::First,
            seed,
            100.0 - seen.len() as f64,
        ));
    }
    assert_eq!(seen.len(), 6, "one number, three rings, a step each way");
    assert!(seeds.iter().all(|seed| *seed == seeds[0]), "the same deals");
    // The promising better half continues: the three highest screening means.
    let (first, confirmation_seed) = confirm(next_bot(&CHAMPION, style, &log));
    assert!(seen[..3].contains(&first), "{seen:?}");
    assert_ne!(
        confirmation_seed, seeds[0],
        "a confirmation plays fresh deals"
    );
}

#[test]
fn a_challenger_proposed_only_after_a_fresh_confirmation_is_above_the_champion() {
    let style = &style::STANDARD;
    let mut log = Vec::new();
    for challenger in [
        at(0.4),
        at(0.625),
        at(0.45),
        at(0.5555555555555556),
        at(0.475),
        at(0.5263157894736842),
    ] {
        log.push(ran(challenger, Stage::First, 7, 200.0));
    }
    // The best half by screening mean is the first three in the list, and the confirmation asks
    // them one at a time.
    let (first, seed) = confirm(next_bot(&CHAMPION, style, &log));
    log.push(ran(first, Stage::Confirmation, seed, 300.0));
    let (second, seed) = confirm(next_bot(&CHAMPION, style, &log));
    log.push(ran(second, Stage::Confirmation, seed, -100.0));
    let (third, seed) = confirm(next_bot(&CHAMPION, style, &log));
    log.push(ran(third, Stage::Confirmation, seed, 50.0));

    match next_bot(&CHAMPION, style, &log) {
        BotNext::Candidate {
            challenger,
            screening,
            confirmation,
        } => {
            assert_eq!(challenger, first);
            assert_eq!(screening, found(200, 200.0));
            assert_eq!(confirmation, found(200, 300.0));
            assert!(confirmation.above_zero());
        }
        other => panic!("expected a candidate, got {other:?}"),
    }
}

#[test]
fn a_screen_at_or_below_the_champion_is_dropped_and_an_inconclusive_confirmation_holds() {
    let style = &style::STANDARD;
    let mut log = Vec::new();
    for challenger in [
        at(0.4),
        at(0.625),
        at(0.45),
        at(0.5555555555555556),
        at(0.475),
        at(0.5263157894736842),
    ] {
        log.push(ran(challenger, Stage::First, 7, -10.0));
    }
    assert_eq!(next_bot(&CHAMPION, style, &log), BotNext::Held);

    // One promising challenger gets its confirmation; a level one is no verdict, and the
    // incumbent holds.
    let mut log = Vec::new();
    for (index, challenger) in [
        at(0.4),
        at(0.625),
        at(0.45),
        at(0.5555555555555556),
        at(0.475),
        at(0.5263157894736842),
    ]
    .into_iter()
    .enumerate()
    {
        log.push(ran(
            challenger,
            Stage::First,
            7,
            if index == 0 { 200.0 } else { -10.0 },
        ));
    }
    let (challenger, seed) = confirm(next_bot(&CHAMPION, style, &log));
    assert_eq!(challenger, at(0.4));
    log.push(ran(challenger, Stage::Confirmation, seed, 50.0));
    assert_eq!(next_bot(&CHAMPION, style, &log), BotNext::Held);
}

#[test]
fn a_style_keeps_the_search_inside_its_bounds() {
    static BOUNDED: Style = Style {
        name: "bounded",
        open_scale: 0.8,
        profile: ACTIVE,
        bounds: &[("exploit", 0.0, 0.45)],
    };
    let mut log = Vec::new();
    let mut seen = Vec::new();
    while let BotNext::Screen { challenger, seed } = next_bot(&CHAMPION, &BOUNDED, &log) {
        seen.push(challenger);
        log.push(ran(challenger, Stage::First, seed, -10.0));
    }
    assert_eq!(seen, [at(0.4), at(0.45)]);
    // A pinned number leaves nothing to search.
    static PINNED: Style = Style {
        name: "pinned",
        open_scale: 0.8,
        profile: ACTIVE,
        bounds: &[("exploit", 0.5, 0.5)],
    };
    assert_eq!(next_bot(&CHAMPION, &PINNED, &[]), BotNext::Held);
}

#[test]
fn another_champion_s_screening_is_not_this_one_s() {
    let old = ran(at(0.4), Stage::First, 7, 300.0);
    let promoted = at(0.4);
    let next = next_bot(&promoted, &style::STANDARD, &[old]);
    let (challenger, _) = screen(next);
    assert_eq!(challenger, at(0.4 * 0.8));
}
