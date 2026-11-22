//! The parity spots: synthetic 6-max self-play decisions the v10.0.2 binary recorded, one JSON
//! line each, `{seed, situation, action}`. Seats are named `p0`..`p5`; no venue data is in them.
//! The reader is the least JSON the file needs, and keeps numbers as text, since seeds pass 2^63.

#![allow(dead_code)]

use std::collections::BTreeMap;

use svanbot_sim::v10::cards::Card;
use svanbot_sim::v10::situation::{ActionKind, ActionRecord, PlayerInfo, Situation, Street};

const SPOTS: &str = include_str!("../data/v10-6max.jsonl");

#[derive(Debug, Clone)]
pub enum Json {
    Null,
    Bool(bool),
    Num(String),
    Str(String),
    Arr(Vec<Json>),
    Obj(BTreeMap<String, Json>),
}

impl Json {
    pub fn parse(text: &str) -> Json {
        let mut p = Parser {
            s: text.as_bytes(),
            i: 0,
        };
        let v = p.value();
        p.ws();
        assert_eq!(p.i, p.s.len(), "trailing text");
        v
    }

    pub fn get(&self, key: &str) -> &Json {
        match self {
            Json::Obj(m) => m.get(key).unwrap_or(&Json::Null),
            _ => panic!("not an object: {key}"),
        }
    }

    pub fn arr(&self) -> &[Json] {
        match self {
            Json::Arr(v) => v,
            other => panic!("not an array: {other:?}"),
        }
    }

    pub fn str(&self) -> &str {
        match self {
            Json::Str(s) => s,
            other => panic!("not a string: {other:?}"),
        }
    }

    pub fn u64(&self) -> u64 {
        match self {
            Json::Num(n) => n.parse().expect("u64"),
            other => panic!("not a number: {other:?}"),
        }
    }

    pub fn i64(&self) -> i64 {
        match self {
            Json::Num(n) => n.parse().expect("i64"),
            other => panic!("not a number: {other:?}"),
        }
    }

    pub fn opt_i64(&self) -> Option<i64> {
        match self {
            Json::Null => None,
            v => Some(v.i64()),
        }
    }

    pub fn bool(&self) -> bool {
        match self {
            Json::Bool(b) => *b,
            Json::Null => false,
            other => panic!("not a bool: {other:?}"),
        }
    }
}

struct Parser<'a> {
    s: &'a [u8],
    i: usize,
}

impl Parser<'_> {
    fn ws(&mut self) {
        while self.i < self.s.len() && self.s[self.i].is_ascii_whitespace() {
            self.i += 1;
        }
    }

    fn eat(&mut self, b: u8) {
        self.ws();
        assert_eq!(self.s[self.i], b, "at {}", self.i);
        self.i += 1;
    }

    fn word(&mut self, w: &str, v: Json) -> Json {
        assert!(self.s[self.i..].starts_with(w.as_bytes()), "at {}", self.i);
        self.i += w.len();
        v
    }

    fn string(&mut self) -> String {
        self.eat(b'"');
        let start = self.i;
        while self.s[self.i] != b'"' {
            assert_ne!(self.s[self.i], b'\\', "no escapes in the spots");
            self.i += 1;
        }
        let out = std::str::from_utf8(&self.s[start..self.i])
            .unwrap()
            .to_owned();
        self.i += 1;
        out
    }

    fn value(&mut self) -> Json {
        self.ws();
        match self.s[self.i] {
            b'n' => self.word("null", Json::Null),
            b't' => self.word("true", Json::Bool(true)),
            b'f' => self.word("false", Json::Bool(false)),
            b'"' => Json::Str(self.string()),
            b'[' => {
                self.i += 1;
                let mut out = Vec::new();
                self.ws();
                if self.s[self.i] == b']' {
                    self.i += 1;
                    return Json::Arr(out);
                }
                loop {
                    out.push(self.value());
                    self.ws();
                    self.i += 1;
                    if self.s[self.i - 1] == b']' {
                        return Json::Arr(out);
                    }
                }
            }
            b'{' => {
                self.i += 1;
                let mut out = BTreeMap::new();
                self.ws();
                if self.s[self.i] == b'}' {
                    self.i += 1;
                    return Json::Obj(out);
                }
                loop {
                    let k = self.string();
                    self.eat(b':');
                    out.insert(k, self.value());
                    self.ws();
                    self.i += 1;
                    if self.s[self.i - 1] == b'}' {
                        return Json::Obj(out);
                    }
                }
            }
            _ => {
                let start = self.i;
                while self.i < self.s.len() && b"-+.eE0123456789".contains(&self.s[self.i]) {
                    self.i += 1;
                }
                Json::Num(
                    std::str::from_utf8(&self.s[start..self.i])
                        .unwrap()
                        .to_owned(),
                )
            }
        }
    }
}

/// What v10.0.2 did at a spot, as the log writes it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Recorded {
    Fold,
    Check,
    Call,
    RaiseTo(i64),
    AllIn,
}

pub struct Spot {
    pub seed: u64,
    pub situation: Situation,
    pub action: Recorded,
}

fn street(s: &str) -> Street {
    match s {
        "Preflop" => Street::Preflop,
        "Flop" => Street::Flop,
        "Turn" => Street::Turn,
        "River" => Street::River,
        other => panic!("street {other}"),
    }
}

fn kind(s: &str) -> ActionKind {
    match s {
        "Fold" => ActionKind::Fold,
        "Check" => ActionKind::Check,
        "Call" => ActionKind::Call,
        "Raise" => ActionKind::Raise,
        "AllIn" => ActionKind::AllIn,
        other => panic!("kind {other}"),
    }
}

fn card(j: &Json) -> Card {
    Card::parse(j.str()).expect("card")
}

fn situation(j: &Json) -> Situation {
    let hole = j.get("hole").arr();
    Situation {
        hero_seat: j.get("hero_seat").u64() as usize,
        hole: [card(&hole[0]), card(&hole[1])],
        board: j.get("board").arr().iter().map(card).collect(),
        street: street(j.get("street").str()),
        button: j.get("button").u64() as usize,
        bb: j.get("bb").i64(),
        pot: j.get("pot").i64(),
        call_amount: j.get("call_amount").i64(),
        current_bet_to: j.get("current_bet_to").opt_i64(),
        can_check: j.get("can_check").bool(),
        min_raise_to: j.get("min_raise_to").opt_i64(),
        max_raise_to: j.get("max_raise_to").opt_i64(),
        players: j
            .get("players")
            .arr()
            .iter()
            .map(|p| PlayerInfo {
                seat: p.get("seat").u64() as usize,
                stack: p.get("stack").i64(),
                bet: p.get("bet").i64(),
                folded: p.get("folded").bool(),
            })
            .collect(),
        history: j
            .get("history")
            .arr()
            .iter()
            .map(|r| ActionRecord {
                seat: r.get("seat").u64() as usize,
                street: street(r.get("street").str()),
                kind: kind(r.get("kind").str()),
                to: r.get("to").i64(),
                pot_before: r.get("pot_before").i64(),
                to_call_before: r.get("to_call_before").i64(),
                bet_before: r.get("bet_before").i64(),
                full_raise: r.get("full_raise").bool(),
            })
            .collect(),
    }
}

fn recorded(j: &Json) -> Recorded {
    let a = j.arr();
    match a[0].str() {
        "fold" => Recorded::Fold,
        "check" => Recorded::Check,
        "call" => Recorded::Call,
        "raise" => Recorded::RaiseTo(a[1].i64()),
        "all_in" => Recorded::AllIn,
        other => panic!("action {other}"),
    }
}

/// Every recorded spot, in the order v10 played them.
pub fn spots() -> Vec<Spot> {
    SPOTS
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|line| {
            let j = Json::parse(line);
            Spot {
                seed: j.get("seed").u64(),
                situation: situation(j.get("situation")),
                action: recorded(j.get("action")),
            }
        })
        .collect()
}

/// The FNV-1a digest of the values' bits.
pub fn digest(values: &[f32]) -> u64 {
    values.iter().fold(0xcbf2_9ce4_8422_2325, |hash, x| {
        x.to_bits().to_le_bytes().iter().fold(hash, |hash, &b| {
            (hash ^ u64::from(b)).wrapping_mul(0x100_0000_01b3)
        })
    })
}
