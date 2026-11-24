//! What a bot and its updater tell each other about health, through two files per bot.
//!
//! `starts`: the bot appends the time and its build each time it comes up. The updater asks how
//! many of those fall inside a window, which is how a crash loop is seen from outside the process:
//! the supervisor restarts the bot, the bot records the start, and three starts inside the window mean the
//! build is not surviving. The log is pruned by its writer, so a season of restarts does not leave
//! a growing file behind, and a line torn by a crash is skipped by its reader.
//!
//! `update`: the updater's request that the bot stop between hands. Existence is the signal. The
//! bot only ever reads it; the updater lifts it once the swap is done, so a bot that comes back up
//! into a request still standing does not stop again at once.
//!
//! Both live beside the bot's store, one set per bot, because a read does not cross bots
//! (`docs/adr/0004-reads-are-per-bot.md`).

use std::fmt;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

/// The window a crash loop is judged over, and the one the updater waits out before deciding.
pub const WINDOW_MS: i64 = 5 * 60 * 1_000;

/// Where a bot's health files live, derived from its store: the `health` directory beside it.
///
/// The bot and its updater both derive the directory this way, so neither has to be told where the
/// other writes and the two can never disagree about where to look.
pub fn beside_store(store: impl AsRef<Path>) -> PathBuf {
    match store.as_ref().parent() {
        Some(dir) => dir.join("health"),
        None => PathBuf::from("health"),
    }
}

/// How many starts the log keeps. Far more than the limit any caller asks about, and small enough
/// that reading it is cheap.
const KEEP: usize = 32;

/// One time the bot came up.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Start {
    /// When it came up, in milliseconds since the Unix epoch.
    pub at_ms: i64,
    /// The build it came up as, the version-named directory `current` pointed at.
    pub version: String,
}

/// Why a health file could not be read or written.
#[derive(Debug)]
pub struct HealthError(io::Error);

impl fmt::Display for HealthError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "health: {}", self.0)
    }
}

impl std::error::Error for HealthError {}

impl From<io::Error> for HealthError {
    fn from(error: io::Error) -> Self {
        Self(error)
    }
}

/// One bot's health files.
#[derive(Clone, Debug)]
pub struct Health {
    dir: PathBuf,
    me: String,
}

impl Health {
    /// The health files for `me` under `dir`, whether or not `dir` exists yet.
    ///
    /// One file per bot, named for the bot, so two bots sharing a directory never read each
    /// other's.
    pub fn in_dir(dir: impl Into<PathBuf>, me: &str) -> Self {
        Self {
            dir: dir.into(),
            me: me.to_owned(),
        }
    }

    /// Records that the bot came up, as `version`, at `at_ms`.
    ///
    /// Appending one line is the whole write, so a start that is killed mid-line leaves the
    /// previous line readable and the torn one skippable.
    pub fn note_start(&self, at_ms: i64, version: &str) -> Result<(), HealthError> {
        fs::create_dir_all(&self.dir)?;
        let path = self.starts_path();
        let mut log = fs::read_to_string(&path).unwrap_or_default();
        log.push_str(&format!("{at_ms}\t{version}\n"));
        // Keep the newest lines: a crash loop is judged on the recent past, and the file must not
        // grow for as long as the box runs.
        let tail: Vec<&str> = log.lines().rev().take(KEEP).collect();
        let kept: String = tail
            .into_iter()
            .rev()
            .fold(String::new(), |mut kept, line| {
                kept.push_str(line);
                kept.push('\n');
                kept
            });
        write_atomically(&path, kept.as_bytes())
    }

    /// Every start inside `window` of `now_ms`, oldest first.
    pub fn starts(&self, now_ms: i64, window: i64) -> Result<Vec<Start>, HealthError> {
        let Ok(log) = fs::read_to_string(self.starts_path()) else {
            // No log is a bot that has never started: not an error, and not a crash loop.
            return Ok(Vec::new());
        };
        let oldest = now_ms.saturating_sub(window);
        Ok(log
            .lines()
            .filter_map(parse_start)
            .filter(|start| start.at_ms >= oldest && start.at_ms <= now_ms)
            .collect())
    }

    /// Whether the bot has come up `limit` times or more inside `window` of `now_ms`.
    pub fn in_crash_loop(
        &self,
        now_ms: i64,
        window: i64,
        limit: usize,
    ) -> Result<bool, HealthError> {
        Ok(self.starts(now_ms, window)?.len() >= limit)
    }

    /// Asks the bot to stop between hands.
    pub fn request_update(&self) -> Result<(), HealthError> {
        fs::create_dir_all(&self.dir)?;
        write_atomically(&self.update_path(), b"")
    }

    /// Whether an update has been asked for and not yet cleared.
    pub fn update_requested(&self) -> bool {
        self.update_path().exists()
    }

    /// Lifts the request, once the swap is done.
    pub fn clear_update(&self) -> Result<(), HealthError> {
        remove_if_there(self.update_path())
    }

    /// Says the bot has left the table and is ending, because it saw the update request.
    ///
    /// The bot claims nothing about the swap: it says only that the connection is gone, which is
    /// the one thing the updater cannot see for itself and the one thing it must know before the
    /// build under `current` changes.
    pub fn note_stop(&self) -> Result<(), HealthError> {
        fs::create_dir_all(&self.dir)?;
        Ok(fs::write(self.stopped_path(), "")?)
    }

    /// Whether the bot has said it stopped.
    pub fn stopped(&self) -> bool {
        self.stopped_path().exists()
    }

    /// Clears the bot's stop, before a new request.
    pub fn clear_stop(&self) -> Result<(), HealthError> {
        remove_if_there(self.stopped_path())
    }

    /// The start log's path.
    pub fn starts_path(&self) -> PathBuf {
        self.dir.join(format!("{}.starts", self.me))
    }

    /// The update request's path.
    pub fn update_path(&self) -> PathBuf {
        self.dir.join(format!("{}.update", self.me))
    }

    /// The stop's path.
    pub fn stopped_path(&self) -> PathBuf {
        self.dir.join(format!("{}.stopped", self.me))
    }
}

/// Removes `path`, treating one that is already gone as success.
fn remove_if_there(path: PathBuf) -> Result<(), HealthError> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
    }
}

/// `at_ms\tversion`, or `None` for a line a crash left torn, which is the only way one is written
/// that is not whole.
fn parse_start(line: &str) -> Option<Start> {
    let (at, version) = line.split_once('\t')?;
    Some(Start {
        at_ms: at.parse().ok()?,
        version: version.to_owned(),
    })
}

/// Writes `bytes` to `path` by writing beside it and renaming over it, so a reader sees the old
/// file or the new one, never a half-written one.
pub(crate) fn write_atomically(path: &Path, bytes: &[u8]) -> Result<(), HealthError> {
    let temporary = path.with_extension("new");
    {
        let mut file = fs::File::create(&temporary)?;
        file.write_all(bytes)?;
        file.sync_all()?;
    }
    match fs::rename(&temporary, path) {
        Ok(()) => Ok(()),
        Err(error) => {
            let _ = fs::remove_file(&temporary);
            Err(error.into())
        }
    }
}
