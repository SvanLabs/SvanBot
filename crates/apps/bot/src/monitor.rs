//! `monitor` — the read-only results monitor: one line per event worth acting on (0317).
//!
//! Ported from `scripts/monitor.py`, whose stdout is an event stream the dashboard reads back by
//! kind (`api/monitor.rs`) and `scripts/supervisors.sh` restarts with the fleet, so the port keeps
//! the same kinds, flags and wording:
//!
//!   START     the ledger the run opened with
//!   SUMMARY   every `--summary-min` minutes: fleet and per-bot net, top donors and takers in the
//!             window, and the toughest all-time opponents (bb/100 moved between us and them in
//!             champion hands, each one ranked without its own biggest pot — 0209)
//!   BIGWIN /  a single hand at or over `--big-win-bb` / `--big-loss-bb` big blinds
//!   BIGLOSS
//!   NEMESIS   an opponent newly beating us at 95% family-wise across every opponent with
//!             `--min-hands` shared hands (0221); experiment-arm treatment hands are left out of
//!             the ledger, as in the bot's own ledger (0361), but stay in the window, which is a
//!             results read where the chips are the answer
//!   STALL     a bot with no completed hand for `--stall-min` minutes
//!   ERROR     new error-level events from the fleet log
//!   MONITOR   a failed pass; the loop keeps running
//!
//! The chips-per-opponent math is [`sv10_core::flow`] and the all-time tallies are
//! [`crate::headtohead::Ledger`]: one implementation each, shared with `review` and the dashboard.
//! The style label in OPPONENTS is [`crate::style::style_of`], the same classifier the scout view
//! uses (the Python monitor carried its own copy; 0246).
//!
//! Usage: `monitor [--once] [--interval 300] [--summary-min 30] [--big-loss-bb 100]
//!                 [--big-win-bb BB] [--min-hands 150] [--stall-min 20] [--active-hours 2]`

use anyhow::Result;
use chrono::{DateTime, Local};
use std::collections::{BTreeSet, HashMap, HashSet};
use std::path::Path;
use sv10_core::model::{HandSummary, ModelStore};
use sv10_stats::normal::family_z;
use sv10_store::store::{MonitorRow, Store};

/// The monitor's flags, with the Python script's defaults.
#[derive(Clone, Copy, Debug)]
pub struct Options {
    pub interval: u64,
    pub summary_min: u64,
    pub big_loss_bb: f64,
    pub big_win_bb: f64,
    pub min_hands: f64,
    pub stall_min: u64,
    pub active_hours: f64,
    pub once: bool,
}

impl Default for Options {
    fn default() -> Self {
        Options {
            interval: 300,
            summary_min: 30,
            big_loss_bb: 100.0,
            big_win_bb: 100.0,
            min_hands: 150.0,
            stall_min: 20,
            active_hours: 2.0,
            once: false,
        }
    }
}

impl Options {
    /// Parse the command line; `--big-win-bb` defaults to `--big-loss-bb`.
    pub fn parse(args: &[String]) -> Result<Options> {
        let mut o = Options::default();
        let usage = "usage: monitor [--once] [--interval 300] [--summary-min 30] [--big-loss-bb 100] \
                     [--big-win-bb BB] [--min-hands 150] [--stall-min 20] [--active-hours 2]";
        let mut big_win: Option<f64> = None;
        let mut i = 0;
        while i < args.len() {
            let arg = args[i].as_str();
            let mut value = |name: &str| -> Result<f64> {
                i += 1;
                args.get(i)
                    .ok_or_else(|| anyhow::anyhow!("{name} needs a value\n{usage}"))?
                    .parse()
                    .map_err(|_| anyhow::anyhow!("{name} takes a number\n{usage}"))
            };
            match arg {
                "--once" => o.once = true,
                "--interval" => o.interval = value("--interval")? as u64,
                "--summary-min" => o.summary_min = value("--summary-min")? as u64,
                "--big-loss-bb" => o.big_loss_bb = value("--big-loss-bb")?,
                "--big-win-bb" => big_win = Some(value("--big-win-bb")?),
                "--min-hands" => o.min_hands = value("--min-hands")?,
                "--stall-min" => o.stall_min = value("--stall-min")? as u64,
                "--active-hours" => o.active_hours = value("--active-hours")?,
                _ => anyhow::bail!("unknown argument {arg:?}\n{usage}"),
            }
            i += 1;
        }
        o.big_win_bb = big_win.unwrap_or(o.big_loss_bb);
        Ok(o)
    }
}

/// Local wall-clock seconds, the clock the monitor's stall watch and summary window run on.
pub fn unix_now() -> f64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs_f64()).unwrap_or(0.0)
}

/// `HH:MM` on the operator's own clock.
fn stamp() -> String {
    Local::now().format("%H:%M").to_string()
}

pub struct Monitor {
    store: Store,
    opts: Options,
    /// Every bot name that has ever stored a hand; refreshed each pass.
    fleet: HashSet<String>,
    /// The all-time flow ledger, ordinary hands only (0361).
    ledger: crate::headtohead::Ledger,
    /// Rowid of the last hand read, and id of the last event read.
    last: i64,
    last_event: i64,
    /// When each active bot last had a hand, and the ones already reported stalled.
    last_hand_at: HashMap<String, f64>,
    stalled: HashSet<String>,
    /// The summary window: per bot (hands, net chips), per opponent bb, and names seen. The order
    /// vectors record first sight, which is the tie-break the Python monitor's stable sorts used.
    window_bot: HashMap<String, (i64, i64)>,
    window_opp: HashMap<String, f64>,
    window_opp_order: Vec<String>,
    window_seen: HashMap<String, i64>,
    window_seen_order: Vec<String>,
    known: HashSet<String>,
    /// The opponents already announced as nemeses.
    nemeses: BTreeSet<String>,
    window_start: f64,
}

impl Monitor {
    /// Open the store under `root/artifacts/svanbot10.db` and build the whole-history ledger.
    pub fn open(root: &Path, opts: Options) -> Result<Monitor> {
        let path = root.join("artifacts").join("svanbot10.db");
        anyhow::ensure!(path.exists(), "no store at {} — start the fleet first", path.display());
        let now = unix_now();
        let store = Store::open(&path)?;
        let fleet: HashSet<String> = store.bot_names()?.into_iter().collect();
        let names: Vec<String> = fleet.iter().cloned().collect();
        let mut ledger = crate::headtohead::Ledger::default();
        ledger.update(&store, &names);
        let last = store.max_hand_rowid()?;
        let last_event = store.max_event_id()?;
        let last_hand_at = active_bots(&store, opts.active_hours, now)?;
        // Everyone the ledger already knows: the first summary does not call them new.
        let known = ledger.table.keys().cloned().collect();
        let mut monitor = Monitor {
            store,
            opts,
            fleet,
            ledger,
            last,
            last_event,
            last_hand_at,
            stalled: HashSet::new(),
            window_bot: HashMap::new(),
            window_opp: HashMap::new(),
            window_opp_order: Vec::new(),
            window_seen: HashMap::new(),
            window_seen_order: Vec::new(),
            known,
            nemeses: BTreeSet::new(),
            window_start: now,
        };
        monitor.nemeses = monitor.nemesis_set();
        Ok(monitor)
    }

    /// The opening line: what the ledger holds and who beats us (family-wise, [`Options::min_hands`]).
    pub fn start_line(&self) -> String {
        format!(
            "{} START monitor: {} hands, {} opponents, {} beat us at 95%: {}",
            stamp(),
            self.last,
            self.ledger.table.len(),
            self.nemeses.len(),
            if self.nemeses.is_empty() { "none".to_string() } else { self.nemeses.iter().cloned().collect::<Vec<_>>().join(", ") }
        )
    }

    /// Opponents beating us at 95% family-wise across every opponent with enough shared hands.
    fn nemesis_set(&self) -> BTreeSet<String> {
        let tested = crate::headtohead::tested(&self.ledger.table, self.opts.min_hands);
        self.ledger.table.iter().filter(|(_, t)| t.beats_us(self.opts.min_hands, tested)).map(|(p, _)| p.clone()).collect()
    }

    /// One pass: every hand and event stored since the last one, then the summary when its window
    /// has come round. `once` forces the summary, as a one-shot run has no later pass to print it.
    /// A failed pass is reported as a MONITOR line, never fatal — the loop keeps running.
    pub fn pass(&mut self, now: f64, once: bool) -> Vec<String> {
        if once {
            self.window_start = 0.0;
        }
        let mut out = Vec::new();
        if let Err(e) = self.pass_inner(now, &mut out) {
            out.push(format!("{} MONITOR db error: {e}", stamp()));
        }
        out
    }

    fn pass_inner(&mut self, now: f64, out: &mut Vec<String>) -> Result<()> {
        self.fleet.extend(self.store.bot_names()?);
        let fleet: Vec<String> = self.fleet.iter().cloned().collect();
        for row in self.store.results_after(self.last)? {
            self.new_hand(row, now, &fleet, out);
        }
        // The ledger reads its own watermark and its own population (ordinary hands only).
        self.ledger.update(&self.store, &fleet);
        self.report_nemeses(out);
        self.report_stalls(now, out);
        self.report_errors(out)?;
        self.report_summary(now, out)?;
        Ok(())
    }

    /// Fold one new hand into the window, and print its BIGWIN/BIGLOSS line.
    fn new_hand(&mut self, row: MonitorRow, now: f64, fleet: &[String], out: &mut Vec<String>) {
        let (rowid, bot, hand_id, net, summary, pot, winners) = row;
        self.last = rowid;
        self.last_hand_at.insert(bot.clone(), now);
        self.stalled.remove(&bot);
        let Some(net) = net else { return };
        let hand: Option<HandSummary> = serde_json::from_str(&summary).ok();
        let bb = hand.as_ref().map(|h| h.bb).filter(|bb| *bb > 0).unwrap_or(20);
        let entry = self.window_bot.entry(bot.clone()).or_insert((0, 0));
        entry.0 += 1;
        entry.1 += net;
        if let Some(h) = &hand {
            let hero = h.players.iter().find(|p| p.1 == bot).map(|p| p.0);
            let winner_names: Vec<&str> = winners.split(',').filter(|w| !w.is_empty()).collect();
            if let Some(flows) = hero.and_then(|hero| sv10_core::flow::flow_to_hero(h, pot, &winner_names, hero)) {
                for (seat, chips) in flows {
                    let Some(name) = h.players.iter().find(|p| p.0 == seat).map(|p| p.1.clone()) else { continue };
                    if !fleet.contains(&name) {
                        if !self.window_opp.contains_key(&name) {
                            self.window_opp_order.push(name.clone());
                        }
                        *self.window_opp.entry(name).or_insert(0.0) += chips / bb as f64;
                    }
                }
            }
            for (_, name) in h.players.iter().filter(|(_, n)| !fleet.contains(n)) {
                if !self.window_seen.contains_key(name) {
                    self.window_seen_order.push(name.clone());
                }
                *self.window_seen.entry(name.clone()).or_insert(0) += 1;
            }
        }
        let bb = bb as f64;
        if net as f64 >= self.opts.big_win_bb * bb {
            let against: Vec<&str> = hand
                .as_ref()
                .map(|h| h.players.iter().filter(|(_, n)| !fleet.contains(n)).map(|(_, n)| n.as_str()).collect())
                .unwrap_or_default();
            out.push(format!(
                "{} BIGWIN {} {:+} chips ({:+.0} bb) hand {}; against {}",
                stamp(),
                bot,
                net,
                net as f64 / bb,
                hand_id,
                if against.is_empty() { "?".to_string() } else { against.join(", ") }
            ));
        }
        if net as f64 <= -self.opts.big_loss_bb * bb {
            out.push(format!(
                "{} BIGLOSS {} {:+} chips ({:+.0} bb) hand {}; won by {}",
                stamp(),
                bot,
                net,
                net as f64 / bb,
                hand_id,
                if winners.is_empty() { "?" } else { &winners }
            ));
        }
    }

    /// Opponents who newly clear the family-wise bar since the last pass.
    fn report_nemeses(&mut self, out: &mut Vec<String>) {
        let tested = crate::headtohead::tested(&self.ledger.table, self.opts.min_hands);
        let z = family_z(tested);
        let proven = self.nemesis_set();
        for p in proven.difference(&self.nemeses) {
            let t = &self.ledger.table[p];
            out.push(format!(
                "{} NEMESIS {p} beats us: {:+.1} bb/100 moved to them over {} champion hands \
                 (experiment-arm hands excluded, 0361; family-wise 95% upper {:+.1}, z {z:.2})",
                stamp(),
                t.mean() * 100.0,
                t.hands as i64,
                t.upper(z) * 100.0
            ));
        }
        self.nemeses = proven;
    }

    fn report_stalls(&mut self, now: f64, out: &mut Vec<String>) {
        let mut stalling: Vec<String> = self
            .last_hand_at
            .iter()
            .filter(|(b, at)| !self.stalled.contains(*b) && now - **at > self.opts.stall_min as f64 * 60.0)
            .map(|(b, _)| b.clone())
            .collect();
        stalling.sort();
        for b in stalling {
            self.stalled.insert(b.clone());
            out.push(format!("{} STALL {b}: no completed hand for {}+ minutes", stamp(), self.opts.stall_min));
        }
    }

    fn report_errors(&mut self, out: &mut Vec<String>) -> Result<()> {
        for (_, bot, msg) in self.store.errors_after(self.last_event, 20)? {
            out.push(format!("{} ERROR {bot}: {}", stamp(), msg.chars().take(200).collect::<String>()));
        }
        self.last_event = self.store.max_event_id()?;
        Ok(())
    }

    fn report_summary(&mut self, now: f64, out: &mut Vec<String>) -> Result<()> {
        if now - self.window_start < self.opts.summary_min as f64 * 60.0 {
            return Ok(());
        }
        let hands: i64 = self.window_bot.values().map(|v| v.0).sum();
        let net: i64 = self.window_bot.values().map(|v| v.1).sum();
        let mut bots: Vec<(&String, &(i64, i64))> = self.window_bot.iter().collect();
        bots.sort_by(|a, b| a.0.cmp(b.0));
        let bots = bots.iter().map(|(b, v)| format!("{b} {:+}/{}", v.1, v.0)).collect::<Vec<_>>().join(" ");
        // Stable by value: equal flows keep the order the opponents were first seen.
        let mut ranked: Vec<(&String, f64)> = self.window_opp_order.iter().map(|p| (p, self.window_opp[p])).collect();
        ranked.sort_by(|a, b| a.1.total_cmp(&b.1));
        let bb_list = |rows: Vec<(&String, f64)>| {
            let joined = rows.iter().map(|(p, v)| format!("{p} {v:+.0}bb")).collect::<Vec<_>>().join(", ");
            if joined.is_empty() { "none".to_string() } else { joined }
        };
        let takers = bb_list(ranked.iter().take(3).filter(|(_, v)| *v < 0.0).map(|(p, v)| (*p, *v)).collect());
        let donors = bb_list(ranked.iter().rev().take(3).filter(|(_, v)| *v > 0.0).map(|(p, v)| (*p, *v)).collect());
        let mut worst: Vec<(f64, &String, &crate::headtohead::HeadToHead)> = self
            .ledger
            .table
            .iter()
            .filter(|(_, t)| t.hands >= self.opts.min_hands)
            .map(|(p, t)| (t.mean_without_biggest_loss(), p, t))
            .collect();
        worst.sort_by(|a, b| a.0.total_cmp(&b.0).then_with(|| a.1.cmp(b.1)));
        let season_worst = worst
            .iter()
            .take(3)
            .map(|(trimmed, p, t)| {
                let extra = if (trimmed - t.mean()).abs() < 0.5 {
                    String::new()
                } else {
                    format!(", {:+.0} without its biggest pot", trimmed * 100.0)
                };
                format!("{p} {:+.0}bb/100{extra} ({})", t.mean() * 100.0, t.hands as i64)
            })
            .collect::<Vec<_>>()
            .join(", ");
        out.push(format!(
            "{} SUMMARY {}m: {hands} hands {net:+} chips | {bots} | took the most from us: {takers} | \
             gave us the most: {donors} | toughest all-time (bb/100 moved between us and them in champion \
             hands, ranked without each one's biggest pot): {season_worst}",
            stamp(),
            self.opts.summary_min
        ));
        let mut new: Vec<String> = self.window_seen.keys().filter(|p| !self.known.contains(*p)).cloned().collect();
        new.sort();
        self.known.extend(self.window_seen.keys().cloned());
        let profiles = self.opponent_profiles()?;
        // Stable by count: equal counts keep the order the opponents were first seen.
        let mut busiest: Vec<(&String, &i64)> = self.window_seen_order.iter().map(|p| (p, &self.window_seen[p])).collect();
        busiest.sort_by(|a, b| b.1.cmp(a.1));
        let reads = busiest
            .iter()
            .take(4)
            .map(|(p, n)| {
                let read = profiles.get(*p).cloned().unwrap_or_else(|| "no model".to_string());
                format!("{p} ({read}; we {:+.0} bb against them over {n} hands)", self.window_opp.get(*p).copied().unwrap_or(0.0))
            })
            .collect::<Vec<_>>()
            .join(", ");
        let new_note =
            if new.is_empty() { String::new() } else { format!(": {}", new.iter().take(6).cloned().collect::<Vec<_>>().join(", ")) };
        out.push(format!(
            "{} OPPONENTS {} faced, {} new{new_note} | most played: {}",
            stamp(),
            self.window_seen.len(),
            new.len(),
            if reads.is_empty() { "none".to_string() } else { reads }
        ));
        self.window_bot.clear();
        self.window_opp.clear();
        self.window_opp_order.clear();
        self.window_seen.clear();
        self.window_seen_order.clear();
        self.window_start = now;
        Ok(())
    }

    /// VPIP/PFR/hands per opponent from the live model (`models.v1`), as short reads.
    fn opponent_profiles(&self) -> Result<HashMap<String, String>> {
        let Some(json) = self.store.get_kv(crate::MODELS_KEY)? else { return Ok(HashMap::new()) };
        let models: ModelStore = serde_json::from_str(&json).unwrap_or_default();
        let mut out = HashMap::new();
        for (name, st) in &models.players {
            let (Some(vpip), Some(pfr)) = (crate::style::rate(&st.vpip), crate::style::rate(&st.pfr)) else { continue };
            out.insert(
                name.clone(),
                format!("{} VPIP {:.0}/PFR {:.0}, {:.0}h", crate::style::style_of(st).0, vpip * 100.0, pfr * 100.0, st.hands),
            );
        }
        Ok(out)
    }
}

/// Bots with a completed hand within `hours`: the stall watch skips retired names (0184). Their
/// value is "now": the watch is about the absence of hands from here on.
fn active_bots(store: &Store, hours: f64, now: f64) -> Result<HashMap<String, f64>> {
    let mut out = HashMap::new();
    for (bot, ended) in store.last_hand_per_bot()? {
        let Ok(t) = DateTime::parse_from_rfc3339(&ended) else { continue };
        if now - t.timestamp() as f64 - f64::from(t.timestamp_subsec_nanos()) / 1e9 <= hours * 3600.0 {
            out.insert(bot, now);
        }
    }
    Ok(out)
}
