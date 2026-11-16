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

use serde_json::{Value, json};

/// The status file that belongs to the store at `store`: `SurSvan.db` has
/// `SurSvan.status.json`.
pub fn status_path(store: &Path) -> PathBuf {
    store.with_extension("status.json")
}

/// One bot's last known state.
#[derive(Clone, Debug, PartialEq, Eq)]
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
    /// The incidents this process has raised. The dashboard raises these: an incident from a
    /// past run is on record, and is not a reason to stay red forever.
    pub run_incidents: u64,
    /// When the last hand finished, in Unix milliseconds, if one has.
    pub last_hand_ms: Option<i64>,
    /// The last connection failure, cleared when a connection succeeds.
    pub last_error: Option<String>,
    /// What the bot plays by and what its hands have won.
    pub play: Play,
}

/// What a bot plays by and what its ledger holds, as the dashboard shows it. Winnings are in
/// thousandths of a big blind, with the luck of the bot's own draws taken out.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Play {
    /// The digest of the champion parameter set.
    pub champion: String,
    /// The running probe in one line, on the canary.
    pub probe: Option<String>,
    /// The hands the ledger has counted.
    pub counted: u64,
    /// What they won in all.
    pub won_mbb: i64,
    /// What each of the latest counted hands won, oldest first, [`RECENT`] at most.
    pub recent: Vec<i64>,
}

/// How many of the latest hands a status carries one by one.
pub const RECENT: usize = 120;

impl Play {
    /// Counts one more hand that won `mbb`.
    pub fn count(&mut self, mbb: i64) {
        self.counted += 1;
        self.won_mbb += mbb;
        self.recent.push(mbb);
        if self.recent.len() > RECENT {
            self.recent.remove(0);
        }
    }
}

/// The table a bot sits at, as the dashboard shows it.
#[derive(Clone, Debug, PartialEq, Eq)]
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
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Seat {
    pub seat: usize,
    pub name: String,
    pub stack: u32,
    pub folded: bool,
}

impl Status {
    /// The status as the JSON the file holds and the dashboard sends.
    pub fn to_json(&self) -> Value {
        json!({
            "bot": self.bot,
            "pid": self.pid,
            "version": self.version,
            "started_ms": self.started_ms,
            "updated_ms": self.updated_ms,
            "connected": self.connected,
            "table": self.table.as_ref().map(|table| json!({
                "id": table.id,
                "seat": table.seat,
                "hand_id": table.hand_id,
                "street": table.street,
                "board": table.board,
                "pot": table.pot,
                "big_blind": table.big_blind,
                "dealer": table.dealer,
                "players": table.players.iter().map(|seat| json!({
                    "seat": seat.seat,
                    "name": seat.name,
                    "stack": seat.stack,
                    "folded": seat.folded,
                })).collect::<Vec<_>>(),
            })),
            "hands": self.hands,
            "incidents": self.incidents,
            "run_incidents": self.run_incidents,
            "last_hand_ms": self.last_hand_ms,
            "last_error": self.last_error,
            "play": {
                "champion": self.play.champion,
                "probe": self.play.probe,
                "counted": self.play.counted,
                "won_mbb": self.play.won_mbb,
                "recent": self.play.recent,
            },
        })
    }

    /// The status in `value`, or the first field that is missing or of the wrong kind.
    pub fn from_json(value: &Value) -> Result<Status, String> {
        let table = match &value["table"] {
            Value::Null => None,
            table => Some(Table {
                id: text(table, "id")?,
                seat: whole(table, "seat").ok().map(|seat| seat as usize),
                hand_id: text(table, "hand_id").ok(),
                street: text(table, "street").ok(),
                board: list(table, "board")?
                    .iter()
                    .filter_map(|card| card.as_str().map(str::to_owned))
                    .collect(),
                pot: whole(table, "pot")? as u32,
                big_blind: whole(table, "big_blind")? as u32,
                dealer: whole(table, "dealer")? as usize,
                players: list(table, "players")?
                    .iter()
                    .map(|seat| {
                        Ok(Seat {
                            seat: whole(seat, "seat")? as usize,
                            name: text(seat, "name")?,
                            stack: whole(seat, "stack")? as u32,
                            folded: seat["folded"].as_bool().unwrap_or(false),
                        })
                    })
                    .collect::<Result<_, String>>()?,
            }),
        };
        let play = &value["play"];
        Ok(Status {
            bot: text(value, "bot")?,
            pid: whole(value, "pid")? as u32,
            version: text(value, "version")?,
            started_ms: signed(value, "started_ms")?,
            updated_ms: signed(value, "updated_ms")?,
            connected: value["connected"].as_bool().ok_or("connected is missing")?,
            table,
            hands: whole(value, "hands")?,
            incidents: whole(value, "incidents")?,
            run_incidents: whole(value, "run_incidents")?,
            last_hand_ms: signed(value, "last_hand_ms").ok(),
            last_error: text(value, "last_error").ok(),
            // A status from before the ledger has no play: it reads as none yet.
            play: Play {
                champion: text(play, "champion").unwrap_or_default(),
                probe: text(play, "probe").ok(),
                counted: whole(play, "counted").unwrap_or(0),
                won_mbb: signed(play, "won_mbb").unwrap_or(0),
                recent: list(play, "recent")
                    .map(|recent| recent.iter().filter_map(Value::as_i64).collect())
                    .unwrap_or_default(),
            },
        })
    }

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
        let bytes = self.to_json().to_string().into_bytes();

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
        let invalid = |why: String| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!("status at {}: {why}", path.display()),
            )
        };
        let value: Value =
            serde_json::from_slice(&fs::read(path)?).map_err(|error| invalid(error.to_string()))?;
        Status::from_json(&value).map_err(invalid)
    }
}

fn text(value: &Value, key: &str) -> Result<String, String> {
    value[key]
        .as_str()
        .map(str::to_owned)
        .ok_or_else(|| format!("{key} is missing"))
}

fn whole(value: &Value, key: &str) -> Result<u64, String> {
    value[key]
        .as_u64()
        .ok_or_else(|| format!("{key} is missing"))
}

fn signed(value: &Value, key: &str) -> Result<i64, String> {
    value[key]
        .as_i64()
        .ok_or_else(|| format!("{key} is missing"))
}

fn list<'a>(value: &'a Value, key: &str) -> Result<&'a Vec<Value>, String> {
    value[key]
        .as_array()
        .ok_or_else(|| format!("{key} is missing"))
}

/// Every bot's status in one directory, with a line for each file that could not be read.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
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
