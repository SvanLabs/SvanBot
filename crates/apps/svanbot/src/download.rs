//! `svanbot download`: the bot's current season hand history into its store (V11-13), then exit.
//! `--all-seasons` also imports completed seasons the bot played.
//!
//! It reads `SVANBOT_API_KEY`, `SVANBOT_NAME` and `SVANBOT_STORE` like the bot, and
//! `SVANBOT_API`, the REST base, `https://api.openpoker.ai/api` unless set. Run it once per bot,
//! with that bot's own key. A lock file beside the store lets only one download run at a time,
//! so the bots download one after another.

use std::fs::{File, TryLockError};
use std::process::ExitCode;

use svanbot_runtime::download::{MAX_BODY, Summary, TIMEOUT, download, download_all, limiter};
use svanbot_runtime::rest::{API, Rest};

use crate::{now_ms, open_store, store_path};

pub(crate) fn run(args: &[String]) -> ExitCode {
    let all = match args {
        [] => false,
        [flag] if flag == "--all-seasons" => true,
        _ => {
            eprintln!("usage: svanbot download [--all-seasons]");
            return ExitCode::from(2);
        }
    };
    let need = |name: &str| std::env::var(name).map_err(|_| format!("set {name}"));
    let (key, me) = match (need("SVANBOT_API_KEY"), need("SVANBOT_NAME")) {
        (Ok(key), Ok(me)) => (key, me),
        (Err(why), _) | (_, Err(why)) => {
            eprintln!("svanbot download: {why}");
            return ExitCode::from(2);
        }
    };
    let store = match open_store(&me) {
        Ok(store) => store,
        Err(why) => {
            eprintln!("svanbot download: {why}");
            return ExitCode::from(2);
        }
    };
    // `open_store` made the directory, so the lock file can go beside the store.
    let lock_path = match store_path(&me) {
        Ok(path) => path.with_file_name("download.lock"),
        Err(why) => {
            eprintln!("svanbot download: {why}");
            return ExitCode::from(2);
        }
    };
    let lock = match File::create(&lock_path) {
        Ok(file) => file,
        Err(error) => {
            eprintln!(
                "svanbot download: cannot open {}: {error}",
                lock_path.display()
            );
            return ExitCode::from(2);
        }
    };
    match lock.try_lock() {
        Ok(()) => {}
        Err(TryLockError::WouldBlock) => {
            eprintln!("svanbot download: another bot's download is running; one at a time");
            return ExitCode::from(3);
        }
        Err(TryLockError::Error(error)) => {
            eprintln!(
                "svanbot download: cannot lock {}: {error}",
                lock_path.display()
            );
            return ExitCode::from(2);
        }
    }
    let base = std::env::var("SVANBOT_API").unwrap_or_else(|_| API.to_owned());
    let mut rest = Rest::new(&base, &key, limiter()).with_large_replies(TIMEOUT, MAX_BODY);
    if all {
        eprintln!("svanbot download: {me}: current and past seasons");
        return match download_all(&mut rest, &store, now_ms, |season, result| {
            let label = season.map_or_else(
                || "current season".into(),
                |number| format!("season {number}"),
            );
            match result {
                Ok(Some(summary)) => say(&me, &label, summary),
                Ok(None) => eprintln!("svanbot download: {me}: {label} already verified"),
                Err(error) => eprintln!("svanbot download: {me}: {label}: {error}"),
            }
        }) {
            Ok(()) => ExitCode::SUCCESS,
            Err(error) => {
                eprintln!("svanbot download: {me}: {error}; rerun to retry");
                ExitCode::from(1)
            }
        };
    }

    eprintln!("svanbot download: {me}: this season's export, newest hand first");
    match download(&mut rest, &store, now_ms) {
        Ok(summary) => {
            say(&me, "current season", summary);
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("svanbot download: {me}: stopped: {error}; rerun to read it again");
            ExitCode::from(1)
        }
    }
}

fn say(me: &str, season: &str, summary: Summary) {
    eprintln!(
        "svanbot download: {me}: {season}: {} pages, {} new hands, {} already kept, {} without an id; next offset {}",
        summary.pages, summary.added, summary.known, summary.skipped, summary.offset
    );
}
