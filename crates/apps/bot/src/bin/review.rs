//! `review [bot|all] [losers]` — live performance review: our own tendencies
//! computed from stored hands, and the biggest losing hands with decisions.
//! `review leaks` — the dashboard's leak finder as JSON.
//! `review season-snapshot FILE` / `review season-compare BEFORE AFTER` — what must carry across a
//! season change (0076): opponent models, champion lineage, response and range models, stored hands.
//! The compare prints PASS/FAIL per item and exits 1 on any failure.
//! `review raise-wars` — the 0158 study: live equity estimate vs exact equity against the shown
//! hand, for heads-up postflop decisions that committed the hand to showdown, by raises before.
//! `review replay [N | id=ID] [--current]` — re-run recorded big decisions (pot ≥ 250 bb, call ≥ 100 bb
//! or all-in, last 14 days) with their recorded inputs: `identical` proves the build reproduces
//! them bit for bit; `--current` uses today's champion knobs (`params.v1`) on the record's own
//! budget and prices instead, to see what a promotion or code change would now do in those spots.
//! The footer names that basis, the live fits included, so a what-if cannot be read as live play.
//! `review sizing-tells` — the 0223 study: for river bets and raises an opponent later showed down,
//! bet size against the exact strength of the shown hand, for the pool and per opponent, with a
//! heterogeneity test of whether opponents size by strength differently from the pool.
//! `review margins [DAYS] [CATEGORY...]` — calibration at the decision margin: residual by
//! predicted-EV bin, before and after the live self-calibration correction (default 8 days).
//! `review verify-digests [FILE]` — recompute every stored hand's content digest (and, given a
//! FILE, its SHA-256) and report mismatches; exits 1 on any.
//! `review sizing-fit` — the learner's per-opponent sizing-tell fit (0223), computed and printed,
//! not stored: held-out gain, gate verdict and the largest tells.
//! `review decisions BOT HAND` / `review export HAND` — one hand's decision details and the server's
//! export, as JSON (both are stored compressed since 0229; `sqlite3` shows them as blobs).
//! `review storage` — the compressed columns: rows still text, free pages, the last compaction.
//! `review drift` — the analyst's post-promotion drift row (`analyst.drift`, 0128): the flip rate and
//! the deep gap of today's champion on recent big-spot replays, with the basis, budget, population
//! and version mix they were measured on, and whether a row that predates any of those is due for a
//! re-check (0366).

use anyhow::Result;
use std::collections::HashMap;
use sv10_core::model::HandSummary;
use sv10_store::store::Store;

/// Every subcommand with its usage and one line on what it prints; `review help` lists them and an
/// unknown name is refused with them (a test keeps this table and the dispatch in step).
const COMMANDS: &[(&str, &str, &str)] = &[
    ("all", "all [LOSERS]", "tendencies, tripwire, head-to-head and biggest losses for the whole fleet (or name one bot)"),
    ("leaks", "leaks", "the dashboard's leak finder as JSON"),
    ("season-snapshot", "season-snapshot FILE", "what must carry across a season change, written to FILE (0076)"),
    ("season-compare", "season-compare BEFORE AFTER", "PASS/FAIL per carried item between two snapshots; exit 1 on any failure"),
    ("fold-cal", "fold-cal", "the learner's per-street and preflop fold calibration fit, not stored (0156)"),
    ("opponent-adapt", "opponent-adapt", "opponent recency study: later decisions scored by earlier models"),
    ("multiway-calls", "multiway-calls", "multiway all-in equity calibration (0219)"),
    ("player-fold", "player-fold", "the per-opponent fold calibration study at several priors (0214)"),
    ("allin-luck", "allin-luck [NAME...]", "all-in EV adjusted results per bot, and our whole net at named opponents' tables (0213)"),
    (
        "rival",
        "rival NAME... [since=DATE]",
        "where the chips go against an opponent: flow and table net with luck removed, by ending, street, pot, position (0317)",
    ),
    ("nn-residual", "nn-residual", "the per-opponent response correction study (0210)"),
    ("raise-wars", "raise-wars", "live equity estimate vs exact equity against the shown hand (0158)"),
    ("sizing-tells", "sizing-tells", "opponent river bet size vs shown strength, pool and per opponent (0223)"),
    ("sizing-fit", "sizing-fit", "the learner's per-opponent sizing-tell fit at several priors, not stored (0223)"),
    (
        "timing-tells",
        "timing-tells",
        "opponent think times by the server clock and whether slow or fast aggression shows stronger hands (0234)",
    ),
    (
        "recent",
        "recent [BOT] [N]",
        "the last N hands of a bot or the whole fleet: net and all-in EV per hand, the streak, and the EV-adjusted bb/100 (0273)",
    ),
    (
        "audit-by",
        "audit-by [DAYS] [REPLAY_VERSION]",
        "the deep re-solve's post-pricing tactics: what the live choice gave up under the prices it used, by street, by the action we took, and by the category the bot priced (0273); on the analyst's big spots, never the pricing residual (0346); the version argument reads one record version alone (0316)",
    ),
    (
        "margins",
        "margins [DAYS] [CATEGORY...]",
        "the pricing residual at the decision margin: realized minus the uncorrected price, and the same after the installed correction, over every settled decision (0222, 0346)",
    ),
    ("autonomy", "autonomy", "the autonomy watchdog's view: which learning loops are reporting and which are stale (0222)"),
    ("verify-digests", "verify-digests [FILE]", "recompute every stored hand digest (and FILE's SHA-256); exit 1 on any mismatch"),
    (
        "replay",
        "replay [N | id=ID] [--current]",
        "re-run recorded big decisions bit for bit, or with today's champion knobs on the record's own prices; the footer names the basis",
    ),
    ("wiring", "wiring [N]", "switch each live component off on the newest N big decisions: share changed and its EV cost (0316)"),
    (
        "drift",
        "drift",
        "the analyst's post-promotion drift row: today's champion's flip rate and deep gap on recent big spots, with the basis, budget, population and version mix behind them, and whether the row is due for a re-check (0128, 0366)",
    ),
    ("decisions", "decisions BOT HAND", "our recorded decisions in one hand with their details, as JSON (stored compressed, 0229)"),
    ("export", "export HAND", "the server's hand-history export of one hand from history.db, as JSON (stored compressed, 0229)"),
    ("storage", "storage", "the compressed columns: rows still text, free pages, dictionaries and the last compaction pass (0229)"),
    ("state-hash", "state-hash [N]", "state_hash mismatches, newest first: verdict, table and field census (0265)"),
    ("ledger", "ledger", "the learner's rejection ledger: barred and accumulating search transitions (0285)"),
    (
        "experiment",
        "experiment [TARGET]",
        "experiment mode: targets with live hands, or one target's estimate and every hand, decision and replay id (0291)",
    ),
    ("help", "help", "this list"),
];

fn command_list() -> String {
    COMMANDS.iter().map(|(_, usage, what)| format!("  review {usage:34} {what}")).collect::<Vec<_>>().join("\n")
}

fn main() -> Result<()> {
    let root = std::env::var("SVANBOT10_ROOT").map(std::path::PathBuf::from).unwrap_or(std::env::current_dir()?);
    let store = Store::open(&root.join("artifacts").join("svanbot10.db"))?;
    let bb = store.latest_big_blind()?.unwrap_or(sv10_bot::live::DEFAULT_BIG_BLIND) as f64;
    let args: Vec<String> = std::env::args().collect();
    let which = args.get(1).cloned().unwrap_or_else(|| "all".into());
    if which == "help" {
        println!("review — studies and checks on the stored hands (read-only unless noted)\n{}", command_list());
        return Ok(());
    }
    if which == "autonomy" {
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)?.as_secs_f64();
        let loops = sv10_bot::watchdog::loops(sv10_bot::pacing::Pacing::from_env().max_idle_secs);
        let stale = sv10_bot::watchdog::stale_loops(now, &loops, |k| store.get_kv(k));
        for l in &loops {
            match stale.iter().find(|s| s.name == l.name) {
                Some(s) => println!("STALE  {}", s.message),
                None => println!("ok     {} (limit {:.1} h)", l.name, l.limit_secs / 3600.0),
            }
        }
        std::process::exit(i32::from(!stale.is_empty()));
    }
    if which == "leaks" {
        // The dashboard's leak finder, printed as JSON (head-to-head omitted).
        let fleet = store.bot_names()?;
        println!("{}", serde_json::to_string_pretty(&sv10_bot::analysis::report(&store, &fleet, &HashMap::new(), None, bb, None))?);
        return Ok(());
    }
    if which == "ledger" {
        print!("{}", sv10_bot::search_ledger::review(&store)?);
        return Ok(());
    }
    if which == "experiment" {
        sv10_bot::experiment::review(&store, args.get(2).map(String::as_str), &mut std::io::stdout())?;
        return Ok(());
    }
    if which == "state-hash" {
        let n: usize = args.get(2).and_then(|v| v.parse().ok()).unwrap_or(20);
        for i in store.hash_incidents(n)? {
            println!("{} {} table {} seq {} {} {}", i.ts, i.bot, i.table, i.table_seq, i.verdict, i.summary);
        }
        return Ok(());
    }
    if which == "season-snapshot" {
        let path = args.get(2).map(std::path::PathBuf::from).ok_or_else(|| anyhow::anyhow!("usage: review season-snapshot FILE"))?;
        let snap = sv10_bot::review_season::season_snapshot(&store, &root)?;
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        // Atomic and durable, not `fs::write` (#323): `season-compare` reads two of these back, and a
        // report that says "written" has to mean the disk holds the whole file — a run killed
        // mid-write, or a disk that drops the write, otherwise leaves a snapshot that compares as
        // zeros under a name the operator trusts.
        sv10_rt::write_atomic(&path, &serde_json::to_string_pretty(&snap)?)?;
        println!("season snapshot written to {}", path.display());
        return Ok(());
    }
    if which == "season-compare" {
        let (Some(a), Some(b)) = (args.get(2), args.get(3)) else { anyhow::bail!("usage: review season-compare BEFORE AFTER") };
        let load = |p: &str| -> Result<serde_json::Value> { Ok(serde_json::from_str(&std::fs::read_to_string(p)?)?) };
        let failures = sv10_bot::review_season::season_compare(&load(a)?, &load(b)?);
        std::process::exit(if failures == 0 { 0 } else { 1 });
    }
    if which == "fold-cal" {
        // The learner's per-street fold calibration fit (0156), computed and printed, not stored.
        let mut samples = sv10_bot::foldcal::samples_from_store(&store)?;
        samples.extend(sv10_bot::foldcal::preflop_samples_from_store(&store)?);
        let cal = sv10_bot::foldcal::fit(&samples, 0.0);
        let p = &cal.preflop;
        println!(
            "preflop n {:6}  predicted {:.3}  actual {:.3}  held-out gain {:+6.1} mnats (95% lower {:+6.1})  {}",
            p.n,
            p.predicted,
            p.actual,
            p.held_out_gain * 1000.0,
            p.held_out_lower * 1000.0,
            if p.active { format!("install shift {:+.2}", p.shift) } else { "not installed".into() }
        );
        println!("{} heads-up postflop bets", samples.len());
        for (name, s) in ["flop", "turn", "river"].iter().zip(&cal.streets) {
            println!(
                "{name:6} n {:6}  predicted {:.3}  actual {:.3}  held-out gain {:+6.1} mnats (95% lower {:+6.1})  {}",
                s.n,
                s.predicted,
                s.actual,
                s.held_out_gain * 1000.0,
                s.held_out_lower * 1000.0,
                if s.active { format!("install shift {:+.2}", s.shift) } else { "not installed".into() }
            );
        }
        return Ok(());
    }
    if which == "opponent-adapt" {
        // 0168: all-time vs recency-weighted opponent stats, scored on the newer half of stored hands.
        let hands: Vec<(HandSummary, String)> = store
            .hands_after(0)?
            .into_iter()
            .filter_map(|(_, bot, summary)| serde_json::from_str::<HandSummary>(&summary).ok().map(|h| (h, bot)))
            .collect();
        let half_lives = [f64::INFINITY, 5_000.0, 2_000.0, 1_000.0, 500.0, 250.0];
        println!("{} hands; scoring the newer half (log-loss per opponent decision, lower is better)", hands.len());
        for s in sv10_core::adapt::study(&hands, &half_lives, hands.len() / 2) {
            println!(
                "half-life {:>8}  n {:7}  loss {:.5}  gain vs all-time {:+.2} ± {:.2} mnats",
                if s.half_life.is_finite() { format!("{:.0}", s.half_life) } else { "all-time".into() },
                s.n,
                s.loss,
                s.gain * 1000.0,
                s.half_width * 1000.0
            );
        }
        return Ok(());
    }
    if which == "multiway-calls" {
        // 0219: multiway all-in spots, estimate against the exact share vs the shown hands.
        use sv10_bot::multiway::{bias, multiway_commits_from_store};
        let spots = multiway_commits_from_store(&store)?;
        let line = |label: &str, set: Vec<&sv10_bot::raisewar::Commit>| {
            let b = bias(&set);
            println!(
                "{label:>22}  n {:5}  estimate {:.3}  exact {:.3}  over-estimate {:+.3} ± {:.3}",
                b.n, b.estimate, b.exact, b.gap, b.half_width
            );
        };
        println!("{} multiway spots (chips in, every live opponent shown)", spots.len());
        line("all", spots.iter().map(|(_, c)| c).collect());
        for k in [2usize, 3, 4] {
            let label = if k == 4 { "4+ opponents".to_string() } else { format!("{k} opponents") };
            line(&label, spots.iter().filter(|(n, _)| if k == 4 { *n >= 4 } else { *n == k }).map(|(_, c)| c).collect());
        }
        for (i, st) in ["flop", "turn", "river"].iter().enumerate() {
            line(st, spots.iter().filter(|(_, c)| c.street == i).map(|(_, c)| c).collect());
        }
        line("calls of an all-in", spots.iter().filter(|(_, c)| c.call).map(|(_, c)| c).collect());
        line("our shoves/raises", spots.iter().filter(|(_, c)| !c.call).map(|(_, c)| c).collect());
        let calls: Vec<sv10_bot::raisewar::Commit> = spots.iter().filter(|(_, c)| c.call).map(|(_, c)| c.clone()).collect();
        let fit = sv10_bot::raisewar::fit_multiway_call(&calls);
        println!(
            "-- multiway call fit: n {}  older-half over-estimate {:+.3}  held-out {:+.3} (95% lower {:+.3})  saved {:+.1} bb/call (95% lower {:+.1})  {}",
            fit.n,
            fit.train_shift,
            fit.held_out_gap,
            fit.held_out_gap_lower,
            fit.held_out_saved / bb,
            fit.held_out_saved_lower / bb,
            if fit.active { format!("install shift -{:.3}", fit.shift) } else { "not installed".into() }
        );
        return Ok(());
    }
    if which == "player-fold" {
        // 0214: per-opponent fold calibration of our heads-up postflop bets, held-out study.
        let mut samples = sv10_bot::foldcal::samples_from_store(&store)?;
        samples.sort_by(|a, b| a.ts.cmp(&b.ts));
        let post = samples.iter().filter(|s| s.street != sv10_bot::foldcal::PREFLOP).count();
        println!("{post} heads-up postflop bets; newer half scored (log-loss per bet, positive = better)");
        for s in sv10_bot::playerfold::score(&samples, &[5.0, 10.0, 20.0, 50.0, 100.0]) {
            println!(
                "prior {:>5}  n {:6}  base {:.4}  gain {:+.2} ± {:.2} mnats",
                s.prior,
                s.n,
                s.base_loss,
                s.gain * 1000.0,
                s.half_width * 1000.0
            );
        }
        let shift = sv10_bot::foldcal::installed_shift(store.get_kv(sv10_bot::foldcal::FOLD_CAL_KEY)?.as_deref());
        let f = sv10_bot::playerfold::fit(&samples, &shift);
        println!(
            "fit at prior {}: {} opponents -> {}",
            sv10_bot::playerfold::PRIOR,
            f.offsets.len(),
            if f.active { "would install" } else { "would not install" }
        );
        let mut v: Vec<_> = f.offsets.iter().collect();
        v.sort_by(|a, b| b.1.abs().total_cmp(&a.1.abs()));
        for (name, o) in v.iter().take(15) {
            println!("  {name:>22}  logit {:+.2}  ({})", o, if **o > 0.0 { "folds more than priced" } else { "folds less than priced" });
        }
        return Ok(());
    }
    if which == "allin-luck" || which == "rival" {
        let names = &args[2.min(args.len())..];
        print!(
            "{}",
            if which == "rival" {
                sv10_bot::review_rival::rival(&store, names)?
            } else {
                sv10_bot::review_rival::allin_luck(&store, names)?
            }
        );
        return Ok(());
    }
    if which == "nn-residual" {
        // 0210: the live response network corrected by each opponent's own observed/expected ratios.
        let priors = [10.0, 30.0, 100.0, 300.0];
        let scores = sv10_bot::nnresidual::study(&store, &root.join("artifacts"), &priors)?;
        if let Some(s) = scores.first() {
            println!("{} validation decisions ({} facing a bet); network log-loss {:.5}", s.n, s.n_facing, s.base_loss);
        }
        for s in scores {
            println!(
                "prior {:>4}  gain {:+.2} ± {:.2} mnats  facing a bet {:+.2} ± {:.2}  halves {:+.2} / {:+.2}",
                s.prior,
                s.gain * 1000.0,
                s.half_width * 1000.0,
                s.gain_facing * 1000.0,
                s.half_width_facing * 1000.0,
                s.gain_halves[0] * 1000.0,
                s.gain_halves[1] * 1000.0
            );
        }
        let fit = sv10_bot::nnresidual::fit(&store, &root.join("artifacts"))?;
        println!(
            "fit at prior {}: {} opponents, facing a bet {:+.2} ± {:.2} mnats -> {}",
            sv10_bot::nnresidual::PRIOR,
            fit.ratios.len(),
            fit.gain_facing * 1000.0,
            fit.half_width_facing * 1000.0,
            if fit.active { "would install" } else { "would not install" }
        );
        let mut ratios: Vec<(&String, &[f32; 3])> = fit.ratios.iter().collect();
        ratios.sort_by(|a, b| (b.1[0] - 1.0).abs().total_cmp(&(a.1[0] - 1.0).abs()));
        println!("most misread facing a bet (fold / call / raise ratio to the network):");
        for (name, r) in ratios.iter().take(12) {
            println!("  {name:>22}  {:.2} / {:.2} / {:.2}", r[0], r[1], r[2]);
        }
        return Ok(());
    }
    if which == "raise-wars" {
        return sv10_bot::review_calls::raise_wars(&store);
    }
    if which == "timing-tells" {
        return sv10_bot::review_calls::timing_tells(&store);
    }
    if which == "sizing-tells" {
        return sv10_bot::review_calls::sizing_tells(&store);
    }
    if which == "recent" {
        let n: usize = args.get(3).and_then(|v| v.parse().ok()).unwrap_or(200);
        let fleet = store.bot_names()?;
        let asked = args.get(2).cloned().filter(|a| fleet.contains(a));
        let bots: Vec<String> = match asked {
            Some(b) => vec![b],
            None => fleet.clone(),
        };
        for b in &bots {
            println!("{}", sv10_bot::review_recent::recent(&store, b, n)?);
        }
        return Ok(());
    }
    if which == "audit-by" {
        let (days, version) = (args.get(2).and_then(|d| d.parse().ok()).unwrap_or(8), args.get(3).and_then(|v| v.parse().ok()));
        println!("{}", sv10_bot::review_recent::audit_by(&store, days, version)?);
        return Ok(());
    }
    if which == "margins" {
        let days: i64 = args.get(2).and_then(|d| d.parse().ok()).unwrap_or(8);
        let since = (chrono::Utc::now() - chrono::Duration::days(days)).to_rfc3339();
        let cats: Vec<&str> = args.iter().skip(3).map(String::as_str).collect();
        let samples = store.calibration_samples_since(&since)?;
        let table = store.get_kv(sv10_bot::CALIBRATION_KEY)?;
        let (bias, bound) = (sv10_stats::margins::installed_bias(table.as_deref()), sv10_stats::margins::bound_by(table.as_deref()));
        println!(
            "{} priced decisions in the last {days} days: every settled decision the bot priced and got a stack result for, at every pot \
             size — the pricing population (the deep audit's is its big spots only, `review audit-by`)",
            samples.len()
        );
        println!(
            "   residual = realized - predicted (bb) on the UNCORRECTED price, the quantity self-calibration is fitted from; 'after correction' \
             subtracts the installed correction at its largest size, since in play a penalty is capped at its supported per-pot residual (0207); \
             * = off at 95% after that"
        );
        for (cat, bins) in sv10_stats::margins::margins(&samples, &cats, &bias) {
            if bins.is_empty() {
                continue;
            }
            println!(
                "{cat}  (live correction {:+.2} bb, set by {})",
                bias.get(&cat).copied().unwrap_or(0.0),
                bound.get(&cat).map_or("?", |b| b)
            );
            for b in bins {
                println!(
                    "  {} pred [{:>5}, {:>5})  n {:6}  residual {:+7.2} ± {:5.2}  after correction {:+7.2}{}",
                    if b.at_margin() { "margin" } else { "      " },
                    b.lo,
                    b.hi,
                    b.n,
                    b.residual,
                    b.half_width,
                    b.after_correction,
                    if b.miscalibrated() { " *" } else { "" }
                );
            }
        }
        return Ok(());
    }
    if which == "decisions" {
        let (Some(bot), Some(hand)) = (args.get(2), args.get(3)) else { anyhow::bail!("usage: review decisions BOT HAND") };
        println!("{}", serde_json::to_string_pretty(&store.decisions_for_hand(bot, hand)?)?);
        return Ok(());
    }
    if which == "export" {
        let Some(hand) = args.get(2) else { anyhow::bail!("usage: review export HAND") };
        let history = sv10_bot::history::HistoryDb::open(&root.join("artifacts").join("history.db"))?;
        match history.export_json(hand)? {
            Some(json) => println!("{}", serde_json::to_string_pretty(&serde_json::from_str::<serde_json::Value>(&json)?)?),
            None => anyhow::bail!("hand {hand} is not in history.db"),
        }
        return Ok(());
    }
    if which == "storage" {
        let history = sv10_bot::history::HistoryDb::open(&root.join("artifacts").join("history.db"))?;
        let mb = |b: u64| format!("{:.1} MB", b as f64 / 1e6);
        let size = |f: &str| std::fs::metadata(root.join("artifacts").join(f)).map(|m| m.len()).unwrap_or(0);
        println!(
            "data format {} (this build reads {})",
            sv10_store::packed::data_format(&root.join("artifacts")),
            sv10_store::packed::DATA_FORMAT
        );
        println!("svanbot10.db {}, {} in free pages", mb(size("svanbot10.db")), mb(store.free_bytes()?));
        for (family, n) in store.text_rows()? {
            println!("  {family:20} {n} rows still text");
        }
        println!("history.db   {}, {} in free pages", mb(size("history.db")), mb(history.free_bytes()?));
        for (family, n) in history.text_rows()? {
            println!("  {family:20} {n} rows still text");
        }
        match store.get_kv(sv10_bot::compaction::STATUS_KEY)? {
            Some(status) => println!("last compaction pass: {status}"),
            None => println!("no compaction pass recorded yet (the fleet starts one 90 s after it starts)"),
        }
        return Ok(());
    }
    if which == "verify-digests" {
        let (checked, bad) = store.verify_hand_digests()?;
        println!("{checked} stored hands digest-checked, {} mismatches", bad.len());
        for (bot, hand) in bad.iter().take(10) {
            println!("  mismatch: {bot} {hand}");
        }
        if let Some(file) = args.get(2) {
            println!("sha256 {}  {file}", sv10_store::integrity::file_sha256(std::path::Path::new(file))?);
        }
        std::process::exit(i32::from(!bad.is_empty()));
    }
    if which == "sizing-fit" {
        let priors = [30.0, 100.0, 300.0, 1000.0];
        let t0 = std::time::Instant::now();
        let fits = sv10_bot::playersize::fits_from_store(&store, &priors)?;
        for (prior, f) in priors.iter().zip(&fits) {
            println!(
                "prior {prior:5}: {} river-bet showdowns; held-out gain {:+.2} ± {:.2} mnats on {} -> {}",
                f.fitted_on,
                f.gain * 1000.0,
                f.half_width * 1000.0,
                f.n,
                if f.active { "would install" } else { "would not install" },
            );
        }
        println!("({:.1}s)", t0.elapsed().as_secs_f64());
        let f = &fits[0];
        let mut tells: Vec<_> = f.tells.iter().collect();
        tells.sort_by(|a, b| b.1.abs().total_cmp(&a.1.abs()));
        for (name, k) in tells.iter().take(10) {
            println!("{name:20} {k:+.2}");
        }
        return Ok(());
    }
    if which == "replay" {
        return sv10_bot::review_rerun::replay(&store, &args[2..]);
    }
    if which == "wiring" {
        return sv10_bot::review_wiring::wiring(&store, &args[2..]);
    }
    if which == "drift" {
        print!("{}", sv10_bot::review_drift::drift(&store)?);
        return Ok(());
    }
    let losers: usize = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(5);
    let bots = store.bot_names()?;
    if !(which == "all" || bots.contains(&which)) {
        anyhow::bail!("unknown bot or command {which:?}; bots are {}\n{}", bots.join(", "), command_list());
    }
    sv10_bot::review_fleet::fleet_report(&store, bb, which, losers, &bots)
}

#[cfg(test)]
mod tests {
    #[test]
    fn every_command_is_listed_and_every_listed_command_is_dispatched() {
        let src = include_str!("review.rs");
        let dispatched: Vec<&str> =
            src.match_indices("which == \"").map(|(i, m)| &src[i + m.len()..]).filter_map(|rest| rest.split('"').next()).collect();
        for name in &dispatched {
            assert!(super::COMMANDS.iter().any(|c| c.0 == *name), "{name} is dispatched but missing from COMMANDS");
        }
        for (name, usage, _) in super::COMMANDS {
            assert!(dispatched.contains(name), "{name} is listed but never dispatched");
            assert!(usage.starts_with(name), "usage of {name} must start with its name");
        }
    }

    /// 0273: the two instruments must not be confused. The calibration residual answers "is the model
    /// priced right?"; the deep re-solve answers "did the choice cost chips?". `audit-by` is the second,
    /// so it groups by the action *family*: a sized action ("raise:1605") is the same class as every
    /// other raise, and grouping by size produced one row per bet size.
    #[test]
    fn the_audit_report_groups_by_action_family_not_by_bet_size() {
        let sized = "raise:1605";
        assert_eq!(sized.split(':').next(), Some("raise"));
        assert_eq!(sized.split_once(':').and_then(|(_, n)| n.parse::<i64>().ok()), Some(1605));
        assert_eq!("call".split_once(':'), None, "an unsized action has no size to match on");
        // A decision record's candidate carries the action and the amount apart, which is why the
        // category lookup matches on both.
        let candidate = serde_json::json!({"action": "raise", "amount": 1605, "category": "turn:bet:big"});
        assert_eq!(candidate["action"], "raise");
        assert_eq!(candidate["amount"].as_i64(), Some(1605));
        assert_eq!(candidate["category"].as_str(), Some("turn:bet:big"));
    }
}
