//! One bot's status: the last known live state, written beside its store and read by the
//! dashboard (`svanbot dashboard`, V11-11). The status is the only thing the dashboard knows
//! about a running bot; the store holds what happened, the status holds what is happening.
//!
//! A write is atomic: the document lands in a temporary file in the same directory, then the
//! rename replaces the old one, so a reader never sees half a status. The file is private to the
//! user, because it names opponents and shows their stacks.

use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use serde::{Deserialize, Serialize};

/// The status file that belongs to the store at `store`: `SurSvan.db` has
/// `SurSvan.status.json`.
pub fn status_path(store: &Path) -> PathBuf {
    store.with_extension("status.json")
}

/// One bot's last known state.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Status {
    /// The bot's name at the venue.
    pub bot: String,
    /// The process that wrote it, so a restarted bot is not mistaken for the old one.
    pub pid: u32,
    /// The build that wrote it.
    pub version: String,
    /// When the process started, in Unix milliseconds.
    pub started_ms: i64,
    /// When this document was written, in Unix milliseconds.
    pub updated_ms: i64,
    /// Whether the venue connection is open.
    pub connected: bool,
    /// The table the bot sits at, or `None` when it sits at none.
    pub table: Option<Table>,
    /// Hands in the store when the process started, plus every hand it has kept since. A hand the
    /// store already held moves `last_hand_ms` without counting again.
    pub hands: u64,
    /// Fair-play incidents in the store when the process started, plus every one raised since.
    pub incidents: u64,
    /// When the last hand finished, in Unix milliseconds, if one has.
    pub last_hand_ms: Option<i64>,
    /// The last connection failure, cleared when a connection succeeds.
    pub last_error: Option<String>,
}

/// The table a bot sits at, as the dashboard shows it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Table {
    pub id: String,
    /// Our seat, once the venue names it.
    pub seat: Option<usize>,
    /// The hand in progress, if any.
    pub hand_id: Option<String>,
    /// `preflop`, `flop`, `turn` or `river`, while a hand is in progress.
    pub street: Option<String>,
    /// The board so far, in the venue's notation (`Ah`).
    pub board: Vec<String>,
    pub pot: u32,
    pub big_blind: u32,
    pub dealer: usize,
    /// Every seated player, lowest seat first.
    pub players: Vec<Seat>,
}

/// One seated player.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Seat {
    pub seat: usize,
    pub name: String,
    pub stack: u32,
    pub folded: bool,
}

impl Status {
    /// Writes the status at `path`, replacing any earlier one. The temporary file lives in the
    /// same directory, so the rename is atomic.
    pub fn write(&self, path: &Path) -> io::Result<()> {
        static NEXT: AtomicU64 = AtomicU64::new(0);

        let parent = path.parent().unwrap_or_else(|| Path::new("."));
        let name = path.file_name().ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidInput, "the status path names no file")
        })?;
        let temp = parent.join(format!(
            ".{}.{}.{}.tmp",
            name.to_string_lossy(),
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let bytes = serde_json::to_vec(self)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;

        let mut options = OpenOptions::new();
        options.write(true).create(true).truncate(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(&temp)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            file.set_permissions(fs::Permissions::from_mode(0o600))?;
        }
        file.write_all(&bytes)?;
        file.flush()?;
        drop(file);
        if let Err(error) = fs::rename(&temp, path) {
            let _ = fs::remove_file(&temp);
            return Err(error);
        }
        Ok(())
    }

    /// Reads the status at `path`.
    pub fn read(path: &Path) -> io::Result<Status> {
        let bytes = fs::read(path)?;
        serde_json::from_slice(&bytes).map_err(|error| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!("status at {}: {error}", path.display()),
            )
        })
    }
}

/// Every bot's status in one directory, with a line for each file that could not be read.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Fleet {
    pub bots: Vec<Status>,
    pub problems: Vec<String>,
}

impl Fleet {
    /// Reads every `*.status.json` in `dir`, sorted by bot. A status filed under another bot's
    /// name is refused: two bots' states must never pool (ADR 0004).
    pub fn read(dir: &Path) -> io::Result<Fleet> {
        let mut bots = Vec::new();
        let mut problems = Vec::new();
        for entry in fs::read_dir(dir)? {
            let entry = entry?;
            if !entry.file_type()?.is_file() {
                continue;
            }
            let name = entry.file_name().to_string_lossy().into_owned();
            let Some(bot) = name.strip_suffix(".status.json") else {
                continue;
            };
            match Status::read(&entry.path()) {
                Ok(status) if status.bot == bot => bots.push(status),
                Ok(status) => {
                    problems.push(format!("{name}: it says it is {}, not {bot}", status.bot))
                }
                Err(error) => problems.push(format!("{name}: {error}")),
            }
        }
        bots.sort_by(|left, right| left.bot.cmp(&right.bot));
        problems.sort();
        Ok(Fleet { bots, problems })
    }
}
