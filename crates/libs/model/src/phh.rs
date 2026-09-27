//! PHH (Poker Hand History, <https://phh.readthedocs.io>) files replayed into `HandSummary`s.
//!
//! Only no-limit hold'em (`variant = 'NT'`) is read. Seats are the PHH player order (p1 = seat 0);
//! for three or more players p1 posts the small blind and the last seat is the button, heads-up
//! the button posts the small blind. Actions: `d dh` deals hole cards (private, not a showdown
//! reveal), `d db` deals board cards and opens a street, `f` folds, `cc` checks or calls, `cbr X`
//! bets or raises to a street total of X, `sm` shows (with cards) or mucks. A hand is rejected
//! unless every folded seat's finishing stack equals its starting stack less its contributions.

use crate::model::HandSummary;
use sv10_cards::cards::Card;
use sv10_engine::engine::{ActionKind, ActionRecord, Street};

#[derive(Clone, Debug, PartialEq)]
enum Value {
    Str(String),
    Int(i64),
    Bool(bool),
    Strs(Vec<String>),
    /// Numbers stay fractional: split pots leave half chips in `finishing_stacks`.
    Nums(Vec<f64>),
}

/// The subset of TOML the PHH spec uses for hold'em: one `key = value` per line, values are
/// quoted strings, integers, booleans or flat arrays of either. Comments and unknown keys are
/// ignored; anything unparseable yields `None`.
fn parse_value(v: &str) -> Option<Value> {
    let v = v.trim();
    let unquote = |s: &str| {
        let s = s.trim();
        let q = s.chars().next()?;
        (q == '\'' || q == '"').then_some(())?;
        s.strip_prefix(q)?.strip_suffix(q).map(str::to_string)
    };
    if let Some(inner) = v.strip_prefix('[').and_then(|s| s.strip_suffix(']')) {
        let items: Vec<&str> = inner.split(',').map(str::trim).filter(|s| !s.is_empty()).collect();
        if items.is_empty() {
            return Some(Value::Nums(Vec::new()));
        }
        if let Some(s) = items.iter().map(|s| unquote(s)).collect::<Option<Vec<_>>>() {
            return Some(Value::Strs(s));
        }
        return items.iter().map(|s| s.parse().ok()).collect::<Option<Vec<f64>>>().map(Value::Nums);
    }
    match v {
        "true" => Some(Value::Bool(true)),
        "false" => Some(Value::Bool(false)),
        _ => unquote(v).map(Value::Str).or_else(|| v.parse().ok().map(Value::Int)),
    }
}

fn fields(text: &str) -> Option<Vec<(String, Value)>> {
    let mut out = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') || line.starts_with('[') {
            continue;
        }
        let (k, v) = line.split_once('=')?;
        // Cut a trailing comment: the first `#` outside quotes.
        let mut quote = None;
        let end = v
            .char_indices()
            .find(|&(_, c)| match (quote, c) {
                (None, '\'' | '"') => {
                    quote = Some(c);
                    false
                }
                (Some(q), c) if c == q => {
                    quote = None;
                    false
                }
                (None, '#') => true,
                _ => false,
            })
            .map_or(v.len(), |(i, _)| i);
        out.push((k.trim().to_string(), parse_value(&v[..end])?));
    }
    Some(out)
}

/// One parsed hand plus the metadata a caller needs to tag or validate it.
#[derive(Clone, Debug)]
pub struct PhhHand {
    /// The replayed hand (seats in PHH player order).
    pub summary: HandSummary,
    /// The file's `hand` number, when present.
    pub hand: Option<i64>,
    /// Recorded finishing stacks (half chips possible after split pots); empty if absent.
    pub finishing_stacks: Vec<f64>,
}

fn cards(s: &str) -> Option<Vec<Card>> {
    if !s.len().is_multiple_of(2) {
        return None;
    }
    (0..s.len()).step_by(2).map(|i| s.get(i..i + 2).and_then(Card::parse)).collect()
}

/// Parse one PHH hand. `None` for other variants, malformed files and hands whose chip flows do
/// not reproduce the recorded finishing stacks.
pub fn parse(text: &str) -> Option<PhhHand> {
    let f = fields(text)?;
    let get = |k: &str| f.iter().find(|(n, _)| n == k).map(|(_, v)| v);
    if get("variant")? != &Value::Str("NT".into()) {
        return None;
    }
    let ints = |v: &Value| match v {
        Value::Nums(x) => x.iter().map(|&f| (f.fract() == 0.0).then_some(f as i64)).collect::<Option<Vec<i64>>>(),
        _ => None,
    };
    let starting = ints(get("starting_stacks")?)?;
    let blinds = ints(get("blinds_or_straddles")?)?;
    let Value::Strs(actions) = get("actions")? else { return None };
    let n = starting.len();
    if !(2..=10).contains(&n) || blinds.len() != n {
        return None;
    }
    let antes = match get("antes") {
        Some(v @ Value::Nums(a)) if a.len() == n => ints(v)?,
        Some(Value::Int(a)) => vec![*a; n],
        _ => vec![0; n],
    };
    if antes.iter().any(|&a| a != 0) || blinds.iter().skip(2).any(|&b| b != 0) {
        // Antes and straddles change who acts first and the dead money; not needed for 6-max NLHE.
        return None;
    }
    let players: Vec<(usize, String)> = match get("players") {
        Some(Value::Strs(p)) if p.len() == n => p.iter().cloned().enumerate().collect(),
        _ => (0..n).map(|i| (i, format!("p{}", i + 1))).collect(),
    };
    let hand = match get("hand") {
        Some(Value::Int(h)) => Some(*h),
        _ => None,
    };
    let finishing = match get("finishing_stacks") {
        Some(Value::Nums(s)) if s.len() == n => s.clone(),
        _ => Vec::new(),
    };
    let bb = blinds[1].max(blinds[0]).max(1);
    let button = if n == 2 { 0 } else { n - 1 };

    let mut behind: Vec<i64> = starting.clone();
    let mut bet = vec![0i64; n];
    let mut contributed = vec![0i64; n];
    let mut folded = vec![false; n];
    let mut pot = 0;
    // Heads-up the button (p1) posts the small blind, so blinds map straight onto seats either way.
    for s in 0..2 {
        let a = blinds[s].min(behind[s]);
        behind[s] -= a;
        bet[s] = a;
        contributed[s] = a;
        pot += a;
    }
    let mut level = bet.iter().copied().max().unwrap_or(0);
    let mut last_raise = bb;
    let mut street = Street::Preflop;
    let mut board: Vec<Card> = Vec::new();
    let mut shown: Vec<(usize, [Card; 2])> = Vec::new();
    let mut history = Vec::new();
    for a in actions {
        let parts: Vec<&str> = a.split_whitespace().collect();
        match parts.as_slice() {
            ["d", "dh", ..] => {}
            ["d", "db", c] => {
                board.extend(cards(c)?);
                street = match board.len() {
                    3 => Street::Flop,
                    4 => Street::Turn,
                    5 => Street::River,
                    _ => return None,
                };
                bet.iter_mut().for_each(|b| *b = 0);
                level = 0;
                last_raise = bb;
            }
            [p, verb, rest @ ..] if p.starts_with('p') => {
                let seat = p[1..].parse::<usize>().ok()?.checked_sub(1).filter(|&s| s < n)?;
                let before = bet[seat];
                let to_call = (level - before).max(0);
                let (kind, to) = match (*verb, rest) {
                    ("sm", [c]) => {
                        let c = cards(c)?;
                        if c.len() != 2 {
                            return None;
                        }
                        shown.push((seat, [c[0], c[1]]));
                        continue;
                    }
                    ("sm", []) => continue,
                    ("f", []) => {
                        folded[seat] = true;
                        (ActionKind::Fold, 0)
                    }
                    ("cc", []) if to_call == 0 => (ActionKind::Check, 0),
                    ("cc", []) if to_call >= behind[seat] => (ActionKind::AllIn, before + behind[seat]),
                    ("cc", []) => (ActionKind::Call, level),
                    ("cbr", [x]) => {
                        let x: i64 = x.parse().ok()?;
                        if x <= level || x > before + behind[seat] {
                            return None;
                        }
                        (if x == before + behind[seat] { ActionKind::AllIn } else { ActionKind::Raise }, x)
                    }
                    _ => return None,
                };
                let full_raise = to > level && to - level >= last_raise;
                history.push(ActionRecord {
                    seat,
                    street,
                    kind,
                    to,
                    pot_before: pot,
                    to_call_before: to_call,
                    bet_before: before,
                    full_raise,
                    think_ms: None,
                    street_open: false,
                });
                if to > before {
                    let add = to - before;
                    behind[seat] -= add;
                    contributed[seat] += add;
                    pot += add;
                    bet[seat] = to;
                }
                if to > level {
                    if full_raise {
                        last_raise = to - level;
                    }
                    level = to;
                }
            }
            _ => return None,
        }
    }
    if history.is_empty() {
        return None;
    }
    if !finishing.is_empty() {
        let folders_match = (0..n).filter(|&s| folded[s]).all(|s| finishing[s] == (starting[s] - contributed[s]) as f64);
        if !folders_match || (finishing.iter().sum::<f64>() - starting.iter().sum::<i64>() as f64).abs() > 1e-6 {
            return None;
        }
    }
    shown.sort_by_key(|s| s.0);
    let stacks = starting.iter().copied().enumerate().collect();
    Some(PhhHand { summary: HandSummary { players, button, bb, history, board, shown, stacks }, hand, finishing_stacks: finishing })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Pluribus hand 89 (phh-dataset `data/pluribus/*/89.phh`, CC-BY-4.0 / MIT).
    const SHOWDOWN: &str = "variant = 'NT'
ante_trimming_status = true
antes = [0, 0, 0, 0, 0, 0]
blinds_or_straddles = [50, 100, 0, 0, 0, 0]
min_bet = 100
starting_stacks = [10000, 10000, 10000, 10000, 10000, 10000]
actions = ['d dh p1 Tc7h', 'd dh p2 8s7d', 'd dh p3 QcKc', 'd dh p4 5c3h', 'd dh p5 8cAs', 'd dh p6 AdQh', 'p3 cbr 200', 'p4 f', 'p5 f', 'p6 cc', 'p1 f', 'p2 cc', 'd db 2d3dQd', 'p2 cc', 'p3 cc', 'p6 cbr 250', 'p2 f', 'p3 cc', 'd db Kh', 'p3 cc', 'p6 cbr 1850', 'p3 cc', 'd db Jh', 'p3 cc', 'p6 cc', 'p3 sm QcKc', 'p6 sm']
hand = 89
players = ['Pluribus', 'MrPink', 'Eddie', 'MrOrange', 'Bill', 'MrBlue']
finishing_stacks = [9950, 9800, 12550, 10000, 10000, 7700]
";

    #[test]
    fn replays_blinds_streets_bets_and_showdown() {
        let h = parse(SHOWDOWN).unwrap();
        let s = &h.summary;
        assert_eq!((s.button, s.bb, h.hand), (5, 100, Some(89)));
        assert_eq!(s.players[0], (0, "Pluribus".to_string()));
        assert_eq!(s.board.len(), 5);
        // Hole cards dealt privately are not reveals; only `sm` with cards is.
        assert_eq!(s.shown.len(), 1);
        assert_eq!(s.shown[0].0, 2);
        let r = &s.history;
        assert_eq!(r.len(), 16);
        // UTG opens to 200 into the blinds.
        assert_eq!(
            (r[0].seat, r[0].kind, r[0].to, r[0].pot_before, r[0].to_call_before, r[0].full_raise),
            (2, ActionKind::Raise, 200, 150, 100, true)
        );
        // SB folds, BB calls 100 more.
        assert_eq!((r[4].seat, r[4].kind), (0, ActionKind::Fold));
        assert_eq!((r[5].kind, r[5].to, r[5].bet_before, r[5].to_call_before), (ActionKind::Call, 200, 100, 100));
        // Flop: checks, bet 250 into 650, then turn bet 1850 into 1150.
        assert_eq!((r[6].street, r[6].kind, r[6].pot_before), (Street::Flop, ActionKind::Check, 650));
        assert_eq!((r[8].kind, r[8].to, r[8].pot_before), (ActionKind::Raise, 250, 650));
        let turn_bet = r.iter().find(|a| a.street == Street::Turn && a.kind == ActionKind::Raise).unwrap();
        assert_eq!((turn_bet.to, turn_bet.pot_before), (1850, 1150));
        assert_eq!(r.last().unwrap().street, Street::River);
        assert_eq!(s.stacks, (0..6).map(|i| (i, 10_000)).collect::<Vec<_>>());
    }

    #[test]
    fn rejects_hands_whose_folders_lose_the_wrong_amount() {
        let bad = SHOWDOWN.replace("[9950, 9800,", "[9950, 9900,");
        assert!(parse(&bad).is_none());
        let other_variant = SHOWDOWN.replace("'NT'", "'FT'");
        assert!(parse(&other_variant).is_none());
        let illegal = SHOWDOWN.replace("'p3 cbr 200'", "'p3 cbr 50'");
        assert!(parse(&illegal).is_none());
        // Split pots leave half chips with the winners.
        let split = SHOWDOWN.replace("12550, 10000, 10000, 7700]", "12549.5, 10000, 10000, 7700.5]");
        assert_eq!(parse(&split).unwrap().finishing_stacks[2], 12549.5);
    }

    #[test]
    fn heads_up_button_posts_small_blind_and_calls_all_in() {
        let text = "variant = 'NT'
antes = [0, 0]
blinds_or_straddles = [1, 2]
min_bet = 2
starting_stacks = [100, 40]
actions = ['d dh p1 AsAh', 'd dh p2 KsKh', 'p1 cbr 100', 'p2 cc', 'd db 2c3c4d', 'd db 9h', 'd db Td', 'p1 sm AsAh', 'p2 sm KsKh']
players = ['a', 'b']
finishing_stacks = [140, 0]
";
        let h = parse(text).unwrap();
        assert_eq!(h.summary.button, 0);
        let r = &h.summary.history;
        assert_eq!((r[0].kind, r[0].to), (ActionKind::AllIn, 100));
        assert_eq!((r[1].kind, r[1].to, r[1].to_call_before), (ActionKind::AllIn, 40, 98));
        assert_eq!(h.summary.shown.len(), 2);
    }

    /// Replays a PHH directory tree (PHH_DIR) and reports parsed/rejected counts.
    #[test]
    #[ignore]
    fn replays_real_files() {
        let Ok(dir) = std::env::var("PHH_DIR") else { return };
        let mut stack = vec![std::path::PathBuf::from(dir)];
        let (mut ok, mut bad) = (0, 0);
        while let Some(p) = stack.pop() {
            for e in std::fs::read_dir(p).unwrap().flatten() {
                let path = e.path();
                if path.is_dir() {
                    stack.push(path);
                } else if path.extension().is_some_and(|x| x == "phh") {
                    match parse(&std::fs::read_to_string(&path).unwrap()) {
                        Some(_) => ok += 1,
                        None => {
                            bad += 1;
                            if bad <= 5 {
                                eprintln!("rejected {}", path.display());
                            }
                        }
                    }
                }
            }
        }
        eprintln!("parsed {ok}, rejected {bad}");
        assert!(ok > 0);
    }
}
