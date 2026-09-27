//! `ingest archive <SvanBotArchive dir> [--dry-run]` — import hands from other sources into the
//! source-tagged corpus in `artifacts/history.db`.
//! `ingest phh <dir> <source> [--dry-run]` — import a PHH tree (e.g. phh-dataset `data/pluribus`).
//! `ingest neural-ab <source> [seeds]` — held-out neural log-loss with and without that source.
//!
//! Archived observation databases from earlier versions hold the raw openpoker frames our older bots
//! received; replaying them through the live tracker yields full hands (every seat's name, stack
//! and contribution) for hands we otherwise hold only as winner-only server exports. Sources are
//! opened read-only and immutable, integrity-checked first, and imported in resumable batches.

use anyhow::Result;
use rusqlite::{Connection, OpenFlags};
use serde_json::Value;
use std::path::{Path, PathBuf};
use sv10_bot::history::{CorpusRow, HistoryDb};

const SOURCE: &str = "openpoker-archive-frames";
const BATCH_EVENTS: i64 = 50_000;

fn observation_dbs(root: &Path) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = std::fs::read_dir(root)
        .into_iter()
        .flatten()
        .filter_map(|e| e.ok())
        .map(|e| e.path().join("artifacts").join("observations.db"))
        .filter(|p| p.exists())
        .collect();
    out.sort();
    out
}

fn open_source(path: &Path) -> Result<Connection> {
    let uri = format!("file:{}?mode=ro&immutable=1", path.display());
    Ok(Connection::open_with_flags(uri, OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_URI | OpenFlags::SQLITE_OPEN_NO_MUTEX)?)
}

#[derive(Default, Debug)]
struct Tally {
    frames: usize,
    hands: usize,
    kept: usize,
    new: usize,
    known: usize,
    dropped_no_hero: usize,
    dropped_no_stacks: usize,
    /// Our net from the replay vs the server's recorded profit for the same hand.
    net_checked: usize,
    net_mismatch: usize,
}

fn import_db(db: &HistoryDb, path: &Path, dry_run: bool, tally: &mut Tally) -> Result<()> {
    let conn = open_source(path)?;
    if let Err(e) = sv10_store::integrity::check_connection(&conn) {
        eprintln!("skip {}: integrity check failed: {e}", path.display());
        return Ok(());
    }
    let slots: Vec<i64> =
        conn.prepare("SELECT DISTINCT slot FROM events ORDER BY slot")?.query_map([], |r| r.get(0))?.collect::<Result<_, _>>()?;
    for slot in slots {
        let key = format!("corpus:{SOURCE}:{}:{slot}", path.display());
        let mut after: i64 = if dry_run { 0 } else { db.meta_value(&key).and_then(|v| v.parse().ok()).unwrap_or(0) };
        // Batches bound memory and commits. A full batch resumes right after its last completed
        // hand, so a hand cut by the batch edge is replayed whole on the next pass.
        loop {
            let mut st = conn.prepare("SELECT id, data FROM events WHERE slot = ?1 AND id > ?2 ORDER BY id LIMIT ?3")?;
            let rows: Vec<(i64, String)> =
                st.query_map(rusqlite::params![slot, after, BATCH_EVENTS], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<Result<_, _>>()?;
            let Some(&(last_id, _)) = rows.last() else { break };
            let full = rows.len() as i64 == BATCH_EVENTS;
            // Live frames carry a `stream` envelope; rows rebuilt from server exports do not.
            let frames: Vec<(i64, Value)> = rows
                .into_iter()
                .filter_map(|(id, d)| serde_json::from_str::<Value>(&d).ok().map(|v| (id, v)))
                .filter(|(_, v)| v.get("stream").is_some())
                .collect();
            tally.frames += frames.len();
            let last_result_id = frames.iter().rev().find(|(_, v)| v["type"] == "hand_result").map(|(id, _)| *id);
            let mut next = if full { last_result_id.unwrap_or(last_id) } else { last_id };
            if next <= after {
                next = last_id;
            }
            let values: Vec<&Value> = frames.iter().filter(|(id, _)| *id <= next).map(|(_, v)| v).collect();
            let mut batch = Vec::new();
            for (ts, f) in sv10_venue::tracker::replay(values) {
                tally.hands += 1;
                let Some(hero) = f.hero_seat.and_then(|h| f.summary.players.iter().find(|(s, _)| *s == h)).map(|(_, n)| n.clone()) else {
                    tally.dropped_no_hero += 1;
                    continue;
                };
                if f.summary.stacks.is_empty() || f.hero_hole.is_none() {
                    tally.dropped_no_stacks += 1;
                    continue;
                }
                if let (Some(net), Some(profit)) = (f.hero_net, db.profit(&hero, &f.hand_id)) {
                    tally.net_checked += 1;
                    if net != profit {
                        tally.net_mismatch += 1;
                        if tally.net_mismatch <= 5 {
                            eprintln!("net mismatch {} {hero}: replay {net} server {profit}", f.hand_id);
                        }
                    }
                }
                tally.kept += 1;
                batch.push(CorpusRow {
                    hand_id: f.hand_id.clone(),
                    bot: hero,
                    table_id: String::new(),
                    started_at: ts.unwrap_or_default(),
                    summary: serde_json::to_string(&f.summary)?,
                });
            }
            if !dry_run {
                let (n, k) = db.insert_corpus(SOURCE, &batch, (&key, &next.to_string()))?;
                tally.new += n;
                tally.known += k;
            }
            after = next;
            if !full {
                break;
            }
        }
        eprintln!("{} slot {slot}: {tally:?}", path.display());
    }
    Ok(())
}

/// PHH files under `root`, sorted by path so imports are deterministic and resumable.
fn phh_files(root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for e in std::fs::read_dir(&dir).into_iter().flatten().flatten() {
            let p = e.path();
            if p.is_dir() {
                stack.push(p);
            } else if p.extension().is_some_and(|x| x == "phh") {
                out.push(p);
            }
        }
    }
    out.sort();
    out
}

/// Import PHH hands as `source`. Player names get a `<source>:` prefix so they can never merge
/// with an openpoker player of the same name; the hand id is `<source>:<path under root>`.
fn import_phh(db: &HistoryDb, root: &Path, source: &str, dry_run: bool) -> Result<()> {
    let files = phh_files(root);
    let key = format!("corpus:{source}:{}", root.display());
    let done: usize = if dry_run { 0 } else { db.meta_value(&key).and_then(|v| v.parse().ok()).unwrap_or(0) };
    let (mut parsed, mut rejected, mut new, mut known) = (0, 0, 0, 0);
    for (i, chunk) in files.chunks(1000).enumerate().skip(done / 1000) {
        let mut batch = Vec::new();
        for path in chunk {
            let rel = path.strip_prefix(root).unwrap_or(path).with_extension("");
            match std::fs::read_to_string(path).ok().and_then(|t| sv10_core::phh::parse(&t)) {
                Some(mut h) => {
                    parsed += 1;
                    for (_, n) in &mut h.summary.players {
                        *n = format!("{source}:{n}");
                    }
                    batch.push(CorpusRow {
                        hand_id: format!("{source}:{}", rel.display()),
                        bot: String::new(),
                        table_id: String::new(),
                        started_at: String::new(),
                        summary: serde_json::to_string(&h.summary)?,
                    });
                }
                None => {
                    rejected += 1;
                    eprintln!("rejected {}", path.display());
                }
            }
        }
        if !dry_run {
            let (n, k) = db.insert_corpus(source, &batch, (&key, &((i + 1) * 1000).min(files.len()).to_string()))?;
            new += n;
            known += k;
        }
    }
    println!(
        "phh import {source}{}: {} files, parsed {parsed}, rejected {rejected}, new {new}, already present {known}",
        if dry_run { " (dry run)" } else { "" },
        files.len()
    );
    Ok(())
}

/// `$SVANBOT10_ROOT/artifacts` (default: the working directory's), refusing imports below 2 GB free.
fn artifacts() -> Result<PathBuf> {
    let home = std::env::var("SVANBOT10_ROOT").map(PathBuf::from).unwrap_or(std::env::current_dir()?);
    let artifacts = home.join("artifacts");
    if let Some(free) = sv10_bot::tasks::free_bytes(&artifacts)
        && free < 2 * 1024 * 1024 * 1024
    {
        anyhow::bail!("refusing to import with only {} MB free", free / 1_048_576);
    }
    Ok(artifacts)
}

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let dry_run = args.iter().any(|a| a == "--dry-run");
    match args.get(1).map(String::as_str) {
        Some("archive") => {
            let root = PathBuf::from(args.get(2).ok_or_else(|| anyhow::anyhow!("usage: ingest archive <dir> [--dry-run]"))?);
            let db = HistoryDb::open(&artifacts()?.join("history.db"))?;
            let mut tally = Tally::default();
            for path in observation_dbs(&root) {
                import_db(&db, &path, dry_run, &mut tally)?;
            }
            println!("archive import{}: {tally:?}", if dry_run { " (dry run)" } else { "" });
            if !dry_run {
                let (checked, bad) = db.verify_corpus()?;
                println!("corpus verified: {checked} rows, {} digest mismatches", bad.len());
            }
            Ok(())
        }
        Some("phh") => {
            let (Some(dir), Some(source)) = (args.get(2), args.get(3)) else {
                anyhow::bail!("usage: ingest phh <dir> <source> [--dry-run]")
            };
            anyhow::ensure!(!source.starts_with("openpoker-"), "PHH sources must not use the openpoker- prefix (it feeds opponent models)");
            let db = HistoryDb::open(&artifacts()?.join("history.db"))?;
            import_phh(&db, Path::new(dir), source, dry_run)?;
            if !dry_run {
                let (checked, bad) = db.verify_corpus()?;
                println!("corpus verified: {checked} rows, {} digest mismatches", bad.len());
            }
            Ok(())
        }
        Some("neural-ab") => {
            // Run against a copy of the artifacts (SVANBOT10_ROOT): training reads the live store.
            let source = args.get(2).ok_or_else(|| anyhow::anyhow!("usage: ingest neural-ab <source> [seeds]"))?;
            let seeds: u64 = args.get(3).and_then(|s| s.parse().ok()).unwrap_or(3);
            let dir = artifacts()?;
            let store = sv10_store::store::Store::open(&dir.join("svanbot10.db"))?;
            let models: sv10_core::model::ModelStore =
                store.get_kv(sv10_bot::MODELS_KEY)?.and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default();
            let extra = HistoryDb::open(&dir.join("history.db"))?.source_summaries(source)?;
            anyhow::ensure!(!extra.is_empty(), "no corpus hands for source {source}");
            let mut diffs = Vec::new();
            for seed in 0..seeds {
                let fit = |x: &[sv10_core::model::HandSummary]| {
                    sv10_bot::neural::train_response_model(&store, &dir, &models, seed, x).map(|n| n.val_loss)
                };
                let (Some(base), Some(with)) = (fit(&[]), fit(&extra)) else { anyhow::bail!("not enough data to train") };
                println!("seed {seed}: val log-loss without {base:.5}, with {source} {with:.5} ({:+.5})", with - base);
                diffs.push(with - base);
            }
            let mean = diffs.iter().sum::<f64>() / diffs.len() as f64;
            println!("{source}: mean change {mean:+.5} nats over {seeds} seeds (negative = better)");
            Ok(())
        }
        _ => anyhow::bail!(
            "usage: ingest archive <SvanBotArchive dir> [--dry-run] | phh <dir> <source> [--dry-run] | neural-ab <source> [seeds]"
        ),
    }
}
