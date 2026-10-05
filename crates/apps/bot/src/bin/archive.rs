//! `archive` — long-term archives (`SVANBOT_ARCHIVE_DIR`, default `artifacts/archive`; set it to a
//! second disk, here `/backup-disk/svanbot10`); see `sv10_store::archive` for the layout and guarantees.
//!
//! - `archive run` — write whatever is due (weekly full, else today's daily; the month's archive),
//!   then prune (14 daily, 8 weekly, 12 monthly). Skips with exit 2 when the disk lacks room.
//! - `archive list` — every archive with its size and creation time.
//! - `archive verify [NAME] [--deep]` — hashes (and decompressed content with `--deep`); all
//!   archives when no name is given. Exit 1 on any problem.
//! - `archive restore NAME --to DIR` — rebuild `svanbot10.db` and `history.db` (and `repo.bundle`,
//!   for archives written before the data-only rule, #772) in DIR, verified. Never writes into `artifacts/`: stop the fleet and copy the
//!   restored files in by hand (docs/OPERATIONS.md, "Restore from the archive").
//! - `archive export-derived --to DIR` — write the public derived set (`schema.sql`,
//!   `aggregates.json`, `SHA256SUMS`) for a dated data release: counts, summaries and the table
//!   definitions, scrubbed of keys, emails and home paths, failing closed on any hit. Never raw
//!   opponent hands (#17).
//!
//! The databases' compressed columns (0229; `--dir DIR` works on a copy instead of `artifacts/`):
//!
//! - `archive compact [--dir DIR]` — pack every stored row now (the fleet does this in the
//!   background) and VACUUM `history.db`; with `--vacuum-main` also the main database, which
//!   needs the fleet stopped. Prints the sizes before and after.
//! - `archive unpack [--dir DIR]` — convert every packed value back to text and record data
//!   format 1, so a build from before 0229 can be installed (`scripts/rollback.sh` refuses it until
//!   then). Needs the fleet stopped.

use anyhow::{Result, bail};
use std::path::PathBuf;
use sv10_store::archive::{self, Kind, Retention, Sources};

fn main() -> Result<()> {
    sv10_bot::init_tool_logging();
    let root = std::env::var("SVANBOT10_ROOT").map(PathBuf::from).unwrap_or(std::env::current_dir()?);
    let _ = sv10_rt::load_env_file(&root.join(".env"));
    let dir = std::env::var("SVANBOT_ARCHIVE_DIR").map(PathBuf::from).unwrap_or_else(|_| root.join("artifacts").join("archive"));
    let args: Vec<String> = std::env::args().skip(1).collect();
    let all = || [Kind::Weekly, Kind::Daily, Kind::Monthly].into_iter().flat_map(|k| archive::list(&dir, k)).collect::<Vec<_>>();
    match args.first().map(String::as_str) {
        Some("run") => {
            let src = Sources {
                live: root.join("artifacts/svanbot10.db"),
                history: root.join("artifacts/history.db"),
                repo: root.join(".git").exists().then(|| root.clone()),
                app_version: env!("CARGO_PKG_VERSION").to_string(),
            };
            std::fs::create_dir_all(&dir)?;
            let need = archive::space_needed(&src);
            if let Some(free) = sv10_rt::free_bytes(&dir)
                && free < need
            {
                tracing::error!("{} has {} MB free, an archive run needs {} MB; skipping", dir.display(), free >> 20, need >> 20);
                std::process::exit(2);
            }
            let (made, removed) = archive::run(&dir, &src, chrono::Utc::now(), Retention::default())?;
            for m in &made {
                let size: u64 = archive::load_manifest(&dir.join(m))?.files.iter().map(|f| f.bytes).sum();
                tracing::info!("wrote {m} ({} MB, verified)", size >> 20);
            }
            for r in &removed {
                tracing::info!("pruned {r}");
            }
            if made.is_empty() {
                tracing::info!("nothing due");
            }
        }
        Some("list") => {
            for name in all() {
                match archive::load_manifest(&dir.join(&name)) {
                    Ok(m) => {
                        let (bytes, raw): (u64, u64) = m.files.iter().fold((0, 0), |(a, b), f| (a + f.bytes, b + f.raw_bytes));
                        println!(
                            "{name:<22} {:>7} MB  (raw {:>6} MB)  {}  {}",
                            bytes >> 20,
                            raw >> 20,
                            sv10_bot::local_time(&m.created_at),
                            m.git_commit.as_deref().map(|c| &c[..c.len().min(8)]).unwrap_or("-")
                        );
                    }
                    Err(e) => println!("{name:<22} UNREADABLE: {e}"),
                }
            }
        }
        Some("verify") => {
            let deep = args.iter().any(|a| a == "--deep");
            let names: Vec<String> = match args.iter().skip(1).find(|a| !a.starts_with("--")) {
                Some(n) => vec![n.clone()],
                None => all(),
            };
            let mut bad = 0;
            for name in &names {
                let problems = archive::verify(&dir.join(name), deep);
                if problems.is_empty() {
                    println!("ok       {name}");
                } else {
                    bad += 1;
                    println!("PROBLEM  {name}: {}", problems.join("; "));
                }
            }
            if bad > 0 {
                std::process::exit(1);
            }
        }
        Some("export-derived") => {
            let Some(to) = args.iter().position(|a| a == "--to").and_then(|i| args.get(i + 1)).map(PathBuf::from) else {
                bail!("usage: archive export-derived --to DIR")
            };
            let to = if to.is_absolute() { to } else { std::env::current_dir()?.join(to) };
            // Secrets are compared, never printed: a scrub hit names the violation class, not the value.
            let dotenv = std::fs::read_to_string(root.join(".env")).unwrap_or_default();
            let secrets = sv10_bot::derived::SecretSet::from_dotenv(&dotenv);
            let home = std::env::var("HOME").unwrap_or_default();
            for p in sv10_bot::derived::export_derived(
                &root.join("artifacts/svanbot10.db"),
                &root.join("artifacts/history.db"),
                &to,
                &secrets,
                &home,
            )? {
                println!("exported {}", p.display());
            }
        }
        Some("restore") => {
            let Some(name) = args.get(1) else { bail!("usage: archive restore NAME --to DIR") };
            let Some(to) = args.iter().position(|a| a == "--to").and_then(|i| args.get(i + 1)).map(PathBuf::from) else {
                bail!("usage: archive restore NAME --to DIR")
            };
            let to = if to.is_absolute() { to } else { std::env::current_dir()?.join(to) };
            // Compared as resolved paths: `..` segments or a symlink walked past a lexical check.
            // The target need not exist yet, so its nearest existing ancestor is what is resolved.
            let resolve =
                |p: &std::path::Path| p.ancestors().find_map(|a| a.canonicalize().ok().map(|c| c.join(p.strip_prefix(a).unwrap_or(p))));
            let (target, live) = (resolve(&to).unwrap_or_else(|| to.clone()), root.join("artifacts"));
            if target.starts_with(resolve(&live).unwrap_or(live)) {
                bail!("refusing to restore into artifacts/: restore elsewhere, stop the fleet, then copy the files in");
            }
            for p in archive::restore(&dir, name, &to)? {
                println!("restored {}", p.display());
            }
        }
        Some(cmd @ ("compact" | "unpack")) => {
            let data = args
                .iter()
                .position(|a| a == "--dir")
                .and_then(|i| args.get(i + 1))
                .map(PathBuf::from)
                .unwrap_or_else(|| root.join("artifacts"));
            let vacuum_main = args.iter().any(|a| a == "--vacuum-main");
            if (cmd == "unpack" || vacuum_main)
                && let Some(pid) = fleet_running(&data)
            {
                bail!("the fleet is running (pid {pid}): stop it first (scripts/stop.sh)");
            }
            let size = |f: &str| std::fs::metadata(data.join(f)).map(|m| m.len() >> 20).unwrap_or(0);
            let before = (size("svanbot10.db"), size("history.db"));
            let store = sv10_store::store::Store::open(&data.join("svanbot10.db"))?;
            let history = sv10_bot::history::HistoryDb::open(&data.join("history.db"))?;
            let started = std::time::Instant::now();
            if cmd == "unpack" {
                let n = store.unpack_all()? + history.unpack_all()?;
                sv10_store::packed::mark_data_format(&data, 1, true)?;
                println!("unpacked {n} values to text; data format 1 recorded (a build before 0229 can be installed)");
            } else {
                let mut cursors = sv10_bot::compaction::Cursors::start();
                let mut packed = 0;
                while !cursors.done() {
                    packed += sv10_bot::compaction::step(&store, Some(&history), &mut cursors, 2_000)?;
                }
                println!("packed {packed} values in {:.1} s", started.elapsed().as_secs_f64());
                let (a, b) = history.vacuum(&data.join("history.db"))?;
                println!("history.db VACUUM: {} MB -> {} MB", a >> 20, b >> 20);
                if vacuum_main {
                    let (a, b) = store.vacuum()?;
                    println!("svanbot10.db VACUUM: {} MB -> {} MB", a >> 20, b >> 20);
                }
            }
            println!(
                "svanbot10.db {} MB -> {} MB, history.db {} MB -> {} MB ({:.1} s)",
                before.0,
                size("svanbot10.db"),
                before.1,
                size("history.db"),
                started.elapsed().as_secs_f64()
            );
        }
        _ => bail!(
            "usage: archive run | list | verify [NAME] [--deep] | restore NAME --to DIR | export-derived --to DIR | compact [--dir DIR] [--vacuum-main] | unpack [--dir DIR]"
        ),
    }
    Ok(())
}

/// A live fleet process whose pid file is in `artifacts` (the databases' directory).
fn fleet_running(artifacts: &std::path::Path) -> Option<u32> {
    let files = std::fs::read_dir(artifacts).ok()?;
    files.filter_map(|e| e.ok()).map(|e| e.path()).find_map(|p| {
        let name = p.file_name()?.to_str()?;
        if !(name == "bot.pid" || name == "head.pid" || (name.starts_with("worker-") && name.ends_with(".pid"))) {
            return None;
        }
        let pid: u32 = std::fs::read_to_string(&p).ok()?.trim().parse().ok()?;
        std::path::Path::new(&format!("/proc/{pid}")).exists().then_some(pid)
    })
}
