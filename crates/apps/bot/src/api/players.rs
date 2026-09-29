//! Player cards (0217): everything we know about one opponent in one payload, for the dashboard's
//! click-a-seat card: leaderboard standing, style, every rate the decisions price them with next to
//! the league (population) rate, our per-opponent corrections for them, and our fleet-wide record
//! against them, actual and all-in EV (0213).

use super::*;
use sv10_core::model::{HandSummary, ModelStore, Profile, hand_stats};
use sv10_store::store::PlayerHand;

fn profile_json(p: &Profile) -> Value {
    json!({
        "vpip": p.vpip, "pfr": p.pfr, "open_raise": p.open_raise, "limp": p.limp, "three_bet": p.three_bet,
        "call_open": p.call_open, "fold_to_3bet": p.fold_to_3bet, "four_bet": p.four_bet, "fold_to_4bet": p.fold_to_4bet,
        "cbet": p.cbet, "fold_to_cbet": p.fold_to_cbet, "wtsd": p.wtsd, "won_showdown": p.won_showdown, "river_bluff": p.river_bluff,
        "bet_first": p.bet_first, "fold_vs_bet": p.fold_vs_bet, "raise_vs_bet": p.raise_vs_bet, "vpip_pos": p.vpip_pos, "open_pos": p.open_pos,
    })
}

/// Hands with a result the card names one by one (0296).
const RECENT_HANDS: usize = 10;

/// Our record against a player over `hands` (oldest first) played by `ours`, in big blinds of `bb`.
///
/// `current` maps a stored bot name to the name that seat answers to today
/// ([`crate::live::Shared::current_name`]): rows written under a bot's earlier name still group
/// under one seat and still point at a replay, and anything here that groups or picks a bot goes
/// through it.
pub(super) fn record(hands: &[PlayerHand], name: &str, ours: &[String], bb: f64, current: impl Fn(&str) -> String) -> Value {
    let (mut n, mut net, mut ev, mut sq, mut evsq) = (0f64, 0f64, 0f64, 0f64, 0f64);
    let (mut won_pots, mut lost_pots) = (0usize, 0usize);
    let mut best: Option<&PlayerHand> = None;
    let mut worst: Option<&PlayerHand> = None;
    let mut by_bot: std::collections::BTreeMap<String, (usize, i64)> = Default::default();
    let mut series = Vec::new();
    let step = (hands.len() / 120).max(1);
    for (i, h) in hands.iter().enumerate() {
        let Some(x) = h.net else { continue };
        let e = h.ev_net.unwrap_or(x as f64);
        n += 1.0;
        net += x as f64;
        ev += e;
        sq += (x * x) as f64;
        evsq += e * e;
        let winners: Vec<&str> = h.winners.split(',').collect();
        if x > 0 && winners.iter().any(|w| ours.iter().any(|o| o == w)) {
            won_pots += 1;
        } else if x < 0 && winners.contains(&name) {
            lost_pots += 1;
        }
        if best.is_none_or(|b| x > b.net.unwrap_or(0)) {
            best = Some(h);
        }
        if worst.is_none_or(|w| x < w.net.unwrap_or(0)) {
            worst = Some(h);
        }
        let b = by_bot.entry(current(&h.bot)).or_default();
        b.0 += 1;
        b.1 += x;
        if i % step == 0 || i + 1 == hands.len() {
            series.push(json!({"hand": i + 1, "net": net.round(), "ev": ev.round()}));
        }
    }
    let rate = |sum: f64, sq: f64| {
        if n < 2.0 {
            return (Value::Null, Value::Null);
        }
        let m = sum / n;
        let sd = (sq / n - m * m).max(0.0).sqrt();
        (json!(m / bb * 100.0), json!(1.96 * sd / n.sqrt() / bb * 100.0))
    };
    let (bb100, ci) = rate(net, sq);
    let (ev_bb100, ev_ci) = rate(ev, evsq);
    let form: Vec<&str> = hands
        .iter()
        .rev()
        .filter_map(|h| h.net)
        .take(10)
        .map(|x| {
            if x > 0 {
                "W"
            } else if x < 0 {
                "L"
            } else {
                "="
            }
        })
        .collect();
    let pick = |h: Option<&PlayerHand>| {
        h.map(|h| json!({"hand_id": h.hand_id, "bot": current(&h.bot), "net": h.net, "pot": h.pot, "ts": parse_ts(&h.ended_at)}))
    };
    // The newest hands with a result, newest first: what the card's key-hand list names one by one.
    let recent: Vec<Value> = hands.iter().rev().filter(|h| h.net.is_some()).filter_map(|h| pick(Some(h))).take(RECENT_HANDS).collect();
    json!({
        "hands": n as i64, "net": net.round(), "ev_net": ev.round(), "bb100": bb100, "confidence": ci, "ev_bb100": ev_bb100, "ev_confidence": ev_ci,
        "won_pots": won_pots, "lost_pots": lost_pots, "biggest_win": pick(best.filter(|b| b.net.unwrap_or(0) > 0)),
        "biggest_loss": pick(worst.filter(|w| w.net.unwrap_or(0) < 0)),
        "by_bot": by_bot.into_iter().map(|(bot, (hands, net))| json!({"bot": bot, "hands": hands, "net": net})).collect::<Vec<_>>(),
        "form": form, "series": series, "recent": recent,
    })
}

/// Style, scouting advice, the priced rates, the league's and our corrections for `name`.
pub(super) fn scouting(models: &ModelStore, name: &str) -> Value {
    let stats = models.players.get(name).cloned().unwrap_or_default();
    let (style, advice) = style_of(&stats);
    let p = models.profile(name);
    // An unobserved name reads as the population prior: the league average the rates are shrunk to.
    let league = models.profile("\u{0}league");
    json!({
        "style": style, "advice": advice, "hands_observed": stats.hands.round(), "confidence": p.confidence,
        "read": profile_json(&p), "league": profile_json(&league),
        "corrections": {
            "fold_offset": (p.fold_logit_offset != 0.0).then_some(p.fold_logit_offset),
            "response_ratio": (p.response_ratio != sv10_core::residual::UNIT_RATIO).then_some(p.response_ratio),
            "size_tell": (p.size_tell != 0.0).then_some(p.size_tell),
        },
    })
}

/// A model holding only `name`'s stats, read from their seat in `hands`, beside the fleet's
/// population as the league: our own seats are never modelled, so their card reads our hands.
pub(super) fn own_model(models: &ModelStore, name: &str, hands: &[HandSummary]) -> ModelStore {
    let mut own = ModelStore { population: models.population.clone(), ..Default::default() };
    let stats = own.players.entry(name.to_string()).or_default();
    for h in hands {
        let Some((seat, _)) = h.players.iter().find(|(_, n)| n == name) else { continue };
        if let Some(s) = hand_stats(h).get(seat) {
            stats.merge_weighted(s, 1.0);
        }
    }
    own
}

/// Hands of our own seat the self card reads its style from.
const OWN_CARD_HANDS: usize = 5_000;

/// Shared hands before the seat card calls a rivalry read; the Rivals panel's floor.
const MIN_SEAT_HANDS: f64 = 150.0;

pub(super) async fn player_card(State(s): State<Arc<Shared>>, Path(name): Path<String>) -> Response {
    let standing = leaderboard_entry(&name).await;
    let shared = s.clone();
    let card = off_runtime(move || -> Result<Option<Value>, ApiError> {
        let ours: Vec<String> = shared.bots.iter().map(|b| b.read().name.clone()).collect();
        let is_us = ours.contains(&name);
        let mut card = if is_us {
            let summaries: Vec<HandSummary> = store_read("own hands", shared.store.recent_hands(&name, OWN_CARD_HANDS))?
                .iter()
                .filter_map(|h| serde_json::from_str(&h.summary).ok())
                .collect();
            let own = own_model(&shared.models.read(), &name, &summaries);
            let mut c = scouting(&own, &name);
            c["advice"] = json!("Our own seat, read from its last hands the way we read opponents: this is what the table sees.");
            c
        } else {
            scouting(&shared.models.read(), &name)
        };
        let known = shared.models.read().players.contains_key(&name);
        let hands = store_read("hands with player", shared.store.hands_with_player(&name))?;
        if !known && !is_us && hands.is_empty() {
            return Ok(None);
        }
        card["name"] = json!(name);
        card["avatar_url"] = json!(shared.avatars.read().get(&name).and_then(|a| crate::avatar_url(&shared.config.rest_base, a)));
        card["reputation"] = json!(shared.reputation.read().get(&name).cloned());
        card["vs_us"] = record(&hands, &name, &ours, shared.big_blind(), |b| shared.current_name(b));
        // Our net in the hands they sat in is a table result; the head-to-head read is the chip
        // flow attributed to their seat, the number every other surface shows (0277).
        card["vs_seat"] = if is_us {
            Value::Null
        } else {
            crate::headtohead::read_one(&shared.head_to_head.read(), &name, MIN_SEAT_HANDS, shared.big_blind()).unwrap_or(Value::Null)
        };
        card["ours"] = json!(is_us);
        Ok(Some(card))
    })
    .await;
    match card.and_then(|r| r) {
        Err(r) => r.into_response(),
        Ok(Some(mut c)) => {
            c["leaderboard"] = standing.unwrap_or(Value::Null);
            Json(c).into_response()
        }
        Ok(None) => (StatusCode::NOT_FOUND, Json(json!({"detail": "No hands with this player"}))).into_response(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hand(bot: &str, net: i64, ev: Option<f64>, winners: &str) -> PlayerHand {
        PlayerHand {
            bot: bot.into(),
            hand_id: format!("h{net}"),
            ended_at: "2026-09-24T00:00:00Z".into(),
            net: Some(net),
            ev_net: ev,
            pot: net.abs() * 2,
            winners: winners.into(),
        }
    }

    #[test]
    fn the_record_counts_pots_won_and_lost_and_reads_all_in_ev() {
        let ours = vec!["A".to_string(), "B".to_string()];
        let hands = vec![
            hand("A", 400, None, "A"),
            hand("B", -1000, Some(300.0), "villain"),
            hand("A", -20, None, "someone"),
            hand("B", 60, None, "B"),
        ];
        let r = record(&hands, "villain", &ours, 20.0, |b| b.to_string());
        assert_eq!(r["hands"], json!(4));
        assert_eq!(r["net"], json!(-560.0));
        assert_eq!(r["ev_net"], json!(740.0), "the all-in loss counts at its EV");
        assert_eq!((r["won_pots"].as_u64(), r["lost_pots"].as_u64()), (Some(2), Some(1)));
        assert_eq!(r["biggest_loss"]["net"], json!(-1000));
        assert_eq!(r["biggest_win"]["net"], json!(400));
        assert_eq!(r["form"], json!(["W", "L", "L", "W"]));
        assert_eq!(r["by_bot"][0], json!({"bot": "A", "hands": 2, "net": 380}));
        assert!(r["bb100"].as_f64().is_some() && r["ev_bb100"].as_f64().unwrap() > r["bb100"].as_f64().unwrap());
    }

    /// 0296: the card's key hands and per-bot split name the seat as it is today, so a hand stored
    /// under a bot's earlier name still resolves to a live slot and keeps its replay link.
    #[test]
    fn a_renamed_bots_hands_still_resolve_to_its_current_seat() {
        let ours = vec!["SvanBotV10".to_string()];
        let mut hands: Vec<PlayerHand> = (0..12).map(|i| hand("SvanBotV7", i * 10 - 50, None, "villain")).collect();
        hands.push(hand("SvanBotV10", 100, None, "SvanBotV10"));
        let rename = |b: &str| if b == "SvanBotV7" { "SvanBotV10".to_string() } else { b.to_string() };
        let r = record(&hands, "villain", &ours, 20.0, rename);
        assert_eq!(r["by_bot"], json!([{"bot": "SvanBotV10", "hands": 13, "net": 160}]), "one seat, not two");
        let recent = r["recent"].as_array().unwrap();
        assert_eq!(recent.len(), 10, "the newest ten");
        assert_eq!(recent[0]["hand_id"], json!("h100"), "newest first");
        assert_eq!(recent[1]["hand_id"], json!("h60"), "and the rest in order");
        assert!(recent.iter().all(|h| h["bot"] == json!("SvanBotV10")), "{recent:?}");
        assert_eq!(r["recent"][0]["pot"], json!(200));
    }

    #[test]
    fn our_own_card_reads_our_seat_against_the_real_league() {
        use sv10_core::engine::{ActionKind, ActionRecord, Street};
        let fold = |seat| ActionRecord {
            seat,
            street: Street::Preflop,
            kind: ActionKind::Fold,
            to: 0,
            pot_before: 30,
            to_call_before: 20,
            bet_before: 0,
            full_raise: false,
            think_ms: None,
            street_open: false,
        };
        let h = HandSummary {
            players: vec![(0, "us".into()), (1, "villain".into())],
            button: 0,
            bb: 20,
            history: vec![fold(0)],
            board: vec![],
            shown: vec![],
            stacks: [(0, 2_000), (1, 2_000)].into_iter().collect(),
        };
        let mut league = ModelStore::default();
        league.population.hands = 5_000.0;
        let own = own_model(&league, "us", &vec![h; 30]);
        assert_eq!(own.players.get("us").map(|s| s.hands), Some(30.0), "only our seat is read");
        assert!(!own.players.contains_key("villain"));
        assert_eq!(own.population.hands, 5_000.0, "the league is the fleet's population, not our hands");
        assert!(own.profile("us").vpip < own.profile("\u{0}league").vpip, "folding every hand reads tighter than the league");
    }

    #[test]
    fn scouting_puts_the_players_rates_beside_the_league() {
        let models = ModelStore {
            fold_offsets: Arc::new([("villain".to_string(), -0.5)].into_iter().collect()),
            size_tells: Arc::new([("villain".to_string(), 0.7f32)].into_iter().collect()),
            ..Default::default()
        };
        let c = scouting(&models, "villain");
        assert!(c["read"]["vpip"].as_f64().is_some() && c["league"]["vpip"].as_f64().is_some());
        assert_eq!(c["corrections"]["fold_offset"], json!(-0.5));
        assert!((c["corrections"]["size_tell"].as_f64().unwrap() - 0.7).abs() < 1e-6, "the river sizing tell the seat reads is in force");
        assert!(c["corrections"]["response_ratio"].is_null());
        assert_eq!(c["hands_observed"], json!(0.0));
        // The card shows a correction only while it is in force: an unfitted name reads neutral.
        assert!(scouting(&ModelStore::default(), "villain")["corrections"]["size_tell"].is_null());
    }
}
