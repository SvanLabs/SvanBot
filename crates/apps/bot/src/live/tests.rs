//! `Shared::for_test` and the unit tests for the fleet-wide live state (0320: split out of
//! `live.rs`). The impl is test-only, and an inherent impl may sit in any module of the crate.

use super::*;

#[cfg(test)]
impl Shared {
    /// A fleet of `names` backed by a fresh store under the temp dir, for handler tests.
    pub(crate) fn for_test(tag: &str, names: &[&str]) -> std::sync::Arc<Shared> {
        let dir = std::env::temp_dir().join(format!("sv10-shared-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let bots: Vec<_> = names.iter().map(|n| crate::config::BotConfig { name: n.to_string(), api_key: String::new() }).collect();
        let config = Config {
            bots: bots.clone(),
            ws_url: String::new(),
            rest_base: String::new(),
            max_buy_in: 5000,
            seek_top_rank: 30,
            bank_stack_bb: 2_000,
            bank_until_chips: 500_000,
            web_host: "127.0.0.1".into(),
            web_port: 0,
            tv_host: "127.0.0.1".into(),
            tv_port: 0,
            operator_token: None,
            dry_run: true,
            artifacts: dir.clone(),
            web_dist: dir.clone(),
            archive_dir: dir.join("archive"),
            root: dir.clone(),
            supervised: false,
            export_cap: 0,
            head: false,
            worker: false,
            numbers: Default::default(),
        };
        std::sync::Arc::new(Shared {
            bots: bots
                .iter()
                .enumerate()
                .map(|(slot, b)| {
                    RwLock::new(BotLive { slot, name: b.name.clone(), desired: "run".into(), big_blind: 20, ..Default::default() })
                })
                .collect(),
            config,
            models: RwLock::new(ModelStore::default()),
            params: RwLock::new(Params::default()),
            nn: RwLock::new(None),
            reputation: RwLock::new(Default::default()),
            head_to_head: RwLock::new(Default::default()),
            avatars: RwLock::new(Default::default()),
            store: Store::open(&dir.join("svanbot10.db")).unwrap(),
            log: Mutex::new(VecDeque::new()),
            events: broadcast::channel(64).0,
            started_at: String::new(),
            restart_requested: Default::default(),
            unstored_hands: Default::default(),
            aliases: Default::default(),
            season_clock: RwLock::new(Default::default()),
            current_season: RwLock::new(None),
            champion_version: RwLock::new("test".into()),
            experiment: RwLock::new(Default::default()),
            resumable: Default::default(),
        })
    }
}

#[test]
fn big_blind_follows_the_tables_not_a_constant() {
    let shared = Shared::for_test("bb", &["A", "B"]);
    shared.update(0, |b| b.big_blind = 0);
    shared.update(1, |b| b.big_blind = 50);
    assert_eq!(shared.big_blind(), 50.0);
    shared.update(1, |b| b.big_blind = 0);
    assert_eq!(shared.big_blind(), DEFAULT_BIG_BLIND as f64);
}

#[test]
fn heartbeats_round_trip_and_reject_stale_or_future_clocks() {
    use super::*;
    let b = BotLive { name: "W".into(), mode: "playing".into(), session_hands: 7, ..Default::default() };
    let v = wrap_heartbeat(&b, 1000.0);
    let (back, age) = unwrap_heartbeat(&v, 1004.0).expect("fresh heartbeat parses");
    assert_eq!((back.name.as_str(), back.mode.as_str(), back.session_hands, age), ("W", "playing", 7, 4.0));
    assert!(unwrap_heartbeat(&v, 1000.0 + HEARTBEAT_STALE_SECS + 1.0).is_some(), "unwrap does not judge staleness");
    assert!(unwrap_heartbeat(&v, 900.0).is_none(), "heartbeat from the future is rejected");
    assert!(unwrap_heartbeat(&serde_json::json!({"hb": 1.0}), 2.0).is_none(), "missing state is rejected");
}

#[test]
fn state_hash_totals_survive_a_restart_and_flush_with_the_models() {
    // 2026-09-27 (0301): the counters lived only in one process's memory, so a hot swap reset
    // them: the Runtime health panel read "367 / 0" for hours after a release, and a serializer
    // that had just started diverging looked exactly like a healthy fleet.
    use super::*;
    let shared = Shared::for_test("hash-totals", &["A"]);
    {
        let mut b = shared.bots[0].write();
        count_state_hash(&mut b, true, None);
        count_state_hash(&mut b, true, None);
        count_state_hash(
            &mut b,
            false,
            Some(StateHashMismatch {
                at: 1_790_000_000.0,
                table: "t1".into(),
                verdict: "DIVERGED".into(),
                summary: "DIVERGED table_seq=9 (last applied 8) pot=number".into(),
            }),
        );
    }
    // The flush rides the model checkpoint (tasks::save_models), which runs every five minutes,
    // at shutdown and before a hot swap.
    crate::tasks::save_models(&shared);

    // The next process: same store, bots fresh from config.
    let bots = vec![RwLock::new(BotLive { name: "A".into(), ..Default::default() })];
    load_state_hash_totals(&shared.store, &bots);
    let b = bots[0].read();
    assert_eq!((b.state_hash_lifetime.ok, b.state_hash_lifetime.bad), (2, 1), "lifetime tallies carry over");
    assert_eq!((b.state_hash_ok, b.state_hash_bad), (0, 0), "the session counters start at zero");
    let m = b.state_hash_lifetime.last_mismatch.as_ref().expect("the newest mismatch is remembered");
    assert_eq!((m.at, m.table.as_str(), m.verdict.as_str()), (1_790_000_000.0, "t1", "DIVERGED"));
    assert!(m.summary.contains("last applied 8"), "{}", m.summary);
    drop(b);

    // A bot the store has never counted for loads zero rather than someone else's numbers.
    let other = Shared::for_test("hash-totals-fresh", &["B"]);
    let fresh = vec![RwLock::new(BotLive { name: "B".into(), ..Default::default() })];
    load_state_hash_totals(&other.store, &fresh);
    assert_eq!(fresh[0].read().state_hash_lifetime, StateHashTotals::default());
}

#[test]
fn desired_mode_commands_nudge_stuck_bots() {
    use super::*;
    let mut b = BotLive { mode: "paused".into(), ..Default::default() };
    apply_desired(&mut b, "run");
    assert_eq!((b.desired.as_str(), b.mode.as_str()), ("run", "connecting"));
    apply_desired(&mut b, "pause");
    assert_eq!((b.desired.as_str(), b.mode.as_str()), ("pause", "connecting"));
}

#[test]
fn decision_view_mirrors_the_decision_and_situation() {
    use super::*;
    use sv10_rng::SeedableRng;
    let sit = sv10_core::situation::fixtures::river_jam_with_all_ins();
    let mut rng = sv10_rng::rngs::SmallRng::seed_from_u64(11);
    let d = sv10_core::policy::decide_with(&sit, &ModelStore::default(), &Params::default(), None, &mut rng);
    let v = DecisionView::for_decision("h1", &sit, &d, "fold".into(), None, 1.5, "v-test".into());
    assert_eq!((v.hand_id.as_str(), v.street.as_str(), v.action.as_str(), v.amount), ("h1", sit.street.name(), "fold", None));
    assert_eq!((v.equity, v.pot_odds, v.pot, v.to_call, v.latency_ms), (d.equity, d.pot_odds, sit.pot, sit.call_amount, 1.5));
    assert_eq!(v.hole.len(), 2);
    assert_eq!(v.board.len(), sit.board.len());
    assert_eq!(v.candidates.len(), d.candidates.len());
    assert_eq!(v.opponents, sit.live_opponents().count());
    assert_eq!((v.reason.as_str(), v.version.as_str()), (d.reason.as_str(), "v-test"));
}

#[test]
fn remote_bot_reads_fresh_heartbeats_and_rejects_the_rest() {
    use super::*;
    let shared = Shared::for_test("remote", &["W"]);
    let beat = |name: &str, v: serde_json::Value| {
        shared.store.put_kv(&heartbeat_key(name), &v.to_string()).unwrap();
    };
    let b = BotLive { name: "W".into(), mode: "playing".into(), ..Default::default() };
    beat("W", wrap_heartbeat(&b, 1000.0));
    let (back, age) = read_remote_bot(&shared.store, "W", 1004.0).expect("fresh heartbeat reads");
    assert_eq!((back.mode.as_str(), age), ("playing", 4.0));
    assert!(read_remote_bot(&shared.store, "W", 1000.0 + HEARTBEAT_STALE_SECS + 1.0).is_none(), "stale heartbeat refused");
    assert!(read_remote_bot(&shared.store, "nobody", 1004.0).is_none(), "missing heartbeat refused");
    beat("W", serde_json::json!({"hb": 1000.0}));
    assert!(read_remote_bot(&shared.store, "W", 1004.0).is_none(), "shapeless heartbeat refused");
    beat("W", wrap_heartbeat(&b, 2000.0));
    assert!(read_remote_bot(&shared.store, "W", 1004.0).is_none(), "future heartbeat refused");
}
